// Static mounts (QWPT3, scratchy R64 2.4 option 3): `wire --mount
// http://web-platform.test:8000=DIR` answers requests for that origin (and its
// `www.`/`www1.`/`www2.` subdomains and the next port as aliases) straight from
// a directory tree, with no socket for the origin. It exists so scratchy's
// testharness.js runner can load web-platform-tests pages without a Python
// server. It imitates the parts of wptserve those tests need: Content-Type by
// extension, `<file>.headers` and `__dir__.headers`, generated `x.any.html` and
// `x.window.html` wrappers (from `x.any.js` / `x.window.js`, honouring the
// `// META:` lines script/title/timeout/global), the WebIDLParser.js alias,
// `.sub.` substitution of a few `{{...}}` keys, and fixed overrides for single
// paths (`--mount-override`). `.py` handlers answer 501.

use std::path::{Path, PathBuf};

use url::Url;

pub struct Mount {
    host: String,
    port: u16,
    root: PathBuf,
}

#[derive(Default)]
pub struct Mounts {
    mounts: Vec<Mount>,
    overrides: Vec<(String, PathBuf)>,
}

pub struct MountResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MountResponse {
    fn text(status: u16, reason: &'static str, msg: &str) -> MountResponse {
        MountResponse { status, reason, headers: vec![("Content-Type".into(), "text/plain".into())], body: format!("wire: {msg}").into_bytes() }
    }
}

const ALIAS_PREFIXES: [&str; 3] = ["www.", "www1.", "www2."];

impl Mounts {
    /// `--mount http://HOST[:PORT]=DIR`.
    pub fn add_mount(&mut self, spec: &str) -> Result<(), String> {
        let (origin, dir) = spec.split_once('=').ok_or("--mount needs ORIGIN=DIR, e.g. http://web-platform.test:8000=/path/to/wpt")?;
        let url = Url::parse(origin).map_err(|e| format!("--mount: bad origin {origin:?}: {e}"))?;
        if url.scheme() != "http" || url.path() != "/" || url.query().is_some() {
            return Err(format!("--mount: origin must be http://host[:port] (got {origin:?})"));
        }
        let host = url.host_str().ok_or("--mount: origin has no host")?.to_ascii_lowercase();
        let port = url.port_or_known_default().unwrap_or(80);
        let root = std::fs::canonicalize(dir).map_err(|e| format!("--mount: cannot use directory {dir:?}: {e}"))?;
        if !root.is_dir() {
            return Err(format!("--mount: {dir:?} is not a directory"));
        }
        self.mounts.push(Mount { host, port, root });
        Ok(())
    }

    /// `--mount-override PATH=FILE`: requests for exactly PATH (on any mount) get FILE.
    pub fn add_override(&mut self, spec: &str) -> Result<(), String> {
        let (path, file) = spec.split_once('=').ok_or("--mount-override needs PATH=FILE")?;
        if !path.starts_with('/') {
            return Err(format!("--mount-override: PATH must start with / (got {path:?})"));
        }
        let file = std::fs::canonicalize(file).map_err(|e| format!("--mount-override: cannot use file {file:?}: {e}"))?;
        if !file.is_file() {
            return Err(format!("--mount-override: {file:?} is not a file"));
        }
        self.overrides.push((path.to_string(), file));
        Ok(())
    }

    /// The mount that owns this URL's origin, if any.
    pub fn find(&self, url: &Url) -> Option<&Mount> {
        if url.scheme() != "http" {
            return None;
        }
        let host = url.host_str()?.to_ascii_lowercase();
        let port = url.port_or_known_default()?;
        self.mounts.iter().find(|m| {
            (port == m.port || port == m.port.wrapping_add(1)) && (host == m.host || ALIAS_PREFIXES.iter().any(|p| host.strip_prefix(p) == Some(m.host.as_str())))
        })
    }

    pub fn serve(&self, mount: &Mount, method: &str, url: &Url) -> MountResponse {
        if method != "GET" && method != "HEAD" {
            return MountResponse::text(405, "Method Not Allowed", "mounted origins only support GET and HEAD");
        }
        let mut resp = self.serve_get(mount, url);
        if method == "HEAD" {
            resp.body.clear();
        }
        resp
    }

    fn serve_get(&self, mount: &Mount, url: &Url) -> MountResponse {
        let Some(comps) = decode_path(url.path()) else {
            return MountResponse::text(403, "Forbidden", "bad path");
        };
        let norm = format!("/{}", comps.join("/"));
        if let Some((_, file)) = self.overrides.iter().find(|(p, _)| *p == norm) {
            return match std::fs::read(file) {
                Ok(body) => MountResponse { status: 200, reason: "OK", headers: vec![("Content-Type".into(), mime_for(&norm).into())], body },
                Err(e) => MountResponse::text(500, "Internal Server Error", &format!("cannot read override {}: {e}", file.display())),
            };
        }
        let comps = if norm == "/resources/WebIDLParser.js" {
            vec!["resources".to_string(), "webidl2".into(), "lib".into(), "webidl2.js".into()]
        } else {
            comps
        };
        let mut fs = mount.root.clone();
        fs.extend(&comps);
        if fs.is_dir() {
            fs.push("index.html");
        }
        if fs.is_file() {
            return serve_real_file(mount, &fs, url);
        }
        // Generated wrappers: the .js file must exist.
        let name = comps.last().map(String::as_str).unwrap_or("");
        for (html_suffix, is_any) in [(".any.html", true), (".window.html", false)] {
            if let Some(stem) = name.strip_suffix(html_suffix) {
                let js_name = format!("{stem}{}", html_suffix.replace(".html", ".js"));
                let js_fs = fs.with_file_name(&js_name);
                if js_fs.is_file() && inside(&mount.root, &js_fs) {
                    return wrapper(mount, &js_fs, url, html_suffix, is_any);
                }
            }
        }
        MountResponse::text(404, "Not Found", &format!("not found: {norm}"))
    }
}

/// Percent-decodes a URL path into components, refusing anything that could
/// leave the mount (`..`, NUL, backslash, an encoded slash, invalid UTF-8).
/// `None` = refuse.
fn decode_path(path: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for raw in path.split('/') {
        let bytes = percent_decode(raw)?;
        let seg = String::from_utf8(bytes).ok()?;
        if seg.contains(['\0', '\\', '/']) || seg == ".." {
            return None;
        }
        if seg.is_empty() || seg == "." {
            continue;
        }
        out.push(seg);
    }
    Some(out)
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// `true` when `p` (after resolving symlinks) is still under `root`.
fn inside(root: &Path, p: &Path) -> bool {
    std::fs::canonicalize(p).is_ok_and(|c| c.starts_with(root))
}

fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "xhtml" | "xht" => "application/xhtml+xml",
        "xml" => "text/xml",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "txt" | "text" => "text/plain",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" | "ogg" => "video/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn serve_real_file(mount: &Mount, fs: &Path, url: &Url) -> MountResponse {
    if !inside(&mount.root, fs) {
        return MountResponse::text(403, "Forbidden", "path resolves outside the mount");
    }
    let name = fs.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".py") {
        eprintln!("wire: mount: {url} needs a Python handler (.py); answering 501");
        return MountResponse::text(501, "Not Implemented", "Python handlers (.py) are not supported by wire --mount");
    }
    if name.ends_with(".headers") {
        return MountResponse::text(404, "Not Found", "not found");
    }
    let mut body = match std::fs::read(fs) {
        Ok(b) => b,
        Err(e) => return MountResponse::text(404, "Not Found", &format!("cannot read file: {e}")),
    };
    let mut headers: Vec<(String, String)> = vec![("Content-Type".into(), mime_for(name).into())];
    load_headers(&mut headers, fs, url, mount);
    // wptserve's `wrap_pipeline`: `.sub.` anywhere in the request path.
    if url.path().contains(".sub.") {
        body = substitute(&body, url.host_str().unwrap_or(""), mount);
    }
    MountResponse { status: 200, reason: "OK", headers, body }
}

/// wptserve's `load_headers(request, path)` for the file `fs`: the NEIGHBOURING `__dir__.headers`
/// only (not the ancestors' ones), then `<file>.headers`; each is `<x>.sub.headers` instead (with
/// `{{...}}` substitution) when that exists. QWPT4a: checked against tools/wptserve at the pin.
fn load_headers(headers: &mut Vec<(String, String)>, fs: &Path, url: &Url, mount: &Mount) {
    let name = fs.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for stem in [fs.with_file_name("__dir__"), fs.with_file_name(name)] {
        let mut sub = stem.clone().into_os_string();
        sub.push(".sub.headers");
        let mut plain = stem.into_os_string();
        plain.push(".headers");
        let (sub, plain) = (PathBuf::from(sub), PathBuf::from(plain));
        if sub.is_file() {
            apply_headers_file(headers, &sub, Some((url.host_str().unwrap_or(""), mount)));
        } else {
            apply_headers_file(headers, &plain, None);
        }
    }
}

/// `Name: value` lines; a later header replaces an earlier one of the same name. `sub`: apply
/// `.sub.` substitution (request host, mount) first.
fn apply_headers_file(headers: &mut Vec<(String, String)>, path: &Path, sub: Option<(&str, &Mount)>) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let bytes = match sub {
        Some((host, mount)) => substitute(&bytes, host, mount),
        None => bytes,
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let Some((n, v)) = line.split_once(':') else { continue };
        let (n, v) = (n.trim(), v.trim());
        if n.is_empty() || n.contains(char::is_whitespace) || n.starts_with('#') {
            continue;
        }
        headers.retain(|(h, _)| !h.eq_ignore_ascii_case(n));
        headers.push((n.to_string(), v.to_string()));
    }
}

/// `.sub.` substitution: {{host}}, {{domains[]}} / {{domains[www]}} / [www1] / [www2],
/// {{ports[http][0]}}, {{ports[http][1]}}. Unknown keys are left as they are.
fn substitute(body: &[u8], request_host: &str, mount: &Mount) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    let mut rest: &str = &text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let key = after[..end].trim();
        let value = match key {
            "host" => Some(request_host.to_string()),
            "domains[]" => Some(mount.host.clone()),
            "domains[www]" | "domains[www1]" | "domains[www2]" => Some(format!("{}.{}", &key[8..key.len() - 1], mount.host)),
            "ports[http][0]" => Some(mount.port.to_string()),
            "ports[http][1]" => Some(mount.port.wrapping_add(1).to_string()),
            _ => None,
        };
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// `// META: key=value` lines at the top of a .js file, as wptserve's `js_meta_re`
/// (`//\s*META:\s*(\w*)=(.*)$`, matched at the line start) reads them: no indentation before
/// `//`, the value kept as written (trailing spaces too); the first other line ends the block.
fn read_meta(js: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in js.lines() {
        let Some(rest) = line.strip_prefix("//") else { break };
        let Some(rest) = rest.trim_start().strip_prefix("META:") else { break };
        let Some((k, v)) = rest.trim_start().split_once('=') else { break };
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            break;
        }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

/// Python's `template % {"meta": .., "script": .., "path": ..}`: ONE pass, so a value that
/// contains `%(path)s` is not expanded again.
fn fill(tmpl: &str, meta: &str, script: &str, path: &str) -> String {
    let mut out = String::with_capacity(tmpl.len() + meta.len() + script.len() + path.len());
    let mut rest = tmpl;
    while let Some(i) = rest.find("%(") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let (key, val) = [("%(meta)s", meta), ("%(script)s", script), ("%(path)s", path)].into_iter().find(|(k, _)| rest.starts_with(k)).expect("known template key");
        out.push_str(val);
        rest = &rest[key.len()..];
    }
    out.push_str(rest);
    out
}

const WINDOW_WRAPPER: &str = "<!doctype html>\n<meta charset=utf-8>\n%(meta)s\n<script src=\"/resources/testharness.js\"></script>\n<script src=\"/resources/testharnessreport.js\"></script>\n%(script)s\n<div id=log></div>\n<script src=\"%(path)s\"></script>\n";

const ANY_WRAPPER: &str = "<!doctype html>\n<meta charset=utf-8>\n%(meta)s\n<script>\nself.GLOBAL = {\n  isWindow: function() { return true; },\n  isWorker: function() { return false; },\n  isShadowRealm: function() { return false; },\n};\n</script>\n<script src=\"/resources/testharness.js\"></script>\n<script src=\"/resources/testharnessreport.js\"></script>\n%(script)s\n<div id=log></div>\n<script src=\"%(path)s\"></script>\n";

/// The generated `x.any.html` / `x.window.html` (wptserve's `AnyHtmlHandler` / `WindowHandler` in
/// tools/serve/serve.py; QWPT4a checked them at the pin): the template, `Content-Type: text/html`
/// plus the `.js` file's own `load_headers`, and `.sub.` substitution when the `.js` path has it.
fn wrapper(mount: &Mount, js_fs: &Path, url: &Url, html_suffix: &str, is_any: bool) -> MountResponse {
    let Ok(js) = std::fs::read_to_string(js_fs) else {
        return MountResponse::text(404, "Not Found", "cannot read script");
    };
    let req_path = url.path();
    let metadata = read_meta(&js);
    // `check_exposure`: the FIRST `global` line; "" = the default set (window + dedicatedworker);
    // otherwise only the item `window` itself gives a window variant.
    if is_any
        && let Some((_, g)) = metadata.iter().find(|(k, _)| k == "global")
        && !g.is_empty()
        && !g.split(',').any(|v| v.trim() == "window")
    {
        // `global=worker` and the like: there is no window variant.
        return MountResponse::text(404, "Not Found", "this .any.js has no window variant");
    }
    let (mut meta, mut script) = (Vec::new(), Vec::new());
    for (k, v) in &metadata {
        match k.as_str() {
            "timeout" if v == "long" => meta.push("<meta name=\"timeout\" content=\"long\">".to_string()),
            "title" => meta.push(format!("<title>{}</title>", v.replace('&', "&amp;").replace('<', "&lt;"))),
            "script" => script.push(format!("<script src=\"{}\"></script>", v.replace('&', "&amp;").replace('"', "&quot;"))),
            _ => {}
        }
    }
    let js_path = format!("{}{}", &req_path[..req_path.len() - html_suffix.len()], html_suffix.replace(".html", ".js"));
    let tmpl = if is_any { ANY_WRAPPER } else { WINDOW_WRAPPER };
    let mut body = fill(tmpl, &meta.join("\n"), &script.join("\n"), &js_path).into_bytes();
    if js_path.contains(".sub.") {
        body = substitute(&body, url.host_str().unwrap_or(""), mount);
    }
    let mut headers = vec![("Content-Type".to_string(), "text/html".to_string())];
    load_headers(&mut headers, js_fs, url, mount);
    MountResponse { status: 200, reason: "OK", headers, body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wire-mount-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("site/resources/webidl2/lib")).unwrap();
        fs::create_dir_all(d.join("site/dom")).unwrap();
        fs::write(d.join("secret.txt"), "TOP SECRET").unwrap();
        d
    }

    fn mounts(d: &Path) -> Mounts {
        let mut m = Mounts::default();
        m.add_mount(&format!("http://web-platform.test:8000={}", d.join("site").display())).unwrap();
        m
    }

    fn get(m: &Mounts, url: &str) -> MountResponse {
        let u = Url::parse(url).unwrap();
        let mount = m.find(&u).expect("mounted origin");
        m.serve(mount, "GET", &u)
    }

    fn hdr<'a>(r: &'a MountResponse, n: &str) -> Option<&'a str> {
        r.headers.iter().find(|(h, _)| h.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
    }

    #[test]
    fn origin_matching_with_aliases() {
        let d = tree("origin");
        let m = mounts(&d);
        for ok in [
            "http://web-platform.test:8000/x",
            "http://www.web-platform.test:8000/x",
            "http://www1.web-platform.test:8001/x",
            "http://www2.web-platform.test:8000/x",
        ] {
            assert!(m.find(&Url::parse(ok).unwrap()).is_some(), "{ok}");
        }
        for no in ["http://web-platform.test/x", "http://example.com:8000/x", "https://web-platform.test:8000/x", "http://evilweb-platform.test:8000/"] {
            assert!(m.find(&Url::parse(no).unwrap()).is_none(), "{no}");
        }
    }

    #[test]
    fn serves_files_with_types_headers_and_dir_headers() {
        let d = tree("files");
        let s = d.join("site");
        fs::write(s.join("dom/a.html"), "<p>a</p>").unwrap();
        fs::write(s.join("dom/a.html.headers"), "X-File: 1\nContent-Type: text/x-custom\n").unwrap();
        fs::write(s.join("dom/b.js"), "1").unwrap();
        fs::write(s.join("dom/__dir__.headers"), "X-Dir: yes\nX-File: 0\n").unwrap();
        fs::write(s.join("__dir__.headers"), "X-Root: r\n").unwrap();
        let m = mounts(&d);
        let r = get(&m, "http://web-platform.test:8000/dom/a.html?q=1");
        assert_eq!((r.status, r.body.as_slice()), (200, &b"<p>a</p>"[..]));
        assert_eq!(hdr(&r, "content-type"), Some("text/x-custom"));
        assert_eq!(hdr(&r, "x-file"), Some("1"));
        assert_eq!(hdr(&r, "x-dir"), Some("yes"));
        // QWPT4a: wptserve reads only the NEIGHBOURING __dir__.headers, not the ancestors'.
        assert_eq!(hdr(&r, "x-root"), None);
        fs::write(s.join("top.txt"), "t").unwrap();
        assert_eq!(hdr(&get(&m, "http://web-platform.test:8000/top.txt"), "x-root"), Some("r"));
        let r = get(&m, "http://web-platform.test:8000/dom/b.js");
        assert_eq!(hdr(&r, "content-type"), Some("text/javascript"));
        assert_eq!(hdr(&r, "x-dir"), Some("yes"));
        assert_eq!(hdr(&r, "x-file"), Some("0"));
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/a.html.headers").status, 404);
        assert_eq!(get(&m, "http://web-platform.test:8000/nope").status, 404);
    }

    #[test]
    fn path_traversal_is_impossible() {
        let d = tree("trav");
        fs::write(d.join("site/ok.txt"), "ok").unwrap();
        let m = mounts(&d);
        for p in ["/../secret.txt", "/%2e%2e/secret.txt", "/dom/../../secret.txt", "/dom/%2e%2e%2f%2e%2e%2fsecret.txt", "/..%2fsecret.txt", "/%2e%2e%5csecret.txt", "/dom/..%2F..%2Fsecret.txt"] {
            let url = Url::parse(&format!("http://web-platform.test:8000{p}")).unwrap();
            let mount = m.find(&url).unwrap();
            let r = m.serve(mount, "GET", &url);
            assert_ne!(r.body, b"TOP SECRET", "{p}");
            assert!(r.status != 200, "{p} -> {}", r.status);
        }
        // Raw (non-normalised) paths handed to the decoder directly.
        assert!(decode_path("/a/../../secret.txt").is_none());
        assert!(decode_path("/a/%2e%2e/b").is_none());
        assert!(decode_path("/a/b%2fc").is_none());
        assert_eq!(decode_path("/a//./b").unwrap(), ["a", "b"]);
        // A symlink pointing out of the mount is refused.
        std::os::unix::fs::symlink(d.join("secret.txt"), d.join("site/link.txt")).unwrap();
        let r = get(&m, "http://web-platform.test:8000/link.txt");
        assert_eq!(r.status, 403);
        assert_ne!(r.body, b"TOP SECRET");
        assert_eq!(get(&m, "http://web-platform.test:8000/ok.txt").status, 200);
    }

    #[test]
    fn any_and_window_wrappers() {
        let d = tree("wrap");
        let s = d.join("site/dom");
        fs::write(s.join("x.any.js"), "// META: title=A <b> & c\n// META: timeout=long\n// META: script=/resources/helper.js\n// META: script=a\"b\ntest(() => {}, 'x');\n// META: title=ignored\n").unwrap();
        fs::write(s.join("w.window.js"), "// META: title=W\ntest(()=>{});").unwrap();
        fs::write(s.join("worker.any.js"), "// META: global=worker\n").unwrap();
        fs::write(s.join("both.any.js"), "// META: global=window,worker\n").unwrap();
        let m = mounts(&d);
        let r = get(&m, "http://web-platform.test:8000/dom/x.any.html");
        assert_eq!(hdr(&r, "content-type"), Some("text/html"));
        let body = String::from_utf8(r.body).unwrap();
        assert!(body.starts_with("<!doctype html>\n<meta charset=utf-8>\n<title>A &lt;b> &amp; c</title>\n<meta name=\"timeout\" content=\"long\">\n<script>\nself.GLOBAL"), "{body}");
        assert!(body.contains("<script src=\"/resources/testharnessreport.js\"></script>\n<script src=\"/resources/helper.js\"></script>\n<script src=\"a&quot;b\"></script>\n<div id=log></div>\n<script src=\"/dom/x.any.js\"></script>\n"), "{body}");
        assert!(!body.contains("ignored"));
        let w = String::from_utf8(get(&m, "http://web-platform.test:8000/dom/w.window.html").body).unwrap();
        assert_eq!(
            w,
            "<!doctype html>\n<meta charset=utf-8>\n<title>W</title>\n<script src=\"/resources/testharness.js\"></script>\n<script src=\"/resources/testharnessreport.js\"></script>\n\n<div id=log></div>\n<script src=\"/dom/w.window.js\"></script>\n"
        );
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/worker.any.html").status, 404);
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/both.any.html").status, 200);
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/none.any.html").status, 404);
        // The .js itself is plain static content.
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/w.window.js").body, b"// META: title=W\ntest(()=>{});");
    }

    #[test]
    fn sub_substitution_alias_override_and_py() {
        let d = tree("misc");
        let s = d.join("site");
        fs::write(s.join("dom/s.sub.html"), "{{host}}|{{domains[www]}}|{{domains[www2]}}|{{domains[]}}|{{ports[http][0]}}|{{ports[http][1]}}|{{unknown}}|{{").unwrap();
        fs::write(s.join("dom/n.html"), "{{host}}").unwrap();
        fs::write(s.join("resources/webidl2/lib/webidl2.js"), "WEBIDL").unwrap();
        fs::write(s.join("resources/testharnessreport.js"), "ORIGINAL").unwrap();
        fs::write(s.join("dom/h.py"), "def main(): pass").unwrap();
        fs::write(d.join("report.js"), "OVERRIDE").unwrap();
        let mut m = mounts(&d);
        m.add_override(&format!("/resources/testharnessreport.js={}", d.join("report.js").display())).unwrap();
        let r = get(&m, "http://www.web-platform.test:8001/dom/s.sub.html");
        assert_eq!(String::from_utf8(r.body).unwrap(), "www.web-platform.test|www.web-platform.test|www2.web-platform.test|web-platform.test|8000|8001|{{unknown}}|{{");
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/n.html").body, b"{{host}}");
        assert_eq!(get(&m, "http://web-platform.test:8000/resources/WebIDLParser.js").body, b"WEBIDL");
        let r = get(&m, "http://web-platform.test:8000/resources/testharnessreport.js");
        assert_eq!((r.body.as_slice(), hdr(&r, "content-type")), (&b"OVERRIDE"[..], Some("text/javascript")));
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/h.py").status, 501);
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/missing.py").status, 404);
        let u = Url::parse("http://web-platform.test:8000/dom/n.html").unwrap();
        let mount = m.find(&u).unwrap();
        let head = m.serve(mount, "HEAD", &u);
        assert!(head.status == 200 && head.body.is_empty());
        assert_eq!(m.serve(mount, "POST", &u).status, 405);
    }

    /// QWPT4a: the points where QWPT3's from-memory wrappers differed from wptserve at the pin
    /// (tools/serve/serve.py, tools/wptserve/wptserve/handlers.py, tools/manifest/sourcefile.py).
    #[test]
    fn wptserve_parity_headers_sub_headers_and_meta() {
        let d = tree("parity");
        let s = d.join("site/dom");
        // The wrapper gets the .js file's __dir__.headers and x.any.js.headers.
        fs::write(s.join("__dir__.headers"), "X-Dir: d\n").unwrap();
        fs::write(s.join("h.any.js"), "test(()=>{});").unwrap();
        fs::write(s.join("h.any.js.headers"), "X-Js: j\n").unwrap();
        let m = mounts(&d);
        let r = get(&m, "http://web-platform.test:8000/dom/h.any.html");
        assert_eq!((hdr(&r, "content-type"), hdr(&r, "x-dir"), hdr(&r, "x-js")), (Some("text/html"), Some("d"), Some("j")));
        // <file>.sub.headers wins over <file>.headers and is substituted.
        fs::write(s.join("p.html"), "p").unwrap();
        fs::write(s.join("p.html.headers"), "X-Plain: 1\n").unwrap();
        fs::write(s.join("p.html.sub.headers"), "X-Host: {{host}}:{{ports[http][0]}}\n").unwrap();
        let r = get(&m, "http://www1.web-platform.test:8000/dom/p.html");
        assert_eq!((hdr(&r, "x-host"), hdr(&r, "x-plain")), (Some("www1.web-platform.test:8000"), None));
        // A .sub. wrapper resource is substituted too.
        fs::write(s.join("q.sub.window.js"), "// META: title={{host}}\n").unwrap();
        let w = String::from_utf8(get(&m, "http://web-platform.test:8000/dom/q.sub.window.html").body).unwrap();
        assert!(w.contains("<title>web-platform.test</title>") && w.contains("src=\"/dom/q.sub.window.js\""), "{w}");
        // META: indented lines are not metadata; values keep trailing spaces; one-pass template.
        fs::write(s.join("i.window.js"), "  // META: title=indented\ntest(()=>{});").unwrap();
        assert!(!String::from_utf8(get(&m, "http://web-platform.test:8000/dom/i.window.html").body).unwrap().contains("<title>"));
        assert_eq!(read_meta("// META: title=a b  \n"), [("title".to_string(), "a b  ".to_string())]);
        fs::write(s.join("t.window.js"), "// META: title=%(path)s\n").unwrap();
        assert!(String::from_utf8(get(&m, "http://web-platform.test:8000/dom/t.window.html").body).unwrap().contains("<title>%(path)s</title>"));
        // `global= ` (not empty, no `window` item) has no window variant; "" is the default set.
        fs::write(s.join("sp.any.js"), "// META: global= \n").unwrap();
        fs::write(s.join("em.any.js"), "// META: global=\n").unwrap();
        fs::write(s.join("sw.any.js"), "// META: global=sharedworker, window\n").unwrap();
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/sp.any.html").status, 404);
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/em.any.html").status, 200);
        assert_eq!(get(&m, "http://web-platform.test:8000/dom/sw.any.html").status, 200);
    }

    #[test]
    fn bad_specs_are_errors() {
        let mut m = Mounts::default();
        assert!(m.add_mount("http://x.test:8000").is_err());
        assert!(m.add_mount("https://x.test=/tmp").is_err());
        assert!(m.add_mount("http://x.test=/no/such/dir/xyz").is_err());
        assert!(m.add_override("nopath=/tmp").is_err());
        assert!(m.add_override("/a=/no/such/file").is_err());
    }
}
