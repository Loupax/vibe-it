#[allow(unused_imports)]
use super::*;

// ---------- Q36: colspan / rowspan ----------

fn collect_table_rows<'a>(l: &'a Laid, out: &mut Vec<&'a Laid>) {
    match &l.content {
        LaidContent::TableRow(_) => out.push(l),
        LaidContent::Blocks(cs) => cs.iter().for_each(|c| collect_table_rows(c, out)),
        _ => {}
    }
}

fn row_cells(l: &Laid) -> &[Laid] {
    let LaidContent::TableRow(c) = &l.content else { panic!("not a row") };
    c
}

#[test]
fn colspan_header_spans_both_columns_without_squeezing_them() {
    let laid = measure_fixture(
        r#"<table><tr><td colspan="2">Header</td></tr><tr><td>Hi</td><td>This is a much longer piece of cell text that needs far more room than the first</td></tr></table>"#,
        900.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    let header = row_cells(rows[0]);
    let second = row_cells(rows[1]);
    assert_eq!(header.len(), 1);
    assert!((header[0].w - (second[0].w + second[1].w)).abs() < 0.5, "header {} vs {} + {}", header[0].w, second[0].w, second[1].w);
    assert!(second[1].w > second[0].w * 3.0, "columns keep their natural widths");
    assert!((second[1].x - (second[0].x + second[0].w)).abs() < 0.5);
}

#[test]
fn cell_after_colspan_cell_lands_in_the_following_column() {
    let laid = measure_fixture(
        r#"<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td colspan="2">ab</td><td>c</td></tr></table>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    let (r0, r1) = (row_cells(rows[0]), row_cells(rows[1]));
    assert!((r1[1].x - r0[2].x).abs() < 0.5, "third column lines up: {} vs {}", r1[1].x, r0[2].x);
    assert!((r1[0].w - (r0[0].w + r0[1].w)).abs() < 0.5);
}

#[test]
fn rowspan_cell_takes_summed_row_height_and_pushes_the_next_column_over() {
    let laid = measure_fixture(
        r#"<table><tr><td rowspan="2"><div>tall</div><div>tall</div><div>tall</div><div>tall</div></td><td>x</td></tr><tr><td>y</td></tr><tr><td>z</td><td>w</td></tr></table>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    assert_eq!(rows.len(), 3);
    let (r0, r1, r2) = (row_cells(rows[0]), row_cells(rows[1]), row_cells(rows[2]));
    assert!((r1[0].x - r0[1].x).abs() < 0.5, "y under x: {} vs {}", r1[0].x, r0[1].x);
    assert!((r0[0].h - (rows[0].h + rows[1].h)).abs() < 0.5, "{} vs {}+{}", r0[0].h, rows[0].h, rows[1].h);
    assert!(r0[0].h > 2.0 * rows[2].h, "four lines tall: {} vs one-line row {}", r0[0].h, rows[2].h);
    assert!((rows[2].y - (rows[1].y + rows[1].h)).abs() < 0.5);
    assert!((r2[0].x - r0[0].x).abs() < 0.5);
}

#[test]
fn table_columns_size_from_their_own_cell_content_not_an_equal_split() {
    // Two columns, deliberately very uneven content: the old equal-
    // width-column layout would give both exactly half of the
    // table's content width regardless. Real auto table layout
    // should size each column from its own cells' content instead.
    let laid = measure_fixture(
        r#"<table><tr><td>Hi</td><td>This is a much longer piece of cell text that genuinely needs a lot more horizontal room than the first column does</td></tr></table>"#,
        900.0,
    );
    let cells = find_table_row_cells(&laid).expect("table row not found");
    assert_eq!(cells.len(), 2, "two <td>s");
    assert!(
        cells[1].w > cells[0].w * 3.0,
        "the long-text column ({}) should end up substantially wider than the short one ({}), not an equal 450/450 split",
        cells[1].w,
        cells[0].w
    );
}

#[test]
fn table_column_widths_still_fill_the_tables_own_content_width() {
    // Real per-column sizing changes how `content_w` is *divided*,
    // not the table's own box-model width — an auto-width table
    // still fills its container exactly as it did with equal-width
    // columns (see `measure_table`'s own docs on why changing that
    // is a separate, bigger scope). `body { margin: 8px }` (default.
    // css) eats 16px total, so a 900px window gives the table a
    // 884px content width.
    let laid = measure_fixture(
        r#"<table style="width:100%"><tr><td>a</td><td>a much much longer second column of text here</td></tr></table>"#,
        900.0,
    );
    let cells = find_table_row_cells(&laid).expect("table row not found");
    let total: f32 = cells.iter().map(|c| c.w).sum();
    assert!((total - 884.0).abs() < 1.0, "columns should sum to the table's own 884px content width, got {total}");
}

#[test]
fn table_narrow_container_shrinks_columns_proportionally_to_min_content_not_below_their_longest_word() {
    // A container too narrow for either column's own natural
    // (unwrapped) width forces both to shrink below their `max` —
    // but a column still shouldn't collapse to less than its own
    // widest unbreakable word (`min`-content): real CSS lets content
    // overflow rather than breaking a word, the same `min-width`-
    // wider-than-container behavior this engine already has
    // elsewhere (see `clamp_box_dim`). Both columns here are a
    // single long word each (own `min` == own `max`, so this
    // exercises the "even every column's `min` doesn't fit" shrink
    // branch, not the ordinary interpolate-between-min-and-max one).
    let laid = measure_fixture(
        r#"<table><tr><td>Supercalifragilisticexpialidocious</td><td>Pneumonoultramicroscopicsilicovolcanoconiosis</td></tr></table>"#,
        60.0,
    );
    let cells = find_table_row_cells(&laid).expect("table row not found");
    // The second word is longer than the first, so even shrunk
    // proportionally to each column's own min-content share, it
    // should still end up wider than the first column, not clamped
    // to some shared minimum.
    assert!(
        cells[1].w > cells[0].w,
        "the longer single-word column ({}) should still end up wider than the shorter one ({}) even once both are shrunk",
        cells[1].w,
        cells[0].w
    );
}

#[test]
fn nested_table_in_a_cell_gets_its_own_independent_column_sizing() {
    // v15's own "deliberately pathological" case, still true after
    // real per-column sizing: a `<table>` nested inside a cell of an
    // outer table isn't flattened into the outer table's own row —
    // `cell_natural_width`'s `Blocks`/`TableRow` recursion has to
    // walk down into it without mistaking its rows for the outer
    // table's own columns.
    let laid = measure_fixture(
        r#"<table><tr><td>outer cell</td><td><table><tr><td>inner a</td><td>inner cell with much longer text in it</td></tr></table></td></tr></table>"#,
        900.0,
    );
    let outer_cells = find_table_row_cells(&laid).expect("outer table row not found");
    assert_eq!(outer_cells.len(), 2, "outer table still has exactly 2 columns, not the inner table's 2 flattened in");
    let inner_cells = find_table_row_cells(&outer_cells[1]).expect("nested table's own row not found inside the second outer cell");
    assert_eq!(inner_cells.len(), 2, "inner table's own row has its own 2 cells");
    assert!(
        inner_cells[1].w > inner_cells[0].w,
        "the inner table's own uneven columns ({}, {}) should still size from their own content",
        inner_cells[0].w,
        inner_cells[1].w
    );
}

// ---------- QA10: anonymous table objects (CSS 2.1 17.2.1) ----------

#[test]
fn qa10_table_cells_in_a_block_get_an_anonymous_row_and_table() {
    // Four `display:table-cell` siblings, no row, no table: one row, 4 cells side by side.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div><div style="display:table-cell;width:20px;height:10px"></div> <div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div></div></body></html>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    assert_eq!(rows.len(), 1, "one anonymous row");
    let cells = row_cells(rows[0]);
    assert_eq!(cells.len(), 4);
    for w in cells.windows(2) {
        assert!((w[1].x - (w[0].x + w[0].w)).abs() < 0.5, "cells abut horizontally");
    }
}

#[test]
fn qa10_cells_directly_in_display_table_get_an_anonymous_row() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display:table"><div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div></div></body></html>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(rows[0]).len(), 3);
}

#[test]
fn qa10_table_rows_in_a_block_get_an_anonymous_table() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div><div style="display:table-row"><div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div></div><div style="display:table-row"><div style="display:table-cell;width:20px;height:10px"></div><div style="display:table-cell;width:20px;height:10px"></div></div></div></body></html>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    assert_eq!(rows.len(), 2);
    let (a, b) = (row_cells(rows[0]), row_cells(rows[1]));
    assert_eq!((a.len(), b.len()), (2, 2));
    assert!((a[1].x - (a[0].x + a[0].w)).abs() < 0.5);
    assert!((b[0].x - a[0].x).abs() < 0.5, "columns line up");
    assert!(b[0].y > a[0].y, "second row below the first");
}

// QA12: inline event-handler content attributes.
#[test]
fn qa12_inline_handlers_click_return_false_body_onload_and_errors() {
    let (engine, doc, log) = events_engine(
        "<html><body onload=\"console.log('L', this === document.body || this === window)\">\
         <button id=a onclick=\"this.id='x'; console.log('c', event.type)\">b</button>\
         <a id=r href=/x onclick=\"return false\">r</a>\
         <p id=e onclick=\"nope.nothing()\" onmouseover=\"syntax error (\">e</p>\
         <p id=s onclick=\"console.log('s1')\"><b id=t>t</b></p></body></html>",
        "document.getElementById('s').addEventListener('click', function(){ console.log('listener') });\
         document.getElementById('s').addEventListener('click', function(){ console.log('listener2') });",
    );
    // body onload ran on the window load event, after the page scripts.
    assert_eq!(log.borrow()[0], "log:L true", "{:?}", log.borrow());
    let init = [("bubbles", EventField::Bool(true)), ("cancelable", EventField::Bool(true))];
    assert!(engine.dispatch_event(Some(&by_id(&doc, "a")), "click", &init));
    assert!(log.borrow().contains(&"log:c click".to_string()), "{:?}", log.borrow());
    assert!(!engine.dispatch_event(Some(&by_id(&doc, "r")), "click", &init), "return false cancels");
    // A throwing handler is logged and does not stop later handlers.
    log.borrow_mut().clear();
    engine.dispatch_event(Some(&by_id(&doc, "e")), "click", &init);
    engine.dispatch_event(Some(&by_id(&doc, "e")), "mouseover", &init);
    assert!(log.borrow()[0].starts_with("error:Uncaught ReferenceError"), "{:?}", log.borrow());
    assert!(log.borrow()[1].starts_with("error:Uncaught SyntaxError"), "{:?}", log.borrow());
    // Listeners fire before the attribute handler's slot (registered earlier), bubbling too.
    log.borrow_mut().clear();
    engine.dispatch_event(Some(&by_id(&doc, "t")), "click", &init);
    assert_eq!(*log.borrow(), ["log:listener", "log:listener2", "log:s1"]);
}

// Q33d1: element.click(), form submit events, mouseover/out relatedTarget.
#[test]
fn q33d1_click_toggles_checkbox_fires_submit_and_follows_links() {
    let (engine, doc, log) = events_engine(
        "<html><body><form id=f onsubmit=\"console.log('submit', event.submitter && event.submitter.id); return false\">\
         <input id=c type=checkbox><input id=r1 type=radio name=g checked><input id=r2 type=radio name=g>\
         <input id=d type=checkbox disabled><button id=b>go</button></form>\
         <a id=l href=/next>l</a><a id=l2 href=/no>n</a><form id=g><button id=gb>x</button></form></body></html>",
        "var $=function(i){return document.getElementById(i)};\
         $('c').addEventListener('click', function(e){ console.log('c', e.isTrusted, $('c').checked) });\
         $('c').addEventListener('change', function(){ console.log('change') });\
         $('c').click(); console.log('after', $('c').checked); $('c').click(); console.log('again', $('c').checked);\
         $('d').click(); console.log('disabled', $('d').checked);\
         $('r2').click(); console.log('radio', $('r1').checked, $('r2').checked);\
         $('b').click();\
         $('l2').addEventListener('click', function(e){ e.preventDefault(); });\
         $('l2').click();\
         $('g').addEventListener('submit', function(){ console.log('g submit'); });\
         $('g').submit(); console.log('submit() fires none');\
         $('g').requestSubmit(); $('l').click();",
    );
    let _ = doc;
    let l = log.borrow().clone();
    assert_eq!(
        l,
        [
            "log:c false true", "log:change", "log:after true", "log:c false false", "log:change", "log:again false",
            "log:disabled false", "log:radio false true", "log:submit b", "log:submit() fires none", "log:g submit",
        ],
        "{l:?}"
    );
    assert_eq!(engine.take_requests().nav.as_deref(), Some("/next"), "only the uncancelled link navigates");
}

#[test]
fn q33d1_cancelled_checkbox_click_reverts_and_mouseover_has_related_target() {
    let (engine, doc, log) = events_engine(
        "<html><body><input id=c type=checkbox><p id=a>a</p><p id=b>b</p></body></html>",
        "var $=function(i){return document.getElementById(i)};\
         $('c').addEventListener('click', function(e){ e.preventDefault(); });\
         $('c').click(); console.log('checked', $('c').checked);\
         $('b').addEventListener('mouseover', function(e){ console.log('over', e.relatedTarget && e.relatedTarget.id, e.isTrusted, e.bubbles); });\
         $('a').addEventListener('mouseout', function(e){ console.log('out', e.relatedTarget && e.relatedTarget.id); });",
    );
    assert_eq!(log.borrow()[0], "log:checked false");
    let a = by_id(&doc, "a");
    let b = by_id(&doc, "b");
    let init = |rel: &markup5ever_rcdom::Handle| vec![("bubbles", EventField::Bool(true)), ("relatedTargetId", EventField::Node(rel.clone()))];
    engine.dispatch_event(Some(&a), "mouseout", &init(&b));
    engine.dispatch_event(Some(&b), "mouseover", &init(&a));
    let l = log.borrow().clone();
    assert_eq!(l[1..], ["log:out b", "log:over a true true"], "{l:?}");
}

#[test]
fn qa10_blocks_between_cells_of_a_table_get_anonymous_cells_in_the_same_row() {
    // The Acid2 line-14 shape: cell, nested table, cell, plain block, all
    // children of one `display:table`: a single row of four cells.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><ul style="display:table;padding:0;margin:0"><li style="display:table-cell;width:10px;height:10px"></li><li style="display:table;width:10px;height:10px"></li><li style="display:table-cell;width:10px;height:10px"></li><li style="list-style:none;width:10px;height:10px"></li></ul></body></html>"#,
        600.0,
    );
    let mut rows = Vec::new();
    collect_table_rows(&laid, &mut rows);
    assert_eq!(rows.len(), 1, "one anonymous row");
    let cells = row_cells(rows[0]);
    assert_eq!(cells.len(), 4);
    for w in cells.windows(2) {
        assert!((w[1].x - (w[0].x + w[0].w)).abs() < 0.5, "cells abut horizontally");
    }
}

// ---------- Q52c1: table rows (separated border model) ----------

fn first_table_row(laid: &Laid) -> Option<&Laid> {
    if matches!(laid.content, LaidContent::TableRow(_)) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        return children.iter().find_map(first_table_row);
    }
    None
}

#[test]
fn table_row_border_margin_padding_ignored_when_separated() {
    let laid = measure_fixture(
        r#"<html><head><style>
        tr { border: 4px solid red; margin: 7px; padding: 5px; }
        td { border: inherit; padding: inherit; }
        </style></head><body><table><tr><td>x</td></tr></table></body></html>"#,
        900.0,
    );
    let row = first_table_row(&laid).expect("row");
    assert_eq!(row.style.border_width, [0.0; 4], "row border is ignored in the separated model");
    assert_eq!(row.style.margin.top.px(), 0.0);
    assert_eq!(row.style.padding.top.px(), 0.0);
    // ...but the computed values still inherit to the cell.
    let LaidContent::TableRow(cells) = &row.content else { unreachable!() };
    assert_eq!(cells[0].style.border_width, [4.0; 4]);
    assert_eq!(cells[0].style.padding.top.px(), 5.0);
}

#[test]
fn table_row_border_kept_with_border_collapse() {
    let laid = measure_fixture(
        r#"<html><head><style>
        table { border-collapse: collapse; }
        tr { border: 4px solid red; }
        </style></head><body><table><tr><td>x</td></tr></table></body></html>"#,
        900.0,
    );
    let row = first_table_row(&laid).expect("row");
    assert_eq!(row.style.border_width, [4.0; 4], "collapse model keeps the row border (approximation)");
}
