#[allow(unused_imports)]
use super::*;

// ---------- QSTK1: deeply nested documents must not overflow the stack ----------
//
// These run on the default 2 MB test thread on purpose (no `stack_size`): the recursive walks
// grow their own stack (`crate::stack::grow`), so the thread's own size must not matter.

const QSTK1_DEPTH: usize = 5000;

/// `depth` nested divs (3 classes each, a `::before` every 10th level, a little text every
/// 100th) around a `<span id=leaf>`; `.a` adds 1px of left margin per level.
fn qstk1_deep_html(depth: usize) -> String {
    let mut s = String::from(
        "<!doctype html><html><head><style>body{margin:0}.a{margin-left:1px}.b::before{content:'x'}</style></head><body>",
    );
    for i in 0..depth {
        let b = if i % 10 == 0 { " b" } else { "" };
        s.push_str(&format!("<div class=\"a c{} d{}{b}\">", i % 7, i % 13));
        if i % 100 == 0 {
            s.push_str("t ");
        }
    }
    s.push_str("<span id=leaf>leaf</span>");
    for _ in 0..depth {
        s.push_str("</div>");
    }
    s.push_str("</body></html>");
    s
}

#[test]
fn qstk1_5000_deep_document_builds_measures_and_paints() {
    let html = qstk1_deep_html(QSTK1_DEPTH);
    let laid = measure_document_fixture(&html, 800.0, 600.0);
    assert!(laid.h > 0.0, "the deep text gives the document a height");
    let (pixels, _, _) = paint_fixture(&html);
    assert_eq!(pixels.len(), 200 * 200);
    let (laid, cache) = measure_fixture_with_cache(&html, 800.0);
    // Hit testing walks the whole depth too (a point in the leaf's line, far right of every div).
    let _ = hit_test_cursor(&laid, QSTK1_DEPTH as f32 + 2.0, laid.h - 2.0, &cache, &HashMap::new(), 0.0, 600.0);
}

#[test]
fn qstk1_5000_deep_document_forced_layout_from_js() {
    let html = qstk1_deep_html(QSTK1_DEPTH);
    let code =
        "var leaf = document.getElementById('leaf'), r = [];
         // Forced layout through the geometry native itself: `offsetWidth` & co. also walk
         // `getComputedStyle` up every ancestor (O(depth^2), seconds at this depth; not QSTK1).
         for (var i = 0; i < 3; i++) {
           leaf.setAttribute('data-x', String(i));
           r.push(__d_rect(leaf.__id, 800, 600).length > 0);
         }
         r.push(Math.round(__d_rect(leaf.__id, 800, 600)[0]));
         r.push(document.querySelectorAll('div').length);
         r.push(document.body.textContent.length > 0);
         r.push(document.body.innerHTML.length > 0);
         r.push(document.body.cloneNode(true).querySelectorAll('div').length);
         r.push(leaf.closest('.d0') !== null);
         globalThis.out = r.join();";
    let doc = parse_html(&html);
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let engine =
        start_scripts(&doc, &[src(code)], Rc::new(move |level, text| l.borrow_mut().push(format!("{level}:{text}"))))
            .unwrap();
    assert_eq!(
        engine.eval_value::<String>("t", "out").unwrap_or_else(|| panic!("script failed: {:?}", log.borrow())),
        format!("true,true,true,{QSTK1_DEPTH},{QSTK1_DEPTH},true,true,{QSTK1_DEPTH},true")
    );
}

// ---------- QSTK1b: geometry reads on a deep document are linear, not O(depth^2) ----------

/// Boots `qstk1_deep_html(depth)`; returns the leaf's rect + offsets after one write and the
/// ancestor-chain steps the computed-style walks took for that one read.
fn qstk1b_run(depth: usize) -> (String, u64) {
    let doc = parse_html(&qstk1_deep_html(depth));
    let code = "Object.defineProperty(window, 'innerWidth', { value: 800, configurable: true });
         Object.defineProperty(window, 'innerHeight', { value: 600, configurable: true });
         globalThis.read = function () {
           var leaf = document.getElementById('leaf'); leaf.setAttribute('data-x', '1');
           var r = leaf.getBoundingClientRect();
           globalThis.out = [r.x, r.y, leaf.offsetLeft].join();
         };";
    let engine = start_scripts(&doc, &[src(code)], Rc::new(|_, _| {})).unwrap();
    let before = crate::js_style::chain_steps();
    engine.eval("t", "read()");
    let out = engine.eval_value::<String>("t", "out").expect("no out");
    (out, crate::js_style::chain_steps() - before)
}

#[test]
fn qstk1b_deep_geometry_read_same_rect_and_linear_chain_work() {
    let (small, steps_small) = qstk1b_run(1000);
    let (big, steps_big) = qstk1b_run(4000);
    // The leaf sits `depth` px right of the body (1px of margin per level).
    assert!(small.starts_with("1000,"), "{small}");
    assert!(big.starts_with("4000,"), "{big}");
    // One read visits each ancestor a small constant number of times. Quadratic code needs
    // ~depth^2 steps (16 000 000 at 4000 deep); the bounds leave room for a few walks only.
    assert!(steps_small <= 20 * 1000, "1000 deep: {steps_small} chain steps");
    assert!(steps_big <= 20 * 4000, "4000 deep: {steps_big} chain steps");
}

#[test]
fn qstk1b_display_none_ancestor_still_hides_the_target_deep() {
    let code = "Object.defineProperty(window, 'innerWidth', { value: 800, configurable: true });
         var leaf = document.getElementById('leaf'), mid = document.querySelectorAll('div')[2000], r = [];
         r.push(leaf.getBoundingClientRect().x);
         mid.style.display = 'none';
         var b = leaf.getBoundingClientRect();
         r.push(b.x + '|' + b.width + '|' + leaf.offsetLeft);
         mid.style.display = '';
         r.push(leaf.getBoundingClientRect().x);
         globalThis.out = r.join(';');";
    let doc = parse_html(&qstk1_deep_html(3000));
    let engine = start_scripts(&doc, &[src(code)], Rc::new(|_, _| {})).unwrap();
    assert_eq!(engine.eval_value::<String>("t", "out").unwrap(), "3000;0|0|0;3000");
}
