#[allow(unused_imports)]
use super::*;

#[test]
fn explicit_width_sets_content_box_width() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 100px; padding: 10px;">x</div></body></html>"#,
        500.0,
    );
    let div = find_by_width(&laid, 100.0).expect("div with explicit width not found");
    assert_eq!(div.w, 120.0, "content-box: 100px content + 10px padding on each side");
}

#[test]
fn explicit_width_can_exceed_available_width() {
    let laid = measure_fixture(r#"<html><body><div style="width: 900px;">x</div></body></html>"#, 200.0);
    let div = find_by_width(&laid, 900.0).expect("div with explicit width not found");
    assert_eq!(div.w, 900.0, "an explicit width isn't clamped to the container, same as real CSS");
}

#[test]
fn explicit_width_constrains_children() {
    // The inner div has no `width` of its own to search by — marked
    // with a distinguishing `margin-top` instead (the only nonzero
    // margin anywhere in this fixture).
    let laid = measure_fixture(
        r#"<html><body><div style="width: 150px;"><div style="margin-top: 1px;">inner</div></div></body></html>"#,
        900.0,
    );
    let inner = find_laid_by_margin_top(&laid, 1.0).expect("inner div not found");
    assert_eq!(inner.w, 150.0, "a child with no width of its own should be laid out against its parent's explicit width");
}

#[test]
fn explicit_height_overrides_reported_size_but_children_still_fully_laid_out() {
    let laid = measure_fixture(
        r#"<html><body><div style="height: 10px;"><p>line one</p><p>line two</p></div></body></html>"#,
        400.0,
    );
    let outer = find_by_height(&laid, 10.0).expect("div with explicit height not found");
    assert_eq!(outer.h, 10.0, "the div's own reported height should be exactly the explicit value");
    let LaidContent::Blocks(children) = &outer.content else {
        panic!("expected the div's content to be Blocks (two <p> children)");
    };
    assert_eq!(children.len(), 2, "both paragraphs should still be present, not truncated");
    for p in children {
        assert!(p.h > 0.0, "each paragraph should still be fully laid out with its own real height, not clipped away");
    }
}

#[test]
fn explicit_height_larger_than_content_extends_the_box() {
    let laid = measure_fixture(r#"<html><body><div style="height: 300px;">x</div></body></html>"#, 400.0);
    let div = find_by_height(&laid, 300.0).expect("div with explicit height not found");
    assert_eq!(div.h, 300.0, "explicit height should win over the much-shorter natural content height");
}

// ---------- v43: box-sizing + min/max-width/height ----------

#[test]
fn box_sizing_border_box_shrinks_content_to_fit_padding_and_border() {
    let laid = measure_fixture(
        r#"<html><body><div style="box-sizing: border-box; width: 100px; padding: 10px; border: 5px solid black;">x</div></body></html>"#,
        500.0,
    );
    let div = find_by_width(&laid, 100.0).expect("div with explicit width not found");
    assert_eq!(
        div.w, 100.0,
        "border-box: the specified width is the box's total width, with padding/border carved out of it rather than added on top"
    );
}

#[test]
fn box_sizing_border_box_applies_to_height_too() {
    let laid = measure_fixture(
        r#"<html><body><div style="box-sizing: border-box; height: 50px; padding: 10px;">x</div></body></html>"#,
        400.0,
    );
    let div = find_by_height(&laid, 50.0).expect("div with explicit height not found");
    assert_eq!(
        div.h, 50.0,
        "border-box: the specified height is the box's total height, not the content height with padding added on top"
    );
}

#[test]
fn min_width_can_force_the_box_wider_than_its_container() {
    let laid = measure_fixture(
        r#"<html><body><div style="min-width: 300px; margin-top: 1px;">x</div></body></html>"#,
        100.0,
    );
    let div = find_laid_by_margin_top(&laid, 1.0).expect("div not found");
    assert_eq!(
        div.w, 300.0,
        "min-width should win even past the container's own available width, same as real CSS"
    );
}

#[test]
fn max_width_clamps_the_box_narrower_than_an_explicit_width() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 500px; max-width: 200px;">x</div></body></html>"#,
        900.0,
    );
    let div = find_by_width(&laid, 500.0).expect("div with explicit width not found");
    assert_eq!(div.w, 200.0, "max-width should clamp the box below its own explicit width, same as real CSS");
}

#[test]
fn min_width_wins_when_author_declares_min_greater_than_max() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 50px; min-width: 400px; max-width: 200px;">x</div></body></html>"#,
        900.0,
    );
    let div = find_by_width(&laid, 50.0).expect("div with explicit width not found");
    assert_eq!(
        div.w, 400.0,
        "when min-width exceeds max-width, the used value is min-width, never something smaller (real CSS clamp order: max first, then min overrides)"
    );
}

#[test]
fn max_height_clamps_reported_height_but_children_still_fully_laid_out() {
    let laid = measure_fixture(
        r#"<html><body><div style="max-height: 10px; margin-top: 1px;"><p>line one</p><p>line two</p></div></body></html>"#,
        400.0,
    );
    let outer = find_laid_by_margin_top(&laid, 1.0).expect("div not found");
    assert_eq!(outer.h, 10.0, "max-height should clamp the box's own reported height even though height itself is auto");
    let LaidContent::Blocks(children) = &outer.content else {
        panic!("expected the div's content to be Blocks (two <p> children)");
    };
    assert_eq!(children.len(), 2, "both paragraphs should still be present, not truncated");
    for p in children {
        assert!(p.h > 0.0, "each paragraph should still be fully laid out with its own real height, not clipped away");
    }
}

// ---------- v54: `float`/`clear` ----------

#[test]
fn float_left_is_removed_from_normal_flow_and_positioned_at_the_containing_blocks_edge() {
    // No inter-tag whitespace — same reason every other exact-pixel
    // flow test in this file avoids it (a whitespace-only text node
    // would otherwise become its own `Blocks` child).
    let laid = measure_fixture(
        r#"<html><body><div style="width: 300px;"><div style="float: left; width: 50px; height: 80px;"></div><div style="height: 20px;">normal</div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("container not found");
    let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
    assert_eq!(
        children.len(),
        2,
        "a float still produces a real Laid box in its parent's children (unlike `position: absolute`, which removes its slot entirely) — it just doesn't advance the flow cursor"
    );
    let float_box = children.iter().find(|c| c.h == 80.0).expect("float box not found");
    let normal = children.iter().find(|c| c.h == 20.0).expect("normal sibling not found");
    assert_eq!(
        (float_box.x, float_box.y),
        (container.x, container.y),
        "an unobstructed `float: left` should sit at the containing block's own content-box left/top edge"
    );
    assert_eq!(float_box.w, 50.0, "an explicit float width should be honored exactly, same as any other box");
    assert_eq!(
        normal.y, container.y,
        "the sibling after a float should land exactly where it would if the float didn't exist at all — a float doesn't advance the flow cursor"
    );
    assert_eq!(
        normal.x, container.x,
        "an ordinary (non-floated) block's own border-box position is unaffected by a float — only its *inline* content narrows around one, not its own box geometry"
    );
}

#[test]
fn float_right_is_positioned_against_the_containing_blocks_right_edge() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 300px;"><div style="float: right; width: 50px; height: 30px;"></div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("container not found");
    let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
    let float_box = children.iter().find(|c| c.h == 30.0).expect("float box not found");
    assert_eq!(
        float_box.x,
        container.x + 300.0 - 50.0,
        "an unobstructed `float: right` should sit flush with the containing block's own right edge"
    );
    assert_eq!(float_box.y, container.y);
}

#[test]
fn clear_moves_a_box_below_an_earlier_float() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 300px;"><div style="float: left; width: 50px; height: 80px;"></div><div style="clear: left; height: 20px;">cleared</div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("container not found");
    let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
    assert_eq!(children.len(), 2);
    let float_box = children.iter().find(|c| c.h == 80.0).expect("float box not found");
    let cleared = children.iter().find(|c| c.h == 20.0).expect("cleared sibling not found");
    assert_eq!(
        cleared.y,
        float_box.y + float_box.h,
        "clear: left should push this box down to exactly the bottom of the earlier left float, instead of landing beside/under it like the uncleared sibling in the test above"
    );
}

#[test]
fn float_in_one_section_affects_text_and_clear_in_the_next_section() {
    // Q34a: a float belongs to its BFC, not its parent `Blocks` box.
    let word_text = "hi ".repeat(40);
    let html = format!(
        r#"<html><body><div style="width: 300px;"><section><div style="float: right; width: 200px; height: 300px;"></div></section><section><p style="margin: 0">{word_text}</p><div style="clear: right; height: 10px;"></div></section></div></body></html>"#
    );
    let laid = measure_fixture(&html, 400.0);
    let container = find_by_width(&laid, 300.0).expect("container not found");
    let run = find_mixed_inline(&laid).expect("paragraph beside a float from an earlier section takes the per-word path");
    let positions: Vec<(f32, f32)> = mixed_items(run).iter().map(mixed_item_xy).collect();
    let first_y = positions.iter().map(|&(_, y)| y).fold(f32::MAX, f32::min);
    assert!(first_y < container.y + 300.0);
    // Float is 200 wide at the right edge of 300: every word must stay
    // within the left 100px corridor.
    for &(x, y) in positions.iter().filter(|&&(_, y)| y < container.y + 300.0) {
        assert!(x < container.x + 100.0 + 0.01, "word at ({x},{y}) is under the float");
    }
    let cleared = find_by_height(&laid, 10.0).expect("cleared div not found");
    assert!(cleared.y >= container.y + 300.0 - 0.01, "clear: right must see the earlier section's float, got y={}", cleared.y);
}

#[test]
fn bfc_root_beside_a_float_is_narrowed_to_the_corridor() {
    // Q34b: `overflow: hidden` must not overlap an earlier float.
    let laid = measure_fixture(
        r#"<html><body><div style="width: 300px;"><div style="float: right; width: 100px; height: 50px;"></div><div style="overflow: hidden; height: 20px;"></div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("container not found");
    let bfc = find_by_height(&laid, 20.0).expect("overflow box not found");
    assert!(bfc.x + bfc.w <= container.x + 200.0 + 0.01, "bfc root overlaps the float: x={} w={}", bfc.x, bfc.w);
    assert_eq!(bfc.x, container.x);
}

#[test]
fn flow_root_contains_its_own_floats() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 300px; display: flow-root;"><div style="float: left; width: 50px; height: 100px;"></div></div></body></html>"#,
        400.0,
    );
    let root = find_by_width(&laid, 300.0).expect("flow-root not found");
    assert_eq!(root.h, 100.0, "flow-root's auto height must include its float");
}

#[test]
fn block_display_table_without_width_shrinks_to_fit() {
    // Q50 (W14): no float, no width -> shrink-to-fit, not container fill.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:600px"><table><tr><td>a</td><td>b</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 600.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    assert!(cs[0].w < 100.0, "auto-width table must shrink to its cells, w={}", cs[0].w);
}

#[test]
fn q51_table_width_includes_border_and_padding() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:400px"><table style="width:100%;border:5px solid black;padding:10px"><tr><td>a</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 400.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    assert_eq!(cs[0].w, 400.0, "table width:100% is its border box");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><table style="width:200px;border:5px solid black;padding:10px"><tr><td>a</td></tr></table></body></html>"#,
        700.0,
    );
    let t = find_by_width(&laid, 200.0).expect("table must be 200px wide including border+padding");
    assert_eq!(t.w, 200.0);
}

#[test]
fn shrink_to_fit_table_is_sum_of_column_maxima_so_no_cell_wraps() {
    // Q50: rows with different per-column widths must not make cells wrap.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:600px"><table><tr><td>Founded</td><td>1993</td></tr><tr><td>HQ</td><td>Santa Clara</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 600.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    let LaidContent::Blocks(rows) = &cs[0].content else { panic!("expected rows") };
    assert!(rows[0].h < 30.0, "first row must stay on one line, h={}", rows[0].h);
    assert!(rows[1].h < 30.0, "second row must stay on one line, h={}", rows[1].h);
}

#[test]
fn shrink_to_fit_table_cell_never_wraps_whatever_the_fractional_glyph_advances() {
    // QCI0a: a column sized to exactly its max-content must hold the text
    // on one line even when cosmic-text's running glyph sum lands one f32
    // ulp above the separately-measured extent (font-dependent luck, seen
    // with 9.92px monospace-ish advances). Sweep many lengths so the
    // relation is exercised under any installed sans-serif.
    let one = |t: &str| {
        let html = format!(
            r#"<html><body style="margin:0"><div style="width:900px"><table><tr><td>{t}</td><td>x</td></tr></table></div></body></html>"#
        );
        let laid = measure_fixture(&html, 1000.0);
        let c = find_by_width(&laid, 900.0).expect("container not found");
        let LaidContent::Blocks(cs) = &c.content else { panic!("expected Blocks") };
        let LaidContent::Blocks(rows) = &cs[0].content else { panic!("expected rows") };
        rows[0].h
    };
    let single = one("a");
    let words = ["Santa", "Clara", "mmm", "iii", "Wqj", "1993", "HQ", "ffi"];
    for n in 1..=12 {
        let text = (0..n).map(|i| words[(i * 3 + n) % words.len()]).collect::<Vec<_>>().join(" ");
        let h = one(&text);
        assert!(h <= single + 0.01, "cell {text:?} wrapped: h={h} vs {single}");
    }
}

#[test]
fn table_caption_does_not_widen_a_shrink_to_fit_table_and_wraps_below() {
    // Q34c: caption adds only its min-content width.
    let cap = "word ".repeat(30);
    let html = format!(
        r#"<html><body style="margin:0"><div style="width: 600px;"><div style="display: table; float: right;"><div>wwwwwwwwwwwwwwwwwwww</div><div style="display: table-caption; caption-side: bottom;">{cap}</div></div></div></body></html>"#
    );
    let laid = measure_fixture(&html, 700.0);
    let container = find_by_width(&laid, 600.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    let table = &cs[0];
    assert!(table.w < 300.0, "table must not be as wide as the caption's one-line width, w={}", table.w);
    let LaidContent::Blocks(children) = &table.content else { panic!("expected Blocks") };
    assert_eq!(children.len(), 2);
    let (block, caption) = (&children[0], &children[1]);
    assert!(caption.y >= block.y + block.h - 0.01, "caption must sit below the grid content");
    assert!(caption.h > 60.0, "caption should wrap onto several lines, h={}", caption.h);
}

#[test]
fn mixed_inline_id_consumption_does_not_depend_on_line_breaks() {
    // Q34d: progressive layout re-runs `measure_document` with the
    // same cache and `next_id` reset, but widths differ between passes
    // (0 for not-yet-shaped text). If the number of ids a run consumes
    // depends on where lines break, every later id shifts and later
    // items read another paragraph's cache entry (blank/wrong words).
    let para = "alpha beta gamma delta epsilon zeta eta theta iota kappa ".repeat(6);
    let mut body = String::from(r#"<div style="float: right; width: 50px; height: 9000px;"></div>"#);
    for i in 0..4 {
        body.push_str(&format!(r#"<p style="margin: 0">p{i} {para}</p>"#));
    }
    let html = format!(r#"<html><body style="margin:0">{body}</body></html>"#);
    let doc = parse_html(&html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut fs = new_font_system();
    let mut ids_used = |width: f32| {
        let mut cache = ShapeCache::new();
        let (mut id, mut inc) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        measure_document(&tree, &mut fs, width, 900.0, &mut id, &mut cache, deadline, &mut inc);
        id
    };
    let (a, b) = (ids_used(5000.0), ids_used(300.0));
    assert_eq!(a, b, "ids consumed must not depend on how lines wrap");
}

#[test]
fn float_paints_above_a_later_full_width_siblings_own_background() {
    // A later, non-cleared, full-width sibling's own box is never
    // narrowed to avoid an earlier float (only its *inline* content
    // is — see the geometry tests above), so its own background can
    // legitimately overlap the float's area. Real CSS still paints
    // the float on top in that overlap (CSS2.1 Appendix E: floats
    // are their own stacking level, above ordinary in-flow,
    // non-positioned block-level boxes) — found as a real, visible
    // bug via a `grim` screenshot before this test existed: without
    // special-casing paint order, the float (collected into its
    // parent's `Blocks` children at its own point in document order,
    // same as any other child) painted *underneath* the later green
    // sibling's own opaque background instead.
    let html = r#"<html><body><div style="width: 400px;"><div style="float: left; width: 150px; height: 150px; background-color: #ff0000;"></div><div style="background-color: #00ff00; height: 100px;"></div></div></body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 400.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (400u32, 400u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    // `body { margin: 8px }` (default.css) offsets everything by
    // 8px — (50, 50) is well inside both the float's 150x150 area
    // and the green sibling's full-width/100px-tall area, which
    // start at the same (8, 8) origin since the float never
    // advances the flow cursor.
    let px = pixels[50 * win_w as usize + 50];
    assert_eq!(px & 0x00FF_FFFF, 0x00FF_0000, "the float (red) should paint on top of the later sibling's own background (green) in their overlap region");
}

#[test]
fn inline_text_narrows_beside_a_left_float_and_resumes_full_width_below_it() {
    // Many repeated short words in a container only 40px wider than
    // the float, so wrapping to at least two lines is guaranteed
    // regardless of real font metrics on whatever system runs this
    // test (same "don't depend on exact glyph widths" discipline
    // `mixed_inline_wraps_when_content_does_not_fit_on_one_line`
    // already established with images) — the float itself is
    // exactly one default line-height (20px) tall, so only the
    // line(s) starting at `y == 0` overlap it; anything starting at
    // `y >= 20` is entirely below it and should see the container's
    // full, unnarrowed width again. `margin: 0` on the `<p>` so its
    // own inline content starts exactly at the container's content
    // box, not shifted down by `default.css`'s own paragraph margin.
    let word_text = "hi ".repeat(40);
    let html = format!(
        r#"<html><body><div style="width: 120px;"><div style="float: left; width: 80px; height: 20px;"></div><p style="margin: 0">{word_text}</p></div></body></html>"#
    );
    let laid = measure_fixture(&html, 400.0);
    let container = find_by_width(&laid, 120.0).expect("container not found");
    let run = find_mixed_inline(&laid)
        .expect("a plain-text paragraph sharing a BFC with an earlier float should take the floats-aware per-word path, not the plain whole-buffer one");
    let positions: Vec<(f32, f32)> = mixed_items(run).iter().map(mixed_item_xy).collect();
    let float_right_edge = container.x + 80.0;
    let float_bottom = container.y + 20.0;

    let first_line_y = positions.iter().map(|&(_, y)| y).fold(f32::MAX, f32::min);
    let (first_x, _) = *positions.iter().find(|&&(_, y)| y == first_line_y).unwrap();
    assert!(
        first_x >= float_right_edge - 0.01,
        "a line overlapping the float's own y range should start at/after its right edge ({float_right_edge}), not the container's own left edge ({}); got x={first_x}",
        container.x
    );

    let below_float = positions.iter().find(|&&(_, y)| y >= float_bottom - 0.01);
    let (x_below, _) = *below_float
        .expect("with 40 words wrapping through a 40px-narrow corridor, at least one line should end up entirely below the float");
    assert!(
        (x_below - container.x).abs() < 0.01,
        "a line entirely below the float's bottom should resume the container's own full width, starting back at its own left edge ({}); got x={x_below}",
        container.x
    );
}

// ---------- `display: flow-root` ----------

/// `<span>` (default `display: inline`) is the right element to prove
/// this with, not a `<div>` — a `<div>` already defaults to `display:
/// block` via `default.css`, so it would report exactly the same
/// layout whether `flow-root` parsed correctly or silently failed and
/// left the property unset. `find_by_height` only ever matches a
/// real `Laid` node's own box (never flattened inline content, which
/// produces no `Laid` of its own at all — see its own docs), so
/// finding one here already proves the span got a real block box,
/// not just a plain-text run.
#[test]
fn display_flow_root_gets_its_own_block_box_not_flattened_into_the_inline_run() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><span style="display: flow-root; height: 40px;">a</span><div style="height: 20px;">b</div></body></html>"#,
        500.0,
    );
    let flow_root = find_by_height(&laid, 40.0).expect("flow-root span should get its own block box, not be flattened into an inline run");
    assert_eq!(flow_root.w, 500.0, "the flow-root box should stretch to the full container width like an ordinary auto-width block box");
    let sibling = find_by_height(&laid, 20.0).expect("sibling div not found");
    assert_eq!(sibling.y, flow_root.y + flow_root.h, "the sibling div should stack below the flow-root box, not share a line with it");
}

// ---------- `display: inline-block`/`inline-flex` ----------

fn find_embedded(items: &[MixedPositioned], predicate: impl Fn(&Laid) -> bool) -> Option<&Laid> {
    items.iter().find_map(|item| match item {
        MixedPositioned::Embedded(laid) if predicate(laid) => Some(laid.as_ref()),
        _ => None,
    })
}

#[test]
fn inline_block_shares_a_line_with_text_and_keeps_its_own_explicit_size() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><p style="margin: 0">a <span style="display: inline-block; width: 50px; height: 20px;">b</span> c</p></body></html>"#,
        500.0,
    );
    let run = find_mixed_inline(&laid).expect("paragraph should take the mixed-inline path, not the fast plain-text one");
    let items = mixed_items(run);
    assert!(
        items.iter().any(|i| matches!(i, MixedPositioned::Word { .. })),
        "the surrounding text ('a'/'c') should still be real words sharing this same run"
    );
    let embedded = find_embedded(items, |l| (l.w, l.h) == (50.0, 20.0))
        .expect("inline-block span should get its own Embedded box with its explicit 50x20 size, not be flattened or ignored");
    assert_eq!(embedded.y, items.iter().find_map(|i| match i { MixedPositioned::Word { y, .. } => Some(*y), _ => None }).unwrap(), "should share the same line (same y) as the surrounding words, not stack like a block");
}

#[test]
fn inline_flex_lays_its_children_out_side_by_side_not_stacked() {
    // A plain `<div>` wrapper, not `<p>` — real HTML5 tree
    // construction auto-closes an open `<p>` the instant a nested
    // `<div>` start tag appears anywhere inside it (confirmed by
    // dumping the parsed DOM directly: a `<p>` wrapper silently
    // splits into two sibling `<p>`s with the flex `<div>`s
    // orphaned onto `<body>` in between, nothing to do with this
    // feature at all), the same real browser behavior `html5ever`
    // is correctly implementing here, not a parsing bug to work
    // around in the renderer.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="margin: 0">a <span style="display: inline-flex; width: 100px;"><div style="flex: 1;">x</div><div style="flex: 1;">y</div></span> b</div></body></html>"#,
        500.0,
    );
    let run = find_mixed_inline(&laid).expect("paragraph should take the mixed-inline path");
    let items = mixed_items(run);
    let embedded =
        find_embedded(items, |l| l.w == 100.0).expect("inline-flex span should get its own 100px-wide Embedded box");
    let LaidContent::Blocks(children) = &embedded.content else {
        panic!("expected the inline-flex box's own content to be Blocks (its two flex items), not normal block-stacked content")
    };
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].y, children[1].y, "two equally-flexed items should share one row, not stack vertically — proves this actually flexed, not just became an inline-block");
    assert_eq!(children[0].w, 50.0, "flex: 1 on both items should split the 100px evenly");
    assert_eq!(children[1].x, children[0].x + children[0].w, "the second flex item should start exactly where the first ends");
}

/// R33/QCI0: `flex: 1` is `1 1 0%`, so two items with very different content
/// widths must still split the box exactly, whatever the system sans-serif.
/// (Single glyphs: since QCI0b an unbreakable word wider than half the box
/// legitimately keeps its min-content width, as in browsers.)
#[test]
fn flex_number_shorthand_has_zero_basis_so_split_ignores_glyph_widths() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: flex; width: 100px;"><div style="flex: 1;">i</div><div style="flex: 1;">W</div></div></body></html>"#,
        500.0,
    );
    let row = find_embedded_or_block_with_two_children(&laid);
    assert_eq!(row[0].w, 50.0);
    assert_eq!(row[1].w, 50.0);
}

/// First `Blocks` node with exactly two children side by side (same y), or
/// stacked (`stacked`), anywhere in the tree.
fn qci0b_pair(l: &Laid, stacked: bool) -> Option<&Vec<Laid>> {
    if let LaidContent::Blocks(c) = &l.content {
        if c.len() == 2 && (c[0].y == c[1].y) != stacked {
            return Some(c);
        }
        return c.iter().find_map(|k| qci0b_pair(k, stacked));
    }
    None
}

/// Width and height of `text` as a shrink-to-fit float, font-independent.
fn qci0b_natural(text: &str) -> (f32, f32) {
    let laid = measure_fixture(
        &format!(r#"<html><body style="margin:0"><div style="float:left">{text}</div></body></html>"#),
        3000.0,
    );
    let div = qci0b_float(&laid).expect("float");
    (div.w, div.h)
}

fn qci0b_float(l: &Laid) -> Option<&Laid> {
    if let LaidContent::Blocks(c) = &l.content {
        for k in c {
            if k.w > 0.0 && k.w < 2999.0 && k.h > 0.0 {
                return Some(k);
            }
            if let Some(r) = qci0b_float(k) {
                return Some(r);
            }
        }
    }
    None
}

const QCI0B_WORD: &str = "Pneumonoultramicroscopicsilicovolcanoconiosis";

/// R35/QCI0b: automatic minimum size is min-content, not clamped to the basis.
#[test]
fn qci0b_flex_number_item_keeps_long_word_whole_and_overflows() {
    let (word_w, word_h) = qci0b_natural(QCI0B_WORD);
    let short_w = ["short", "text", "here"].iter().map(|w| qci0b_natural(w).0).fold(0.0f32, f32::max);
    assert!(word_w > 300.0);
    let laid = measure_fixture(
        &format!(r#"<html><body style="margin:0"><div style="display:flex;width:300px;align-items:flex-start"><div style="flex:1">{QCI0B_WORD}</div><div style="flex:1">short text here</div></div></body></html>"#),
        1000.0,
    );
    let row = qci0b_pair(&laid, false).expect("row");
    assert!((row[0].w - word_w).abs() < 0.5, "item0 {} vs word {}", row[0].w, word_w);
    assert!((row[0].h - word_h).abs() < 0.5, "one line: {} vs {}", row[0].h, word_h);
    // min-content of "short text here" is its widest word (the engine adds
    // about one space width of slack, so allow a few px, but far below 300/2).
    assert!(row[1].w >= short_w - 0.5 && row[1].w < short_w + 10.0, "item1 {} vs widest word {}", row[1].w, short_w);
}

#[test]
fn qci0b_column_flex_items_keep_content_height() {
    let (_, h3) = qci0b_natural("one<br>two<br>three");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display:flex;flex-direction:column;height:100px"><div style="flex:1">one<br>two<br>three</div><div style="flex:1">four<br>five<br>six</div></div></body></html>"#,
        500.0,
    );
    let col = qci0b_pair(&laid, true).expect("column");
    assert!((col[0].h - h3).abs() < 0.5, "item0 h {} vs {}", col[0].h, h3);
    assert!((col[1].h - h3).abs() < 0.5, "item1 h {} vs {}", col[1].h, h3);
    assert!((col[1].y - (col[0].y + col[0].h)).abs() < 0.5, "no overlap");
}

#[test]
fn qci0b_specified_width_caps_the_automatic_minimum() {
    let laid = measure_fixture(
        &format!(r#"<html><body style="margin:0"><div style="display:flex;width:300px"><div style="flex:1;width:50px">{QCI0B_WORD}</div><div style="flex:none;width:280px">x</div></div></body></html>"#),
        1000.0,
    );
    let row = qci0b_pair(&laid, false).expect("row");
    assert_eq!(row[0].w, 50.0);
    assert_eq!(row[1].w, 280.0);
}

#[test]
fn qci0b_scroll_container_item_has_zero_floor_ellipsis_pattern() {
    let laid = measure_fixture(
        &format!(r#"<html><body style="margin:0"><div style="display:flex;width:300px"><div style="flex:1;overflow:hidden;white-space:nowrap;text-overflow:ellipsis">{QCI0B_WORD} {QCI0B_WORD}</div><div style="flex:none;width:100px">x</div></div></body></html>"#),
        1000.0,
    );
    let row = qci0b_pair(&laid, false).expect("row");
    assert_eq!(row[0].w, 200.0);
    assert_eq!(row[1].w, 100.0);
}

fn find_embedded_or_block_with_two_children(l: &Laid) -> &Vec<Laid> {
    fn go(l: &Laid) -> Option<&Vec<Laid>> {
        if let LaidContent::Blocks(c) = &l.content {
            if c.len() == 2 && c[0].y == c[1].y && c[0].w + c[1].w == 100.0 {
                return Some(c);
            }
            return c.iter().find_map(go);
        }
        None
    }
    go(l).expect("flex row with two items")
}

#[test]
fn q7_css_wide_keywords_inherit_initial_unset_revert() {
    // Resolves <div> then its <p> child.
    let styles = |css: &str, html: &str| {
        let doc = parse_html(html);
        let rules = parse_stylesheet(css);
        let div = find_element_by_tag(&doc, "div").expect("no <div>");
        let p = find_element_by_tag(&doc, "p").expect("no <p>");
        let ds = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        resolve_style(&p, &rules, ds.inherited(), 800.0)
    };
    let red = TextColor::rgb(255, 0, 0);
    let h = r#"<div><p>x</p></div>"#;
    // inherit on a non-inherited property reads the parent's value
    let s = styles("div{background-color:red;display:block} p{background-color:inherit;display:inherit}", h);
    assert_eq!(s.background_color, Some(red));
    assert_eq!(s.display, Display::Block);
    // background shorthand
    assert_eq!(styles("div{background:red} p{background:inherit}", h).background_color, Some(red));
    // initial on an inherited property
    assert_eq!(styles("div{color:red} p{color:initial}", h).color, ROOT_STYLE.color);
    assert_eq!(styles("div{color:red} p{color:inherit}", h).color, red);
    // unset: inherited prop inherits, non-inherited resets
    assert_eq!(styles("div{color:red;background:red} p{color:unset;background:unset}", h).color, red);
    assert_eq!(styles("div{color:red;background:red} p{color:unset;background:unset}", h).background_color, None);
    // revert behaves as unset
    assert_eq!(styles("div{color:red} p{color:revert}", h).color, red);
    assert_eq!(styles("div{font-size:30px} p{font-size:initial}", h).font_size, 16.0);
    assert_eq!(styles("div{font-size:30px} p{font-size:unset}", h).font_size, 30.0);
    // keywords win the cascade at their own specificity
    assert_eq!(styles("div{color:red} #a{color:blue} p{color:inherit}", r#"<div><p id="a">x</p></div>"#).color, TextColor::rgb(0, 0, 255));
    assert_eq!(styles("div{color:red} p{color:blue} #a{color:inherit}", r#"<div><p id="a">x</p></div>"#).color, red);
    // last declaration in the same rule wins, in either order
    assert_eq!(styles("div{color:red} p{color:blue;color:inherit}", h).color, red);
    assert_eq!(styles("div{color:red} p{color:inherit;color:blue}", h).color, TextColor::rgb(0, 0, 255));
    // !important keyword and inline style
    assert_eq!(styles("div{color:red} p{color:inherit !important}", r#"<div><p style="color:blue">x</p></div>"#).color, red);
    assert_eq!(styles("div{color:red}", r#"<div><p style="color:initial">x</p></div>"#).color, ROOT_STYLE.color);
}

#[test]
fn q7c_css_wide_keywords_text_decoration_and_logical_shorthands() {
    let styles = |css: &str, html: &str| {
        let doc = parse_html(html);
        let rules = parse_stylesheet(css);
        let div = find_element_by_tag(&doc, "div").expect("no <div>");
        let p = find_element_by_tag(&doc, "p").expect("no <p>");
        let ds = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        resolve_style(&p, &rules, ds.inherited(), 800.0)
    };
    let h = r#"<div><p>x</p></div>"#;
    // text-decoration-line: inherit takes the parent's own value
    let s = styles("div{text-decoration-line:underline} p{text-decoration-line:inherit}", h);
    assert!(s.text_decoration_line.is_some_and(|l| l.underline));
    // initial / unset leave it undeclared
    let s = styles("div{text-decoration-line:underline} p{text-decoration-line:underline;text-decoration-line:initial}", h);
    assert!(s.text_decoration_line.is_none());
    let s = styles("p{text-decoration-line:underline;text-decoration-line:unset}", h);
    assert!(s.text_decoration_line.is_none());
    // shorthand keyword covers the longhands too
    let s = styles("div{text-decoration:underline dashed red} p{text-decoration:inherit}", h);
    assert!(s.text_decoration.is_some());
    let s = styles("div{text-decoration-color:red} p{text-decoration-color:inherit}", h);
    assert!(s.text_decoration_color.is_some());
    let s = styles("div{text-decoration-style:dashed} p{text-decoration-style:inherit}", h);
    assert!(s.text_decoration_style.is_some());
    // later normal declaration beats an earlier keyword
    let s = styles("div{text-decoration-style:dashed} p{text-decoration-style:inherit;text-decoration-style:dotted}", h);
    assert!(s.text_decoration_style == Some(DecorationStyle::Dotted));
    // logical border shorthands
    let s = styles("div{border:3px solid red} p{border-block:inherit}", h);
    assert_eq!(s.border_width, [3.0, 0.0, 3.0, 0.0]);
    let s = styles("div{border:3px solid red} p{border-inline-start-width:inherit}", h);
    assert_eq!(s.border_width, [0.0, 0.0, 0.0, 3.0]);
    // inset-*
    let s = styles("div{top:5px;left:6px} p{position:relative;inset:inherit}", h);
    assert!(matches!(s.inset.top, Some(BoxLength::Px(v)) if v == 5.0));
    assert!(matches!(s.inset.left, Some(BoxLength::Px(v)) if v == 6.0));
    let s = styles("div{top:5px} p{position:relative;top:5px;inset-block-start:initial}", h);
    assert!(s.inset.top.is_none());
}

#[test]
fn q7b_css_wide_keywords_on_remaining_properties() {
    let styles = |css: &str, html: &str| {
        let doc = parse_html(html);
        let rules = parse_stylesheet(css);
        let div = find_element_by_tag(&doc, "div").expect("no <div>");
        let p = find_element_by_tag(&doc, "p").expect("no <p>");
        let ds = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        resolve_style(&p, &rules, ds.inherited(), 800.0)
    };
    let h = r#"<div><p>x</p></div>"#;
    // margin shorthand inherit
    let s = styles("div{margin:10px} p{margin:inherit}", h);
    assert!(matches!(s.margin.top, BoxLength::Px(v) if v == 10.0));
    assert!(matches!(s.margin.left, BoxLength::Px(v) if v == 10.0));
    // padding longhand inherit + initial
    let s = styles("div{padding-left:7px} p{padding-left:inherit}", h);
    assert!(matches!(s.padding.left, BoxLength::Px(v) if v == 7.0));
    let s = styles("div{padding:7px} p{padding:5px;padding:initial}", h);
    assert!(matches!(s.padding.top, BoxLength::Px(v) if v == 0.0));
    // width / position / float / overflow
    let s = styles("div{width:120px;position:relative;overflow:hidden} p{width:inherit;position:inherit;overflow:inherit}", h);
    assert!(matches!(s.width, Some(BoxLength::Px(v)) if v == 120.0));
    assert!(s.position == Position::Relative);
    assert!(s.overflow_x == Overflow::Hidden && s.overflow_y == Overflow::Hidden);
    // initial resets a rule's own declared value; unset on a
    // non-inherited property also resets
    let s = styles("p{float:left;float:initial}", h);
    assert!(s.float == Float::None);
    assert!(styles("div{width:50px} p{width:unset}", h).width.is_none());
    // display is re-forced to block when inherit brings in a float
    let s = styles("div{float:left} p{float:inherit}", h);
    assert!(s.float == Float::Left);
    assert_eq!(s.display, Display::Block);
    // border shorthand and per-side
    let s = styles("div{border:3px solid red} p{border:inherit}", h);
    assert_eq!(s.border_width, [3.0; 4]);
    let s = styles("div{border-top:3px solid red} p{border-top-width:inherit}", h);
    assert_eq!(s.border_width, [3.0, 0.0, 0.0, 0.0]);
    // inherited property via unset
    assert_eq!(styles("div{font-style:italic} p{font-style:unset}", h).font_style, FontStyle::Italic);
    // cascade order: later normal declaration beats earlier keyword
    let s = styles("div{margin-top:10px} p{margin-top:inherit;margin-top:4px}", h);
    assert!(matches!(s.margin.top, BoxLength::Px(v) if v == 4.0));
    // specificity: a more specific keyword beats a less specific value
    let s = styles("div{margin-top:10px} p{margin-top:4px} #a{margin-top:inherit}", r#"<div><p id="a">x</p></div>"#);
    assert!(matches!(s.margin.top, BoxLength::Px(v) if v == 10.0));
    // background: inherit also takes the parent's image
    let uri = tiny_png_data_uri(2, 2);
    let doc = parse_html(r#"<div><p>x</p></div>"#);
    let rules = parse_stylesheet(&format!("div{{background-image:url({uri})}} p{{background:inherit}}"));
    let p = find_element_by_tag(&doc, "p").unwrap();
    assert!(resolve_background_image(&p, &rules, 800.0, None, &HashMap::new()).is_some());
    let rules = parse_stylesheet(&format!("div{{background-image:url({uri})}} p{{background-image:initial}}"));
    assert!(resolve_background_image(&p, &rules, 800.0, None, &HashMap::new()).is_none());
}

#[test]
fn q6_important_declarations_beat_specificity_and_inline_style() {
    let color_of = |css: &str, html: &str| {
        let doc = parse_html(html);
        let p = find_element_by_tag(&doc, "p").expect("no <p>");
        resolve_style(&p, &parse_stylesheet(css), ROOT_STYLE, 800.0).color
    };
    let red = TextColor::rgb(255, 0, 0);
    let blue = TextColor::rgb(0, 0, 255);
    assert_eq!(color_of("#a{color:red} p{color:blue !important}", r#"<p id="a">x</p>"#), blue);
    assert_eq!(color_of("#a{color:red} p{color:blue}", r#"<p id="a">x</p>"#), red);
    // important rule beats a normal inline style
    assert_eq!(color_of("p{color:blue !important}", r#"<p style="color:red">x</p>"#), blue);
    // inline important beats important rule
    assert_eq!(color_of("p{color:blue !important}", r#"<p style="color:red ! IMPORTANT">x</p>"#), red);
    // within the tier, specificity then source order
    assert_eq!(color_of("#a{color:red !important} p{color:blue !important}", r#"<p id="a">x</p>"#), red);
}

#[test]
fn q9_vertical_align_box_keywords_on_inline_blocks_and_table_cells() {
    // Inline-block (embedded) boxes on one line: 40px tall one sets
    // the line height; the 10px one moves per its `vertical-align`.
    let offset = |va: &str| -> f32 {
        let html = format!(
            r#"<html><body style="margin:0"><p style="margin:0"><span style="display:inline-block;width:20px;height:40px">a</span><span style="display:inline-block;width:30px;height:10px;vertical-align:{va}">b</span></p></body></html>"#
        );
        let laid = measure_fixture(&html, 400.0);
        let run = find_mixed_inline(&laid).expect("mixed inline run");
        let tall = find_embedded(mixed_items(run), |l| l.w == 20.0).expect("tall box");
        let small = find_embedded(mixed_items(run), |l| l.w == 30.0).expect("small box");
        small.y - tall.y
    };
    assert_eq!(offset("baseline"), 30.0);
    assert_eq!(offset("bottom"), 30.0);
    assert_eq!(offset("text-bottom"), 30.0);
    assert_eq!(offset("middle"), 15.0);
    assert_eq!(offset("top"), 0.0);
    assert_eq!(offset("text-top"), 0.0);

    // Table cells: a 60px-tall cell next to a short one.
    let cell_content_dy = |va: &str| -> (f32, f32) {
        let html = format!(
            r#"<html><body style="margin:0"><table style="border-spacing:0"><tr><td style="padding:0"><div style="height:60px">a</div></td><td style="padding:0;vertical-align:{va}"><div style="height:10px">b</div></td></tr></table></body></html>"#
        );
        let laid = measure_fixture(&html, 400.0);
        let cells = find_table_row_cells(&laid).expect("row cells");
        let inner = find_by_height(&cells[1], 10.0).expect("inner div");
        (inner.y - cells[1].y, cells[1].h)
    };
    assert_eq!(cell_content_dy("top"), (0.0, 10.0));
    assert_eq!(cell_content_dy("middle"), (25.0, 60.0));
    assert_eq!(cell_content_dy("bottom"), (50.0, 60.0));
}

#[test]
fn q5_bare_text_in_flex_container_becomes_anonymous_flex_item() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display:flex">hello <b>x</b></div></body></html>"#,
        500.0,
    );
    let mut ids = Vec::new();
    collect_text_shape_ids(&laid, &mut ids);
    assert_eq!(ids.len(), 2, "bare text 'hello' should lay out as its own item alongside <b>x</b>, not be dropped");
}

#[test]
fn inline_block_width_auto_shrinks_to_its_own_content_not_the_full_line() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><p style="margin: 0"><span style="display: inline-block;">hi</span></p></body></html>"#,
        400.0,
    );
    let run = find_mixed_inline(&laid).expect("paragraph should take the mixed-inline path");
    let items = mixed_items(run);
    let embedded = find_embedded(items, |_| true).expect("inline-block span not found");
    assert!(
        embedded.w < 400.0 && embedded.w > 0.0,
        "a width:auto inline-block should shrink-to-fit its own short content (CSS2.1 §10.3.5), not fill the whole 400px line the way an ordinary width:auto block would: got {}",
        embedded.w
    );
}

#[test]
fn position_absolute_inside_a_static_inline_block_bubbles_out_to_the_real_positioned_ancestor() {
    let laid = measure_fixture(
        r#"<html><body><div style="position: relative; width: 300px; height: 100px; padding: 10px;"><span style="display: inline-block;"><i style="position: absolute; top: 5px; left: 5px; width: 50px; height: 50px;">x</i></span></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("positioned container not found");
    assert_eq!(
        container.positioned.len(),
        1,
        "the absolute descendant should bubble all the way out past its own static-positioned inline-block parent to resolve against the real positioned ancestor"
    );
    let abs = &container.positioned[0];
    assert_eq!(abs.x, container.x + 5.0, "left should resolve against the containing block's own padding edge, not the inline-block's");
    assert_eq!(abs.y, container.y + 5.0, "top should resolve against the containing block's own padding edge, not the inline-block's");
    assert_eq!((abs.w, abs.h), (50.0, 50.0));
}

/// Regression test for a real panic found via live testing (not by
/// any of the tests above): `measure_mixed_inline` always runs both
/// of its own passes, `trial` or not — `cell_natural_width`'s own
/// two sizing-only probes call the *whole* function, not just its
/// first pass, so an `InlineItem::Embedded`'s pass-2 handling must
/// cope with `laid` genuinely being `None` (trial mode never
/// produces a real one), not assume it can always be taken. A
/// `<table>` cell is the natural real-world trigger, since real CSS
/// auto table-layout column sizing is `cell_natural_width`'s own
/// original use. Asserting this doesn't panic *is* the test; the
/// column-width assertion just confirms the resulting layout is
/// sane, not only crash-free.
#[test]
fn inline_block_inside_a_table_cell_does_not_panic_during_column_sizing() {
    let laid = measure_fixture(
        r#"<html><body><table><tr><td><span style="display: inline-block; width: 30px; height: 10px;">x</span></td></tr></table></body></html>"#,
        400.0,
    );
    let run = find_mixed_inline(&laid).expect("table cell's own mixed-inline run not found");
    let embedded =
        find_embedded(mixed_items(run), |l| (l.w, l.h) == (30.0, 10.0)).expect("inline-block inside the cell should still get its own real, correctly-sized box");
    assert!(embedded.w > 0.0);
}

// ---------- `text-align: start`/`end` ----------

#[test]
fn text_align_start_and_end_resolve_to_left_and_right() {
    let laid = measure_fixture(
        r#"<html><body><div style="text-align: start; width: 10px;">a</div><div style="text-align: end; width: 20px;">b</div></body></html>"#,
        400.0,
    );
    let start = find_by_width(&laid, 10.0).expect("start div not found");
    assert!(start.style.text_align == TextAlign::Left, "text-align: start should resolve to left for this engine's always-left-to-right content");
    let end = find_by_width(&laid, 20.0).expect("end div not found");
    assert!(end.style.text_align == TextAlign::Right, "text-align: end should resolve to right for this engine's always-left-to-right content");
}

// ---------- Q59p: horizontal `margin: auto` ----------

fn find_box_at(laid: &Laid, x: f32, w: f32) -> Option<&Laid> {
    if laid.w == w && laid.x == x {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        return children.iter().find_map(|c| find_box_at(c, x, w));
    }
    None
}

#[test]
fn auto_margins_centre_and_right_align_blocks_and_images() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 50.0, 100.0).is_some(), "width box centred");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:100px;margin-left:auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 100.0, 100.0).is_some(), "right-aligned");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="max-width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 50.0, 100.0).is_some(), "max-width box centred");
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body style="margin:0"><img src="{src}" style="display:block;margin:0 auto"></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    let img = find_image_laid(&laid).expect("img");
    assert_eq!((img.x, img.w), (80.0, 40.0), "block img centred");
    // Floats keep auto margins at 0.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="float:left;width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 0.0, 100.0).is_some(), "float not centred");
}

#[test]
fn q59_block_img_with_explicit_width_centres_with_auto_margins() {
    // Regression: auto margins on a block <img> with a width were resolved
    // twice and the second pass zeroed them (block-replaced-width-006).
    let src = tiny_png_data_uri(15, 15);
    for (body, want_x) in [
        (format!(r#"<div style="width:200px"><img src="{src}" width="50%" style="display:block;margin-left:auto;margin-right:auto"></div>"#), 50.0),
        (format!(r#"<div style="width:200px"><img src="{src}" width="100" style="display:block;margin:0 auto"></div>"#), 50.0),
        (format!(r#"<img src="{src}" width="100" style="display:block;margin:0 auto">"#), 350.0),
    ] {
        let laid = measure_fixture(&format!(r#"<html><body style="margin:0">{body}</body></html>"#), 800.0);
        let img = find_image_laid(&laid).expect("img");
        assert_eq!((img.x, img.w), (want_x, 100.0));
    }
}

#[test]
fn q59_text_node_trailing_space_only_where_source_had_whitespace() {
    let ahem = std::path::PathBuf::from("/home/loupax/src/wpt/fonts/Ahem.ttf");
    let fonts: Vec<std::path::PathBuf> = if ahem.exists() { vec![ahem] } else { vec![] };
    fn collect(l: &Laid, out: &mut Vec<f32>) {
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                out.push(k.w);
                collect(k, out);
            }
        }
    }
    let float_w = |inner: &str| -> f32 {
        let doc = parse_html(&format!(
            "<html><body style=\"margin:0;font:20px Ahem\"><div style=\"float:left\">{inner}</div></body></html>"
        ));
        let tree = build_layout_tree(&doc, 1920.0);
        let mut fs = new_font_system_with(&fonts);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        let mut ws = Vec::new();
        collect(&laid, &mut ws);
        ws.into_iter().filter(|w| *w < 700.0).fold(0.0, f32::max)
    };
    let tight = float_w("a<span>b</span>");
    let spaced = float_w("a <span>b</span>");
    assert!(spaced > tight + 1.0, "real space preserved: {tight} vs {spaced}");
    // Punctuation after an element adds no gap.
    assert!(float_w("<b>x</b>,") <= float_w("<b>x</b> ,") - 1.0);
    // Collapsing across node boundaries: one space, not two.
    assert!((float_w("a <span> b</span>") - spaced).abs() < 0.01);
    if !fonts.is_empty() {
        assert!((tight - 40.0).abs() < 0.01, "{tight}");
        assert!((spaced - 60.0).abs() < 0.01, "{spaced}");
    }
}

// Q48a: adjacent in-flow sibling vertical margins collapse (CSS2.1 8.3.1).
fn q48a_pair(a: &str, b: &str) -> f32 {
    q48_y_of(
        &format!(r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:{a}"></div><div style="height:77px;margin-top:{b}"></div></body></html>"#),
        77.0,
    )
}

#[test]
fn sibling_margins_collapse_positive() {
    assert_eq!(q48a_pair("20px", "30px"), 40.0);
    assert_eq!(q48a_pair("30px", "20px"), 40.0);
}

#[test]
fn sibling_margins_collapse_negative() {
    assert_eq!(q48a_pair("-20px", "-30px"), 10.0 - 30.0);
}

#[test]
fn sibling_margins_collapse_mixed() {
    // 30 + (-10) = 20
    assert_eq!(q48a_pair("30px", "-10px"), 30.0);
    assert_eq!(q48a_pair("-10px", "30px"), 30.0);
}

#[test]
fn sibling_margins_percent_top_collapses() {
    // 10% of 900px = 90 collapses with 20 -> 90
    assert_eq!(q48a_pair("20px", "10%"), 100.0);
}

#[test]
fn float_between_siblings_does_not_break_or_join_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:20px"></div><div style="float:left;width:5px;height:5px;margin:100px"></div><div style="height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
}

#[test]
fn abspos_between_siblings_is_transparent_to_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:20px"></div><div style="position:absolute;height:5px;margin:100px"></div><div style="height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
}

#[test]
fn clearance_prevents_sibling_margin_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:50px"></div><div style="height:10px;margin-bottom:20px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    // Q48d: clearance puts the cleared box's BORDER edge at the float's
    // bottom (50); its margin-top is absorbed, not added on top.
    assert_eq!(y, 50.0);
}

#[test]
fn q48d_clear_without_clearance_still_collapses_and_clear_on_first_child() {
    // Float only 5px tall: the collapsed hypothetical border edge (10 + max(20,30)
    // = 40) is already below it, so no clearance is added.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:5px"></div><div style="height:10px;margin-bottom:20px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
    // First in-flow child cleared past a 50px float with margin-top 30: border edge at 50.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:50px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 50.0);
}

/// QINC1k1 (found by the y-drift fuzz): the clearance test read a `%` margin-top of the not yet finalized
/// cleared box with `px()` (0 in release, a debug assertion in debug). 10 % of 900 = 90: the collapsed
/// hypothetical border edge (10 + max(20, 90) = 100) is below the 50 px float, so no clearance.
#[test]
fn qinc1k1_clearance_resolves_a_percentage_margin_top() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:50px"></div><div style="height:10px;margin-bottom:20px"></div><div style="clear:left;height:77px;margin-top:10%"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 100.0);
}

#[test]
fn q52a_iframe_is_a_replaced_box_default_300x150_honouring_attrs_and_css() {
    fn collect(l: &Laid, out: &mut Vec<(f32, f32)>) {
        out.push((l.w, l.h));
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                collect(k, out);
            }
        }
    }
    let sizes = |html: &str| -> Vec<(f32, f32)> {
        let doc = parse_html(&format!("<html><body style=\"margin:0\">{html}</body></html>"));
        let tree = build_layout_tree(&doc, 1000.0);
        let mut fs = new_font_system_with(&[]);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 1000.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        let mut v = Vec::new();
        collect(&laid, &mut v);
        v
    };
    assert!(sizes("<iframe style=\"display:block\"></iframe>").contains(&(300.0, 150.0)));
    assert!(sizes("<iframe style=\"display:block\" width=\"200\" height=\"80\"></iframe>").contains(&(200.0, 80.0)));
    assert!(sizes("<iframe style=\"display:block;width:120px\" width=\"200\" height=\"80\"></iframe>").contains(&(120.0, 80.0)));
    let v = sizes("<iframe>hello world fallback</iframe>");
    assert!(v.iter().any(|&(_, h)| h == 150.0), "{v:?}");
}

#[test]
fn qa2_nbsp_only_line_has_height() {
    assert!(qa2_body_h("<div>&nbsp;</div>") >= 11.0, "{}", qa2_body_h("<div>&nbsp;</div>"));
    assert!(qa2_body_h("<div>&nbsp;&nbsp;&nbsp;</div>") >= 11.0);
    // A plain space still collapses to nothing.
    assert_eq!(qa2_body_h("<div> </div>"), 0.0);
}

// ---------- QW2: `text-indent` (wikipedia.org search-button label) ----------

fn red_pixel_count_of(html: &str) -> usize {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, 300.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    );
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (300u32, 100u32);
    let mut pixels = vec![0xFFFFFFu32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    pixels.iter().filter(|&&p| ((p >> 16) & 0xFF) > 200 && ((p >> 8) & 0xFF) < 80 && (p & 0xFF) < 80).count()
}

#[test]
fn negative_text_indent_moves_the_label_off_the_icon_qw2() {
    // wikipedia.org: `<button><i class="sprite svg-search-icon">Search</i></button>`
    // with `.svg-search-icon{width:22px;height:22px;display:inline-block}` and
    // `button .svg-search-icon{text-indent:-9999px}`: the label must be hidden
    // off to the left, not spill out of the 22px icon box ("Searc" clipped).
    let page = |indent: &str| {
        format!(
            r#"<html><body><button style="white-space:nowrap;font-weight:700;font-style:italic;color:#f00;padding:8px 16px"><i style="display:inline-block;width:22px;height:22px;vertical-align:middle;{indent}">Search</i></button></body></html>"#
        )
    };
    let without = red_pixel_count_of(&page(""));
    let with = red_pixel_count_of(&page("text-indent:-9999px"));
    assert!(without > 20, "control: the label must paint without text-indent (got {without} red px)");
    assert_eq!(with, 0, "text-indent:-9999px must push the first line off-screen (got {with} red px)");
}

#[test]
fn negative_text_indent_applies_on_the_mixed_inline_path_too_qw2() {
    // wikipedia.org has a float earlier in the BFC, which routes the label
    // through `measure_mixed_inline` (per-word `Word` items) instead of the
    // single-buffer path; the indent must apply there as well.
    let page = |indent: &str| {
        format!(
            r#"<html><body><div style="float:left;width:10px;height:200px"></div><button style="white-space:nowrap;font-weight:700;font-style:italic;color:#f00;padding:8px 16px"><i style="display:inline-block;width:22px;height:22px;vertical-align:middle;{indent}">Search</i></button></body></html>"#
        )
    };
    let without = red_pixel_count_of(&page(""));
    let with = red_pixel_count_of(&page("text-indent:-9999px"));
    assert!(without > 20, "control: the label must paint without text-indent (got {without} red px)");
    assert_eq!(with, 0, "text-indent:-9999px must push the first mixed-inline line off-screen (got {with} red px)");
}

// ---------- Q35b3: HN vote arrow column ----------

fn hn_vote_row_html(arrow_display: &str) -> String {
    format!(
        r#"<style>.votearrow{{width:10px;height:10px;border:0;margin:3px 2px 6px;background:url('grayarrow.gif') no-repeat;{arrow_display}}} td{{font-size:10pt}}</style>
<table border="0" cellpadding="0" cellspacing="0" width="85%"><tr class="athing"><td valign="top" class="votelinks"><center><a href="vote?id=1"><div class="votearrow" title="upvote"></div></a></center></td><td class="title"><span class="titleline"><a href="https://example.com/">Old Bitly Links Are Being Hijacked</a></span></td></tr></table>"#
    )
}

#[test]
fn hn_vote_arrow_div_in_center_anchor_gets_its_own_column_wide_enough() {
    // Real HN: block-level `div.votearrow` (10px + 2px side margins) inside
    // `<center><a>` in a narrow cell; the column must fit its margin box and
    // the title cell must start to its right (Q35b3).
    for d in ["", "display:inline-block"] {
        let laid = measure_fixture(&hn_vote_row_html(d), 900.0);
        let cells = find_table_row_cells(&laid).expect("table row not found");
        assert_eq!(cells.len(), 2);
        assert!(cells[0].w >= 14.0 - 0.01, "vote column {} fits 10px + 2*2px margins ({d})", cells[0].w);
        assert!(cells[1].x >= cells[0].x + 14.0 - 0.01, "title cell x {} starts right of the arrow ({d})", cells[1].x);
    }
}
