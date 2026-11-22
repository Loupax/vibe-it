//! QCSS-C: CSS box alignment keywords, `place-*`, `grid-auto-flow`, `grid-gap` aliases.
#[allow(unused_imports)]
use super::*;

/// The laid box whose `margin-top` is `mt` (a unique marker per test box).
fn boxed(html: &str, width: f32, mt: f32) -> Laid {
    let laid = measure_fixture(html, width);
    find_laid_by_margin_top(&laid, mt).expect("marked box not found").clone()
}

fn computed(style: &str, props: &[&str]) -> Vec<String> {
    let js: String = props
        .iter()
        .map(|p| format!("console.log(getComputedStyle(document.getElementById('a')).getPropertyValue('{p}'));"))
        .collect();
    run_in_html(&format!("<div id=a style=\"{style}\"></div>"), &js)
        .into_iter()
        .map(|l| l.strip_prefix("log:").unwrap_or(&l).to_string())
        .collect()
}

#[test]
fn qcss_c_computed_initial_values() {
    let v = computed(
        "",
        &["justify-content", "align-content", "align-items", "align-self", "justify-items", "justify-self", "grid-auto-flow", "place-items"],
    );
    assert_eq!(v, ["normal", "normal", "normal", "auto", "normal", "auto", "row", "normal"]);
}

#[test]
fn qcss_c_computed_keywords_round_trip() {
    for (decl, prop, want) in [
        ("justify-content:start", "justify-content", "start"),
        ("justify-content:end", "justify-content", "end"),
        ("justify-content:left", "justify-content", "left"),
        ("justify-content:right", "justify-content", "right"),
        ("justify-content:space-evenly", "justify-content", "space-evenly"),
        ("justify-content:stretch", "justify-content", "stretch"),
        ("justify-content:safe center", "justify-content", "center"),
        ("justify-content:first baseline", "justify-content", "baseline"),
        ("justify-content:last baseline", "justify-content", "last baseline"),
        ("align-content:end", "align-content", "end"),
        ("align-content:normal", "align-content", "normal"),
        ("align-items:self-end", "align-items", "self-end"),
        ("align-items:self-start", "align-items", "self-start"),
        ("align-items:unsafe end", "align-items", "end"),
        ("align-items:last baseline", "align-items", "last baseline"),
        ("align-items:first baseline", "align-items", "baseline"),
        ("align-self:auto", "align-self", "auto"),
        ("align-self:flex-end", "align-self", "flex-end"),
        ("justify-items:legacy", "justify-items", "legacy"),
        ("justify-items:legacy center", "justify-items", "legacy center"),
        ("justify-items:right", "justify-items", "right"),
        ("justify-self:left", "justify-self", "left"),
        ("grid-auto-flow:column", "grid-auto-flow", "column"),
        ("grid-auto-flow:dense", "grid-auto-flow", "row dense"),
        ("grid-auto-flow:dense column", "grid-auto-flow", "column dense"),
        ("grid-auto-flow:row dense", "grid-auto-flow", "row dense"),
    ] {
        assert_eq!(computed(decl, &[prop]), [want], "{decl}");
    }
}

#[test]
fn qcss_c_invalid_values_are_ignored() {
    for (decl, prop, want) in [
        ("justify-content:foo", "justify-content", "normal"),
        ("align-items:left", "align-items", "normal"),
        ("align-content:left", "align-content", "normal"),
        ("align-items:center center", "align-items", "normal"),
        ("align-items:safe", "align-items", "normal"),
        ("align-items:legacy", "align-items", "normal"),
        ("align-content:self-end", "align-content", "normal"),
        ("grid-auto-flow:row column", "grid-auto-flow", "row"),
        ("grid-auto-flow:dense dense", "grid-auto-flow", "row"),
    ] {
        assert_eq!(computed(decl, &[prop]), [want], "{decl}");
    }
}

#[test]
fn qcss_c_place_shorthands() {
    assert_eq!(computed("place-content:center", &["align-content", "justify-content", "place-content"]), ["center", "center", "center"]);
    assert_eq!(
        computed("place-content:end space-between", &["align-content", "justify-content", "place-content"]),
        ["end", "space-between", "end space-between"]
    );
    assert_eq!(computed("place-items:end center", &["align-items", "justify-items"]), ["end", "center"]);
    assert_eq!(computed("place-items:first baseline", &["align-items", "justify-items"]), ["baseline", "baseline"]);
    assert_eq!(computed("place-items:safe end legacy right", &["align-items", "justify-items"]), ["end", "legacy right"]);
    assert_eq!(computed("place-self:start", &["align-self", "justify-self"]), ["start", "start"]);
    assert_eq!(computed("place-self:auto end", &["align-self", "justify-self"]), ["auto", "end"]);
    // Invalid: nothing is set.
    assert_eq!(computed("place-items:foo", &["align-items", "justify-items"]), ["normal", "normal"]);
    assert_eq!(computed("place-content:center foo", &["align-content", "justify-content"]), ["normal", "normal"]);
}

#[test]
fn qcss_c_gap_aliases() {
    assert_eq!(computed("grid-gap:5px 7px", &["row-gap", "column-gap", "gap", "grid-gap"]), ["5px", "7px", "5px 7px", "5px 7px"]);
    assert_eq!(computed("grid-row-gap:3px", &["row-gap", "column-gap"]), ["3px", "0px"]);
    assert_eq!(computed("grid-column-gap:4px", &["row-gap", "column-gap", "grid-column-gap"]), ["0px", "4px", "4px"]);
    assert_eq!(computed("gap:9px", &["gap"]), ["9px"]);
    assert_eq!(computed("gap:normal", &["gap"]), ["0px"]);
}

#[test]
fn qcss_c_later_align_self_auto_overrides() {
    let v = computed("align-self:end;align-self:auto", &["align-self"]);
    assert_eq!(v, ["auto"]);
}

fn flex_item_x(container_css: &str, item_css: &str) -> f32 {
    let html = format!(
        r#"<html><body style="margin:0"><div style="display:flex;width:300px;height:100px;{container_css}"><div style="width:41px;height:20px;{item_css}"></div></div></body></html>"#
    );
    find_by_width(&measure_fixture(&html, 800.0), 41.0).expect("item").x
}

fn flex_item_y(container_css: &str, item_css: &str) -> f32 {
    let html = format!(
        r#"<html><body style="margin:0"><div style="display:flex;width:300px;height:100px;{container_css}"><div style="width:41px;height:20px;{item_css}"></div></div></body></html>"#
    );
    find_by_width(&measure_fixture(&html, 800.0), 41.0).expect("item").y
}

#[test]
fn qcss_c_flex_justify_content_keywords() {
    assert_eq!(flex_item_x("justify-content:start", ""), 0.0);
    // 300 - 41 free.
    assert_eq!(flex_item_x("justify-content:left", ""), 0.0);
    assert_eq!(flex_item_x("justify-content:normal", ""), 0.0);
    assert_eq!(flex_item_x("justify-content:end", ""), 259.0);
    assert_eq!(flex_item_x("justify-content:right", ""), 259.0);
    assert_eq!(flex_item_x("justify-content:flex-end", ""), 259.0);
    assert_eq!(flex_item_x("justify-content:center", ""), 129.5);
    assert_eq!(flex_item_x("justify-content:safe center", ""), 129.5);
    assert_eq!(flex_item_x("justify-content:space-evenly", ""), 129.5);
    assert_eq!(flex_item_x("justify-content:safe end", ""), 259.0);
}

#[test]
fn qcss_c_flex_align_items_and_self_keywords() {
    assert_eq!(flex_item_y("align-items:start", "height:20px"), 0.0);
    assert_eq!(flex_item_y("align-items:self-start", "height:20px"), 0.0);
    assert_eq!(flex_item_y("align-items:end", "height:20px"), 80.0);
    assert_eq!(flex_item_y("align-items:self-end", "height:20px"), 80.0);
    assert_eq!(flex_item_y("align-items:last baseline", "height:20px"), 80.0);
    assert_eq!(flex_item_y("align-items:first baseline", "height:20px"), 0.0);
    assert_eq!(flex_item_y("align-items:center", "height:20px"), 40.0);
    assert_eq!(flex_item_y("align-items:end", "height:20px;align-self:start"), 0.0);
    assert_eq!(flex_item_y("align-items:end", "height:20px;align-self:auto"), 80.0);
    assert_eq!(flex_item_y("place-items:end", "height:20px"), 80.0);
    assert_eq!(flex_item_y("", "height:20px;place-self:center"), 40.0);
    // normal and stretch stretch an auto-height item.
    let html = r#"<html><body style="margin:0"><div style="display:flex;width:300px;height:100px;align-items:normal"><div style="margin-top:1px;width:40px"></div></div></body></html>"#;
    assert_eq!(boxed(html, 800.0, 1.0).h, 99.0, "stretched to the line minus the 1px margin");
}

#[test]
fn qcss_c_flex_place_content_wrapped_lines() {
    let html = |css: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display:flex;flex-wrap:wrap;width:100px;height:200px;{css}"><div style="width:97px;height:20px"></div></div></body></html>"#
        )
    };
    let y = |c: &str| find_by_width(&measure_fixture(&html(c), 800.0), 97.0).expect("item").y;
    assert_eq!(y("align-content:start"), 0.0);
    assert_eq!(y("align-content:end"), 180.0);
    assert_eq!(y("place-content:center"), 90.0);
}

fn grid(container_css: &str, item_css: &str) -> Laid {
    let html = format!(
        r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 60px;width:200px;{container_css}"><div style="width:41px;height:20px;{item_css}"></div></div></body></html>"#
    );
    find_by_width(&measure_fixture(&html, 800.0), 41.0).expect("item").clone()
}

#[test]
fn qcss_c_grid_justify_items_and_align_items() {
    let at = |c: &str, i: &str| {
        let l = grid(c, i);
        (l.x, l.y)
    };
    assert_eq!(at("", ""), (0.0, 0.0));
    assert_eq!(at("justify-items:center", ""), (29.5, 0.0));
    assert_eq!(at("justify-items:end", ""), (59.0, 0.0));
    assert_eq!(at("justify-items:right", ""), (59.0, 0.0));
    assert_eq!(at("justify-items:left", ""), (0.0, 0.0));
    assert_eq!(at("align-items:end", ""), (0.0, 40.0));
    assert_eq!(at("align-items:center", ""), (0.0, 20.0));
    assert_eq!(at("place-items:end center", ""), (29.5, 40.0));
    assert_eq!(at("place-items:center", ""), (29.5, 20.0));
    // The item's own *-self wins; auto defers to the container.
    assert_eq!(at("justify-items:end", "justify-self:start"), (0.0, 0.0));
    assert_eq!(at("justify-items:end", "justify-self:auto"), (59.0, 0.0));
    assert_eq!(at("", "place-self:end"), (59.0, 40.0));
    assert_eq!(at("align-items:center", "align-self:end"), (0.0, 40.0));
    assert_eq!(at("justify-items:safe center", ""), (29.5, 0.0));
}

#[test]
fn qcss_c_grid_stretch_and_normal_keep_filling_the_column() {
    let html = |c: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100px;{c}"><div style="margin-top:1px"></div></div></body></html>"#
        )
    };
    for c in ["", "justify-items:stretch", "justify-items:normal", "justify-items:legacy"] {
        assert_eq!(boxed(&html(c), 800.0, 1.0).w, 100.0, "{c}");
    }
    // A non-stretch auto-width item shrinks to its content (empty here: 0).
    assert_eq!(boxed(&html("justify-items:center"), 800.0, 1.0).w, 0.0);
}

#[test]
fn qcss_c_grid_item_with_explicit_width_is_positioned_not_resized() {
    let html = r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100px;justify-items:end"><div style="width:31px"></div></div></body></html>"#;
    let l = find_by_width(&measure_fixture(html, 800.0), 31.0).expect("item").clone();
    assert_eq!((l.w, l.x), (31.0, 69.0));
}

fn grid_flow_positions(flow: &str, n: usize) -> Vec<(f32, f32)> {
    let items: String = (1..=n)
        .map(|i| format!(r#"<div style="width:{}px;height:10px"></div>"#, 10 + i))
        .collect();
    let html = format!(
        r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:50px 50px;grid-template-rows:20px 20px;width:100px;{flow}">{items}</div></body></html>"#
    );
    let laid = measure_fixture(&html, 800.0);
    (1..=n)
        .map(|i| {
            let l = find_by_width(&laid, (10 + i) as f32).expect("item");
            (l.x, l.y)
        })
        .collect()
}

#[test]
fn qcss_c_grid_auto_flow_row_is_the_default() {
    let want = vec![(0.0, 0.0), (50.0, 0.0), (0.0, 20.0)];
    assert_eq!(grid_flow_positions("", 3), want);
    assert_eq!(grid_flow_positions("grid-auto-flow:row", 3), want);
}

#[test]
fn qcss_c_grid_auto_flow_column_fills_down_then_adds_columns() {
    let p = grid_flow_positions("grid-auto-flow:column", 5);
    assert_eq!(&p[..4], &[(0.0, 0.0), (0.0, 20.0), (50.0, 0.0), (50.0, 20.0)]);
    // The fifth item needs a third (implicit, auto-sized) column after the two 50px ones.
    assert_eq!(p[4], (100.0, 0.0));
}

#[test]
fn qcss_c_grid_auto_flow_dense_backfills_holes() {
    // Item 2 spans two columns and does not fit after item 1; item 3 fits the hole only when dense.
    let html = |flow: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:50px 50px 50px;width:150px;{flow}">
            <div style="width:11px;grid-column:span 2;height:10px"></div>
            <div style="width:12px;grid-column:span 2;height:10px"></div>
            <div style="width:13px;height:10px"></div></div></body></html>"#
        )
    };
    let pos = |flow: &str| {
        let laid = measure_fixture(&html(flow), 800.0);
        let l = find_by_width(&laid, 13.0).expect("item 3");
        (l.x, l.y)
    };
    // Sparse: item 3 follows item 2 (row 2, col 3).
    assert_eq!(pos(""), (100.0, 10.0));
    // Dense: item 3 back-fills row 1's free third column.
    assert_eq!(pos("grid-auto-flow:row dense"), (100.0, 0.0));
    assert_eq!(pos("grid-auto-flow:dense"), (100.0, 0.0));
}

#[test]
fn qcss_c_grid_gap_alias_spaces_tracks() {
    let html = r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:50px 50px;grid-gap:10px 20px"><div style="height:10px;width:11px"></div><div style="width:12px;height:10px"></div><div style="width:13px;height:10px"></div></div></body></html>"#;
    let laid = measure_fixture(html, 800.0);
    assert_eq!(find_by_width(&laid, 12.0).expect("item 2").x, 70.0);
    assert_eq!(find_by_width(&laid, 13.0).expect("item 3").y, 20.0);
}
