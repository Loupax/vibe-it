//! QINC1c: bottom-up z-index hoisting (`layout::finish_z`) against the old whole-tree post-pass
//! (`hoist_negative_z`, kept under `cfg(test)`).
//!
//! Every `measure_document` call a test makes (hundreds of pages: box layout, z-index, stacking,
//! positioning, flex, tables, JS geometry, the fuzz harness) is laid a second time with the old
//! pass and compared with `cmp_laid` inside `measure_document` itself, and panics on the first
//! difference. These tests add the shapes whose parent-view readers (first baseline, last line
//! extent, collapsing through an empty block, table rows, static inline-blocks) a bubble could
//! change, and check that the hoisting really happened (so the comparison is not vacuous).
use super::*;

fn lay(html: &str, w: f32) -> (Laid, ShapeCache) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, w);
    let mut fs = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut id, mut inc) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    // Panics inside when the bubble and the old post-pass disagree.
    let laid = measure_document(&tree, &mut fs, w, 600.0, &mut id, &mut cache, deadline, &mut inc);
    assert!(!inc);
    (laid, cache)
}

fn count_positioned(l: &Laid) -> usize {
    let mut n = l.positioned.len() + l.positioned.iter().map(count_positioned).sum::<usize>();
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => n += c.iter().map(count_positioned).sum::<usize>(),
        LaidContent::MixedInline(items) => {
            for i in items {
                if let MixedPositioned::Embedded(e) = i {
                    n += count_positioned(e);
                }
            }
        }
        _ => {}
    }
    n
}

fn no_bubble_left(l: &Laid) {
    assert!(l.zb.as_ref().is_none_or(|z| z.bubble.is_empty()), "an unabsorbed bubble is left in the final tree");
    for p in &l.positioned {
        no_bubble_left(p);
    }
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(no_bubble_left),
        LaidContent::MixedInline(items) => {
            for i in items {
                if let MixedPositioned::Embedded(e) = i {
                    no_bubble_left(e);
                }
            }
        }
        _ => {}
    }
}

#[test]
fn qinc1c_relative_and_z_indexed_descendants_reach_the_nearest_boundary() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><div id=o style="position:relative;z-index:0;width:300px;height:200px">
        <div><div style="position:relative;z-index:3;height:20px">a</div><div style="position:relative;height:20px">b</div></div>
        <div style="position:absolute;top:0;left:0;z-index:-1;width:10px;height:10px"></div>
        <div style="overflow:hidden;height:50px"><div style="position:relative;z-index:9;height:5px">c</div></div>
        </div></body></html>"#,
        400.0,
    );
    // 3 entries reach `#o` (two relatives and the abs), 1 stays in the clipping box.
    assert!(count_positioned(&laid) >= 4);
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_relative_first_child_keeps_the_flex_baseline() {
    // The parent reads the first baseline of a measured item: a hoisted relative child must still count.
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><div style="display:flex;align-items:baseline;width:400px">
        <div><div style="position:relative;top:2px">first baseline text</div></div>
        <div style="padding-top:12px">second</div></div></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_empty_block_with_a_relative_child_does_not_collapse_through() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><p style="margin:0">x</p><div style="height:0;margin:30px 0"><div style="position:relative;height:0"></div></div><p style="margin:0">y</p></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_inline_runs_with_relative_inline_blocks_and_floats() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0;width:300px">before text <span style="display:inline-block;position:relative;z-index:2;width:30px;height:12px">i</span> after
        <div style="float:left;width:40px;height:40px"></div>more text here and there<span style="display:inline-block;position:relative;width:20px;height:9px"></span></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_static_inline_block_is_not_descended_like_the_old_pass() {
    // The old pass never visited a static inline-block: its relative / z-indexed children stay put.
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><div style="position:relative;z-index:1;width:200px">text <span style="display:inline-block;overflow:hidden;width:80px"><div style="position:relative;z-index:4;height:8px"></div></span></div></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_table_rows_cells_rowspan_and_sticky() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><table style="width:300px"><tr><td style="position:relative;z-index:2" rowspan=2>a<br>b<br>c</td><td>d</td></tr><tr><td style="position:sticky;top:0">e</td></tr></table>
        <div style="position:sticky;top:5px;height:10px"><div style="position:relative;z-index:1;height:4px"></div></div></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_fixed_and_orphan_absolute_boxes_with_nested_entries() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><div style="height:2000px"></div>
        <div style="position:fixed;top:0;left:0;width:100px;height:100px"><div style="position:relative;z-index:2;height:5px"></div><div style="position:absolute;z-index:3;width:5px;height:5px"></div></div>
        <div style="position:absolute;top:10px;left:10px;width:50px;height:50px"><div style="position:relative;z-index:-2;height:5px"></div></div>
        <div style="position:relative"><div style="position:absolute;z-index:1;top:0;width:5px;height:5px"></div></div></body></html>"#,
        400.0,
    );
    no_bubble_left(&laid);
}

#[test]
fn qinc1c_equal_z_index_ties_follow_document_order() {
    let (laid, _) = lay(
        r#"<html><body style="margin:0"><div style="position:relative;z-index:1;height:5px" id=a></div><div style="position:relative;z-index:1;height:5px" id=b></div><div style="position:absolute;z-index:1;width:3px;height:3px" id=c></div><div style="position:relative;z-index:1;height:5px" id=d></div></body></html>"#,
        400.0,
    );
    let order: Vec<usize> = laid.positioned.iter().map(|p| p.style.tree_order).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "ties must come out in document order: {order:?}");
    assert_eq!(order.len(), 4);
}

#[test]
fn qinc1c_builder_numbers_boxes_in_document_order_from_one() {
    let doc = parse_html(r#"<html><body><div id=a><span id=b>x</span></div><div id=c></div></body></html>"#);
    let tree = build_layout_tree(&doc, 400.0);
    fn walk(b: &LayoutBox, out: &mut Vec<usize>) {
        if b.style.node != 0 {
            out.push(b.style.tree_order);
        }
        match &b.content {
            BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().for_each(|c| walk(c, out)),
            BoxContent::Inline(items) => {
                for i in items {
                    if let InlineItem::Embedded(e) = i {
                        walk(e, out);
                    }
                }
            }
            BoxContent::Image(_) => {}
        }
    }
    let mut v = Vec::new();
    walk(&tree, &mut v);
    assert_eq!(v.first(), Some(&1));
    assert!(v.windows(2).all(|w| w[0] <= w[1]), "pre-order numbering: {v:?}");
}
