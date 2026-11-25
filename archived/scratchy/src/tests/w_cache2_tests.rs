// W-CACHE2 (scratchy side): `--forget-site` sends the `clear-site` control op, which wire answers
// by also removing the site's cache partition. The op is the same request as before (no new op
// name): the test pins the headers wire's partition clearing relies on.

use std::sync::Mutex;

struct Capture(Mutex<Vec<String>>);

impl crate::js_net::NetExecutor for Capture {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<crate::net::WireResponse, crate::net::NetError> {
        self.0.lock().unwrap().push(String::from_utf8_lossy(&job.request).to_string());
        Ok(crate::net::WireResponse { status: 204, ..Default::default() })
    }
}

#[test]
fn w_cache2_forget_site_sends_clear_site_for_the_sites_partition() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("w-cache2-tests").join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("wire.sock");
    std::fs::write(crate::net_policy::control_token_path(&sock), "tok2\n").unwrap();
    let f = Capture(Mutex::new(Vec::new()));
    let (lines, code) = crate::forget::forget_site(&dir, "https://www.example.org/some/page", Some((&sock, &f)));
    assert_eq!(code, 0, "{lines:?}");
    let raw = f.0.lock().unwrap().clone();
    assert_eq!(raw.len(), 1, "one control request, no second op for the cache: {raw:?}");
    for want in ["X-Wire-Cookie-Op: clear-site", "X-Wire-Top-Site: https://example.org", "X-Wire-Jar: scratchy", "X-Wire-Control-Token: tok2"] {
        assert!(raw[0].contains(want), "{want} in {:?}", raw[0]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
