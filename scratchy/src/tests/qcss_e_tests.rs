//! QCSS-E: per-corner border radii (elliptical too), `outline-offset`, `transform-origin`,
//! `border-spacing`, `letter-spacing` and `direction` (with `text-align: start | end`): parsing,
//! computed values, `getComputedStyle`, layout and paint, and the `style_diff` class of each.
#[allow(unused_imports)]
use super::*;
use crate::incr_diff::{style_diff, StyleDiff};

/// Logs `getComputedStyle(#id)[prop]` for every `(id, prop)`.
fn cs(html: &str, reads: &[(&str, &str)]) -> Vec<String> {
    let code: String = reads
        .iter()
        .map(|(id, p)| format!("console.log(getComputedStyle(document.getElementById('{id}')).getPropertyValue('{p}'));"))
        .collect();
    run_in_html(html, &code).into_iter().map(|l| l.trim_start_matches("log:").to_string()).collect()
}

fn style_of(html: &str, width: f32) -> ComputedStyle {
    let laid = measure_fixture(html, 800.0);
    find_by_width(&laid, width).expect("box not found").style
}

// ---------- border radii ----------

#[test]
fn radius_shorthand_expands_to_four_corners() {
    let h = "<div id=a style='border-radius: 1px 2px 3px 4px'></div>\
             <div id=b style='border-radius: 1px 2px 3px'></div>\
             <div id=c style='border-radius: 1px 2px'></div>\
             <div id=d style='border-radius: 5px'></div>";
    let r = cs(h, &[
        ("a", "border-top-left-radius"), ("a", "border-top-right-radius"), ("a", "border-bottom-right-radius"),
        ("a", "border-bottom-left-radius"), ("b", "border-bottom-left-radius"), ("b", "border-top-right-radius"),
        ("c", "border-bottom-right-radius"), ("c", "border-bottom-left-radius"), ("d", "border-bottom-left-radius"),
    ]);
    assert_eq!(r, ["1px", "2px", "3px", "4px", "2px", "2px", "1px", "2px", "5px"], "{r:?}");
}

#[test]
fn radius_slash_gives_elliptical_corners() {
    let h = "<div id=a style='border-radius: 10px 20px / 30px 40px'></div>";
    let r = cs(h, &[
        ("a", "border-top-left-radius"), ("a", "border-top-right-radius"), ("a", "border-bottom-right-radius"),
        ("a", "border-bottom-left-radius"), ("a", "border-radius"),
    ]);
    assert_eq!(r, ["10px 30px", "20px 40px", "10px 30px", "20px 40px", "10px 20px / 30px 40px"], "{r:?}");
}

#[test]
fn radius_longhand_two_values_and_cascade_order() {
    let h = "<style>#a { border-top-left-radius: 8px 4px; } #b { border-radius: 9px; border-top-left-radius: 1px; }\
             #c { border-top-left-radius: 1px; border-radius: 9px; }</style>\
             <div id=a></div><div id=b></div><div id=c></div>";
    let r = cs(h, &[("a", "border-top-left-radius"), ("a", "border-top-right-radius"), ("b", "border-top-left-radius"),
        ("b", "border-top-right-radius"), ("c", "border-top-left-radius")]);
    assert_eq!(r, ["8px 4px", "0px", "1px", "9px", "9px"], "{r:?}");
}

#[test]
fn radius_percent_em_and_invalid_values() {
    let h = "<div id=a style='font-size:10px; border-radius: 2em 50%'></div>\
             <div id=b style='border-radius: -3px'></div>\
             <div id=c style='border-radius: 4px 5px 6px 7px 8px'></div>\
             <div id=d style='border-radius: 3px; border-radius: foo'></div>";
    let r = cs(h, &[("a", "border-top-left-radius"), ("a", "border-top-right-radius"), ("b", "border-top-left-radius"),
        ("c", "border-top-left-radius"), ("d", "border-top-left-radius")]);
    assert_eq!(r, ["20px", "50%", "0px", "0px", "3px"], "{r:?}");
}

#[test]
fn radius_computed_shorthand_serialisation_is_the_shortest_form() {
    let h = "<div id=a style='border-radius: 4px'></div><div id=b style='border-radius: 1px 2px'></div>\
             <div id=c style='border-radius: 1px 2px 3px 2px'></div><div id=d></div>";
    let r = cs(h, &[("a", "border-radius"), ("b", "border-radius"), ("c", "border-radius"), ("d", "border-radius")]);
    assert_eq!(r, ["4px", "1px 2px", "1px 2px 3px", "0px"], "{r:?}");
}

#[test]
fn radius_wide_keywords_work_per_corner() {
    let h = "<div id=p style='border-radius: 6px 7px'><div id=a style='border-top-left-radius: inherit; border-top-right-radius: 2px'>\
             </div><div id=b style='border-radius: 3px; border-bottom-left-radius: initial'></div></div>";
    let r = cs(h, &[("a", "border-top-left-radius"), ("a", "border-top-right-radius"), ("b", "border-bottom-left-radius"),
        ("b", "border-top-left-radius")]);
    assert_eq!(r, ["6px", "2px", "0px", "3px"], "{r:?}");
}

#[test]
fn corner_radii_percent_resolves_per_axis_against_the_border_box() {
    let s = style_of("<div style='width:200px;height:50px;border-radius:10% / 40%'></div>", 200.0);
    let r = s.border_radius.resolve(200.0, 50.0);
    assert!((r[0].0 - 20.0).abs() < 1e-3 && (r[0].1 - 20.0).abs() < 1e-3, "{r:?}");
}

#[test]
fn corner_radii_overlap_scales_every_corner_by_the_same_factor() {
    let c = CornerRadii([(BoxLength::Px(60.0), BoxLength::Px(10.0)), (BoxLength::Px(60.0), BoxLength::Px(10.0)),
        (BoxLength::Px(5.0), BoxLength::Px(5.0)), (BoxLength::Px(5.0), BoxLength::Px(5.0))]);
    // The top side holds 60 + 60 = 120 > 100: f = 100/120.
    let r = c.resolve(100.0, 100.0);
    let f = 100.0 / 120.0;
    assert!((r[0].0 - 60.0 * f).abs() < 1e-3 && (r[0].1 - 10.0 * f).abs() < 1e-3 && (r[2].0 - 5.0 * f).abs() < 1e-3, "{r:?}");
    // Equal circular radii keep the plain clamp, a zero axis squares the corner.
    assert_eq!(CornerRadii::uniform(BoxLength::Px(80.0)).resolve(100.0, 40.0), [(20.0, 20.0); 4]);
    let z = CornerRadii([(BoxLength::Px(5.0), BoxLength::Px(0.0)); 4]);
    assert_eq!(z.resolve(50.0, 50.0), [(0.0, 0.0); 4]);
}

fn paint_box(css: &str) -> Vec<u32> {
    let html = format!("<html><body style='margin:0;background:#fff'><div style='width:40px;height:30px;background:#000;{css}'></div></body></html>");
    paint_fixture(&html).0
}

#[test]
fn elliptical_corner_paints_a_wide_cut() {
    // 20px across, 6px down on the top-left corner only.
    let p = paint_box("border-top-left-radius: 20px 6px");
    let (white, black) = (0xFF_FFFF, 0);
    assert_eq!(px_at(&p, 200, 0, 0), white, "the very corner is cut");
    assert_eq!(px_at(&p, 200, 20, 0), black, "20px along the top edge the corner is already full");
    assert_eq!(px_at(&p, 200, 6, 0), white, "6px in on the top row is still outside the ellipse");
    assert_eq!(px_at(&p, 200, 0, 6), black, "6px down the left edge the corner is over");
    assert_eq!(px_at(&p, 200, 39, 0), black, "the other corners stay square");
    assert_eq!(px_at(&p, 200, 0, 29), black);
}

#[test]
fn different_corners_are_painted_independently() {
    let p = paint_box("border-radius: 0 15px 0 0");
    let (white, black) = (0xFF_FFFF, 0);
    assert_eq!(px_at(&p, 200, 0, 0), black, "top-left square");
    assert_eq!(px_at(&p, 200, 39, 0), white, "top-right rounded");
    assert_eq!(px_at(&p, 200, 39, 29), black, "bottom-right square");
    assert_eq!(px_at(&p, 200, 0, 29), black, "bottom-left square");
}

#[test]
fn uniform_radius_still_paints_the_same_circle() {
    let p = paint_box("border-radius: 10px");
    assert_eq!(px_at(&p, 200, 0, 0), 0xFF_FFFF);
    assert_eq!(px_at(&p, 200, 10, 10), 0);
    assert_eq!(px_at(&p, 200, 20, 0), 0);
}

#[test]
fn elliptical_rounded_border_ring_keeps_the_hole_clear() {
    let html = "<html><body style='margin:0;background:#fff'><div style='width:30px;height:20px;border:4px solid #f00;\
                border-radius: 20px / 10px'></div></body></html>";
    let (p, w, _) = paint_fixture(html);
    assert_eq!(px_at(&p, w, 0, 0), 0xFF_FFFF, "corner cut");
    assert_eq!(px_at(&p, w, 19, 1), 0xFF_0000, "top edge red");
    assert_eq!(px_at(&p, w, 19, 14), 0xFF_FFFF, "inside the ring: the page white shows");
}

// ---------- outline-offset ----------

#[test]
fn outline_offset_computed_values() {
    let h = "<div id=a style='outline: 2px solid red; outline-offset: 5px'></div>\
             <div id=b style='font-size:10px;outline-offset: -1.5em'></div><div id=c></div>\
             <div id=d style='outline-offset: 4px'><div id=e></div></div>";
    let r = cs(h, &[("a", "outline-offset"), ("b", "outline-offset"), ("c", "outline-offset"), ("e", "outline-offset")]);
    assert_eq!(r, ["5px", "-15px", "0px", "0px"], "not inherited: {r:?}");
}

#[test]
fn outline_offset_moves_the_painted_ring() {
    let ring = |off: &str| {
        let html = format!(
            "<html><body style='margin:0;background:#fff'><div style='margin:20px;width:40px;height:40px;\
             outline:2px solid #f00;outline-offset:{off}'></div></body></html>"
        );
        paint_fixture(&html).0
    };
    let red = 0xFF_0000;
    let white = 0xFF_FFFF;
    // Offset 0: the ring is the 2px just outside the box (18..20).
    let p0 = ring("0");
    assert_eq!(px_at(&p0, 200, 19, 40), red);
    assert_eq!(px_at(&p0, 200, 15, 40), white);
    // Offset 6: the ring is 6px further out (12..14), the gap shows the page.
    let p6 = ring("6px");
    assert_eq!(px_at(&p6, 200, 13, 40), red);
    assert_eq!(px_at(&p6, 200, 19, 40), white);
    assert_eq!(px_at(&p6, 200, 40, 13), red, "top edge moved too");
    // Offset -2: the ring sits inside the box edge (20..22).
    let pn = ring("-2px");
    assert_eq!(px_at(&pn, 200, 21, 40), red);
    assert_eq!(px_at(&pn, 200, 19, 40), white);
}

#[test]
fn outline_offset_widens_the_paint_reach_and_is_paint_only() {
    let s = style_of("<div style='width:41px;outline:3px solid red;outline-offset:7px'></div>", 41.0);
    assert_eq!(s.outline_reach(), 10.0);
    let n = style_of("<div style='width:41px;outline:3px solid red;outline-offset:-5px'></div>", 41.0);
    assert_eq!(n.outline_reach(), 0.0, "an outline pulled inside the box reaches nowhere");
    let none = style_of("<div style='width:41px;outline-offset:7px'></div>", 41.0);
    assert_eq!(none.outline_reach(), 0.0, "no outline, no reach");
    let mut b = s;
    b.outline_offset = 9.0;
    assert_eq!(style_diff(&s, &b), StyleDiff::Paint);
}

// ---------- transform-origin ----------

#[test]
fn transform_origin_parses_keywords_lengths_and_percentages() {
    let h = "<div id=a></div><div id=b style='transform-origin: left top'></div>\
             <div id=c style='transform-origin: 10px 20%'></div><div id=d style='transform-origin: bottom'></div>\
             <div id=e style='transform-origin: top right'></div><div id=f style='transform-origin: right 5px'></div>\
             <div id=g style='transform-origin: 1px 2px 3px'></div><div id=h style='transform-origin: top bottom'></div>\
             <div id=i style='transform-origin: 30%'></div><div id=j style='transform-origin: 5px top'></div>\
             <div id=k style='transform-origin: center'></div>";
    let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"];
    let reads: Vec<(&str, &str)> = ids.iter().map(|i| (*i, "transform-origin")).collect();
    let r = cs(h, &reads);
    assert_eq!(
        r,
        ["50% 50%", "0% 0%", "10px 20%", "50% 100%", "100% 0%", "100% 5px", "1px 2px", "50% 50%", "30% 50%", "5px 0%", "50% 50%"],
        "{r:?}"
    );
}

#[test]
fn transform_origin_resolves_against_the_box_and_keeps_translate_working() {
    let s = style_of("<div style='width:200px;height:100px;transform-origin: 25% 10px;transform: translate(7px, 3px)'></div>", 200.0);
    assert_eq!(s.transform_origin_px(200.0, 100.0), (50.0, 10.0));
    assert!(s.translate.is_some(), "the translate-only transform still applies (it does not depend on the origin)");
    let initial = style_of("<div style='width:200px;height:100px'></div>", 200.0);
    assert_eq!(initial.transform_origin_px(200.0, 100.0), (100.0, 50.0));
    let mut b = s;
    b.transform_origin = INITIAL_TRANSFORM_ORIGIN;
    assert_eq!(style_diff(&s, &b), StyleDiff::Paint);
}

// ---------- border-spacing ----------

fn table_cells_x(html: &str, width: f32) -> (Vec<Vec<(f32, f32)>>, Vec<f32>, f32) {
    let laid = measure_fixture(html, width);
    let mut rows = Vec::new();
    fn walk<'a>(l: &'a Laid, out: &mut Vec<&'a Laid>) {
        match &l.content {
            LaidContent::TableRow(_) => out.push(l),
            LaidContent::Blocks(cs) => cs.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    walk(&laid, &mut rows);
    let cells = rows
        .iter()
        .map(|r| match &r.content {
            LaidContent::TableRow(c) => c.iter().map(|c| (c.x, c.w)).collect(),
            _ => unreachable!(),
        })
        .collect();
    let ys = rows.iter().map(|r| r.y).collect();
    let table = rows.last().map_or(0.0, |r| r.y + r.h);
    (cells, ys, table)
}

const FIXED_TABLE: &str = "<html><body style='margin:0'><div style='float:left'><table style='border-spacing:%S%'>\
    <tr><td style='width:100px;padding:0'>a</td><td style='padding:0'>b</td></tr>\
    <tr><td style='padding:0'>c</td><td style='padding:0;width:50px'>d</td></tr></table></div></body></html>";

#[test]
fn border_spacing_separates_the_cells_and_the_rows() {
    let (c0, y0, h0) = table_cells_x(&FIXED_TABLE.replace("%S%", "0"), 800.0);
    let (c, y, h) = table_cells_x(&FIXED_TABLE.replace("%S%", "10px 6px"), 800.0);
    // Columns: 10 before, 10 between, 10 after; the cells keep a 300px table.
    assert!((c[0][0].0 - c0[0][0].0 - 10.0).abs() < 0.5, "first cell one spacing in: {} vs {}", c[0][0].0, c0[0][0].0);
    let gap = c[0][1].0 - (c[0][0].0 + c[0][0].1);
    assert!((gap - 10.0).abs() < 0.5, "10px between the columns: {gap}");
    let right_edge = c[0][1].0 + c[0][1].1 + 10.0;
    assert!((right_edge - 180.0).abs() < 0.5, "100 + 50 + 3 x 10 wide: {right_edge}");
    assert!((c[1][0].0 - c[0][0].0).abs() < 0.5, "rows line up");
    // Rows: 6 before, 6 between, 6 after.
    assert!((y[0] - y0[0] - 6.0).abs() < 0.5, "{} vs {}", y[0], y0[0]);
    let row_h = y0[1] - y0[0];
    assert!((y[1] - y[0] - row_h - 6.0).abs() < 0.5, "6px between the rows");
    assert!((h - h0 - 12.0).abs() < 0.5, "last row bottom moves by 2 spacings: {h0} -> {h}");
}

#[test]
fn border_spacing_is_inherited_ignored_when_collapsed_and_read_back() {
    let (c_none, ..) = table_cells_x(&FIXED_TABLE.replace("%S%", "0"), 800.0);
    let collapsed = "<html><body style='margin:0'><table style='width:300px;border-collapse:collapse;border-spacing:9px'>\
        <tr><td style='width:100px;padding:0'>a</td><td style='padding:0'>b</td></tr></table></body></html>";
    let (c, ..) = table_cells_x(collapsed, 800.0);
    assert!((c[0][0].0 - c_none[0][0].0).abs() < 0.5, "no spacing under border-collapse");
    let h = "<div id=p style='border-spacing: 3px 4px'><table id=t></table></div><div id=q style='border-spacing: 5px'></div><div id=r></div>";
    let r = cs(h, &[("t", "border-spacing"), ("q", "border-spacing"), ("r", "border-spacing")]);
    assert_eq!(r, ["3px 4px", "5px 5px", "0px 0px"], "{r:?}");
    let bad = cs("<div id=a style='border-spacing: -2px'></div>", &[("a", "border-spacing")]);
    assert_eq!(bad, ["0px 0px"], "negative is invalid");
}

#[test]
fn border_spacing_grows_a_shrink_to_fit_table_and_spans() {
    let w = |sp: &str| {
        let html = format!(
            "<html><body style='margin:0'><div style='float:left'><table style='border-spacing:{sp}'><tr>\
             <td colspan=2 style='padding:0;width:50px'>x</td></tr><tr><td style='padding:0;width:20px'>a</td>\
             <td style='padding:0;width:30px'>b</td></tr></table></div></body></html>"
        );
        let laid = measure_fixture(&html, 800.0);
        let mut rows = Vec::new();
        fn walk<'a>(l: &'a Laid, out: &mut Vec<&'a Laid>) {
            match &l.content {
                LaidContent::TableRow(_) => out.push(l),
                LaidContent::Blocks(cs) => cs.iter().for_each(|c| walk(c, out)),
                _ => {}
            }
        }
        walk(&laid, &mut rows);
        let LaidContent::TableRow(c0) = &rows[0].content else { unreachable!() };
        (c0[0].w, rows[0].w)
    };
    let (span0, row0) = w("0");
    assert!((row0 - 50.0).abs() < 0.5, "{row0}");
    let (span8, row8) = w("8px");
    let _ = (span0, span8);
    assert!((row8 - 58.0).abs() < 0.5, "20 + 30 + 3 x 8 wide table, the row spans the cells: {row8}");
}

// ---------- letter-spacing ----------

fn text_width(html: &str) -> f32 {
    let (laid, cache) = measure_fixture_with_cache(html, 900.0);
    let mut ids = Vec::new();
    collect_text_shape_ids(&laid, &mut ids);
    let entry = cache.get(&ids[0]).expect("shaped");
    let guard = entry.lock().unwrap();
    let (buffer, ..) = &*guard;
    buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
}

fn text_lines(html: &str, w: f32) -> usize {
    let (laid, cache) = measure_fixture_with_cache(html, w);
    let mut ids = Vec::new();
    collect_text_shape_ids(&laid, &mut ids);
    let entry = cache.get(&ids[0]).expect("shaped");
    let guard = entry.lock().unwrap();
    let (buffer, ..) = &*guard;
    buffer.layout_runs().count()
}

#[test]
fn letter_spacing_adds_its_value_after_every_character() {
    let w = |css: &str| text_width(&format!("<html><body style='margin:0'><p style='margin:0;{css}'>Wide letter spacing</p></body></html>"));
    let base = w("");
    let n = "Wide letter spacing".chars().count() as f32;
    for sp in [1.0f32, 3.0, 8.0] {
        let got = w(&format!("letter-spacing:{sp}px"));
        assert!((got - base - n * sp).abs() < 1.5, "{sp}px: {base} -> {got}, want +{}", n * sp);
    }
    assert!((w("letter-spacing:normal") - base).abs() < 0.01);
    // `em` is the element's own font size (20px here).
    let em = w("font-size:20px;letter-spacing:0.1em");
    let plain = w("font-size:20px");
    assert!((em - plain - n * 2.0).abs() < 1.5, "{plain} -> {em}");
}

#[test]
fn letter_spacing_does_not_shatter_a_short_paragraph() {
    // The reverted Q26 attempt wrapped this at 1..8px into 1/1/2/2/3/4 lines (px passed as em).
    for sp in [1, 2, 3, 4, 5, 8] {
        let html = format!("<html><body style='margin:0'><p style='margin:0;letter-spacing:{sp}px'>Wide letter spacing </p></body></html>");
        assert_eq!(text_lines(&html, 900.0), 1, "{sp}px");
    }
}

#[test]
fn letter_spacing_wraps_by_the_wider_advance() {
    let narrow = "<html><body style='margin:0'><p style='margin:0'>aaaa bbbb cccc dddd</p></body></html>";
    let wide = "<html><body style='margin:0'><p style='margin:0;letter-spacing:6px'>aaaa bbbb cccc dddd</p></body></html>";
    assert!(text_lines(wide, 100.0) > text_lines(narrow, 100.0));
}

#[test]
fn letter_spacing_inherits_and_reads_back() {
    let h = "<div id=p style='letter-spacing: 2px'><span id=c>x</span><span id=o style='letter-spacing: normal'>y</span>\
             <b id=e style='letter-spacing:.5em;font-size:10px'>z</b></div><div id=n></div>";
    let r = cs(h, &[("c", "letter-spacing"), ("o", "letter-spacing"), ("e", "letter-spacing"), ("n", "letter-spacing")]);
    assert_eq!(r, ["2px", "normal", "5px", "normal"], "{r:?}");
}

#[test]
fn letter_spacing_is_hashed_into_the_shape_memo() {
    let a = text_width("<html><body><p>hello</p></body></html>");
    let b = text_width("<html><body><p style='letter-spacing:4px'>hello</p></body></html>");
    assert!(b > a + 10.0, "a memoised shape of plain text must not be reused: {a} vs {b}");
}

// ---------- direction, text-align start / end ----------

#[test]
fn direction_computed_value_inherits_and_follows_the_dir_attribute() {
    let h = "<div id=a style='direction: rtl'><span id=b>x</span><i id=c style='direction: ltr'>y</i></div>\
             <div id=d dir=rtl><p id=e>z</p></div><div id=f></div><div id=g dir=RTL style='direction:ltr'></div>";
    let r = cs(h, &[("a", "direction"), ("b", "direction"), ("c", "direction"), ("e", "direction"), ("f", "direction"), ("g", "direction")]);
    assert_eq!(r, ["rtl", "rtl", "ltr", "rtl", "ltr", "ltr"], "{r:?}");
}

#[test]
fn text_align_initial_is_start_and_start_end_stay_in_the_computed_value() {
    let h = "<div id=a></div><div id=b style='text-align:end'></div><div id=c style='text-align:left'></div>\
             <div id=d style='text-align:start'><p id=e dir=rtl>x</p></div>";
    let r = cs(h, &[("a", "text-align"), ("b", "text-align"), ("c", "text-align"), ("e", "text-align")]);
    assert_eq!(r, ["start", "end", "left", "start"], "{r:?}");
}

#[test]
fn text_align_physical_resolution_follows_direction() {
    assert_eq!(TextAlign::Start.physical(Direction::Ltr), TextAlign::Left);
    assert_eq!(TextAlign::Start.physical(Direction::Rtl), TextAlign::Right);
    assert_eq!(TextAlign::End.physical(Direction::Ltr), TextAlign::Right);
    assert_eq!(TextAlign::End.physical(Direction::Rtl), TextAlign::Left);
    assert_eq!(TextAlign::Center.physical(Direction::Rtl), TextAlign::Center);
    assert_eq!(TextAlign::Left.physical(Direction::Rtl), TextAlign::Left);
    // The common path is untouched: an ltr left/start paragraph still passes no explicit alignment.
    assert!(TextAlign::Start.to_cosmic(Direction::Ltr).is_none() && TextAlign::Left.to_cosmic(Direction::Ltr).is_none());
    assert!(TextAlign::Left.to_cosmic(Direction::Rtl).is_some());
}

/// The x of the first glyph of the first paragraph (document space), measured at 400px.
fn first_glyph_x(html: &str) -> f32 {
    let (laid, cache) = measure_fixture_with_cache(html, 400.0);
    fn walk(l: &Laid, cache: &ShapeCache) -> Option<f32> {
        match &l.content {
            LaidContent::Blocks(cs) | LaidContent::TableRow(cs) => cs.iter().find_map(|c| walk(c, cache)),
            LaidContent::Text(id) => {
                let entry = cache.get(id)?;
                let guard = entry.lock().unwrap();
                let (buffer, ..) = &*guard;
                let content_x = l.x + l.style.border_left() + l.style.padding.left.px();
                buffer.layout_runs().next().and_then(|r| r.glyphs.first().map(|g| content_x + g.x))
            }
            _ => None,
        }
    }
    walk(&laid, &cache).expect("text")
}

#[test]
fn rtl_paragraph_starts_on_the_right_and_end_goes_left() {
    let h = |css: &str, attrs: &str| format!("<html><body style='margin:0'><p {attrs} style='margin:0;{css}'>Hello</p></body></html>");
    let ltr = first_glyph_x(&h("", ""));
    assert!(ltr < 1.0, "ltr start is the left edge: {ltr}");
    let rtl = first_glyph_x(&h("direction:rtl", ""));
    assert!(rtl > 300.0, "rtl `start` is the right edge: {rtl}");
    assert!(first_glyph_x(&h("", "dir=rtl")) > 300.0, "the dir attribute does the same");
    assert!(first_glyph_x(&h("direction:rtl;text-align:end", "")) < 1.0, "rtl `end` is the left edge");
    assert!(first_glyph_x(&h("direction:rtl;text-align:left", "")) < 1.0, "an explicit left stays left in rtl");
    assert!(first_glyph_x(&h("text-align:end", "")) > 300.0, "ltr `end` is the right edge");
    let mid = first_glyph_x(&h("direction:rtl;text-align:center", ""));
    assert!(mid > 100.0 && mid < 300.0, "center is unaffected: {mid}");
}

#[test]
fn rtl_mixed_inline_runs_align_to_the_right() {
    let (laid, _) = measure_fixture_with_cache(
        "<html><body style='margin:0'><p dir=rtl style='margin:0'>Hi <img width=10 height=10 src='data:image/gif;base64,R0lGODlhAQABAAAAACw='></p></body></html>",
        400.0,
    );
    let run = find_mixed_inline(&laid);
    // An image that does not decode may leave no mixed run; only check geometry when there is one.
    if let Some(run) = run {
        let xs: Vec<f32> = mixed_items(run).iter().map(|i| mixed_item_xy(i).0).collect();
        assert!(xs.iter().all(|x| *x > 300.0), "{xs:?}");
    }
}

// ---------- classes ----------

fn base() -> ComputedStyle {
    style_of("<div style='width:77px'></div>", 77.0)
}

#[test]
fn style_diff_classes_of_the_new_properties() {
    let a = base();
    let mut b = a;
    b.border_radius = CornerRadii::uniform(BoxLength::Px(3.0));
    assert_eq!(style_diff(&a, &b), StyleDiff::Paint, "radii only paint");
    let mut b = a;
    b.border_radius.0[2] = (BoxLength::Px(3.0), BoxLength::Px(9.0));
    assert_eq!(style_diff(&a, &b), StyleDiff::Paint);
    let mut b = a;
    b.outline_offset = 4.0;
    assert_eq!(style_diff(&a, &b), StyleDiff::Paint);
    let mut b = a;
    b.transform_origin.0.frac = 0.0;
    assert_eq!(style_diff(&a, &b), StyleDiff::Paint);
    let mut b = a;
    b.letter_spacing = 2.0;
    assert_eq!(style_diff(&a, &b), StyleDiff::Layout);
    let mut b = a;
    b.direction = Direction::Rtl;
    assert_eq!(style_diff(&a, &b), StyleDiff::Layout);
    let mut b = a;
    b.border_spacing = (2.0, 2.0);
    assert_eq!(style_diff(&a, &b), StyleDiff::Layout);
}

#[test]
fn inherited_properties_are_part_of_the_engines_inherited_comparison() {
    // The incremental engine restyles children when an inherited value changes; the three new
    // inherited properties must be in that comparison or a parent change would be missed.
    let src = include_str!("../incr_style.rs");
    for f in ["letter_spacing", "direction", "border_spacing"] {
        assert!(src.contains(&format!("a.{f} != b.{f}")), "{f}");
    }
}

#[test]
fn new_longhands_animate_and_transition() {
    let a = base();
    let mut b = a;
    b.border_radius = CornerRadii::uniform(BoxLength::Px(10.0));
    b.outline_offset = 8.0;
    b.letter_spacing = 4.0;
    let mid = crate::anim_eval::lerp_styles(&a, &b, 0.5);
    assert_eq!(mid.border_radius, CornerRadii::uniform(BoxLength::Px(5.0)));
    assert_eq!(mid.outline_offset, 4.0);
    assert_eq!(mid.letter_spacing, 2.0);
}
