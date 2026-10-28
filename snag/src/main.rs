use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Read as _;
use std::time::Duration;

use base64::Engine as _;
use html5ever::driver::{parse_document, ParseOpts};
use html5ever::tendril::TendrilSink;
use html5ever::{Attribute, LocalName, Namespace, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use url::Url;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: snag <url> <output.html>");
        std::process::exit(1);
    }
    let start_url = match Url::parse(&args[1]) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("snag: invalid URL {:?}: {e}", args[1]);
            std::process::exit(1);
        }
    };

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("snag/0.1 (+https://github.com/Loupax/vibe-it) scratchy test fixture bundler")
        .build();

    let html = match fetch_text(&agent, &start_url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("snag: failed to fetch {start_url}: {e}");
            std::process::exit(1);
        }
    };

    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("parsing a string can't fail with an I/O error");

    let base = find_base_href(&dom.document, &start_url).unwrap_or_else(|| start_url.clone());

    let mut visited_css = HashSet::new();
    walk(&dom.document, &base, &agent, &mut visited_css);

    let mut out = match std::fs::File::create(&args[2]) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("snag: failed to create {:?}: {e}", args[2]);
            std::process::exit(1);
        }
    };
    let serializable: SerializableHandle = dom.document.clone().into();
    html5ever::serialize::serialize(&mut out, &serializable, Default::default())
        .expect("writing to a just-created file shouldn't fail");
    eprintln!("snag: wrote {}", args[2]);
}

// ---------- fetching ----------

fn fetch_text(agent: &ureq::Agent, url: &Url) -> Result<String, String> {
    let resp = agent.get(url.as_str()).call().map_err(|e| e.to_string())?;
    resp.into_string().map_err(|e| e.to_string())
}

/// Fetches `url` and returns it as a `data:` URI (base64-encoded), or
/// `None` on any failure (unreachable, non-2xx, I/O error) — a missing
/// sub-resource shouldn't abort the whole bundle, it should just stay as
/// whatever the caller falls back to.
fn fetch_as_data_uri(agent: &ureq::Agent, url: &Url) -> Option<String> {
    let resp = agent.get(url.as_str()).call().ok()?;
    let content_type = resp.content_type().to_string();
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).ok()?;
    let mime = if content_type.is_empty() || content_type == "text/plain" {
        guess_mime(url.path())
    } else {
        content_type
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

fn guess_mime(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "ico" => "image/x-icon",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ---------- CSS text rewriting ----------
//
// Deliberately a hand-rolled text scanner, not a round-trip CSS parser —
// `cssparser` (already a dependency, reused from the html5ever/rcdom
// stack scratchy itself uses) is a parser-consumer, not a source-
// preserving rewriter, so reconstructing exact CSS text token-by-token
// would need its own serializer. A plain scan for `@import` and `url(`
// is simpler, and correct for the overwhelming majority of real-world
// CSS (including Wikipedia's, which is machine-generated/minified, not
// hand-crafted to break a scanner). Known, accepted gap: a comment or
// string literal that happens to *contain* the literal text `url(` or
// `@import` could be misdetected — not handled, since it's rare in
// practice and full comment/string-aware tokenizing is a lot of extra
// machinery for an edge case that hasn't actually come up.

/// Fetches and fully resolves one stylesheet's text: inlines `@import`s
/// (recursively, each resolved relative to *its own* URL, not the
/// original page's) and rewrites every remaining `url(...)` reference to
/// a `data:` URI. `visited` guards against re-inlining the same
/// stylesheet twice (duplicate `@import`s, or an import cycle).
fn process_css(css: &str, css_url: &Url, agent: &ureq::Agent, visited: &mut HashSet<String>) -> String {
    let after_imports = inline_imports(css, css_url, agent, visited);
    rewrite_urls(&after_imports, css_url, agent)
}

fn inline_imports(css: &str, base: &Url, agent: &ureq::Agent, visited: &mut HashSet<String>) -> String {
    let lower = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    loop {
        let Some(rel) = lower[i..].find("@import") else {
            out.push_str(&css[i..]);
            break;
        };
        let start = i + rel;
        out.push_str(&css[i..start]);
        let Some(semi_rel) = css[start..].find(';') else {
            // Malformed (no terminator) — keep the rest verbatim and stop.
            out.push_str(&css[start..]);
            break;
        };
        let stmt = &css[start + "@import".len()..start + semi_rel];
        i = start + semi_rel + 1;
        let Some(raw_url) = extract_import_url(stmt) else { continue };
        let Ok(target) = base.join(&raw_url) else { continue };
        if !visited.insert(target.to_string()) {
            continue; // already inlined this exact stylesheet — skip the dupe/cycle
        }
        if let Ok(text) = fetch_text(agent, &target) {
            out.push_str(&process_css(&text, &target, agent, visited));
        }
        // Fetch failure: drop the import silently, same as any other
        // unreachable sub-resource.
    }
    out
}

/// Pulls the URL out of an `@import` statement's body (everything after
/// `@import`, before the terminating `;`) — `url(X)`, `"X"`, or `'X'`,
/// optionally followed by a media-query condition that's just ignored
/// (scratchy doesn't support `@media` either, so there's nothing for it
/// to select between).
fn extract_import_url(stmt: &str) -> Option<String> {
    let rest = stmt.trim_start();
    if let Some(r) = rest.strip_prefix("url(").or_else(|| rest.strip_prefix("URL(")) {
        let end = r.find(')')?;
        return Some(unquote(r[..end].trim()));
    }
    if let Some(r) = rest.strip_prefix('"') {
        let end = r.find('"')?;
        return Some(r[..end].to_string());
    }
    if let Some(r) = rest.strip_prefix('\'') {
        let end = r.find('\'')?;
        return Some(r[..end].to_string());
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && (bytes[0] == b'"' || bytes[0] == b'\'') && bytes[bytes.len() - 1] == bytes[0] {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Rewrites every `url(...)` in `css` (background images, `@font-face`
/// `src`, ...) to a `data:` URI fetched relative to `base`. Leaves an
/// already-`data:` URI alone, and leaves a reference this file couldn't
/// fetch as its original (now probably-broken, but harmless — scratchy
/// just won't find anything to paint there) URL rather than dropping it.
fn rewrite_urls(css: &str, base: &Url, agent: &ureq::Agent) -> String {
    let lower = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    loop {
        let Some(rel) = lower[i..].find("url(") else {
            out.push_str(&css[i..]);
            break;
        };
        let start = i + rel;
        out.push_str(&css[i..start]);
        let args_start = start + "url(".len();
        let Some(close_rel) = find_url_close_paren(&css[args_start..]) else {
            out.push_str(&css[start..]);
            break;
        };
        let inner = unquote(css[args_start..args_start + close_rel].trim());
        i = args_start + close_rel + 1;
        let resolved = if inner.starts_with("data:") || inner.is_empty() {
            inner
        } else {
            match base.join(&inner) {
                Ok(target) => fetch_as_data_uri(agent, &target).unwrap_or(inner),
                Err(_) => inner,
            }
        };
        out.push_str("url(\"");
        out.push_str(&resolved);
        out.push_str("\")");
    }
    out
}

/// Finds the index of the `)` that closes a `url(...)` call, skipping
/// over a quoted argument's contents first (which could itself contain a
/// `)`, e.g. `url("a)b.png")`) — doesn't handle a backslash-escaped quote
/// inside the string, a narrow gap accepted for the same reason as the
/// module-level scanning limitations above.
fn find_url_close_paren(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
        let quote = bytes[i];
        i += 1;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
    }
    s[i..].find(')').map(|p| i + p)
}

// ---------- DOM walking ----------

enum NodeAction {
    Keep,
    Remove,
    Replace(Handle),
}

/// Walks `handle`'s children, replacing/removing/mutating nodes as
/// `process_node` decides, then recurses into whatever's kept. Builds a
/// fresh children `Vec` and swaps it in at the end rather than mutating
/// mid-iteration, so replacing a `<link>` with a `<style>` (or dropping a
/// `<script>`) at the same position can't conflict with the `RefCell`
/// borrow still iterating over the old list.
fn walk(handle: &Handle, base: &Url, agent: &ureq::Agent, visited_css: &mut HashSet<String>) {
    let children = handle.children.borrow().clone();
    let mut new_children = Vec::with_capacity(children.len());
    for child in children {
        match process_node(&child, base, agent, visited_css) {
            NodeAction::Remove => {}
            NodeAction::Replace(replacement) => new_children.push(replacement),
            NodeAction::Keep => {
                walk(&child, base, agent, visited_css);
                new_children.push(child);
            }
        }
    }
    *handle.children.borrow_mut() = new_children;
}

fn process_node(handle: &Handle, base: &Url, agent: &ureq::Agent, visited_css: &mut HashSet<String>) -> NodeAction {
    let NodeData::Element { name, attrs, .. } = &handle.data else {
        return NodeAction::Keep;
    };
    match name.local.as_ref() {
        // Never executed by scratchy (no JS engine) — stripped outright
        // rather than left as unused dead weight in the bundle.
        "script" => NodeAction::Remove,
        "link" => {
            let is_stylesheet = get_attr(attrs, "rel")
                .map(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")))
                .unwrap_or(false);
            if !is_stylesheet {
                return NodeAction::Keep;
            }
            let Some(href) = get_attr(attrs, "href") else { return NodeAction::Keep };
            let Ok(target) = base.join(&href) else { return NodeAction::Keep };
            if !visited_css.insert(target.to_string()) {
                return NodeAction::Remove; // duplicate stylesheet, already inlined once
            }
            match fetch_text(agent, &target) {
                Ok(text) => {
                    let processed = process_css(&text, &target, agent, visited_css);
                    NodeAction::Replace(make_style_node(&processed))
                }
                Err(_) => NodeAction::Remove, // unreachable — drop rather than leave a dead <link>
            }
        }
        "style" => {
            let text = collect_text(handle);
            let processed = process_css(&text, base, agent, visited_css);
            replace_text_children(handle, &processed);
            NodeAction::Keep
        }
        "img" => {
            if let Some(src) = get_attr(attrs, "src")
                && !src.starts_with("data:")
                    && let Ok(target) = base.join(&src)
                        && let Some(data_uri) = fetch_as_data_uri(agent, &target) {
                            set_attr(attrs, "src", &data_uri);
                        }
            // Unused until scratchy has a notion of responsive image
            // candidates — left in would just be clutter full of
            // relative URLs nothing will ever resolve.
            remove_attr(attrs, "srcset");
            NodeAction::Keep
        }
        "a" => {
            if let Some(href) = get_attr(attrs, "href")
                && let Ok(target) = base.join(&href) {
                    set_attr(attrs, "href", target.as_str());
                }
            NodeAction::Keep
        }
        _ => NodeAction::Keep,
    }
}

fn get_attr(attrs: &RefCell<Vec<Attribute>>, name: &str) -> Option<String> {
    attrs.borrow().iter().find(|a| a.name.local.as_ref() == name).map(|a| a.value.to_string())
}

fn set_attr(attrs: &RefCell<Vec<Attribute>>, name: &str, value: &str) {
    let mut attrs = attrs.borrow_mut();
    if let Some(a) = attrs.iter_mut().find(|a| a.name.local.as_ref() == name) {
        a.value = value.into();
    }
}

fn remove_attr(attrs: &RefCell<Vec<Attribute>>, name: &str) {
    attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
}

fn collect_text(handle: &Handle) -> String {
    let mut out = String::new();
    for child in handle.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            out.push_str(&contents.borrow());
        }
    }
    out
}

fn replace_text_children(handle: &Handle, text: &str) {
    let text_node = Node::new(NodeData::Text { contents: RefCell::new(text.into()) });
    *handle.children.borrow_mut() = vec![text_node];
}

fn make_style_node(css_text: &str) -> Handle {
    let name = QualName::new(None, Namespace::from(XHTML_NS), LocalName::from("style"));
    let node = Node::new(NodeData::Element {
        name,
        attrs: RefCell::new(Vec::new()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    let text_node = Node::new(NodeData::Text { contents: RefCell::new(css_text.into()) });
    node.children.borrow_mut().push(text_node);
    node
}

/// A `<base href>`, if the document has one — resolved against the
/// page's own URL in case it's itself relative (unusual, but valid).
/// First match wins (document order), matching the HTML spec.
fn find_base_href(handle: &Handle, page_url: &Url) -> Option<Url> {
    if let NodeData::Element { name, attrs, .. } = &handle.data
        && name.local.as_ref() == "base"
            && let Some(href) = get_attr(attrs, "href") {
                return Url::parse(&href).ok().or_else(|| page_url.join(&href).ok());
            }
    for child in handle.children.borrow().iter() {
        if let Some(u) = find_base_href(child, page_url) {
            return Some(u);
        }
    }
    None
}
