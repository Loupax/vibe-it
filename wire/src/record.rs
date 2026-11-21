// Record and replay (QREC1, scratchy R64 section 6): `wire --record DIR`
// stores every network response it serves, `wire --replay DIR` answers only
// from such a recording and never opens a socket to the network. Independent
// of `mount.rs`: a mounted origin is answered before either hook runs, so it
// is never recorded and never replayed.
//
// Layout of DIR: `blobs/<sha256 of the body>` (zstd) and `index.jsonl`, one
// JSON object per response: {seq, t_ms, method, url, range, req_body_sha256,
// status, reason, headers, body_sha256, body_len}. A recording can hold
// cookies and personal data: it stays local (scratchy DECISIONS Q5), never
// commit one. WebSocket traffic is not recorded and is refused in replay.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

/// Headers wire adds itself to every response; not stored, re-added on replay.
const WIRE_OWN: [&str; 3] = ["x-wire-version", "x-wire-features", "x-wire-cookies"];

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A writer that copies everything written to it into `buf` as well.
pub struct Tee<'a, W: Write> {
    inner: &'a mut W,
    pub buf: Vec<u8>,
}

impl<'a, W: Write> Tee<'a, W> {
    pub fn new(inner: &'a mut W) -> Self {
        Tee { inner, buf: Vec::new() }
    }
}

impl<W: Write> Write for Tee<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(data)?;
        self.buf.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A response as wire wrote it, split up.
struct RawResponse {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn parse_raw(raw: &[u8]) -> Option<RawResponse> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let text = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.splitn(3, ' ');
    first.next()?;
    let status = first.next()?.parse().ok()?;
    let reason = first.next().unwrap_or("").to_string();
    let headers = lines.filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))).collect();
    Some(RawResponse { status, reason, headers, body: raw[end + 4..].to_vec() })
}

pub struct Recorder {
    dir: PathBuf,
    index: Mutex<File>,
    seq: AtomicU64,
    start: Instant,
}

impl Recorder {
    /// `--record DIR`: creates DIR and appends to an existing recording.
    pub fn open(dir: &Path) -> Result<Recorder, String> {
        std::fs::create_dir_all(dir.join("blobs")).map_err(|e| format!("--record: cannot create {}: {e}", dir.display()))?;
        let path = dir.join("index.jsonl");
        let seq = std::fs::read_to_string(&path).map(|t| t.lines().count() as u64).unwrap_or(0);
        let index = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("--record: cannot open {}: {e}", path.display()))?;
        Ok(Recorder { dir: dir.to_path_buf(), index: Mutex::new(index), seq: AtomicU64::new(seq), start: Instant::now() })
    }

    /// Stores one response (`raw` = the bytes wire wrote for it). Synthetic
    /// error responses (X-Wire-Error) are not network answers and are skipped.
    pub fn record(&self, method: &str, url: &Url, range: Option<&str>, req_body: &[u8], raw: &[u8]) {
        let Some(r) = parse_raw(raw) else { return };
        if r.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("x-wire-error")) {
            return;
        }
        let sha = sha256_hex(&r.body);
        let blob = self.dir.join("blobs").join(&sha);
        if !blob.exists() {
            let tmp = self.dir.join("blobs").join(format!("{sha}.tmp{}", std::process::id()));
            let res = zstd::encode_all(&r.body[..], 3).and_then(|z| std::fs::write(&tmp, z)).and_then(|_| std::fs::rename(&tmp, &blob));
            if let Err(e) = res {
                eprintln!("wire: record: cannot write blob {sha}: {e}");
                return;
            }
        }
        let headers: Vec<Value> = r.headers.iter().filter(|(n, _)| !WIRE_OWN.contains(&n.to_ascii_lowercase().as_str())).map(|(n, v)| json!([n, v])).collect();
        let line = json!({
            "seq": self.seq.fetch_add(1, Ordering::Relaxed),
            "t_ms": self.start.elapsed().as_millis() as u64,
            "method": method,
            "url": url.as_str(),
            "range": range,
            "req_body_sha256": if req_body.is_empty() { String::new() } else { sha256_hex(req_body) },
            "status": r.status,
            "reason": r.reason,
            "headers": headers,
            "body_sha256": sha,
            "body_len": r.body.len(),
        });
        let mut f = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = writeln!(f, "{line}").and_then(|_| f.flush()) {
            eprintln!("wire: record: cannot append to index.jsonl: {e}");
        }
    }
}

struct Entry {
    method: String,
    url: String,
    norm: String,
    path_key: String,
    range: Option<String>,
    req_body: String,
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body_sha: String,
    body_len: usize,
}

/// Per-host query parameters ignored when matching (`host name,name` lines,
/// `*` = every host, `#` comments).
#[derive(Default)]
pub struct Rules {
    ignore: Vec<(String, Vec<String>)>,
}

impl Rules {
    pub fn parse(text: &str) -> Rules {
        let mut ignore = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some((host, params)) = line.split_once(char::is_whitespace) {
                let params = params.split([',', ' ']).map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
                ignore.push((host.to_ascii_lowercase(), params));
            }
        }
        Rules { ignore }
    }

    fn ignored(&self, host: &str, name: &str) -> bool {
        self.ignore.iter().any(|(h, ps)| (h == "*" || h == host) && ps.iter().any(|p| p == name))
    }

    /// The URL without its fragment and without ignored query parameters.
    fn normalize(&self, url: &Url) -> String {
        let mut u = url.clone();
        u.set_fragment(None);
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        if u.query().is_some() {
            let kept: Vec<(String, String)> = u.query_pairs().filter(|(k, _)| !self.ignored(&host, k)).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            if kept.is_empty() {
                u.set_query(None);
            } else {
                u.query_pairs_mut().clear().extend_pairs(kept);
            }
        }
        u.to_string()
    }
}

fn path_key(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), url.path())
}

pub struct Replayer {
    dir: PathBuf,
    entries: Vec<Entry>,
    rules: Rules,
    delay: Duration,
    counters: Mutex<HashMap<String, usize>>,
    misses: AtomicU32,
}

/// `5ms`, `2s`, or plain milliseconds.
pub fn parse_delay(s: &str) -> Result<Duration, String> {
    let (num, unit) = match s.strip_suffix("ms") {
        Some(n) => (n, 1.0),
        None => match s.strip_suffix('s') {
            Some(n) => (n, 1000.0),
            None => (s, 1.0),
        },
    };
    let n: f64 = num.parse().map_err(|_| format!("--replay-delay: bad duration {s:?} (use 5ms or 1s)"))?;
    if !(0.0..=60_000.0).contains(&(n * unit)) {
        return Err(format!("--replay-delay: {s:?} out of range"));
    }
    Ok(Duration::from_secs_f64(n * unit / 1000.0))
}

/// `bytes=A-B` or `bytes=A-` -> (A, Some(B)).
fn parse_range(r: &str) -> Option<(usize, Option<usize>)> {
    let (a, b) = r.trim().strip_prefix("bytes=")?.split_once('-')?;
    let a = a.trim().parse().ok()?;
    let b = if b.trim().is_empty() { None } else { Some(b.trim().parse().ok()?) };
    Some((a, b))
}

/// `bytes S-E/T` -> (S, E, T).
fn parse_content_range(v: &str) -> Option<(usize, usize, usize)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (se, t) = rest.split_once('/')?;
    let (s, e) = se.split_once('-')?;
    Some((s.parse().ok()?, e.parse().ok()?, t.parse().ok()?))
}

fn hdr<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

impl Replayer {
    pub fn open(dir: &Path, rules: Rules, delay: Duration) -> Result<Replayer, String> {
        let path = dir.join("index.jsonl");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("--replay: cannot read {}: {e}", path.display()))?;
        let mut entries = Vec::new();
        for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).map_err(|e| format!("--replay: {}:{}: {e}", path.display(), i + 1))?;
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            let url = Url::parse(&s("url")).map_err(|e| format!("--replay: {}:{}: bad url: {e}", path.display(), i + 1))?;
            let headers = v["headers"].as_array().map(|a| a.iter().filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string()))).collect()).unwrap_or_default();
            entries.push(Entry {
                method: s("method"),
                norm: rules.normalize(&url),
                path_key: path_key(&url),
                url: url.to_string(),
                range: v["range"].as_str().map(String::from),
                req_body: s("req_body_sha256"),
                status: v["status"].as_u64().unwrap_or(0) as u16,
                reason: s("reason"),
                headers,
                body_sha: s("body_sha256"),
                body_len: v["body_len"].as_u64().unwrap_or(0) as usize,
            });
        }
        Ok(Replayer { dir: dir.to_path_buf(), entries, rules, delay, counters: Mutex::new(HashMap::new()), misses: AtomicU32::new(0) })
    }

    #[cfg(test)]
    pub fn misses(&self) -> u32 {
        self.misses.load(Ordering::Relaxed)
    }

    fn blob(&self, sha: &str) -> Option<Vec<u8>> {
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        zstd::decode_all(&std::fs::read(self.dir.join("blobs").join(sha)).ok()?[..]).ok()
    }

    /// The n-th (per key) candidate; the last one once the recording runs out.
    fn pick(&self, key: String, cands: &[&Entry]) -> Option<usize> {
        if cands.is_empty() {
            return None;
        }
        let mut c = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let n = c.entry(key).or_insert(0);
        let i = (*n).min(cands.len() - 1);
        *n += 1;
        Some(i)
    }

    fn miss(&self, w: &mut impl Write, msg: &str) -> std::io::Result<()> {
        let hs = vec![("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Replay".to_string(), "miss".to_string())];
        crate::write_status_and_headers(w, 404, "Not Found", &hs)?;
        w.write_all(msg.as_bytes())?;
        w.flush()
    }

    /// Answers one request from the recording. Never touches the network.
    pub fn serve(&self, method: &str, url: &Url, range: Option<&str>, req_body: &[u8], w: &mut impl Write) -> std::io::Result<()> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let body_sha = if req_body.is_empty() { String::new() } else { sha256_hex(req_body) };
        let norm = self.rules.normalize(url);
        let same_req = |e: &&Entry| e.method == method && e.req_body == body_sha;

        // 1. exact (method, url, request body, range)
        let exact: Vec<&Entry> = self.entries.iter().filter(same_req).filter(|e| e.url == url.as_str() && e.range.as_deref() == range).collect();
        let mut chosen: Option<(&Entry, Option<(usize, usize)>)> = self.pick(format!("1|{method}|{url}|{body_sha}|{range:?}"), &exact).map(|i| (exact[i], None));
        // 2. the same with the ignored query parameters removed
        if chosen.is_none() {
            let c: Vec<&Entry> = self.entries.iter().filter(same_req).filter(|e| e.norm == norm && e.range.as_deref() == range).collect();
            chosen = self.pick(format!("2|{method}|{norm}|{body_sha}|{range:?}"), &c).map(|i| (c[i], None));
        }
        // 3. a range request inside a recorded body or recorded range
        if chosen.is_none()
            && let Some((a, b)) = range.and_then(parse_range)
        {
            let c: Vec<&Entry> = self
                .entries
                .iter()
                .filter(same_req)
                .filter(|e| e.norm == norm && hdr(&e.headers, "content-encoding").is_none_or(|v| v.eq_ignore_ascii_case("identity")))
                .filter(|e| match (e.status, &e.range) {
                    (200, None) => a < e.body_len,
                    (206, _) => hdr(&e.headers, "content-range").and_then(parse_content_range).is_some_and(|(s, en, _)| s <= a && b.is_some_and(|b| b <= en)),
                    _ => false,
                })
                .collect();
            chosen = self.pick(format!("3|{method}|{norm}|{body_sha}|{range:?}"), &c).map(|i| (c[i], Some((a, b.unwrap_or(usize::MAX)))));
        }
        // 4. the n-th request to the same path
        if chosen.is_none() {
            let pk = path_key(url);
            let c: Vec<&Entry> = self.entries.iter().filter(|e| e.method == method && e.path_key == pk && e.range.is_none()).collect();
            chosen = self.pick(format!("4|{method}|{pk}"), &c).map(|i| (c[i], None));
        }

        let Some((e, slice)) = chosen else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            eprintln!("wire: replay miss {method} {url}");
            return self.miss(w, "wire: replay miss");
        };
        let Some(body) = self.blob(&e.body_sha) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            eprintln!("wire: replay: blob {} missing or corrupt for {method} {url}", e.body_sha);
            return self.miss(w, "wire: replay blob missing");
        };
        let mut headers = e.headers.clone();
        let (status, reason, body) = match slice {
            None => (e.status, e.reason.clone(), body),
            Some((a, b)) => {
                // Offset of the stored body within the full resource, and the full size.
                let (base, total) = match hdr(&e.headers, "content-range").and_then(parse_content_range) {
                    Some((s, _, t)) if e.status == 206 => (s, t),
                    _ => (0, body.len()),
                };
                let last = b.min(base + body.len() - 1);
                let part = body[a - base..=last - base].to_vec();
                headers.retain(|(n, _)| !["content-length", "content-range"].contains(&n.to_ascii_lowercase().as_str()));
                headers.push(("Content-Range".into(), format!("bytes {a}-{last}/{total}")));
                headers.push(("Content-Length".into(), part.len().to_string()));
                (206, "Partial Content".to_string(), part)
            }
        };
        headers.push(("X-Wire-Replay".into(), "hit".into()));
        crate::write_status_and_headers(w, status, &reason, &headers)?;
        w.write_all(&body)?;
        w.flush()
    }
}
