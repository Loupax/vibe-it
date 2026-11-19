#[allow(unused_imports)]
use super::*;

/// Searches the *pre-measure* `LayoutBox` tree (not a `Laid`) for a
/// box whose own `position` is `pos` — used only by `absolute_
/// position_computes_display_to_block`, where what's actually being
/// tested is whether a box exists for it at all (see that test's own
/// docs for why that's the right thing to check).
fn find_box_by_position(b: &LayoutBox, pos: Position) -> Option<&LayoutBox> {
    if b.style.position == pos {
        return Some(b);
    }
    match &b.content {
        BoxContent::Blocks(children) | BoxContent::TableRow(children) => {
            for c in children {
                if let Some(found) = find_box_by_position(c, pos) {
                    return Some(found);
                }
            }
        }
        _ => {}
    }
    None
}

#[test]
fn absolute_position_computes_display_to_block() {
    // A default-`display: inline` `<span>` with no block content of
    // its own is never given its own `LayoutBox` at all —
    // `collect_children` recurses straight into it, merging its text
    // into the surrounding run, so there would be *no* box anywhere
    // in the tree carrying its `position: absolute`, if the real
    // CSS "computed `display`" rule (spec §9.7) weren't applied. Its
    // mere existence here is the test.
    let doc = parse_html(r#"<html><body><span style="position: absolute; top: 0; left: 0;">text</span></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let span = find_box_by_position(&tree, Position::Absolute).expect(
        "an absolutely positioned <span> should still get its own box (real CSS computes its display to block, instead of being flattened into the surrounding inline run the way a default `display: inline` span would be)",
    );
    assert_eq!(span.style.display, Display::Block, "position: absolute should force computed display to block");
}

#[test]
fn position_relative_offsets_paint_position_not_sibling_flow() {
    // Deliberately no whitespace between the `<div>`s — a literal
    // newline/indentation between block-level tags becomes its own
    // whitespace-only anonymous inline box, consuming real flow
    // height and defeating the exact-pixel deltas this test checks
    // (same lesson v51's own font-family test already hit).
    let laid = measure_fixture(
        r#"<html><body><div style="height: 20px;">first</div><div style="position: relative; top: 10px; left: 5px; height: 21px;">second</div><div style="height: 22px;">third</div></body></html>"#,
        400.0,
    );
    let first = find_by_height(&laid, 20.0).expect("first div not found");
    let second = find_by_height(&laid, 21.0).expect("second (relative) div not found");
    let third = find_by_height(&laid, 22.0).expect("third div not found");
    let first_x = first.x;
    let first_bottom = first.y + first.h;
    assert_eq!(second.y, first_bottom + 10.0, "a relative `top` offset should shift paint position");
    assert_eq!(second.x, first_x + 5.0, "a relative `left` offset should shift paint position");
    assert_eq!(
        third.y,
        first_bottom + second.h,
        "a later sibling's flow position must be exactly where it would land if the relative box had no offset at all — `position: relative` never affects normal flow"
    );
}

#[test]
fn position_relative_percent_offset_resolves_against_containing_block() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 200px; height: 100px;"><div style="position: relative; top: 10%; left: 50%; height: 20px;">x</div></div></body></html>"#,
        400.0,
    );
    let outer = find_by_width(&laid, 200.0).expect("outer div not found");
    let (outer_x, outer_y) = (outer.x, outer.y);
    let inner = find_by_height(&laid, 20.0).expect("inner (relative) div not found");
    assert_eq!(inner.x, outer_x + 100.0, "left: 50% should resolve against the containing block's own 200px width");
    assert_eq!(inner.y, outer_y + 10.0, "top: 10% should resolve against the containing block's own 100px height");
}

#[test]
fn position_absolute_removed_from_flow_and_anchored_to_positioned_ancestor() {
    // No inter-tag whitespace — same reason as the `relative` test
    // above (a whitespace-only text node would otherwise show up as
    // an extra `Blocks` child, breaking the exact `children.len()`
    // check below).
    let laid = measure_fixture(
        r#"<html><body><div style="position: relative; width: 300px; height: 100px; padding: 10px;"><div style="position: absolute; top: 5px; left: 5px; width: 50px; height: 50px;">abs</div><div style="height: 20px;">normal</div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("positioned container not found");
    let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
    assert_eq!(
        children.len(),
        1,
        "an absolutely positioned child must not occupy a slot in its container's normal flow"
    );
    let normal = &children[0];
    let content_y = container.y + container.style.padding.top.px();
    assert_eq!(
        normal.y, content_y,
        "the sibling after an absolutely positioned box should land exactly where it would if that box didn't exist"
    );

    assert_eq!(container.positioned.len(), 1, "the absolute child should resolve against its nearest positioned ancestor");
    let abs = &container.positioned[0];
    assert_eq!(abs.x, container.x + 5.0, "left resolves against the containing block's own padding edge");
    assert_eq!(abs.y, container.y + 5.0, "top resolves against the containing block's own padding edge");
    assert_eq!((abs.w, abs.h), (50.0, 50.0));
}

#[test]
fn position_absolute_right_bottom_solve_from_box_own_size() {
    // No `left`/`top` at all — only `right`/`bottom`, which need
    // this box's own (just-measured) width/height to solve for its
    // final position (see `resolve_positioned`'s own docs).
    let laid = measure_fixture(
        r#"<html><body><div style="position: relative; width: 300px; height: 100px;">
<div style="position: absolute; right: 10px; bottom: 5px; width: 40px; height: 20px;"></div>
</div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 300.0).expect("positioned container not found");
    let abs = container.positioned.first().expect("absolute child not resolved");
    assert_eq!(abs.x, container.x + 300.0 - 10.0 - 40.0, "right: 10px should solve against this box's own 40px width");
    assert_eq!(abs.y, container.y + 100.0 - 5.0 - 20.0, "bottom: 5px should solve against this box's own 20px height");
}

#[test]
fn position_absolute_with_no_positioned_ancestor_and_fixed_both_bubble_to_document_root() {
    let laid = measure_document_fixture(
        r#"<html><body>
<div style="height: 3000px;"></div>
<div style="position: absolute; top: 5px; left: 5px; width: 10px; height: 10px;"></div>
<div style="position: fixed; top: 5px; left: 5px; width: 10px; height: 10px;"></div>
</body></html>"#,
        400.0,
        600.0,
    );
    assert_eq!(laid.positioned.len(), 2, "both an orphaned absolute and a fixed box should bubble all the way to the document root");
    let fixed_count = laid.positioned.iter().filter(|l| l.fixed).count();
    assert_eq!(fixed_count, 1, "exactly one of the two (the `position: fixed` one) should be flagged for scroll-independent painting");
    for item in &laid.positioned {
        assert_eq!(
            (item.x, item.y),
            (5.0, 5.0),
            "both resolve against the same viewport-sized containing block, regardless of the preceding 3000px-tall sibling"
        );
    }
}

#[test]
fn position_fixed_anchors_to_viewport_not_wherever_it_sits_in_a_tall_document() {
    let laid = measure_document_fixture(
        r#"<html><body>
<div style="height: 3000px;"></div>
<div style="position: fixed; top: 10px; right: 20px; width: 50px; height: 30px;"></div>
</body></html>"#,
        400.0,
        600.0,
    );
    let fixed = laid.positioned.first().expect("fixed box not resolved");
    assert!(fixed.fixed);
    assert_eq!(fixed.y, 10.0, "top anchors against the 600px viewport, not the 3000px document");
    assert_eq!(fixed.x, 400.0 - 20.0 - 50.0, "right anchors against the 400px viewport width");
}

#[test]
fn z_index_controls_paint_order_for_overlapping_absolute_boxes() {
    // The higher-`z-index` box (red) appears *first* in document
    // order but should still paint on top of the lower one (blue,
    // later in markup) wherever they overlap — document order alone
    // would give the opposite (wrong) answer here if `z-index`
    // weren't honored at all (blue painting last/on top).
    let html = r#"<html><body>
<div style="position: absolute; top: 0px; left: 0px; width: 40px; height: 40px; background-color: #ff0000; z-index: 1;"></div>
<div style="position: absolute; top: 10px; left: 10px; width: 40px; height: 40px; background-color: #0000ff; z-index: 0;"></div>
</body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (200u32, 200u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    let px = pixels[20 * win_w as usize + 20];
    assert_eq!(
        px & 0x00FF_FFFF,
        0x00FF_0000,
        "the higher z-index box (red) should paint on top in the overlap region"
    );
}

// ---------- v55: `position: sticky` ----------

/// Searches a laid-out `Laid` tree (not the pre-measure `LayoutBox`
/// tree `find_box_by_position` searches) for a box whose own
/// `position` is `pos` — used by this item's own tests, which all
/// need the real, measured `Laid` (its `.sticky` field, its exact
/// `x`/`y`/`h`), not just whether a box with this position exists.
fn find_laid_by_position(laid: &Laid, pos: Position) -> Option<&Laid> {
    if laid.style.position == pos {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_laid_by_position(c, pos) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn sticky_box_stays_in_normal_flow_and_contributes_to_its_containers_size() {
    // No inter-tag whitespace — same reason every other exact-pixel
    // flow test in this file avoids it (a whitespace-only text node
    // would otherwise become its own `Blocks` child).
    let laid = measure_fixture(
        r#"<html><body><div style="width: 100px;"><div style="height: 20px;"></div><div style="position: sticky; top: 50px; height: 21px;"></div><div style="height: 22px;"></div></div></body></html>"#,
        400.0,
    );
    let container = find_by_width(&laid, 100.0).expect("container not found");
    let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
    assert_eq!(
        children.len(),
        3,
        "a sticky box still occupies a real slot in its parent's normal flow, unlike `absolute`/`fixed` (which drop their slot entirely)"
    );
    let (first, sticky_box, third) = (&children[0], &children[1], &children[2]);
    assert_eq!(sticky_box.style.position, Position::Sticky);
    assert_eq!(
        sticky_box.y,
        first.y + first.h,
        "a sticky box's own layout-time position is exactly its ordinary static position — unlike `relative`, its `top`/`bottom` offset is never baked into `Laid::y` at all, only applied at paint time"
    );
    assert_eq!(
        third.y,
        sticky_box.y + sticky_box.h,
        "the sibling after a sticky box must land exactly where it would if the sticky box had no offset at all — `position: sticky` never affects normal flow, same as `relative`"
    );
    assert_eq!(
        container.h,
        first.h + sticky_box.h + third.h,
        "the sticky box's own height is fully counted toward its container's auto height, unlike an out-of-flow `absolute`/`fixed` box"
    );

    let info = sticky_box.sticky.expect("position: sticky should capture a StickyInfo at layout time");
    assert_eq!(info.static_top, sticky_box.y);
    assert_eq!(info.top, Some(50.0), "top: 50px should resolve against the containing block's own height basis, same timing `relative`'s own offsets use");
    assert_eq!(info.bottom, None);
}

/// Shared fixture for the scroll-dependent tests below: a 100px-wide
/// column with a 200px spacer, the sticky box itself (30px tall,
/// `top: 10px`, painted red so a pixel-level check can confirm it),
/// and a 1000px spacer after it — enough room for the box to clear
/// its sticking threshold long before its containing block's own far
/// edge would catch up to it, so each test below can pick a scroll
/// offset that isolates exactly one of the three states (`static`,
/// `stuck`, `stopped sticking`) without the others interfering.
fn sticky_scroll_fixture() -> Laid {
    let html = r#"<html><body><div style="width: 100px;"><div style="height: 200px;"></div><div style="position: sticky; top: 10px; height: 30px; background-color: #ff0000;"></div><div style="height: 1000px;"></div></div></body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure(
        &tree, &mut font_system, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    )
}

#[test]
fn sticky_box_paints_at_its_static_position_before_scroll_reaches_the_threshold() {
    let laid = sticky_scroll_fixture();
    let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
    let (static_top, box_h) = (sticky_box.y, sticky_box.h);
    let info = sticky_box.sticky.expect("sticky info not captured");

    assert_eq!(
        sticky_shift(&info, box_h, 0.0, 600.0),
        0.0,
        "with no scroll at all, the box hasn't reached its sticking threshold yet, so it should paint with zero offset from its own static position"
    );

    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (win_w, win_h) = (400u32, (static_top + box_h + 50.0) as u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    let probe_y = (static_top + box_h / 2.0) as usize;
    let px = pixels[probe_y * win_w as usize + 10];
    assert_eq!(
        px & 0x00FF_FFFF,
        0x00FF_0000,
        "with no scroll yet, the sticky box should paint at its own static document-space position, exactly like an ordinary (or `relative`) box would"
    );
}

#[test]
fn sticky_box_clamps_to_its_top_offset_once_scroll_passes_the_threshold() {
    let laid = sticky_scroll_fixture();
    let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
    let (static_top, box_h) = (sticky_box.y, sticky_box.h);
    let info = sticky_box.sticky.expect("sticky info not captured");

    // Scrolled well past the point `top: 10px` should have kicked
    // in, but nowhere near the container's own far edge (1000px of
    // slack below) — squarely in the "stuck" range.
    let scroll_offset = static_top + 200.0;
    let dy = sticky_shift(&info, box_h, scroll_offset, 600.0);
    assert_eq!(
        dy,
        scroll_offset + 10.0 - static_top,
        "once stuck, the box should track the live scroll offset exactly: its document-space position is always `scroll_offset + top`"
    );

    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (win_w, win_h) = (400u32, 100u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, scroll_offset, None);
    // Screen-space row = document y (`scroll_offset + 10`) minus
    // `scroll_offset` = exactly `10` — the box should now sit flush
    // `top`'s own 10px from the window's own top edge, nowhere near
    // the (off-screen, were it not stuck) row its static position
    // alone would have put it at.
    let probe_y = (10.0 + box_h / 2.0) as usize;
    let px = pixels[probe_y * win_w as usize + 10];
    assert_eq!(
        px & 0x00FF_FFFF,
        0x00FF_0000,
        "once stuck, the box should paint `top`'s own 10px from the window's top edge, not wherever its static position would otherwise have scrolled to"
    );
}

#[test]
fn sticky_box_stops_sticking_once_its_containing_blocks_far_edge_would_scroll_past() {
    let laid = sticky_scroll_fixture();
    let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
    let (static_top, box_h) = (sticky_box.y, sticky_box.h);
    let info = sticky_box.sticky.expect("sticky info not captured");
    let max_top = info.cb_bottom - box_h;
    assert!(
        max_top > static_top,
        "the fixture's own 1000px trailing spacer should leave real slack between the box's static position and its containing block's far edge"
    );

    // Exactly at the boundary (`scroll_offset + top == max_top`) and
    // far past it — both should land the box at the *same* final
    // offset, never escaping further past its own containing
    // block's bottom edge no matter how much further the page
    // scrolls.
    let dy_at_boundary = sticky_shift(&info, box_h, max_top - 10.0, 600.0);
    let dy_way_past = sticky_shift(&info, box_h, max_top + 5000.0, 600.0);
    let expected = max_top - static_top;
    assert_eq!(dy_at_boundary, expected, "right at the containing block's far edge, the box should already be clamped there");
    assert_eq!(
        dy_way_past, expected,
        "scrolling far past that point must not push the box any further — it stops sticking and scrolls away with the rest of its container instead of escaping past its own parent"
    );
}

#[test]
fn scrolling_only_changes_the_painted_position_never_the_laid_tree() {
    // `paint` takes `&Laid`, never `&mut Laid` — so calling it twice
    // against the very same tree with two very different scroll
    // offsets already *cannot* mutate layout, a compile-time
    // guarantee, not just a runtime one (this is the whole
    // architectural point of resolving `position: sticky` the way
    // this file does — see NOTES.md's v4/v5 writeup on why
    // scrolling is deliberately cheap). This test pins that down
    // concretely: snapshot the sticky box's own layout-time fields,
    // paint twice at very different scroll offsets, and confirm (a)
    // those fields are still exactly what they were (no relayout
    // happened — the same `Laid` tree, not a rebuilt one) while (b)
    // the two painted results genuinely differ (the box visibly
    // moved) — scroll changes the picture, never the layout.
    let laid = sticky_scroll_fixture();
    let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
    let (static_top, static_x, box_h, box_w) = (sticky_box.y, sticky_box.x, sticky_box.h, sticky_box.w);
    let info_before = sticky_box.sticky.expect("sticky info not captured");

    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (win_w, win_h) = (400u32, 300u32);
    let mut pixels_a = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_a, win_w, win_h, 0.0, None);
    let mut pixels_b = vec![0u32; (win_w * win_h) as usize];
    paint(
        &laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_b, win_w, win_h, static_top + 200.0,
        None,
    );

    let sticky_box_after = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found after painting");
    assert_eq!(
        (sticky_box_after.x, sticky_box_after.y, sticky_box_after.w, sticky_box_after.h),
        (static_x, static_top, box_w, box_h),
        "the sticky box's own layout-time geometry must be byte-identical after painting at two different scroll offsets"
    );
    let info_after = sticky_box_after.sticky.expect("sticky info missing after painting");
    assert_eq!(
        (info_after.static_top, info_after.top, info_after.bottom, info_after.cb_top, info_after.cb_bottom),
        (info_before.static_top, info_before.top, info_before.bottom, info_before.cb_top, info_before.cb_bottom),
        "StickyInfo itself must also be untouched by painting — nothing about scroll ever re-resolves it"
    );

    assert_ne!(
        pixels_a, pixels_b,
        "the two different scroll offsets should visibly move the sticky box on screen, even though the underlying Laid tree never changed at all"
    );
}

/// `measure_document` + `paint` at `scroll_offset` in a 200x200 window.
fn paint_doc_at(html: &str, scroll_offset: f32) -> (Vec<u32>, u32, u32) {
    let doc = parse_html(html);
    let tree = build_layout_tree_sized(&doc, 1920.0, 200.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (200u32, 200u32);
    let mut pixels = vec![0xFFFF_FFFFu32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, scroll_offset, None);
    (pixels, win_w, win_h)
}

const TALL_100PCT_PAGE: &str = r#"<html><head><style>
    html, body { height: 100%; margin: 0 }
    .blk { height: 200px; background: #ff0000 }
</style></head><body>
    <div class="blk"></div><div class="blk"></div><div class="blk"></div>
    <div class="blk"></div><div class="blk"></div><div class="blk"></div>
</body></html>"#;

#[test]
fn paint_past_first_viewport_still_draws_with_height_100_percent_body() {
    // R1: `html,body{height:100%}` makes <body> exactly one viewport
    // tall while its content overflows; culling on the border box
    // used to skip the whole body once scrolled past it.
    let (pixels, w, _) = paint_doc_at(TALL_100PCT_PAGE, 500.0);
    assert_eq!(px_at(&pixels, w, 100, 100), 0xFF0000, "overflowing content below the first viewport must paint");
}

#[test]
fn hit_test_finds_link_below_first_viewport_with_height_100_percent_body() {
    let html = r#"<html><head><style>html, body { height: 100%; margin: 0 }
        .pad { height: 500px }</style></head><body>
        <div class="pad"></div><p><a href="/deep">deep link</a></p></body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree_sized(&doc, 1920.0, 200.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 400.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let (lx, ly) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
    assert!(ly > 400.0, "fixture must put the link well past the first viewport, got y={ly}");
    let so = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, lx, ly, &cache, &so, 0.0, 200.0).1, Some("/deep"));
}

#[test]
fn zero_height_list_with_visible_children_still_paints_and_hit_tests() {
    let html = r#"<html><head><style>html, body { margin: 0 }
        ul { height: 0; margin: 0; padding: 0; list-style: none }
        li { height: 50px; background: #00ff00 }
        .pad { height: 400px }</style></head><body>
        <div class="pad"></div><ul><li></li><li></li></ul><div class="pad"></div></body></html>"#;
    let (pixels, w, _) = paint_doc_at(html, 420.0);
    assert_eq!(px_at(&pixels, w, 100, 10), 0x00FF00, "li at y=400..450 should be visible at scroll 420 although its h=0 ul is above the viewport");
}

// -- R2.2 / R3.1 / R3.3 / R4 --

#[test]
fn viewport_height_flag_only_set_when_layout_can_depend_on_it() {
    let build = |html: &str| {
        let doc = parse_html(html);
        build_layout_tree_sized(&doc, 800.0, 600.0);
        viewport_height_read()
    };
    assert!(!build("<html><body><p>plain</p><div style='height:40px'>x</div></body></html>"));
    assert!(build("<html><head><style>html,body{height:100%}</style></head><body><p>x</p></body></html>"));
    assert!(build("<html><body><div style='height:50vh'>x</div></body></html>"));
    assert!(build("<html><body><div style='position:fixed;top:0'>x</div></body></html>"));
    assert!(!build("<html><body><div style='width:50vw'>x</div></body></html>"));
}

#[test]
fn shape_memo_reuses_unchanged_paragraphs_across_trees() {
    SHAPE_MEMO.with(|m| m.borrow_mut().clear());
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let html = "<html><body><p>First paragraph</p><p>Second <b>one</b> here</p></body></html>";
    // Node ids are DOM addresses: a restyle re-shapes the *same* document.
    let shape = |doc: &Handle| {
        let tree = build_layout_tree(doc, 400.0);
        let mut fs = new_font_system();
        let mut cache = ShapeCache::new();
        let (mut next_id, mut inc) = (0usize, false);
        let dl = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure_document(&tree, &mut fs, 400.0, 300.0, &mut next_id, &mut cache, dl, &mut inc);
        cache
    };
    let doc = parse_html(html);
    let first = shape(&doc);
    let second = shape(&doc);
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    assert!(!first.is_empty());
    let shaped: Vec<_> = first.iter().filter(|(_, e)| e.lock().unwrap().1 > 0.0).collect();
    assert!(!shaped.is_empty());
    for (id, entry) in shaped {
        assert!(Arc::ptr_eq(entry, &second[id]), "paragraph {id} should come from the memo, not be reshaped");
    }
    // A changed paragraph is not served stale.
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let doc2 = parse_html("<html><body><p>First paragraph</p><p>Changed text</p></body></html>");
    let third = shape(&doc2);
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    let changed = first.iter().filter(|(id, e)| !Arc::ptr_eq(e, &third[id])).count();
    assert!(changed >= 1, "paragraphs of a different document must be reshaped");
}

#[test]
fn rule_index_returns_the_same_matches_as_a_full_scan() {
    let css = "p{color:red} .a{color:blue} #x{color:green} div p{margin:0} *{padding:0} \
               a:hover{color:red} .a.b, span{color:pink} p::before{content:'x'} li:nth-child(2){color:red}";
    let doc = parse_html(
        "<html><body><div id=x class='a b'><p class=a>t<span>s</span></p><ul><li>1<li>2</ul><a href=#>l</a></div></body></html>",
    );
    let rules = parse_stylesheet(css);
    let mut elements = Vec::new();
    fn collect(h: &Handle, out: &mut Vec<Handle>) {
        if matches!(h.data, NodeData::Element { .. }) {
            out.push(h.clone());
        }
        for c in h.children.borrow().iter() {
            collect(c, out);
        }
    }
    collect(&doc, &mut elements);
    let summarize = |h: &Handle, pseudo: Option<&str>| -> Vec<u64> {
        matching_declarations_for(h, &rules, 800.0, pseudo).iter().map(|(sp, _)| *sp).collect()
    };
    let full: Vec<_> = elements.iter().map(|h| (summarize(h, None), summarize(h, Some("before")))).collect();
    RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), RuleIndex::build(&rules))));
    let indexed: Vec<_> = elements.iter().map(|h| (summarize(h, None), summarize(h, Some("before")))).collect();
    RULE_INDEX.with(|ri| *ri.borrow_mut() = None);
    assert_eq!(full, indexed);
    assert!(full.iter().any(|(n, _)| !n.is_empty()));
}

#[test]
fn cached_stylesheet_still_reports_state_rules() {
    let doc = parse_html("<html><head><style>a:hover{color:red}</style></head><body><a href=#>x</a></body></html>");
    build_layout_tree_sized(&doc, 800.0, 600.0);
    assert_eq!(state_rules_mask() & STATE_HOVER, STATE_HOVER);
    // Second build hits the parsed-rules cache; the mask must survive.
    build_layout_tree_sized(&doc, 800.0, 600.0);
    assert_eq!(state_rules_mask() & STATE_HOVER, STATE_HOVER);
}

#[test]
fn hover_keys_cover_exactly_the_elements_a_hover_rule_can_restyle() {
    let doc = parse_html("<html><body><a href=#>l</a><p class=btn>b</p><div id=d>d</div><p>plain</p></body></html>");
    let find = |tag: &str, class: Option<&str>| -> Handle {
        fn walk(h: &Handle, tag: &str, class: Option<&str>) -> Option<Handle> {
            if let NodeData::Element { name, .. } = &h.data
                && &*name.local == tag
                && class.is_none_or(|c| element_attr(h, "class").is_some_and(|v| &*v == c))
            {
                return Some(h.clone());
            }
            h.children.borrow().iter().find_map(|c| walk(c, tag, class))
        }
        walk(&doc, tag, class).unwrap()
    };
    let keys = HoverKeys::build(&parse_stylesheet("a:hover{color:red} .btn:hover > span{color:blue}"));
    assert!(!keys.any);
    assert!(keys.may_match(&find("a", None)));
    assert!(keys.may_match(&find("p", Some("btn"))));
    assert!(!keys.may_match(&find("div", None)));
    // `:hover` with no id/class/tag requirement, or nested, matches anything.
    assert!(HoverKeys::build(&parse_stylesheet(":hover{color:red}")).any);
    assert!(HoverKeys::build(&parse_stylesheet("div:not(:hover){color:red}")).any);
    assert!(!HoverKeys::build(&parse_stylesheet("div{color:red}")).any);
}

#[test]
fn negative_z_index_paints_above_its_containing_boxs_background_but_below_normal_content() {
    // CSS2.1 Appendix E's own bottommost-first order
    // (https://www.w3.org/TR/CSS21/zindex.html) is: 1 (this box's own
    // background/border), 2 (negative-`z-index` descendant stacking
    // contexts), 3-5 (normal in-flow content) — i.e. negative
    // `z-index` paints *above* this box's own background (it is only
    // ever hidden behind *normal content*, layers 3+, never behind
    // the background fill of the box that establishes its stacking
    // context). Confirmed against the spec directly — the intuitive-
    // sounding "negative z-index hides behind its own box's
    // background" is a common but *inaccurate* paraphrase of the
    // real rule (it only looks that way when the relevant box has no
    // background of its own, so there's nothing *else* for the
    // negative descendant to appear in front of). Two assertions:
    // red (the `z-index: -1` child) must beat green (the container's
    // own background) where only those two overlap, and lose to blue
    // (an ordinary static sibling, layer 3) wherever blue also
    // covers the same pixel.
    //
    // No inter-tag whitespace — a literal newline/indentation between
    // these block-level tags would become its own whitespace-only
    // anonymous inline box, consuming real flow height and silently
    // shifting every box below it (the exact v51/v52-documented
    // pitfall; caught here by a debug dump during development, not
    // assumed away).
    let html = r#"<html><body style="margin:0"><div style="position: relative; z-index: 0; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    assert_eq!(
        px_at(&pixels, win_w, 50, 80),
        0x00FF_0000,
        "the z-index:-1 child should paint above the container's own green background"
    );
    assert_eq!(
        px_at(&pixels, win_w, 50, 20),
        0x0000_00FF,
        "the z-index:-1 child should still paint below an ordinary (layer 3) static sibling"
    );
}

#[test]
fn seven_layer_stacking_order_within_one_containing_block() {
    // One `position: relative` container with three children spanning
    // four of CSS2.1 Appendix E's seven layers: its own background
    // (layer 1, green, full 100x100), a `z-index: -1` absolute child
    // (layer 2, red, full 100x100 — above the background, below
    // everything else), an ordinary static in-flow child (layer 3,
    // blue, top 100x50 half — above the negative-z-index child, same
    // as any other non-positioned in-flow content), and a `z-index: 5`
    // absolute child (layer 7, yellow, top-left 30x30 — above
    // everything, including the static child). Document order is
    // deliberately *not* the paint order here (red is declared first,
    // yellow last, but neither is topmost solely from parsing order) —
    // z-index/layer membership alone must decide it.
    // No inter-tag whitespace — see the test above's own note.
    let html = r#"<html><body style="margin:0"><div style="position: relative; z-index: 0; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div><div style="position: absolute; top: 0px; left: 0px; width: 30px; height: 30px; background-color: #ffff00; z-index: 5;"></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    // Outside the static child's 0-50 range and the small yellow
    // child's 0-30 range: only green (layer 1) and red (layer 2)
    // cover this pixel — red must win (layer 2 above layer 1).
    assert_eq!(px_at(&pixels, win_w, 80, 80), 0x00FF_0000, "layer 2 (negative z-index) should paint above layer 1 (this box's own background)");
    // Inside both the static child's range and the small yellow
    // child's range: yellow (layer 7) must win over everything,
    // including the in-flow static child (layer 3).
    assert_eq!(px_at(&pixels, win_w, 10, 10), 0x00FF_FF00, "layer 7 (positive z-index) should paint above layer 3 (ordinary in-flow content)");
    // Inside the static child's range but outside the small yellow
    // child's range: blue (layer 3) must win over red (layer 2).
    assert_eq!(px_at(&pixels, win_w, 10, 40), 0x0000_00FF, "layer 3 (ordinary in-flow content) should paint above layer 2 (negative z-index)");
}

#[test]
fn relative_box_with_explicit_z_index_establishes_its_own_stacking_context() {
    // A `position: relative; z-index: 1` box stays in normal flow
    // (contributes to its container's size, never displaces a
    // sibling — unchanged from `relative`'s own pre-existing
    // behavior) but, with an *explicit* z-index, now establishes its
    // own stacking context: real CSS2 §9.9.1. Shifted down by `top:
    // 30px` to genuinely overlap a later, ordinary static sibling
    // (declared *after* it, and which would win a plain document-
    // order paint) — the explicit z-index must still win.
    // No inter-tag whitespace — see the first test above's own note.
    let html = r#"<html><body style="margin:0"><div style="position: relative; top: 30px; left: 0px; width: 50px; height: 50px; background-color: #ff0000; z-index: 1;"></div><div style="width: 50px; height: 50px; background-color: #0000ff;"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    // Overlap region: the shifted red box covers screen y 30-80; the
    // ordinary blue sibling (document order: right after red's own
    // *unshifted* 0-50 flow slot) covers y 50-100. Sampled at y=60,
    // inside both.
    assert_eq!(
        px_at(&pixels, win_w, 10, 60),
        0x00FF_0000,
        "the relative box's own explicit z-index should establish a stacking context above its later static sibling"
    );
}

#[test]
fn relative_box_with_auto_z_index_does_not_establish_a_stacking_context() {
    // Exact same fixture as the test above, minus the explicit
    // `z-index` on the shifted relative box — real CSS: `z-index:
    // auto` does *not* establish a new stacking context, so this
    // box keeps today's plain paint-position-only treatment and the
    // later static sibling (document order) still wins the overlap,
    // same as before this session's feature existed at all. This is
    // the regression pin for that "same as before" claim.
    // No inter-tag whitespace — see the first test in this section.
    let html = r#"<html><body style="margin:0"><div style="position: relative; top: 30px; left: 0px; width: 50px; height: 50px; background-color: #ff0000;"></div><div style="width: 50px; height: 50px; background-color: #0000ff;"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    // QY1: the old expectation here (blue, the later static sibling, wins) was
    // wrong per CSS2.1 Appendix E: a positioned `z-index:auto` box paints in
    // layer 6, above all in-flow non-positioned content (Chrome shows red).
    // It still does not establish a stacking context (see the next test).
    assert_eq!(
        px_at(&pixels, win_w, 10, 60),
        0x00FF_0000,
        "z-index:auto relative box paints above later non-positioned in-flow content (layer 6)"
    );
}

fn qp4_key(span: SpanStyle) -> Option<u64> {
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let k = memo_key(0, &[InlineItem::Text("abc".into(), span)], 100.0, false, None);
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    k
}

#[test]
fn qp4_memo_key_equal_spans_equal_key_and_each_field_changes_it() {
    let base = SpanStyle::from(ROOT_STYLE);
    let k0 = qp4_key(base).expect("key");
    assert_eq!(Some(k0), qp4_key(base));
    // One mutation per SpanStyle field; `hash_into` destructures without `..`,
    // so a new field fails to compile there, and this list must grow with it.
    type Mutation = (&'static str, Box<dyn Fn(&mut SpanStyle)>);
    let muts: Vec<Mutation> = vec![
        ("color", Box::new(|s| s.color = TextColor(s.color.0 ^ 0x00ff00))),
        ("background_color", Box::new(|s| s.background_color = Some(TextColor(0x123456)))),
        ("font_size", Box::new(|s| s.font_size += 1.0)),
        ("font_weight", Box::new(|s| s.font_weight = FontWeight(s.font_weight.0 + 100))),
        ("font_style", Box::new(|s| s.font_style = FontStyle::Italic)),
        ("font_family", Box::new(|s| s.font_family = FontFamily(&[FontFamilyItem::Monospace]))),
        ("text_decoration", Box::new(|s| s.text_decoration.strikethrough = true)),
        ("decoration_style", Box::new(|s| s.decoration_style = DecorationStyle::Dashed)),
        ("vertical_align", Box::new(|s| s.vertical_align = VerticalAlign::Super)),
        ("line_height", Box::new(|s| s.line_height += 1.0)),
        ("visibility", Box::new(|s| s.visibility = Visibility::Hidden)),
        ("cursor", Box::new(|s| s.cursor = CursorStyle::Pointer)),
        ("word_spacing", Box::new(|s| s.word_spacing += 1.0)),
        ("word_break", Box::new(|s| s.word_break = WordBreak::BreakAll)),
        ("white_space", Box::new(|s| s.white_space = WhiteSpace::Pre)),
        ("href", Box::new(|s| s.href = Some("https://x.test/"))),
        ("node", Box::new(|s| s.node = 77)),
        ("tnode", Box::new(|s| s.tnode = 78)),
    ];
    for (name, m) in &muts {
        let mut s = base;
        m(&mut s);
        assert_ne!(Some(k0), qp4_key(s), "changing {name} must change the memo key");
    }
    // Alignment is part of the key too.
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let items = [InlineItem::Text("abc".into(), base)];
    let a = memo_key(0, &items, 100.0, false, None);
    let b = memo_key(0, &items, 100.0, false, Some(CosmicAlign::Center));
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    assert_ne!(a, b);
}

#[test]
fn qy53o3b_memo_key_text_matches_item_key_without_string_copy() {
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let base = SpanStyle::from(ROOT_STYLE);
    for kind in [1u8, 2u8] {
        let via_items = memo_key(kind, &[InlineItem::Text("hello world".into(), base)], 1000.0, true, None);
        assert!(via_items.is_some());
        assert_eq!(via_items, memo_key_text(kind, "hello world", &base, 1000.0), "borrowed-parts key equals the item key");
        assert_ne!(via_items, memo_key_text(kind, "hello worle", &base, 1000.0), "text change misses");
        let mut s = base;
        s.node = 5;
        assert_ne!(via_items, memo_key_text(kind, "hello world", &s, 1000.0), "span change misses");
    }
    // Short/odd-length texts must not collide.
    let a = memo_key_text(1, "ab", &base, 10.0);
    let b = memo_key_text(1, "ab\0", &base, 10.0);
    let c = memo_key_text(1, "a", &base, 10.0);
    assert!(a != b && a != c && b != c);
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
}
