//! QV8-4: the V8 startup snapshot (js_engine_v8.rs `Snapshot`). A page engine made from it must be
//! indistinguishable from one that ran the preludes itself. Only built with `--features engine-v8`.
#![cfg(feature = "engine-v8")]
#[allow(unused_imports)]
use super::*;

// (sorted: V8 places its lazily installed builtins - Temporal, WebAssembly... - at a different spot in the snapshot)
const PROBE: &str = "console.log(JSON.stringify({\
    names: Object.getOwnPropertyNames(globalThis).sort(),\
    href: location.href, host: location.host, origin: location.origin, search: location.search, hash: location.hash,\
    ua: navigator.userAgent, app: navigator.appVersion, secure: isSecureContext,\
    iw: innerWidth, ih: innerHeight, ow: outerWidth, oh: outerHeight, sw: screen.width,\
    url: document.URL, duri: document.documentURI, title: document.title, all: typeof document.all,\
    hist: history.length, histState: history.state,\
    origin_time: Math.abs(performance.timeOrigin - Date.now()) < 10000,\
    timing: Math.abs(performance.timing.navigationStart - Date.now()) < 10000,\
    env_left: typeof __scratchy_env, ce: typeof customElements, fetch: typeof fetch, ws: typeof WebSocket,\
    intl: typeof Intl, b64: typeof atob, qm: typeof queueMicrotask,\
    video: document.querySelectorAll('video').length\
  }));";

fn probe_with(url: &str, html: &str, slow: bool) -> (String, bool) {
    set_page_url(Url::parse(url).ok().as_ref());
    let run = || {
        let doc = parse_html(html);
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let scripts = vec![src(PROBE)];
        let engine = start_scripts(&doc, &scripts, Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
        let line = out.borrow().join("\n");
        (line, engine.eng.prebuilt())
    };
    let r = if slow { crate::js_engine::without_snapshot(run) } else { run() };
    set_page_url(None);
    r
}

#[test]
fn qv8_4_snapshot_engine_matches_the_slow_start() {
    for (url, html) in [
        ("https://page.test:8443/a/b?q=1#h", "<html><head><title>T</title></head><body><p>x</p></body></html>"),
        ("http://example.test/", "<html><body><video src=a.mp4></video></body></html>"),
        ("file:///tmp/x.html", "<html><body></body></html>"),
    ] {
        let (fast, fast_pre) = probe_with(url, html, false);
        let (slow, slow_pre) = probe_with(url, html, true);
        assert!(fast_pre, "page engines use the snapshot");
        assert!(!slow_pre, "the slow path is selectable");
        assert!(fast.starts_with("log:{"), "{fast}");
        assert_eq!(fast, slow, "snapshot and slow start differ for {url}");
    }
}

#[test]
fn qv8_4_page_state_is_the_pages_not_the_snapshots() {
    let (a, _) = probe_with("https://one.test/x", "<html></html>", false);
    let (b, _) = probe_with("http://two.test:81/y?z", "<html></html>", false);
    assert!(a.contains("\"href\":\"https://one.test/x\"") && a.contains("\"secure\":true"), "{a}");
    assert!(b.contains("\"href\":\"http://two.test:81/y?z\"") && b.contains("\"secure\":false"), "{b}");
    assert!(a.contains("\"url\":\"https://one.test/x\"") && b.contains("\"url\":\"http://two.test:81/y?z\""), "{a} {b}");
    assert!(a.contains("\"env_left\":\"undefined\"") && a.contains("\"all\":\"undefined\""), "{a}");
}

#[test]
fn qv8_4_engines_from_one_snapshot_are_independent() {
    // Two live engines: each one's natives talk to its own closures (console to its own log), a
    // global written in one is not in the other, and Math.random is not a shared state.
    let (e1, out1) = qb8_start("<html><body><script>var mine = 1; console.log('one', Math.random());</script></body></html>");
    let (e2, out2) = qb8_start("<html><body><script>console.log('two', typeof mine, Math.random());</script></body></html>");
    assert!(e1.eng.prebuilt() && e2.eng.prebuilt());
    let (o1, o2) = (out1.borrow().clone(), out2.borrow().clone());
    assert!(o1[0].starts_with("log:one 0."), "{o1:?}");
    assert!(o2[0].starts_with("log:two undefined 0."), "{o2:?}");
    let r1 = o1[0].rsplit(' ').next().unwrap();
    let r2 = o2[0].rsplit(' ').next().unwrap();
    assert_ne!(r1, r2, "Math.random must not repeat across isolates");
    assert_eq!(o1.len(), 1, "{o1:?}");
    assert_eq!(o2.len(), 1, "{o2:?}");
}

#[test]
fn qv8_4_dom_of_the_page_not_the_snapshots_empty_document() {
    // Nodes of the real document are wrapped fresh (no wrapper cached from the empty baked one).
    let (_e, out) = qb8_start(
        "<html><body><div id=a class=c>hi</div><script>var d = document.getElementById('a');\
         console.log(d.tagName, d.className, document.body.children.length, document.documentElement.tagName, document.querySelectorAll('div').length);</script></body></html>",
    );
    assert_eq!(*out.borrow(), ["log:DIV c 2 HTML 1"]);
}
