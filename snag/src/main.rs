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

/// Two modes, both writing the bundled result to stdout (classic Unix
/// filter behavior — compose with `>`/`|` rather than an output-path
/// argument):
///   `snag <url>`  — fetches `<url>` itself and bundles it.
///   `snag`        — reads HTML from stdin instead of fetching the top
///                   page (e.g. `curl -s <url> | snag | scratchy`).
///                   Sub-resources (stylesheets/images) are still fetched
///                   over the network either way — only the *top-level*
///                   page's own fetch is skipped in this mode. Since
///                   there's no URL argument to resolve relative links
///                   against, one has to come from the document itself:
///                   `<base href>` if present, else `<link
///                   rel="canonical">` (which real sites, including
///                   MediaWiki, always emit as an absolute URL). No usable
///                   base at all → hard error, since silently leaving
///                   every relative link unresolved would produce a
///                   "bundle" that's barely different from the raw page.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 {
        eprintln!("usage: snag [url]");
        eprintln!("  snag <url>   fetch <url>, bundle it, write to stdout");
        eprintln!("  snag         bundle HTML read from stdin instead of fetching it");
        eprintln!("               (relative links resolve against <base href> or");
        eprintln!("               <link rel=canonical>); e.g. curl -s <url> | snag");
        std::process::exit(1);
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("snag/0.1 (+https://github.com/Loupax/vibe-it) scratchy test fixture bundler")
        .build();

    let (explicit_url, html_bytes) = if let Some(arg) = args.get(1) {
        let url = match Url::parse(arg) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("snag: invalid URL {arg:?}: {e}");
                std::process::exit(1);
            }
        };
        let text = match fetch_text(&agent, &url) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("snag: failed to fetch {url}: {e}");
                std::process::exit(1);
            }
        };
        (Some(url), text.into_bytes())
    } else {
        let mut bytes = Vec::new();
        if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
            eprintln!("snag: failed to read stdin: {e}");
            std::process::exit(1);
        }
        (None, bytes)
    };

    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html_bytes.as_slice())
        .expect("parsing a byte stream can't fail with an I/O error");

    let Some(base) = determine_base(&dom.document, explicit_url.as_ref()) else {
        eprintln!("snag: no URL given and the page has neither <base href> nor");
        eprintln!("      <link rel=canonical> — can't resolve its relative links");
        std::process::exit(1);
    };

    let mut visited_css = HashSet::new();
    walk(&dom.document, &base, &agent, &mut visited_css);

    let stdout = std::io::stdout();
    let serializable: SerializableHandle = dom.document.clone().into();
    html5ever::serialize::serialize(stdout.lock(), &serializable, Default::default())
        .expect("writing to stdout shouldn't fail");
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
                && let Some(data_uri) = fetch_as_data_uri(agent, &target)
            {
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
                && let Ok(target) = base.join(&href)
            {
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

/// The base URL every relative link in the document resolves against.
/// `<base href>` always wins when present (resolved against
/// `explicit_url`, if there is one, in case the `<base href>` is itself
/// relative — unusual but valid); otherwise `explicit_url` itself (the
/// `snag <url>` case); otherwise `<link rel="canonical">` — the only
/// option left once there's neither an explicit URL nor a `<base>` tag
/// (the `snag` *-reading-stdin* case), and real sites emit it as an
/// absolute URL, so no further fallback is attempted against it.
fn determine_base(dom_root: &Handle, explicit_url: Option<&Url>) -> Option<Url> {
    if let Some(href) = find_tag_href(dom_root, "base") {
        if let Ok(u) = Url::parse(&href) {
            return Some(u);
        }
        if let Some(p) = explicit_url
            && let Ok(u) = p.join(&href)
        {
            return Some(u);
        }
    }
    if let Some(p) = explicit_url {
        return Some(p.clone());
    }
    let href = find_canonical(dom_root)?;
    Url::parse(&href).ok()
}

/// First `<TAG href="...">`'s `href`, in document order.
fn find_tag_href(handle: &Handle, tag: &str) -> Option<String> {
    if let NodeData::Element { name, attrs, .. } = &handle.data
        && name.local.as_ref() == tag
        && let Some(href) = get_attr(attrs, "href")
    {
        return Some(href);
    }
    for child in handle.children.borrow().iter() {
        if let Some(h) = find_tag_href(child, tag) {
            return Some(h);
        }
    }
    None
}

/// First `<link rel="canonical" href="...">`'s `href`, in document order.
fn find_canonical(handle: &Handle) -> Option<String> {
    if let NodeData::Element { name, attrs, .. } = &handle.data
        && name.local.as_ref() == "link"
        && get_attr(attrs, "rel")
            .is_some_and(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("canonical")))
        && let Some(href) = get_attr(attrs, "href")
    {
        return Some(href);
    }
    for child in handle.children.borrow().iter() {
        if let Some(h) = find_canonical(child) {
            return Some(h);
        }
    }
    None
}
