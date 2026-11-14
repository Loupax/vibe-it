use super::*;

// ---------- link navigation (v65, re-platformed onto `wire` in v66) ----------
//
// scratchy still performs zero networking itself (see NOTES.md's
// "rendering, not networking" priority, in force since v1) — clicking a
// link builds a real HTTP/1.1 request (absolute-form request line, see
// `build_get_request_text`) and hands it to a `wire` daemon over a Unix
// socket (see `wire/NOTES.md` for that daemon's own design — a separate
// crate in this same repo, not a dependency of this one), then swaps
// whatever normalized response comes back into the live `App`, same as
// `run()`'s own `Input::File` case builds the *initial* document. v65
// originally talked to `snag --serve` directly (a GET-only, bundle-
// shaped line protocol); that's gone now, replaced outright per
// `scratchy/TODO.txt`'s own "TOP PRIORITY" planning entry, which also
// covers *why*: `snag`'s own job is bundling a page to one file, not
// being scratchy's live HTTP client, and the two were already showing
// seams (no way for the old protocol to express a form's method/body).
// Everything in this section is plain, `App`-free logic so it can be
// unit tested without a real window/event loop, matching this file's
// existing "headless function-level testing" convention (see NOTES.md's
// "Working notes for next session" — no other feature here constructs
// a real winit `EventLoop`/`Window` inside `#[cfg(test)]` either).

/// A real click (mouse-down then mouse-up close to the same point) vs. a
/// drag (mouse-down, move, mouse-up elsewhere) — scratchy has no text
/// selection yet, so there's nothing a drag would otherwise *do*, but
/// binding navigation to "mouse released over a link" without this
/// check would still misfire the moment a drag just happens to start
/// and end over one (and the instant text selection exists, it would
/// also fire navigation mid-selection-drag, which is squarely wrong).
/// `threshold` is in document pixels — `down`/`up` are both whatever
/// consistent coordinate space the caller uses for both (document space
/// in `App`'s own use, see `WindowEvent::MouseInput`'s handler).
pub(crate) fn is_click(down: (f32, f32), up: (f32, f32), threshold: f32) -> bool {
    let dx = up.0 - down.0;
    let dy = up.1 - down.1;
    dx * dx + dy * dy <= threshold * threshold
}

/// Half a cosmic-text glyph's worth of slack — small enough that an
/// actual drag (even a short one) is never mistaken for a click, large
/// enough that ordinary human hand tremor between a real mouse-down and
/// mouse-up over the same link doesn't miss.
pub(crate) const CLICK_DRAG_THRESHOLD_PX: f32 = 5.0;

/// How long each tick of `App::spawn_shaping_worker`'s own background
/// loop lets `measure_document` run before yielding and reporting
/// whatever it has so far — originally bounded `App::redraw`'s own
/// progressive-layout branch directly, back when shaping ran inline on
/// the single-threaded main loop (winit) and every input event sat
/// unhandled for the entire budget if it arrived mid-tick; measured
/// then (on a 151-paragraph page, 80ms budget -> ~6-8 paragraphs/tick,
/// 58 ticks, 7.1s total; 500ms -> ~30-34 paragraphs/tick, ~12 ticks,
/// 6.2s total; on a much larger, RTL page, 16ms -> 306 ticks/17.0s,
/// 80ms -> 90 ticks/13.0s, 150ms -> 53 ticks/12.6s — diminishing
/// returns past ~80ms there already, since individual complex-script
/// paragraphs can themselves take hundreds of ms, capping how much a
/// larger budget can batch together regardless of this value). Once
/// `measure_document` moved to its own background thread, the reason
/// to keep this *small* (bounding input latency) went away — but
/// ticking at all turned out to still matter, just for a different
/// reason: it's what lets a big page fill in progressively again
/// (`App::shape_cache`'s own docs) instead of showing nothing until
/// the entire page is shaped, which running to completion in one call
/// (tried first, see TODO.txt and this constant's own git history)
/// made painfully visible on a real, large page (confirmed: real
/// Wikipedia went from progressively filling in to a blank page for
/// most of a minute). Still `80ms` — nothing about *this* value's own
/// tradeoff changed, only who's bound by it (a background thread's own
/// progress-reporting cadence now, not main-thread input latency).
pub(crate) const PROGRESSIVE_LAYOUT_TICK_BUDGET: std::time::Duration = std::time::Duration::from_millis(80);

/// Resolves a clicked `<a href>` into an absolute URL against the
/// page's own `<base href>` — ordinary URL-join logic via the `url`
/// crate (already a dependency of `snag`, not previously one of
/// scratchy's own — see `Cargo.toml`), not hand-rolled relative-path
/// math. `Url::join`'s real behavior already does the right thing for
/// an `href` that's already absolute (resolves to itself, `base`
/// ignored, same as a real browser) and for one that isn't (resolves
/// against `base`) — so the only genuinely different case this
/// function itself has to handle is `base` being `None` at all (no
/// `<base href>` anywhere in the current page — see `find_base_url`'s
/// own docs for when that happens), where a relative `href` has nothing
/// to resolve against and reports a clear error rather than guessing.
pub(crate) fn resolve_href(base: Option<&Url>, href: &str) -> Result<Url, String> {
    match base {
        Some(base) => base.join(href).map_err(|e| format!("cannot resolve link {href:?}: {e}")),
        None => Url::parse(href)
            .map_err(|_| format!("cannot resolve link {href:?}: no <base href> on this page to resolve it against")),
    }
}

/// Finds the document's own `<base href="...">`, if any, and parses it
/// — the resolution basis `resolve_href` needs. `snag` (v9) always
/// emits a real one reflecting the page's actual URL, in both its
/// one-shot CLI and `--serve` modes (see `snag/NOTES.md`'s own v9
/// entry); a hand-written local test fixture with no `<base>` at all
/// just means a *relative* link on it can't be resolved — reported as
/// an inline error by `resolve_href`, not guessed at — while an
/// already-absolute one still resolves fine regardless.
pub(crate) fn find_base_url(document: &Handle) -> Option<Url> {
    find_base_href(document).and_then(|h| Url::parse(&h).ok())
}

/// The base URL `AppEvent::NavResult`'s handler resolves a freshly-
/// fetched page against (v68): the page's own `<base href>`, if it
/// declares one, else `fetched_url` — the real address `wire` fetched
/// it from. Mirrors a real browser's own "document base URL" rule
/// (defaults to the document's own address when there's no `<base>`
/// element at all) and the equivalent `snag` already gives a bundled
/// page via a synthesized `<base href>` (v9) — `wire`-navigated pages
/// never had that until now. Before this fix, a fetched page with no
/// `<base>` of its own left `base_url` at `None`, which broke *every*
/// relative link on that page, including a same-page fragment link
/// (found live: MediaWiki's own `#bodyContent` "jump to content" skip-
/// link, on an otherwise ordinary page with no explicit `<base>` tag —
/// the overwhelmingly common real-world case, not an edge case). Pulled
/// out as its own pure function specifically so this exact regression
/// has a permanent test, not just a one-line inline fix nobody can
/// re-verify later.
pub(crate) fn resolve_page_base_url(document: &Handle, fetched_url: Url) -> Url {
    find_base_url(document).unwrap_or(fetched_url)
}

/// Same recursive shape as `find_html_element` — walks the whole tree
/// (not just `<head>`) so a malformed/fragment document still finds a
/// `<base>` wherever it actually ended up, rather than silently falling
/// back to "none found" the moment it's outside where it belongs.
pub(crate) fn find_base_href(handle: &Handle) -> Option<String> {
    if let NodeData::Element { name, .. } = &handle.data {
        if name.local.as_ref() == "base" {
            if let Some(href) = element_attr(handle, "href") {
                return Some(href.to_string());
            }
        }
    }
    for child in handle.children.borrow().iter() {
        if let Some(found) = find_base_href(child) {
            return Some(found);
        }
    }
    None
}

/// Minimal HTML-escaping for `App::show_message`'s synthetic `<p>...
/// </p>` wrapper — `text` can carry a clicked `href` or an OS/`io::
/// Error` message, neither of which scratchy controls the contents of,
/// so a stray `<`/`&`/`>` needs to display literally rather than
/// corrupt the synthetic markup it's being spliced into.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Builds a real HTTP/1.1 request, in the standard proxy absolute-form
/// shape (RFC 7230 §5.3.2) `wire` itself parses (see `wire/src/main.rs`'s
/// own `parse_request`) — the request line carries the *whole* URL, not
/// just a path, since `wire` sits behind a bare Unix socket with no
/// DNS/TLS-SNI of its own to infer a destination from. GET-only for now
/// (no body, no method parameter) — form submission needs a real
/// `<button>`/focus/keyboard subsystem scratchy doesn't have yet (see
/// `scratchy/TODO.txt`'s own "Form elements — sequencing plan" item);
/// this is purely what a clicked `<a href>` needs.
pub(crate) fn build_get_request_text(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    format!("GET {url} HTTP/1.1\r\nHost: {host_header}\r\n\r\n")
}

/// Splits a raw response buffer into its `Content-Type` (if any) and
/// body — `wire` always frames its responses with a real `Content-
/// Length` and never `Transfer-Encoding: chunked` (see `wire/NOTES.md`),
/// so there's no chunked-response decoding to do here; this just finds
/// the blank-line header terminator and reads one header out of the
/// block before it, per the contract `scratchy/TODO.txt`'s own "TOP
/// PRIORITY" item settled: scratchy reads `Content-Type` and nothing
/// else — every other header (`Cache-Control`, `Server`, cookies, ...)
/// is deliberately left unparsed.
pub(crate) fn parse_wire_response(bytes: &[u8]) -> Result<(Option<String>, Vec<u8>), String> {
    let header_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed response from wire: no header terminator found".to_string())?;
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "malformed response from wire: headers are not valid UTF-8".to_string())?;
    let content_type = header_text.split("\r\n").skip(1 /* status line */).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-type").then(|| value.trim().to_string())
    });
    let body = bytes[header_end + 4..].to_vec();
    Ok((content_type, body))
}

/// Wraps already-decoded image bytes (`wire` never forwards a
/// `Content-Encoding`, so these are the real final bytes) in a minimal
/// synthetic HTML document, reusing the `<img src="data:...">` decoding
/// path `decode_data_uri`/`decode_image_bytes` already implement for
/// `<img>` tags — rather than inventing a second, parallel "the whole
/// document is just an image" rendering mode. `mime` is expected to
/// already be one `dispatch_wire_response` has recognized (`image/png`/
/// `jpeg`/`gif`/`webp`).
pub(crate) fn wrap_image_as_html(mime: &str, bytes: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(r#"<html><body><img src="data:{mime};base64,{encoded}"></body></html>"#).into_bytes()
}

/// The response-framing contract `scratchy/TODO.txt`'s own "TOP
/// PRIORITY" item settled on: `text/html` goes straight into the
/// existing HTML-parse path untouched; a real decoded image gets
/// wrapped (`wrap_image_as_html`) so it still reaches scratchy through
/// that same one path, never a second rendering mode; anything else is a
/// clear, named "can't render this" error — never a guess at forcing
/// unknown bytes through either path. The `;charset=...`/`; boundary=...`
/// parameter half of a real `Content-Type`, if any, is dropped before
/// matching — only the MIME type itself decides.
pub(crate) fn dispatch_wire_response(content_type: Option<String>, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let content_type = content_type.unwrap_or_default();
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match mime.as_str() {
        "text/html" => Ok(decode_html_bytes(&body, content_type_charset(&content_type)).into_bytes()),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => Ok(wrap_image_as_html(&mime, &body)),
        _ => Err(format!("can't render this content type: {mime:?}")),
    }
}

/// What connecting to `wire` and reading one full response back came out
/// to — the lowest-level outcome, shared by the top-level page fetch
/// (`start_navigation`) and every sub-resource fetch (v67's `fetch_
/// stylesheet_via_wire`/`fetch_image_via_wire`): either a real response
/// genuinely came back (status is deliberately not kept at all — see
/// `parse_wire_response`'s own docs on why only `Content-Type` ever
/// mattered), or nothing did. Kept distinct from a plain `Result` for
/// the same reason `start_navigation` itself needs it for the top-level
/// page: "never reached the daemon at all" is the one case with
/// something useful to do about it (the stdout-fallback print, for the
/// top-level page only — a sub-resource fetch has nothing worth printing
/// since the user never directly asked for *it*).
pub(crate) enum WireFetchRaw {
    Responded { content_type: Option<String>, body: Vec<u8> },
    Unreachable,
}

/// Connects to `path`, sends `request_text` verbatim, reads the whole
/// response until `wire` closes the connection (its own protocol's
/// "done" signal — no length prefix, no explicit end marker, same shape
/// `snag --serve`'s older line protocol already used), then parses just
/// far enough to hand the caller `Content-Type` and the raw body —
/// dispatching *what that means* (HTML? an image to wrap? CSS? an
/// unsupported type?) is each caller's own job, since the top-level page
/// and a sub-resource fetch each need a different answer to that
/// question (see `dispatch_wire_response`/`fetch_stylesheet_via_wire`/
/// `fetch_image_via_wire`). Any failure before a complete response was
/// read back (connect refused, write failed, connection reset mid-read)
/// collapses to `Unreachable` — from scratchy's point of view those are
/// all exactly the same outcome: no usable response came back, for
/// whatever reason.
pub(crate) fn fetch_raw_via_wire(path: &std::path::Path, request_text: &str) -> WireFetchRaw {
    let Ok(mut stream) = UnixStream::connect(path) else { return WireFetchRaw::Unreachable };
    if stream.write_all(request_text.as_bytes()).is_err() {
        return WireFetchRaw::Unreachable;
    }
    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).is_err() {
        return WireFetchRaw::Unreachable;
    }
    match parse_wire_response(&buf) {
        Ok((content_type, body)) => WireFetchRaw::Responded { content_type, body },
        Err(_) => WireFetchRaw::Unreachable,
    }
}

/// `start_navigation`'s own stdout fallback — per `scratchy/TODO.txt`'s
/// own design, printed verbatim as the literal HTTP/1.1 wire text
/// (request line, `Host`, blank line) specifically so it's directly
/// replayable by `nc`/a raw socket/a manually-run `wire` instance, with
/// zero translation, whenever no `--socket` was given at all or the
/// configured one couldn't be reached.
pub(crate) fn print_fallback_request(request_text: &str) {
    print!("{request_text}");
    let _ = io::stdout().flush();
}

/// `SCRATCHY_TIMING=1 scratchy ...` prints one line per navigation's
/// own page fetch and, separately, its external-stylesheet fetches —
/// same measure-don't-guess habit (and the same env-var-gated, zero-
/// cost-when-off shape) `snag`'s own `SNAG_TIMING`/`wire`'s own `WIRE_
/// TIMING` already established, added here to see how much of the
/// "Loading…" wait is the page fetch itself (`start_navigation`'s own
/// `fetch_raw_via_wire` call — real network time, now that `wire`
/// streams rather than buffers, plus whatever's left of DNS/connect/TLS
/// on a cold host) versus its own stylesheets, fetched synchronously
/// one at a time before the very first layout can happen at all (see
/// `process_external_resources`'s own docs). Shaping itself isn't
/// measured here — it already has its own background-worker
/// architecture and diagnostics (`measure_mixed_inline`'s own, in
/// `mod tests`).
pub(crate) fn timing_enabled() -> bool {
    std::env::var_os("SCRATCHY_TIMING").is_some()
}

// ---------- sub-resources: external stylesheets and images (v67) ----------
//
// The same `wire` daemon the top-level page fetch already talks to is
// reused for a page's own `<link rel=stylesheet href>`/non-`data:`
// `<img src>` references — scratchy still performs zero networking of
// its own beyond that one daemon. Nothing here is `snag`'s job to
// duplicate: `snag`'s own inlining only ever applies to a page bundled
// *ahead of time* via its CLI; a page reached by clicking a live link
// never goes through `snag` at all (see `scratchy/TODO.txt`'s own "TOP
// PRIORITY" item), so scratchy has to do this inlining-equivalent work
// itself for exactly that path.

/// Mime-checked low-level fetch; returns the matched MIME and body.
pub(crate) fn fetch_sub_resource_typed(path: &std::path::Path, url: &Url, accept: &[&str]) -> Option<(String, Vec<u8>)> {
    let request_text = build_get_request_text(url);
    let WireFetchRaw::Responded { content_type, body } = fetch_raw_via_wire(path, &request_text) else { return None };
    let content_type = content_type.unwrap_or_default();
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if !accept.contains(&mime.as_str()) {
        return None;
    }
    Some((mime, body))
}

/// Decodes stylesheet bytes: BOM, then the `Content-Type` header's
/// `charset=`, then a leading `@charset "<label>";` rule, then UTF-8.
pub(crate) fn decode_css_bytes(bytes: &[u8], header: Option<&'static encoding_rs::Encoding>) -> String {
    let enc = if let Some((enc, _)) = encoding_rs::Encoding::for_bom(bytes) {
        enc
    } else if let Some(enc) = header {
        enc
    } else {
        let head = &bytes[..bytes.len().min(1024)];
        let at = b"@charset \"";
        head.strip_prefix(&at[..])
            .and_then(|rest| rest.iter().position(|&b| b == b'"').map(|i| &rest[..i]))
            .and_then(encoding_rs::Encoding::for_label)
            .filter(|e| e.is_ascii_compatible())
            .unwrap_or(encoding_rs::UTF_8)
    };
    enc.decode_with_bom_removal(bytes).0.into_owned()
}

/// Fetches one external stylesheet — `None` on any failure (unreachable
/// or not real CSS). Decoded per `decode_css_bytes` (Q8 follow-up: no
/// longer UTF-8 only).
pub(crate) fn fetch_stylesheet_via_wire(path: &std::path::Path, url: &Url) -> Option<String> {
    let request_text = build_get_request_text(url);
    let WireFetchRaw::Responded { content_type, body } = fetch_raw_via_wire(path, &request_text) else { return None };
    let content_type = content_type.unwrap_or_default();
    if !content_type.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("text/css") {
        return None;
    }
    Some(decode_css_bytes(&body, content_type_charset(&content_type)))
}

/// Maximum `@import` nesting depth `expand_css_imports` follows.
pub(crate) const MAX_IMPORT_DEPTH: usize = 8;

/// Splits the leading `@charset`/`@import` rules off `css`. Returns the
/// imports as `(target, media)` pairs and the remaining stylesheet text.
pub(crate) fn split_leading_imports(css: &str) -> (Vec<(String, String)>, &str) {
    let mut rest = css.trim_start_matches('\u{feff}');
    let mut imports = Vec::new();
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("/*") {
            match r.find("*/") {
                Some(i) => {
                    rest = &r[i + 2..];
                    continue;
                }
                None => return (imports, ""),
            }
        }
        let lower = rest.get(..8).map(|s| s.to_ascii_lowercase());
        let is_import = lower.as_deref() == Some("@import ") || rest.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("@import\"") || s.eq_ignore_ascii_case("@import\n") || s.eq_ignore_ascii_case("@import\t"));
        let is_charset = rest.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("@charset"));
        if !is_import && !is_charset {
            return (imports, rest);
        }
        let Some(end) = find_rule_end(rest) else { return (imports, "") };
        let body = rest[if is_import { 7 } else { 8 }..end].trim();
        rest = &rest[end + 1..];
        if is_charset {
            continue;
        }
        if let Some((target, media)) = parse_import_prelude(body) {
            imports.push((target, media));
        }
    }
}

/// Index of the `;` ending the at-rule at the start of `s`, skipping
/// quoted strings and parentheses.
pub(crate) fn find_rule_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ';' => return Some(i),
            None => {}
        }
    }
    None
}

/// `url(x) screen` / `"x" screen` -> (`x`, `screen`). `None` for forms we
/// don't follow (`supports(...)`, `layer(...)`, an empty target).
pub(crate) fn parse_import_prelude(body: &str) -> Option<(String, String)> {
    let body = body.trim();
    let (target, rest) = if body.len() >= 4 && body[..4].eq_ignore_ascii_case("url(") {
        let close = body.find(')')?;
        (body[4..close].trim().trim_matches(|c| c == '"' || c == '\'').to_string(), &body[close + 1..])
    } else if let Some(q) = body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = body[1..].find(q)? + 1;
        (body[1..end].to_string(), &body[end + 1..])
    } else {
        return None;
    };
    let mut media = rest.trim();
    let lower = media.to_ascii_lowercase();
    if lower.contains("supports(") || lower.contains("layer(") {
        return None;
    }
    if lower == "layer" {
        media = "";
    } else if let Some(m) = lower.strip_prefix("layer ") {
        media = media[media.len() - m.len()..].trim();
    }
    if target.is_empty() {
        return None;
    }
    Some((target, media.to_string()))
}

/// Expands the leading `@import` rules of a fetched stylesheet in place
/// (Q18). Each import is resolved against `sheet_url`, fetched through
/// `fetch`, expanded recursively (depth <= `MAX_IMPORT_DEPTH`, cycles
/// skipped) and inserted where the `@import` was, wrapped in `@media`
/// when it has a media list. `stack` holds the URLs being expanded.
pub(crate) fn expand_css_imports(
    css: &str,
    sheet_url: &Url,
    stack: &mut Vec<Url>,
    fetch: &mut dyn FnMut(&Url) -> Option<String>,
) -> String {
    let (imports, rest) = split_leading_imports(css);
    let mut out = String::new();
    if stack.len() < MAX_IMPORT_DEPTH {
        for (target, media) in imports {
            let Ok(url) = sheet_url.join(&target) else { continue };
            if url == *sheet_url || stack.contains(&url) {
                continue;
            }
            let Some(text) = fetch(&url) else { continue };
            stack.push(url.clone());
            let expanded = expand_css_imports(&text, &url, stack, fetch);
            stack.pop();
            if media.is_empty() {
                out.push_str(&expanded);
            } else {
                out.push_str(&format!("@media {media} {{\n{expanded}\n}}"));
            }
            out.push('\n');
        }
    }
    out.push_str(rest);
    out
}

/// Fetches and decodes one external image — `None` on any failure
/// (unreachable, not a supported image content type, or bytes `image`
/// itself can't decode). Reuses `decode_image_bytes` verbatim — the
/// exact same decoder `resolve_img` already uses for a `data:` URI
/// `src`, so a live-fetched image and an inlined one decode identically.
pub(crate) fn fetch_image_via_wire(path: &std::path::Path, url: &Url) -> Option<DecodedImage> {
    let (mime, body) =
        fetch_sub_resource_typed(path, url, &["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml"])?;
    if mime == "image/svg+xml" {
        // Trust the wire Content-Type even when the bytes lack a
        // recognisable prefix.
        return decode_svg_bytes(&body);
    }
    decode_image_bytes(&body)
}

/// Every external stylesheet/image reference a page's own markup names,
/// exactly as written (unresolved — resolving against the page's base
/// URL is each caller's own job, done once both lists are complete, not
/// per-element here).
#[derive(Default)]
pub(crate) struct ExternalResources {
    /// Each `<link rel=stylesheet href="...">`'s own `href`, in document
    /// order.
    pub(crate) stylesheets: Vec<String>,
    /// Each non-`data:` `<img src="...">`'s own `src` — a `data:` URI is
    /// already fully inlined, so it's deliberately excluded here (it
    /// needs no fetch, and `resolve_img` never consults `image_cache`
    /// for one anyway).
    pub(crate) images: Vec<String>,
}

/// Walks the whole tree (not just `<head>`, same "a malformed document
/// might put it anywhere" latitude `find_base_href` already takes)
/// collecting every stylesheet `<link>`/non-`data:` `<img>` reference.
/// `rel` is matched the same space-separated, case-insensitive way a
/// real `rel` attribute's token list always is (a `<link>` can legally
/// carry more than one keyword, e.g. `rel="stylesheet alternate"`).
pub(crate) fn find_external_resources(handle: &Handle, out: &mut ExternalResources) {
    if let NodeData::Element { name, .. } = &handle.data {
        match name.local.as_ref() {
            "link" => {
                let is_stylesheet = element_attr(handle, "rel")
                    .map(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")))
                    .unwrap_or(false);
                if is_stylesheet
                    && let Some(href) = element_attr(handle, "href")
                {
                    out.stylesheets.push(href.to_string());
                }
            }
            "img" => {
                if let Some(src) = element_attr(handle, "src")
                    && !src.starts_with("data:")
                {
                    out.images.push(src.to_string());
                }
            }
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        find_external_resources(child, out);
    }
}

/// The one function that turns a freshly-fetched page's raw HTML bytes
/// into both halves of v67's sub-resource support: every external
/// stylesheet, fetched synchronously (right here, on `start_navigation`'s
/// own background thread) and concatenated into the returned `String`
/// (`App::extra_css` — CSS has to be fully in hand *before* the page's
/// very first layout, so this can't be deferred the way images can);
/// and every non-`data:` image, each fired off as its own independent
/// background thread (`thread::spawn`, mirroring `wire`'s own explicit
/// "concurrent *separate* connections are expected" design) reporting
/// back through `AppEvent::ImageFetched` whenever it finishes — this
/// function itself returns immediately after *starting* those, long
/// before any of them resolve.
///
/// Parses `html` into its own throwaway `Handle` purely to walk it with
/// `find_external_resources` and `find_base_url` — that DOM is never
/// touched again once this function returns and, critically, never
/// crosses a thread boundary itself (a `Handle` is `Rc`-based, so it
/// can't: `Rc<T>` is `!Send`). Only plain owned `String`s (a stylesheet's
/// fetched text, an image's own raw `src` key) and a plain `DecodedImage`
/// struct (no `Rc` of its own — see `AppEvent::ImageFetched`'s own docs)
/// ever actually travel back across the channel.
pub(crate) fn process_external_resources(
    proxy: &EventLoopProxy<AppEvent>,
    path: &std::path::Path,
    page_url: &Url,
    html: &[u8],
    generation: u64,
) -> String {
    let Ok(dom) = new_parser().read_from(&mut &html[..]) else { return String::new() };
    // The fetched page's own `<base href>`, if it declares one, wins —
    // same priority `resolve_href`'s every other caller already gives
    // an explicit `<base>` over the document's own URL.
    let base = find_base_url(&dom.document).unwrap_or_else(|| page_url.clone());
    let mut resources = ExternalResources::default();
    find_external_resources(&dom.document, &mut resources);
    let extra_css = fetch_stylesheets(path, &base, &resources.stylesheets);
    spawn_image_fetches(proxy, path, &base, resources.images, generation);
    extra_css
}

/// Fetch (and `@import`-expand) every stylesheet href against `base`,
/// concatenated in document order.
pub(crate) fn fetch_stylesheets(path: &std::path::Path, base: &Url, hrefs: &[String]) -> String {
    let mut extra_css = String::new();
    for href in hrefs {
        let Ok(url) = resolve_href(Some(base), href) else { continue };
        if let Some(css) = fetch_stylesheet_via_wire(path, &url) {
            let mut stack = vec![url.clone()];
            let css = expand_css_imports(&css, &url, &mut stack, &mut |u| fetch_stylesheet_via_wire(path, u));
            extra_css.push_str(&css);
            extra_css.push('\n');
        }
    }
    extra_css
}

/// One background thread per distinct image `src` (a real page commonly
/// repeats the same icon URL many times; one fetch and one `image_cache`
/// entry keyed by the raw `src` covers every occurrence).
pub(crate) fn spawn_image_fetches(proxy: &EventLoopProxy<AppEvent>, path: &std::path::Path, base: &Url, images: Vec<String>, generation: u64) {
    let mut seen = HashSet::new();
    for src in images {
        if !seen.insert(src.clone()) {
            continue;
        }
        let Ok(url) = resolve_href(Some(base), &src) else { continue };
        let path = path.to_path_buf();
        let proxy = proxy.clone();
        thread::spawn(move || {
            let image = fetch_image_via_wire(&path, &url);
            let _ = proxy.send_event(AppEvent::ImageFetched(generation, src, image));
        });
    }
}

/// Q22b: sub-resources for a streamed (stdin) page, started at EOF when
/// the whole document (`<head>` included) finally exists. The base is
/// the page's own `<base href>` (what `snag` synthesizes for a piped
/// page); with none there is nothing to resolve relative references
/// against, so nothing is fetched. Unlike a file page, the stylesheets
/// are fetched on a background thread (the window is already up and the
/// first layout has happened) and arrive as `AppEvent::StreamCss`.
pub(crate) fn start_stream_resources(proxy: &EventLoopProxy<AppEvent>, socket: &std::path::Path, document: &Handle, generation: u64) {
    let Some(base) = find_base_url(document) else { return };
    let mut resources = ExternalResources::default();
    find_external_resources(document, &mut resources);
    spawn_image_fetches(proxy, socket, &base, resources.images, generation);
    if resources.stylesheets.is_empty() {
        return;
    }
    let (proxy, socket, hrefs) = (proxy.clone(), socket.to_path_buf(), resources.stylesheets);
    thread::spawn(move || {
        let css = fetch_stylesheets(&socket, &base, &hrefs);
        let _ = proxy.send_event(AppEvent::StreamCss(generation, css));
    });
}

/// `file://<absolute path>` for a page opened from a local file (Q22):
/// the base URL its relative `src`/`href`s resolve against, and what
/// `wire` is asked for (it serves `file://`). `None` if the path can't
/// be made absolute.
pub(crate) fn file_page_url(path: &str) -> Option<Url> {
    Url::from_file_path(fs::canonicalize(path).ok()?).ok()
}

/// Q22: a statically opened file with `--socket` given gets its own
/// base URL and its external stylesheets/images fetched through `wire`
/// exactly like a live-navigated page (`process_external_resources`).
/// Runs synchronously before the window exists, so the stylesheets are
/// in hand for the first layout; images arrive later through
/// `AppEvent::ImageFetched` (generation 0, the app's initial one).
pub(crate) fn load_file_resources(proxy: &EventLoopProxy<AppEvent>, socket: &std::path::Path, path: &str, html: &[u8]) -> (Option<Url>, String) {
    let Some(url) = file_page_url(path) else { return (None, String::new()) };
    let css = process_external_resources(proxy, socket, &url, html, 0);
    (Some(url), css)
}
