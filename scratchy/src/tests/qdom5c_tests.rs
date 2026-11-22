// QDOM5c: a script-made same-origin iframe keeps its initial about:blank Window when its page arrives (it is navigated
// in place, as in browsers): contentWindow and window.navigator keep their identity across the load.
use super::*;

#[test]
fn qdom5c_initial_window_is_reused_by_the_async_load() {
    let fake = QfFake::new(vec![("/blank.html", qf7_ok("<!doctype html><title>t</title><p id=x>hi</p>", &[("content-type", "text/html")]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "var f = document.createElement('iframe'); f.src = '/blank.html'; document.body.appendChild(f);\
         var w = f.contentWindow, nav = w.navigator, d0 = w.location.href;\
         var f2 = document.createElement('iframe'); document.body.appendChild(f2);\
         var w2 = f2.contentWindow, nav2 = w2.navigator; f2.src = '/blank.html';\
         var n = 0, done = function () {\
           if (++n < 2) return;\
           console.log('same', f.contentWindow === w, w.navigator === nav, f2.contentWindow === w2, w2.navigator === nav2);\
           console.log('loaded', w.location.href, w.document.getElementById('x') !== null, w2.document.URL, w.closed);\
         };\
         f.onload = done; f2.onload = done; console.log('before', d0);",
    );
    // The fetch ends through the net drive, the load itself runs from timers: alternate until quiet.
    for _ in 0..20 {
        qf7_drive(&engine, &fake);
        std::thread::sleep(std::time::Duration::from_millis(5));
        engine.run_due();
    }
    let l = log.borrow().clone();
    assert!(l.contains(&"log:before about:blank".to_string()), "{l:?}");
    assert!(l.contains(&"log:same true true true true".to_string()), "{l:?}");
    assert!(l.contains(&"log:loaded https://page.test/blank.html true https://page.test/blank.html false".to_string()), "{l:?}");
}
