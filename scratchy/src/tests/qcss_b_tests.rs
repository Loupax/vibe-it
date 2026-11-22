//! QCSS-B: CSS `aspect-ratio` (css-sizing-4): parsing, computed value, and layout sizes.
#[allow(unused_imports)]
use super::*;
use crate::css::AspectRatio;

/// The laid box whose `margin-top` is `mt` (a unique marker per test box).
fn boxed(html: &str, width: f32, mt: f32) -> Laid {
    let laid = measure_fixture(html, width);
    find_laid_by_margin_top(&laid, mt).expect("marked box not found").clone()
}

#[test]
fn qcss_b_parses_the_three_forms_and_rejects_garbage() {
    let st = |css: &str| {
        let html = format!(r#"<html><body><div style="width: 13px; {css}"></div></body></html>"#);
        let laid = measure_fixture(&html, 200.0);
        find_by_width(&laid, 13.0).expect("div").style.aspect_ratio
    };
    assert_eq!(st(""), AspectRatio::AUTO);
    assert_eq!(st("aspect-ratio: 16 / 9").explicit().map(|r| (r * 1000.0).round()), Some(1778.0));
    assert!(!st("aspect-ratio: 16/9").auto);
    assert!(st("aspect-ratio: auto 4 / 3").auto);
    assert!(st("aspect-ratio: 4 / 3 auto").auto);
    assert_eq!(st("aspect-ratio: 2").explicit(), Some(2.0));
    assert_eq!(st("aspect-ratio: 1.5 !important").explicit(), Some(1.5));
    assert_eq!(st("aspect-ratio: auto").explicit(), None);
    assert_eq!(st("aspect-ratio: 0 / 1").explicit(), None, "a degenerate ratio is no ratio");
    for bad in ["aspect-ratio: -1", "aspect-ratio: 1 / -2", "aspect-ratio: 1 / 2 / 3", "aspect-ratio: 10px", "aspect-ratio: auto auto"] {
        assert_eq!(st(bad), AspectRatio::AUTO, "{bad} is invalid and ignored");
    }
}

#[test]
fn qcss_b_get_computed_style_serialises() {
    let log = run_in_html(
        "<div id=a style='aspect-ratio:16/9'></div><div id=b style='aspect-ratio:auto 4 / 3'></div><div id=c></div>\
         <div id=d style='aspect-ratio:1'></div><div id=e style='aspect-ratio:1.5'></div>",
        "for (const id of 'abcde') console.log(id, getComputedStyle(document.getElementById(id)).aspectRatio);",
    );
    assert_eq!(log, ["log:a 16 / 9", "log:b auto 4 / 3", "log:c auto", "log:d 1 / 1", "log:e 1.5 / 1"], "{log:?}");
}

#[test]
fn qcss_b_block_height_from_width() {
    let l = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:320px;aspect-ratio:16/9"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((l.w, l.h), (320.0, 180.0));
}

#[test]
fn qcss_b_auto_width_block_fills_then_derives_height() {
    let l = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;aspect-ratio:2"></div></body></html>"#,
        400.0,
        1.0,
    );
    assert_eq!((l.w, l.h), (400.0, 200.0));
}

#[test]
fn qcss_b_ratio_applies_to_the_box_sizing_box() {
    let content = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;padding:10px;aspect-ratio:1"></div></body></html>"#,
        800.0,
        1.0,
    );
    // content-box: 100x100 content + padding.
    assert_eq!((content.w, content.h), (120.0, 120.0));
    let border = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;box-sizing:border-box;width:100px;padding:10px;aspect-ratio:2"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((border.w, border.h), (100.0, 50.0));
}

#[test]
fn qcss_b_explicit_height_wins() {
    let l = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;height:30px;aspect-ratio:1"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((l.w, l.h), (100.0, 30.0));
}

#[test]
fn qcss_b_min_height_auto_lets_content_grow_the_box() {
    // 100px wide, ratio 1 -> 100px, but 300px of content: overflow visible grows the box.
    let grow = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;aspect-ratio:1"><div style="height:300px"></div></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!(grow.h, 300.0);
    // A scroll container has an automatic minimum of 0: it stays at the ratio height.
    let clip = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;aspect-ratio:1;overflow:hidden"><div style="height:300px"></div></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!(clip.h, 100.0);
}

#[test]
fn qcss_b_max_and_min_height_clamp() {
    let max = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;aspect-ratio:1/2;max-height:80px"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!(max.h, 80.0);
    let min = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;aspect-ratio:2;min-height:70px"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!(min.h, 70.0);
}

#[test]
fn qcss_b_percent_height_child_resolves_against_the_ratio_box() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:200px;aspect-ratio:2"><div style="margin-top:1px;height:50%"></div></div></body></html>"#,
        800.0,
    );
    let child = find_laid_by_margin_top(&laid, 1.0).expect("child");
    assert_eq!(child.h, 50.0, "50% of the 100px ratio-derived height");
}

#[test]
fn qcss_b_auto_alone_changes_nothing() {
    let l = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;width:100px;aspect-ratio:auto"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((l.w, l.h), (100.0, 0.0));
}

#[test]
fn qcss_b_flex_item_in_row_gets_height_from_forced_width_and_is_not_stretched() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display:flex;align-items:stretch">
            <div style="margin-top:1px;width:100px;aspect-ratio:1"></div>
            <div style="margin-top:2px;width:50px;height:200px"></div></div></body></html>"#,
        800.0,
    );
    let a = find_laid_by_margin_top(&laid, 1.0).expect("a");
    assert_eq!((a.w, a.h), (100.0, 100.0), "keeps the ratio height instead of stretching to 200");
}

#[test]
fn qcss_b_flex_item_with_definite_height_gets_width_from_ratio() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display:flex">
            <div style="margin-top:1px;height:60px;aspect-ratio:2"></div></div></body></html>"#,
        800.0,
    );
    let a = find_laid_by_margin_top(&laid, 1.0).expect("a");
    assert_eq!((a.w, a.h), (120.0, 60.0));
}

#[test]
fn qcss_b_grid_item_stretches_width_and_derives_height() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:200px 100px">
            <div style="margin-top:1px;aspect-ratio:4/1"></div></div></body></html>"#,
        800.0,
    );
    let a = find_laid_by_margin_top(&laid, 1.0).expect("a");
    assert_eq!((a.w, a.h), (200.0, 50.0));
}

#[test]
fn qcss_b_flex_and_grid_containers_take_a_ratio_too() {
    let f = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;display:flex;width:300px;aspect-ratio:3"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((f.w, f.h), (300.0, 100.0));
    let g = boxed(
        r#"<html><body style="margin:0"><div style="margin-top:1px;display:grid;width:300px;aspect-ratio:3"></div></body></html>"#,
        800.0,
        1.0,
    );
    assert_eq!((g.w, g.h), (300.0, 100.0));
}

fn img_style(css: &str) -> ComputedStyle {
    let html = format!(r#"<html><body><div style="width: 13px; {css}"></div></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    find_laid_by_margin_top(&laid, 0.0).map(|_| ()).unwrap_or(());
    let mut st = find_by_width(&laid, 13.0).expect("div").style;
    st.width = None;
    st
}

#[test]
fn qcss_b_replaced_element_ratio() {
    let image = DecodedImage { width: 200, height: 100, pixels: vec![TextColor::rgb(1, 2, 3); 200 * 100], svg: None, mask: None };
    let size = |css: &str, w: Option<f32>, h: Option<f32>| {
        let mut s = img_style(css);
        s.width = w.map(BoxLength::Px);
        s.height = h.map(BoxLength::Px);
        crate::layout::image_box_size(&image, &s)
    };
    assert_eq!(size("", None, None), (200.0, 100.0));
    assert_eq!(size("aspect-ratio: auto", Some(100.0), None), (100.0, 50.0), "natural ratio");
    assert_eq!(size("aspect-ratio: 1", Some(100.0), None), (100.0, 100.0), "explicit ratio overrides the natural one");
    assert_eq!(size("aspect-ratio: 1", None, Some(80.0)), (80.0, 80.0));
    assert_eq!(size("aspect-ratio: 1", None, None), (200.0, 200.0), "natural width, ratio height");
    assert_eq!(size("aspect-ratio: auto 1", Some(100.0), None), (100.0, 50.0), "auto <ratio> prefers the natural ratio");
}
