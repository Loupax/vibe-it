// QINC1i (R60 sections 3, 9): script forced layouts through the incremental engine. The js_geometry
// and observer suites run a second time with the engine (`*_incr_forced_tests`); these tests cover
// what only the engine path does. Every engine update here is also checked against a from-scratch
// layout (tests run in check mode: a difference panics).
#[allow(unused_imports)]
use super::*;
use crate::perf::{incr_get, Incr};

/// Runs `code` in `html` with forced layouts through the engine (`on`) or from scratch; returns the
/// log and (forced layouts served by the engine, of them restarts from a from-scratch build).
fn forced_run(html: &str, code: &str, on: bool) -> (Vec<String>, u64, u64) {
    let was = crate::incr_engine::set_forced_seam(Some(on));
    let (e0, r0) = (incr_get(Incr::ForcedEngine), incr_get(Incr::ForcedRestarts));
    let log = run_in_html(html, code);
    let n = (incr_get(Incr::ForcedEngine) - e0, incr_get(Incr::ForcedRestarts) - r0);
    crate::incr_engine::set_forced_seam(was);
    (log, n.0, n.1)
}

const PAGE: &str = "<html><head><style>.big{height:50px} #box{position:absolute;left:5px;top:7px;width:20px;height:9px} \
     .hide{display:none}</style></head><body><div id=a style='height:10px'></div><p id=p>some text here</p>\
     <div id=h class=hide><span id=hs>x</span></div><div id=box></div><div id=b>after</div></body></html>";

const SCRIPT: &str = "var r = function (id) { var b = document.getElementById(id).getBoundingClientRect(); return [b.x, b.y, b.width, b.height].join(','); };\
     var out = [];\
     for (var i = 0; i < 4; i++) {\
       document.getElementById('a').className = i % 2 ? 'big' : '';\
       out.push(r('b'));\
       document.getElementById('box').style.left = (10 + i) + 'px';\
       out.push(r('box'));\
       document.getElementById('hs').textContent = 'y' + i;\
       out.push(r('b'));\
       document.getElementById('p').appendChild(document.createTextNode(' more'));\
       out.push(r('p'), r('b'));\
     }\
     var rg = document.createRange(); rg.selectNodeContents(document.getElementById('p'));\
     var cr = rg.getBoundingClientRect(); out.push('range ' + [cr.width > 0, cr.height > 0].join(','));\
     console.log(out.join(' | '));";

#[test]
fn qinc1i_forced_layouts_use_the_engine_and_match_from_scratch() {
    let (off, n_off, _) = forced_run(PAGE, SCRIPT, false);
    assert_eq!(n_off, 0, "flag off: no engine forced layout");
    let (on, n_on, restarts) = forced_run(PAGE, SCRIPT, true);
    assert_eq!(on, off, "the engine's geometry is the from-scratch geometry");
    // 4 x (class, style, hidden text, text) writes, each followed by a read (the hidden write too: the
    // HiddenSnap skip is bypassed), plus the range.
    assert!(n_on >= 16, "every read after a write went through the engine: {n_on}");
    assert_eq!(restarts, 1, "only the first forced layout builds from scratch");
}

#[test]
fn qinc1i_special_cases_are_bypassed_with_the_engine() {
    // An offset-only `style` write on an abs box (QYTC1a), a write inside a hidden subtree (QY53l2) and
    // a class write no rule reads (QYTC1d), each followed by a read: from scratch they force no layout
    // (special cases); with the engine every read updates the engine (cheaply) instead.
    let code = "var b = document.getElementById('box'); b.setAttribute('style', 'left:6px'); b.getBoundingClientRect();\
         b.setAttribute('style', 'left:30px'); b.getBoundingClientRect();\
         document.getElementById('hs').setAttribute('data-x', '1'); b.getBoundingClientRect();\
         b.classList.add('nothing-matches');\
         console.log(b.getBoundingClientRect().x);";
    let layouts = |on: bool| {
        crate::js_geom::GEOM_LAYOUTS.with(|c| c.set(0));
        let (log, ..) = forced_run(PAGE, code, on);
        (log, crate::js_geom::GEOM_LAYOUTS.with(|c| c.get()))
    };
    let (off, n_off) = layouts(false);
    let (on, n_on) = layouts(true);
    assert_eq!(off, vec!["log:30".to_string()]);
    assert_eq!(on, off);
    assert!(n_off < 4, "flag off: the special cases skip layouts ({n_off})");
    assert_eq!(n_on, 4, "with the engine every read after a write updates");
}

#[test]
fn qinc1i_forced_layouts_see_the_windows_images() {
    // R60 open question 2 (user: yes): with the engine, script geometry lays images out at their decoded
    // size, from the window's image cache (the App hands it over at each build).
    let html = "<html><body><img id=i src='pic.png'><div id=after>x</div></body></html>";
    let code = "var i = document.getElementById('i').getBoundingClientRect(); var a = document.getElementById('after').getBoundingClientRect();\
         console.log([i.width, i.height, a.y].join(','));";
    let run = |on: bool, with_image: bool| {
        let was = crate::incr_engine::set_forced_seam(Some(on));
        let doc = parse_html(html);
        if with_image {
            let mut images = HashMap::new();
            images.insert(
                "pic.png".to_string(),
                Arc::new(DecodedImage { width: 40, height: 30, pixels: vec![TextColor::rgb(1, 2, 3); 1200], svg: None, mask: None }),
            );
            crate::incr_engine::sync_window_images(&doc, &images);
        }
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        run_scripts_in(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
        crate::incr_engine::set_forced_seam(was);
        out.take()
    };
    let on = run(true, true);
    assert_eq!(on.len(), 1);
    let v: Vec<f32> = on[0].trim_start_matches("log:").split(',').map(|x| x.parse().unwrap()).collect();
    assert_eq!((v[0], v[1]), (40.0, 30.0), "the decoded size: {on:?}");
    assert!(v[2] >= 30.0, "content after the image moved below it: {on:?}");
    // Flag off: the window's images are not handed over (behaviour unchanged).
    let off = run(false, true);
    let w: Vec<f32> = off[0].trim_start_matches("log:").split(',').map(|x| x.parse().unwrap()).collect();
    assert_ne!((w[0], w[1]), (40.0, 30.0), "flag off: no decoded size: {off:?}");
    // Another document never sees them.
    let other = run(true, false);
    assert_eq!(other, off, "images are per document");
}

#[test]
fn qinc1i_intersection_observer_reads_the_engine() {
    // The kept App layout is not used with the engine (the observer's reads update the engine instead).
    let was = crate::incr_engine::set_forced_seam(Some(true));
    let doc = parse_html("<html><body><div id=a style='height:10px'></div></body></html>");
    let laid = measure_document_fixture("<html><body><div style='height:10px'></div></body></html>", 1024.0, 768.0);
    crate::js_geom::set_rendered(&laid, crate::js_geom::render_stamp(), 1024, 768);
    assert!(!crate::js_geom::rendered_ok(1024, 768), "the App layout is not an IO rect source with the engine");
    crate::incr_engine::set_forced_seam(Some(false));
    assert!(crate::js_geom::rendered_ok(1024, 768), "flag off: unchanged");
    crate::js_geom::clear_rendered();
    crate::incr_engine::set_forced_seam(was);
    drop(doc);
}

#[test]
fn qinc1i_text_rects_after_engine_updates() {
    // Lazy glyphs: collected from the kept laid tree on the first Range read after an update.
    let code = "var p = document.getElementById('p'); var t = p.firstChild; var out = [];\
         for (var i = 0; i < 3; i++) {\
           document.getElementById('a').style.height = (10 + 20 * i) + 'px';\
           var rg = document.createRange(); rg.setStart(t, 0); rg.setEnd(t, 4);\
           var b = rg.getBoundingClientRect(); out.push([Math.round(b.y), b.width > 0].join(':'));\
         }\
         console.log(out.join(' '));";
    let (off, ..) = forced_run(PAGE, code, false);
    let (on, n, _) = forced_run(PAGE, code, true);
    assert_eq!(on, off);
    assert!(n >= 3);
}

#[test]
fn qinc1i5_engine_forced_layout_keeps_the_shape_memo() {
    // QINC1i5: an engine forced layout re-measures only what changed; its memo sweep used to keep only
    // the keys IT used, so a forced layout after a tiny write emptied the main thread's shape memo and
    // the next App frame relayout (Wikipedia: 138k boxes re-measured, walks to the root) shaped every
    // paragraph again (layout_ms 1.2 s -> 5.8 s). The oracle is off here: its from-scratch layout
    // would refill the memo itself.
    let paras: String = (0..30).map(|i| format!("<p>paragraph number {i} with some words to shape</p>")).collect();
    let html = format!(
        "<html><head><style>#box{{position:absolute;left:5px;top:7px;width:20px;height:9px}}</style></head>\
         <body><div id=a style='height:10px'></div>{paras}<div id=box></div><div id=b>after</div></body></html>"
    );
    let code = "var b = document.getElementById('b').getBoundingClientRect().y;\
         for (var i = 0; i < 3; i++) { document.getElementById('box').style.left = (10 + i) + 'px';\
           b += document.getElementById('box').getBoundingClientRect().x; }\
         console.log(b > 0);";
    crate::js_geom::NO_ORACLE.with(|c| c.set(true));
    crate::layout::memo_clear();
    let (log, n, restarts) = forced_run(&html, code, true);
    crate::js_geom::NO_ORACLE.with(|c| c.set(false));
    assert_eq!(log, vec!["log:true".to_string()]);
    assert_eq!(restarts, 1, "the first read builds from scratch");
    assert!(n >= 4, "the box writes were read through the engine: {n}");
    let kept = crate::layout::memo_len();
    assert!(kept >= 30, "the paragraphs' shapes stay in the memo after small engine updates: {kept}");
}
