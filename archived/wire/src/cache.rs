// W-CACHE1: wire's private HTTP cache (RFC 9111), on disk only.
//
// No cache data lives in wire's memory: no index, no hot-body tier. A lookup
// is one `open` of `<dir>/<sha256(top site)>/<sha256(cache key)>`; hot entries
// stay in the kernel page cache. An entry file is
//
//   `WIRECACHE1 <body length, 20 digits>\n` (32 bytes)
//   one JSON line of metadata (status, headers, times, Vary values, classes)
//   the body as wire emitted it: uncompressed when wire undid gzip/br, else as sent
//
// Bodies are streamed into `.tmp-<pid>-<n>` in the partition directory while
// they are copied to the client and renamed into place only once complete, so
// a reader never sees a partial entry and several wire processes may share one
// directory. Eviction deletes the oldest-mtime files (a hit touches mtime)
// whenever the running byte count goes over `--cache-size`, at start-up and
// once a minute while over. Details: NOTES.md "The HTTP cache".

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use url::Url;

use crate::record::sha256_hex;
use crate::{CACHE_HDR, Caps, ParsedRequest, header, now_unix};

/// `--cache-size` default: 1 GiB.
pub const DEFAULT_SIZE: u64 = 1 << 30;
const MAGIC: &str = "WIRECACHE1 ";
const PREAMBLE_LEN: usize = 32;
/// Longest metadata line accepted when reading an entry.
const MAX_META: u64 = 1 << 20;
/// Longest response head the cache writer collects before giving up storing.
const MAX_HEAD: usize = 256 * 1024;
/// A temp file older than this is a leftover of a crashed writer.
const STALE_TMP: Duration = Duration::from_secs(3600);
const HEURISTIC_CAP: i64 = 7 * 86400;
const EVICT_PERIOD: Duration = Duration::from_secs(60);

/// `$XDG_CACHE_HOME/wire`, falling back to `~/.cache/wire`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(x.join("wire"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache").join("wire"))
}

/// `256M`, `1G`, `512K`, `1048576` (binary units; a trailing `B`/`iB` is fine).
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let t = t.strip_suffix("iB").or_else(|| t.strip_suffix('B')).unwrap_or(t);
    let (num, mul) = match t.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('M') => (&t[..t.len() - 1], 1 << 20),
        Some('G') => (&t[..t.len() - 1], 1 << 30),
        Some('T') => (&t[..t.len() - 1], 1 << 40),
        _ => (t, 1),
    };
    match num.trim().parse::<u64>().ok().and_then(|n| n.checked_mul(mul)) {
        Some(n) if n > 0 => Ok(n),
        _ => Err(format!("--cache-size: {s:?} is not a size (e.g. 256M, 1G)")),
    }
}

/// True when `dir` lives on tmpfs/ramfs (its pages cannot be given back).
pub fn on_tmpfs(dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(dir) else { return false };
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else { return false };
    let mut best: Option<(usize, bool)> = None;
    for line in mounts.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() < 3 {
            continue;
        }
        let mp = f[1].replace("\\040", " ");
        if real.starts_with(&mp) && best.is_none_or(|(l, _)| mp.len() >= l) {
            best = Some((mp.len(), matches!(f[2], "tmpfs" | "ramfs")));
        }
    }
    best.is_some_and(|b| b.1)
}

// ---------- Cache-Control ----------

#[derive(Default, Debug, Clone)]
pub struct Cc {
    pub no_store: bool,
    pub no_cache: bool,
    pub must_revalidate: bool,
    pub public: bool,
    pub immutable: bool,
    pub only_if_cached: bool,
    pub max_age: Option<i64>,
    pub s_maxage: Option<i64>,
    /// `max-stale` (None inside: no value, any staleness).
    pub max_stale: Option<Option<i64>>,
}

pub fn parse_cc(headers: &[(String, String)]) -> Cc {
    let mut cc = Cc::default();
    for (n, v) in headers {
        if !n.eq_ignore_ascii_case("cache-control") {
            continue;
        }
        for d in split_list(v) {
            let (name, val) = match d.split_once('=') {
                Some((a, b)) => (a.trim().to_ascii_lowercase(), Some(b.trim().trim_matches('"'))),
                None => (d.trim().to_ascii_lowercase(), None),
            };
            let secs = || val.and_then(|x| x.parse::<i64>().ok()).map(|x| x.max(0));
            match name.as_str() {
                "no-store" => cc.no_store = true,
                "no-cache" => cc.no_cache = true,
                "must-revalidate" | "proxy-revalidate" => cc.must_revalidate = cc.must_revalidate || name == "must-revalidate",
                "public" => cc.public = true,
                "immutable" => cc.immutable = true,
                "only-if-cached" => cc.only_if_cached = true,
                // Duplicates: the first one counts (conservative enough here).
                "max-age" => cc.max_age = cc.max_age.or(Some(secs().unwrap_or(0))),
                "s-maxage" => cc.s_maxage = cc.s_maxage.or(secs()),
                "max-stale" => cc.max_stale = Some(secs()),
                _ => {}
            }
        }
    }
    cc
}

/// Comma list items, ignoring commas inside quoted strings.
fn split_list(v: &str) -> Vec<&str> {
    let (mut out, mut start, mut quoted) = (Vec::new(), 0, false);
    for (i, c) in v.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(v[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(v[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

/// What the request's own headers ask of the cache (the Fetch `cache` modes).
#[derive(Debug, Default)]
pub struct ReqCc {
    pub cc: Cc,
    /// `Cache-Control: no-cache` or `Pragma: no-cache`: fetch unconditionally.
    pub reload: bool,
}

pub fn parse_req_cc(headers: &[(String, String)]) -> ReqCc {
    let cc = parse_cc(headers);
    let pragma = headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("pragma") && v.to_ascii_lowercase().contains("no-cache"));
    ReqCc { reload: cc.no_cache || pragma, cc }
}

pub fn parse_date(s: &str) -> Option<i64> {
    crate::cookies::parse_cookie_date(s)
}

/// An IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) for unix seconds.
#[cfg(test)]
pub fn http_date(t: i64) -> String {
    const D: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{}, {d:02} {} {y} {:02}:{:02}:{:02} GMT", D[days.rem_euclid(7) as usize], M[(m - 1) as usize], secs / 3600, secs / 60 % 60, secs % 60)
}

// ---------- entries ----------

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub url: String,
    pub req_time: i64,
    pub resp_time: i64,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    /// Request header values named by `Vary`, as sent (None = absent).
    pub vary: Vec<(String, Option<String>)>,
    pub target_class: Option<String>,
    pub hop_classes: Option<String>,
    pub final_url: String,
}

impl Meta {
    fn to_json(&self) -> String {
        let pairs = |v: &[(String, String)]| v.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>();
        json!({
            "url": self.url, "req": self.req_time, "resp": self.resp_time, "status": self.status, "reason": self.reason,
            "headers": pairs(&self.headers),
            "vary": self.vary.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "class": self.target_class, "hops": self.hop_classes, "final": self.final_url,
        })
        .to_string()
    }

    fn from_json(s: &str) -> Option<Meta> {
        let v: Value = serde_json::from_str(s).ok()?;
        let st = |k: &str| v[k].as_str().map(String::from);
        let pairs = |k: &str| -> Option<Vec<(String, String)>> {
            v[k].as_array()?.iter().map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string()))).collect()
        };
        Some(Meta {
            url: st("url")?,
            req_time: v["req"].as_i64()?,
            resp_time: v["resp"].as_i64()?,
            status: u16::try_from(v["status"].as_u64()?).ok()?,
            reason: st("reason").unwrap_or_default(),
            headers: pairs("headers")?,
            vary: v["vary"].as_array()?.iter().map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str().map(String::from)))).collect::<Option<_>>()?,
            target_class: st("class"),
            hop_classes: st("hops"),
            final_url: st("final").unwrap_or_default(),
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }

    /// RFC 9111 4.2.1 (private cache: `s-maxage` ignored).
    pub fn freshness_lifetime(&self) -> i64 {
        let cc = parse_cc(&self.headers);
        if let Some(m) = cc.max_age {
            return m;
        }
        let date = self.header("date").and_then(parse_date).unwrap_or(self.resp_time);
        if let Some(exp) = self.header("expires") {
            // An invalid Expires (e.g. `0`) means "already expired".
            return parse_date(exp).map_or(0, |e| (e - date).max(0));
        }
        if (heuristic_status(self.status) || cc.public)
            && let Some(lm) = self.header("last-modified").and_then(parse_date)
        {
            return ((date - lm).max(0) / 10).min(HEURISTIC_CAP);
        }
        0
    }

    /// RFC 9111 4.2.3.
    pub fn current_age(&self, now: i64) -> i64 {
        let date = self.header("date").and_then(parse_date).unwrap_or(self.resp_time);
        let apparent = (self.resp_time - date).max(0);
        let age_value = self.header("age").and_then(|a| a.trim().parse::<i64>().ok()).unwrap_or(0).max(0);
        let corrected = age_value + (self.resp_time - self.req_time).max(0);
        apparent.max(corrected) + (now - self.resp_time).max(0)
    }

    fn has_validator(&self) -> bool {
        self.header("etag").is_some() || self.header("last-modified").is_some()
    }

    /// Every hop of the stored answer was a public address (W-POOL1 class).
    fn all_public(&self) -> bool {
        self.target_class.as_deref() == Some("public") && self.hop_classes.as_deref().is_some_and(|h| h.split(',').all(|c| c.trim() == "public"))
    }
}

/// Statuses a cache may give heuristic freshness (RFC 9110 15.1), minus the
/// redirects wire does not store in v1 and 206 (never stored).
fn heuristic_status(s: u16) -> bool {
    matches!(s, 200 | 203 | 204 | 300 | 404 | 405 | 410 | 414 | 501)
}

/// An entry opened for reading. The open file stays readable even when an
/// eviction (here or in another wire) unlinks it meanwhile.
pub struct Entry {
    file: File,
    pub path: PathBuf,
    pub meta: Meta,
    body_off: u64,
    pub body_len: u64,
}

impl Entry {
    /// `None` for a missing (or vanished), unreadable or corrupt file.
    pub fn open(path: &Path) -> Option<Entry> {
        let file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let mut r = BufReader::new(&file);
        let mut pre = [0u8; PREAMBLE_LEN];
        let parsed = (|| {
            r.read_exact(&mut pre).ok()?;
            let pre = std::str::from_utf8(&pre).ok()?;
            let body_len: u64 = pre.strip_prefix(MAGIC)?.trim_end_matches('\n').parse().ok()?;
            let mut line = String::new();
            (&mut r).take(MAX_META).read_line(&mut line).ok()?;
            let meta = Meta::from_json(line.trim_end_matches('\n'))?;
            let body_off = (PREAMBLE_LEN + line.len()) as u64;
            (body_off + body_len == len).then_some((meta, body_off, body_len))
        })();
        drop(r);
        let Some((meta, body_off, body_len)) = parsed else {
            // Renames are atomic, so this is an old format or a damaged disk.
            let _ = std::fs::remove_file(path);
            return None;
        };
        Some(Entry { file, path: path.to_path_buf(), meta, body_off, body_len })
    }

    fn touch(&self) {
        let _ = self.file.set_modified(SystemTime::now());
    }

    fn body_reader(&self, start: u64, len: u64) -> std::io::Result<impl Read + '_> {
        let mut f = &self.file;
        f.seek(SeekFrom::Start(self.body_off + start))?;
        Ok(f.take(len))
    }
}

fn preamble(body_len: u64) -> String {
    format!("{MAGIC}{body_len:020}\n")
}

// ---------- the cache ----------

pub struct Cache {
    pub dir: PathBuf,
    pub limit: u64,
    /// Bytes in the directory: exact after a scan, plus what this process
    /// stored since. Not an index: only decides when to scan.
    approx: AtomicU64,
    seq: AtomicU64,
    write_warned: AtomicBool,
    kick: Mutex<Option<mpsc::Sender<()>>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct EvictStats {
    pub before: u64,
    pub after: u64,
    pub removed: usize,
    pub vanished: usize,
}

/// One scanned file: (mtime ns, size, partition index, name bytes).
type Item = (i128, u64, u32, [u8; 32]);

fn hex32(name: &str) -> Option<[u8; 32]> {
    if name.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(name.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn mtime_ns(md: &std::fs::Metadata) -> i128 {
    md.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i128)
}

impl Cache {
    /// Creates `dir` (mode 0700). Does not scan: `start_evictor` does.
    pub fn open(dir: &Path, limit: u64) -> Result<Cache, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot chmod cache dir {}: {e}", dir.display()))?;
        Ok(Cache { dir: dir.to_path_buf(), limit, approx: AtomicU64::new(0), seq: AtomicU64::new(0), write_warned: AtomicBool::new(false), kick: Mutex::new(None) })
    }

    /// Largest body stored (Chromium's rule: an eighth of the cache).
    pub fn max_entry(&self) -> u64 {
        (self.limit / 8).max(1)
    }

    pub fn part_dir(&self, partition: &str) -> PathBuf {
        self.dir.join(sha256_hex(partition.as_bytes()))
    }

    pub fn entry_path(&self, partition: &str, key: &str) -> PathBuf {
        self.part_dir(partition).join(sha256_hex(key.as_bytes()))
    }

    /// W-CACHE2: forget-site. Removes one partition's directory (one
    /// removal, no scan). Open readers keep their file; an in-flight store
    /// in that partition fails its rename and is dropped. Returns whether
    /// there was a directory.
    pub fn clear_partition(&self, partition: &str) -> bool {
        std::fs::remove_dir_all(self.part_dir(partition)).is_ok()
    }

    /// W-CACHE2 `Clear-Site-Data: "cache"`: removes the entries of one origin
    /// in one partition (a scan of that partition only). Returns how many.
    pub fn clear_origin(&self, partition: &str, origin: &url::Origin) -> usize {
        let Ok(rd) = std::fs::read_dir(self.part_dir(partition)) else { return 0 };
        let mut n = 0;
        for f in rd.flatten() {
            if f.file_name().to_str().and_then(hex32).is_none() {
                continue; // temp files of in-flight stores
            }
            if Entry::open(&f.path()).is_some_and(|e| Url::parse(&e.meta.url).is_ok_and(|u| u.origin() == *origin)) && std::fs::remove_file(f.path()).is_ok() {
                n += 1;
            }
        }
        n
    }

    fn new_tmp(&self, part: &Path) -> std::io::Result<(PathBuf, File)> {
        let mut last = None;
        for _ in 0..2 {
            if !part.is_dir() {
                std::fs::create_dir_all(part)?;
                let _ = std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o700));
            }
            let p = part.join(format!(".tmp-{}-{}", std::process::id(), self.seq.fetch_add(1, Ordering::Relaxed)));
            match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&p) {
                Ok(f) => return Ok((p, f)),
                // An eviction removed the empty partition directory meanwhile: once more.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("cannot create a temp file")))
    }

    /// A write error or a full disk: don't store, one warning line (again only
    /// after a store has worked in between).
    fn write_failed(&self, e: &std::io::Error) {
        if !self.write_warned.swap(true, Ordering::Relaxed) {
            eprintln!("wire: cache: cannot store in {}: {e}; answering from the network without storing", self.dir.display());
        }
    }

    fn stored(&self, bytes: u64) {
        self.write_warned.store(false, Ordering::Relaxed);
        let now = self.approx.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if now > self.limit
            && let Some(tx) = self.kick.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        {
            let _ = tx.send(());
        }
    }

    /// Bytes this process believes are stored (for tests and logs).
    pub fn approx_bytes(&self) -> u64 {
        self.approx.load(Ordering::Relaxed)
    }

    /// A thread that scans at start-up, then whenever a store took the cache
    /// over its limit, and once a minute while it is over.
    pub fn start_evictor(self: &Arc<Self>) {
        let (tx, rx) = mpsc::channel();
        *self.kick.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            loop {
                let s = me.evict_now();
                if s.removed > 0 && crate::timing_enabled() {
                    eprintln!("wire: cache evicted {} files, {} -> {} bytes", s.removed, s.before, s.after);
                }
                loop {
                    match rx.recv_timeout(EVICT_PERIOD) {
                        Ok(()) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) if me.approx_bytes() > me.limit => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                while rx.try_recv().is_ok() {}
            }
        });
    }

    /// Lists every file (entries and temp files). Old temp files are
    /// deleted on the way. A file or directory vanishing mid-scan (another
    /// wire evicting) is skipped, never an error.
    pub fn scan(&self) -> (Vec<PathBuf>, Vec<Item>, u64) {
        let (mut parts, mut items, mut total) = (Vec::new(), Vec::new(), 0u64);
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return (parts, items, total) };
        let now = SystemTime::now();
        for d in rd.flatten() {
            if !d.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(d.path()) else { continue };
            let idx = parts.len() as u32;
            parts.push(d.path());
            for f in files.flatten() {
                let Ok(md) = f.metadata() else { continue };
                if !md.is_file() {
                    continue;
                }
                let name = f.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(".tmp-") {
                    if md.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|a| a > STALE_TMP) {
                        let _ = std::fs::remove_file(f.path());
                    } else {
                        total += md.len();
                    }
                    continue;
                }
                total += md.len();
                if let Some(h) = hex32(&name) {
                    items.push((mtime_ns(&md), md.len(), idx, h));
                }
            }
        }
        (parts, items, total)
    }

    /// Deletes oldest-mtime first until the total is at most 90% of the limit.
    pub fn evict_items(&self, parts: &[PathBuf], mut items: Vec<Item>, total: u64) -> EvictStats {
        let mut s = EvictStats { before: total, after: total, ..EvictStats::default() };
        if total > self.limit {
            let target = self.limit / 10 * 9;
            items.sort_unstable_by_key(|i| i.0);
            let mut touched = Vec::new();
            for (_, size, idx, name) in items {
                if s.after <= target {
                    break;
                }
                let hex: String = name.iter().map(|b| format!("{b:02x}")).collect();
                match std::fs::remove_file(parts[idx as usize].join(hex)) {
                    Ok(()) => s.removed += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => s.vanished += 1,
                    Err(_) => continue,
                }
                s.after = s.after.saturating_sub(size);
                if !touched.contains(&idx) {
                    touched.push(idx);
                }
            }
            for idx in touched {
                let _ = std::fs::remove_dir(&parts[idx as usize]); // only when empty
            }
        }
        self.approx.store(s.after, Ordering::Relaxed);
        s
    }

    pub fn evict_now(&self) -> EvictStats {
        let (parts, items, total) = self.scan();
        self.evict_items(&parts, items, total)
    }

    /// Removes the stored GET entries for `url` (both jar modes) in `partition`.
    fn invalidate(&self, partition: &str, url: &Url) {
        for jar in [true, false] {
            let _ = std::fs::remove_file(self.entry_path(partition, &cache_key(jar, url)));
        }
    }
}

/// The cache key inside a partition: jar on/off, method (only GET is
/// stored), URL without fragment.
pub fn cache_key(use_jar: bool, url: &Url) -> String {
    let mut u = url.clone();
    u.set_fragment(None);
    format!("{}\nGET\n{u}", if use_jar { "jar" } else { "nojar" })
}

/// The partition of a request: the normalized `X-Wire-Top-Site`, or ""
/// (the unpartitioned bucket) without one.
pub fn partition_of(headers: &[(String, String)]) -> String {
    match header(headers, "x-wire-top-site") {
        Some(t) => crate::cookies::normalize_site(t).unwrap_or_else(|| t.trim().to_ascii_lowercase()),
        None => String::new(),
    }
}

// ---------- the request path ----------

/// What the request looks like to `Vary`, as wire sends it upstream.
pub struct ReqInfo<'a> {
    pub use_jar: bool,
    /// The User-Agent wire actually sends (after `X-Wire-User-Agent`).
    pub ua: &'a str,
    /// The `Cookie` header wire sends (the jar's on the jar path).
    pub cookie: Option<String>,
}

impl ReqInfo<'_> {
    fn vary_value(&self, req: &ParsedRequest, name: &str) -> Option<String> {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "user-agent" => Some(self.ua.to_string()),
            "cookie" => self.cookie.clone(),
            _ => {
                let v: Vec<&str> = req.headers.iter().filter(|(h, _)| h.eq_ignore_ascii_case(&n)).map(|(_, v)| v.trim()).collect();
                (!v.is_empty()).then(|| v.join(", "))
            }
        }
    }

    /// Vary names of a response (lowercase; `Accept-Encoding` is wire's own
    /// choice and ignored). `None` for `Vary: *`.
    fn vary_names(headers: &[(String, String)]) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for (n, v) in headers {
            if n.eq_ignore_ascii_case("vary") {
                for x in split_list(v) {
                    let x = x.to_ascii_lowercase();
                    if x == "*" {
                        return None;
                    }
                    if x != "accept-encoding" && !out.contains(&x) {
                        out.push(x);
                    }
                }
            }
        }
        Some(out)
    }

    fn vary_matches(&self, req: &ParsedRequest, meta: &Meta) -> bool {
        meta.vary.iter().all(|(n, v)| norm_ws(self.vary_value(req, n).as_deref()) == norm_ws(v.as_deref()))
    }
}

fn norm_ws(v: Option<&str>) -> Option<String> {
    v.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The upstream call: sends `req` and writes wire's normal response.
pub type Network<'a> = dyn FnMut(&ParsedRequest, &mut dyn Write) -> std::io::Result<()> + 'a;

fn set_hdr(v: &'static str) {
    CACHE_HDR.with(|c| c.set(v));
}

fn is_safe(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "OPTIONS" | "TRACE")
}

/// One request through the cache. Everything that is not GET/HEAD goes
/// upstream; an unsafe method's 2xx/3xx invalidates (RFC 9111 4.4).
pub fn handle(cache: &Cache, req: &ParsedRequest, caps: &Caps, info: &ReqInfo, w: &mut dyn Write, net: &mut Network) -> std::io::Result<()> {
    let partition = partition_of(&req.headers);
    if !matches!(req.method.as_str(), "GET" | "HEAD") {
        set_hdr("bypass");
        let mut cw = CacheWriter::new(w, Plan::Observe);
        let r = net(req, &mut cw);
        if !is_safe(&req.method)
            && let Some(h) = &cw.head
            && (200..400).contains(&h.status)
        {
            cache.invalidate(&partition, &req.url);
            for name in ["location", "content-location"] {
                if let Some(u) = header(&h.headers, name).and_then(|l| req.url.join(l).ok()).filter(|u| u.origin() == req.url.origin()) {
                    cache.invalidate(&partition, &u);
                }
            }
        }
        return r;
    }
    let rq = parse_req_cc(&req.headers);
    if rq.cc.no_store {
        set_hdr("bypass");
        return net(req, w);
    }
    let range = header(&req.headers, "range");
    let client_conditional = ["if-none-match", "if-modified-since"].iter().any(|n| header(&req.headers, n).is_some());
    let path = cache.entry_path(&partition, &cache_key(info.use_jar, &req.url));
    let now = now_unix();
    // Security (W-POOL1 class): under `public-only` an entry is usable only
    // when every hop it came from was public. This runs BEFORE anything is
    // served from it, including a 304 or a revalidation.
    let entry = if rq.reload {
        None
    } else {
        Entry::open(&path).filter(|e| !caps.public_only || e.meta.all_public()).filter(|e| info.vary_matches(req, &e.meta))
    };
    if let Some(e) = &entry {
        let age = e.meta.current_age(now);
        let life = e.meta.freshness_lifetime();
        let rcc = parse_cc(&e.meta.headers);
        let fresh = age < life;
        let usable = if rcc.no_cache {
            None
        } else if let Some(ma) = rq.cc.max_age {
            // `max-age=0` (cache mode no-cache, normal reload): revalidate,
            // except `immutable` answers that are still fresh.
            // max-age=0 always revalidates (as browsers treat it), even at age 0.
            (fresh && ((ma > 0 && age <= ma) || rcc.immutable)).then_some("hit")
        } else if fresh {
            Some("hit")
        } else if !rcc.must_revalidate && (rq.cc.only_if_cached || rq.cc.max_stale.is_some_and(|m| m.is_none_or(|m| age - life <= m))) {
            Some("stale")
        } else {
            None
        };
        if let Some(how) = usable {
            if client_conditional {
                if validators_match(req, &e.meta) {
                    e.touch();
                    return write_not_modified(e, req, how, now, w);
                }
                // The client's own conditional did not match: its answer
                // (a full 200) is stored as usual below.
            } else if header(&req.headers, "if-range").is_none() || range.is_none() {
                e.touch();
                return serve_entry(e, req, caps, how, now, w);
            }
        }
    }
    if rq.cc.only_if_cached {
        set_hdr("miss");
        crate::write_status_and_headers(
            w,
            504,
            "Gateway Timeout",
            &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "only-if-cached".to_string())],
        )?;
        w.write_all(b"wire: only-if-cached and nothing usable is stored")?;
        return w.flush();
    }
    set_hdr("miss");
    if range.is_some() || req.method == "HEAD" {
        // A partial or headless answer is never stored.
        return net(req, w);
    }
    // Revalidate a stored answer with its validators (wire's own conditional).
    let reval = entry.filter(|e| e.meta.has_validator() && !client_conditional);
    let mut sent = req.clone();
    if let Some(e) = &reval {
        if let Some(t) = e.meta.header("etag") {
            sent.headers.push(("If-None-Match".into(), t.to_string()));
        }
        if let Some(lm) = e.meta.header("last-modified") {
            sent.headers.push(("If-Modified-Since".into(), lm.to_string()));
        }
    }
    let store = StoreCtx { cache, partition: &partition, path: &path, req, info, req_time: now, max_bytes: caps.max_bytes };
    let mut cw = CacheWriter::new(w, Plan::Store(store, reval.is_some()));
    let r = net(&sent, &mut cw);
    let not_modified = cw.swallowed.take();
    let finished = cw.finish(r.is_ok());
    r?;
    finished?;
    let (Some(h304), Some(mut e)) = (not_modified, reval) else { return Ok(()) };
    match merge_304(cache, &partition, &e, &h304, now) {
        Some(n) => e = n,
        None => {
            // Could not rewrite: serve the stored body with the merged headers.
            e.meta.headers = merged_headers(&e.meta.headers, &h304.headers);
            (e.meta.req_time, e.meta.resp_time) = (now, now_unix());
        }
    }
    serve_entry(&e, req, caps, "revalidated", now_unix(), w)
}

fn etag_list_match(list: &str, tag: &str) -> bool {
    let weak = |t: &str| t.trim().trim_start_matches("W/").to_string();
    list.trim() == "*" || split_list(list).iter().any(|t| weak(t) == weak(tag))
}

/// The client's own conditional is answered by the stored validators.
fn validators_match(req: &ParsedRequest, meta: &Meta) -> bool {
    if let Some(inm) = header(&req.headers, "if-none-match") {
        return meta.header("etag").is_some_and(|t| etag_list_match(inm, t));
    }
    match (header(&req.headers, "if-modified-since").and_then(parse_date), meta.header("last-modified").and_then(parse_date)) {
        (Some(ims), Some(lm)) => lm <= ims,
        _ => false,
    }
}

fn entry_headers(e: &Entry, now: i64) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = e.meta.headers.iter().filter(|(n, _)| !n.eq_ignore_ascii_case("age")).cloned().collect();
    h.push(("Age".into(), e.meta.current_age(now).max(0).to_string()));
    h.push(("X-Wire-Final-Url".into(), e.meta.final_url.clone()));
    if let (Some(c), Some(hops)) = (&e.meta.target_class, &e.meta.hop_classes) {
        h.push(("X-Wire-Target-Class".into(), c.clone()));
        h.push(("X-Wire-Hop-Classes".into(), hops.clone()));
    }
    h
}

fn write_not_modified(e: &Entry, req: &ParsedRequest, how: &'static str, now: i64, w: &mut dyn Write) -> std::io::Result<()> {
    set_hdr(how);
    let keep = ["cache-control", "content-location", "date", "etag", "expires", "vary", "last-modified"];
    let mut h: Vec<(String, String)> = entry_headers(e, now).into_iter().filter(|(n, _)| keep.contains(&n.to_ascii_lowercase().as_str()) || n.to_ascii_lowercase().starts_with("x-wire-") || n == "Age").collect();
    h.dedup();
    crate::write_status_and_headers(w, 304, "Not Modified", &h)?;
    crate::log_timing(&req.method, &req.url, 304, Duration::ZERO, Duration::ZERO);
    w.flush()
}

/// One `bytes=` range against `total`: `Some(Ok((start, end_inclusive)))`,
/// `Some(Err(()))` when unsatisfiable, `None` when absent, multi-range or
/// unparsable (answer the full body, RFC 9110 14.2).
pub fn parse_byte_range(spec: &str, total: u64) -> Option<Result<(u64, u64), ()>> {
    let r = spec.trim().strip_prefix("bytes=")?.trim();
    if r.contains(',') {
        return None;
    }
    let (a, b) = r.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 || total == 0 {
            return Some(Err(()));
        }
        return Some(Ok((total.saturating_sub(n), total - 1)));
    }
    let start: u64 = a.parse().ok()?;
    let end: Option<u64> = if b.is_empty() { None } else { Some(b.parse().ok()?) };
    if end.is_some_and(|e| e < start) {
        return None;
    }
    if start >= total {
        return Some(Err(()));
    }
    Some(Ok((start, end.map_or(total - 1, |e| e.min(total - 1)))))
}

/// Answers from a stored entry: 200 (or 206/416 for a `Range`, HEAD without
/// body), `Age` computed, the class headers replayed from the entry.
fn serve_entry(e: &Entry, req: &ParsedRequest, caps: &Caps, how: &'static str, now: i64, w: &mut dyn Write) -> std::io::Result<()> {
    set_hdr(how);
    let mut headers = entry_headers(e, now);
    let total = e.body_len;
    let (mut status, mut reason, mut start, mut len) = (e.meta.status, e.meta.reason.clone(), 0, total);
    if e.meta.status == 200
        && let Some(r) = header(&req.headers, "range").and_then(|r| parse_byte_range(r, total))
    {
        headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-range"));
        match r {
            Ok((a, b)) => {
                (status, reason, start, len) = (206, "Partial Content".into(), a, b - a + 1);
                headers.push(("Content-Range".into(), format!("bytes {a}-{b}/{total}")));
            }
            Err(()) => {
                (status, reason, len) = (416, "Range Not Satisfiable".into(), 0);
                headers.push(("Content-Range".into(), format!("bytes */{total}")));
            }
        }
        if header(&headers, "accept-ranges").is_none() {
            headers.push(("Accept-Ranges".into(), "bytes".into()));
        }
    }
    if let Some(max) = caps.max_bytes
        && len > max as u64
        && req.method != "HEAD"
    {
        crate::write_too_large(w, max)?;
        return Ok(());
    }
    crate::write_status_and_headers(w, status, &reason, &headers)?;
    if req.method != "HEAD" && len > 0 {
        crate::copy_body(e.body_reader(start, len)?, w)?;
    }
    w.flush()?;
    crate::log_timing(&req.method, &req.url, status, Duration::ZERO, Duration::ZERO);
    if crate::timing_enabled() {
        eprintln!("wire: cache {how} {}", req.url);
    }
    Ok(())
}

/// RFC 9111 4.3.4: the 304's fields replace the stored ones.
fn merged_headers(stored: &[(String, String)], fresh: &[(String, String)]) -> Vec<(String, String)> {
    let skip = |n: &str| {
        let n = n.to_ascii_lowercase();
        n.starts_with("x-wire-") || matches!(n.as_str(), "content-length" | "transfer-encoding" | "content-encoding" | "set-cookie" | "content-range")
    };
    let names: Vec<String> = fresh.iter().filter(|(n, _)| !skip(n)).map(|(n, _)| n.to_ascii_lowercase()).collect();
    let mut out: Vec<(String, String)> = stored.iter().filter(|(n, _)| !names.contains(&n.to_ascii_lowercase())).cloned().collect();
    out.extend(fresh.iter().filter(|(n, _)| !skip(n)).cloned());
    out
}

/// Rewrites the entry with the 304's headers (temp file + rename, the body
/// copied from the open entry). `None` when that failed.
fn merge_304(cache: &Cache, partition: &str, e: &Entry, h: &Head, req_time: i64) -> Option<Entry> {
    let mut meta = e.meta.clone();
    meta.headers = merged_headers(&e.meta.headers, &h.headers);
    meta.req_time = req_time;
    meta.resp_time = now_unix();
    if let (Some(c), Some(hops)) = (&h.class, &h.hops) {
        meta.target_class = Some(c.clone());
        meta.hop_classes = Some(hops.clone());
    }
    let part = cache.part_dir(partition);
    let result = (|| -> std::io::Result<PathBuf> {
        let (tmp, f) = cache.new_tmp(&part)?;
        let r = (|| {
            let mut bw = std::io::BufWriter::new(f);
            bw.write_all(preamble(e.body_len).as_bytes())?;
            bw.write_all(meta.to_json().as_bytes())?;
            bw.write_all(b"\n")?;
            std::io::copy(&mut e.body_reader(0, e.body_len)?, &mut bw)?;
            bw.flush()?;
            std::fs::rename(&tmp, &e.path)
        })();
        if let Err(err) = r {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
        Ok(e.path.clone())
    })();
    match result {
        Ok(p) => {
            let out = Entry::open(&p);
            if let Some(n) = &out {
                cache.stored(n.body_off + n.body_len);
            }
            out
        }
        Err(err) => {
            cache.write_failed(&err);
            None
        }
    }
}

// ---------- the writer between wire's response and the client ----------

/// A parsed response head as wire wrote it.
pub struct Head {
    pub status: u16,
    reason: String,
    pub headers: Vec<(String, String)>,
    class: Option<String>,
    hops: Option<String>,
    final_url: Option<String>,
    error: bool,
}

fn parse_head(raw: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next()?;
    let mut parts = status_line.splitn(3, ' ');
    parts.next()?;
    let status: u16 = parts.next()?.parse().ok()?;
    let reason = parts.next().unwrap_or("").to_string();
    let mut h = Head { status, reason, headers: Vec::new(), class: None, hops: None, final_url: None, error: false };
    for l in lines.filter(|l| !l.is_empty()) {
        let (n, v) = l.split_once(':')?;
        let (n, v) = (n.trim(), v.trim());
        match n.to_ascii_lowercase().as_str() {
            "x-wire-target-class" => h.class = Some(v.to_string()),
            "x-wire-hop-classes" => h.hops = Some(v.to_string()),
            "x-wire-final-url" => h.final_url = Some(v.to_string()),
            "x-wire-error" => h.error = true,
            l if l.starts_with("x-wire-") => {}
            _ => h.headers.push((n.to_string(), v.to_string())),
        }
    }
    Some(h)
}

pub struct StoreCtx<'a> {
    cache: &'a Cache,
    partition: &'a str,
    path: &'a Path,
    req: &'a ParsedRequest,
    info: &'a ReqInfo<'a>,
    req_time: i64,
    max_bytes: Option<usize>,
}

enum Plan<'a> {
    /// Forward only; remember the head (invalidation).
    Observe,
    /// Forward and store when storable. `true`: wire sent its own
    /// conditional, so a 304 is swallowed (the caller answers from the store).
    Store(StoreCtx<'a>, bool),
}

struct Tmp {
    path: PathBuf,
    out: std::io::BufWriter<File>,
    written: u64,
}

struct CacheWriter<'a> {
    inner: &'a mut dyn Write,
    plan: Plan<'a>,
    raw_head: Vec<u8>,
    head: Option<Head>,
    tmp: Option<Tmp>,
    /// The 304 to wire's own conditional (never reaches the client).
    swallowed: Option<Head>,
    gave_up: bool,
}

impl<'a> CacheWriter<'a> {
    fn new(inner: &'a mut dyn Write, plan: Plan<'a>) -> Self {
        CacheWriter { inner, plan, raw_head: Vec::new(), head: None, tmp: None, swallowed: None, gave_up: false }
    }

    /// Whether this answer may be stored (RFC 9111 3, wire's v1 limits).
    fn storable(ctx: &StoreCtx, h: &Head) -> bool {
        let cc = parse_cc(&h.headers);
        let status_ok = !matches!(h.status, 100..=199 | 206 | 300..=399) && h.status < 600;
        let explicit = cc.max_age.is_some() || header(&h.headers, "expires").is_some();
        let meta_probe = Meta { status: h.status, headers: h.headers.clone(), resp_time: now_unix(), ..Meta::default() };
        let useful = meta_probe.freshness_lifetime() > 0 || meta_probe.has_validator();
        let auth_ok = header(&ctx.req.headers, "authorization").is_none() || cc.public || cc.must_revalidate || cc.s_maxage.is_some();
        let announced = header(&h.headers, "content-length").and_then(|v| v.parse::<u64>().ok());
        // A redirect-free answer only (v1): its final URL is the request URL.
        let same_url = h.final_url.as_deref().and_then(|f| Url::parse(f).ok()).is_some_and(|mut f| {
            f.set_fragment(None);
            let mut u = ctx.req.url.clone();
            u.set_fragment(None);
            f == u
        });
        status_ok
            && (explicit || heuristic_status(h.status) || cc.public)
            && useful
            && !cc.no_store
            // W-CACHE2: an answer that clears site data is never stored itself.
            && header(&h.headers, "clear-site-data").is_none()
            && !h.error
            && auth_ok
            && same_url
            && ReqInfo::vary_names(&h.headers).is_some()
            && announced.is_none_or(|n| n <= ctx.cache.max_entry())
    }

    fn on_head(&mut self) -> std::io::Result<()> {
        let end = self.raw_head.windows(4).position(|x| x == b"\r\n\r\n").map(|p| p + 4).unwrap_or(self.raw_head.len());
        let head = parse_head(&self.raw_head[..end.saturating_sub(4)]);
        let rest = self.raw_head.split_off(end);
        if let (Plan::Store(_, true), Some(h)) = (&self.plan, &head)
            && h.status == 304
        {
            self.swallowed = head;
            self.raw_head.clear();
            return Ok(());
        }
        self.inner.write_all(&self.raw_head)?;
        if let (Plan::Store(ctx, _), Some(h)) = (&self.plan, &head)
            && Self::storable(ctx, h)
        {
            self.tmp = Self::start_tmp(ctx, h);
        }
        self.head = head;
        if !rest.is_empty() {
            self.body(&rest)?;
        }
        Ok(())
    }

    fn start_tmp(ctx: &StoreCtx, h: &Head) -> Option<Tmp> {
        let names = ReqInfo::vary_names(&h.headers)?;
        let meta = Meta {
            url: ctx.req.url.to_string(),
            req_time: ctx.req_time,
            resp_time: now_unix(),
            status: h.status,
            reason: h.reason.clone(),
            // The jar has seen `Set-Cookie`; a stored answer never replays it.
            headers: h.headers.iter().filter(|(n, _)| !n.eq_ignore_ascii_case("set-cookie")).cloned().collect(),
            vary: names.iter().map(|n| (n.clone(), ctx.info.vary_value(ctx.req, n))).collect(),
            target_class: h.class.clone(),
            hop_classes: h.hops.clone(),
            final_url: h.final_url.clone().unwrap_or_default(),
        };
        let r = (|| {
            let (path, f) = ctx.cache.new_tmp(&ctx.cache.part_dir(ctx.partition))?;
            let mut out = std::io::BufWriter::with_capacity(64 * 1024, f);
            let w = (|| {
                out.write_all(preamble(0).as_bytes())?;
                out.write_all(meta.to_json().as_bytes())?;
                out.write_all(b"\n")
            })();
            match w {
                Ok(()) => Ok(Tmp { path, out, written: 0 }),
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    Err(e)
                }
            }
        })();
        match r {
            Ok(t) => Some(t),
            Err(e) => {
                ctx.cache.write_failed(&e);
                None
            }
        }
    }

    fn body(&mut self, data: &[u8]) -> std::io::Result<()> {
        if self.swallowed.is_some() {
            return Ok(());
        }
        if let Err(e) = self.inner.write_all(data) {
            self.abandon(None);
            return Err(e);
        }
        if let Some(t) = &mut self.tmp {
            let max = match &self.plan {
                Plan::Store(ctx, _) => ctx.cache.max_entry(),
                Plan::Observe => 0,
            };
            t.written += data.len() as u64;
            if t.written > max {
                self.abandon(None);
            } else if let Err(e) = t.out.write_all(data) {
                self.abandon(Some(e));
            }
        }
        Ok(())
    }

    fn abandon(&mut self, err: Option<std::io::Error>) {
        if let Some(t) = self.tmp.take() {
            drop(t.out);
            let _ = std::fs::remove_file(&t.path);
            if let (Some(e), Plan::Store(ctx, _)) = (err, &self.plan) {
                ctx.cache.write_failed(&e);
            }
        }
    }

    /// After the network call: rename a complete body into place, else drop it.
    fn finish(mut self, ok: bool) -> std::io::Result<()> {
        if !self.raw_head.is_empty() && self.head.is_none() && self.swallowed.is_none() {
            // The head never ended (wire never does that): pass on what came.
            self.inner.write_all(&self.raw_head)?;
        }
        let info = crate::BODY_INFO.with(std::cell::Cell::get);
        let Some(mut t) = self.tmp.take() else { return Ok(()) };
        let Plan::Store(ctx, _) = &self.plan else { return Ok(()) };
        let announced = self.head.as_ref().and_then(|h| header(&h.headers, "content-length")).and_then(|v| v.parse::<u64>().ok()).or(info.announced);
        // W-MEDIA2: a cut body (upstream read error, length mismatch) is never stored.
        let complete = ok && !info.truncated && announced.is_none_or(|n| n == t.written) && ctx.max_bytes.is_none_or(|m| t.written <= m as u64);
        if !complete {
            drop(t.out);
            let _ = std::fs::remove_file(&t.path);
            return Ok(());
        }
        let r = (|| -> std::io::Result<u64> {
            t.out.flush()?;
            let mut f = t.out.into_inner().map_err(|e| e.into_error())?;
            f.seek(SeekFrom::Start(0))?;
            f.write_all(preamble(t.written).as_bytes())?;
            let len = f.metadata()?.len();
            std::fs::rename(&t.path, ctx.path)?;
            Ok(len)
        })();
        match r {
            Ok(len) => ctx.cache.stored(len),
            Err(e) => {
                let _ = std::fs::remove_file(&t.path);
                ctx.cache.write_failed(&e);
            }
        }
        Ok(())
    }
}

impl Write for CacheWriter<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.head.is_none() && self.swallowed.is_none() && !self.gave_up {
            self.raw_head.extend_from_slice(data);
            if self.raw_head.windows(4).any(|x| x == b"\r\n\r\n") {
                self.on_head()?;
            } else if self.raw_head.len() > MAX_HEAD {
                self.gave_up = true;
                let raw = std::mem::take(&mut self.raw_head);
                self.inner.write_all(&raw)?;
            }
            return Ok(data.len());
        }
        if self.gave_up {
            self.inner.write_all(data)?;
        } else {
            self.body(data)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.swallowed.is_some() { Ok(()) } else { self.inner.flush() }
    }
}
