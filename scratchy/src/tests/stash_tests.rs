// QSTASH1: src/stash.rs (STASH.md). Every test works in its own directory on real disk under the crate's
// target/ (never /tmp, which is RAM on the dev machine, and never the user's state directory).
use super::*;
use crate::stash::{self, CommitOutcome, Loaded, Op, Stash, Writer};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("stash-tests").join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        TestDir(p)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

const SITE: &str = "https!loupax.com";
const ORIGIN: &str = "https!social.loupax.com!443";

fn origin_dir(d: &TestDir) -> PathBuf {
    d.0.join("storage").join(SITE).join(ORIGIN)
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn qstash1_sha256_known_vectors() {
    let hex = |b: [u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(hex(stash::sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(stash::sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hex(stash::sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn qstash1_wtf8_round_trips_lone_surrogates() {
    let cases: Vec<Vec<u16>> = vec![
        vec![],
        u("plain ascii"),
        u("ünïcödé €"),
        u("emoji 😀 pair"),
        vec![0x61, 0xD83D],         // "a\uD83D": lone lead at the end
        vec![0xDE00, 0x62],         // lone trail at the start
        vec![0xDC00, 0xD800],       // trail then lead: two lone surrogates, not a pair
        vec![0xD83D, 0xD83D, 0xDE00], // lone lead followed by a real pair
        vec![0, 0x7F, 0x80, 0x7FF, 0x800, 0xFFFF],
    ];
    for c in &cases {
        let b = stash::wtf8_encode(c);
        assert_eq!(stash::wtf8_decode(&b).as_ref(), Some(c), "{c:x?} -> {b:x?}");
    }
    // A lone surrogate is three bytes (ED A0 BD for U+D83D), never U+FFFD.
    assert_eq!(stash::wtf8_encode(&[0xD83D]), vec![0xED, 0xA0, 0xBD]);
    // A pair is one 4-byte sequence, as in UTF-8.
    assert_eq!(stash::wtf8_encode(&u("😀")), "😀".as_bytes());
    // Not UTF-8 at all (Latin-1 é).
    assert_eq!(stash::wtf8_decode(&[0x61, 0xE9]), None);
}

#[test]
fn qstash1_key_names_escape_every_case_and_round_trip() {
    let cases: &[(&str, &str)] = &[
        ("vimOff", "vimOff"),
        ("", "%"),
        ("a/b", "a%2Fb"),
        ("100%", "100%25"),
        ("%", "%25"),
        ("a\0b", "a%00b"),
        (".hidden", "%2Ehidden"),
        ("a.b.", "a.b."),
        ("backup~", "backup%7E"),
        ("~home", "~home"),
        ("a~b", "a~b"),
        (".", "%2E"),
        ("~", "%7E"),
        (".~", "%2E%7E"),
        ("%h00", "%25h00"),
    ];
    for (key, name) in cases {
        let k = u(key);
        let enc = stash::encode_key_name(&k);
        assert_eq!(String::from_utf8_lossy(&enc), *name, "key {key:?}");
        assert_eq!(stash::decode_key_name(&enc), Some(k), "name {name:?}");
    }
    let lone = vec![0x2E, 0xD800, 0x7E]; // ".\uD800~"
    let enc = stash::encode_key_name(&lone);
    assert_eq!(enc, b"%2E\xED\xA0\x80%7E");
    assert_eq!(stash::decode_key_name(&enc), Some(lone));
    // Hand-made name with a stray `%` reads as a literal `%`.
    assert_eq!(stash::decode_key_name(b"50%off"), Some(u("50%off")));
}

#[test]
fn qstash1_set_load_remove_round_trip_through_files() {
    let d = TestDir::new("roundtrip");
    let s = Stash::open(&d.0).unwrap();
    let l = s.load(SITE, ORIGIN).unwrap();
    assert_eq!(l, Loaded::default());
    let keys: Vec<Vec<u16>> = vec![u("vimOff"), u(""), u("a/b"), u("%"), u("a\0b"), u(".x"), u("y~"), vec![0x61, 0xD83D], u("z")];
    let ops: Vec<Op> = keys.iter().enumerate().map(|(i, k)| Op::Set(k.clone(), vec![0xDC00, 0x30 + i as u16])).collect();
    assert_eq!(s.commit(SITE, ORIGIN, 0, &ops).unwrap(), CommitOutcome::Committed { skipped: 0 });
    // The documented layout.
    assert_eq!(std::fs::read(origin_dir(&d).join("vimOff")).unwrap(), b"\xED\xB0\x80\x30\n");
    assert!(d.0.join("storage/.locks").join(SITE).is_file());
    let l = s.load(SITE, ORIGIN).unwrap();
    let mut want: Vec<(Vec<u16>, Vec<u16>)> = keys.iter().enumerate().map(|(i, k)| (k.clone(), vec![0xDC00, 0x30 + i as u16])).collect();
    want.sort();
    assert_eq!(l.items, want);
    // No temp files left behind.
    assert!(file_names(&origin_dir(&d)).iter().all(|n| !n.starts_with('.')));

    s.commit(SITE, ORIGIN, 0, &[Op::Remove(u("vimOff")), Op::Set(u("z"), u("new")), Op::Remove(u("never-set"))]).unwrap();
    let l = s.load(SITE, ORIGIN).unwrap();
    assert!(!l.items.iter().any(|(k, _)| *k == u("vimOff")));
    assert!(l.items.contains(&(u("z"), u("new"))));
    assert_eq!(l.items.len(), keys.len() - 1);

    assert_eq!(s.clear(SITE, ORIGIN, 0).unwrap(), CommitOutcome::Committed { skipped: 0 });
    assert!(s.load(SITE, ORIGIN).unwrap().items.is_empty());
}

#[test]
fn qstash1_long_key_hashed_name_with_key_on_first_line() {
    let d = TestDir::new("longkey");
    let s = Stash::open(&d.0).unwrap();
    let long: Vec<u16> = u(&"k/".repeat(80)); // 80 * ("k" + "%2F") = 320 encoded bytes
    let short = u(&"k".repeat(200)); // exactly 200: not hashed
    s.commit(SITE, ORIGIN, 0, &[Op::Set(long.clone(), u("v\nw")), Op::Set(short.clone(), u("s"))]).unwrap();
    let names = file_names(&origin_dir(&d));
    let hashed: Vec<&String> = names.iter().filter(|n| n.starts_with("%h")).collect();
    assert_eq!(hashed.len(), 1, "{names:?}");
    assert_eq!(hashed[0].len(), 66);
    assert!(names.contains(&"k".repeat(200)));
    let data = std::fs::read(origin_dir(&d).join(hashed[0])).unwrap();
    let enc = stash::encode_key_name(&long);
    assert_eq!(data, [&enc[..], b"\nv\nw\n"].concat());
    let l = s.load(SITE, ORIGIN).unwrap();
    assert!(l.items.contains(&(long.clone(), u("v\nw"))));
    assert!(l.items.contains(&(short, u("s"))));
    s.commit(SITE, ORIGIN, 0, &[Op::Remove(long)]).unwrap();
    assert_eq!(file_names(&origin_dir(&d)), vec!["k".repeat(200)]);
}

#[test]
fn qstash1_hand_written_files_with_and_without_final_newline() {
    let d = TestDir::new("hand");
    let s = Stash::open(&d.0).unwrap();
    let o = origin_dir(&d);
    std::fs::create_dir_all(&o).unwrap();
    std::fs::write(o.join("nonl"), "1").unwrap();
    std::fs::write(o.join("nl"), "1\n").unwrap();
    std::fs::write(o.join("two"), "1\n\n").unwrap();
    std::fs::write(o.join("empty"), "").unwrap();
    std::fs::write(o.join("%"), "empty key\n").unwrap();
    let l = s.load(SITE, ORIGIN).unwrap();
    assert_eq!(
        l.items,
        vec![
            (u(""), u("empty key")),
            (u("empty"), u("")),
            (u("nl"), u("1")),
            (u("nonl"), u("1")),
            (u("two"), u("1\n")),
        ]
    );
}

#[test]
fn qstash1_dotfiles_and_backups_are_ignored() {
    let d = TestDir::new("ignored");
    let s = Stash::open(&d.0).unwrap();
    let o = origin_dir(&d);
    std::fs::create_dir_all(&o).unwrap();
    std::fs::write(o.join("key"), "v\n").unwrap();
    std::fs::write(o.join(".tmp-1-2"), "half written").unwrap();
    std::fs::write(o.join(".key.swp"), "vim swap").unwrap();
    std::fs::write(o.join("key~"), "backup").unwrap();
    std::fs::write(o.join("latin1"), b"\xE9\n").unwrap(); // not WTF-8: skipped, not lossy
    assert_eq!(s.load(SITE, ORIGIN).unwrap().items, vec![(u("key"), u("v"))]);
    // clear() unlinks key files only.
    s.clear(SITE, ORIGIN, 0).unwrap();
    assert_eq!(file_names(&o), vec![".key.swp", ".tmp-1-2", "key~"]);
}

#[test]
fn qstash1_two_file_handles_contend_on_the_lock() {
    let d = TestDir::new("lock");
    let s = Arc::new(Stash::open(&d.0).unwrap());
    let _ = s.load(SITE, ORIGIN).unwrap(); // creates the lock file
    let open = || std::fs::OpenOptions::new().read(true).write(true).open(s.lock_path(SITE)).unwrap();
    let (a, b) = (open(), open());
    a.lock().unwrap();
    assert!(b.try_lock_shared().is_err(), "exclusive blocks shared");
    assert!(b.try_lock().is_err(), "exclusive blocks exclusive");
    a.unlock().unwrap();
    a.lock_shared().unwrap();
    b.try_lock_shared().unwrap();
    b.unlock().unwrap();
    assert!(b.try_lock().is_err(), "shared blocks exclusive");
    a.unlock().unwrap();

    // stash's own load waits for a held exclusive lock.
    a.lock().unwrap();
    let s2 = s.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let t = std::thread::spawn(move || tx.send(s2.load(SITE, ORIGIN).unwrap()).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "load ran while the site was locked");
    a.unlock().unwrap();
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    t.join().unwrap();
}

#[test]
fn qstash1_commit_after_forget_with_stale_generation_is_dropped() {
    let d = TestDir::new("stale");
    let s = Stash::open(&d.0).unwrap();
    s.commit(SITE, ORIGIN, 0, &[Op::Set(u("a"), u("1"))]).unwrap();
    let page = s.load(SITE, ORIGIN).unwrap();
    assert_eq!(page.generation, 0);
    assert_eq!(s.forget(SITE).unwrap(), 1);
    assert!(!d.0.join("storage").join(SITE).exists());
    assert!(d.0.join("storage/.locks").join(SITE).exists(), "lock files are never deleted");
    assert_eq!(std::fs::read_to_string(s.lock_path(SITE)).unwrap(), "1\n");
    let w = Writer::spawn(Arc::new(Stash::open(&d.0).unwrap())).unwrap();
    let r = w.commit(SITE, ORIGIN, page.generation, vec![Op::Set(u("b"), u("2"))]);
    assert_eq!(r.recv().unwrap().unwrap(), CommitOutcome::Forgotten);
    assert!(!d.0.join("storage").join(SITE).exists());
    // A page loaded after the forget writes normally.
    let fresh = s.load(SITE, ORIGIN).unwrap();
    assert_eq!(fresh, Loaded { generation: 1, items: vec![] });
    assert_eq!(w.commit(SITE, ORIGIN, 1, vec![Op::Set(u("c"), u("3"))]).recv().unwrap().unwrap(), CommitOutcome::Committed { skipped: 0 });
    assert_eq!(s.load(SITE, ORIGIN).unwrap().items, vec![(u("c"), u("3"))]);
}

#[test]
fn qstash1_forget_while_a_commit_waits_does_not_resurrect() {
    let d = TestDir::new("forgetwait");
    let s = Arc::new(Stash::open(&d.0).unwrap());
    s.commit(SITE, ORIGIN, 0, &[Op::Set(u("a"), u("1"))]).unwrap();
    let page = s.load(SITE, ORIGIN).unwrap();
    let w = Writer::spawn(s.clone()).unwrap();
    let mut held = s.lock_exclusive(SITE).unwrap();
    let r = w.commit(SITE, ORIGIN, page.generation, vec![Op::Set(u("b"), u("2"))]);
    assert!(r.recv_timeout(Duration::from_millis(200)).is_err(), "commit ran while forget held the lock");
    s.forget_locked(SITE, &mut held).unwrap();
    drop(held);
    assert_eq!(r.recv_timeout(Duration::from_secs(10)).unwrap().unwrap(), CommitOutcome::Forgotten);
    w.flush();
    assert!(!d.0.join("storage").join(SITE).exists(), "the waiting commit recreated the site");
}

#[test]
fn qstash1_quota_checked_under_the_lock() {
    let d = TestDir::new("quota");
    let s = Stash::open(&d.0).unwrap();
    let big = vec![0x61u16; stash::QUOTA_UNITS - 10];
    assert_eq!(s.commit(SITE, ORIGIN, 0, &[Op::Set(u("big"), big.clone())]).unwrap(), CommitOutcome::Committed { skipped: 0 });
    // 3 + (QUOTA-10) + 1 + 6 = QUOTA: fits; one more unit does not.
    assert_eq!(s.commit(SITE, ORIGIN, 0, &[Op::Set(u("k"), u("123456"))]).unwrap(), CommitOutcome::Committed { skipped: 0 });
    assert_eq!(s.commit(SITE, ORIGIN, 0, &[Op::Set(u("k"), u("1234567"))]).unwrap(), CommitOutcome::Committed { skipped: 1 });
    // Replacing a value counts the new size only; removing first makes room in the same task.
    assert_eq!(
        s.commit(SITE, ORIGIN, 0, &[Op::Remove(u("big")), Op::Set(u("k"), u("1234567"))]).unwrap(),
        CommitOutcome::Committed { skipped: 0 }
    );
    assert_eq!(s.load(SITE, ORIGIN).unwrap().items, vec![(u("k"), u("1234567"))]);
}

#[test]
fn qstash1_site_and_origin_names() {
    let url = |s: &str| url::Url::parse(s).unwrap();
    assert_eq!(stash::site_dir_name(&url("https://social.loupax.com/x")).as_deref(), Some("https!loupax.com"));
    assert_eq!(stash::origin_dir_name(&url("https://social.loupax.com/x")).as_deref(), Some("https!social.loupax.com!443"));
    assert_eq!(stash::origin_dir_name(&url("http://Example.org:8080/")).as_deref(), Some("http!example.org!8080"));
    for opaque in ["file:///etc/hosts", "data:text/html,x", "about:blank"] {
        assert_eq!(stash::site_dir_name(&url(opaque)), None, "{opaque}");
        assert_eq!(stash::origin_dir_name(&url(opaque)), None, "{opaque}");
    }
    let d = TestDir::new("names");
    let s = Stash::open(&d.0).unwrap();
    assert!(s.load("../escape", ORIGIN).is_err());
    assert!(s.load(SITE, "a/b").is_err());
    assert!(s.forget(".locks").is_err());
}

#[test]
fn qstash1_state_dir_mode_and_unusable_fallback() {
    use std::os::unix::fs::PermissionsExt;
    let d = TestDir::new("mode");
    let s = Stash::open(&d.0.join("state")).unwrap();
    assert_eq!(std::fs::metadata(s.dir()).unwrap().permissions().mode() & 0o777, 0o700);
    // A regular file where the directory should be: unusable, the caller keeps localStorage in memory.
    std::fs::write(d.0.join("file"), "").unwrap();
    let bad = d.0.join("file").join("state");
    let e = Stash::open(&bad).unwrap_err();
    let line = stash::fallback_line(&bad, &e);
    assert!(line.starts_with(&format!("scratchy: stash: {}: ", bad.display())), "{line}");
    assert!(line.ends_with(", localStorage is in-memory for this session"), "{line}");
    assert!(Stash::open_or_log(&bad).is_none());
}

// ---------- QSTASH2: localStorage over stash ----------

use crate::js_storage::{self, Backend};

/// Installs a backend on `d` for this test thread (pages started here use it) and removes it on drop.
struct LsEnv;

impl LsEnv {
    fn new(d: &TestDir) -> LsEnv {
        js_storage::set_backend(Some(Backend::open(&d.0).expect("state dir usable")));
        LsEnv
    }
}

impl Drop for LsEnv {
    fn drop(&mut self) {
        js_storage::set_backend(None);
        set_page_url(None);
    }
}

/// One page load at `url` running `code`; returns the engine (alive) and its log.
fn ls_page(url: &str, html: &str, code: &str) -> (Option<JsEngine>, Rc<RefCell<Vec<String>>>) {
    set_page_url(Url::parse(url).ok().as_ref());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let e = crate::js::start_scripts(&parse_html(html), &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    (e, out)
}

/// `ls_page`, the engine dropped (its last ops handed over) and the writer flushed.
fn ls_load(url: &str, code: &str) -> Vec<String> {
    let (e, out) = ls_page(url, "", code);
    drop(e);
    if let Some(b) = js_storage::backend() {
        b.flush();
    }
    out.take()
}

const LS_URL: &str = "https://social.loupax.com/home";

#[test]
fn qstash2_a_value_survives_the_next_page_load_and_a_restart() {
    let d = TestDir::new("qstash2-persist");
    let env = LsEnv::new(&d);
    assert_eq!(ls_load(LS_URL, "console.log(localStorage.length); localStorage.vimOff = '1'; localStorage.setItem('b', 2);"), ["log:0"]);
    assert_eq!(std::fs::read(origin_dir(&d).join("vimOff")).unwrap(), b"1\n", "one file per key, on disk");
    // The next page load of the same origin (another path) sees it.
    assert_eq!(ls_load("https://social.loupax.com/other", "console.log(localStorage.getItem('vimOff'), localStorage.b, localStorage.length, localStorage.key(0));"), ["log:1 2 2 b"]);
    // Restart: a new backend (a new process) on the same --state-dir.
    drop(env);
    let _env = LsEnv::new(&d);
    assert_eq!(ls_load(LS_URL, "console.log(localStorage.vimOff); localStorage.removeItem('vimOff'); delete localStorage.b;"), ["log:1"]);
    assert_eq!(ls_load(LS_URL, "console.log(localStorage.length, localStorage.getItem('vimOff'));"), ["log:0 null"]);
    // Same site, other origin: its own storage.
    assert_eq!(ls_load(LS_URL, "localStorage.x = 'mine';"), Vec::<String>::new());
    assert_eq!(ls_load("https://loupax.com/", "console.log(localStorage.getItem('x'));"), ["log:null"]);
    // clear() empties the origin's files.
    assert_eq!(ls_load(LS_URL, "localStorage.clear(); console.log(localStorage.length);"), ["log:0"]);
    assert_eq!(file_names(&origin_dir(&d)), Vec::<String>::new());
}

#[test]
fn qstash2_opaque_and_unconfigured_pages_do_not_persist() {
    let d = TestDir::new("qstash2-opaque");
    {
        let _env = LsEnv::new(&d);
        for url in ["file:///home/u/page.html", "about:blank", "data:text/html,x"] {
            assert_eq!(ls_load(url, "localStorage.k = 'v'; console.log(localStorage.k);"), ["log:v"], "{url}");
            assert_eq!(ls_load(url, "console.log(localStorage.getItem('k'));"), ["log:null"], "{url}: in memory only");
        }
        assert_eq!(file_names(&d.0.join("storage")), [".locks"], "no site directory was made");
    }
    // No backend (tests, --wpt, --js-report without --state-dir): in memory, nothing written.
    assert_eq!(ls_load(LS_URL, "localStorage.k = 'v';"), Vec::<String>::new());
    assert_eq!(ls_load(LS_URL, "console.log(localStorage.getItem('k'));"), ["log:null"]);
    set_page_url(None);
}

#[test]
fn qstash2_quota_throws_synchronously_and_nothing_is_recorded() {
    let d = TestDir::new("qstash2-quota");
    let env = LsEnv::new(&d);
    // Exactly stash::QUOTA_UNITS code units (key + value) fit; one more unit throws.
    let code = format!(
        "var big = 'x'.repeat({} - 1); localStorage.setItem('k', big); console.log(localStorage.k.length);\
         try {{ localStorage.setItem('j', ''); }} catch (e) {{ console.log(e.name, e instanceof DOMException, e.code); }}\
         try {{ localStorage.j = '1'; }} catch (e) {{ console.log('prop', e.name); }}\
         console.log(localStorage.length, localStorage.getItem('j'));\
         localStorage.setItem('k', big.slice(2)); localStorage.setItem('j', 'y'); console.log(localStorage.length);",
        stash::QUOTA_UNITS
    );
    let log = ls_load(LS_URL, &code);
    let want = [format!("log:{}", stash::QUOTA_UNITS - 1), "log:QuotaExceededError true 22".into(), "log:prop QuotaExceededError".into(), "log:1 null".into(), "log:2".into()];
    assert_eq!(log, want);
    assert_eq!(file_names(&origin_dir(&d)), ["j", "k"], "the writer accepted what the page accepted");
    // In memory (no state dir) the same limit holds.
    drop(env);
    let log = ls_load(LS_URL, &format!("try {{ localStorage.setItem('k', 'x'.repeat({})); }} catch (e) {{ console.log(e.name); }}", stash::QUOTA_UNITS));
    assert_eq!(log, ["log:QuotaExceededError"]);
    set_page_url(None);
}

#[test]
fn qstash2_one_tasks_ops_are_one_commit() {
    let d = TestDir::new("qstash2-task");
    let _env = LsEnv::new(&d);
    let (e, _out) = ls_page(LS_URL, "", "localStorage.a = '1';");
    let e = e.unwrap();
    let (persist, sent0, queued) = e.storage.stats();
    assert!(persist);
    assert_eq!(queued, 0, "the page script's task ended: its op was handed over");
    assert!(sent0 >= 1);
    // One task (a callback): three ops, one commit, all on disk together.
    e.eval_value::<bool>("task", "localStorage.b = '2'; localStorage.c = '3'; localStorage.removeItem('a'); true");
    let (_, sent1, queued) = e.storage.stats();
    assert_eq!((sent1 - sent0, queued), (1, 0));
    js_storage::backend().unwrap().flush();
    assert_eq!(file_names(&origin_dir(&d)), ["b", "c"]);
    // A task with no change (reads, the same value again) sends nothing.
    e.eval_value::<bool>("task", "localStorage.getItem('b'); localStorage.b = '2'; true");
    assert_eq!(e.storage.stats().1, sent1);
}

#[test]
fn qstash2_lone_surrogates_round_trip_exactly() {
    let d = TestDir::new("qstash2-wtf8");
    let _env = LsEnv::new(&d);
    assert_eq!(ls_load(LS_URL, "localStorage.setItem('v', 'a\\uD83D'); localStorage.setItem('k\\uDC00', 'b\\uD83D\\uDE00c');"), Vec::<String>::new());
    // WTF-8 on disk: the lone high surrogate D83D is ED A0 BD (never EF BF BD, U+FFFD).
    assert_eq!(std::fs::read(origin_dir(&d).join("v")).unwrap(), b"a\xED\xA0\xBD\n");
    let log = ls_load(
        LS_URL,
        "var v = localStorage.getItem('v'), w = localStorage.getItem('k\\uDC00');\
         console.log(v === 'a\\uD83D', v.length, v.charCodeAt(1).toString(16), w === 'b\\uD83D\\uDE00c', localStorage.key(0) === 'k\\uDC00');",
    );
    assert_eq!(log, ["log:true 2 d83d true true"]);
}

#[test]
fn qstash2_a_same_origin_iframe_shares_the_pages_storage() {
    let d = TestDir::new("qstash2-frame");
    let _env = LsEnv::new(&d);
    let html = r#"<iframe srcdoc="<script>localStorage.fromFrame = 'f'; parent.seen = localStorage.getItem('fromPage');</script>"></iframe>"#;
    let (e, out) = ls_page(LS_URL, html, "localStorage.fromPage = 'p'; addEventListener('load', function () { console.log(window.seen, localStorage.fromFrame); });");
    drop(e);
    js_storage::backend().unwrap().flush();
    assert_eq!(*out.borrow(), ["log:p f"]);
    assert_eq!(file_names(&origin_dir(&d)), ["fromFrame", "fromPage"]);
}

#[test]
fn qstash2_state_dir_resolution() {
    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_string())
    }
    let all: &'static [(&str, &str)] = &[("SCRATCHY_STATE_DIR", "/s"), ("XDG_STATE_HOME", "/x"), ("HOME", "/h")];
    assert_eq!(js_storage::resolve_state_dir(Some("/f".into()), env(all)), Some(PathBuf::from("/f")));
    assert_eq!(js_storage::resolve_state_dir(None, env(all)), Some(PathBuf::from("/s")));
    assert_eq!(js_storage::resolve_state_dir(None, env(&[("SCRATCHY_STATE_DIR", ""), ("XDG_STATE_HOME", "/x"), ("HOME", "/h")])), Some(PathBuf::from("/x/scratchy")));
    assert_eq!(js_storage::resolve_state_dir(None, env(&[("HOME", "/h")])), Some(PathBuf::from("/h/.local/state/scratchy")));
    assert_eq!(js_storage::resolve_state_dir(None, env(&[])), None);
}

#[test]
fn qstash2_an_unusable_state_dir_keeps_localstorage_in_memory() {
    let d = TestDir::new("qstash2-bad");
    std::fs::create_dir_all(d.0.parent().unwrap()).unwrap();
    std::fs::write(&d.0, b"a file, not a directory").unwrap();
    assert!(Backend::open(&d.0).is_none());
    js_storage::set_backend(None);
    assert_eq!(ls_load(LS_URL, "localStorage.k = 'v'; console.log(localStorage.k);"), ["log:v"]);
    set_page_url(None);
    let _ = std::fs::remove_file(&d.0);
}

/// The App path: `handle_nav_result` loads a page (the old page's engine is dropped after the new one
/// starts, so the load must read the writer's queue back), then a new App on the same state dir.
#[test]
fn qstash2_app_page_loads_and_a_new_app_see_the_value() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let Ok(event_loop) = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() else {
        return; // no display available (headless CI)
    };
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let was = js_enabled();
    set_js_enabled(true);
    let d = TestDir::new("qstash2-app");
    let env = LsEnv::new(&d);
    let page = |n: u32, code: &str| FetchedPage {
        html: b"<html><body>x</body></html>".to_vec(),
        extra_css: String::new(),
        sheets: Vec::new(),
        scripts: vec![src(code)],
        url: Url::parse(&format!("https://social.loupax.com/p{n}")).unwrap(),
    };
    let nav = |app: &mut App, n: u32, code: &str| {
        let url = Url::parse(&format!("https://social.loupax.com/p{n}")).unwrap();
        app.history = vec![url.clone()];
        app.history_index = 0;
        app.fetch_url(url);
        let g = app.nav_generation;
        app.handle_nav_result(g, Ok(page(n, code)));
    };
    let read = |app: &App| app.js.as_ref().and_then(|e| e.eval_value::<String>("t", "String(localStorage.getItem('vimOff'))"));
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    nav(&mut app, 1, "localStorage.vimOff = '1';");
    nav(&mut app, 2, "");
    assert_eq!(read(&app).as_deref(), Some("1"), "the next page load");
    drop(app);
    drop(env);
    let _env = LsEnv::new(&d);
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    nav(&mut app, 3, "");
    assert_eq!(read(&app).as_deref(), Some("1"), "a new App with the same state dir");
    drop(app);
    set_js_enabled(was);
}

// ---------- QSTASH4: sessionStorage across navigations ----------

#[test]
fn qstash4_session_storage_survives_navigation_per_origin_and_never_touches_disk() {
    let d = TestDir::new("qstash4");
    let env = LsEnv::new(&d);
    assert_eq!(ls_load(LS_URL, "sessionStorage.a = '1'; sessionStorage.setItem('b', '2'); sessionStorage.setItem('c', '3'); sessionStorage.removeItem('b'); console.log(sessionStorage.length);"), ["log:2"]);
    // The next page of the same origin: the pairs, in insertion order.
    assert_eq!(ls_load("https://social.loupax.com/other", "console.log(sessionStorage.length, sessionStorage.getItem('a'), sessionStorage.key(0), sessionStorage.key(1), sessionStorage.b); sessionStorage.a = 'x'; sessionStorage.d = '4';"), ["log:2 1 a c undefined"]);
    assert_eq!(ls_load(LS_URL, "console.log(sessionStorage.a, sessionStorage.d, sessionStorage.key(2));"), ["log:x 4 d"]);
    // Another origin (other host, scheme, port) does not see it.
    for url in ["https://loupax.com/", "http://social.loupax.com/", "https://social.loupax.com:8443/"] {
        assert_eq!(ls_load(url, "console.log(sessionStorage.length, sessionStorage.getItem('a'));"), ["log:0 null"], "{url}");
    }
    // It is not localStorage, and nothing about it is on disk.
    assert_eq!(ls_load(LS_URL, "console.log(localStorage.length);"), ["log:0"]);
    assert_eq!(file_names(&d.0.join("storage")), [".locks"], "no site directory was made");
    // clear() empties the process copy.
    assert_eq!(ls_load(LS_URL, "sessionStorage.clear();"), Vec::<String>::new());
    assert_eq!(ls_load(LS_URL, "console.log(sessionStorage.length);"), ["log:0"]);
    // Opaque origins keep a per-page area.
    for url in ["file:///home/u/page.html", "about:blank"] {
        assert_eq!(ls_load(url, "sessionStorage.k = 'v';"), Vec::<String>::new());
        assert_eq!(ls_load(url, "console.log(sessionStorage.getItem('k'));"), ["log:null"], "{url}");
    }
    // Without a state directory it works the same (it never used one).
    drop(env);
    assert_eq!(ls_load(LS_URL, "sessionStorage.z = 'q';"), Vec::<String>::new());
    assert_eq!(ls_load(LS_URL, "console.log(sessionStorage.z);"), ["log:q"]);
    set_page_url(None);
}
