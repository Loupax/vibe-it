//! QY15: ES modules. QuickJS' module loader is synchronous, so the whole
//! static import graph is pre-fetched (through `wire`, before the engine
//! runs, on the same thread as the classic scripts' pre-fetch) into plain
//! `ScriptSource`s of kind `ModuleDep`; the engine's `Resolver`/`Loader`
//! then only read that map. Bare specifiers go through the page's
//! `<script type=importmap>`. A dynamic `import()` of a literal specifier
//! is scanned like a static import (so it is pre-fetched too); one that
//! is computed at run time and is not in the map rejects with a clear
//! error (runtime fetching is QY15a).
use super::*;
use crate::js::{ScriptKind, ScriptSource};
use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Ctx, Error as JsError, Module};

/// Graph caps: modules, total source bytes, import depth.
pub(crate) const MAX_MODULES: usize = 200;
pub(crate) const MAX_MODULE_BYTES: usize = 16 << 20;
pub(crate) const MAX_MODULE_DEPTH: usize = 32;

// ---------------------------------------------------------------- JSON

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool,
    Num,
    Str(String),
    Arr,
    Obj(Vec<(String, Json)>),
}

struct JsonP<'a> {
    b: &'a [u8],
    i: usize,
}

impl JsonP<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 16 {
            return None;
        }
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Json::Obj(v));
                }
                loop {
                    self.ws();
                    let Json::Str(k) = self.value(depth + 1)? else { return None };
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return None;
                    }
                    self.i += 1;
                    let val = self.value(depth + 1)?;
                    v.push((k, val));
                    self.ws();
                    match self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Json::Obj(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Json::Arr);
                }
                loop {
                    self.value(depth + 1)?;
                    self.ws();
                    match self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Json::Arr);
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => {
                self.i += 1;
                let mut out: Vec<u8> = Vec::new();
                loop {
                    let c = *self.b.get(self.i)?;
                    self.i += 1;
                    match c {
                        b'"' => break,
                        b'\\' => {
                            let e = *self.b.get(self.i)?;
                            self.i += 1;
                            match e {
                                b'n' => out.push(b'\n'),
                                b't' => out.push(b'\t'),
                                b'r' => out.push(b'\r'),
                                b'b' => out.push(8),
                                b'f' => out.push(12),
                                b'u' => {
                                    let h = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
                                    self.i += 4;
                                    let ch = char::from_u32(u32::from_str_radix(h, 16).ok()?).unwrap_or('\u{fffd}');
                                    let mut buf = [0u8; 4];
                                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                }
                                other => out.push(other),
                            }
                        }
                        c => out.push(c),
                    }
                }
                Some(Json::Str(String::from_utf8_lossy(&out).into_owned()))
            }
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Json::Bool)
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Json::Bool)
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(Json::Null)
            }
            c if c == b'-' || c.is_ascii_digit() => {
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                Some(Json::Num)
            }
            _ => None,
        }
    }
}

fn parse_json(s: &str) -> Option<Json> {
    let mut p = JsonP { b: s.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.i == s.len()).then_some(v)
}

// ------------------------------------------------------------- import map

/// A parsed `<script type=importmap>`: addresses already resolved against
/// the document base. Entries are kept longest-key-first.
#[derive(Debug, Clone, Default)]
pub(crate) struct ImportMap {
    imports: Vec<(String, String)>,
    scopes: Vec<(String, Vec<(String, String)>)>,
}

fn parse_specifier_map(v: &Json, base: &Url) -> Vec<(String, String)> {
    let Json::Obj(items) = v else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in items {
        let Json::Str(addr) = v else { continue };
        // Keys: URL-like keys are normalised against the base.
        let key = if k.starts_with("./") || k.starts_with("../") || k.starts_with('/') { base.join(k).map(|u| u.to_string()).unwrap_or_else(|_| k.clone()) } else { k.clone() };
        let Some(val) = resolve_url_like(addr, base) else { continue };
        let val = val.to_string();
        // A key ending in `/` needs an address ending in `/`.
        if key.ends_with('/') && !val.ends_with('/') {
            continue;
        }
        out.push((key, val));
    }
    out.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    out
}

impl ImportMap {
    pub(crate) fn parse(text: &str, base: &Url) -> Option<ImportMap> {
        let Json::Obj(top) = parse_json(text)? else { return None };
        let mut m = ImportMap::default();
        for (k, v) in &top {
            if k == "imports" {
                m.imports = parse_specifier_map(v, base);
            } else if k == "scopes"
                && let Json::Obj(scopes) = v
            {
                for (sk, sv) in scopes {
                    let Ok(sk) = base.join(sk) else { continue };
                    m.scopes.push((sk.to_string(), parse_specifier_map(sv, base)));
                }
                m.scopes.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
            }
        }
        Some(m)
    }

    fn lookup(map: &[(String, String)], key: &str) -> Option<String> {
        for (k, v) in map {
            if k == key {
                return Some(v.clone());
            }
            if k.ends_with('/')
                && let Some(rest) = key.strip_prefix(k.as_str())
            {
                return Some(format!("{v}{rest}"));
            }
        }
        None
    }

    fn apply(&self, key: &str, referrer: &str) -> Option<String> {
        for (scope, map) in &self.scopes {
            if (referrer == scope || (scope.ends_with('/') && referrer.starts_with(scope.as_str())))
                && let Some(v) = Self::lookup(map, key)
            {
                return Some(v);
            }
        }
        Self::lookup(&self.imports, key)
    }
}

/// `/x`, `./x`, `../x` against `base`, or an absolute URL; `None` for a
/// bare specifier.
fn resolve_url_like(spec: &str, base: &Url) -> Option<Url> {
    if spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../") {
        return base.join(spec).ok();
    }
    Url::parse(spec).ok()
}

/// The module specifier algorithm: import map first (scoped, then
/// top-level), then URL-like resolution. The error text is what the page
/// sees.
pub(crate) fn resolve_specifier(spec: &str, referrer: &str, map: &ImportMap, doc_base: &Url) -> Result<String, String> {
    let ref_url = Url::parse(referrer).unwrap_or_else(|_| doc_base.clone());
    let ref_key = ref_url.to_string();
    let url_like = resolve_url_like(spec, &ref_url);
    let key = url_like.as_ref().map(|u| u.to_string()).unwrap_or_else(|| spec.to_string());
    if let Some(v) = map.apply(&key, &ref_key) {
        return Ok(v);
    }
    match url_like {
        Some(u) => Ok(u.to_string()),
        None => Err(format!("Failed to resolve module specifier '{spec}': relative specifier or an import map entry required")),
    }
}

// ----------------------------------------------------------- import scan

/// Static `import`/`export ... from` specifiers plus dynamic `import('lit')`
/// literals, found by a small comment/string/template/regex-aware scan
/// (not a parser: a miss only means the module is not pre-fetched).
pub(crate) fn scan_import_specifiers(src: &str) -> Vec<String> {
    scan_imports(src).into_iter().map(|(s, _)| s).collect()
}

/// `(specifier, is_dynamic)` pairs, see `scan_import_specifiers`.
pub(crate) fn scan_imports(src: &str) -> Vec<(String, bool)> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    // The previous significant token allows a `/` to start a regex.
    let mut regex_ok = true;
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    while i < n {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'\'' | b'"' => {
                i = skip_string(b, i).0;
                regex_ok = false;
            }
            b'`' => {
                i += 1;
                while i < n && b[i] != b'`' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                regex_ok = false;
            }
            b'/' if regex_ok => {
                i += 1;
                let mut class = false;
                while i < n && b[i] != b'\n' {
                    match b[i] {
                        b'\\' => i += 1,
                        b'[' => class = true,
                        b']' => class = false,
                        b'/' if !class => break,
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                regex_ok = false;
            }
            c if is_id(c) => {
                let s = i;
                while i < n && is_id(b[i]) {
                    i += 1;
                }
                let word = &src[s..i];
                let after_dot = s > 0 && b[..s].iter().rev().find(|c| !c.is_ascii_whitespace()) == Some(&b'.');
                if !after_dot && (word == "import" || word == "export") {
                    let dynamic = word == "import" && b[i..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'(');
                    let (spec, end) = scan_module_clause(b, i, word == "import");
                    if let Some(spec) = spec {
                        out.push((spec, dynamic));
                        i = end;
                    }
                }
                regex_ok = matches!(word, "return" | "typeof" | "case" | "in" | "of" | "delete" | "void" | "throw" | "new" | "else" | "do" | "yield" | "await");
            }
            b')' | b']' | b'}' => {
                regex_ok = c == b'}';
                i += 1;
            }
            _ => {
                regex_ok = true;
                i += 1;
            }
        }
    }
    out
}

/// Returns (index after the string, the string's raw contents).
fn skip_string(b: &[u8], start: usize) -> (usize, String) {
    let q = b[start];
    let mut i = start + 1;
    let mut s = Vec::new();
    while i < b.len() && b[i] != q && b[i] != b'\n' {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
        }
        s.push(b[i]);
        i += 1;
    }
    (i + 1, String::from_utf8_lossy(&s).into_owned())
}

/// After an `import`/`export` keyword at `i`: the module specifier if this
/// is `import 'x'`, `import(  'x'`, or a `... from 'x'` clause.
fn scan_module_clause(b: &[u8], mut i: usize, is_import: bool) -> (Option<String>, usize) {
    let n = b.len();
    let skip_ws = |mut i: usize| {
        loop {
            while i < n && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
                i += 2;
                while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            } else if i + 1 < n && b[i] == b'/' && b[i + 1] == b'/' {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
            } else {
                return i;
            }
        }
    };
    i = skip_ws(i);
    if i >= n {
        return (None, i);
    }
    if is_import {
        if b[i] == b'(' {
            let j = skip_ws(i + 1);
            if j < n && matches!(b[j], b'\'' | b'"') {
                let (end, s) = skip_string(b, j);
                let k = skip_ws(end);
                if k < n && matches!(b[k], b')' | b',') {
                    return (Some(s), end);
                }
            }
            return (None, i);
        }
        if b[i] == b'.' {
            return (None, i);
        }
        if matches!(b[i], b'\'' | b'"') {
            let (end, s) = skip_string(b, i);
            return (Some(s), end);
        }
    }
    // Clause: identifiers, `{ } , *`, comments, until `from 'x'`.
    let mut last_word = String::new();
    loop {
        i = skip_ws(i);
        if i >= n {
            return (None, i);
        }
        match b[i] {
            b'{' | b'}' | b',' | b'*' => i += 1,
            b'\'' | b'"' => {
                let (end, s) = skip_string(b, i);
                return if last_word == "from" { (Some(s), end) } else { (None, i) };
            }
            c if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 => {
                let st = i;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] >= 0x80) {
                    i += 1;
                }
                last_word = String::from_utf8_lossy(&b[st..i]).into_owned();
            }
            _ => return (None, i),
        }
    }
}

// ---------------------------------------------------------- graph fetch

pub(crate) type Fetcher<'a> = &'a (dyn Fn(&Url) -> Option<String> + Sync);

/// Pre-fetches the static import graph below `entries` (each `(url, source)`
/// of an entry module). Returns one `ModuleDep` `ScriptSource` per module
/// found (the entries themselves excluded). Failures are skipped here: the
/// loader reports a missing module when (and only when) it is imported.
pub(crate) fn prefetch_graph(entries: &[(String, String)], map: &ImportMap, doc_base: &Url, fetch: Fetcher<'_>) -> Vec<ScriptSource> {
    let mut have: HashSet<String> = entries.iter().map(|(u, _)| u.clone()).collect();
    let mut bytes: usize = entries.iter().map(|(_, s)| s.len()).sum();
    let mut out: Vec<ScriptSource> = Vec::new();
    let mut level: Vec<(String, String)> = entries.to_vec();
    let mut depth = 0;
    while !level.is_empty() && depth < MAX_MODULE_DEPTH {
        depth += 1;
        let mut wanted: Vec<String> = Vec::new();
        for (url, src) in &level {
            for (spec, dynamic) in scan_imports(src) {
                // Literal dynamic `import('x')` is only followed from the
                // entries themselves: following it transitively would pull in
                // every lazy chunk of a big app (github's graph is > 5 MB).
                if dynamic && depth > 1 {
                    continue;
                }
                if let Ok(u) = resolve_specifier(&spec, url, map, doc_base)
                    && have.insert(u.clone())
                {
                    wanted.push(u);
                }
            }
        }
        wanted.truncate(MAX_MODULES.saturating_sub(have.len() - wanted.len()));
        let fetched: Vec<Option<String>> = thread::scope(|scope| {
            let hs: Vec<_> = wanted
                .iter()
                .map(|u| {
                    scope.spawn(move || {
                        if is_data_url(u) {
                            decode_data_uri_text(u)
                        } else {
                            let parsed = Url::parse(u).ok()?;
                            if !matches!(parsed.scheme(), "http" | "https" | "file") {
                                return None;
                            }
                            let r = fetch(&parsed);
                            if r.is_none() {
                                diag!("scratchy: js: could not fetch module {u}");
                            }
                            r
                        }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().ok().flatten()).collect()
        });
        let mut next = Vec::new();
        for (u, src) in wanted.into_iter().zip(fetched) {
            let Some(src) = src else { continue };
            if bytes + src.len() > MAX_MODULE_BYTES {
                diag!("scratchy: js: module graph over {} bytes; not fetching more", MAX_MODULE_BYTES);
                return out;
            }
            bytes += src.len();
            next.push((u.clone(), src.clone()));
            out.push(ScriptSource { name: u, code: src, kind: ScriptKind::ModuleDep, ..Default::default() });
        }
        level = next;
    }
    out
}

// ------------------------------------------------------------ the engine

/// Everything the engine's resolver and loader need.
pub(crate) struct ModuleStore {
    pub(crate) sources: HashMap<String, String>,
    pub(crate) map: ImportMap,
    pub(crate) doc_base: Url,
}

impl ModuleStore {
    pub(crate) fn from_scripts(scripts: &[ScriptSource], log: &dyn Fn(&str, &str)) -> ModuleStore {
        let doc_base = scripts
            .iter()
            .find_map(|s| s.base.as_deref().and_then(|b| Url::parse(b).ok()))
            .or_else(page_url)
            .unwrap_or_else(|| Url::parse("about:blank").expect("static url"));
        let mut sources = HashMap::new();
        let mut map = None;
        for s in scripts {
            match s.kind {
                ScriptKind::Module | ScriptKind::ModuleDep => {
                    sources.insert(s.name.clone(), s.code.clone());
                }
                ScriptKind::ImportMap if map.is_none() => {
                    map = Some(ImportMap::parse(&s.code, &doc_base).unwrap_or_else(|| {
                        log("error", &format!("{}: invalid import map (not a JSON object)", s.name));
                        ImportMap::default()
                    }));
                }
                _ => {}
            }
        }
        ModuleStore { sources, map: map.unwrap_or_default(), doc_base }
    }
}

pub(crate) struct PageResolver(pub(crate) Rc<ModuleStore>);
pub(crate) struct PageLoader(pub(crate) Rc<ModuleStore>);

impl Resolver for PageResolver {
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str, _attrs: Option<rquickjs::loader::ImportAttributes<'js>>) -> rquickjs::Result<String> {
        resolve_specifier(name, base, &self.0.map, &self.0.doc_base).map_err(|m| JsError::new_resolving_message(base, name, m))
    }
}

impl Loader for PageLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _attrs: Option<rquickjs::loader::ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        let src = match self.0.sources.get(name) {
            Some(s) => s.clone(),
            None if is_data_url(name) => decode_data_uri_text(name).ok_or_else(|| JsError::new_loading_message(name, "malformed data: URL"))?,
            None => {
                return Err(JsError::new_loading_message(name, "module was not pre-fetched (the fetch failed, or it is a run-time import of an unlisted URL: QY15a)"));
            }
        };
        let m = Module::declare(ctx.clone(), name, src)?;
        // `import.meta.url`: an inline module's name carries a `#inline-module-N` tag.
        let url = match name.rsplit_once("#inline-module-") {
            Some((u, _)) => u,
            None => name,
        };
        m.meta()?.set("url", url)?;
        Ok(m)
    }
}
