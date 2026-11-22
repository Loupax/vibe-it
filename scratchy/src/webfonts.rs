//! QFF1a: `@font-face` web fonts (ttf/otf/woff/woff2).
//!
//! **Rules.** css.rs parses `@font-face` blocks (also inside `@media`/`@supports`) into [`FaceRule`]s
//! (`parse_face_block`); the rules cache publishes the page's list after every parse
//! ([`publish_declared`], main thread). `font-display` is ignored: a face swaps in when it arrives.
//!
//! **Use.** `FontFamily::resolve` (css.rs; any thread, memoized) asks [`lookup`] for every named family
//! it tries: a family some rule declares marks the best-matching face (style, then weight, per CSS
//! Fonts 4 §5.2; faces whose `unicode-range` misses Basic Latin only when no other face fits) as
//! wanted, and a family with a loaded face resolves to its per-page mangled name
//! (`"<family> \u{2063}wf<page>"`), so faces never leak into another page.
//!
//! **Loading.** The App drains the wanted faces ([`take_wanted`]), resolves `local()` sources in its
//! system font database and fetches the rest through wire on a thread each ([`load_sources`]: CORS
//! mode, credentials same-origin, the page's partition headers; svg / eot sources are skipped
//! and the next one is tried), decodes woff (zlib) to sfnt, and registers the result ([`finish_load`]).
//! Script faces (`FontFace` in `document.fonts`) register through the same registry (`add_script_face`).
//!
//! **Font systems.** The registry keeps an append-only log of face additions and removals. Every font
//! system (the App's paint one, the shaping worker's, the forced-layout one in js_geom, the reftest
//! renderer's) replays the log with [`sync`] before it shapes: the same operations in the same order
//! give the same `fontdb::ID`s in every database (the worker shapes, the App paints with those ids).
//! A removed face's slot is replayed as a dummy add + remove, so a database that joins late still
//! ends up with the same ids. A sync that removed faces rebuilds the `FontSystem` around its database
//! (cosmic-text's font cache would otherwise keep the bytes alive). A sync that changed anything clears
//! this thread's family-resolution cache and shape memo; the App also restarts the incremental engine
//! (`incr_style::drop_engines`) so text that used a fallback is shaped again.
//!
//! [`new_page`] (navigation) removes every face of the page.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The marker between a family name and its page number in a mangled family name.
const MANGLE: char = '\u{2063}';

/// At most this many faces are fetched per page.
pub(crate) const MAX_FACES_PER_PAGE: usize = 64;
/// A font file larger than this is not used.
pub(crate) const MAX_FONT_BYTES: usize = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FaceStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FontSrc {
    Url { url: String, format: Option<String> },
    Local(String),
}

/// One `@font-face` rule.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FaceRule {
    /// As written (unquoted, whitespace collapsed).
    pub(crate) family: String,
    pub(crate) srcs: Vec<FontSrc>,
    /// `font-weight` range; `None` = `auto` (the font's own weight).
    pub(crate) weight: Option<(u16, u16)>,
    pub(crate) style: FaceStyle,
    /// `unicode-range` (inclusive); empty = everything.
    pub(crate) ranges: Vec<(u32, u32)>,
    /// The enclosing `@media` conditions (empty = always).
    pub(crate) media: Vec<MediaQuery>,
}

impl FaceRule {
    /// The registry key: lower-cased family and the first source as written. `document.fonts`
    /// (fonts_prelude.js) builds the same key for a rule's `FontFace`.
    pub(crate) fn key(&self) -> String {
        let first = match self.srcs.first() {
            Some(FontSrc::Url { url, .. }) => url.clone(),
            Some(FontSrc::Local(n)) => format!("local({n})"),
            None => String::new(),
        };
        format!("{}|{first}", family_key(&self.family))
    }

    fn covers_latin(&self) -> bool {
        self.ranges.is_empty() || self.ranges.iter().any(|&(a, b)| a <= 0x7a && b >= 0x41)
    }

    fn loadable(&self) -> bool {
        self.srcs.iter().any(|s| match s {
            FontSrc::Url { url, format } => src_supported(url, format.as_deref()),
            FontSrc::Local(_) => true,
        })
    }
}

/// A family name as the registry compares it.
pub(crate) fn family_key(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Whether a `url()` source can be decoded here: ttf/otf/woff/woff2 (and collections). svg
/// and eot are skipped. Without `format()` the file extension decides (unknown = try it).
pub(crate) fn src_supported(url: &str, format: Option<&str>) -> bool {
    if let Some(f) = format {
        return f.split(',').any(|f| {
            matches!(f.trim().trim_matches(['"', '\'']).to_ascii_lowercase().as_str(), "truetype" | "opentype" | "woff" | "woff2" | "collection" | "truetype-aat")
        });
    }
    if url.trim_start().get(..5).is_some_and(|s| s.eq_ignore_ascii_case("data:")) {
        let head = url.get(..48).unwrap_or(url).to_ascii_lowercase();
        return !head.contains("svg") && !head.contains("embedded-opentype");
    }
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    !(path.ends_with(".svg") || path.ends_with(".svgz") || path.ends_with(".eot"))
}

// ------------------------------------------------------------------ parsing

/// Parses the body of an `@font-face` block (the text between the braces). `None` without a usable
/// `font-family` or `src`.
pub(crate) fn parse_face_block(text: &str, media: Vec<MediaQuery>) -> Option<FaceRule> {
    let mut family = None;
    let mut srcs = None;
    let mut weight = Some((400, 400));
    let mut style = FaceStyle::Normal;
    let mut ranges = Vec::new();
    for decl in split_top(text, ';') {
        let Some((name, value)) = decl.split_once(':') else { continue };
        let value = value.trim().trim_end_matches("!important").trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "font-family" => family = parse_family_value(value),
            "src" => srcs = parse_src(value),
            "font-weight" => {
                if let Some(w) = parse_weight(value) {
                    weight = w;
                }
            }
            "font-style" => {
                if let Some(s) = parse_style(value) {
                    style = s;
                }
            }
            "unicode-range" => ranges = parse_unicode_range(value),
            _ => {}
        }
    }
    let family = family?;
    let srcs = srcs.filter(|s: &Vec<FontSrc>| !s.is_empty())?;
    Some(FaceRule { family, srcs, weight, style, ranges, media })
}

/// Splits at `sep` outside quotes and parentheses.
fn split_top(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start, mut esc) = (0i32, None::<char>, 0usize, false);
    for (i, c) in text.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match (quote, c) {
            (_, '\\') => esc = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = (depth - 1).max(0),
            (None, c) if c == sep && depth == 0 => {
                out.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        &s[1..s.len() - 1]
    } else {
        s
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_family_value(v: &str) -> Option<String> {
    let f = unquote(v);
    let f = f.split_whitespace().collect::<Vec<_>>().join(" ");
    (!f.is_empty() && !f.contains(',')).then_some(f)
}

/// `src`: a comma list of `url(...) [format(...)] [tech(...)]` and `local(...)`.
fn parse_src(v: &str) -> Option<Vec<FontSrc>> {
    let mut out = Vec::new();
    for item in split_top(v, ',') {
        let item = item.trim();
        let lower = item.to_ascii_lowercase();
        if lower.starts_with("local(") {
            let end = item.rfind(')')?;
            let name = unquote(&item[6..end]);
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if !name.is_empty() {
                out.push(FontSrc::Local(name));
            }
        } else if lower.starts_with("url(") {
            // The url's closing paren: after its quoted string, or the first `)`.
            let rest = &item[4..];
            let t = rest.trim_start();
            let (url, after) = if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &t[1..];
                let mut end = None;
                let mut esc = false;
                for (i, c) in body.char_indices() {
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == q {
                        end = Some(i);
                        break;
                    }
                }
                let end = end?;
                let after = body[end + 1..].trim_start();
                (unquote(&t[..end + 2]), after.strip_prefix(')')?)
            } else {
                let end = t.find(')')?;
                (t[..end].trim().to_string(), &t[end + 1..])
            };
            let after_l = after.to_ascii_lowercase();
            let format = after_l.find("format(").map(|i| {
                let s = &after[i + 7..];
                s[..s.find(')').unwrap_or(s.len())].to_string()
            });
            // `tech(...)`: color fonts, variations, palettes... only plain outlines are supported.
            if after_l.contains("tech(") && !after_l.contains("tech(variations)") {
                continue;
            }
            if !url.is_empty() {
                out.push(FontSrc::Url { url, format: format.map(|f| f.trim().to_string()) });
            }
        }
    }
    Some(out)
}

fn weight_number(t: &str) -> Option<u16> {
    match t.to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        n => n.parse::<f32>().ok().filter(|w| (1.0..=1000.0).contains(w)).map(|w| w.round() as u16),
    }
}

/// `font-weight` descriptor: `auto` (`Some(None)`), one weight, or a range.
pub(crate) fn parse_weight(v: &str) -> Option<Option<(u16, u16)>> {
    let parts: Vec<&str> = v.split_whitespace().collect();
    match parts.as_slice() {
        [a] if a.eq_ignore_ascii_case("auto") => Some(None),
        [a] => weight_number(a).map(|w| Some((w, w))),
        [a, b] => {
            let (a, b) = (weight_number(a)?, weight_number(b)?);
            Some(Some((a.min(b), a.max(b))))
        }
        _ => None,
    }
}

/// `font-style` descriptor (an oblique angle range counts as oblique).
pub(crate) fn parse_style(v: &str) -> Option<FaceStyle> {
    let first = v.split_whitespace().next()?.to_ascii_lowercase();
    match first.as_str() {
        "normal" | "auto" => Some(FaceStyle::Normal),
        "italic" => Some(FaceStyle::Italic),
        "oblique" => Some(FaceStyle::Oblique),
        _ => None,
    }
}

/// `unicode-range`: `U+26`, `U+0-7F`, `U+4??` (wildcards); bad entries are dropped.
pub(crate) fn parse_unicode_range(v: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for part in v.split(',') {
        let p = part.trim();
        let Some(h) = p.strip_prefix("U+").or_else(|| p.strip_prefix("u+")) else { continue };
        let r = if let Some((a, b)) = h.split_once('-') {
            match (u32::from_str_radix(a.trim(), 16), u32::from_str_radix(b.trim(), 16)) {
                (Ok(a), Ok(b)) if a <= b => Some((a, b)),
                _ => None,
            }
        } else if h.contains('?') {
            let lo = u32::from_str_radix(&h.replace('?', "0"), 16);
            let hi = u32::from_str_radix(&h.replace('?', "F"), 16);
            match (lo, hi) {
                (Ok(a), Ok(b)) => Some((a, b)),
                _ => None,
            }
        } else {
            u32::from_str_radix(h, 16).ok().map(|a| (a, a))
        };
        if let Some((a, b)) = r
            && a <= 0x10FFFF
        {
            out.push((a, b.min(0x10FFFF)));
        }
    }
    out
}

thread_local! {
    /// The `@font-face` rules of the last `parse_stylesheet_into` on this thread (taken by the rules
    /// cache right after its own parse).
    static PARSED: RefCell<Vec<FaceRule>> = const { RefCell::new(Vec::new()) };
    /// The list `publish_declared` last published from this thread.
    static PUBLISHED: RefCell<Option<Rc<Vec<FaceRule>>>> = const { RefCell::new(None) };
}

/// css.rs: the rules a parse found (replaces the previous parse's).
pub(crate) fn stash_parsed(faces: Vec<FaceRule>) {
    PARSED.with(|p| *p.borrow_mut() = faces);
}

/// rules_cache.rs: the rules of the parse that just ran.
pub(crate) fn take_parsed() -> Vec<FaceRule> {
    PARSED.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

// ------------------------------------------------------------------ registry

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadState {
    Unrequested,
    Wanted,
    Loading,
    Loaded,
    Failed,
}

impl LoadState {
    pub(crate) fn word(self) -> &'static str {
        match self {
            LoadState::Unrequested => "unloaded",
            LoadState::Wanted | LoadState::Loading => "loading",
            LoadState::Loaded => "loaded",
            LoadState::Failed => "error",
        }
    }
}

struct Entry {
    rule: FaceRule,
    state: LoadState,
    /// Slot of the loaded face.
    slot: Option<usize>,
}

/// A decoded face ready to be added to the font systems.
pub(crate) struct LoadedFace {
    pub(crate) info: fontdb::FaceInfo,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Add(usize),
    Remove(usize),
}

#[derive(Default)]
struct Registry {
    page: u64,
    entries: HashMap<String, Entry>,
    /// Keys of the declared (CSS) faces by family key, in source order.
    by_family: HashMap<String, Vec<String>>,
    /// Keys waiting for the App to fetch them.
    wanted: Vec<String>,
    requested: usize,
    /// Families with at least one loaded face (family key -> mangled name).
    loaded: HashMap<String, &'static str>,
    slots: Vec<Option<Arc<LoadedFace>>>,
    log: Vec<Op>,
}

/// One App's web fonts: the registry and its cheap flags. Shared by the App's thread and its shaping
/// worker (`install`); every other thread (a test, the reftest renderer, a WPT child) gets its own on
/// first use, so parallel tests never see each other's faces.
pub(crate) struct Shared {
    reg: Mutex<Registry>,
    /// Bumped on every registry change that can change a resolution (declared set, loads, removals).
    generation: AtomicU64,
    /// The page declares or loaded anything (`lookup`'s fast path).
    active: AtomicBool,
    wanted: AtomicBool,
    page: AtomicU64,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}

/// This thread's registry handle (created on first use).
pub(crate) fn handle() -> Arc<Shared> {
    CURRENT.with(|c| {
        c.borrow_mut()
            .get_or_insert_with(|| {
                Arc::new(Shared {
                    reg: Mutex::new(Registry { page: 1, ..Default::default() }),
                    generation: AtomicU64::new(1),
                    active: AtomicBool::new(false),
                    wanted: AtomicBool::new(false),
                    page: AtomicU64::new(1),
                })
            })
            .clone()
    })
}

/// Makes `h` this thread's registry (the shaping worker uses its App's).
pub(crate) fn install(h: Arc<Shared>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(h));
}

fn with_reg<T>(f: impl FnOnce(&Shared, &mut Registry) -> T) -> T {
    let sh = handle();
    let mut g = sh.reg.lock().unwrap_or_else(|e| e.into_inner());
    f(&sh, &mut g)
}

fn bump() {
    handle().generation.fetch_add(1, Ordering::AcqRel);
}

fn update_active(sh: &Shared, r: &Registry) {
    sh.active.store(!r.by_family.is_empty() || !r.loaded.is_empty(), Ordering::Release);
}

/// The registry generation (changes whenever a resolution may change).
pub(crate) fn generation() -> u64 {
    handle().generation.load(Ordering::Acquire)
}



fn mangled(page: u64, fam: &str) -> &'static str {
    Box::leak(format!("{fam} {MANGLE}wf{page}").into_boxed_str())
}

/// Navigation: every face of the old page goes (from every font system at its next sync).
pub(crate) fn new_page() {
    with_reg(|sh, r| {
        let page = sh.page.fetch_add(1, Ordering::AcqRel) + 1;
        for (i, s) in r.slots.iter_mut().enumerate() {
            if s.take().is_some() {
                r.log.push(Op::Remove(i));
            }
        }
        r.page = page;
        r.entries.clear();
        r.by_family.clear();
        r.wanted.clear();
        r.requested = 0;
        r.loaded.clear();
        update_active(sh, r);
        sh.wanted.store(false, Ordering::Release);
    });
    PUBLISHED.with(|p| *p.borrow_mut() = None);
    bump();
    clear_thread_caches();
}

/// rules_cache.rs: the page's `@font-face` rules after a parse (cheap when unchanged). Rules inside a
/// non-matching `@media` are left out (evaluated at the current viewport).
pub(crate) fn publish_declared(rules: &Rc<Vec<FaceRule>>) {
    let same = PUBLISHED.with(|p| p.borrow().as_ref().is_some_and(|old| Rc::ptr_eq(old, rules) || **old == **rules));
    if same {
        PUBLISHED.with(|p| *p.borrow_mut() = Some(rules.clone()));
        return;
    }
    PUBLISHED.with(|p| *p.borrow_mut() = Some(rules.clone()));
    let vw = crate::css::current_viewport().0;
    with_reg(|sh, r| {
        r.by_family.clear();
        for rule in rules.iter() {
            if !crate::css::media_list_matches(&rule.media, vw) {
                continue;
            }
            let key = rule.key();
            r.entries.entry(key.clone()).or_insert_with(|| Entry { rule: rule.clone(), state: LoadState::Unrequested, slot: None });
            let list = r.by_family.entry(family_key(&rule.family)).or_default();
            if !list.contains(&key) {
                list.push(key);
            }
        }
        update_active(sh, r);
    });
    bump();
    clear_thread_caches();
}

/// CSS Fonts 4 §5.2 font-style then font-weight distance of a face for a request (lower is better).
fn match_score(rule: &FaceRule, weight: u16, style: FaceStyle) -> (u8, u32) {
    let s = match (style, rule.style) {
        (a, b) if a == b => 0,
        (FaceStyle::Italic, FaceStyle::Oblique) | (FaceStyle::Oblique, FaceStyle::Italic) => 1,
        (FaceStyle::Normal, FaceStyle::Oblique) => 1,
        _ => 2,
    };
    let (lo, hi) = rule.weight.unwrap_or((400, 400));
    let w = if (lo..=hi).contains(&weight) {
        0
    } else {
        let below = weight > hi;
        let d = if below { (weight - hi) as u32 } else { (lo - weight) as u32 };
        // Prefer the direction CSS prefers for this weight (lighter for < 400 and 400-500, heavier above 500).
        let preferred = if weight <= 500 { below } else { !below };
        if (400..=500).contains(&weight) && !below && lo <= 500 {
            d
        } else if preferred {
            d + 1000
        } else {
            d + 2000
        }
    };
    (s, w)
}

/// `FontFamily::resolve`: the mangled family to try for `name` (a family with a loaded face), and
/// the wanted-face note for this request. `None` for families no `@font-face` names.
pub(crate) fn lookup(name: &str, weight: FontWeight, style: FontStyle) -> Option<&'static str> {
    let sh = handle();
    if !sh.active.load(Ordering::Acquire) {
        return None;
    }
    let fam = family_key(name);
    let st = match style {
        FontStyle::Normal => FaceStyle::Normal,
        FontStyle::Italic => FaceStyle::Italic,
        FontStyle::Oblique => FaceStyle::Oblique,
    };
    let mut g = sh.reg.lock().unwrap_or_else(|e| e.into_inner());
    let r = &mut *g;
    {
        if let Some(keys) = r.by_family.get(&fam) {
            let pick = |latin: bool| {
                keys.iter()
                    .filter_map(|k| r.entries.get(k).map(|e| (k, e)))
                    .filter(|(_, e)| e.rule.loadable() && (!latin || e.rule.covers_latin()))
                    .min_by_key(|(_, e)| match_score(&e.rule, weight.0, st))
                    .map(|(k, _)| k.clone())
            };
            if let Some(k) = pick(true).or_else(|| pick(false))
                && let Some(e) = r.entries.get_mut(&k)
                && e.state == LoadState::Unrequested
                && r.requested < MAX_FACES_PER_PAGE
            {
                e.state = LoadState::Wanted;
                r.requested += 1;
                r.wanted.push(k);
                sh.wanted.store(true, Ordering::Release);
            }
        }
        r.loaded.get(&fam).copied()
    }
}

/// Whether some face is waiting to be fetched (cheap).
pub(crate) fn has_wanted() -> bool {
    handle().wanted.load(Ordering::Acquire)
}

/// A face to fetch: its key, the page it belongs to, and its sources in order.
pub(crate) struct FetchJob {
    pub(crate) key: String,
    pub(crate) page: u64,
    pub(crate) srcs: Vec<FontSrc>,
}

/// The App: the wanted faces (now `loading`).
pub(crate) fn take_wanted() -> Vec<FetchJob> {
    if !handle().wanted.swap(false, Ordering::AcqRel) {
        return Vec::new();
    }
    with_reg(|_, r| {
        let keys = std::mem::take(&mut r.wanted);
        let page = r.page;
        keys.into_iter()
            .filter_map(|k| {
                let e = r.entries.get_mut(&k)?;
                e.state = LoadState::Loading;
                Some(FetchJob { key: k, page, srcs: e.rule.srcs.clone() })
            })
            .collect()
    })
}

/// The load state of a face by key (`document.fonts`).
pub(crate) fn state_of(key: &str) -> LoadState {
    with_reg(|_, r| r.entries.get(key).map_or(LoadState::Unrequested, |e| e.state))
}

fn register(sh: &Shared, r: &mut Registry, key: &str, mut info: fontdb::FaceInfo) {
    let Some(e) = r.entries.get_mut(key) else { return };
    let fam = family_key(&e.rule.family);
    let name = *r.loaded.entry(fam).or_insert_with(|| mangled(r.page, &family_key(&e.rule.family)));
    info.families = vec![(name.to_string(), fontdb::Language::English_UnitedStates)];
    if let Some((lo, hi)) = e.rule.weight {
        info.weight = fontdb::Weight(info.weight.0.clamp(lo, hi));
    }
    info.style = match e.rule.style {
        FaceStyle::Normal => fontdb::Style::Normal,
        FaceStyle::Italic => fontdb::Style::Italic,
        FaceStyle::Oblique => fontdb::Style::Oblique,
    };
    let slot = r.slots.len();
    r.slots.push(Some(Arc::new(LoadedFace { info })));
    r.log.push(Op::Add(slot));
    e.state = LoadState::Loaded;
    e.slot = Some(slot);
    update_active(sh, r);
}

/// The App: a fetch finished (`face` = the decoded face, `None` = every source failed). False when
/// the page has changed meanwhile (nothing is registered).
pub(crate) fn finish_load(key: &str, page: u64, face: Option<fontdb::FaceInfo>) -> bool {
    let changed = with_reg(|sh, r| {
        if r.page != page {
            return false;
        }
        let Some(e) = r.entries.get_mut(key) else { return false };
        if e.state == LoadState::Loaded {
            return false;
        }
        match face {
            Some(info) => register(sh, r, key, info),
            None => e.state = LoadState::Failed,
        }
        true
    });
    if changed {
        bump();
    }
    changed
}

/// `FontFace` bytes from script (`__scratchy_font_add`): decodes and registers the face under `key`
/// (a stylesheet face's key, or a script face's own). False when the bytes are not a usable font.
pub(crate) fn add_script_face(key: &str, family: &str, bytes: Vec<u8>, weight: &str, style: &str) -> bool {
    let Some(info) = decode_face(bytes) else {
        with_reg(|_, r| {
            if let Some(e) = r.entries.get_mut(key)
                && e.state != LoadState::Loaded
            {
                e.state = LoadState::Failed;
            }
        });
        return false;
    };
    let family = family.split_whitespace().collect::<Vec<_>>().join(" ");
    if family.is_empty() {
        return false;
    }
    with_reg(|sh, r| {
        let e = r.entries.entry(key.to_string()).or_insert_with(|| Entry {
            rule: FaceRule {
                family: family.clone(),
                srcs: Vec::new(),
                weight: parse_weight(weight).unwrap_or(Some((400, 400))),
                style: parse_style(style).unwrap_or(FaceStyle::Normal),
                ranges: Vec::new(),
                media: Vec::new(),
            },
            state: LoadState::Unrequested,
            slot: None,
        });
        if e.state == LoadState::Loaded {
            return;
        }
        register(sh, r, key, info);
    });
    bump();
    true
}

/// `document.fonts.delete(face)` of a script face: the face goes.
pub(crate) fn remove_script_face(key: &str) {
    let changed = with_reg(|sh, r| {
        let Some(e) = r.entries.remove(key) else { return false };
        if let Some(slot) = e.slot
            && r.slots.get_mut(slot).and_then(Option::take).is_some()
        {
            r.log.push(Op::Remove(slot));
        }
        // The family keeps its mangled name while another loaded face uses it.
        let fam = family_key(&e.rule.family);
        if !r.entries.values().any(|o| o.state == LoadState::Loaded && family_key(&o.rule.family) == fam) {
            r.loaded.remove(&fam);
        }
        update_active(sh, r);
        true
    });
    if changed {
        bump();
    }
}

// ------------------------------------------------------------------ font systems

/// One font system's position in the face log.
#[derive(Default)]
pub(crate) struct FontSync {
    applied: usize,
    ids: Vec<Option<fontdb::ID>>,
    seen_gen: u64,
}

fn dummy_face() -> fontdb::FaceInfo {
    fontdb::FaceInfo {
        id: fontdb::ID::dummy(),
        source: fontdb::Source::Binary(Arc::new(Vec::<u8>::new())),
        index: 0,
        families: vec![(format!("{MANGLE}removed"), fontdb::Language::English_UnitedStates)],
        post_script_name: String::new(),
        style: fontdb::Style::Normal,
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        monospaced: false,
    }
}

/// This thread's cached family resolutions and shape memo (a resolution may have changed).
pub(crate) fn clear_thread_caches() {
    crate::css::clear_family_cache();
}

/// Brings `fs` up to the registry: replays the face log since `st`'s position. True when faces were
/// added or removed (this thread's family cache and shape memo were cleared; text needs shaping again).
pub(crate) fn sync(fs: &mut FontSystem, st: &mut FontSync) -> bool {
    let g = generation();
    if g == st.seen_gen {
        return false;
    }
    st.seen_gen = g;
    let (changed, removed) = {
        let sh = handle();
        let guard = sh.reg.lock().unwrap_or_else(|e| e.into_inner());
        let r = &*guard;
        if st.applied >= r.log.len() {
            (false, false)
        } else {
            let mut removed = false;
            let db = fs.db_mut();
            for op in &r.log[st.applied..] {
                match *op {
                    Op::Add(slot) => {
                        let info = r.slots.get(slot).and_then(|s| s.as_ref()).map_or_else(dummy_face, |f| f.info.clone());
                        let id = db.push_face_info(info);
                        if st.ids.len() <= slot {
                            st.ids.resize(slot + 1, None);
                        }
                        st.ids[slot] = Some(id);
                    }
                    Op::Remove(slot) => {
                        if let Some(id) = st.ids.get_mut(slot).and_then(Option::take) {
                            db.remove_face(id);
                            removed = true;
                        }
                    }
                }
            }
            st.applied = r.log.len();
            (true, removed)
        }
    };
    if removed {
        // cosmic-text caches loaded fonts by id for the life of the FontSystem: start a new one around
        // the same database so the removed faces' bytes are freed.
        let placeholder = FontSystem::new_with_locale_and_db(String::new(), fontdb::Database::new());
        let (locale, db) = std::mem::replace(fs, placeholder).into_locale_and_db();
        *fs = FontSystem::new_with_locale_and_db(locale, db);
    }
    clear_thread_caches();
    if changed {
        crate::layout::memo_clear();
    }
    changed
}

// ------------------------------------------------------------------ decoding

/// sfnt bytes from a font file: ttf/otf/ttc as they are, woff (1.0) decompressed, woff2 (QFF1b) decoded. `None` for anything else.
pub(crate) fn to_sfnt(bytes: Vec<u8>) -> Option<Vec<u8>> {
    match bytes.get(..4)? {
        [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" | b"typ1" => Some(bytes),
        b"wOFF" => woff_to_sfnt(&bytes),
        b"wOF2" => crate::woff2::woff2_to_sfnt(&bytes),
        _ => None,
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// WOFF 1.0 (W3C REC 2012): a 44-byte header, a table directory (tag, offset, compLength, origLength,
/// origChecksum), each table zlib-compressed when compLength < origLength.
pub(crate) fn woff_to_sfnt(b: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let flavor = be32(b, 4)?;
    let num = be16(b, 12)? as usize;
    let total = be32(b, 16)? as usize;
    if num == 0 || num > 4096 || total > MAX_FONT_BYTES {
        return None;
    }
    let mut tables = Vec::with_capacity(num);
    for i in 0..num {
        let at = 44 + i * 20;
        let tag = be32(b, at)?;
        let off = be32(b, at + 4)? as usize;
        let comp = be32(b, at + 8)? as usize;
        let orig = be32(b, at + 12)? as usize;
        let sum = be32(b, at + 16)?;
        let src = b.get(off..off.checked_add(comp)?)?;
        let data = if comp < orig {
            let mut out = Vec::with_capacity(orig);
            flate2::read::ZlibDecoder::new(src).take(orig as u64 + 1).read_to_end(&mut out).ok()?;
            if out.len() != orig {
                return None;
            }
            out
        } else if comp == orig {
            src.to_vec()
        } else {
            return None;
        };
        tables.push((tag, sum, data));
    }
    tables.sort_by_key(|t| t.0);
    let mut out = Vec::with_capacity(total.max(12 + 16 * num));
    let mut sel = 0u16;
    while (1usize << (sel + 1)) <= num {
        sel += 1;
    }
    let range = (1u16 << sel) * 16;
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(num as u16).to_be_bytes());
    out.extend_from_slice(&range.to_be_bytes());
    out.extend_from_slice(&sel.to_be_bytes());
    out.extend_from_slice(&((num as u16) * 16 - range).to_be_bytes());
    let mut off = 12 + 16 * num;
    for (tag, sum, data) in &tables {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&(off as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        off += data.len().div_ceil(4) * 4;
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Some(out)
}

/// Decodes a font file and reads its first face (`None` when it is not a usable font).
pub(crate) fn decode_face(bytes: Vec<u8>) -> Option<fontdb::FaceInfo> {
    if bytes.len() > MAX_FONT_BYTES {
        return None;
    }
    let sfnt = to_sfnt(bytes)?;
    let mut tmp = fontdb::Database::new();
    let ids = tmp.load_font_source(fontdb::Source::Binary(Arc::new(sfnt)));
    let id = *ids.first()?;
    tmp.face(id).cloned()
}

/// The first face of `local(name)` in a system font database: the PostScript name or a family name
/// (case-insensitive; web fonts of earlier pages never match).
pub(crate) fn local_face(db: &fontdb::Database, name: &str) -> Option<fontdb::FaceInfo> {
    let want = family_key(name);
    let hit = db.faces().find(|f| {
        !f.families.iter().any(|(n, _)| n.contains(MANGLE))
            && (family_key(&f.post_script_name) == want || f.families.iter().any(|(n, _)| family_key(n) == want))
    })?;
    // A copy of the bytes, so the face lives in every font system's database the same way.
    let (data, index) = db.with_face_data(hit.id, |d, i| (d.to_vec(), i))?;
    let mut tmp = fontdb::Database::new();
    let ids = tmp.load_font_source(fontdb::Source::Binary(Arc::new(data)));
    let id = ids.iter().copied().find(|id| tmp.face(*id).is_some_and(|f| f.index == index)).or_else(|| ids.first().copied())?;
    tmp.face(id).cloned()
}

/// One source fetched: a `data:` URL decoded here, any other URL through wire in CORS mode (credentials
/// same-origin; a cross-origin response needs `Access-Control-Allow-Origin`). `page` is the
/// document's URL (origin), `top` the top-level page (partition).
pub(crate) fn fetch_font_bytes(socket: &std::path::Path, url: &Url, page: &Url, top: &Url) -> Option<Vec<u8>> {
    if url.scheme() == "data" {
        return crate::images::decode_data_uri(url.as_str());
    }
    let same_origin = url.origin() == page.origin() || (url.scheme() == "file" && page.scheme() == "file");
    let ctx = crate::net_policy::CookieContext::fetch(top, crate::net_policy::CredentialsMode::SameOrigin, same_origin);
    let mut req = crate::net::build_get_request_ctx(url, &ctx);
    if !same_origin {
        req.truncate(req.len() - 2);
        req.push_str(&format!("Origin: {}\r\n\r\n", crate::net_policy::origin_header(page)));
    }
    let r = crate::net::fetch_raw_sub_headers(socket, &req)?;
    if !(200..300).contains(&r.status) {
        return None;
    }
    if !same_origin {
        let headers: Vec<(String, String)> = r.cors_headers;
        if let Err(e) = crate::net_policy::check_cors_response(&crate::net_policy::origin_header(page), false, &headers) {
            diag!("scratchy: font {url} blocked: {e:?}");
            return None;
        }
    }
    Some(r.body)
}

/// A fetch thread: tries `srcs` in order (`local()` was resolved by the caller into `locals`, by
/// index) and returns the first face that decodes.
pub(crate) fn load_sources(
    socket: Option<&std::path::Path>,
    base: &Url,
    page: &Url,
    top: &Url,
    srcs: &[FontSrc],
    locals: &HashMap<usize, fontdb::FaceInfo>,
) -> Option<fontdb::FaceInfo> {
    for (i, src) in srcs.iter().enumerate() {
        match src {
            FontSrc::Local(_) => {
                if let Some(f) = locals.get(&i) {
                    return Some(f.clone());
                }
            }
            FontSrc::Url { url, format } => {
                if !src_supported(url, format.as_deref()) {
                    continue;
                }
                let Ok(u) = base.join(url.trim()) else { continue };
                let bytes = if u.scheme() == "data" {
                    crate::images::decode_data_uri(url.trim())
                } else {
                    let Some(socket) = socket else { continue };
                    fetch_font_bytes(socket, &u, page, top)
                };
                if let Some(face) = bytes.and_then(decode_face) {
                    return Some(face);
                }
            }
        }
    }
    None
}

/// The `local()` sources of `srcs` found in `db`, by index (resolved on the thread that owns `db`).
pub(crate) fn resolve_locals(db: &fontdb::Database, srcs: &[FontSrc]) -> HashMap<usize, fontdb::FaceInfo> {
    srcs.iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            FontSrc::Local(n) => local_face(db, n).map(|f| (i, f)),
            FontSrc::Url { .. } => None,
        })
        .collect()
}

/// Synchronous loading for runners without an event loop (the reftest renderer): fetches every
/// wanted face with `fetch` (url -> bytes) right away. True when a face was added.
pub(crate) fn load_wanted_now(db: &fontdb::Database, mut fetch: impl FnMut(&str) -> Option<Vec<u8>>) -> bool {
    let mut any = false;
    for job in take_wanted() {
        let locals = resolve_locals(db, &job.srcs);
        let mut face = None;
        for (i, src) in job.srcs.iter().enumerate() {
            match src {
                FontSrc::Local(_) => face = locals.get(&i).cloned(),
                FontSrc::Url { url, format } if src_supported(url, format.as_deref()) => {
                    face = fetch(url.trim()).and_then(decode_face);
                }
                FontSrc::Url { .. } => {}
            }
            if face.is_some() {
                break;
            }
        }
        any |= finish_load(&job.key, job.page, face.clone()) && face.is_some();
    }
    any
}

#[cfg(test)]
pub(crate) fn loaded_family(name: &str) -> Option<&'static str> {
    with_reg(|_, r| r.loaded.get(&family_key(name)).copied())
}

#[cfg(test)]
pub(crate) fn page() -> u64 {
    handle().page.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn face_count() -> usize {
    with_reg(|_, r| r.slots.iter().filter(|s| s.is_some()).count())
}
