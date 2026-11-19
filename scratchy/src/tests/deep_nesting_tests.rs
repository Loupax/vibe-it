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
