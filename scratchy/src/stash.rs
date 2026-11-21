//! stash: the persistent `localStorage` store (QSTASH1; design in STASH.md, which wins over these comments).
//!
//! No daemon: scratchy reads and writes the state directory itself.
//!
//! ```text
//! DIR/storage/.locks/<site>             lock file (flock), holds the generation number; never deleted
//! DIR/storage/<site>/<origin>/<key>     one file per key; contents = WTF-8 value + "\n"
//! ```
//!
//! - `<site>` is `net_policy::site_of` of the top-level page with `://` written `!` ([`site_dir_name`]).
//! - `<origin>` is `scheme!host!port`, port always explicit ([`origin_dir_name`]). Opaque origins are never stored.
//! - Keys and values are UTF-16 code units (`Vec<u16>`, as V8 hands them over) at the API boundary and WTF-8 on
//!   disk, so lone surrogates round-trip; never a lossy UTF-8 conversion.
//!
//! This module has no JS wiring (QSTASH2) and no `--state-dir` flag (QSTASH2). Its public surface:
//! [`Stash::open`] / [`Stash::open_or_log`], [`Stash::load`], [`Stash::commit`], [`Stash::clear`],
//! [`Stash::forget`], and [`Writer`], the thread that runs commits off the page's thread.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

/// Per-origin limit, in UTF-16 code units of keys plus values (the unit JS `length` counts; Firefox's rule).
pub(crate) const QUOTA_UNITS: usize = 5 * 1024 * 1024;

/// An encoded key filename longer than this is stored as `%h<sha256 hex>` with the encoded key on its first line.
pub(crate) const MAX_NAME_BYTES: usize = 200;

/// What one key/value pair costs against [`QUOTA_UNITS`]. QSTASH2 checks the page's copy with the same rule.
pub(crate) fn entry_units(key: &[u16], value: &[u16]) -> usize {
    key.len() + value.len()
}

// ------------------------------------------------------------ WTF-8

/// UTF-16 code units -> WTF-8: UTF-8 where a lone surrogate is encoded like any other code point (3 bytes).
pub(crate) fn wtf8_encode(units: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i] as u32;
        let cp = if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() && (0xDC00..0xE000).contains(&(units[i + 1] as u32)) {
            i += 1;
            0x10000 + ((u - 0xD800) << 10) + (units[i] as u32 - 0xDC00)
        } else {
            u
        };
        i += 1;
        match cp {
            0..=0x7F => out.push(cp as u8),
            0x80..=0x7FF => out.extend_from_slice(&[0xC0 | (cp >> 6) as u8, 0x80 | (cp & 0x3F) as u8]),
            0x800..=0xFFFF => out.extend_from_slice(&[
                0xE0 | (cp >> 12) as u8,
                0x80 | ((cp >> 6) & 0x3F) as u8,
                0x80 | (cp & 0x3F) as u8,
            ]),
            _ => out.extend_from_slice(&[
                0xF0 | (cp >> 18) as u8,
                0x80 | ((cp >> 12) & 0x3F) as u8,
                0x80 | ((cp >> 6) & 0x3F) as u8,
                0x80 | (cp & 0x3F) as u8,
            ]),
        }
    }
    out
}

/// WTF-8 -> UTF-16 code units. `None` for bytes that are not (generalised) UTF-8, e.g. a hand-made Latin-1 file.
/// A 3-byte surrogate followed by another decodes to the two units as written (lenient; we never write that).
pub(crate) fn wtf8_decode(bytes: &[u8]) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let cont = |b: Option<&u8>| b.filter(|b| (0x80..0xC0).contains(*b)).map(|b| (b & 0x3F) as u32);
    while i < bytes.len() {
        let b = bytes[i];
        let (cp, n) = match b {
            0x00..=0x7F => (b as u32, 1),
            0xC2..=0xDF => (((b & 0x1F) as u32) << 6 | cont(bytes.get(i + 1))?, 2),
            0xE0..=0xEF => {
                let cp = ((b & 0x0F) as u32) << 12 | cont(bytes.get(i + 1))? << 6 | cont(bytes.get(i + 2))?;
                if cp < 0x800 {
                    return None;
                }
                (cp, 3)
            }
            0xF0..=0xF4 => {
                let cp = ((b & 0x07) as u32) << 18
                    | cont(bytes.get(i + 1))? << 12
                    | cont(bytes.get(i + 2))? << 6
                    | cont(bytes.get(i + 3))?;
                if !(0x10000..=0x10FFFF).contains(&cp) {
                    return None;
                }
                (cp, 4)
            }
            _ => return None,
        };
        i += n;
        if cp >= 0x10000 {
            let c = cp - 0x10000;
            out.push(0xD800 + (c >> 10) as u16);
            out.push(0xDC00 + (c & 0x3FF) as u16);
        } else {
            out.push(cp as u16);
        }
    }
    Some(out)
}

// ------------------------------------------------------------ key <-> filename

/// The filename for a key (before the [`MAX_NAME_BYTES`] rule): WTF-8, with `%XX` for `/`, `%` and NUL anywhere,
/// a leading `.` and a trailing `~`. The empty key is `%`.
pub(crate) fn encode_key_name(key: &[u16]) -> Vec<u8> {
    if key.is_empty() {
        return b"%".to_vec();
    }
    let raw = wtf8_encode(key);
    let mut out = Vec::with_capacity(raw.len() + 4);
    let last = raw.len() - 1;
    for (i, &b) in raw.iter().enumerate() {
        if matches!(b, b'/' | b'%' | 0) || (i == 0 && b == b'.') || (i == last && b == b'~') {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        } else {
            out.push(b);
        }
    }
    out
}

/// The inverse of [`encode_key_name`]. Lenient for hand-made names: a `%` not followed by two hex digits is a
/// literal `%`. `None` when the bytes are not WTF-8.
pub(crate) fn decode_key_name(name: &[u8]) -> Option<Vec<u16>> {
    if name == b"%" {
        return Some(Vec::new());
    }
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut raw = Vec::with_capacity(name.len());
    let mut i = 0;
    while i < name.len() {
        if name[i] == b'%'
            && let (Some(h), Some(l)) = (name.get(i + 1).and_then(|&b| hex(b)), name.get(i + 2).and_then(|&b| hex(b)))
        {
            raw.push(h << 4 | l);
            i += 3;
            continue;
        }
        raw.push(name[i]);
        i += 1;
    }
    wtf8_decode(&raw)
}

/// Where a key lives in its origin directory: the encoded name, or `%h<sha256 hex of the encoded name>` when that
/// is over [`MAX_NAME_BYTES`] (then the file's first line is the encoded name).
fn key_file_name(encoded: &[u8]) -> Vec<u8> {
    if encoded.len() <= MAX_NAME_BYTES {
        return encoded.to_vec();
    }
    let mut s = b"%h".to_vec();
    for b in sha256(encoded) {
        s.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    s
}

/// The value's file contents: WTF-8 + `\n`; a hashed name puts the encoded key line first.
fn file_contents(encoded: &[u8], value: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    if encoded.len() > MAX_NAME_BYTES {
        out.extend_from_slice(encoded);
        out.push(b'\n');
    }
    out.extend_from_slice(&wtf8_encode(value));
    out.push(b'\n');
    out
}

/// Reading removes exactly one trailing `\n` if present (hand-written files and editors).
fn strip_one_newline(b: &[u8]) -> &[u8] {
    b.strip_suffix(b"\n").unwrap_or(b)
}

/// Loading ignores dotfiles (our temp files) and `*~` (editor backups). Encoded keys never produce either.
fn ignored_name(name: &[u8]) -> bool {
    name.first() == Some(&b'.') || name.last() == Some(&b'~')
}

/// One directory entry -> (key, value), or `None` for a file that is ignored or does not parse.
fn read_entry(path: &Path, name: &[u8]) -> io::Result<Option<(Vec<u16>, Vec<u16>)>> {
    if ignored_name(name) {
        return Ok(None);
    }
    let data = match fs::read(path) {
        Ok(d) => d,
        // A directory or other oddity a user put there.
        Err(e) if matches!(e.kind(), io::ErrorKind::IsADirectory | io::ErrorKind::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let hashed = name.len() == 66 && name.starts_with(b"%h") && name[2..].iter().all(u8::is_ascii_hexdigit);
    let (key, value) = if hashed {
        let Some(nl) = data.iter().position(|&b| b == b'\n') else { return Ok(None) };
        (decode_key_name(&data[..nl]), wtf8_decode(strip_one_newline(&data[nl + 1..])))
    } else {
        (decode_key_name(name), wtf8_decode(strip_one_newline(&data)))
    };
    Ok(key.zip(value))
}

// ------------------------------------------------------------ names of sites and origins

/// The directory name of a top-level page's site (`net_policy::site_of`, `://` -> `!`). `None` for pages that
/// have no site (opaque origins: `file:`, `data:`, `about:`): their storage stays in memory.
pub(crate) fn site_dir_name(top: &url::Url) -> Option<String> {
    crate::net_policy::site_of(top).map(|s| s.replacen("://", "!", 1))
}

/// The directory name of a document's origin, `scheme!host!port` with the port always explicit. `None` for opaque
/// origins.
pub(crate) fn origin_dir_name(url: &url::Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    Some(format!("{}!{}!{}", url.scheme(), host, url.port_or_known_default()?))
}

/// A site or origin name must be one plain path component (it comes from a URL or a CLI argument).
fn check_component(s: &str) -> io::Result<()> {
    if s.is_empty() || s.starts_with('.') || s.contains(['/', '\0']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("stash: bad site/origin name {s:?}")));
    }
    Ok(())
}

// ------------------------------------------------------------ the store

/// One storage mutation, as `localStorage` produces them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Set(Vec<u16>, Vec<u16>),
    Remove(Vec<u16>),
    Clear,
}

/// What [`Stash::load`] returns: the generation to hand back to [`Stash::commit`], and the pairs sorted by key
/// (code-unit order; `localStorage.key(i)` uses this order).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Loaded {
    pub generation: u64,
    pub items: Vec<(Vec<u16>, Vec<u16>)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitOutcome {
    /// The ops were applied; `skipped` sets were dropped (with a log line) because they would exceed the quota.
    Committed { skipped: usize },
    /// The site was forgotten since the page loaded (the generation differs): nothing was written and the page
    /// must empty its copy.
    Forgotten,
}

/// An open state directory.
#[derive(Debug)]
pub(crate) struct Stash {
    dir: PathBuf,
    storage: PathBuf,
}

/// Temp-file counter (`.tmp-<pid>-<n>`).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The fallback line has been logged in this process.
static FALLBACK_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn mkdir_0700(p: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(p)
}

/// A held flock on a site's lock file; dropping the file releases it.
pub(crate) struct SiteLock {
    file: File,
}

impl SiteLock {
    /// The generation number stored in the lock file (empty or missing = 0).
    fn generation(&mut self) -> io::Result<u64> {
        let mut s = String::new();
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_to_string(&mut s)?;
        Ok(s.trim().parse().unwrap_or(0))
    }

    /// Written in place: replacing the file would give other processes a different inode to lock.
    fn set_generation(&mut self, g: u64) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(format!("{g}\n").as_bytes())?;
        self.file.sync_all()
    }
}

impl Stash {
    /// Opens (creating, mode 0700) the state directory `dir` and its `storage/.locks/`.
    pub(crate) fn open(dir: &Path) -> io::Result<Stash> {
        let storage = dir.join("storage");
        mkdir_0700(dir)?;
        mkdir_0700(&storage.join(".locks"))?;
        // Proves the directory is writable now rather than at the first commit.
        let probe = storage.join(".locks").join(format!(".probe-{}", std::process::id()));
        File::create(&probe)?;
        fs::remove_file(&probe)?;
        Ok(Stash { dir: dir.to_path_buf(), storage })
    }

    /// [`Stash::open`], or `None` after logging (once per process) that localStorage is in memory this session.
    pub(crate) fn open_or_log(dir: &Path) -> Option<Stash> {
        match Stash::open(dir) {
            Ok(s) => Some(s),
            Err(e) => {
                if !FALLBACK_LOGGED.swap(true, Ordering::Relaxed) {
                    diag!("{}", fallback_line(dir, &e));
                }
                None
            }
        }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// `storage/.locks/<site>`.
    pub(crate) fn lock_path(&self, site: &str) -> PathBuf {
        self.storage.join(".locks").join(site)
    }

    fn origin_path(&self, site: &str, origin: &str) -> PathBuf {
        self.storage.join(site).join(origin)
    }

    fn open_lock_file(&self, site: &str) -> io::Result<File> {
        check_component(site)?;
        OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(self.lock_path(site))
    }

    /// Takes the site's shared lock (blocks while someone holds it exclusively).
    pub(crate) fn lock_shared(&self, site: &str) -> io::Result<SiteLock> {
        let file = self.open_lock_file(site)?;
        file.lock_shared()?;
        Ok(SiteLock { file })
    }

    /// Takes the site's exclusive lock (blocks while anyone holds it).
    pub(crate) fn lock_exclusive(&self, site: &str) -> io::Result<SiteLock> {
        let file = self.open_lock_file(site)?;
        file.lock()?;
        Ok(SiteLock { file })
    }

    /// Page start: every pair of `origin` under `site`, sorted by key, and the site's generation (shared lock).
    pub(crate) fn load(&self, site: &str, origin: &str) -> io::Result<Loaded> {
        check_component(origin)?;
        let mut lock = self.lock_shared(site)?;
        let generation = lock.generation()?;
        let items = self.read_origin(site, origin)?;
        Ok(Loaded { generation, items })
    }

    /// Reads an origin directory (the caller holds the site lock). Sorted by key; a key that two hand-made files
    /// decode to (`a` and `%61`) keeps the first in filename order.
    fn read_origin(&self, site: &str, origin: &str) -> io::Result<Vec<(Vec<u16>, Vec<u16>)>> {
        let dir = self.origin_path(site, origin);
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut names: Vec<Vec<u8>> = Vec::new();
        for ent in rd {
            names.push(ent?.file_name().into_vec());
        }
        names.sort();
        let mut items = Vec::new();
        for name in names {
            let path = dir.join(std::ffi::OsStr::from_bytes(&name));
            if let Some(kv) = read_entry(&path, &name)? {
                items.push(kv);
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items.dedup_by(|b, a| a.0 == b.0);
        Ok(items)
    }

    /// Applies one task's ops under the site's exclusive lock. A `generation` other than the lock file's means the
    /// site was forgotten since the page loaded: the ops are dropped ([`CommitOutcome::Forgotten`]). A set that
    /// would take the origin over [`QUOTA_UNITS`] (counted from the files, under the lock) is skipped with a log line.
    /// Normally called on the [`Writer`] thread.
    pub(crate) fn commit(&self, site: &str, origin: &str, generation: u64, ops: &[Op]) -> io::Result<CommitOutcome> {
        check_component(origin)?;
        let mut lock = self.lock_exclusive(site)?;
        if lock.generation()? != generation {
            return Ok(CommitOutcome::Forgotten);
        }
        if ops.is_empty() {
            return Ok(CommitOutcome::Committed { skipped: 0 });
        }
        let dir = self.origin_path(site, origin);
        // Current usage, from the files (another window may have written since this page loaded).
        let mut sizes: std::collections::HashMap<Vec<u16>, usize> =
            self.read_origin(site, origin)?.into_iter().map(|(k, v)| { let n = entry_units(&k, &v); (k, n) }).collect();
        let mut total: usize = sizes.values().sum();
        let mut skipped = 0;
        let mut created = false;
        for op in ops {
            match op {
                Op::Set(k, v) => {
                    let new = entry_units(k, v);
                    let after = total - sizes.get(k).copied().unwrap_or(0) + new;
                    if after > QUOTA_UNITS {
                        skipped += 1;
                        diag!(
                            "scratchy: stash: {}: setItem of {new} code units skipped, origin would be over the {QUOTA_UNITS} limit",
                            dir.display()
                        );
                        continue;
                    }
                    if !created {
                        mkdir_0700(&dir)?;
                        created = true;
                    }
                    let encoded = encode_key_name(k);
                    let target = dir.join(std::ffi::OsStr::from_bytes(&key_file_name(&encoded)));
                    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
                    let write = || -> io::Result<()> {
                        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
                        f.write_all(&file_contents(&encoded, v))?;
                        f.sync_all()?;
                        fs::rename(&tmp, &target)
                    };
                    if let Err(e) = write() {
                        let _ = fs::remove_file(&tmp);
                        return Err(e);
                    }
                    total = after;
                    sizes.insert(k.clone(), new);
                }
                Op::Remove(k) => {
                    if let Some(n) = sizes.remove(k) {
                        total -= n;
                    }
                    let name = key_file_name(&encode_key_name(k));
                    remove_if_exists(&dir.join(std::ffi::OsStr::from_bytes(&name)))?;
                }
                Op::Clear => {
                    clear_dir(&dir)?;
                    sizes.clear();
                    total = 0;
                }
            }
        }
        if dir.exists() {
            File::open(&dir)?.sync_all()?;
        }
        Ok(CommitOutcome::Committed { skipped })
    }

    /// `localStorage.clear()` on its own: [`Stash::commit`] with a single [`Op::Clear`].
    pub(crate) fn clear(&self, site: &str, origin: &str, generation: u64) -> io::Result<CommitOutcome> {
        self.commit(site, origin, generation, &[Op::Clear])
    }

    /// Forget site: under the exclusive lock, `rm -r storage/<site>` and increment the generation. Returns the new
    /// generation. The lock file stays.
    pub(crate) fn forget(&self, site: &str) -> io::Result<u64> {
        let mut lock = self.lock_exclusive(site)?;
        self.forget_locked(site, &mut lock)
    }

    /// [`Stash::forget`] with the lock already held (tests: forget while a commit waits).
    pub(crate) fn forget_locked(&self, site: &str, lock: &mut SiteLock) -> io::Result<u64> {
        check_component(site)?;
        match fs::remove_dir_all(self.storage.join(site)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let g = lock.generation()? + 1;
        lock.set_generation(g)?;
        Ok(g)
    }
}

/// The one line logged when the state directory cannot be used.
pub(crate) fn fallback_line(dir: &Path, e: &io::Error) -> String {
    format!("scratchy: stash: {}: {e}, localStorage is in-memory for this session", dir.display())
}

fn remove_if_exists(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Unlinks every key file of an origin directory (not dotfiles or `*~`, which are not keys).
fn clear_dir(dir: &Path) -> io::Result<()> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for ent in rd {
        let ent = ent?;
        if !ignored_name(ent.file_name().as_bytes()) && !ent.file_type()?.is_dir() {
            remove_if_exists(&ent.path())?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ writer thread

enum Job {
    Commit {
        site: String,
        origin: String,
        generation: u64,
        ops: Vec<Op>,
        reply: mpsc::Sender<io::Result<CommitOutcome>>,
    },
    Flush(mpsc::Sender<()>),
}

/// The writer thread: commits run here, in submission order, so no file IO happens on the page's thread.
/// Dropping the writer finishes the queued commits and joins the thread.
pub(crate) struct Writer {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    pub(crate) fn spawn(stash: std::sync::Arc<Stash>) -> io::Result<Writer> {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new().name("stash-writer".into()).spawn(move || {
            for job in rx {
                match job {
                    Job::Commit { site, origin, generation, ops, reply } => {
                        let r = stash.commit(&site, &origin, generation, &ops);
                        if let Err(e) = &r {
                            diag!("scratchy: stash: {}: commit failed: {e}", stash.origin_path(&site, &origin).display());
                        }
                        let _ = reply.send(r);
                    }
                    Job::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })?;
        Ok(Writer { tx: Some(tx), thread: Some(thread) })
    }

    /// Queues one task's ops. The receiver gets the outcome; on [`CommitOutcome::Forgotten`] the page empties its
    /// copy. Callers that do not care may drop it.
    pub(crate) fn commit(&self, site: &str, origin: &str, generation: u64, ops: Vec<Op>) -> mpsc::Receiver<io::Result<CommitOutcome>> {
        let (reply, rx) = mpsc::channel();
        let job = Job::Commit { site: site.to_string(), origin: origin.to_string(), generation, ops, reply };
        if let Some(tx) = &self.tx {
            let _ = tx.send(job);
        }
        rx
    }

    /// Blocks until every commit queued before this call has finished.
    pub(crate) fn flush(&self) {
        let (done, rx) = mpsc::channel();
        if let Some(tx) = &self.tx
            && tx.send(Job::Flush(done)).is_ok()
        {
            let _ = rx.recv();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ------------------------------------------------------------ SHA-256 (FIPS 180-4)

/// SHA-256, only for naming long key files (not used for anything secret). No crate in Cargo.lock provides it.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}
