//! QINC1j (R60 sections 3, 9): App frames through the incremental engine (the only App frame path
//! since QINC1m). Every Main frame is checked against a from-scratch build and layout (check mode is
//! always on in tests: a difference panics), so these tests pin WHICH path a frame takes and what
//! the App then shows.
use super::*;
use crate::perf::{incr_get, Incr};

pub(super) fn find_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| &*v == id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_id(c, id))
}

const W: u32 = 600;
const H: u32 = 400;

/// The shared test loop's proxy (`test_event_loop`); `None` without a display.
fn proxy() -> Option<EventLoopProxy<AppEvent>> {
    test_event_loop().ok().map(|l| l.create_proxy())
}

struct Live;

impl Drop for Live {
    fn drop(&mut self) {
        crate::anim_eval::set_app_clock_override(None);
        set_interaction(&InteractionState::default());
    }
}

/// An App on `html` after its first frame. `None` without a display.
fn live_app(html: &str) -> Option<(App, Live)> {
    let proxy = proxy()?;
    let live = Live;
    crate::anim_eval::set_app_clock_override(Some(0.0));
    set_interaction(&InteractionState::default());
    let mut app = App::new_static(parse_html(html), proxy, None);
    let before = counts();
    app.rebuild_layout(W, H, false);
    assert_eq!(counts().2 - before.2, 1, "the first layout goes to the worker");
    assert!(app.live_ver != 0, "the engine adopted the worker's layout");
    assert!(app.cached.as_ref().is_some_and(crate::incr_layout::owns), "the App shows the engine's tree");
    Some((app, live))
}

/// (current, main, worker) frames so far.
fn counts() -> (u64, u64, u64) {
    (incr_get(Incr::FramesCurrent), incr_get(Incr::FramesMain), incr_get(Incr::FramesWorker))
}

/// The next frame; returns which path it took (counter deltas).
fn frame(app: &mut App) -> (u64, u64, u64) {
    frame_at(app, W, H)
}

fn frame_at(app: &mut App, w: u32, h: u32) -> (u64, u64, u64) {
    let b = counts();
    let height_rebuild = app.cached_width == w && app.cached_height != h && app.cached.is_some();
    app.rebuild_layout(w, h, height_rebuild);
    let a = counts();
    assert!(app.cached.is_some(), "a frame always leaves a tree to paint");
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn find_laid(l: &Laid, node: usize) -> Option<&Laid> {
    if l.style.node == node && l.style.pseudo == 0 {
        return Some(l);
    }
    let kids: Vec<&Laid> = match &l.content {
        LaidContent::Blocks(v) | LaidContent::TableRow(v) => v.iter().collect(),
        LaidContent::MixedInline(items) => items.iter().filter_map(|i| if let MixedPositioned::Embedded(e) = i { Some(&**e) } else { None }).collect(),
        _ => Vec::new(),
    };
    kids.into_iter().chain(l.positioned.iter()).find_map(|k| find_laid(k, node))
}

fn shown<'a>(app: &'a App, id: &str) -> &'a Laid {
    let n = node_id(&find_id(&app.document, id).expect("id"));
    find_laid(app.cached.as_ref().expect("laid"), n).expect("box of the element")
}

fn hover(app: &mut App, id: &str) {
    let n = node_id(&find_id(&app.document, id).expect("id"));
    let mut st = app.interaction.clone();
    st.hover = app.chain_for(n);
    app.set_interaction_state(st);
}

const MAIN: (u64, u64, u64) = (0, 1, 0);
const CURRENT: (u64, u64, u64) = (1, 0, 0);
const WORKER: (u64, u64, u64) = (0, 0, 1);

/// A paint-only hover (background colour) and a layout hover (height) are incremental main-thread
/// frames (no hover patch), shown at once; leaving restores the old boxes.
#[test]
fn qinc1j_hover_paint_and_layout_frames_run_on_main() {
    let html = "<html><head><style>body{margin:0}#a{height:20px}#a:hover{background-color:red}\
                #b{height:20px}#b:hover{height:60px}</style></head><body><div id=a>a</div><div id=b>b</div><p id=c>c</p></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let y_c = shown(&app, "c").y;
    hover(&mut app, "a");
    assert!(app.layout_dirty, "a :hover rule: the frame restyles");
    assert_eq!(frame(&mut app), MAIN);
    assert!(shown(&app, "a").style.background_color.is_some(), "paint-only hover shown");
    assert_eq!(shown(&app, "c").y, y_c);
    hover(&mut app, "b");
    assert_eq!(frame(&mut app), MAIN);
    assert!(shown(&app, "a").style.background_color.is_none(), "left #a");
    assert_eq!(shown(&app, "b").h, 60.0, "layout hover shown");
    assert_eq!(shown(&app, "c").y, y_c + 40.0, "the following box moved");
    assert!(!app.hold_old_frame && !app.layout_in_flight, "installed at once");
    // Nothing changed since: the next frame does nothing.
    assert_eq!(frame(&mut app), CURRENT);
}

/// QINC1i6: with no script forced layout ever, the main thread's shape memo is on for engine
/// relayouts: re-measuring a paragraph at a width it was already shaped at is a memo hit.
#[test]
fn qinc1i6_main_frames_use_the_shape_memo_without_forced_layouts() {
    let html = "<html><head><style>body{margin:0}#b{width:600px}#b:hover{width:300px}</style></head>\
                <body><div id=b><p>Some paragraph text that needs shaping to measure.</p></div><p id=c>c</p></body></html>";
    crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    crate::layout::memo_clear();
    let Some((mut app, _l)) = live_app(html) else { return };
    hover(&mut app, "b");
    assert_eq!(frame(&mut app), MAIN);
    assert!(crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.get()), "the first Main frame turns the memo on");
    hover(&mut app, "c");
    assert_eq!(frame(&mut app), MAIN);
    let (h0, _) = crate::layout::shape_memo_lookups();
    hover(&mut app, "b");
    assert_eq!(frame(&mut app), MAIN);
    let (h1, _) = crate::layout::shape_memo_lookups();
    assert!(h1 > h0, "the 300px shape of the paragraph came from the memo ({h0} -> {h1})");
    crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    crate::layout::memo_clear();
}

/// Focus is an interaction-state input like hover.
#[test]
fn qinc1j_focus_frame() {
    let html = "<html><head><style>body{margin:0}#f{height:10px}#f:focus{height:70px}</style></head>\
                <body><div id=f tabindex=0>f</div><p id=after>x</p></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let f = find_id(&app.document, "f").expect("f");
    app.set_focus_element(Some(&f), true);
    assert_eq!(frame(&mut app), MAIN);
    assert_eq!(shown(&app, "f").h, 70.0);
    app.set_focus_element(None, false);
    assert_eq!(frame(&mut app), MAIN);
    assert_eq!(shown(&app, "f").h, 10.0);
}

/// Animation frames: each tick re-cascades the animated element on the main thread.
#[test]
fn qinc1j_animation_frames() {
    let html = "<html><head><style>body{margin:0}@keyframes k{from{height:10px}to{height:110px}}\
                #an{animation:k 1s linear}</style></head><body><div id=an>a</div><p id=p>p</p></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    assert!((shown(&app, "an").h - 10.0).abs() < 0.5);
    for (ms, h) in [(250.0, 35.0), (500.0, 60.0), (750.0, 85.0)] {
        crate::anim_eval::set_app_clock_override(Some(ms));
        app.anim_tick(ms);
        assert_eq!(frame(&mut app), MAIN, "tick at {ms} ms");
        let got = shown(&app, "an").h;
        assert!((got - h).abs() < 0.5, "height at {ms} ms: {got}");
    }
}

/// An image arriving in the App's cache rebuilds only the boxes that read it (a main frame).
#[test]
fn qinc1j_image_arrival_frame() {
    let html = "<html><head><style>body{margin:0}</style></head><body><p>text</p><img id=i src=\"http://example.test/a.png\"></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let img = DecodedImage { width: 40, height: 30, pixels: vec![TextColor::rgb(1, 2, 3); 1200], svg: None, mask: None };
    app.image_cache.insert("http://example.test/a.png".into(), Arc::new(img));
    let b = incr_get(Incr::BoxesRebuilt);
    assert_eq!(frame(&mut app), MAIN);
    assert!(incr_get(Incr::BoxesRebuilt) > b);
    let root = app.cached.as_ref().expect("laid");
    let mut found = false;
    fn has_image(l: &Laid, found: &mut bool) {
        match &l.content {
            LaidContent::Image(i) if i.width == 40 => *found = true,
            LaidContent::Blocks(v) | LaidContent::TableRow(v) => v.iter().for_each(|c| has_image(c, found)),
            LaidContent::MixedInline(items) => items.iter().for_each(|i| {
                if let MixedPositioned::Embedded(e) = i {
                    has_image(e, found)
                }
            }),
            _ => {}
        }
    }
    has_image(root, &mut found);
    assert!(found, "the decoded image is laid out");
}

/// QINC1k6: a forced layout in the image's `load` handler already sees the decoded size (the
/// engine's window images are updated before `load` fires, not by the next frame).
#[test]
fn qinc1k6_load_handler_forced_layout_sees_the_decoded_image() {
    let html = "<html><head><style>body{margin:0}</style></head><body><p>text</p><img id=i src=\"http://example.test/a.png\"></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let doc = app.document.clone();
    app.js = run_page_scripts(
        &doc,
        &[src("var i = document.getElementById('i'); window.seen = 'none'; i.addEventListener('load', function () { var b = i.getBoundingClientRect(); window.seen = b.width + 'x' + b.height; });")],
        None,
    );
    let Some(js) = app.js.as_ref() else { return };
    js.set_viewport(W, H);
    let before = js.eval_value::<String>("t", "var b = document.getElementById('i').getBoundingClientRect(); b.width + 'x' + b.height");
    assert_ne!(before.as_deref(), Some("40x30"), "placeholder size before the image arrives");
    let img = DecodedImage { width: 40, height: 30, pixels: vec![TextColor::rgb(1, 2, 3); 1200], svg: None, mask: None };
    let generation = app.nav_generation;
    app.handle_image_fetched(generation, "http://example.test/a.png".into(), Some(img));
    let seen = app.js.as_ref().expect("js").eval_value::<String>("t", "window.seen");
    assert_eq!(seen.as_deref(), Some("40x30"), "load handler's forced layout sees the decoded size");
}

/// A width change goes to the worker (from scratch); a height change is a main frame (only `vh`
/// readers and the viewport-dependent boxes change).
#[test]
fn qinc1j_resize_frames() {
    let html = "<html><head><style>body{margin:0}#v{height:50vh}#w{width:50%;height:5px}</style></head>\
                <body><div id=v>v</div><div id=w></div></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    assert_eq!(shown(&app, "v").h, H as f32 / 2.0);
    assert_eq!(frame_at(&mut app, W, 300), MAIN, "height only");
    assert_eq!(shown(&app, "v").h, 150.0);
    assert_eq!(frame_at(&mut app, 500, 300), WORKER, "width");
    assert_eq!(shown(&app, "w").w, 250.0);
    assert!(app.live_ver != 0, "adopted");
    assert_eq!(frame_at(&mut app, 500, 300), CURRENT);
    hover(&mut app, "v");
    assert_eq!(frame_at(&mut app, 500, 300), MAIN, "incremental again after the adoption");
}

/// A stylesheet arriving later (a new rule-list generation) goes to the worker; the engine adopts
/// the result and later frames are incremental again.
#[test]
fn qinc1j_link_css_arrival_frame() {
    let html = "<html><head><style>body{margin:0}#a:hover{height:33px}</style></head><body><div id=a>a</div><div id=b>b</div></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    app.extra_css.push_str("#b{height:44px}");
    app.layout_dirty = true;
    assert_eq!(frame(&mut app), WORKER);
    assert_eq!(shown(&app, "b").h, 44.0);
    hover(&mut app, "a");
    assert_eq!(frame(&mut app), MAIN);
    assert_eq!(shown(&app, "a").h, 33.0);
}

/// A script write, then a forced layout (a geometry read), then the frame: the forced layout
/// already laid the page out, so the frame does no style, box or layout work.
#[test]
fn qinc1j_js_write_forced_read_then_frame_does_no_layout() {
    let html = "<html><head><style>body{margin:0}#a{height:10px}</style></head><body><div id=a>a</div><p id=p>p</p></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let doc = app.document.clone();
    app.js = run_page_scripts(&doc, &[src("window.ok = 1;")], None);
    let Some(js) = app.js.as_ref() else { return };
    js.set_viewport(W, H);
    let h = js.eval_value::<f64>("t", "var a = document.getElementById('a'); a.style.height = '77px'; a.offsetHeight");
    assert_eq!(h, Some(77.0));
    let (lu, bu, up) = (incr_get(Incr::LayoutUpdates), incr_get(Incr::BoxUpdates), incr_get(Incr::Updates));
    app.layout_dirty = true;
    assert_eq!(frame(&mut app), CURRENT, "the forced layout did the work");
    assert_eq!((incr_get(Incr::LayoutUpdates), incr_get(Incr::BoxUpdates), incr_get(Incr::Updates)), (lu, bu, up), "no layout in the frame");
    assert_eq!(shown(&app, "a").h, 77.0, "the frame shows the forced layout's tree");
    // A write without a read: the frame does the work itself.
    app.js.as_ref().expect("js").eval_value::<bool>("t", "document.getElementById('a').style.height = '12px'; true");
    assert_eq!(frame(&mut app), MAIN);
    assert_eq!(shown(&app, "a").h, 12.0);
}

/// A restyle of more than a quarter of the page (and at least `WORKER_MIN_RESTYLED` elements) goes
/// to the worker with a COPY of the kept box tree; the engine adopts the worker's layout of it and
/// the next frames are incremental again.
#[test]
fn qinc1j_big_restyle_goes_to_the_worker_with_the_kept_tree() {
    let items: String = (0..300).map(|i| format!("<div>item {i}</div>")).collect();
    let html = format!(
        "<html><head><style>body{{margin:0}}#box:hover div{{padding-left:3px}}#t:hover{{height:9px}}</style></head>\
         <body><div id=t>t</div><div id=box>{items}</div></body></html>"
    );
    let Some((mut app, _l)) = live_app(&html) else { return };
    hover(&mut app, "box");
    let fb = incr_get(Incr::BoxUpdates);
    assert_eq!(frame(&mut app), WORKER, "restyled share");
    assert!(incr_get(Incr::BoxUpdates) > fb, "the box tree was updated on main, its copy laid out");
    assert!(app.live_ver != 0, "adopted");
    hover(&mut app, "t");
    assert_eq!(frame(&mut app), WORKER, "leaving #box restyles the 300 items again");
    assert_eq!(shown(&app, "t").h, 9.0);
    // A small change after the adoption: incremental, checked against a from-scratch layout.
    let t = node_id(&find_id(&app.document, "t").expect("t"));
    let mut st = app.interaction.clone();
    st.hover.remove(&t);
    app.set_interaction_state(st);
    assert_eq!(frame(&mut app), MAIN);
    assert!(shown(&app, "t").h != 9.0, "back to auto height");
}

/// App-side inputs reach the write log only when they change.
#[test]
fn qinc1j_media_size_change_is_logged_once() {
    use crate::incr_log::{Ambient, Change};
    let _ = crate::incr_log::drain();
    let sizes: HashMap<usize, (u32, u32)> = [(1usize, (320u32, 240u32))].into_iter().collect();
    crate::media_video::set_intrinsic(sizes.clone());
    crate::media_video::set_intrinsic(sizes);
    let n = crate::incr_log::drain().entries.iter().filter(|e| e.change == Change::Ambient(Ambient::MediaSize)).count();
    assert_eq!(n, 1);
    crate::media_video::set_intrinsic(HashMap::new());
    let _ = crate::incr_log::drain();
}

// ---------- QINC1n: engine evidence on App frames ----------

const EV_PAGE: &str = "<html><head><style>body{margin:0}#b{height:20px}#b:hover{height:60px}</style></head>\
                       <body><div id=b>secret words</div><p id=c>c</p></body></html>";

#[test]
fn qinc1n_app_restarts_count_and_fallbacks_log_once() {
    use super::incr_forced_tests::{printed, EvDir};
    let _d = EvDir::new("app-fallback");
    let Some((mut app, _l)) = live_app(EV_PAGE) else { return };
    let c = crate::incr_evidence::counts();
    assert_eq!((c.restarts, c.fallbacks), (1, 0), "the first layout is a restart: {c:?}");
    // A resize is a restart too: counted, no line.
    assert_eq!(frame_at(&mut app, W + 50, H), WORKER);
    assert_eq!(crate::incr_evidence::counts().restarts, 2);
    hover(&mut app, "b");
    assert_eq!(frame_at(&mut app, W + 50, H), MAIN);
    assert_eq!(crate::incr_evidence::counts().updates, 1);
    assert!(printed().is_empty(), "restarts print nothing: {:?}", printed());
    // Forced fallbacks: the frame lays out from scratch; one line for the page, every one counted.
    crate::incr_evidence::with_seam(|s| s.borrow_mut().fallback = true);
    for id in ["c", "b"] {
        hover(&mut app, id);
        assert_eq!(frame_at(&mut app, W + 50, H), WORKER);
    }
    let c = crate::incr_evidence::counts();
    assert_eq!((c.fallbacks, c.restarts), (2, 2), "{c:?}");
    let lines = printed();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("scratchy: engine: fallback test_hook on "), "{lines:?}");
    // `:engine` shows the counts in the status bar.
    app.dispatch_command("engine", &[]);
    assert_eq!(app.status_message, "engine: 2 fallbacks, checks off, 1 upd, 2 rst");
}

#[test]
fn qinc1n_app_sampled_mismatch_shows_the_from_scratch_layout() {
    use super::incr_forced_tests::{printed, EvDir};
    let d = EvDir::new("app-mismatch");
    let Some((mut app, _l)) = live_app(EV_PAGE) else { return };
    crate::incr_evidence::with_seam(|s| {
        let mut s = s.borrow_mut();
        s.every = Some(1);
        s.dir = Some(d.0.clone());
    });
    // A real check that agrees: counted, nothing written.
    hover(&mut app, "b");
    assert_eq!(frame(&mut app), MAIN);
    assert!(app.live_ver != 0, "the engine's tree is shown");
    // The test hook reports a mismatch: one log line, the from-scratch layout is shown.
    crate::incr_evidence::with_seam(|s| s.borrow_mut().mismatch = true);
    hover(&mut app, "c");
    assert_eq!(frame(&mut app), MAIN);
    assert_eq!(app.live_ver, 0, "the from-scratch layout is shown");
    assert_eq!(shown(&app, "b").h, 20.0, "left #b");
    let c = crate::incr_evidence::counts();
    assert_eq!((c.checks, c.mismatches), (2, 1), "{c:?}");
    let lines = d.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("\troot test hook") && !lines[0].contains("secret"), "{lines:?}");
    assert_eq!(printed().len(), 1, "{:?}", printed());
    // The engine starts over; the next frame is a restart.
    crate::incr_evidence::with_seam(|s| s.borrow_mut().mismatch = false);
    hover(&mut app, "b");
    frame(&mut app);
    assert_eq!(crate::incr_evidence::counts().restarts, 2);
}
