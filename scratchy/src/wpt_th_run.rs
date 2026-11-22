//! QWPT4b (R64 2.3, 2.5, 3.1): the parent of the testharness.js runner.
//!
//! `scratchy --wpt-th <checkout> [--jobs N] [--update] [--area A,B,dir/*] [--except A,B] [--wire PATH] [--dir DIR]
//! [--timeout SECS] [--no-scope] [--server mount|wptserve] [-v]` (run from `scratchy/`, like `--wpt`):
//!
//! 1. reads `wpt-th/subset.txt` (paths, one per line; `# area: NAME` comments group them),
//! 2. starts its OWN private `wire --mount ... --mount-override ...` on its own socket in a temp
//!    dir (never `/run/user/1000/wire.sock`; only that child's PID is ever killed),
//! 3. runs one `scratchy --wpt-th-one` child per test file in a pool of `--jobs` threads; a child
//!    that takes more than 30 s of real time is killed (its own PID) and recorded `TIMEOUT`, one
//!    that prints nothing is `CRASH`; worker variants (`.worker.`, `.serviceworker.`,
//!    `.sharedworker.`) are not run and listed `NOT-RUN`,
//! 4. compares with `wpt-th/expected/<area>.txt` (format R64 2.5, parsed/written below), honours
//!    `wpt-th/flaky.txt` (run, never compared, not counted in the table) and writes
//!    `wpt-th/areas.txt` (R64 3.1: mean per-file score, subtest pass % beside it),
//! 5. exits 1 on any REGRESSION / UNRECORDED / CHANGED; `--update` rewrites the expected files.
//!
//! `scratchy --wpt-th --areas-only [--dir DIR]` (QWPT10) only recomputes `areas.txt` from subset.txt + expected/ + flaky.txt.
//!
//! Memory cap: the whole runner re-executes itself once inside
//! `systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0` (env `SCRATCHY_WPT_TH_MEM`
//! overrides the 6G; `--no-scope` or no usable systemd-run: runs as is, one stderr line). One scope
//! for the runner, wire and all children together (not one per child: a scope costs a D-Bus round
//! trip per test, ~3x the 60 ms process start): inside a full cgroup the OOM killer takes the
//! runaway child, which is then recorded CRASH, and the machine is untouched.
//!
//! Finding wire: `--wire PATH`, else `$SCRATCHY_WIRE`, else `wire` next to this executable, else
//! `../wire/target/release/wire` (a checkout built with `cargo build --release` in `wire/`), else
//! `wire` on `$PATH`.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The vendor hook (R64 2.2), embedded so an installed binary needs no source tree.
const REPORT_JS: &str = include_str!("../tools/wpt/testharnessreport.js");
/// QWPT7: the testdriver vendor (trusted input through `wpt_input.rs`), embedded the same way.
const VENDOR_JS: &str = include_str!("../tools/wpt/testdriver-vendor.js");
/// The pinned WPT commit, for the areas.txt header.
const WPT_PIN: &str = include_str!("../tools/wpt-pin.txt");
/// The static mount's origin (wire `--mount`), the default server.
const ORIGIN: &str = "http://web-platform.test:8000";
/// QWPT8: areas that need WPT's Python server (`--server wptserve`); tools/wpt-subset.py has the same list.
/// The default (mount) run skips them, a wptserve run runs only them.
pub(crate) const WPTSERVE_AREAS: [&str; 3] = ["fetch/api", "fetch/other", "xhr"];
pub(crate) fn is_wptserve_area(area: &str) -> bool {
    WPTSERVE_AREAS.contains(&area)
}
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);

// ------------------------------------------------------------------ a file's result

/// One test file's outcome (what the expected files record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileResult {
    /// `OK` `ERROR` `TIMEOUT` `PRECONDITION_FAILED` (harness), or `CRASH` / `NOT-RUN` (no result).
    pub status: String,
    pub passed: usize,
    pub total: usize,
    /// Subtests that did not pass: (status, name); duplicate names carry ` #2`, ` #3`.
    pub nonpass: Vec<(String, String)>,
}

impl FileResult {
    fn bare(status: &str) -> FileResult {
        FileResult { status: status.to_string(), passed: 0, total: 0, nonpass: Vec::new() }
    }
    fn has_result(&self) -> bool {
        !matches!(self.status.as_str(), "CRASH" | "NOT-RUN")
    }
    fn score(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.passed as f64 / self.total as f64 }
    }
}

enum J {
    Null,
    Bool(#[allow(dead_code)] bool),
    Num(#[allow(dead_code)] f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

struct JP<'a> {
    b: &'a [u8],
    i: usize,
}

impl JP<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut units: Vec<u16> = Vec::new();
        let mut out = String::new();
        let flush = |units: &mut Vec<u16>, out: &mut String| {
            out.push_str(&String::from_utf16_lossy(units));
            units.clear();
        };
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let h = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
                            units.push(u16::from_str_radix(h, 16).ok()?);
                            self.i += 4;
                            continue;
                        }
                        other => other as char, // `"` `\` `/`
                    };
                    flush(&mut units, &mut out);
                    out.push(ch);
                }
                _ => {
                    flush(&mut units, &mut out);
                    let start = self.i - 1;
                    let mut end = self.i;
                    while end < self.b.len() && !matches!(self.b[end], b'"' | b'\\') {
                        end += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.b[start..end]).ok()?);
                    self.i = end;
                }
            }
        }
        flush(&mut units, &mut out);
        Some(out)
    }
    fn value(&mut self, depth: u32) -> Option<J> {
        self.ws();
        if depth > 16 {
            return None;
        }
        match *self.b.get(self.i)? {
            b'"' => self.string().map(J::Str),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(J::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    if self.eat(b',').is_none() {
                        self.eat(b']')?;
                        return Some(J::Arr(v));
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(J::Obj(v));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    v.push((k, self.value(depth + 1)?));
                    if self.eat(b',').is_none() {
                        self.eat(b'}')?;
                        return Some(J::Obj(v));
                    }
                }
            }
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(J::Bool(true))
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(J::Bool(false))
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(J::Null)
            }
            _ => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[s..self.i]).ok()?.parse().ok().map(J::Num)
            }
        }
    }
}

/// Parses the child's one JSON line (NOTES.md "WPT testharness child"); `None` if malformed.
pub(crate) fn parse_child_json(line: &str) -> Option<FileResult> {
    let mut p = JP { b: line.trim().as_bytes(), i: 0 };
    let J::Obj(top) = p.value(0)? else { return None };
    let mut harness = None;
    let mut tests = None;
    for (k, v) in top {
        match (k.as_str(), v) {
            ("harness", J::Str(s)) => harness = Some(s),
            ("tests", J::Arr(a)) => tests = Some(a),
            _ => {}
        }
    }
    let mut r = FileResult::bare(&harness?);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for t in tests? {
        let J::Arr(t) = t else { return None };
        let (Some(J::Str(name)), Some(J::Str(status))) = (t.first(), t.get(1)) else { return None };
        let n = seen.entry(name.clone()).or_insert(0);
        *n += 1;
        let name = if *n > 1 { format!("{name} #{n}") } else { name.clone() };
        r.total += 1;
        if status == "PASS" {
            r.passed += 1;
        } else {
            r.nonpass.push((status.clone(), name));
        }
    }
    Some(r)
}

// ------------------------------------------------------------------ expected files (R64 2.5)

fn escape_name(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
}

fn unescape_name(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// `wpt-th/expected/<area>.txt`: one line per file, then one indented line per non-passing subtest.
pub(crate) fn format_expected(area: &str, files: &BTreeMap<String, FileResult>) -> String {
    let mut out = format!("# wpt-th expected results for area {area}: generated by `scratchy --wpt-th <wpt> --update`.\n");
    for (path, r) in files {
        if r.status == "NOT-RUN" {
            out.push_str(&format!("{:<8} {:>7}  {path}\n", r.status, ""));
            continue;
        }
        out.push_str(&format!("{:<8} {:>7}  {path}\n", r.status, format!("{}/{}", r.passed, r.total)));
        for (st, name) in &r.nonpass {
            let pad = 10.max(st.len() + 1);
            out.push_str(&format!("  {st:<pad$}{}\n", escape_name(name)));
        }
    }
    out
}

pub(crate) fn parse_expected(text: &str) -> BTreeMap<String, FileResult> {
    let mut m: BTreeMap<String, FileResult> = BTreeMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("  ") {
            let (Some(path), Some(f)) = (&cur, rest.split(' ').next()) else { continue };
            let skip = 10.max(f.len() + 1);
            let name = unescape_name(rest.get(skip..).unwrap_or(""));
            if let Some(r) = m.get_mut(path) {
                r.nonpass.push((f.to_string(), name));
            }
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(status) = it.next() else { continue };
        let mut r = FileResult::bare(status);
        let mut rest = line[status.len()..].trim_start();
        if let Some((count, tail)) = rest.split_once(char::is_whitespace)
            && let Some((p, t)) = count.split_once('/')
            && let (Ok(p), Ok(t)) = (p.parse(), t.parse())
        {
            r.passed = p;
            r.total = t;
            rest = tail.trim_start();
        }
        let path = rest.trim().to_string();
        m.insert(path.clone(), r);
        cur = Some(path);
    }
    m
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Same,
    /// Blocks: a passing subtest fails, or the file got worse.
    Regression(String),
    /// Blocks: subtests appeared/disappeared or a non-pass changed kind; fix the name or flaky.txt.
    Changed(String),
    /// Reported, does not block: something now passes; `--update` records it.
    NewlyPassing(String),
}

fn status_rank(s: &str) -> u8 {
    match s {
        "OK" => 0,
        "CRASH" | "NOT-RUN" => 2,
        _ => 1,
    }
}

pub(crate) fn compare(e: &FileResult, a: &FileResult) -> Verdict {
    if e == a {
        return Verdict::Same;
    }
    if e.status == "NOT-RUN" || a.status == "NOT-RUN" {
        return Verdict::Changed(format!("{} -> {}", e.status, a.status));
    }
    let (re, ra) = (status_rank(&e.status), status_rank(&a.status));
    if ra > re {
        return Verdict::Regression(format!("{} -> {}", e.status, a.status));
    }
    if e.total != a.total {
        return Verdict::Changed(format!("subtests {} -> {}", e.total, a.total));
    }
    let enp: BTreeMap<&str, &str> = e.nonpass.iter().map(|(s, n)| (n.as_str(), s.as_str())).collect();
    let anp: BTreeMap<&str, &str> = a.nonpass.iter().map(|(s, n)| (n.as_str(), s.as_str())).collect();
    if let Some((n, s)) = anp.iter().find(|(n, _)| !enp.contains_key(*n)) {
        return Verdict::Regression(format!("subtest was passing, now {s}: {n}"));
    }
    if let Some((n, s)) = anp.iter().find(|(n, s)| enp.get(*n).is_some_and(|o| o != *s)) {
        return Verdict::Changed(format!("subtest {} -> {s}: {n}", enp[n]));
    }
    if ra == re && e.status != a.status {
        return Verdict::Changed(format!("{} -> {}", e.status, a.status));
    }
    Verdict::NewlyPassing(format!("{}/{} -> {}/{}", e.passed, e.total, a.passed, a.total))
}

// ------------------------------------------------------------------ subset and flaky lists

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub path: String,
    pub area: String,
}

/// `subset.txt`: `# area: NAME` starts an area; other `#` lines and blanks are ignored.
pub(crate) fn parse_subset(text: &str) -> Vec<Entry> {
    let mut area = "default".to_string();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(c) = line.strip_prefix('#') {
            if let Some(a) = c.trim().strip_prefix("area:") {
                area = a.trim().to_string();
            }
            continue;
        }
        if !line.is_empty() && seen.insert(line.to_string()) {
            out.push(Entry { path: line.to_string(), area: area.clone() });
        }
    }
    out
}

/// `flaky.txt`: first whitespace-separated token of each non-comment line is a path.
pub(crate) fn parse_flaky(text: &str) -> BTreeSet<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

/// Worker variants (`x.any.worker.html`, `x.worker.html`, service/shared worker) need a worker
/// global scope scratchy does not have: listed `NOT-RUN`.
pub(crate) fn is_worker_variant(path: &str) -> bool {
    let file = path.split(['?', '#']).next().unwrap_or(path).rsplit('/').next().unwrap_or(path);
    [".worker.", ".serviceworker.", ".sharedworker."].iter().any(|m| file.contains(m))
}

fn area_file(area: &str) -> String {
    let s: String = area.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    format!("{s}.txt")
}

// ------------------------------------------------------------------ areas table (R64 3.1)

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AreaRow {
    pub area: String,
    pub files: usize,
    pub ran: usize,
    pub all_pass: usize,
    pub subtests: usize,
    pub pass: usize,
    /// Mean per-file score, percent.
    pub score: f64,
}

pub(crate) fn area_row(area: &str, files: usize, results: &[&FileResult]) -> AreaRow {
    let ran: Vec<&&FileResult> = results.iter().filter(|r| r.has_result()).collect();
    let score = if ran.is_empty() { 0.0 } else { 100.0 * ran.iter().map(|r| r.score()).sum::<f64>() / ran.len() as f64 };
    AreaRow {
        area: area.to_string(),
        files,
        ran: ran.len(),
        all_pass: ran.iter().filter(|r| r.status == "OK" && r.total > 0 && r.passed == r.total).count(),
        subtests: ran.iter().map(|r| r.total).sum(),
        pass: ran.iter().map(|r| r.passed).sum(),
        score,
    }
}

fn row_line(r: &AreaRow) -> String {
    let pct = if r.subtests == 0 { 0.0 } else { 100.0 * r.pass as f64 / r.subtests as f64 };
    format!("{:<28} {:>6} {:>6} {:>9} {:>9} {:>7} {:>6.1} {:>6.1}\n", r.area, r.files, r.ran, r.all_pass, r.subtests, r.pass, pct, r.score)
}

pub(crate) fn format_areas(rows: &[AreaRow]) -> String {
    let mut out = format!(
        "# wpt-th areas at WPT {}; score = mean over files of passed/total subtests (files that ran; flaky.txt files and NOT-RUN/CRASH excluded)\n",
        WPT_PIN.trim()
    );
    out.push_str(&format!("# {:<26} {:>6} {:>6} {:>9} {:>9} {:>7} {:>6} {:>6}\n", "area", "files", "ran", "all-pass", "subtests", "pass", "pass%", "score"));
    for r in rows {
        out.push_str(&row_line(r));
    }
    let ran: usize = rows.iter().map(|r| r.ran).sum();
    let total = AreaRow {
        area: "TOTAL".into(),
        files: rows.iter().map(|r| r.files).sum(),
        ran,
        all_pass: rows.iter().map(|r| r.all_pass).sum(),
        subtests: rows.iter().map(|r| r.subtests).sum(),
        pass: rows.iter().map(|r| r.pass).sum(),
        score: if ran == 0 { 0.0 } else { rows.iter().map(|r| r.score * r.ran as f64).sum::<f64>() / ran as f64 },
    };
    out.push_str(&row_line(&total));
    out
}

pub(crate) fn parse_areas(text: &str) -> Vec<AreaRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if line.starts_with('#') || f.len() != 8 || f[0] == "TOTAL" {
            continue;
        }
        let n = |i: usize| f[i].parse::<usize>().ok();
        if let (Some(files), Some(ran), Some(all_pass), Some(subtests), Some(pass), Ok(score)) = (n(1), n(2), n(3), n(4), n(5), f[7].parse::<f64>()) {
            rows.push(AreaRow { area: f[0].to_string(), files, ran, all_pass, subtests, pass, score });
        }
    }
    rows
}

// ------------------------------------------------------------------ children

/// How to start a child: `program args... URL`.
#[derive(Clone)]
pub(crate) struct ChildCmd {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub verbose: bool,
    /// `http://web-platform.test:PORT` the test URLs are built on.
    pub origin: String,
}

enum ChildEnd {
    Line(String),
    NoResult,
    TimedOut,
}

/// Runs one child; on `timeout` kills THAT child's PID only.
fn run_child(cmd: &ChildCmd, url: &str, timeout: Duration) -> ChildEnd {
    let mut c = Command::new(&cmd.program);
    c.args(&cmd.args).arg(url).stdin(Stdio::null()).stdout(Stdio::piped());
    c.stderr(if cmd.verbose { Stdio::inherit() } else { Stdio::null() });
    let Ok(mut child) = c.spawn() else { return ChildEnd::NoResult };
    let pid = child.id();
    let mut out = child.stdout.take().expect("piped");
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut s = String::new();
        let _ = Read::read_to_string(&mut Read::take(&mut out, 64 << 20), &mut s);
        let _ = tx.send(s);
    });
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) if t0.elapsed() >= timeout => {
                diag!("scratchy --wpt-th: killing child pid {pid} after {} s: {url}", timeout.as_secs());
                let _ = child.kill(); // our own Child handle: SIGKILL to this PID
                let _ = child.wait();
                return ChildEnd::TimedOut;
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
        }
    }
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(s) if !s.trim().is_empty() => ChildEnd::Line(s.lines().next().unwrap_or("").to_string()),
        _ => ChildEnd::NoResult,
    }
}

fn result_of(cmd: &ChildCmd, entry: &Entry, timeout: Duration) -> FileResult {
    if is_worker_variant(&entry.path) {
        return FileResult::bare("NOT-RUN");
    }
    match run_child(cmd, &format!("{}/{}", cmd.origin, entry.path), timeout) {
        ChildEnd::Line(l) => parse_child_json(&l).unwrap_or_else(|| FileResult::bare("CRASH")),
        ChildEnd::NoResult => FileResult::bare("CRASH"),
        ChildEnd::TimedOut => FileResult::bare("TIMEOUT"),
    }
}

/// Runs every entry with `jobs` threads; results in entry order.
#[cfg(test)]
pub(crate) fn run_all(cmd: &ChildCmd, entries: &[Entry], jobs: usize, timeout: Duration) -> Vec<FileResult> {
    run_all_timed(cmd, entries, jobs, timeout).into_iter().map(|(r, _)| r).collect()
}

/// `run_all` plus each file's wall time (QCISPLIT2: the per-file CI timings).
pub(crate) fn run_all_timed(cmd: &ChildCmd, entries: &[Entry], jobs: usize, timeout: Duration) -> Vec<(FileResult, Duration)> {
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<(FileResult, Duration)>>> = entries.iter().map(|_| Mutex::new(None)).collect();
    thread::scope(|s| {
        for _ in 0..jobs.clamp(1, entries.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(e) = entries.get(i) else { break };
                    let t = Instant::now();
                    let r = result_of(cmd, e, timeout);
                    *slots[i].lock().unwrap() = Some((r, t.elapsed()));
                }
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().unwrap_or_else(|| (FileResult::bare("CRASH"), Duration::ZERO))).collect()
}

// ------------------------------------------------------------------ the run

pub(crate) struct Config {
    /// The `wpt-th` directory (subset.txt, expected/, flaky.txt, areas.txt).
    pub dir: PathBuf,
    pub update: bool,
    /// `--area`: comma-separated exact areas or `prefix/*` patterns (QCISPLIT2); None = all.
    pub area: Option<String>,
    /// `--except`: comma-separated areas/patterns to leave out (the CI "rest" shard).
    pub except: Option<String>,
    pub jobs: usize,
    pub timeout: Duration,
    pub verbose: bool,
    /// `--server wptserve` (QWPT8): run only the WPTSERVE_AREAS; false = run everything but those.
    pub wptserve: bool,
}

/// One `--area`/`--except` item: an exact area, or `prefix/*` for every area under `prefix/`.
fn area_matches(pat: &str, area: &str) -> bool {
    match pat.strip_suffix("/*") {
        Some(p) => area.strip_prefix(p).is_some_and(|r| r.starts_with('/')),
        None => pat == area,
    }
}

/// QCISPLIT2: the shard filter. `only` (None = everything) minus `except`.
pub(crate) fn selected_area(only: Option<&str>, except: Option<&str>, area: &str) -> bool {
    let any = |l: &str| l.split(',').any(|p| area_matches(p, area));
    only.is_none_or(any) && !except.is_some_and(any)
}

/// Compare (or, with `update`, record) and report. Returns the exit code.
pub(crate) fn execute(cfg: &Config, cmd: &ChildCmd) -> i32 {
    let Ok(subset) = fs::read_to_string(cfg.dir.join("subset.txt")) else {
        diag!("scratchy --wpt-th: cannot read {}", cfg.dir.join("subset.txt").display());
        return 2;
    };
    let flaky = fs::read_to_string(cfg.dir.join("flaky.txt")).map(|t| parse_flaky(&t)).unwrap_or_default();
    let all = parse_subset(&subset);
    let mut area_order: Vec<String> = Vec::new();
    for e in &all {
        if !area_order.contains(&e.area) {
            area_order.push(e.area.clone());
        }
    }
    // Every --area item must name something (a typo must not turn a shard into an empty pass).
    for a in cfg.area.iter().chain(cfg.except.iter()).flat_map(|l| l.split(',')) {
        if !area_order.iter().any(|x| area_matches(a, x)) {
            diag!("scratchy --wpt-th: no area {a} in subset.txt (areas: {})", area_order.join(", "));
            return 2;
        }
    }
    // QWPT8: an explicitly named area must belong to the chosen server; otherwise each server runs its own areas.
    for a in cfg.area.iter().flat_map(|l| l.split(',')) {
        if let Some(x) = area_order.iter().find(|x| area_matches(a, x) && is_wptserve_area(x) != cfg.wptserve) {
            diag!("scratchy --wpt-th: area {x} {} --server wptserve", if cfg.wptserve { "is served by the static mount: run it without" } else { "needs" });
            return 2;
        }
    }
    let selected = |area: &str| is_wptserve_area(area) == cfg.wptserve && selected_area(cfg.area.as_deref(), cfg.except.as_deref(), area);
    let entries: Vec<Entry> = all.iter().filter(|e| selected(&e.area)).cloned().collect();
    if entries.is_empty() {
        diag!("scratchy --wpt-th: the --area/--except selection is empty");
        return 2;
    }
    let t0 = Instant::now();
    let timed = run_all_timed(cmd, &entries, cfg.jobs, cfg.timeout);
    diag!("scratchy --wpt-th: ran {} files in {:.1} s ({} jobs)", entries.len(), t0.elapsed().as_secs_f64(), cfg.jobs);
    // QCISPLIT2: the slowest files, so CI logs show what dominates the step.
    let mut slow: Vec<(f64, &str)> = timed.iter().zip(&entries).map(|((_, d), e)| (d.as_secs_f64(), e.path.as_str())).collect();
    slow.sort_by(|a, b| b.0.total_cmp(&a.0));
    let total: f64 = slow.iter().map(|s| s.0).sum();
    diag!("scratchy --wpt-th: summed file time {total:.1} s; slowest files:");
    for (secs, path) in slow.iter().take(10) {
        diag!("  SLOW {secs:6.2} s {path}");
    }
    let over2 = slow.iter().filter(|s| s.0 >= 2.0).count();
    diag!("scratchy --wpt-th: {over2} files took >= 2 s, summing {:.1} s", 0.0 + slow.iter().filter(|s| s.0 >= 2.0).map(|s| s.0).sum::<f64>());
    let mut per_area: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    for ((_, d), e) in timed.iter().zip(&entries) {
        let x = per_area.entry(e.area.as_str()).or_default();
        x.0 += 1;
        x.1 += d.as_secs_f64();
    }
    for (a, (n, secs)) in &per_area {
        diag!("  AREA-TIME {a} {n} files {secs:.1} s");
    }
    let results: Vec<FileResult> = timed.into_iter().map(|(r, _)| r).collect();

    let mut blocking = 0usize;
    let mut newly = 0usize;
    let mut by_area: BTreeMap<&str, BTreeMap<String, FileResult>> = BTreeMap::new();
    for (e, r) in entries.iter().zip(&results) {
        if !flaky.contains(&e.path) {
            by_area.entry(&e.area).or_default().insert(e.path.clone(), r.clone());
        }
    }
    for (area, files) in &by_area {
        let efile = cfg.dir.join("expected").join(area_file(area));
        let expected = fs::read_to_string(&efile).map(|t| parse_expected(&t)).unwrap_or_default();
        for (path, a) in files {
            let line = match expected.get(path) {
                None => {
                    blocking += 1;
                    Some(format!("UNRECORDED {path}: {}", describe(a)))
                }
                Some(e) => match compare(e, a) {
                    Verdict::Same => None,
                    Verdict::Regression(m) => {
                        blocking += 1;
                        Some(format!("REGRESSION {path}: {m}"))
                    }
                    Verdict::Changed(m) => {
                        blocking += 1;
                        Some(format!("CHANGED {path}: {m}"))
                    }
                    Verdict::NewlyPassing(m) => {
                        newly += 1;
                        Some(format!("NEWLY-PASSING {path}: {m}"))
                    }
                },
            };
            if let Some(l) = line {
                diag!("{l}");
            } else if cfg.verbose {
                diag!("same {path}: {}", describe(a));
            }
        }
        if cfg.update {
            let _ = fs::create_dir_all(cfg.dir.join("expected"));
            if let Err(e) = fs::write(&efile, format_expected(area, files)) {
                diag!("scratchy --wpt-th: cannot write {}: {e}", efile.display());
                return 2;
            }
        }
    }
    for e in entries.iter().zip(&results).filter(|(e, _)| flaky.contains(&e.path)) {
        diag!("flaky (not compared) {}: {}", e.0.path, describe(e.1));
    }

    // areas.txt: this run's rows; with --area/--except the other areas keep their old rows.
    let old = fs::read_to_string(cfg.dir.join("areas.txt")).map(|t| parse_areas(&t)).unwrap_or_default();
    let mut rows = Vec::new();
    for area in &area_order {
        let idx: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].area == *area).collect();
        if selected(area) {
            let counted: Vec<&FileResult> = idx.iter().filter(|&&i| !flaky.contains(&entries[i].path)).map(|&i| &results[i]).collect();
            rows.push(area_row(area, idx.len(), &counted));
        } else if let Some(o) = old.iter().find(|r| r.area == *area) {
            rows.push(o.clone());
        }
    }
    if let Err(e) = fs::write(cfg.dir.join("areas.txt"), format_areas(&rows)) {
        diag!("scratchy --wpt-th: cannot write areas.txt: {e}");
        return 2;
    }
    write_raw_stdout(format_args!("{}", format_areas(&rows)));
    if cfg.update {
        diag!("scratchy --wpt-th: expected files updated ({blocking} unrecorded/regressed/changed and {newly} newly passing recorded)");
        return 0;
    }
    if blocking > 0 {
        diag!("scratchy --wpt-th: {blocking} REGRESSION/UNRECORDED/CHANGED (fix, or `--update` for new tests and new passes; flaky.txt for unstable names)");
        return 1;
    }
    if newly > 0 {
        diag!("scratchy --wpt-th: {newly} files newly pass something: run with --update and commit wpt-th/expected");
    }
    0
}

fn describe(r: &FileResult) -> String {
    if r.has_result() { format!("{} {}/{}", r.status, r.passed, r.total) } else { r.status.clone() }
}

// ------------------------------------------------------------------ wire and the command line

/// Our own wire child; dropping it kills that PID (and only that) and removes its temp dir.
struct PrivateWire {
    child: Child,
    dir: PathBuf,
}

impl Drop for PrivateWire {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn find_wire(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p);
    }
    if let Some(p) = std::env::var_os("SCRATCHY_WIRE") {
        return Some(p.into());
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().as_deref().and_then(|p| p.parent()) {
        cands.push(d.join("wire"));
    }
    cands.push(PathBuf::from("../wire/target/release/wire"));
    if let Some(path) = std::env::var_os("PATH") {
        cands.extend(std::env::split_paths(&path).map(|d| d.join("wire")));
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Whether `wire --help` lists `flag`.
fn wire_has_flag(wire: &std::path::Path, flag: &str) -> bool {
    Command::new(wire).arg("--help").stdin(Stdio::null()).output().is_ok_and(|o| String::from_utf8_lossy(&o.stderr).contains(flag) || String::from_utf8_lossy(&o.stdout).contains(flag))
}

/// QWPT8: the WPT Python server and its wire are started OUTSIDE this process by `tools/wptserve.sh start CHECKOUT`
/// (this process runs under QNET1's seccomp filter, which every child inherits: no TCP socket can be created in
/// it, so python and a wire that dials 127.0.0.1 cannot be our children). We only read the script's state:
/// `<checkout>/.wptserve/port` and `wire.sock.path`, and check that the wire socket answers.
pub(crate) fn wptserve_state(checkout: &std::path::Path) -> Result<(u16, PathBuf), String> {
    let st = checkout.join(".wptserve");
    let hint = format!("run `tools/wptserve.sh start {} --wire PATH` first", checkout.display());
    let port = fs::read_to_string(st.join("port")).ok().and_then(|t| t.trim().parse::<u16>().ok()).ok_or_else(|| format!("no wptserve state in {}: {hint}", st.display()))?;
    let sock = PathBuf::from(fs::read_to_string(st.join("wire.sock.path")).map_err(|_| format!("no wire socket recorded in {}: {hint}", st.display()))?.trim());
    std::os::unix::net::UnixStream::connect(&sock).map_err(|e| format!("wptserve's wire socket {} does not answer ({e}): {hint}", sock.display()))?;
    Ok((port, sock))
}

fn start_wire(wire: &std::path::Path, checkout: &std::path::Path, verbose: bool) -> Result<(PrivateWire, PathBuf), String> {
    let dir = std::env::temp_dir().join(format!("scratchy-wpt-th-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let hook = dir.join("testharnessreport.js");
    fs::write(&hook, REPORT_JS).map_err(|e| format!("{}: {e}", hook.display()))?;
    let vendor = dir.join("testdriver-vendor.js");
    fs::write(&vendor, VENDOR_JS).map_err(|e| format!("{}: {e}", vendor.display()))?;
    let sock = dir.join("wire.sock");
    let checkout = fs::canonicalize(checkout).map_err(|e| format!("{}: {e}", checkout.display()))?;
    let mut c = Command::new(wire);
    c.arg("--socket").arg(&sock).arg("--no-cookies");
    // W-CACHE1: mounts are never cached; keep the run from touching ~/.cache/wire.
    // An older wire has no such flag (its `--help` does not mention it).
    if wire_has_flag(wire, "--no-cache") {
        c.arg("--no-cache");
    }
    c.arg("--mount").arg(format!("{ORIGIN}={}", checkout.display()));
    c.arg("--mount-override").arg(format!("/resources/testharnessreport.js={}", hook.display()));
    c.arg("--mount-override").arg(format!("/resources/testdriver-vendor.js={}", vendor.display()));
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(if verbose { Stdio::inherit() } else { Stdio::null() });
    let child = c.spawn().map_err(|e| format!("cannot start {}: {e}", wire.display()))?;
    let mut w = PrivateWire { child, dir };
    let t0 = Instant::now();
    loop {
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return Ok((w, sock));
        }
        if let Ok(Some(st)) = w.child.try_wait() {
            return Err(format!("{} exited early ({st}); rerun with -v", wire.display()));
        }
        if t0.elapsed() > Duration::from_secs(10) {
            return Err(format!("{} did not open {} in 10 s", wire.display(), sock.display()));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Re-executes this runner inside a memory-capped systemd scope (see the module doc). Only returns
/// when that is not possible (then the run continues unscoped).
fn rescope(args: &[String]) {
    if std::env::var_os("SCRATCHY_WPT_TH_SCOPED").is_some() {
        return;
    }
    let mem = std::env::var("SCRATCHY_WPT_TH_MEM").unwrap_or_else(|_| "6G".into());
    let base = |c: &mut Command| {
        c.args(["--user", "--scope", "-q", "-p"]).arg(format!("MemoryMax={mem}")).args(["-p", "MemorySwapMax=0"]);
    };
    let mut probe = Command::new("systemd-run");
    base(&mut probe);
    let usable = probe.arg("true").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    let Ok(exe) = std::env::current_exe() else { return };
    if !usable {
        diag!("scratchy --wpt-th: no usable `systemd-run --user --scope`: running without a memory cap");
        return;
    }
    use std::os::unix::process::CommandExt;
    let mut c = Command::new("systemd-run");
    base(&mut c);
    let err = c.arg(exe).arg("--wpt-th").args(args).env("SCRATCHY_WPT_TH_SCOPED", "1").exec();
    diag!("scratchy --wpt-th: cannot enter a systemd scope ({err}): running without a memory cap");
}

/// QWPT10: the areas table computed from subset.txt + expected/*.txt + flaky.txt, no test run. Same rows as a
/// full run: `files` counts every subset entry of the area (flaky included), the other columns only the
/// non-flaky entries that have an expected record (a full run would have a result for those).
pub(crate) fn areas_from_dir(dir: &std::path::Path) -> Result<String, String> {
    let subset = fs::read_to_string(dir.join("subset.txt")).map_err(|e| format!("cannot read {}: {e}", dir.join("subset.txt").display()))?;
    let flaky = fs::read_to_string(dir.join("flaky.txt")).map(|t| parse_flaky(&t)).unwrap_or_default();
    let mut order: Vec<String> = Vec::new();
    let mut by_area: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for e in parse_subset(&subset) {
        if !order.contains(&e.area) {
            order.push(e.area.clone());
        }
        by_area.entry(e.area.clone()).or_default().push(e);
    }
    let mut rows = Vec::new();
    for area in &order {
        let entries = &by_area[area];
        let expected = fs::read_to_string(dir.join("expected").join(area_file(area))).map(|t| parse_expected(&t)).unwrap_or_default();
        let counted: Vec<&FileResult> = entries.iter().filter(|e| !flaky.contains(&e.path)).filter_map(|e| expected.get(&e.path)).collect();
        rows.push(area_row(area, entries.len(), &counted));
    }
    Ok(format_areas(&rows))
}

const USAGE: &str = "usage: scratchy --wpt-th <wpt-checkout> [--jobs N] [--update] [--area A,B,dir/*] [--except A,B] [--wire PATH] [--dir wpt-th] [--timeout SECS] [--no-scope] [--server mount|wptserve] [-v]\n       scratchy --wpt-th --areas-only [--dir wpt-th]   (rewrite areas.txt from subset/expected/flaky, no test run, no checkout)";

pub(crate) fn main(args: &[String]) -> i32 {
    let (mut checkout, mut wire, mut dir) = (None, None, PathBuf::from("wpt-th"));
    let mut cfg = Config {
        dir: PathBuf::new(),
        update: false,
        area: None,
        except: None,
        jobs: thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
        timeout: CHILD_TIMEOUT,
        verbose: false,
        wptserve: false,
    };
    let mut no_scope = false;
    let mut areas_only = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut val = || {
            i += 1;
            args.get(i).cloned()
        };
        match a {
            "--jobs" | "--area" | "--except" | "--wire" | "--dir" | "--timeout" | "--server" => {
                let Some(v) = val() else {
                    diag!("scratchy --wpt-th: {a} needs an argument");
                    return 2;
                };
                match a {
                    "--jobs" => cfg.jobs = v.parse().unwrap_or(0).max(1),
                    "--area" => cfg.area = Some(v),
                    "--except" => cfg.except = Some(v),
                    "--wire" => wire = Some(PathBuf::from(v)),
                    "--dir" => dir = PathBuf::from(v),
                    "--server" => match v.as_str() {
                        "wptserve" => cfg.wptserve = true,
                        "mount" => cfg.wptserve = false,
                        _ => {
                            diag!("scratchy --wpt-th: --server is mount or wptserve");
                            return 2;
                        }
                    },
                    _ => cfg.timeout = Duration::from_secs(v.parse().unwrap_or(30).max(1)),
                }
            }
            "--update" => cfg.update = true,
            "--no-scope" => no_scope = true,
            "--areas-only" => areas_only = true,
            "-v" => cfg.verbose = true,
            a if !a.starts_with("--") && checkout.is_none() => checkout = Some(PathBuf::from(a)),
            a => {
                diag!("scratchy --wpt-th: unexpected argument {a}");
                return 2;
            }
        }
        i += 1;
    }
    if areas_only {
        return match areas_from_dir(&dir) {
            Ok(t) => match fs::write(dir.join("areas.txt"), &t) {
                Ok(()) => {
                    write_raw_stdout(format_args!("{t}"));
                    0
                }
                Err(e) => {
                    diag!("scratchy --wpt-th: cannot write areas.txt: {e}");
                    2
                }
            },
            Err(e) => {
                diag!("scratchy --wpt-th: {e}");
                2
            }
        };
    }
    let Some(checkout) = checkout else {
        diag!("{USAGE}");
        return 2;
    };
    cfg.dir = dir;
    if !no_scope {
        rescope(args);
    }
    // SAFETY: setrlimit on our own process; children inherit "no core files" (AGENT_RULES: ulimit -c 0).
    unsafe {
        let z = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &z);
    }
    let mut origin = ORIGIN.to_string();
    let _wire;
    let sock;
    if cfg.wptserve {
        match wptserve_state(&checkout) {
            Ok((port, s)) => {
                origin = format!("http://web-platform.test:{port}");
                sock = s;
                _wire = None;
            }
            Err(e) => {
                diag!("scratchy --wpt-th: {e}");
                return 2;
            }
        }
    } else {
        let Some(wire) = find_wire(wire) else {
            diag!("scratchy --wpt-th: no wire binary: pass --wire PATH or set SCRATCHY_WIRE (build it: `cargo build --release` in wire/)");
            return 2;
        };
        match start_wire(&wire, &checkout, cfg.verbose) {
            Ok((w, s)) => {
                _wire = Some(w);
                sock = s;
            }
            Err(e) => {
                diag!("scratchy --wpt-th: {e}");
                return 2;
            }
        }
    }
    let Ok(exe) = std::env::current_exe() else {
        diag!("scratchy --wpt-th: cannot find own executable");
        return 2;
    };
    let cmd = ChildCmd { program: exe, args: vec!["--wpt-th-one".into(), "--socket".into(), sock.display().to_string()], verbose: cfg.verbose, origin };
    execute(&cfg, &cmd)
}
