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

/// QY19: longest delay (seconds) a `<meta http-equiv=refresh>` redirect is
/// honoured with; larger values are clamped to this.
pub(crate) const META_REFRESH_MAX_DELAY: u64 = 10;
/// QY19: most chained meta redirects followed for one user navigation.
pub(crate) const META_REFRESH_MAX_CHAIN: u8 = 3;

/// QY19: parses a `<meta http-equiv=refresh content="...">` value into
/// `(delay seconds, target URL)`. Only `url=` redirects are honoured: a bare
/// `N` (reload the same page), a missing/negative number or an empty URL
/// give `None`, so a page can never make scratchy reload itself in a loop.
/// The delay is clamped to `META_REFRESH_MAX_DELAY`. Accepts `N; url=x`,
/// `N, URL = 'x'`, `N;x` (HTML spec), quotes optional, resolved against `base`.
pub(crate) fn parse_meta_refresh(content: &str, base: Option<&Url>) -> Option<(u64, Url)> {
    let s = content.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let delay: u64 = s[..digits].parse().unwrap_or(u64::MAX);
    let mut rest = &s[digits..];
    // Optional fractional part is ignored.
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ';' || c == ',');
    let mut url_part = rest;
    if rest.len() >= 3 && rest.is_char_boundary(3) && rest[..3].eq_ignore_ascii_case("url") {
        let after = rest[3..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        if let Some(a) = after.strip_prefix('=') {
            url_part = a.trim_start_matches(|c: char| c.is_ascii_whitespace());
        }
    }
    let url_part = match url_part.chars().next() {
        Some(q @ ('"' | '\'')) => {
            let inner = &url_part[1..];
            inner.split(q).next().unwrap_or("")
        }
        _ => url_part,
    };
    let url_part = url_part.trim();
    if url_part.is_empty() {
        return None;
    }
    let url = resolve_href(base, url_part).ok()?;
    Some((delay.min(META_REFRESH_MAX_DELAY), url))
}

/// QY19: the `content` of the first `<meta http-equiv=refresh>` (value
/// matched case-insensitively) in the tree. With scripts off, `<noscript>`
/// children are real elements (see `new_parser`), so one inside it is
/// found; with scripts on, noscript content is raw text and is not.
pub(crate) fn find_meta_refresh_content(handle: &Handle) -> Option<String> {
    if let NodeData::Element { name, .. } = &handle.data {
        if name.local.as_ref() == "meta"
            && element_attr(handle, "http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh"))
        {
            if let Some(c) = element_attr(handle, "content") {
                return Some(c.to_string());
            }
        }
    }
    for child in handle.children.borrow().iter() {
        if let Some(found) = find_meta_refresh_content(child) {
            return Some(found);
        }
    }
    None
}

/// QY19: what to do for a freshly loaded document: `(delay seconds, URL)`,
/// or `None` when it has no usable meta refresh or `chain` redirects have
/// already been followed for this navigation.
pub(crate) fn meta_refresh_action(document: &Handle, base: Option<&Url>, chain: u8) -> Option<(u64, Url)> {
    if chain >= META_REFRESH_MAX_CHAIN {
        return None;
    }
    parse_meta_refresh(&find_meta_refresh_content(document)?, base)
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
    build_get_request_text_with(url, user_agent_state())
}

/// `build_get_request_text` with an explicit User-Agent override (QY39b).
pub(crate) fn build_get_request_text_with(url: &Url, ua: &UaOverride) -> String {
    let host = url.host_str().unwrap_or("");
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let ua_line = ua.header().map(|(n, v)| format!("{n}: {v}\r\n")).unwrap_or_default();
    format!("GET {url} HTTP/1.1\r\nHost: {host_header}\r\n{ua_line}\r\n")
}

/// QF14b: `build_get_request_text` plus the cookie headers of `ctx` (the ONE
/// builder is `wire_cookie_headers`; empty for a `file:` page or an old wire).
pub(crate) fn build_get_request_ctx(url: &Url, ctx: &CookieContext) -> String {
    let mut text = build_get_request_text(url);
    let extra: String = wire_cookie_headers(ctx).iter().map(|(n, v)| format!("{n}: {v}\r\n")).collect();
    // Insert before the terminating blank line.
    text.truncate(text.len() - 2);
    text.push_str(&extra);
    text.push_str("\r\n");
    text
}

/// QF14b: true when a request head asks for the jar (or is a control request).
pub(crate) fn request_uses_jar(head: &[u8]) -> bool {
    let end = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(head.len());
    String::from_utf8_lossy(&head[..end]).to_ascii_lowercase().lines().any(|l| l.starts_with(&format!("{}:", WIRE_HEADER_JAR.to_ascii_lowercase())))
}

/// QF14b: learns cookie support from a response to a jar request; logs the
/// one-time "unavailable" line.
pub(crate) fn note_cookie_response(support: &CookieSupport, request_head: &[u8], version: Option<&str>, cookies: Option<&str>) {
    if request_uses_jar(request_head) {
        if let Some(line) = support.observe(version, cookies) {
            diag!("{line}");
        }
    }
}

/// Reads one header value out of a raw response head.
fn raw_response_header(bytes: &[u8], name: &str) -> Option<String> {
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&bytes[..end]).ok()?;
    head.split("\r\n").skip(1).find_map(|l| {
        let (n, v) = l.split_once(':')?;
        n.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
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
        "text/plain" => Ok(wrap_text_as_html(&decode_text_bytes(&body, content_type_charset(&content_type)))),
        _ => Err(format!("can't render this content type: {mime:?}")),
    }
}

/// Decodes a `text/plain` body: the `charset` parameter if recognised,
/// else UTF-8 (lossy). No `<meta>` sniffing — it isn't HTML.
fn decode_text_bytes(bytes: &[u8], header: Option<&'static encoding_rs::Encoding>) -> String {
    header.unwrap_or(encoding_rs::UTF_8).decode_with_bom_removal(bytes).0.into_owned()
}

/// Wraps plain text in a synthetic HTML document: an escaped `<pre>`
/// (whitespace preserved, monospace from the UA sheet, long lines wrap
/// so wire's one-line error bodies stay readable). Same "everything
/// reaches the one HTML path" idea as `wrap_image_as_html`.
pub(crate) fn wrap_text_as_html(text: &str) -> Vec<u8> {
    // A leading newline right after `<pre>` is dropped by the HTML parser.
    let lead = if text.starts_with('\n') { "\n" } else { "" };
    format!(r#"<html><body><pre style="white-space:pre-wrap">{lead}{}</pre></body></html>"#, escape_html(text)).into_bytes()
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
    Responded { content_type: Option<String>, body: Vec<u8>, final_url: Option<String> },
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
    fetch_raw_via_wire_with_timeout(path, request_text, RequestPolicy::default().legacy_idle_timeout)
}

/// `fetch_raw_via_wire` with an explicit per-read (idle) timeout: a hung
/// upstream (measured: 169.254.169.254) used to pin the thread forever.
/// A timeout collapses to `Unreachable` like every other failure.
pub(crate) fn fetch_raw_via_wire_with_timeout(path: &std::path::Path, request_text: &str, read_timeout: std::time::Duration) -> WireFetchRaw {
    fetch_raw_via_wire_with_support(path, request_text, read_timeout, cookie_support())
}

pub(crate) fn fetch_raw_via_wire_with_support(
    path: &std::path::Path,
    request_text: &str,
    read_timeout: std::time::Duration,
    support: &CookieSupport,
) -> WireFetchRaw {
    fetch_raw_detailed(path, request_text, read_timeout, support).unwrap_or(WireFetchRaw::Unreachable)
}

/// `fetch_raw_via_wire_with_support` that keeps WHY it failed (QWU1): the
/// `Err` text is the OS error for a connect/IO failure (`No such file or
/// directory, os error 2` = no socket file, `Connection refused, os error
/// 111` = stale socket file), or a short description. `Ok` is always
/// `WireFetchRaw::Responded`.
fn fetch_raw_detailed(
    path: &std::path::Path,
    request_text: &str,
    read_timeout: std::time::Duration,
    support: &CookieSupport,
) -> Result<WireFetchRaw, String> {
    let mut stream = UnixStream::connect(path).map_err(|e| describe_os_error(&e))?;
    let _ = stream.set_read_timeout(Some(read_timeout));
    stream.write_all(request_text.as_bytes()).map_err(|e| describe_os_error(&e))?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| describe_os_error(&e))?;
    match parse_wire_response(&buf) {
        Ok((content_type, body)) => {
            let version = raw_response_header(&buf, "x-wire-version");
            let cookies = raw_response_header(&buf, "x-wire-cookies");
            note_cookie_response(support, request_text.as_bytes(), version.as_deref(), cookies.as_deref());
            note_ua_response(raw_response_header(&buf, "x-wire-features").as_deref());
            let final_url = raw_response_header(&buf, "x-wire-final-url");
            Ok(WireFetchRaw::Responded { content_type, body, final_url })
        }
        Err(_) => Err("unreadable reply from wire".to_string()),
    }
}

/// `io::Error` as `<text>, os error N` (the std Display is
/// `<text> (os error N)`, which would nest parentheses in the QWU1 line).
pub(crate) fn describe_os_error(e: &io::Error) -> String {
    let t = e.to_string();
    match t.strip_suffix(')').and_then(|x| x.rsplit_once(" (os error ")) {
        Some((text, n)) => format!("{text}, os error {n}"),
        None => t,
    }
}

/// A sub-resource fetch (stylesheet, script, image, iframe): same as
/// `fetch_raw_via_wire`, but a failure is counted for the QWU1 "wire not
/// reachable" line of the current page load.
pub(crate) fn fetch_raw_sub(path: &std::path::Path, request_text: &str) -> WireFetchRaw {
    match fetch_raw_detailed(path, request_text, RequestPolicy::default().legacy_idle_timeout, cookie_support()) {
        Ok(r) => r,
        Err(e) => {
            note_wire_skip(Some(path), &e);
            WireFetchRaw::Unreachable
        }
    }
}

// ---------- QWU1: "wire not reachable" line ----------
//
// One stderr line per page load when sub-resources could not be fetched
// because wire could not be reached (before, a page just looked broken).
// Approach (the entry allowed "print at the first failure and update" OR
// "print once the load settles"; this is the second): every failed
// sub-resource fetch (stylesheet, @import, script, image, CSS image,
// iframe, page-JS fetch/XHR, media) calls `note_wire_skip`; the first one
// of a load starts a watcher thread which prints the line once no new
// failure arrived for `WIRE_QUIET_MS` (the load has settled). Failures
// are instant (ENOENT/ECONNREFUSED), so a quiet period is a reliable
// "settled" signal. Stragglers after the line are still counted but not
// printed (one line per load). `begin_wire_load(generation)` (called when
// a navigation starts and for a static page opened with `--socket`)
// resets the count; failures only count after a load has begun, so
// `file://` pages without `--socket` (no wire needed) never print.
// Page JS and media report their own errors; they are counted here too
// and not printed twice. A fetch from a previous navigation that is
// still running when a new one begins counts toward the new load (the
// fetch helpers carry no generation); harmless since both failed for the
// same reason.
#[derive(Default)]
struct WireDown {
    armed: bool,
    generation: u64,
    count: usize,
    first: Option<(String, String)>,
    reported: bool,
    watcher: bool,
    /// Bumped by every `begin_wire_load`: a watcher thread of an earlier load
    /// (same generation number, e.g. two loads of generation 1) must not
    /// report the new load's count (QY61).
    epoch: u64,
}

static WIRE_DOWN: std::sync::Mutex<WireDown> = std::sync::Mutex::new(WireDown { armed: false, generation: 0, count: 0, first: None, reported: false, watcher: false, epoch: 0 });
static WIRE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WIRE_QUIET_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(400);
static WIRE_CAPTURE: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// Starts a page load: resets the count and arms the tracker.
pub(crate) fn begin_wire_load(generation: u64) {
    #[cfg(test)]
    if !wire_test_scope_allows_begin() {
        return;
    }
    if let Ok(mut g) = WIRE_DOWN.lock() {
        let epoch = WIRE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *g = WireDown { armed: true, generation, epoch, ..WireDown::default() };
    }
}

/// Counts one sub-resource skipped because wire could not be reached.
pub(crate) fn note_wire_skip(path: Option<&std::path::Path>, err: &str) {
    #[cfg(test)]
    if !wire_test_scope_allows_skip(path) {
        return;
    }
    let Ok(mut g) = WIRE_DOWN.lock() else { return };
    if !g.armed {
        return;
    }
    g.count += 1;
    if g.first.is_none() {
        let target = path.map_or_else(|| "no --socket given".to_string(), |p| format!("at {}", p.display()));
        g.first = Some((target, err.to_string()));
    }
    if !g.watcher && !g.reported {
        g.watcher = true;
        let (generation, epoch) = (g.generation, g.epoch);
        thread::spawn(move || loop {
            let before = WIRE_DOWN.lock().map_or(0, |g| g.count);
            thread::sleep(std::time::Duration::from_millis(WIRE_QUIET_MS.load(std::sync::atomic::Ordering::Relaxed)));
            let Ok(mut g) = WIRE_DOWN.lock() else { return };
            if g.generation != generation || g.epoch != epoch || g.reported {
                return;
            }
            if g.count == before {
                report_wire_down(&mut g);
                return;
            }
        });
    }
}

fn report_wire_down(g: &mut WireDown) {
    if g.reported || g.count == 0 {
        return;
    }
    g.reported = true;
    let (target, err) = g.first.clone().unwrap_or_default();
    let line = if target.starts_with("at ") {
        format!("scratchy: wire not reachable {target} ({err}): {} sub-resources skipped", g.count)
    } else {
        format!("scratchy: wire not reachable, {target}: {} sub-resources skipped", g.count)
    };
    if let Ok(mut c) = WIRE_CAPTURE.lock()
        && let Some(v) = c.as_mut()
    {
        v.push(line);
        return;
    }
    diag!("{line}");
}

/// Prints the line now if any skip is pending (what the watcher does after
/// the quiet period). For tests.
#[cfg(test)]
pub(crate) fn flush_wire_report() {
    if let Ok(mut g) = WIRE_DOWN.lock() {
        report_wire_down(&mut g);
    }
}

// QY61: the tracker is process-global and `cargo test` runs tests in parallel.
// Other tests (JS fetch/XHR, load pipeline) fail wire fetches against bogus
// sockets and start loads, which landed in a QWU1 test's count. Once a QWU1
// test has set up, only the QWU1 test threads may begin a load, and only
// skips on a `scratchy-test-qwu1*` socket (or the explicit no-socket skip)
// count.
#[cfg(test)]
static WIRE_TEST_SCOPE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
thread_local! {
    static WIRE_TEST_OWNER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn wire_test_scope_allows_begin() -> bool {
    !WIRE_TEST_SCOPE.load(std::sync::atomic::Ordering::SeqCst) || WIRE_TEST_OWNER.with(std::cell::Cell::get)
}

#[cfg(test)]
fn wire_test_scope_allows_skip(path: Option<&std::path::Path>) -> bool {
    !WIRE_TEST_SCOPE.load(std::sync::atomic::Ordering::SeqCst)
        || path.is_none_or(|p| p.to_string_lossy().contains("scratchy-test-qwu1"))
}

#[cfg(test)]
pub(crate) fn wire_report_test_setup(quiet_ms: u64) {
    WIRE_TEST_SCOPE.store(true, std::sync::atomic::Ordering::SeqCst);
    WIRE_TEST_OWNER.with(|o| o.set(true));
    wire_report_test_disarm();
    WIRE_QUIET_MS.store(quiet_ms, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut c) = WIRE_CAPTURE.lock() {
        *c = Some(Vec::new());
    }
}

#[cfg(test)]
pub(crate) fn wire_report_test_disarm() {
    if let Ok(mut g) = WIRE_DOWN.lock() {
        *g = WireDown { epoch: WIRE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1, ..WireDown::default() };
    }
}

#[cfg(test)]
pub(crate) fn wire_report_test_take() -> Vec<String> {
    WIRE_CAPTURE.lock().ok().and_then(|mut c| c.as_mut().map(std::mem::take)).unwrap_or_default()
}

// ---------- QF5: full wire codec for page-JS networking ----------
//
// Pure building blocks (nothing calls them yet; QF7/QF8 will). What wire
// does today (read from wire/src/main.rs): any method; request headers
// forwarded except Host/Content-Length/Connection; request bodies read by
// Content-Length (no chunked requests); redirects followed by ureq; the
// response is `HTTP/1.1 <status> <reason>` + headers with NO Content-Length
// and the body read until the connection closes; transport failures become a
// `502` `text/plain` body starting `wire: upstream request failed:`.
// Duplicate response headers (Set-Cookie) are collapsed by wire today and
// the final URL / an error marker are not reported: QF6.

#[allow(unused_imports)]
pub(crate) use wire_codec::*;

#[allow(dead_code)] // nothing calls the codec until QF7/QF8
mod wire_codec {
use super::*;

/// Limits and timeouts for one request through the new path.
#[derive(Debug, Clone)]
pub(crate) struct WireLimits {
    pub max_body_bytes: usize,
    pub connect_timeout: std::time::Duration,
    /// Total time allowed for sending the request and reading the whole response.
    pub read_timeout: std::time::Duration,
}

impl Default for WireLimits {
    fn default() -> Self {
        RequestPolicy::default().wire_limits()
    }
}

impl RequestPolicy {
    pub(crate) fn wire_limits(&self) -> WireLimits {
        WireLimits { max_body_bytes: self.max_response_bytes, connect_timeout: self.connect_timeout, read_timeout: self.read_timeout }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetError {
    Connect(String),
    Timeout,
    Io(String),
    TooLarge,
    Malformed(String),
    /// A header name/value or method the codec refuses to serialise.
    BadRequest(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Connect(e) => write!(f, "cannot connect to wire: {e}"),
            NetError::Timeout => write!(f, "request timed out"),
            NetError::Io(e) => write!(f, "network I/O error: {e}"),
            NetError::TooLarge => write!(f, "response is too large"),
            NetError::Malformed(e) => write!(f, "malformed response from wire: {e}"),
            NetError::BadRequest(e) => write!(f, "bad request: {e}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct WireResponse {
    pub status: u16,
    pub status_text: String,
    /// Lowercased names, trimmed values, wire order, duplicates kept
    /// (`set-cookie` stays here: the cookie jar, QF14, needs it).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Set when this is wire's synthesized transport-failure response (the
    /// `X-Wire-Error` header of QF6, or today's 502 text/plain marker).
    pub error: Option<String>,
    /// `X-Wire-Final-Url` (QF6), when wire sends it.
    pub final_url: Option<String>,
}

impl WireResponse {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn is_header_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Serialises one absolute-form HTTP/1.1 request for wire. Host,
/// Content-Length, Connection and Transfer-Encoding in `headers` are dropped
/// (wire/ureq own them). `Content-Length` is added for a non-empty body and
/// for an empty POST/PUT/PATCH. CR/LF/NUL in a name or value, a non-token
/// name or method are rejected. Header policy (forbidden names, `X-Wire-*`)
/// is the caller's job: this codec is trusted and lets `X-Wire-Policy` through.
pub(crate) fn build_wire_request(method: &str, url: &Url, headers: &[(String, String)], body: &[u8]) -> Result<Vec<u8>, NetError> {
    build_wire_request_with(method, url, headers, body, user_agent_state())
}

/// `build_wire_request` with an explicit User-Agent override (QY39b): adds
/// `X-Wire-User-Agent` exactly once when `ua` has one.
pub(crate) fn build_wire_request_with(method: &str, url: &Url, headers: &[(String, String)], body: &[u8], ua: &UaOverride) -> Result<Vec<u8>, NetError> {
    if !is_header_token(method) {
        return Err(NetError::BadRequest(format!("invalid method {method:?}")));
    }
    let host = url.host_str().unwrap_or("");
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let mut out = format!("{method} {url} HTTP/1.1\r\nHost: {host_header}\r\n").into_bytes();
    let ua_header = ua.header();
    if let Some((n, v)) = &ua_header {
        out.extend_from_slice(format!("{n}: {v}\r\n").as_bytes());
    }
    for (name, value) in headers {
        if ua_header.is_some() && name.eq_ignore_ascii_case(WIRE_HEADER_USER_AGENT) {
            continue;
        }
        if !is_header_token(name) {
            return Err(NetError::BadRequest(format!("invalid header name {name:?}")));
        }
        if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(NetError::BadRequest(format!("header {name} value contains CR, LF or NUL")));
        }
        if matches!(name.to_ascii_lowercase().as_str(), "host" | "content-length" | "connection" | "transfer-encoding") {
            continue;
        }
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    if !body.is_empty() || matches!(method.to_ascii_uppercase().as_str(), "POST" | "PUT" | "PATCH") {
        out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    Ok(out)
}

/// Parses wire's full response (status line, headers, body to EOF).
pub(crate) fn parse_wire_response_full(bytes: &[u8]) -> Result<WireResponse, NetError> {
    let header_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| NetError::Malformed("no header terminator found".into()))?;
    let head = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(NetError::Malformed(format!("bad status line {status_line:?}")));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|s| (100..600).contains(s))
        .ok_or_else(|| NetError::Malformed(format!("bad status code in {status_line:?}")))?;
    let status_text = parts.next().unwrap_or("").trim().to_string();
    let mut headers = Vec::new();
    let (mut wire_error, mut final_url) = (None, None);
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| NetError::Malformed(format!("bad header line {line:?}")))?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
        match name.as_str() {
            "x-wire-error" => wire_error = Some(value),
            "x-wire-final-url" => final_url = Some(value),
            _ => headers.push((name, value)),
        }
    }
    let body = bytes[header_end + 4..].to_vec();
    let mut error = wire_error;
    // Today's wire (before QF6): the synthesized 502 text/plain body.
    if error.is_none() && status == 502 && body.starts_with(b"wire: upstream request failed:") {
        error = Some(String::new());
    }
    if let Some(e) = error.as_mut() {
        *e = String::from_utf8_lossy(&body).trim().to_string();
    }
    Ok(WireResponse { status, status_text, headers, body, error, final_url })
}

/// Sends the request bytes to wire and reads the whole response with a
/// connect timeout, a total read deadline and a body cap (over the cap ->
/// `NetError::TooLarge`, without buffering more).
pub(crate) fn fetch_full_via_wire(path: &std::path::Path, request: &[u8], limits: &WireLimits) -> Result<WireResponse, NetError> {
    use std::time::Instant;
    // Unix connect() has no timeout API: do it on a thread.
    let (tx, rx) = mpsc::channel();
    let p = path.to_path_buf();
    thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(p));
    });
    let mut stream = match rx.recv_timeout(limits.connect_timeout) {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let d = describe_os_error(&e);
            note_wire_skip(Some(path), &d);
            return Err(NetError::Connect(e.to_string()));
        }
        Err(_) => return Err(NetError::Timeout),
    };
    let deadline = Instant::now() + limits.read_timeout;
    let remaining = |d: Instant| d.checked_duration_since(Instant::now()).filter(|r| !r.is_zero());
    let io_err = |e: io::Error| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => NetError::Timeout,
        _ => NetError::Io(e.to_string()),
    };
    stream.set_write_timeout(remaining(deadline)).map_err(io_err)?;
    stream.write_all(request).map_err(io_err)?;
    // The head is not part of the body cap; allow it generous room.
    let cap = limits.max_body_bytes.saturating_add(64 * 1024);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let left = remaining(deadline).ok_or(NetError::Timeout)?;
        stream.set_read_timeout(Some(left)).map_err(io_err)?;
        let n = stream.read(&mut chunk).map_err(io_err)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > cap {
            return Err(NetError::TooLarge);
        }
    }
    let resp = parse_wire_response_full(&buf)?;
    note_cookie_response(cookie_support(), request, resp.header("x-wire-version"), resp.header("x-wire-cookies"));
    note_ua_response(resp.header("x-wire-features"));
    if resp.body.len() > limits.max_body_bytes {
        return Err(NetError::TooLarge);
    }
    Ok(resp)
}
} // mod wire_codec

/// `start_navigation`'s own stdout fallback — per `scratchy/TODO.txt`'s
/// own design, printed verbatim as the literal HTTP/1.1 wire text
/// (request line, `Host`, blank line) specifically so it's directly
/// replayable by `nc`/a raw socket/a manually-run `wire` instance, with
/// zero translation, whenever no `--socket` was given at all or the
/// configured one couldn't be reached.
pub(crate) fn print_fallback_request(request_text: &str) {
    write_raw_stdout(format_args!("{request_text}"));
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
pub(crate) fn fetch_sub_resource_typed(path: &std::path::Path, url: &Url, top: &Url, accept: &[&str]) -> Option<(String, Vec<u8>)> {
    let request_text = build_get_request_ctx(url, &CookieContext::subresource(top));
    let WireFetchRaw::Responded { content_type, body, .. } = fetch_raw_sub(path, &request_text) else { return None };
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
pub(crate) fn fetch_stylesheet_via_wire(path: &std::path::Path, url: &Url, top: &Url) -> Option<String> {
    let request_text = build_get_request_ctx(url, &CookieContext::subresource(top));
    let WireFetchRaw::Responded { content_type, body, .. } = fetch_raw_sub(path, &request_text) else { return None };
    let content_type = content_type.unwrap_or_default();
    if !content_type.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("text/css") {
        return None;
    }
    Some(decode_css_bytes(&body, content_type_charset(&content_type)))
}

/// Fetches one external script (Q33a). `None` on failure or an HTML
/// response (an error page is not a script).
pub(crate) fn fetch_script_via_wire(path: &std::path::Path, url: &Url, top: &Url) -> Option<String> {
    let request_text = build_get_request_ctx(url, &CookieContext::subresource(top));
    let WireFetchRaw::Responded { content_type, body, .. } = fetch_raw_sub(path, &request_text) else { return None };
    let content_type = content_type.unwrap_or_default();
    if content_type.trim_start().to_ascii_lowercase().starts_with("text/html") {
        return None;
    }
    let enc = content_type_charset(&content_type).unwrap_or(encoding_rs::UTF_8);
    Some(enc.decode_with_bom_removal(&body).0.into_owned())
}

/// QA18b1: bytes cap for a pre-fetched iframe document.
const IFRAME_MAX_BYTES: usize = 1 << 20;
/// QA18b1: at most this many iframes per page are pre-fetched.
const IFRAME_MAX_COUNT: usize = 8;

static IFRAME_PAGES: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

/// QA18b1: the pre-fetched HTML of an iframe, by absolute URL (what
/// `iframe.contentDocument` parses into its second document).
pub(crate) fn iframe_page(url: &str) -> Option<String> {
    IFRAME_PAGES.lock().ok()?.as_ref()?.get(url).cloned()
}

pub(crate) fn set_iframe_pages(pages: HashMap<String, String>) {
    if let Ok(mut g) = IFRAME_PAGES.lock() {
        *g = Some(pages);
    }
}

/// Same origin as `base` (all `file:` URLs count as one origin).
fn same_frame_origin(base: &Url, url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" => url.origin() == base.origin(),
        "file" => base.scheme() == "file",
        _ => false,
    }
}

/// QA18b1: fetches the `src` documents of the page's `<iframe>`s that are
/// same-origin http(s)/file, concurrently, bounded by `IFRAME_MAX_COUNT`
/// and `IFRAME_MAX_BYTES`, and stores them for the JS engine (which
/// cannot touch the network). Scripts in them never run. Replaces the
/// previous page's set. No-op with JS off.
pub(crate) fn prefetch_iframes(path: &std::path::Path, base: &Url, top: &Url, document: &Handle) {
    set_iframe_pages(HashMap::new());
    if !js_enabled() {
        return;
    }
    fn collect(h: &Handle, base: &Url, out: &mut Vec<Url>) {
        if let NodeData::Element { name, .. } = &h.data
            && name.local.as_ref() == "iframe"
            && let Some(src) = element_attr(h, "src")
            && let Ok(u) = resolve_href(Some(base), src.trim())
            && same_frame_origin(base, &u)
            && !out.contains(&u)
            && out.len() < IFRAME_MAX_COUNT
        {
            out.push(u);
        }
        for c in h.children.borrow().iter() {
            collect(c, base, out);
        }
    }
    let mut urls = Vec::new();
    collect(document, base, &mut urls);
    if urls.is_empty() {
        return;
    }
    let pages: HashMap<String, String> = thread::scope(|scope| {
        let hs: Vec<_> = urls
            .into_iter()
            .map(|u| {
                scope.spawn(move || {
                    let WireFetchRaw::Responded { content_type, body, .. } = fetch_raw_sub(path, &build_get_request_ctx(&u, &CookieContext::subresource(top))) else { return None };
                    let ct = content_type.unwrap_or_default().to_ascii_lowercase();
                    if body.len() > IFRAME_MAX_BYTES || !(ct.trim_start().starts_with("text/html") || ct.trim().is_empty() && u.scheme() == "file") {
                        return None;
                    }
                    Some((u.to_string(), decode_html_bytes(&body, None)))
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
    });
    set_iframe_pages(pages);
}

/// What `process_external_resources` hands back: the concatenated
/// stylesheets and the scripts ready to run, in document order.
pub(crate) struct PageResources {
    pub(crate) css: String,
    /// QY54b1: what `css` is made of.
    pub(crate) sheets: crate::link_css::SheetParts,
    pub(crate) scripts: Vec<ScriptSource>,
}

/// Maximum `@import` nesting depth `expand_css_imports` follows.
pub(crate) const MAX_IMPORT_DEPTH: usize = 8;

/// Splits the leading `@charset`/`@import` rules off `css`. Returns the
/// imports as `(target, media)` pairs and the remaining stylesheet text.
pub(crate) fn split_leading_imports(css: &str) -> (Vec<(String, String, Option<String>)>, &str) {
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
        if let Some((target, media, layer)) = parse_import_prelude(body) {
            imports.push((target, media, layer));
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

/// `url(x) layer(a.b) screen` / `"x" screen` -> (`x`, `screen`, layer).
/// The layer is `Some(name)` for `layer(name)` and `Some("")` for a bare
/// `layer` (QY17: the importer wraps the sheet in `@layer`). `None` for
/// forms we don't follow (`supports(...)`, an empty target).
pub(crate) fn parse_import_prelude(body: &str) -> Option<(String, String, Option<String>)> {
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
    let mut layer = None;
    if lower.starts_with("layer(") {
        let close = media.find(')')?;
        layer = Some(media[6..close].split_whitespace().collect::<String>());
        media = media[close + 1..].trim();
    } else if lower == "layer" || lower.starts_with("layer ") {
        layer = Some(String::new());
        media = media[5..].trim();
    }
    if media.to_ascii_lowercase().contains("supports(") || target.is_empty() {
        return None;
    }
    Some((target, media.to_string(), layer))
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
        for (target, media, layer) in imports {
            if let Some(text) = decode_data_uri_text(target.trim()) {
                let expanded = expand_css_imports(&text, sheet_url, stack, fetch);
                out.push_str(&wrap_import(expanded, &media, layer.as_deref()));
                out.push('\n');
                continue;
            }
            let Ok(url) = sheet_url.join(&target) else { continue };
            if url == *sheet_url || stack.contains(&url) {
                continue;
            }
            let Some(text) = fetch(&url) else { continue };
            stack.push(url.clone());
            let expanded = expand_css_imports(&text, &url, stack, fetch);
            stack.pop();
            out.push_str(&wrap_import(expanded, &media, layer.as_deref()));
            out.push('\n');
        }
    }
    out.push_str(&absolutize_css_urls(rest, sheet_url));
    out
}

/// Wraps an imported sheet's text in `@media` (when it has a media list)
/// and `@layer name`/`@layer` (QY17, `layer(name)` / bare `layer`).
fn wrap_import(expanded: String, media: &str, layer: Option<&str>) -> String {
    let mut text = expanded;
    if !media.is_empty() {
        text = format!("@media {media} {{\n{text}\n}}");
    }
    if let Some(name) = layer {
        text = format!("@layer{}{name} {{\n{text}\n}}", if name.is_empty() { "" } else { " " });
    }
    text
}

/// Q35a: rewrites every relative `url(...)` in a fetched stylesheet's text
/// to an absolute one against the sheet's own URL, so a `background-image`
/// or `mask-image` reference means the same thing once the text is
/// concatenated into the page's one stylesheet (where the resolution base
/// would otherwise be lost). `data:`, absolute, protocol-relative-free
/// (`//host` is resolved too), fragment-only (`#id`) and empty references
/// are left alone, as is everything when the sheet URL is not a base
/// (`data:`).
pub(crate) fn absolutize_css_urls(css: &str, sheet_url: &Url) -> String {
    if sheet_url.cannot_be_a_base() {
        return css.to_string();
    }
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut last = 0;
    while i + 4 <= bytes.len() {
        let is_url = bytes[i..i + 4].eq_ignore_ascii_case(b"url(")
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-' || bytes[i - 1] == b'_'));
        if !is_url {
            i += 1;
            continue;
        }
        let start = i + 4;
        let mut j = start;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let (arg, end) = if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            let q = bytes[j];
            let Some(close) = css[j + 1..].find(q as char) else { break };
            let arg = &css[j + 1..j + 1 + close];
            let mut e = j + 1 + close + 1;
            while e < bytes.len() && bytes[e].is_ascii_whitespace() {
                e += 1;
            }
            if e >= bytes.len() || bytes[e] != b')' {
                i = start;
                continue;
            }
            (arg, e + 1)
        } else {
            let Some(close) = css[j..].find(')') else { break };
            (css[j..j + close].trim_end(), j + close + 1)
        };
        let skip = arg.is_empty() || is_data_url(arg) || arg.starts_with('#') || arg.contains('\\') || arg.contains('"') || arg.contains('\'');
        if !skip && let Ok(abs) = sheet_url.join(arg) && abs.as_str() != arg {
            out.push_str(&css[last..i]);
            out.push_str("url(\"");
            out.push_str(abs.as_str());
            out.push_str("\")");
            last = end;
        }
        i = end;
    }
    out.push_str(&css[last..]);
    out
}

/// Upper bound on distinct CSS `background-image`/`mask-image` urls fetched per page (Q35a).
pub(crate) const MAX_CSS_IMAGE_FETCHES: usize = 64;

/// QY56: upper bound on images fetched after load per page (script-set `src`,
/// created or inserted `<img>`), separate from the load-time images.
pub(crate) const MAX_LATE_IMAGE_FETCHES: usize = 256;

/// Fetches and decodes one external image — `None` on any failure
/// (unreachable, not a supported image content type, or bytes `image`
/// itself can't decode). Reuses `decode_image_bytes` verbatim — the
/// exact same decoder `resolve_img` already uses for a `data:` URI
/// `src`, so a live-fetched image and an inlined one decode identically.
pub(crate) fn fetch_image_via_wire(path: &std::path::Path, url: &Url, top: &Url) -> Option<DecodedImage> {
    let (mime, body) =
        fetch_sub_resource_typed(path, url, top, &["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml"])?;
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
    /// Each runnable classic `<script>`, in document order (Q33a).
    pub(crate) scripts: Vec<ScriptRef>,
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
                // QY54b1: the same test the live link sync uses (`disabled` links do not apply).
                if let Some(href) = crate::link_css::stylesheet_link_href(handle) {
                    out.stylesheets.push(href);
                }
            }
            "script" => {
                if let Some(r) = script_ref(handle) {
                    out.scripts.push(r);
                }
            }
            "img" => {
                if let Some(src) = element_attr(handle, "src")
                    && !src.starts_with("data:")
                {
                    out.images.push(src.to_string());
                }
            }
            "video" => {
                // QM1g: the poster shows until the first frame.
                if let Some(src) = element_attr(handle, "poster")
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
) -> PageResources {
    let Ok(dom) = new_parser().read_from(&mut &html[..]) else { return PageResources { css: String::new(), sheets: Vec::new(), scripts: Vec::new() } };
    // The fetched page's own `<base href>`, if it declares one, wins —
    // same priority `resolve_href`'s every other caller already gives
    // an explicit `<base>` over the document's own URL.
    let base = find_base_url(&dom.document).unwrap_or_else(|| page_url.clone());
    let mut resources = ExternalResources::default();
    find_external_resources(&dom.document, &mut resources);
    let (extra_css, sheets) = fetch_stylesheet_parts(path, &base, page_url, &resources.stylesheets);
    spawn_image_fetches(proxy, path, &base, page_url, resources.images, generation);
    let scripts = fetch_scripts(path, &base, page_url, resources.scripts);
    prefetch_iframes(path, &base, page_url, &dom.document);
    PageResources { css: extra_css, sheets, scripts }
}

/// Fetch (and `@import`-expand) every stylesheet href against `base`,
/// concatenated in document order.
pub(crate) fn fetch_stylesheets(path: &std::path::Path, base: &Url, top: &Url, hrefs: &[String]) -> String {
    fetch_stylesheet_parts(path, base, top, hrefs).0
}

/// QY54b1: `fetch_stylesheets` plus what the text is made of (each href's range, `None` when its
/// fetch failed), so later link changes can rebuild it (`link_css::LinkSheets`).
pub(crate) fn fetch_stylesheet_parts(path: &std::path::Path, base: &Url, top: &Url, hrefs: &[String]) -> (String, crate::link_css::SheetParts) {
    let mut extra_css = String::new();
    let mut parts: crate::link_css::SheetParts = Vec::with_capacity(hrefs.len());
    for href in hrefs {
        // QY54b1: a sheet (by resolved URL) is applied once, at its first link (as `LinkSheets::sync` does).
        let key = crate::link_css::link_key(Some(base), href);
        if parts.iter().any(|(k, _)| *k == key) {
            continue;
        }
        match fetch_one_stylesheet(path, base, top, href) {
            Some(css) => {
                let start = extra_css.len();
                extra_css.push_str(&css);
                parts.push((key, Some(start..extra_css.len())));
                extra_css.push('\n');
            }
            None => parts.push((key, None)),
        }
    }
    // QMEM3b: push_str doubling leaves up to 2x capacity (4.4 MB of text held 8.8 MB on YouTube),
    // and this string lives as long as the page (the App keeps it, `EXTRA_CSS` copies it).
    extra_css.shrink_to_fit();
    (extra_css, parts)
}

/// One stylesheet href (a `data:` URI or a URL resolved against `base`), fetched through wire and
/// `@import`-expanded. `None` when it cannot be resolved or fetched.
pub(crate) fn fetch_one_stylesheet(path: &std::path::Path, base: &Url, top: &Url, href: &str) -> Option<String> {
    if let Some(css) = decode_data_uri_text(href.trim()) {
        // Local `data:` stylesheet (QA5); its `@import`s resolve like any other.
        let url = Url::parse("data:text/css,").expect("static url");
        let mut stack = vec![url.clone()];
        return Some(expand_css_imports(&css, base, &mut stack, &mut |u| fetch_stylesheet_via_wire(path, u, top)));
    }
    let url = resolve_href(Some(base), href).ok()?;
    let css = fetch_stylesheet_via_wire(path, &url, top)?;
    let mut stack = vec![url.clone()];
    Some(expand_css_imports(&css, &url, &mut stack, &mut |u| fetch_stylesheet_via_wire(path, u, top)))
}

/// One background thread per distinct image `src` (a real page commonly
/// repeats the same icon URL many times; one fetch and one `image_cache`
/// entry keyed by the raw `src` covers every occurrence).
pub(crate) fn spawn_image_fetches(proxy: &EventLoopProxy<AppEvent>, path: &std::path::Path, base: &Url, top: &Url, images: Vec<String>, generation: u64) {
    let fresh: Vec<String> = images.into_iter().filter(|src| claim_image_fetch(generation, src)).collect();
    spawn_claimed_image_fetches(proxy, path, base, top, fresh, generation);
}

/// QY56: the `(generation, raw src)` pairs already fetched or in flight, so
/// the load-time fetch, CSS `url()` misses and images that appear later
/// (script-set `src`, created or inserted `<img>`) never fetch one URL twice.
/// A newer generation starts an empty set; a stale generation claims nothing.
static IMAGE_CLAIMS: Mutex<(u64, Option<HashSet<String>>)> = Mutex::new((0, None));

/// True when `src` was not claimed yet for `generation` (and is now).
pub(crate) fn claim_image_fetch(generation: u64, src: &str) -> bool {
    let mut g = IMAGE_CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
    if generation < g.0 {
        return false;
    }
    if generation > g.0 {
        *g = (generation, None);
    }
    g.1.get_or_insert_with(HashSet::new).insert(src.to_string())
}

/// QY56: whether `src` is already claimed for `generation`.
#[cfg(test)]
pub(crate) fn image_fetch_claimed(generation: u64, src: &str) -> bool {
    let g = IMAGE_CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
    generation == g.0 && g.1.as_ref().is_some_and(|s| s.contains(src))
}

/// Starts one fetch thread per src (callers already claimed them). A src
/// that cannot be resolved reports a failed fetch at once.
pub(crate) fn spawn_claimed_image_fetches(proxy: &EventLoopProxy<AppEvent>, path: &std::path::Path, base: &Url, top: &Url, images: Vec<String>, generation: u64) {
    for src in images {
        let Ok(url) = resolve_href(Some(base), &src) else {
            let _ = proxy.send_event(AppEvent::ImageFetched(generation, src, None));
            continue;
        };
        let path = path.to_path_buf();
        let proxy = proxy.clone();
        let top = top.clone();
        thread::spawn(move || {
            let image = fetch_image_via_wire(&path, &url, &top);
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
    begin_wire_load(generation);
    let mut resources = ExternalResources::default();
    find_external_resources(document, &mut resources);
    spawn_image_fetches(proxy, socket, &base, &base, resources.images, generation);
    if resources.stylesheets.is_empty() && resources.scripts.is_empty() {
        return;
    }
    let top = base.clone();
    let (proxy, socket, hrefs, refs) = (proxy.clone(), socket.to_path_buf(), resources.stylesheets, resources.scripts);
    thread::spawn(move || {
        let (css, sheets) = fetch_stylesheet_parts(&socket, &base, &top, &hrefs);
        let scripts = fetch_scripts(&socket, &base, &top, refs);
        let _ = proxy.send_event(AppEvent::StreamCss(generation, css, sheets, scripts));
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
pub(crate) fn load_file_resources(proxy: &EventLoopProxy<AppEvent>, socket: &std::path::Path, path: &str, html: &[u8]) -> (Option<Url>, PageResources) {
    let Some(url) = file_page_url(path) else { return (None, PageResources { css: String::new(), sheets: Vec::new(), scripts: Vec::new() }) };
    begin_wire_load(0);
    let res = process_external_resources(proxy, socket, &url, html, 0);
    (Some(url), res)
}
