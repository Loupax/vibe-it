use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::write::EncoderWriter;
use url::Url;

/// A real Wikipedia article can embed 200+ images — fetching each one is
/// genuinely where almost all of this tool's wall-clock time goes.
/// Printing one stderr line per image would scroll past faster than it's
/// readable, so a single `\r`-updating counter gives a visible "it's
/// still working, not hung" signal instead.
static RESOURCE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Every HTTP request this run has made and their summed latency —
/// cheap to maintain unconditionally (a couple of atomic adds per
/// request), so it's always available for `print_timing_summary` at
/// exit rather than needing a separate code path to turn on.
static REQUEST_COUNT: AtomicU32 = AtomicU32::new(0);
static REQUEST_NANOS: AtomicU64 = AtomicU64::new(0);

/// `SNAG_TIMING=1 snag ...` prints one line per HTTP request (method,
/// URL, latency) as it happens, plus a final summary (request count,
/// total time spent waiting on the network vs. total wall time) — added
/// specifically to answer "is this actually hung, or just slow, and if
/// slow, where's the time going" with real numbers instead of a guess,
/// the same measure-don't-guess habit `scratchy/NOTES.md` already
/// documents for this project's performance questions generally. Off by
/// default: every request's timing is cheap to accumulate (see
/// `REQUEST_COUNT`/`REQUEST_NANOS`) and always feeds the end-of-run
/// summary, but the noisy per-request line is opt-in.
fn timing_enabled() -> bool {
    std::env::var_os("SNAG_TIMING").is_some()
}

/// Times one `agent.get(url).call()`, always accrues it into the
/// request-count/latency totals, and — only when `SNAG_TIMING` is set —
/// prints it immediately (via `progress_line`, so it doesn't collide
/// with the `\r`-updating resource counter). The one function every
/// actual network request in this file goes through, so nothing can be
/// measured inconsistently or missed.
// `ureq::Error` is a large enum (carries a full `Response` in one of its
// variants) — the same type every direct `.call()` already returned
// before this wrapper existed, just not previously named in a function
// signature clippy could see and flag. Not worth boxing just to quiet
// the lint on a function called a few hundred times per run at most.
#[allow(clippy::result_large_err)]
fn timed_get(agent: &ureq::Agent, url: &Url) -> Result<ureq::Response, ureq::Error> {
    let start = Instant::now();
    let result = agent.get(url.as_str()).call();
    let elapsed = start.elapsed();
    REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);
    REQUEST_NANOS.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    if timing_enabled() {
        let status = match &result {
            Ok(r) => r.status().to_string(),
            Err(e) => format!("ERR {e}"),
        };
        progress_line(&format!("TIMING {:>6.1}ms {status} {url}", elapsed.as_secs_f64() * 1000.0));
    }
    result
}

/// Printed at the very end of `main`, after all output is written —
/// `wall_start` is captured at the top of `main`, before anything else.
fn print_timing_summary(wall_start: Instant) {
    if !timing_enabled() {
        return;
    }
    let requests = REQUEST_COUNT.load(Ordering::Relaxed);
    let http_ms = REQUEST_NANOS.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    progress_line(&format!(
        "TIMING summary: {requests} requests, {http_ms:.0}ms total in HTTP calls \
         ({:.0}ms avg), {wall_ms:.0}ms wall clock",
        if requests > 0 { http_ms / requests as f64 } else { 0.0 }
    ));
}

/// A true streaming tag-and-resource-inlining filter, not a parse/mutate/
/// serialize batch tool (that was v1-v4's design; rewritten here because
/// it meant buffering the *entire* document, fetching *every* resource,
/// and holding the whole tree in memory before writing a single byte of
/// output — scratchy, piped straight after this, renders incrementally
/// as bytes arrive, so none of that buffering was earning its keep).
///
/// The model, exactly as asked for: every tag that doesn't fetch a
/// resource is copied to stdout byte-for-byte, unexamined, the instant
/// it's read. A `<link rel=stylesheet>` or `<img src=...>`/`<style>`
/// with `url(...)` references fetches synchronously and writes the
/// fetched content in its place; everything else (including `<script>`,
/// explicitly left untouched) passes straight through, exactly as
/// found — no re-serialization, no normalized quoting, no added or
/// dropped closing tags beyond what the source itself had.
///
/// `snag [url]`: `snag <url>` fetches `<url>` itself and streams its
/// *response body* straight into the tag scanner (never buffered as a
/// whole string). `snag` alone streams stdin the same way. Either way,
/// output starts flowing before the fetch (or stdin read) even
/// finishes.
fn main() {
    let wall_start = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 {
        eprintln!("usage: snag [url]");
        eprintln!("  snag <url>   fetch <url>, stream the bundled result to stdout");
        eprintln!("  snag         bundle HTML streamed from stdin instead of fetching it");
        eprintln!("               (relative links resolve against <base href> or");
        eprintln!("               <link rel=canonical>, found by scanning ahead no");
        eprintln!("               further than the end of <head>); e.g. curl -s <url> | snag");
        std::process::exit(1);
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("snag/0.1 (+https://github.com/Loupax/vibe-it) scratchy test fixture bundler")
        .build();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if let Some(arg) = args.get(1) {
        let url = match Url::parse(arg) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("snag: invalid URL {arg:?}: {e}");
                std::process::exit(1);
            }
        };
        progress_line(&format!("fetching {url}"));
        let resp = match timed_get(&agent, &url) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("snag: failed to fetch {url}: {e}");
                std::process::exit(1);
            }
        };
        let mut reader = BufReader::new(resp.into_reader());
        let mut visited_css = HashSet::new();
        let mut deferred = Vec::new();
        if let Err(e) = stream_body(&mut reader, &mut out, Some(&url), &agent, &mut visited_css, &mut deferred) {
            finish_with_io_error(e);
        }
    } else {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        match discover_base(&mut reader) {
            Ok(DiscoveredBase { base, head }) => {
                if base.is_none() {
                    // Not fatal — a page with nothing to fetch (no
                    // relative stylesheet/image references at all) is
                    // perfectly fine without one. Only an actual
                    // resolution attempt with no base to resolve
                    // against falls back to passthrough (see
                    // `stream_body`); this is just advance notice of
                    // why that might happen.
                    eprintln!("snag: no <base href> or <link rel=canonical> found in <head> —");
                    eprintln!("      relative stylesheet/image references won't be inlined");
                }
                let mut chained = std::io::Cursor::new(head).chain(reader);
                let mut visited_css = HashSet::new();
                let mut deferred = Vec::new();
                if let Err(e) =
                    stream_body(&mut chained, &mut out, base.as_ref(), &agent, &mut visited_css, &mut deferred)
                {
                    finish_with_io_error(e);
                }
            }
            Err(e) => finish_with_io_error(e),
        }
    }
    print_timing_summary(wall_start);
}

fn finish_with_io_error(e: std::io::Error) {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        // A downstream reader closing early (`snag ... | head`) — the
        // normal shape of a Unix pipeline, not a real error.
        return;
    }
    eprintln!("snag: {e}");
    std::process::exit(1);
}

fn progress_line(msg: &str) {
    if RESOURCE_COUNT.load(Ordering::Relaxed) > 0 {
        eprintln!();
    }
    eprintln!("snag: {msg}");
}

// ---------- base URL discovery (stdin mode only) ----------
//
// `snag <url>` already knows its base the instant it starts — no lookahead
// needed, full streaming from byte one. Reading from stdin has no such
// URL, and *something* has to anchor every relative link in the page to
// an absolute one. The obvious candidates — `<base href>` or `<link
// rel=canonical>` — don't reliably come first: real pages (confirmed on
// Wikipedia) put `<link rel=stylesheet>` tags *before* `<link
// rel=canonical>` in `<head>`, so there's no way to resolve the first
// stylesheet's `href` without having already looked further ahead in the
// document than that tag itself. This is the one deliberate exception to
// "don't buffer anything": the `<head>` section (small — a few KB on a
// real page, nowhere near the size of the full document or its
// resources) is buffered just long enough to find a base, then replayed
// through the exact same streaming processor used for everything after
// it (via `Read::chain`) — so the buffering is bounded and one-time, not
// a growing in-memory document.

struct DiscoveredBase {
    base: Option<Url>,
    /// Every byte consumed while searching — handed back so the caller
    /// can replay it through `stream_body` instead of losing it.
    head: Vec<u8>,
}

/// Capped so a pathological page without a `</head>` (or any head tag
/// recognized by name) can't make this buffer grow unboundedly — treated
/// the same as "no base found" once hit.
const HEAD_SCAN_CAP: usize = 1 << 20; // 1 MiB

fn discover_base(reader: &mut impl BufRead) -> std::io::Result<DiscoveredBase> {
    let mut head = Vec::new();
    loop {
        let n = reader.read_until(b'<', &mut head)?;
        if n == 0 {
            return Ok(DiscoveredBase { base: None, head });
        }
        let Some(tag) = read_tag(reader, &mut head)? else {
            return Ok(DiscoveredBase { base: None, head });
        };
        if !tag.closing {
            let is_base_source = tag.name == "base" || (tag.name == "link" && tag.rel_has("canonical"));
            if is_base_source {
                if let Some(href) = tag.attr("href")
                    && let Ok(u) = Url::parse(href)
                {
                    return Ok(DiscoveredBase { base: Some(u), head });
                }
            } else if tag.is_raw_text_element() {
                // Don't let a <style>/<script> in the head confuse the
                // scan — skip its content the same way the main loop
                // would, without acting on it (nothing fetchable can
                // establish a base anyway).
                skip_raw_text_content(reader, &mut head, &tag.name)?;
            }
        } else if tag.name == "head" {
            return Ok(DiscoveredBase { base: None, head });
        }
        if head.len() > HEAD_SCAN_CAP {
            return Ok(DiscoveredBase { base: None, head });
        }
    }
}

// ---------- the streaming processor ----------

/// Reads tags and text from `reader`, writing to `out` as it goes.
/// Recognized resource-fetching tags are replaced with their fetched
/// content (flushed immediately after); everything else — every other
/// tag, every text run, comments, the doctype, `<script>` included — is
/// copied through exactly as found, the moment it's read.
fn stream_body(
    reader: &mut impl BufRead,
    out: &mut impl Write,
    base: Option<&Url>,
    agent: &ureq::Agent,
    visited_css: &mut HashSet<String>,
    deferred: &mut Vec<Vec<u8>>,
) -> std::io::Result<()> {
    loop {
        let mut chunk = Vec::new();
        reader.read_until(b'<', &mut chunk)?;
        // `read_until`'s return value alone can't tell "found the
        // delimiter" apart from "hit EOF without one" — both read a
        // positive byte count whenever there's trailing text before the
        // end of input. Checking the last byte can: it's only `<` when
        // the delimiter genuinely terminated the read, not when EOF cut
        // it off first (the empty-input case also falls out of this
        // correctly, since `chunk.last()` is `None` either way).
        if chunk.last() != Some(&b'<') {
            out.write_all(&chunk)?;
            flush_deferred(deferred, out)?; // no </body> ever showed up — tack them on at true EOF
            out.flush()?;
            return Ok(());
        }
        // Everything up to (but not including) the '<' is plain text —
        // write it now, before even looking at the tag that follows.
        out.write_all(&chunk[..chunk.len() - 1])?;

        let mut tag_bytes = vec![b'<'];
        let Some(tag) = read_tag(reader, &mut tag_bytes)? else {
            // EOF mid-tag (truncated input) — write whatever was read
            // and stop, same as any other unexpected-EOF case.
            out.write_all(&tag_bytes)?;
            flush_deferred(deferred, out)?;
            out.flush()?;
            return Ok(());
        };

        if tag.is_deferred_script() {
            // Held back, not written here — a bounded, targeted
            // exception to "don't buffer anything" (same shape as
            // `discover_base`'s own `<head>` buffering): only the
            // specific tags this applies to are ever held, not the
            // document as a whole, and `defer`'s own real-browser
            // semantics already guarantee this doesn't change observed
            // behavior anywhere this bundle might end up (see
            // `is_deferred_script`'s own docs). Moving it out of the
            // way means scratchy's streaming renderer reaches real
            // visible content sooner instead of first having to read
            // past however much inert script sits ahead of it.
            let content = read_raw_text_content(reader, "script")?;
            let mut buf = tag.raw.clone();
            buf.extend_from_slice(&content.text);
            buf.extend_from_slice(&content.closing_tag);
            deferred.push(buf);
            continue;
        }

        if tag.closing && tag.name == "body" {
            flush_deferred(deferred, out)?;
            out.write_all(&tag.raw)?;
            continue;
        }

        if tag.closing || tag.name != "link" && tag.name != "style" && tag.name != "img" {
            // Not a tag this file does anything special with — copied
            // through byte-for-byte, including <script> without `defer`
            // (left alone on purpose) and any other closing tag.
            out.write_all(&tag.raw)?;
            if tag.is_raw_text_element() {
                stream_raw_text_passthrough(reader, out, &tag.name)?;
            }
            continue;
        }

        match tag.name.as_str() {
            "link" if tag.rel_has("stylesheet") => {
                let Some(href) = tag.attr("href") else {
                    out.write_all(&tag.raw)?;
                    continue;
                };
                let Some(target) = base.and_then(|b| b.join(href).ok()) else {
                    out.write_all(&tag.raw)?;
                    continue;
                };
                if !visited_css.insert(target.to_string()) {
                    continue; // duplicate stylesheet, already inlined once — drop silently
                }
                progress_line(&format!("inlining stylesheet {target}"));
                out.write_all(b"<style>")?;
                if let Ok(text) = fetch_text(agent, &target) {
                    process_css(&text, &target, agent, visited_css, out)?;
                }
                out.write_all(b"</style>")?;
                out.flush()?;
            }
            "link" => out.write_all(&tag.raw)?,
            "style" => {
                out.write_all(&tag.raw)?;
                let content = read_raw_text_content(reader, "style")?;
                match base {
                    Some(base) => {
                        let text = String::from_utf8_lossy(&content.text);
                        process_css(&text, base, agent, visited_css, out)?;
                    }
                    // No base to resolve this stylesheet's own url()s
                    // against — pass its content through unprocessed
                    // rather than guessing.
                    None => out.write_all(&content.text)?,
                }
                out.write_all(&content.closing_tag)?;
                out.flush()?;
            }
            "img" => {
                let Some(src) = tag.attr("src") else {
                    out.write_all(&tag.raw)?;
                    continue;
                };
                if src.starts_with("data:") {
                    out.write_all(&tag.raw)?;
                    continue;
                }
                let Some(target) = base.and_then(|b| b.join(src).ok()) else {
                    out.write_all(&tag.raw)?;
                    continue;
                };
                let before = replace_src_with_placeholder(&tag.raw);
                match before {
                    Some((prefix, suffix)) => {
                        out.write_all(&prefix)?;
                        if !stream_data_uri(agent, &target, out)? {
                            // Unreachable — fall back to the original
                            // (now-broken, but harmless) src rather than
                            // leaving a dangling data: prefix.
                            out.write_all(src.as_bytes())?;
                        }
                        out.write_all(&suffix)?;
                    }
                    None => out.write_all(&tag.raw)?,
                }
                out.flush()?;
            }
            _ => unreachable!("filtered above"),
        }
    }
}

// ---------- fetching ----------

fn fetch_text(agent: &ureq::Agent, url: &Url) -> Result<String, String> {
    let resp = timed_get(agent, url).map_err(|e| e.to_string())?;
    resp.into_string().map_err(|e| e.to_string())
}

/// Fetches `url` and writes `data:<mime>;base64,<encoded>` straight to
/// `out` — the response body is streamed through a `base64` encoding
/// `Write` adapter one chunk at a time via `io::copy`, so the resource's
/// bytes are never held in memory as a whole, however large (the one
/// path every fetched resource takes to reach stdout, image tag or CSS
/// `url()` alike — see `process_css`). Returns `false` (writes nothing)
/// on any failure, leaving it to the caller to fall back to the
/// original, now-probably-broken reference.
fn stream_data_uri(agent: &ureq::Agent, url: &Url, out: &mut impl Write) -> std::io::Result<bool> {
    let Ok(resp) = timed_get(agent, url) else { return Ok(false) };
    let content_type = resp.content_type().to_string();
    let mime = if content_type.is_empty() || content_type == "text/plain" {
        guess_mime(url.path())
    } else {
        content_type
    };
    write!(out, "data:{mime};base64,")?;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut encoder = EncoderWriter::new(&mut *out, &engine);
    let mut body = resp.into_reader();
    if std::io::copy(&mut body, &mut encoder).is_err() {
        return Ok(false);
    }
    if encoder.finish().is_err() {
        return Ok(false);
    }
    note_resource_fetched();
    Ok(true)
}

fn note_resource_fetched() {
    let n = RESOURCE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    eprint!("\rsnag: inlined {n} resources so far...");
    let _ = std::io::stderr().flush();
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
// Still a hand-rolled text scanner over one stylesheet's text at a time,
// not the whole document (see `<head>`-scanning above for the same
// reasoning): `@import`/`url(...)` rewriting inherently needs to look
// ahead for a matching terminator (`;`, `)`), and the stylesheet's own
// text has to be read in full to scan it — but that text, typically tens
// of KB, is a different scale of thing from the *images it references*,
// which used to each be fully buffered (`fetch_as_data_uri`, building a
// `String`) before the whole stylesheet was written out in one piece.
// `@import` and `url(...)` are now handled in a single pass that writes
// straight to `out` as it scans — plain CSS text between them goes
// through immediately, and each `url(...)`'s referenced resource streams
// through `stream_data_uri` (the same zero-buffer base64 encoder
// `<img src>` uses) directly into the output, rather than ever
// existing as a whole base64 string in memory first.

/// Fetches and fully resolves one stylesheet's text, writing it straight
/// to `out`: inlines `@import`s (recursively, each resolved relative to
/// *its own* URL, not the original page's) and streams every remaining
/// `url(...)` reference's resource directly into place. `visited` guards
/// against re-inlining the same stylesheet twice (duplicate `@import`s,
/// or an import cycle).
fn process_css(
    css: &str,
    base: &Url,
    agent: &ureq::Agent,
    visited: &mut HashSet<String>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let lower = css.to_ascii_lowercase();
    let mut i = 0;
    loop {
        let next_import = lower[i..].find("@import").map(|p| i + p);
        let next_url = lower[i..].find("url(").map(|p| i + p);
        let Some(pos) = min_some(next_import, next_url) else {
            out.write_all(&css.as_bytes()[i..])?;
            return Ok(());
        };
        out.write_all(&css.as_bytes()[i..pos])?;

        if Some(pos) == next_import {
            let Some(semi_rel) = css[pos..].find(';') else {
                out.write_all(&css.as_bytes()[pos..])?;
                return Ok(());
            };
            let stmt = &css[pos + "@import".len()..pos + semi_rel];
            i = pos + semi_rel + 1;
            let Some(raw_url) = extract_import_url(stmt) else { continue };
            let Ok(target) = base.join(&raw_url) else { continue };
            if !visited.insert(target.to_string()) {
                continue; // duplicate import/cycle — skip silently
            }
            if let Ok(text) = fetch_text(agent, &target) {
                process_css(&text, &target, agent, visited, out)?;
            }
        } else {
            let args_start = pos + "url(".len();
            let Some(close_rel) = find_url_close_paren(&css[args_start..]) else {
                out.write_all(&css.as_bytes()[pos..])?;
                return Ok(());
            };
            let inner = unquote(css[args_start..args_start + close_rel].trim());
            i = args_start + close_rel + 1;
            out.write_all(b"url(\"")?;
            if inner.starts_with("data:") || inner.is_empty() {
                out.write_all(inner.as_bytes())?;
            } else {
                match base.join(&inner) {
                    Ok(target) if stream_data_uri(agent, &target, out)? => {}
                    _ => out.write_all(inner.as_bytes())?,
                }
            }
            out.write_all(b"\")")?;
        }
    }
}

fn min_some(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

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

// ---------- tag scanning ----------
//
// Deliberately hand-rolled rather than driven through html5ever's own
// tokenizer/tree builder: this file needs byte-exact pass-through of
// everything it doesn't specifically act on (original attribute
// quoting, self-closing slashes, whether a void element happened to
// have a stray closing tag in the source or not, ...), and a real
// parser's job is the opposite of that — normalizing input into a
// canonical structure, which is exactly what v1-v4 used it for and
// exactly what this rewrite moved away from.

struct RawTag {
    /// The exact bytes as found in the source, `<` through `>` inclusive.
    raw: Vec<u8>,
    name: String,
    closing: bool,
    attrs: Vec<(String, String)>,
}

impl RawTag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn rel_has(&self, value: &str) -> bool {
        self.attr("rel")
            .map(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case(value)))
            .unwrap_or(false)
    }

    /// `<style>`/`<script>` are HTML "raw text" elements — their content
    /// isn't parsed for nested tags at all, only the literal closing tag
    /// text ends them. Anything that scans tag-by-tag has to know this,
    /// or `if (a < b)` inside a `<script>` would be misread as a tag.
    fn is_raw_text_element(&self) -> bool {
        !self.closing && (self.name == "style" || self.name == "script")
    }

    /// `<script defer>` — a real browser already defers this script's
    /// *execution* until after parsing finishes, in document order among
    /// other deferred scripts, so moving its *position* to just before
    /// `</body>` changes nothing about real-browser behavior. `async`
    /// takes priority over `defer` per spec when both are present (and
    /// carries no ordering guarantee of its own), so a script with both
    /// isn't treated as deferred here.
    fn is_deferred_script(&self) -> bool {
        !self.closing && self.name == "script" && self.attr("defer").is_some() && self.attr("async").is_none()
    }
}

/// Reads one tag's bytes, starting right after the `<` already pushed
/// onto `buf` by the caller, through and including the matching `>`
/// (comment- and quote-aware: a `>` inside a `"..."`/`'...'` attribute
/// value or a `<!-- ... -->` comment doesn't end the tag/comment early).
/// Appends every byte read to `buf` either way. Returns `None` on EOF
/// before a terminating `>` was found — the caller treats that as "pass
/// through whatever was read" (truncated input), not an error.
fn read_tag(reader: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<Option<RawTag>> {
    let start = buf.len() - 1; // index of the '<' already in buf

    // `<!--` is the one construct allowed to contain an unquoted `>` —
    // scan for the literal `-->` terminator instead of a bare `>`.
    let mut peek = [0u8; 3];
    if peek_bytes(reader, &mut peek)? == 3 && &peek == b"!--" {
        buf.extend_from_slice(&peek);
        reader.consume(3);
        read_until_sequence(reader, buf, b"-->")?;
        return Ok(Some(finish_tag(buf, start, false)));
    }

    let mut quote: Option<u8> = None;
    loop {
        let mut byte = [0u8; 1];
        if reader.read(&mut byte)? == 0 {
            return Ok(None);
        }
        let b = byte[0];
        buf.push(b);
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Ok(Some(parse_tag(buf, start))),
                _ => {}
            },
        }
    }
}

fn peek_bytes(reader: &mut impl BufRead, out: &mut [u8]) -> std::io::Result<usize> {
    let avail = reader.fill_buf()?;
    let n = avail.len().min(out.len());
    out[..n].copy_from_slice(&avail[..n]);
    Ok(n)
}

/// Reads (and appends to `buf`) until `needle` has been fully matched,
/// inclusive, or EOF.
fn read_until_sequence(reader: &mut impl BufRead, buf: &mut Vec<u8>, needle: &[u8]) -> std::io::Result<bool> {
    let mut matched = 0usize;
    loop {
        let mut byte = [0u8; 1];
        if reader.read(&mut byte)? == 0 {
            return Ok(false);
        }
        buf.push(byte[0]);
        if byte[0] == needle[matched] {
            matched += 1;
            if matched == needle.len() {
                return Ok(true);
            }
        } else {
            matched = if byte[0] == needle[0] { 1 } else { 0 };
        }
    }
}

fn finish_tag(buf: &[u8], start: usize, closing: bool) -> RawTag {
    RawTag { raw: buf[start..].to_vec(), name: String::new(), closing, attrs: Vec::new() }
}

fn parse_tag(buf: &[u8], start: usize) -> RawTag {
    let raw = buf[start..].to_vec();
    let Ok(s) = std::str::from_utf8(&raw) else {
        return RawTag { raw, name: String::new(), closing: false, attrs: Vec::new() };
    };
    let inner = s.trim_start_matches('<').trim_end_matches('>');
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let inner = inner.strip_suffix('/').unwrap_or(inner); // self-closing slash, if any
    let name_end = inner.find(|c: char| c.is_ascii_whitespace()).unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let attrs = if closing { Vec::new() } else { parse_attrs(&inner[name_end..]) };
    RawTag { raw, name, closing, attrs }
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let name = s[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let val_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                attrs.push((name, decode_entities(&s[val_start..i])));
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let val_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                attrs.push((name, decode_entities(&s[val_start..i])));
            }
        } else {
            attrs.push((name, String::new()));
        }
    }
    attrs
}

/// A minimal entity decoder — just the 5 predefined XML entities plus
/// numeric character references (`&#NN;`/`&#xHH;`), not the full HTML5
/// named-entity table (`&nbsp;` and friends). The only place decoded
/// values are actually *used* for is resolving a URL to fetch
/// (`href`/`src`), and `&amp;` in a query string (seen constantly in
/// real `href`s, e.g. MediaWiki's `?lang=de&amp;modules=...`) is by far
/// the dominant real-world case — anything else passes through
/// literally rather than failing to decode.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some(semi) = s[i..].find(';')
        {
            let entity = &s[i + 1..i + semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if entity.starts_with(['#', 'x', 'X']) => decode_numeric_entity(entity),
                _ => None,
            };
            if let Some(c) = decoded {
                out.push(c);
                i += semi + 1;
                continue;
            }
        }
        let ch_len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    let num = entity.strip_prefix('#')?;
    let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        num.parse::<u32>().ok()
    };
    code.and_then(char::from_u32)
}

/// Replaces `<img ...>`'s `src="..."` value with a placeholder split
/// point: returns `(bytes before the value, bytes from the closing
/// quote onward)`, so the caller can stream the replacement value
/// directly between them without ever holding the whole tag — or the
/// image — assembled in memory. `None` if `src` isn't a quoted attribute
/// (unquoted `src=foo.png` is accepted when *reading* it, matching
/// `parse_attrs`, but not when *rewriting* it in place — a narrow,
/// documented gap: real/machine-generated markup, including
/// Wikipedia's, always quotes its attributes).
fn replace_src_with_placeholder(tag_raw: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let s = std::str::from_utf8(tag_raw).ok()?;
    let lower = s.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("src=") {
        let pos = search_from + rel;
        let preceded_ok = pos == 0 || s.as_bytes()[pos - 1].is_ascii_whitespace();
        if preceded_ok {
            let after_eq = pos + "src=".len();
            if after_eq < s.len() && (s.as_bytes()[after_eq] == b'"' || s.as_bytes()[after_eq] == b'\'') {
                let quote = s.as_bytes()[after_eq];
                let val_start = after_eq + 1;
                if let Some(end_rel) = s[val_start..].find(quote as char) {
                    let val_end = val_start + end_rel;
                    return Some((s.as_bytes()[..val_start].to_vec(), s.as_bytes()[val_end..].to_vec()));
                }
            }
        }
        search_from = pos + "src=".len();
    }
    None
}

/// Writes out every `<script defer>` held back by `stream_body`, in the
/// order they were originally encountered, then empties the list —
/// called right before `</body>` (the normal case) or at true end of
/// input if `</body>` never showed up at all.
fn flush_deferred(deferred: &mut Vec<Vec<u8>>, out: &mut impl Write) -> std::io::Result<()> {
    for buf in deferred.drain(..) {
        out.write_all(&buf)?;
    }
    Ok(())
}

/// The non-fetching-tag path for `<style>`/`<script>`'s raw-text content
/// — copies it straight through unexamined (used for `<script>`, left
/// alone entirely, and for `<style>`'s own *opening* tag handling, where
/// the content itself still needs separate processing — see
/// `stream_body`'s `"style"` arm, which reads the content itself rather
/// than calling this).
fn stream_raw_text_passthrough(reader: &mut impl BufRead, out: &mut impl Write, tag_name: &str) -> std::io::Result<()> {
    let content = read_raw_text_content(reader, tag_name)?;
    out.write_all(&content.text)?;
    out.write_all(&content.closing_tag)?;
    Ok(())
}

struct RawTextContent {
    text: Vec<u8>,
    /// The exact closing tag bytes found (`</style   >`, whatever casing/
    /// spacing the source had) — empty if EOF was hit first.
    closing_tag: Vec<u8>,
}

/// Reads a raw-text element's content up to (not including) its closing
/// tag, then the closing tag itself — case-insensitive match on
/// `</NAME`, since that's genuinely how the HTML5 spec defines where
/// these elements end (not a generic "next `<`"; a literal `<` inside
/// `<script>`/`<style>` content, e.g. `if (a < b)`, doesn't end anything).
fn read_raw_text_content(reader: &mut impl BufRead, tag_name: &str) -> std::io::Result<RawTextContent> {
    let closer = format!("</{tag_name}");
    let closer_lower = closer.to_ascii_lowercase();
    let mut text = Vec::new();
    loop {
        let n = reader.read_until(b'<', &mut text)?;
        if n == 0 {
            return Ok(RawTextContent { text, closing_tag: Vec::new() });
        }
        // Peek ahead far enough to test for the closing tag name without
        // consuming it yet (so a false match — `<strong>` inside
        // <script> — can still be scanned as ordinary content).
        let mut peek = vec![0u8; closer_lower.len() - 1];
        let got = peek_bytes(reader, &mut peek)?;
        if got == peek.len() && peek.to_ascii_lowercase() == closer_lower.as_bytes()[1..] {
            text.pop(); // drop the trailing '<' — it belongs to the closing tag
            let mut closing_tag = vec![b'<'];
            reader.consume(got);
            closing_tag.extend_from_slice(&peek);
            // consume through the closing '>' (quote-awareness doesn't
            // matter for a closing tag with no attributes).
            loop {
                let mut byte = [0u8; 1];
                if reader.read(&mut byte)? == 0 {
                    break;
                }
                closing_tag.push(byte[0]);
                if byte[0] == b'>' {
                    break;
                }
            }
            return Ok(RawTextContent { text, closing_tag });
        }
    }
}

/// `discover_base`'s own use of raw-text skipping — same reasoning as
/// `read_raw_text_content`, just discarding the content instead of
/// returning it (the head-scan only cares about finding a base, not
/// preserving what it skips past for replay — the whole buffer it's
/// building *is* the replay source already).
fn skip_raw_text_content(reader: &mut impl BufRead, buf: &mut Vec<u8>, tag_name: &str) -> std::io::Result<()> {
    let content = read_raw_text_content(reader, tag_name)?;
    buf.extend_from_slice(&content.text);
    buf.extend_from_slice(&content.closing_tag);
    Ok(())
}
