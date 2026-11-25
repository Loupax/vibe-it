#[allow(unused_imports)]
use super::*;

#[test]
fn row_direction_lays_items_out_left_to_right() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 300px;"><div style="width: 50px;">a</div><div style="width: 80px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items.len(), 2, "both children should become flex items");
    assert!(items[0].x < items[1].x, "row direction: items should lay out left to right");
    assert_eq!(items[1].x, items[0].x + items[0].w, "second item should start exactly where the first ends (no gap/grow)");
    assert_eq!(items[0].y, items[1].y, "both items should start at the same cross-axis (y) position");
}

#[test]
fn column_direction_lays_items_out_top_to_bottom() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; flex-direction: column; width: 300px;"><div style="height: 40px;">a</div><div style="height: 60px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items.len(), 2);
    assert!(items[0].y < items[1].y, "column direction: items should lay out top to bottom");
    assert_eq!(items[1].y, items[0].y + items[0].h, "second item should start exactly where the first ends");
    assert_eq!(items[0].x, items[1].x, "both items should share the same cross-axis (x) position");
}

#[test]
fn justify_content_space_between_pushes_items_to_opposite_edges() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: flex; justify-content: space-between; width: 300px;"><div style="width: 50px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].x, 0.0, "first item should sit at the container's own left edge");
    assert_eq!(items[1].x, 250.0, "second item's right edge (250+50) should sit exactly at the container's right edge (300)");
}

#[test]
fn align_items_stretch_grows_the_shorter_sibling_to_match_the_tallest() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 300px;"><div style="width: 50px; height: 100px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].h, 100.0, "the explicitly-tall item keeps its own height");
    assert_eq!(items[1].h, 100.0, "align-items: stretch (the default) should grow the shorter, auto-height sibling to match the line's cross size");
}

#[test]
fn align_items_center_positions_without_resizing() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; align-items: center; width: 300px;"><div style="width: 50px; height: 100px;">a</div><div style="width: 50px; height: 20px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[1].h, 20.0, "align-items: center should not stretch the shorter item's own height");
    // Compared *relative* to the taller sibling (which, at the
    // line's own full 100px cross size, centers to no offset at
    // all) rather than against an absolute `y`, so this doesn't
    // depend on `body`'s own default margin shifting everything's
    // absolute document-space position uniformly.
    assert_eq!(
        items[1].y - items[0].y,
        (100.0 - 20.0) / 2.0,
        "the shorter item should be vertically centered within the line's cross size (100px)"
    );
}

#[test]
fn q23b_align_content_distributes_wrapped_lines() {
    let html = |ac: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display: flex; flex-wrap: wrap; width: 100px; height: 200px; align-items: flex-start; {ac}"><div style="width: 100px; height: 20px;">a</div><div style="width: 100px; height: 20px;">b</div></div></body></html>"#
        )
    };
    let ys = |ac: &str| -> Vec<f32> {
        let laid = measure_fixture(&html(ac), 500.0);
        flex_items(&laid, 100.0).iter().map(|i| i.y).collect()
    };
    let base = ys("align-content: flex-start;");
    assert_eq!(base[1] - base[0], 20.0);
    let center = ys("align-content: center;");
    assert_eq!(center[0] - base[0], 80.0, "(200-40)/2 of free space above the lines");
    let end = ys("align-content: flex-end;");
    assert_eq!(end[0] - base[0], 160.0);
    let between = ys("align-content: space-between;");
    assert_eq!(between[0] - base[0], 0.0);
    assert_eq!(between[1] - between[0], 180.0);
    let stretch = ys("");
    assert_eq!(stretch[1] - stretch[0], 100.0, "default stretch: each line grows by 160/2");
}

#[test]
fn q23_order_reorders_flex_items_stably() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 300px;"><div style="width: 10px; order: 2;">a</div><div style="width: 20px;">b</div><div style="width: 30px; order: -1;">c</div><div style="width: 40px;">d</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    let ws: Vec<f32> = items.iter().map(|i| i.w).collect();
    assert_eq!(ws, vec![30.0, 20.0, 40.0, 10.0], "order -1, then the two order-0 items in source order, then order 2");
    assert!(items[0].x < items[1].x && items[2].x < items[3].x);
}

#[test]
fn flex_grow_distributes_free_space_proportionally() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 300px;"><div style="flex: 1 0 0;">a</div><div style="flex: 2 0 0;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].w, 100.0, "flex-grow 1 of (1+2) should claim 1/3 of the 300px free space");
    assert_eq!(items[1].w, 200.0, "flex-grow 2 of (1+2) should claim 2/3 of the 300px free space — not just 'fit content'");
}

#[test]
fn flex_shrink_distributes_deficit_proportionally_to_scaled_factor() {
    // Uses `flex-basis`, not `width`, for each item's own 100px basis
    // — an item's own explicit CSS `width` wins over a flex-shrunk
    // size in this engine (a documented, accepted simplification,
    // same category `measure_table`'s own column-width-vs-cell-width
    // gap already has — see `measure_flex_row`'s own docs), so using
    // `width` here would make every item keep its own literal width
    // regardless of `flex-shrink`, defeating the point of this test.
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 100px;"><div style="flex-basis: 100px; flex-shrink: 1;">a</div><div style="flex-basis: 100px; flex-shrink: 3;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 100.0);
    // Total natural width 200px into a 100px container: a 100px
    // deficit, split by *scaled* shrink factor (`flex_shrink * basis`,
    // CSS Flexbox §9.7) — both items share the same 100px basis, so
    // the split is exactly proportional to their shrink factors alone
    // (1:3): item a loses 25px (100 * 1/4), item b loses 75px (100 * 3/4).
    assert_eq!(items[0].w, 75.0, "flex-shrink 1 (of a 1:3 scaled ratio) should lose proportionally less width");
    assert_eq!(items[1].w, 25.0, "flex-shrink 3 (of a 1:3 scaled ratio) should lose proportionally more width — not shrink equally");
}

#[test]
fn flex_item_with_nowrap_text_never_shrinks_below_its_own_unbreakable_content_width() {
    // Two auto-width, nowrap-text flex items, together far wider
    // than the 40px container. Real CSS Flexbox's own "automatic
    // minimum size" (§4.5) means an auto-width item with no
    // `min-width` still can't shrink past its own content-based
    // minimum; getting this wrong (defaulting the automatic minimum
    // to 0, as this engine used to) lets the shrink phase squeeze
    // each item narrower than its own unbreakable nowrap text,
    // overflowing it past the item's own box into its neighbor —
    // found live on a real Wikipedia page's nav tabs (Article/Talk,
    // Read/Edit/View history). If the fix works, both items keep
    // their full natural width and overflow the container instead
    // (real CSS behavior for content that can't be made to fit).
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; width: 40px;"><div style="white-space: nowrap;">Article</div><div style="white-space: nowrap;">Talk</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 40.0);
    assert!(
        items[0].w + items[1].w > 40.0,
        "both nowrap items together should overflow the 40px container, not be squeezed down to fit it (got {} + {})",
        items[0].w,
        items[1].w
    );
    assert!(items[1].x >= items[0].x + items[0].w, "the second item should start no earlier than where the first item's own (unshrunk) box ends");
}

#[test]
fn flex_wrap_starts_a_second_line_once_items_no_longer_fit() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; flex-wrap: wrap; width: 100px;"><div style="width: 60px; height: 20px;">a</div><div style="width: 60px; height: 20px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 100.0);
    assert_eq!(items[0].x, items[1].x, "both items should be left-aligned on their own line (each is alone on its own line)");
    assert!(items[1].y > items[0].y, "the second item (60+60 > 100px container) should wrap onto a new, lower line");
    assert_eq!(items[1].y, items[0].y + items[0].h, "the second line should start exactly where the first line's cross size ends (no row-gap declared)");
}

#[test]
fn gap_adds_fixed_spacing_between_items_on_top_of_justify_content() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; gap: 20px; width: 300px;"><div style="width: 50px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[1].x, items[0].x + items[0].w + 20.0, "column-gap (from the `gap` shorthand) should add exactly 20px between items, beyond their own widths");
}

#[test]
fn position_absolute_child_of_a_flex_container_is_not_treated_as_a_flex_item() {
    let laid = measure_fixture(
        // `position: relative` on the container itself so it's a
        // real "positioned ancestor" and actually catches its own
        // `absolute` child locally (`resolve_or_bubble_positioned`)
        // — otherwise (a plain `static` flex container, like every
        // other test in this section) it would correctly keep
        // bubbling past it, same as any other `Blocks` box; this
        // test is about the flex *item-list* split, not about
        // `position`'s own containing-block search.
        r#"<html><body style="margin: 0"><div style="display: flex; position: relative; width: 300px;"><div style="position: absolute; top: 5px; left: 5px; width: 20px; height: 20px;">p</div><div style="width: 50px;">a</div></div></body></html>"#,
        500.0,
    );
    let container = find_by_width(&laid, 300.0).expect("flex container not found");
    let LaidContent::Blocks(children) = &container.content else {
        panic!("expected Blocks content");
    };
    assert_eq!(children.len(), 1, "the absolutely-positioned child should be pulled out of the flex item list entirely");
    assert_eq!(children[0].x, 0.0, "the one real flex item should start at the container's left edge, not pushed aside by the out-of-flow child");
    assert_eq!(container.positioned.len(), 1, "the absolutely-positioned child should instead land in the container's own `positioned` list");
}

// ---------- Q23c: flex leftovers ----------

#[test]
fn q23c_flexible_lengths_freeze_clamped_items_and_redistribute() {
    let info = |basis: f32, max: f32| FlexBasisInfo { basis, grow: 1.0, shrink: 1.0, min: 0.0, max };
    // Three items of basis 20 in 300px: an even split is 100 each, but
    // the first is capped at 50, so the other two share the rest (125).
    let sizes = resolve_flex_main_sizes(300.0, 0.0, &[info(20.0, 50.0), info(20.0, f32::INFINITY), info(20.0, f32::INFINITY)]);
    assert_eq!(sizes, vec![50.0, 125.0, 125.0]);
    // Shrinking: one item floored at its basis (min) pushes the whole
    // deficit onto the other.
    let shrink = |basis: f32, min: f32| FlexBasisInfo { basis, grow: 0.0, shrink: 1.0, min, max: f32::INFINITY };
    let sizes = resolve_flex_main_sizes(100.0, 0.0, &[shrink(100.0, 80.0), shrink(100.0, 0.0)]);
    assert_eq!(sizes, vec![80.0, 20.0]);
}

#[test]
fn q23c_column_items_do_not_shrink_below_their_content_height() {
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; flex-direction: column; width: 300px; height: 60px;"><div><div style="height: 40px;"></div></div><div><div style="height: 40px;"></div></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].h, 40.0, "automatic minimum size: an overflow:visible column item keeps its content height");
    assert_eq!(items[1].h, 40.0);
    let laid = measure_fixture(
        r#"<html><body><div style="display: flex; flex-direction: column; width: 300px; height: 60px;"><div style="overflow: hidden"><div style="height: 40px;"></div></div><div style="overflow: hidden"><div style="height: 40px;"></div></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].h, 30.0, "overflow:hidden items have a zero automatic minimum and shrink");
}

#[test]
fn q23c_column_wrap_makes_side_by_side_columns() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; flex-direction: column; flex-wrap: wrap; align-content: flex-start; width: 300px; height: 100px;"><div style="width: 50px; height: 60px;"></div><div style="width: 50px; height: 60px;"></div><div style="width: 50px; height: 60px;"></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].x, 0.0);
    assert_eq!(items[1].x, 50.0, "second item wraps into a new column to the right");
    assert_eq!(items[2].x, 100.0);
    assert_eq!(items[0].y, items[1].y, "each column starts at the top");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; flex-direction: column; flex-wrap: wrap-reverse; align-content: flex-start; width: 300px; height: 100px;"><div style="width: 50px; height: 60px;"></div><div style="width: 50px; height: 60px;"></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert!(items[0].x > items[1].x);
    // Default align-content stretches the columns to fill the width.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; flex-direction: column; flex-wrap: wrap; width: 300px; height: 100px;"><div style="width: 50px; height: 60px;"></div><div style="height: 60px;"></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    // Cross sizes 50 and 0, plus 125 of free width each.
    assert_eq!(items[1].x, 175.0, "second column starts after the first (175px wide)");
    assert_eq!(items[1].w, 125.0, "a stretched auto-width item fills its column");
}

#[test]
fn q23c_baseline_alignment_lines_up_first_text_baselines() {
    let html = |align: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display: flex; align-items: {align}; width: 300px;"><div style="font-size: 40px;">Big</div><div style="font-size: 10px;">small</div></div></body></html>"#
        )
    };
    let start = measure_fixture(&html("flex-start"), 500.0);
    let s_items = flex_items(&start, 300.0);
    assert_eq!(s_items[0].y, s_items[1].y);
    let base = measure_fixture(&html("baseline"), 500.0);
    let b_items = flex_items(&base, 300.0);
    assert_eq!(b_items[0].y, s_items[0].y, "the item with the larger ascent stays on top");
    assert!(b_items[1].y > b_items[0].y + 10.0, "the small item is pushed down to share the baseline (got {})", b_items[1].y);
}

// ---------- v58: `display: grid` ----------

/// The grid container's own direct `LaidContent::Blocks` children,
/// in document order — exactly the grid items `measure_grid`
/// placed, found by locating the container itself (via its own
/// distinctive `width`, same `find_by_width` helper every other
/// section already uses), mirroring `flex_items` above.
fn grid_items(laid: &Laid, container_w: f32) -> Vec<&Laid> {
    let container = find_by_width(laid, container_w).expect("grid container not found");
    let LaidContent::Blocks(children) = &container.content else {
        panic!("expected the grid container's content to be Blocks (its own grid items)");
    };
    children.iter().collect()
}

#[test]
fn q15d_grid_repeat_auto_fill_fit() {
    let cells = |cols: &str, n: usize, w: u32| {
        let kids = "<div>x</div>".repeat(n);
        let html = format!(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: {cols}; width: {w}px;">{kids}</div></body></html>"#
        );
        let laid = measure_fixture(&html, 700.0);
        grid_items(&laid, w as f32).iter().map(|l| (l.x, l.y, l.w)).collect::<Vec<_>>()
    };
    // 400px fits four 100px tracks; 5 items wrap to a second row.
    let it = cells("repeat(auto-fill, 100px)", 5, 400);
    assert_eq!(it.iter().filter(|c| c.1 == it[0].1).count(), 4);
    assert_eq!(it[3].0, 300.0);
    // Remainder is left empty for auto-fill: 450px still fits four.
    assert_eq!(cells("repeat(auto-fill, 100px)", 2, 450)[1].0, 100.0);
    // auto-fit with minmax(100px, 1fr): 2 items stretch over 4 slots.
    let it = cells("repeat(auto-fit, minmax(100px, 1fr))", 2, 400);
    assert_eq!((it[0].2, it[1].0), (200.0, 200.0));
    // auto-fill keeps the empty tracks, so 1fr items stay 100px wide.
    let it = cells("repeat(auto-fill, minmax(100px, 1fr))", 2, 400);
    assert_eq!(it[0].2, 100.0);
    // Fixed tracks around the repeat reduce the space.
    let it = cells("50px repeat(auto-fill, 100px)", 5, 400);
    assert_eq!(it.iter().filter(|c| c.1 == it[0].1).count(), 4, "50 + 3*100 fits in 400");
    // Too narrow still yields one repetition.
    assert_eq!(cells("repeat(auto-fill, 100px)", 2, 50)[1].0, 0.0);
    // A single item still gets one 100px column of the repeat.
    let it = cells("repeat(auto-fill, 100px)", 1, 400);
    assert_eq!(it[0].2, 100.0);
}

#[test]
fn q15c_grid_numeric_placement() {
    let grid = |style: &str, kids: &str| {
        let html = format!(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 100px 100px 100px; width: 400px; {style}">{kids}</div></body></html>"#
        );
        let laid = measure_fixture(&html, 500.0);
        grid_items(&laid, 400.0).iter().map(|l| (l.x, l.y, l.w)).collect::<Vec<_>>()
    };
    // grid-column line numbers and span
    let it = grid("", r#"<div style="grid-column: 2 / 4">a</div><div style="grid-column: 1 / span 2">b</div><div style="grid-column: -2 / -1">c</div>"#);
    // items come back in row-then-column order: a, c (row 0), b (pushed to row 1)
    assert_eq!((it[0].0, it[0].2), (100.0, 200.0));
    assert_eq!((it[1].0, it[1].2), (300.0, 100.0), "-2/-1 is the last column");
    assert_eq!((it[2].0, it[2].2), (0.0, 200.0), "b collides with a in row 0 and moves down");
    // longhands + span only: auto-placed with a column span
    let it = grid("", r#"<div style="grid-column-start: span 3">a</div><div>b</div><div style="grid-column-start: 3; grid-column-end: span 2">c</div>"#);
    // c has a fixed column so it is placed first (row 0, cols 3-4); a's
    // 3-wide span no longer fits row 0, so a and b share row 1.
    assert_eq!((it[0].0, it[0].2), (200.0, 200.0));
    assert_eq!((it[1].0, it[1].2), (0.0, 300.0));
    assert_eq!((it[2].0, it[2].1), (300.0, it[1].1), "b follows a's span in the same row");
    // grid-row fixes the row; auto column finds the first free cell in it
    let it = grid("grid-template-rows: 30px 40px;", r#"<div style="grid-row: 2">a</div><div>b</div>"#);
    assert_eq!(it[1].1 - it[0].1, 30.0, "a sits in the second row, b auto-places into the first");
    // 4-value grid-area: row-start / col-start / row-end / col-end
    let it = grid("grid-template-rows: 30px 40px;", r#"<div style="grid-area: 2 / 2 / 3 / 4">a</div><div>b</div>"#);
    assert_eq!((it[1].0, it[1].2), (100.0, 200.0));
    assert_eq!(it[1].1 - it[0].1, 30.0);
    // invalid values (line 0) are dropped
    let it = grid("", r#"<div style="grid-column: 0 / 2">a</div>"#);
    assert_eq!((it[0].0, it[0].2), (0.0, 100.0));
}

#[test]
fn q15a_grid_content_sized_tracks() {
    let widths = |cols: &str, a: &str| -> (f32, f32) {
        let html = format!(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: {cols}; width: 400px;"><div>{a}</div><div>b</div></div></body></html>"#
        );
        let laid = measure_fixture(&html, 500.0);
        let items = grid_items(&laid, 400.0);
        (items[0].w, items[1].w)
    };
    let text = "alpha bravo charlie";
    let (max_w, rest) = widths("max-content 1fr", text);
    let (min_w, _) = widths("min-content 1fr", text);
    assert!(max_w > 60.0 && max_w < 350.0, "max-content width {max_w}");
    assert!(min_w > 20.0 && min_w < max_w * 0.6, "min-content {min_w} should be the longest word, less than {max_w}");
    assert!((max_w + rest - 400.0).abs() < 0.01, "1fr should take the rest");
    // `auto` + fr: auto sizes like max-content here.
    let (auto_w, auto_rest) = widths("auto 1fr", text);
    assert!((auto_w - max_w).abs() < 0.01);
    assert!((auto_w + auto_rest - 400.0).abs() < 0.01);
    // `auto` alone stretches to fill the container.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: auto; width: 400px;"><div>a</div></div></body></html>"#,
        500.0,
    );
    assert_eq!(grid_items(&laid, 400.0)[0].w, 400.0);
    // `minmax(300px, max-content)`: floor wins over small content.
    let (w, _) = widths("minmax(300px, max-content) 1fr", "a");
    assert_eq!(w, 300.0);
    // `minmax(min-content, 1fr)`: flexible with a content floor.
    let (w, r) = widths("minmax(min-content, 1fr) 100px", text);
    assert_eq!((w, r), (300.0, 100.0));
}

#[test]
fn q15b_grid_template_shorthand() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template: 30px 40px / 100px 1fr; width: 400px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!((items[0].w, items[1].w), (100.0, 300.0));
    assert_eq!(items[2].y - items[0].y, 30.0, "first row track is 30px tall");
    // area strings with row sizes and a column list
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template: 'h h' 50px 'm s' 70px / 1fr 100px; width: 400px;"><div style="grid-area:s">s</div><div style="grid-area:h">h</div><div style="grid-area:m">m</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    let mut pos: Vec<(f32, f32, f32)> = items.iter().map(|l| (l.y, l.x, l.w)).collect();
    pos.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (y0, y1) = (pos[0].0, pos[1].0);
    assert_eq!(y1 - y0, 50.0, "first area row is 50px tall");
    assert_eq!((pos[0].1, pos[0].2), (0.0, 400.0), "'h h' spans both columns");
    assert_eq!((pos[1].1, pos[1].2), (0.0, 300.0), "'m' is the 1fr column");
    assert_eq!((pos[2].1, pos[2].2), (300.0, 100.0), "'s' is the 100px column");
    // an invalid value is dropped as a whole
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template: bogus / 1fr; width: 400px;"><div>a</div></div></body></html>"#,
        500.0,
    );
    assert_eq!(grid_items(&laid, 400.0)[0].w, 400.0);
}

#[test]
fn grid_fixed_track_widths_place_items_at_the_right_x_offsets() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 50px 100px 150px; width: 300px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 300.0);
    assert_eq!(items.len(), 3, "all three children should become grid items");
    assert_eq!(items[0].x, 0.0, "the first column should start at the grid's own content edge");
    assert_eq!(items[0].w, 50.0, "the first item should fill its own 50px column, not shrink to its own content");
    assert_eq!(items[1].x, 50.0, "the second column should start exactly where the first ends");
    assert_eq!(items[1].w, 100.0);
    assert_eq!(items[2].x, 150.0, "the third column should start exactly where the second ends");
    assert_eq!(items[2].w, 150.0);
    assert_eq!(items[0].y, items[1].y, "fixed-width columns should all share one row");
    assert_eq!(items[1].y, items[2].y);
}

#[test]
fn grid_fr_tracks_distribute_proportionally_with_asymmetric_ratios() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 1fr 3fr; width: 400px;"><div>a</div><div>b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!(items[0].w, 100.0, "1fr of (1+3) should claim 1/4 of the 400px content width");
    assert_eq!(items[1].w, 300.0, "3fr of (1+3) should claim 3/4 — not an equal 50/50 split");
    assert_eq!(items[1].x, items[0].x + items[0].w, "the second column should start exactly where the first, proportionally-sized one ends");
}

#[test]
fn grid_fr_and_fixed_tracks_mix_correctly() {
    // A fixed 100px track plus a 1fr/3fr pair sharing whatever's
    // left (400 - 100 = 300, split 1:3 → 75/225) — pins that `fr`
    // distribution happens against the *leftover* space, not the
    // container's full width, once a fixed track is also present.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 1fr 3fr; width: 400px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!(items[0].w, 100.0, "the fixed track keeps its own declared width regardless of its fr siblings");
    assert_eq!(items[1].w, 75.0, "1fr of the remaining 300px (1+3 share) is 75px");
    assert_eq!(items[2].w, 225.0, "3fr of the remaining 300px (1+3 share) is 225px");
}

#[test]
fn grid_auto_placement_wraps_to_a_new_row_once_a_row_fills() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 100px; width: 200px;"><div style="height: 30px;">a</div><div style="height: 30px;">b</div><div style="height: 40px;">c</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 200.0);
    assert_eq!(items.len(), 3, "all three children should become grid items despite the wrap");
    assert_eq!(items[0].y, items[1].y, "the first two items (filling the 2-column row) should share the first row");
    assert_eq!(items[0].x, 0.0, "the first item should sit in the first column");
    assert_eq!(items[1].x, 100.0, "the second item should sit in the second column, not stacked on the first");
    assert_eq!(items[2].x, 0.0, "a third item, once the 2-column row is full, should wrap back to the first column of a new row");
    assert_eq!(
        items[2].y,
        items[0].y + 30.0,
        "the new row should start exactly where the first row's own height (the taller of its two 30px items) ends"
    );
}

#[test]
fn grid_gap_adds_spacing_between_both_columns_and_rows() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 50px 50px; gap: 10px; width: 110px;"><div style="height: 20px;">a</div><div style="height: 20px;">b</div><div style="height: 20px;">c</div><div style="height: 20px;">d</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 110.0);
    assert_eq!(items.len(), 4, "a 2x2 grid of 4 items, wrapping once the 2-column row fills");
    assert_eq!(items[1].x, items[0].x + items[0].w + 10.0, "column-gap (from the `gap` shorthand) should add exactly 10px between the two columns");
    assert_eq!(items[2].y, items[0].y + items[0].h + 10.0, "row-gap (from the same `gap` shorthand) should add exactly 10px between the two rows");
}

#[test]
fn grid_explicit_row_track_sizes_the_row_regardless_of_content() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px; grid-template-rows: 80px; width: 100px;"><div style="height: 10px;">a</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 100.0);
    assert_eq!(items.len(), 1);
    let container = find_by_width(&laid, 100.0).expect("grid container not found");
    assert_eq!(container.h, 80.0, "an explicit grid-template-rows size should set the row's (and so the container's) height regardless of its shorter content");
    assert_eq!(items[0].h, 10.0, "the item itself still reports its own natural content height — only the row's track, not the item, is forced to 80px");
}

#[test]
fn grid_repeat_expands_to_the_same_track_list_as_writing_it_out() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: repeat(3, 50px); width: 150px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 150.0);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].w, 50.0);
    assert_eq!(items[1].x, 50.0);
    assert_eq!(items[2].x, 100.0);
}

#[test]
fn grid_position_absolute_child_is_not_treated_as_a_grid_item() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; position: relative; grid-template-columns: 100px; width: 100px;"><div style="position: absolute; top: 5px; left: 5px; width: 20px; height: 20px;">p</div><div>a</div></div></body></html>"#,
        500.0,
    );
    let container = find_by_width(&laid, 100.0).expect("grid container not found");
    let LaidContent::Blocks(children) = &container.content else {
        panic!("expected Blocks content");
    };
    assert_eq!(children.len(), 1, "the absolutely-positioned child should be pulled out of the grid item list entirely");
    assert_eq!(children[0].x, container.x, "the one real grid item should occupy the grid's first cell (its own left content edge), not pushed aside by the out-of-flow child");
    assert_eq!(container.positioned.len(), 1, "the absolutely-positioned child should instead land in the container's own `positioned` list");
}

#[test]
fn grid_minmax_zero_floor_behaves_exactly_like_plain_fr() {
    // The one real-world shape confirmed by pulling actual Wikipedia
    // CSS (see `TODO.txt`'s own `display: grid` entry): `minmax(0,
    // 1fr)` is meant purely to stop a flexible track from growing
    // past its own fair share, never to add a real floor — so it
    // should distribute identically to a bare `1fr` in the same
    // position.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px minmax(0, 1fr); width: 400px;"><div>a</div><div>b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!(items[0].w, 100.0);
    assert_eq!(items[1].w, 300.0, "minmax(0, 1fr) should claim all remaining space, identically to a bare 1fr");
}

#[test]
fn grid_minmax_nonzero_floor_is_reserved_before_fr_distribution() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: minmax(150px, 1fr) minmax(150px, 1fr); width: 400px;"><div>a</div><div>b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!(items[0].w, 200.0, "each 150px floor plus an equal share of the 100px left over (400 - 150 - 150) should land at 200px");
    assert_eq!(items[1].w, 200.0);
}

#[test]
fn grid_minmax_fixed_ceiling_clamps_below_free_space() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: minmax(0, 120px) 1fr; width: 400px;"><div>a</div><div>b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 400.0);
    assert_eq!(items[0].w, 120.0, "a minmax() whose own max is a fixed length should resolve to exactly that length, not grow to fill free space");
    assert_eq!(items[1].w, 280.0, "the 1fr sibling should claim everything the fixed-ceiling track didn't");
}

#[test]
fn grid_template_areas_places_items_by_name_not_document_order() {
    // Document order here is deliberately the *reverse* of the
    // template's own left-to-right area order, so this can only
    // pass if placement genuinely comes from `grid-area`, not from
    // falling back to auto-placement.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 100px; grid-template-areas: 'left right'; width: 200px;"><div style="grid-area: right;">b</div><div style="grid-area: left;">a</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 200.0);
    assert_eq!(items.len(), 2);
    let left = items.iter().find(|i| i.x == 0.0).expect("an item should land in the left area's own column");
    let right = items.iter().find(|i| i.x == 100.0).expect("an item should land in the right area's own column");
    assert_eq!(left.y, right.y, "both named areas are in the template's one row");
    assert_eq!(left.w, 100.0);
    assert_eq!(right.w, 100.0);
}

#[test]
fn grid_template_areas_name_spanning_two_columns_gets_their_combined_width() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 50px 150px; grid-template-areas: 'wide wide' 'a b'; width: 200px;"><div style="grid-area: wide;">w</div><div style="grid-area: a;">a</div><div style="grid-area: b;">b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 200.0);
    assert_eq!(items.len(), 3);
    // Row-major order (the spanning "wide" area is alone in row 0;
    // "a"/"b" share row 1, sorted left-to-right) makes `items[0]`
    // deterministically the spanning item.
    assert_eq!(items[0].x, 0.0, "a two-column-spanning area should start at the grid's own left edge");
    assert_eq!(items[0].w, 200.0, "a two-column-spanning area should claim both columns' widths combined (50 + 150), not just the first");
    assert!(items[0].y < items[1].y, "the spanning area's own row should come before the row holding 'a'/'b'");
}

#[test]
fn grid_template_areas_with_no_explicit_columns_falls_back_to_equal_fr_per_area_column() {
    // The real shape this item was built to fix: `grid-template-
    // areas` declared with no (parseable) `grid-template-columns`
    // at all — real Wikipedia CSS hits this because its own column
    // sizes are declared via a `grid-template` shorthand or keyword
    // this file still doesn't parse (see `TODO.txt`). Two named
    // areas, two implied columns, should still split the container
    // evenly rather than collapsing to one full-width column.
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-areas: 'left right'; width: 200px;"><div style="grid-area: left;">a</div><div style="grid-area: right;">b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 200.0);
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|i| i.x == 0.0 && i.w == 100.0), "the left area should get an equal half of the 200px container");
    assert!(items.iter().any(|i| i.x == 100.0 && i.w == 100.0), "the right area should get the other equal half, not stack full-width under the first");
}

#[test]
fn grid_template_areas_item_with_no_matching_grid_area_auto_places_into_the_free_cell() {
    let laid = measure_fixture(
        r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 100px; grid-template-areas: 'left right'; width: 200px;"><div style="grid-area: left;">a</div><div>b</div></div></body></html>"#,
        500.0,
    );
    let items = grid_items(&laid, 200.0);
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|i| i.x == 0.0), "the named item should still claim the left area");
    assert!(items.iter().any(|i| i.x == 100.0), "the plain item, with no grid-area of its own, should auto-place into the one remaining free cell, not overlap the named one");
}

// QJ4b: items needing more columns than the explicit tracks used to panic in `measure_grid`
// (`col_widths[col_start..col_end]` out of range). They now create implicit tracks.
fn qj4b_items(style: &str, kids: &str) -> Vec<(f32, f32, f32)> {
    let html = format!(
        r#"<html><body style="margin: 0"><div style="display: grid; width: 400px; {style}">{kids}</div></body></html>"#
    );
    let laid = measure_fixture(&html, 700.0);
    grid_items(&laid, 400.0).iter().map(|l| (l.x, l.y, l.w)).collect()
}

#[test]
fn qj4b_span_wider_than_explicit_tracks_makes_implicit_column() {
    let it = qj4b_items(
        "grid-template-columns: 100px; grid-auto-columns: 60px;",
        r#"<div style="grid-column: span 2">a</div><div>b</div>"#,
    );
    assert_eq!(it.len(), 2);
    assert_eq!((it[0].0, it[0].2), (0.0, 160.0), "span 2 covers 100px + implicit 60px");
}

#[test]
fn qj4b_numeric_line_beyond_explicit_tracks() {
    let it = qj4b_items(
        "grid-template-columns: 100px; grid-auto-columns: 50px;",
        r#"<div style="grid-column: 1 / 3">a</div><div style="grid-column: 4">b</div>"#,
    );
    assert_eq!((it[0].0, it[0].2), (0.0, 150.0));
    assert_eq!((it[1].0, it[1].2), (200.0, 50.0), "column 4 is the third implicit track");
}

#[test]
fn qj4b_named_area_wider_than_tracks() {
    let it = qj4b_items(
        "grid-template-columns: 100px; grid-template-areas: 'a b';",
        r#"<div style="grid-area: a">x</div><div style="grid-area: b">y</div>"#,
    );
    assert_eq!(it.len(), 2);
    assert_eq!(it[0].0, 0.0);
    assert!(it[1].0 >= 100.0, "second area sits in an implicit column");
}

#[test]
fn qj4b_auto_fill_with_wide_span_and_no_track_list() {
    let it = qj4b_items("grid-template-columns: repeat(auto-fill, 100px);", r#"<div style="grid-column: span 6">a</div>"#);
    assert_eq!(it.len(), 1);
    let it = qj4b_items("", r#"<div style="grid-column: 1 / 4">a</div><div style="grid-row: 3; grid-column: 2">b</div>"#);
    assert_eq!(it.len(), 2);
}

#[test]
fn qj4b_rows_and_huge_lines_do_not_panic() {
    let it = qj4b_items(
        "grid-template-columns: 100px 100px; grid-template-rows: 20px; grid-auto-rows: 30px;",
        r#"<div style="grid-row: 3 / 5">a</div><div style="grid-row: span 3">b</div><div style="grid-column: 1 / 99999999">c</div><div style="grid-row: -9 / -1">d</div>"#,
    );
    assert_eq!(it.len(), 4);
    let it = qj4b_items("grid-template-columns: 100px; grid-auto-rows: 30px;", r#"<div>a</div><div>b</div>"#);
    assert_eq!(it[1].1 - it[0].1, 30.0, "implicit rows take grid-auto-rows");
}
