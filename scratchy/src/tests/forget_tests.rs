// QSTASH3: `scratchy --forget-site` (src/forget.rs, STASH.md "Forget site"). Real disk under target/, never /tmp.
use crate::stash::{CommitOutcome, Op, Stash, Writer};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("forget-tests").join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
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

struct Fake {
    required: String,
    raw: Mutex<Vec<String>>,
    fail: bool,
}

impl crate::js_net::NetExecutor for Fake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<crate::net::WireResponse, crate::net::NetError> {
        if self.fail {
            return Err(crate::net::NetError::Connect("refused".into()));
        }
        let text = String::from_utf8_lossy(&job.request).to_string();
        self.raw.lock().unwrap().push(text.clone());
        let ok = text.contains(&format!("X-Wire-Control-Token: {}", self.required)) && text.contains("X-Wire-Cookie-Op: clear-site");
        Ok(crate::net::WireResponse { status: if ok { 204 } else { 403 }, ..Default::default() })
    }
}

fn fake(required: &str, fail: bool) -> Fake {
    Fake { required: required.into(), raw: Mutex::new(Vec::new()), fail }
}

#[test]
fn qstash3_forget_site_removes_directory_bumps_generation_and_drops_later_commit() {
    let d = TestDir::new("cli");
    let s = Arc::new(Stash::open(&d.0).unwrap());
    s.commit(SITE, ORIGIN, 0, &[Op::Set(u("a"), u("1"))]).unwrap();
    s.commit("https!other.org", "https!other.org!443", 0, &[Op::Set(u("k"), u("v"))]).unwrap();
    let page = s.load(SITE, ORIGIN).unwrap();
    let w = Writer::spawn(s.clone()).unwrap();
    let sock = d.0.join("wire.sock");
    std::fs::write(crate::net_policy::control_token_path(&sock), "tok1\n").unwrap();
    let f = fake("tok1", false);
    let (lines, code) = crate::forget::forget_site(&d.0, "https://social.loupax.com/x", Some((&sock, &f)));
    assert_eq!(code, 0, "{lines:?}");
    assert!(lines.last().unwrap().contains("cookies of") && !lines.last().unwrap().contains("not cleared"), "{lines:?}");
    assert!(!d.0.join("storage").join(SITE).exists());
    assert!(d.0.join("storage/https!other.org").exists(), "only that site is removed");
    assert_eq!(std::fs::read_to_string(s.lock_path(SITE)).unwrap(), "1\n");
    let raw = f.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 1);
    assert!(raw[0].contains("X-Wire-Top-Site: https://loupax.com"), "{raw:?}");
    let r = w.commit(SITE, ORIGIN, page.generation, vec![Op::Set(u("b"), u("2"))]);
    assert_eq!(r.recv().unwrap().unwrap(), CommitOutcome::Forgotten);
    assert!(!d.0.join("storage").join(SITE).exists());
}

#[test]
fn qstash3_unreachable_wire_still_forgets_and_says_so() {
    let d = TestDir::new("nowire");
    let s = Stash::open(&d.0).unwrap();
    s.commit(SITE, ORIGIN, 0, &[Op::Set(u("a"), u("1"))]).unwrap();
    let sock = d.0.join("wire.sock");
    std::fs::write(crate::net_policy::control_token_path(&sock), "tok1\n").unwrap();
    let f = fake("tok1", true);
    let (lines, code) = crate::forget::forget_site(&d.0, "loupax.com", Some((&sock, &f)));
    assert_eq!(code, 0);
    assert!(lines.last().unwrap().contains("cookies were not cleared"), "{lines:?}");
    assert!(!d.0.join("storage").join(SITE).exists());
    let (lines, _) = crate::forget::forget_site(&d.0, "loupax.com", None);
    assert!(lines.last().unwrap().contains("cookies were not cleared"), "{lines:?}");
    let (lines, code) = crate::forget::forget_site(&d.0, "../..", None);
    assert_ne!(code, 0, "{lines:?}");
}

#[test]
fn qstash3_403_rereads_the_token_and_retries_once() {
    let d = TestDir::new("tok");
    let sock = d.0.join("wire.sock");
    std::fs::write(crate::net_policy::control_token_path(&sock), "stale-tok\n").unwrap();
    crate::net_policy::control_token(&sock, true).unwrap();
    std::fs::write(crate::net_policy::control_token_path(&sock), "new-tok\n").unwrap();
    let f = fake("new-tok", false);
    let (lines, code) = crate::forget::forget_site(&d.0, "https://loupax.com", Some((&sock, &f)));
    assert_eq!(code, 0, "{lines:?}");
    assert!(!lines.last().unwrap().contains("not cleared"), "{lines:?}");
    assert_eq!(f.raw.lock().unwrap().len(), 2);
}
