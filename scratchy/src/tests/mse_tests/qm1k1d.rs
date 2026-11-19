// QM1k1d: JS MediaSource + SourceBufferList + attachment (src/mse_prelude.js), FakeBackend.
use super::super::*;
use crate::media_el::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Js {
    engine: JsEngine,
    clock: Rc<Cell<f64>>,
    log: Rc<RefCell<Vec<String>>>,
    fakes: Rc<RefCell<Vec<FakeHandle>>>,
}

impl Js {
    fn new(html: &str, code: &str) -> Js {
        let fakes: Rc<RefCell<Vec<FakeHandle>>> = Default::default();
        let m = fakes.clone();
        set_backend_factory(Some(Rc::new(move |_| {
            let f = fake_new();
            m.borrow_mut().push(f.clone());
            Box::new(FakeBackend(f))
        })));
        set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let clock = Rc::new(Cell::new(0.0));
        let engine = crate::js::start_scripts_clocked(&parse_html(html), &[src(code)], Rc::new(move |lv, t| l.borrow_mut().push(format!("{lv}:{t}"))), Some(clock.clone())).unwrap();
        set_backend_factory(None);
        Js { engine, clock, log, fakes }
    }

    fn step(&self, ms: f64) {
        let end = self.clock.get() + ms;
        for _ in 0..2000 {
            match self.engine.next_due_ms() {
                Some(d) if d <= end => {
                    self.clock.set(self.clock.get().max(d));
                    self.engine.run_due();
                }
                _ => break,
            }
        }
        self.clock.set(end);
    }

    fn run(&self, code: &str) {
        self.engine.eval("t", code);
        self.step(50.0);
    }

    fn val(&self, code: &str) -> String {
        self.engine.eval_value::<String>("t", &format!("String({code})")).unwrap()
    }

    fn lines(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn feeds(&self) -> usize {
        self.engine.media.as_ref().unwrap().borrow().mse.len()
    }
}

const HTML: &str = "<video id=v muted></video>";
const SETUP: &str = "var v = document.getElementById('v'); var ms = new MediaSource(); var ev = function (t, n) { t.addEventListener(n, function () { console.log(n); }); };\
    ['sourceopen', 'sourceended', 'sourceclose'].forEach(function (n) { ev(ms, n); });\
    ['loadstart', 'durationchange', 'error', 'emptied'].forEach(function (n) { ev(v, n); });\
    var errName = function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } };\
    var url = URL.createObjectURL(ms); v.src = url;";

fn attached() -> Js {
    let js = Js::new(HTML, SETUP);
    js.step(100.0);
    js
}

#[test]
fn qm1k1d_api_surface_and_is_type_supported() {
    let js = Js::new(
        HTML,
        "var S = MediaSource.isTypeSupported;\
         console.log([typeof MediaSource, typeof SourceBuffer, typeof SourceBufferList, MediaSource.canConstructInDedicatedWorker,\
           S('video/mp4; codecs=\"avc1.64001f\"'), S('audio/mp4; codecs=\"mp4a.40.2\"'), S('video/webm; codecs=\"vp9\"'), S('foo/bar'), S('')].join());\
         var m = new MediaSource(); console.log([m.readyState, m.duration, m.sourceBuffers.length, m.activeSourceBuffers.length, m.sourceBuffers === m.sourceBuffers, m instanceof EventTarget].join());\
         try { MediaSource() } catch (e) { console.log(e.name) }\
         try { new SourceBuffer() } catch (e) { console.log(e.name) }\
         try { S() } catch (e) { console.log(e.name) }",
    );
    js.step(10.0);
    assert_eq!(js.lines(), ["log:function,function,function,false,true,true,false,false,false", "log:closed,NaN,0,0,true,true", "log:TypeError", "log:TypeError", "log:TypeError"]);
}

#[test]
fn qm1k1d_sourceopen_from_a_task_after_attach() {
    let js = Js::new(HTML, SETUP);
    // nothing is synchronous: the load starts from a task
    assert!(js.lines().is_empty());
    assert_eq!(js.val("ms.readyState"), "closed");
    js.step(100.0);
    assert_eq!(js.lines(), ["log:loadstart", "log:sourceopen"]);
    assert_eq!(js.val("ms.readyState"), "open");
    assert_eq!(js.val("[v.networkState, v.currentSrc !== '', v.error === null].join()"), "2,true,true");
    assert_eq!(js.feeds(), 1);
    // no backend load until an init segment exists
    assert!(js.fakes.borrow().iter().all(|f| f.borrow().calls.iter().all(|c| !c.starts_with("load_mse"))));
}

#[test]
fn qm1k1d_src_object_attaches_and_getter_returns_it() {
    let js = Js::new(
        HTML,
        "var v = document.getElementById('v'); var ms = new MediaSource(); ms.addEventListener('sourceopen', function () { console.log('open'); });\
         console.log(v.srcObject === null); v.srcObject = ms; console.log(v.srcObject === ms);",
    );
    js.step(100.0);
    assert_eq!(js.lines(), ["log:true", "log:true", "log:open"]);
    assert_eq!(js.val("document.getElementById('v').srcObject === ms ? ms.readyState : 'x'"), "open");
}

#[test]
fn qm1k1d_add_source_buffer_errors_in_the_right_state() {
    let js = Js::new(HTML, SETUP);
    // closed (not attached yet)
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"') })"), "InvalidStateError");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('') })"), "TypeError");
    js.step(100.0);
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('') })"), "TypeError");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer() })"), "TypeError");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('video/webm; codecs=\"vp9\"') })"), "NotSupportedError");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('text/plain') })"), "NotSupportedError");
    assert_eq!(js.val("ms.sourceBuffers.length"), "0");
    js.run(
        "ms.sourceBuffers.addEventListener('addsourcebuffer', function () { console.log('added', ms.sourceBuffers.length); });\
         var a = ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"'); var b = ms.addSourceBuffer('audio/mp4; codecs=\"mp4a.40.2\"');",
    );
    assert_eq!(js.lines(), ["log:loadstart", "log:sourceopen", "log:added 2", "log:added 2"]);
    assert_eq!(js.val("[ms.sourceBuffers.length, ms.sourceBuffers[0] === a, ms.sourceBuffers[1] === b, ms.sourceBuffers.item(1) === b, ms.sourceBuffers.item(2), ms.activeSourceBuffers.length, a.updating, a instanceof SourceBuffer, a instanceof EventTarget].join()"), "2,true,true,true,,2,false,true,true");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('audio/mp4; codecs=\"mp4a.40.2\"') })"), "QuotaExceededError");
    assert_eq!(js.val("[].slice.call(ms.sourceBuffers).length + ',' + Array.from(ms.sourceBuffers).length"), "2,2");
}

#[test]
fn qm1k1d_remove_source_buffer() {
    let js = attached();
    js.run(
        "var a = ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"'); var b = ms.addSourceBuffer('audio/mp4; codecs=\"mp4a.40.2\"');\
         ms.sourceBuffers.onremovesourcebuffer = function () { console.log('removed', ms.sourceBuffers.length); };",
    );
    js.log.borrow_mut().clear();
    assert_eq!(js.val("errName(function () { ms.removeSourceBuffer({}) })"), "TypeError");
    js.run("ms.removeSourceBuffer(a);");
    assert_eq!(js.val("errName(function () { ms.removeSourceBuffer(a) })"), "NotFoundError");
    assert_eq!(js.val("[ms.sourceBuffers.length, ms.sourceBuffers[0] === b, ms.sourceBuffers[1], ms.activeSourceBuffers.length].join()"), "1,true,,1");
    assert_eq!(js.lines(), ["log:removed 1"]);
    // the slot is free again
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"') })"), "ok");
}

#[test]
fn qm1k1d_duration_rules() {
    let js = Js::new(HTML, SETUP);
    assert_eq!(js.val("errName(function () { ms.duration = 5 })"), "InvalidStateError");
    js.step(100.0);
    assert_eq!(js.val("String(ms.duration) + ',' + String(v.duration)"), "NaN,NaN");
    assert_eq!(js.val("errName(function () { ms.duration = NaN })"), "TypeError");
    assert_eq!(js.val("errName(function () { ms.duration = -1 })"), "TypeError");
    js.log.borrow_mut().clear();
    js.run("ms.duration = 42;");
    assert_eq!(js.val("ms.duration + ',' + v.duration"), "42,42");
    assert_eq!(js.lines(), ["log:durationchange"]);
    js.run("ms.duration = 42;");
    assert_eq!(js.lines(), ["log:durationchange"], "same value: no second event");
}

#[test]
fn qm1k1d_end_of_stream_states_and_events() {
    let js = attached();
    js.log.borrow_mut().clear();
    assert_eq!(js.val("errName(function () { ms.endOfStream('bogus') })"), "TypeError");
    js.run("ms.endOfStream();");
    assert_eq!(js.val("ms.readyState"), "ended");
    assert_eq!(js.lines(), ["log:sourceended"]);
    assert_eq!(js.val("errName(function () { ms.endOfStream() })"), "InvalidStateError");
    assert_eq!(js.val("errName(function () { ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"') })"), "InvalidStateError");
    assert_eq!(js.val("errName(function () { ms.duration = 3 })"), "InvalidStateError");
}

#[test]
fn qm1k1d_end_of_stream_with_error_sets_media_error() {
    let js = attached();
    js.run("ms.endOfStream('network');");
    assert_eq!(js.val("[ms.readyState, v.error && v.error.code].join()"), "ended,2");
    assert!(js.lines().contains(&"log:sourceended".to_string()) && js.lines().contains(&"log:error".to_string()), "{:?}", js.lines());
    let js = attached();
    js.run("ms.endOfStream('decode');");
    assert_eq!(js.val("v.error.code"), "3");
    let js = attached();
    js.run("ms.endOfStream('');");
    assert_eq!(js.val("String(v.error)"), "null");
}

#[test]
fn qm1k1d_live_seekable_range_rules() {
    let js = Js::new(HTML, SETUP);
    assert_eq!(js.val("errName(function () { ms.setLiveSeekableRange(0, 1) })"), "InvalidStateError");
    assert_eq!(js.val("errName(function () { ms.clearLiveSeekableRange() })"), "InvalidStateError");
    js.step(100.0);
    assert_eq!(js.val("errName(function () { ms.setLiveSeekableRange(0, 10); ms.clearLiveSeekableRange(); })"), "ok");
    assert_eq!(js.val("errName(function () { ms.setLiveSeekableRange(5, 1) })"), "TypeError");
    assert_eq!(js.val("errName(function () { ms.setLiveSeekableRange(-1, 1) })"), "TypeError");
}

#[test]
fn qm1k1d_revoke_before_attach_is_a_load_error() {
    let js = Js::new(HTML, "var v = document.getElementById('v'); var ms = new MediaSource(); var u = URL.createObjectURL(ms); URL.revokeObjectURL(u);\
         v.addEventListener('error', function () { console.log('error', v.error.code); }); ms.addEventListener('sourceopen', function () { console.log('sourceopen'); }); v.src = u;");
    js.step(100.0);
    assert_eq!(js.lines(), ["log:error 4"]);
    assert_eq!(js.val("ms.readyState"), "closed");
    assert_eq!(js.feeds(), 0);
}

#[test]
fn qm1k1d_revoke_after_attach_keeps_it_attached() {
    let js = attached();
    js.run("URL.revokeObjectURL(url);");
    js.step(200.0);
    assert_eq!(js.val("ms.readyState"), "open");
    assert_eq!(js.lines(), ["log:loadstart", "log:sourceopen"]);
    assert_eq!(js.feeds(), 1);
}

#[test]
fn qm1k1d_new_src_closes_the_old_media_source() {
    let js = attached();
    js.run("var a = ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"'); ms.sourceBuffers.addEventListener('removesourcebuffer', function () { console.log('rm'); });");
    js.log.borrow_mut().clear();
    js.run(
        "var ms2 = new MediaSource(); ms2.addEventListener('sourceopen', function () { console.log('open2'); });\
         v.src = URL.createObjectURL(ms2);",
    );
    js.step(100.0);
    let l = js.lines();
    let pos = |s: &str| l.iter().position(|x| x == s).unwrap_or_else(|| panic!("{s} in {l:?}"));
    assert!(pos("log:sourceclose") < pos("log:open2"), "{l:?}");
    assert_eq!(js.val("[ms.readyState, String(ms.duration), ms.sourceBuffers.length, ms2.readyState].join()"), "closed,NaN,0,open");
    assert!(l.contains(&"log:rm".to_string()), "{l:?}");
    // the old feed is gone, the new one lives
    assert_eq!(js.feeds(), 1);
    // a closed MediaSource can be attached again
    js.run("v.srcObject = ms;");
    js.step(100.0);
    assert_eq!(js.val("[ms.readyState, ms2.readyState].join()"), "open,closed");
}

#[test]
fn qm1k1d_src_object_null_detaches() {
    let js = Js::new(HTML, "var v = document.getElementById('v'); var ms = new MediaSource(); ms.addEventListener('sourceclose', function () { console.log('close'); }); v.srcObject = ms;");
    js.step(100.0);
    assert_eq!(js.val("ms.readyState"), "open");
    js.run("v.srcObject = null;");
    assert_eq!(js.lines(), ["log:close"]);
    assert_eq!(js.val("[ms.readyState, v.srcObject === null].join()"), "closed,true");
    assert_eq!(js.feeds(), 0);
}

#[test]
fn qm1k1d_second_element_cannot_take_an_attached_source() {
    let js = Js::new(
        "<video id=v muted></video><video id=w muted></video>",
        "var ms = new MediaSource(); var a = document.getElementById('v'), b = document.getElementById('w'); a.srcObject = ms;",
    );
    js.step(100.0);
    assert_eq!(js.val("ms.readyState"), "open");
    assert_eq!(js.feeds(), 1);
}

#[test]
fn qm1k1d_event_handlers_and_to_string_tag() {
    let js = Js::new(
        HTML,
        "var v = document.getElementById('v'); var ms = new MediaSource(); ms.onsourceopen = function () { console.log('h-open'); }; ms.onsourceended = function () { console.log('h-ended'); };\
         console.log(ms.onsourceclose === null, typeof ms.onsourceopen, Object.prototype.toString.call(ms)); v.src = URL.createObjectURL(ms);",
    );
    js.step(100.0);
    js.run("ms.endOfStream();");
    assert_eq!(js.lines(), ["log:true function [object MediaSource]", "log:h-open", "log:h-ended"]);
}

#[test]
fn qm1k1d_a_page_without_media_source_pays_nothing() {
    let js = Js::new("<video id=v src='/a.mp4' muted></video>", "var v = document.getElementById('v'); console.log(typeof MediaSource);");
    js.step(100.0);
    assert_eq!(js.lines(), ["log:function"]);
    assert_eq!(js.feeds(), 0);
    // `new MediaSource()` alone creates no feed either (the feed appears at attach)
    js.run("var m = new MediaSource(); m.sourceBuffers; m.activeSourceBuffers;");
    assert_eq!(js.feeds(), 0);
    // fetch of a MediaSource object URL fails cleanly instead of throwing
    js.run("var u = URL.createObjectURL(m); fetch(u).then(function () { console.log('ok'); }, function (e) { console.log('rejected', e.name); });");
    assert_eq!(js.lines().last().unwrap(), "log:rejected TypeError");
}

#[test]
fn qm1k1d_without_a_backend_is_type_supported_is_false() {
    set_backend_factory(Some(Rc::new(|_| Box::new(UnavailableBackend))));
    set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
    let engine = crate::js::start_scripts_clocked(&parse_html(HTML), &[src("")], Rc::new(|_, _| {}), None).unwrap();
    set_backend_factory(None);
    let r = engine.eval_value::<String>("t", "String(MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.64001f\"'))").unwrap();
    assert_eq!(r, "false");
}
