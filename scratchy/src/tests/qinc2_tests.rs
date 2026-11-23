//! QINC2: engine check-mode mismatch "laid at root: h" (a few px per affected row, 1 run in 2-3) on
//! tools/incr-gate/hover-big-js.html. The shaping worker lays the kept tree out in ticks (a deadline per
//! tick) over ONE shape cache keyed by paragraph ids; the last, complete tick is what the engine adopts.
use super::*;

/// The worker's ticks over one cache (each one stops at its deadline, until one ends complete) number the
/// paragraphs like a single complete pass, and the last one lays out the same page. A natural-width cache
/// hit (stored only by a tick that is still complete) used to skip the ids its computation consumes, so an
/// incomplete tick numbered everything after it differently, and the complete tick read other paragraphs'
/// stale entries from the shared cache (a glyphless 21 px line where "engine" is 18 px).
#[test]
fn qinc2_ticks_number_paragraphs_like_a_single_complete_pass() {
    let doc = parse_html(include_str!("../../tools/incr-gate/hover-big-js.html"));
    let tree = crate::images::build_layout_tree_with_resources(&doc, 956.0, Some(700.0), "", &HashMap::new());
    let mut fs = new_font_system();
    let far = std::time::Instant::now() + std::time::Duration::from_secs(600);

    let mut fresh_cache = ShapeCache::new();
    let (mut id3, mut inc3) = (0usize, false);
    let fresh = crate::incr_layout::tick_for_test(&tree, &mut fs, 956.0, 700.0, &mut id3, &mut fresh_cache, far, &mut inc3);
    assert!(!inc3);

    let mut cache = ShapeCache::new();
    let mut ticks = 0;
    let ticked = loop {
        let (mut id, mut inc) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(40);
        let laid = crate::incr_layout::tick_for_test(&tree, &mut fs, 956.0, 700.0, &mut id, &mut cache, deadline, &mut inc);
        ticks += 1;
        assert_eq!(id, id3, "tick {ticks} (incomplete: {inc}) numbers the paragraphs like a single pass");
        if !inc {
            break laid;
        }
        assert!(ticks < 200, "ticks finish");
    };
    assert!(ticks > 1, "the page takes more than one tick ({ticks})");
    if let Err(m) = crate::incr_cmp::cmp_laid(&crate::incr_cmp::Names::default(), "root", &fresh, &fresh_cache, &ticked, &cache) {
        panic!("ticked layout differs from the single pass: {m}");
    }
}
