// W-CACHE1 (scratchy side): fetch()'s `cache` option and the reload commands
// become the standard request headers wire's HTTP cache reads, and wire's
// `X-Wire-Cache` response header never reaches the page.

use super::*;
use crate::net::{reload_headers, with_reload_revalidate};
use crate::net_policy::cache_mode_headers;

fn hs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn w_cache1_fetch_cache_modes_map_to_standard_headers() {
    let m = |mode: &str| cache_mode_headers(mode, &[]);
    assert!(m("default").is_empty() && m("").is_empty());
    assert_eq!(m("no-store"), hs(&[("Cache-Control", "no-store")]));
    assert_eq!(m("reload"), hs(&[("Cache-Control", "no-cache"), ("Pragma", "no-cache")]));
    assert_eq!(m("no-cache"), hs(&[("Cache-Control", "max-age=0")]));
    assert_eq!(m("force-cache"), hs(&[("Cache-Control", "max-stale")]));
    assert_eq!(m("only-if-cached"), hs(&[("Cache-Control", "only-if-cached")]));
    // The page's own headers win (Fetch: only when the header list lacks them).
    assert!(cache_mode_headers("no-cache", &hs(&[("cache-control", "max-age=5")])).is_empty());
    assert_eq!(cache_mode_headers("reload", &hs(&[("Cache-Control", "x")])), hs(&[("Pragma", "no-cache")]));
}

#[test]
fn w_cache1_reload_headers() {
    assert!(reload_headers(false, &[]).is_empty());
    assert_eq!(reload_headers(true, &[]), hs(&[("Cache-Control", "no-cache"), ("Pragma", "no-cache")]));
    assert!(reload_headers(true, &hs(&[("Cache-Control", "max-stale")])).is_empty(), "fetch(.., {{cache}}) wins");
    let t = "GET https://a.test/ HTTP/1.1\r\nHost: a.test\r\n\r\n";
    assert_eq!(with_reload_revalidate(t), "GET https://a.test/ HTTP/1.1\r\nCache-Control: max-age=0\r\nHost: a.test\r\n\r\n");
    let hard = "GET https://a.test/ HTTP/1.1\r\nHost: a.test\r\nCache-Control: no-cache\r\n\r\n";
    assert_eq!(with_reload_revalidate(hard), hard, "a hard reload keeps its no-cache");
}

fn sent_heads(fake: &QfFake) -> Vec<String> {
    fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_string()).collect()
}

#[test]
fn w_cache1_fetch_sends_the_cache_mode_without_a_preflight() {
    let fake = QfFake::new(vec![("/c", qf7_ok("x", &[("access-control-allow-origin", "*")]))]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://other.test/c1', {cache: 'no-store'}).then(function(r){ console.log('a', r.status); });\
         fetch('/c2', {cache: 'reload'}).then(function(r){ console.log('b', r.status); });\
         fetch('/c3', {cache: 'only-if-cached', mode: 'same-origin'}).then(function(r){ console.log('c', r.status); });\
         fetch('/c4').then(function(r){ console.log('d', r.status); });",
    );
    assert!(["log:a 200", "log:b 200", "log:c 200", "log:d 200"].iter().all(|x| l.contains(&x.to_string())), "{l:?}");
    let heads = sent_heads(&fake);
    assert_eq!(heads.len(), 4, "a cross-origin GET with a cache mode needs no preflight: {heads:?}");
    let find = |p: &str| heads.iter().find(|h| h.contains(p)).cloned().unwrap();
    assert!(find("/c1").contains("\r\nCache-Control: no-store\r\n"), "{}", find("/c1"));
    let b = find("/c2");
    assert!(b.contains("\r\nCache-Control: no-cache\r\n") && b.contains("\r\nPragma: no-cache\r\n"), "{b}");
    assert!(find("/c3").contains("\r\nCache-Control: only-if-cached\r\n"));
    assert!(!find("/c4").to_ascii_lowercase().contains("cache-control"), "default sends nothing");
}

#[test]
fn w_cache1_x_wire_cache_is_hidden_from_the_page() {
    let fake = QfFake::new(vec![("/a", qf7_ok("x", &[("x-wire-cache", "hit"), ("age", "3")]))]);
    let l = qf8_run("https://page.test/", &fake, "fetch('/a').then(function(r){ console.log('h', r.headers.get('x-wire-cache'), r.headers.get('age')); });");
    assert!(l.contains(&"log:h null 3".to_string()), "{l:?}");
}
