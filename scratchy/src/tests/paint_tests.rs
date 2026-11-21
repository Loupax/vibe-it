#[allow(unused_imports)]
use super::*;

// ---------- v38: `border-radius` + a visible border (rounded ring) ----------

#[test]
fn rounded_bg_then_ring_combo() {
    const W: u32 = 40;
    const H: u32 = 40;
    let mut pixels = vec![0u32; (W * H) as usize];
    let bg = TextColor::rgb(0, 0, 255);
    let border = TextColor::rgb(255, 0, 0);
    fill_rounded_rect(&mut pixels, W, H, 0.0, 0.0, 40.0, 40.0, 10.0, bg, None);
    fill_rounded_ring(&mut pixels, W, H, 0.0, 0.0, 40.0, 40.0, 10.0, 5.0, border, None);
    let px = |x: u32, y: u32| pixels[(y * W + x) as usize];

    assert_eq!(px(0, 0), 0x000000, "corner should be untouched (cut by rounding)");
    assert_eq!(px(20, 2), 0x_FF0000, "top edge within border thickness should be red");
    assert_eq!(px(20, 20), 0x_0000FF, "center should show background through the ring's hole");
    assert_eq!(px(2, 20), 0x_FF0000, "left edge within border thickness should be red");
}

// ---------- v39: text-decoration reaching a block's own text ----------

fn collect_decorations(b: &LayoutBox, out: &mut Vec<TextDecoration>) {
    if let BoxContent::Inline(items) = &b.content {
        for item in items {
            if let InlineItem::Text(_, s) = item {
                out.push(s.text_decoration);
            }
        }
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            collect_decorations(c, out);
        }
    }
}

#[test]
fn block_own_direct_text_gets_its_own_decoration() {
    let doc = parse_html(r#"<html><body><p style="text-decoration: underline">text</p></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut decos = Vec::new();
    collect_decorations(&tree, &mut decos);
    assert!(decos.iter().any(|d| *d != TextDecoration::new()), "p's own direct text should be underlined: {:?}", decos);
}

#[test]
fn decoration_propagates_through_nested_block_descendant() {
    let doc = parse_html(
        r#"<html><body><div style="text-decoration: underline"><div>nested</div></div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut decos = Vec::new();
    collect_decorations(&tree, &mut decos);
    assert!(decos.iter().any(|d| *d != TextDecoration::new()), "nested block descendant should inherit ambient decoration: {:?}", decos);
}

#[test]
fn nested_block_can_opt_out_of_decoration_with_none() {
    let doc = parse_html(
        r#"<html><body><div style="text-decoration: underline"><div style="text-decoration: none">nested</div></div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut decos = Vec::new();
    collect_decorations(&tree, &mut decos);
    assert!(decos.iter().all(|d| *d == TextDecoration::new()), "explicit none should opt out: {:?}", decos);
}

#[test]
fn sibling_block_does_not_leak_decoration() {
    let doc = parse_html(
        r#"<html><body><div style="text-decoration: underline">a</div><div>b</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut decos = Vec::new();
    collect_decorations(&tree, &mut decos);
    assert_eq!(decos.len(), 2, "expected exactly 2 spans: {:?}", decos);
    assert_ne!(decos[0], TextDecoration::new(), "first div should be underlined: {:?}", decos);
    assert_eq!(decos[1], TextDecoration::new(), "second (sibling) div should NOT inherit the first's decoration: {:?}", decos);
}

/// QINC1k1: an edge a few f32 ulps below a pixel boundary (a `bottom: 0` box ending at 484.99957
/// instead of 485) still covers that pixel; an edge well inside a pixel still does not.
#[test]
fn qinc1k1_fill_rect_edges_within_rounding_noise_snap_to_the_pixel_boundary() {
    let color = TextColor::rgb(255, 0, 0);
    let painted_rows = |y: f32, h: f32| {
        let mut pixels = vec![0u32; 4 * 20];
        fill_rect(&mut pixels, 4, 20, 0.0, y, 4.0, h, color, None);
        (0..20u32).filter(|r| pixels[(r * 4) as usize] != 0).collect::<Vec<_>>()
    };
    assert_eq!(painted_rows(5.0, 9.99957), (5..15).collect::<Vec<_>>(), "bottom edge 14.99957 counts as 15");
    assert_eq!(painted_rows(4.99957, 10.0), (5..15).collect::<Vec<_>>(), "top edge 4.99957 counts as 5");
    assert_eq!(painted_rows(5.0, 9.9), (5..14).collect::<Vec<_>>(), "14.9 is still inside row 14");
}

// ---------- v40: `overflow: hidden` clips box decoration ----------

#[test]
fn fill_rect_respects_clip() {
    let mut pixels = vec![0u32; 20 * 20];
    let color = TextColor::rgb(255, 0, 0);
    fill_rect(&mut pixels, 20, 20, 0.0, 0.0, 20.0, 20.0, color, Some((5.0, 5.0, 5.0, 5.0)));
    for y in 0..20u32 {
        for x in 0..20u32 {
            let painted = pixels[(y * 20 + x) as usize] != 0;
            let in_clip = (5..10).contains(&x) && (5..10).contains(&y);
            assert_eq!(painted, in_clip, "pixel ({x},{y}) painted={painted} in_clip={in_clip}");
        }
    }
}

#[test]
fn nested_block_decoration_clipped_by_ancestor_overflow_hidden() {
    let doc = parse_html(
        r#"<html><body>
<div style="overflow: hidden; padding: 10px; background-color: #0000ff;">
  <div style="margin-left: 500px; background-color: #ff0000; padding: 5px;">inner</div>
</div>
</body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, 200.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    );
    let mut swash_cache = SwashCache::new();
    let win_w = 800u32;
    let win_h = 200u32;
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);

    let has_red = pixels.iter().any(|&p| (p & 0x00FF0000) >> 16 == 0xFF && (p & 0xFF) == 0x00 && ((p >> 8) & 0xFF) == 0x00);
    assert!(!has_red, "inner div's red background should be fully clipped by ancestor overflow:hidden");
    let has_blue = pixels.contains(&0x0000FF);
    assert!(has_blue, "outer div's own background should still paint");
}

// ---------- border-style bevel variants: groove/ridge/inset/outset (v63) ----------

/// Reads back the plain `0x00RRGGBB` pixel at `(x, y)` out of a
/// `paint_border`-painted buffer — same masking `px_at` uses, kept
/// local to this section since these tests paint directly into a
/// hand-rolled buffer rather than through `paint_fixture`.
fn bevel_px(pixels: &[u32], win_w: u32, x: f32, y: f32) -> u32 {
    pixels[(y as u32 * win_w + x as u32) as usize] & 0x00FF_FFFF
}

#[test]
fn inset_border_paints_top_left_edges_darker_than_bottom_right() {
    // Mid-gray so `lighten`/`darken` produce two values that are
    // both clearly different from the base and from each other.
    let base = TextColor::rgb(128, 128, 128);
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
    let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
    paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Inset, base, None);

    let top = bevel_px(&pixels, win_w, x + w / 2.0, y + bw / 2.0);
    let left = bevel_px(&pixels, win_w, x + bw / 2.0, y + h / 2.0);
    let bottom = bevel_px(&pixels, win_w, x + w / 2.0, y + h - bw / 2.0);
    let right = bevel_px(&pixels, win_w, x + w - bw / 2.0, y + h / 2.0);

    let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    assert_eq!(top, dark, "inset's top edge should be the darkened variant");
    assert_eq!(left, dark, "inset's left edge should be the darkened variant");
    assert_eq!(bottom, light, "inset's bottom edge should be the lightened variant");
    assert_eq!(right, light, "inset's right edge should be the lightened variant");
    assert_ne!(dark, light, "the two computed variants must actually differ for this test to mean anything");
}

#[test]
fn outset_border_is_the_reverse_of_inset() {
    let base = TextColor::rgb(128, 128, 128);
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
    let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
    paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Outset, base, None);

    let top = bevel_px(&pixels, win_w, x + w / 2.0, y + bw / 2.0);
    let left = bevel_px(&pixels, win_w, x + bw / 2.0, y + h / 2.0);
    let bottom = bevel_px(&pixels, win_w, x + w / 2.0, y + h - bw / 2.0);
    let right = bevel_px(&pixels, win_w, x + w - bw / 2.0, y + h / 2.0);

    let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    assert_eq!(top, light, "outset's top edge should be the lightened variant (opposite of inset)");
    assert_eq!(left, light, "outset's left edge should be the lightened variant");
    assert_eq!(bottom, dark, "outset's bottom edge should be the darkened variant");
    assert_eq!(right, dark, "outset's right edge should be the darkened variant");
}

#[test]
fn groove_border_splits_each_edge_into_an_outer_dark_and_inner_light_half() {
    // `Groove`'s own "carved in" crease: unlike `Inset`, each single
    // edge shows *two* shades, not one — the half farther from this
    // box's own content ("outer") dark, the half closer to it
    // ("inner") light, for the top/left edges; reversed for
    // bottom/right. `bw = 10` → `half = 5`, so the outer/inner
    // sample points below (2px/8px into a 10px-thick edge) land
    // solidly inside their own half with no rounding ambiguity.
    let base = TextColor::rgb(128, 128, 128);
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
    let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
    paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Groove, base, None);

    let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;

    // Top edge: outer (upper, y=12) dark, inner (lower, y=18) light.
    assert_eq!(bevel_px(&pixels, win_w, x + w / 2.0, y + 2.0), dark, "groove top edge's outer half should be dark");
    assert_eq!(bevel_px(&pixels, win_w, x + w / 2.0, y + 8.0), light, "groove top edge's inner half should be light");
    // Left edge: outer (left, x=12) dark, inner (right, x=18) light.
    assert_eq!(bevel_px(&pixels, win_w, x + 2.0, y + h / 2.0), dark, "groove left edge's outer half should be dark");
    assert_eq!(bevel_px(&pixels, win_w, x + 8.0, y + h / 2.0), light, "groove left edge's inner half should be light");
    // Bottom edge: inner (upper, closer to content) dark, outer (lower) light.
    assert_eq!(
        bevel_px(&pixels, win_w, x + w / 2.0, y + h - 8.0),
        dark,
        "groove bottom edge's inner half should be dark"
    );
    assert_eq!(
        bevel_px(&pixels, win_w, x + w / 2.0, y + h - 2.0),
        light,
        "groove bottom edge's outer half should be light"
    );
    // Right edge: inner (left, closer to content) dark, outer (right) light.
    assert_eq!(
        bevel_px(&pixels, win_w, x + w - 8.0, y + h / 2.0),
        dark,
        "groove right edge's inner half should be dark"
    );
    assert_eq!(
        bevel_px(&pixels, win_w, x + w - 2.0, y + h / 2.0),
        light,
        "groove right edge's outer half should be light"
    );
}

#[test]
fn ridge_border_is_the_reverse_of_groove_at_every_sampled_point() {
    let base = TextColor::rgb(128, 128, 128);
    let (win_w, win_h) = (100u32, 100u32);
    let mut groove_pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
    let mut ridge_pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
    let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
    paint_border(&mut groove_pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Groove, base, None);
    paint_border(&mut ridge_pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Ridge, base, None);

    let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
    let sample_points = [
        (x + w / 2.0, y + 2.0),       // top, outer
        (x + w / 2.0, y + 8.0),       // top, inner
        (x + 2.0, y + h / 2.0),       // left, outer
        (x + 8.0, y + h / 2.0),       // left, inner
        (x + w / 2.0, y + h - 8.0),   // bottom, inner
        (x + w / 2.0, y + h - 2.0),   // bottom, outer
        (x + w - 8.0, y + h / 2.0),   // right, inner
        (x + w - 2.0, y + h / 2.0),   // right, outer
    ];
    for (sx, sy) in sample_points {
        let g = bevel_px(&groove_pixels, win_w, sx, sy);
        let r = bevel_px(&ridge_pixels, win_w, sx, sy);
        assert_ne!(g, r, "groove and ridge should disagree at every sampled point ({sx}, {sy})");
        assert!(g == dark || g == light, "groove's own sample should be one of the two bevel shades");
        assert!(r == dark || r == light, "ridge's own sample should be one of the two bevel shades");
        assert_eq!(g, if r == dark { light } else { dark }, "ridge should be exactly groove's opposite shade at this point");
    }
}

#[test]
fn q4_per_side_borders_parse_and_resolve() {
    let div_of = |style: &str, width: f32| {
        let html = format!(r#"<html><body><div style="width: {width}px; {style}">x</div></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        find_by_width(&laid, width).expect("div not found").style
    };
    let s = div_of("border-bottom: 2px solid red;", 301.0);
    assert_eq!(s.border_width, [0.0, 0.0, 2.0, 0.0]);
    assert!(s.border_color[2].is_some() && s.border_color[0].is_none());
    let s = div_of("border-width: 1px 2px 3px 4px; border-style: solid;", 302.0);
    assert_eq!(s.border_width, [1.0, 2.0, 3.0, 4.0]);
    let s = div_of("border: 1px solid red; border-top: none;", 303.0);
    assert_eq!(s.border_width, [0.0, 1.0, 1.0, 1.0]);
    let s = div_of("border-left-width: 5px; border-left-style: dashed; border-left-color: blue;", 304.0);
    assert_eq!(s.border_width, [0.0, 0.0, 0.0, 5.0]);
    assert_eq!(s.border_style[3], BorderStyle::Dashed);
    let s = div_of("border: 1px solid; border-color: red green;", 305.0);
    assert_eq!(s.border_color[0], s.border_color[2]);
    assert_ne!(s.border_color[0], s.border_color[1]);
}

#[test]
fn q4b_per_side_borders_miter_round_and_bevel() {
    // Mitered join: 10px red top, 2px blue left; a pixel just right of
    // the left strip near the top-left corner is red (top claims it),
    // one far down the left strip is blue.
    let (pixels, win_w, _h) = paint_fixture(
        r#"<html><body style="margin:0"><div style="box-sizing: border-box; width: 60px; height: 60px; border-top: 10px solid #ff0000; border-left: 2px solid #0000ff;"></div></body></html>"#,
    );
    let red = px_at(&pixels, win_w, 5, 3);
    let blue = px_at(&pixels, win_w, 1, 30);
    assert_ne!(red, blue);
    assert_eq!(px_at(&pixels, win_w, 1, 3), red, "top side owns the mitered corner above the diagonal");
    assert_eq!(px_at(&pixels, win_w, 1, 9), blue, "left side owns the corner below the diagonal");
    // Rounded: the outer corner pixel is not painted with unequal sides.
    let (pixels, win_w, _h) = paint_fixture(
        r#"<html><body style="margin:0;background:#ffffff"><div style="box-sizing: border-box; width: 60px; height: 60px; border-radius: 20px; border-top: 6px solid #ff0000; border-left: 6px solid #0000ff;"></div></body></html>"#,
    );
    let bg = px_at(&pixels, win_w, 100, 30);
    assert_eq!(px_at(&pixels, win_w, 0, 0), bg, "outer corner is rounded away");
    assert_ne!(px_at(&pixels, win_w, 30, 2), bg, "top edge still painted");
    // Bevel in per-side path: inset top and left are dark, differ from a right edge in outset.
    let (pixels, win_w, _h) = paint_fixture(
        r#"<html><body style="margin:0"><div style="box-sizing: border-box; width: 60px; height: 60px; border-top: 8px inset gray; border-bottom: 8px inset gray;"></div></body></html>"#,
    );
    assert_ne!(px_at(&pixels, win_w, 30, 3), px_at(&pixels, win_w, 30, 56), "inset top (dark) and bottom (light) differ");
}

#[test]
fn q4c_uniform_non_solid_borders_round_with_border_radius() {
    for style in ["dashed", "dotted", "double", "inset", "groove"] {
        let html = format!(
            r#"<html><body style="margin:0;background:#ffffff"><div style="box-sizing: border-box; width: 60px; height: 60px; border-radius: 24px; border: 8px {style} #ff0000;"></div></body></html>"#
        );
        let (pixels, win_w, _h) = paint_fixture(&html);
        let bg = px_at(&pixels, win_w, 100, 30);
        assert_eq!(px_at(&pixels, win_w, 0, 0), bg, "{style}: outer corner is rounded away");
        assert_eq!(px_at(&pixels, win_w, 5, 5), bg, "{style}: still outside the curve");
    }
    // Solid dashed-with-radius still paints the top edge somewhere.
    let (pixels, win_w, _h) = paint_fixture(
        r#"<html><body style="margin:0;background:#ffffff"><div style="box-sizing: border-box; width: 60px; height: 60px; border-radius: 24px; border: 8px double #ff0000;"></div></body></html>"#,
    );
    let bg = px_at(&pixels, win_w, 100, 30);
    assert!((24..36).any(|x| px_at(&pixels, win_w, x, 1) != bg), "double border painted along the top");
}

#[test]
fn border_style_longhand_parses_all_four_bevel_keywords() {
    for (keyword, width, expected) in [
        ("groove", 201.0, BorderStyle::Groove),
        ("ridge", 202.0, BorderStyle::Ridge),
        ("inset", 203.0, BorderStyle::Inset),
        ("outset", 204.0, BorderStyle::Outset),
    ] {
        let html = format!(r#"<html><body><div style="width: {width}px; border-style: {keyword};">x</div></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let div = find_by_width(&laid, width).unwrap_or_else(|| panic!("div with border-style: {keyword} not found"));
        assert_eq!(
            div.style.border_style[0], expected,
            "border-style: {keyword} should resolve to BorderStyle::{expected:?}"
        );
    }
}

#[test]
fn border_style_inset_paints_visibly_different_shades_through_the_real_pipeline() {
    // End-to-end through `paint_box_decoration`'s own real
    // `paint_border` call (not the primitive tested directly above)
    // — confirms the new bevel branch is actually reached from a
    // real laid-out, painted document, not just callable in
    // isolation.
    let (pixels, win_w, _win_h) = paint_fixture(
        r#"<html><body style="margin:0"><div style="box-sizing: border-box; width: 100px; height: 100px; border: 10px solid gray; border-style: inset;"></div></body></html>"#,
    );
    let top = px_at(&pixels, win_w, 50, 5);
    let bottom = px_at(&pixels, win_w, 50, 95);
    assert_ne!(top, bottom, "an inset border's top and bottom edges should paint visibly different shades");
}

// ---------- v62: `overflow: scroll`/`auto` ----------

/// Depth-first search for the first box with its own `ScrollInfo`
/// (an `overflow: scroll`/`auto` box) — the scroll analogue of
/// `find_laid_by_position`, just above.
fn find_laid_with_scroll(laid: &Laid) -> Option<&Laid> {
    if laid.scroll.is_some() {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_laid_with_scroll(c) {
                return Some(found);
            }
        }
    }
    None
}

/// Finds a box by its own exact laid-out height — used instead of
/// `find_laid_by_margin_top` wherever a fixture's boxes all happen to
/// share the real CSS initial margin (`0`, e.g. every plain `<div>`
/// here), which would make that finder match the very first box in
/// the whole tree (even the synthetic document-root wrapper) instead
/// of the one a test actually means.
fn find_laid_by_height(laid: &Laid, target: f32) -> Option<&Laid> {
    if laid.h == target {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_laid_by_height(c, target) {
                return Some(found);
            }
        }
    }
    None
}

/// Same as `find_laid_with_scroll`, but collects every match instead
/// of stopping at the first — needed by tests with more than one
/// scrollable region on the same page (e.g. the wheel-routing test),
/// to tell them apart by their own distinct `x`/`y`/`id`.
fn find_all_laid_with_scroll<'a>(laid: &'a Laid, out: &mut Vec<&'a Laid>) {
    if laid.scroll.is_some() {
        out.push(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            find_all_laid_with_scroll(c, out);
        }
    }
}

#[test]
fn overflow_scroll_assigns_a_scroll_id_and_computes_content_vs_client_height() {
    let laid = measure_fixture(
        r#"<html><body><div style="overflow: scroll; height: 50px;">
            <p style="margin: 0;">one</p>
            <p style="margin: 0;">two</p>
            <p style="margin: 0;">three</p>
            <p style="margin: 0;">four</p>
        </div></body></html>"#,
        400.0,
    );
    let scroll_box = find_laid_with_scroll(&laid).expect("overflow: scroll box should get its own ScrollInfo");
    let info = scroll_box.scroll.expect("already found via find_laid_with_scroll");
    assert_eq!(info.client_h, 50.0, "client_h should be this box's own clamped content-box height");
    assert!(
        info.content_h > info.client_h,
        "four 20px-tall paragraphs (80px) should overflow a 50px client height: content_h={}, client_h={}",
        info.content_h,
        info.client_h
    );
    assert_eq!(
        info.max_scroll(),
        info.content_h - info.client_h,
        "max_scroll should be exactly the overflowing amount"
    );
}

#[test]
fn overflow_auto_only_scrollable_when_content_actually_overflows() {
    let fits = measure_fixture(
        r#"<html><body><div style="overflow: auto; height: 200px;"><p style="margin: 0;">one short line</p></div></body></html>"#,
        400.0,
    );
    let fits_box = find_laid_with_scroll(&fits).expect("overflow: auto box should still get a ScrollInfo");
    assert_eq!(
        fits_box.scroll.unwrap().max_scroll(),
        0.0,
        "content shorter than the box's own height shouldn't be scrollable at all"
    );

    let overflows = measure_fixture(
        r#"<html><body><div style="overflow: auto; height: 20px;">
            <p style="margin: 0;">one</p>
            <p style="margin: 0;">two</p>
            <p style="margin: 0;">three</p>
        </div></body></html>"#,
        400.0,
    );
    let overflow_box = find_laid_with_scroll(&overflows).expect("overflow: auto box should get a ScrollInfo");
    assert!(
        overflow_box.scroll.unwrap().max_scroll() > 0.0,
        "three 20px paragraphs inside a 20px-tall box should actually overflow and be scrollable"
    );
}

#[test]
fn scroll_offset_clips_and_translates_painted_content() {
    // No inter-tag whitespace — same reason every other exact-pixel
    // test in this file avoids it (a stray text node between tags
    // would become its own anonymous inline box and shift things).
    let laid = measure_fixture(
        concat!(
            r#"<html><body><div style="overflow: scroll; height: 50px; width: 100px; background-color: blue;">"#,
            r#"<div style="background-color: red; height: 20px; margin-top: 100px;"></div>"#,
            "</div></body></html>",
        ),
        400.0,
    );
    let scroll_box = find_laid_with_scroll(&laid).expect("overflow: scroll box not found");
    let info = scroll_box.scroll.expect("already confirmed Some above");
    let max_scroll = info.max_scroll();
    assert!(max_scroll > 0.0, "the red child (100px margin-top + 20px height) should overflow the 50px box");

    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (win_w, win_h) = (200u32, 200u32);

    // At the default (no entry at all, i.e. `0.0`) scroll offset, the
    // red child sits far below the 50px-tall visible window — fully
    // clipped. The outer box's own blue background is unaffected by
    // its *own* overflow (an element's `overflow` clips its content,
    // never itself — same rule v31/v40 already established).
    let mut pixels_unscrolled = vec![0u32; (win_w * win_h) as usize];
    paint(
        &laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_unscrolled, win_w,
        win_h, 0.0, None,
    );
    assert!(!pixels_unscrolled.contains(&0xFF0000), "the red child should be clipped away before any scrolling");
    assert!(pixels_unscrolled.contains(&0x0000FF), "the blue outer box should still paint regardless of its own scroll");

    // Scrolled all the way to `max_scroll`, the red child's bottom
    // edge lines up with the box's own bottom edge — now visible.
    let mut scroll_offsets = HashMap::new();
    scroll_offsets.insert(info.id, max_scroll);
    let mut pixels_scrolled = vec![0u32; (win_w * win_h) as usize];
    paint(
        &laid, &mut font_system, &mut swash_cache, &mut cache, &scroll_offsets, &mut pixels_scrolled, win_w,
        win_h, 0.0, None,
    );
    assert!(
        pixels_scrolled.contains(&0xFF0000),
        "scrolling to max_scroll should bring the red child into the clipped view"
    );
    assert!(pixels_scrolled.contains(&0x0000FF), "the blue outer box should still paint after scrolling too");
}

#[test]
fn q24_flex_grid_and_table_containers_can_be_scroll_regions() {
    for display in ["flex", "grid", "table"] {
        let html = format!(
            concat!(
                r#"<html><body style="margin:0"><div style="display: {d}; overflow: auto; height: 50px; width: 100px;">"#,
                r#"<div style="height: 120px; width: 40px;"></div></div></body></html>"#,
            ),
            d = display
        );
        let laid = measure_fixture(&html, 400.0);
        let b = find_laid_with_scroll(&laid).unwrap_or_else(|| panic!("display:{display} overflow:auto got no ScrollInfo"));
        let info = b.scroll.unwrap();
        assert_eq!(info.client_h, 50.0, "{display}: client_h");
        assert!(info.max_scroll() > 0.0, "{display}: content_h {} should overflow", info.content_h);
    }
}

#[test]
fn q24_absolute_child_scrolls_with_its_scroll_container() {
    let laid = measure_fixture(
        concat!(
            r#"<html><body style="margin:0"><div style="position: relative; overflow: scroll; height: 50px; width: 100px;">"#,
            r#"<div style="height: 200px;"></div>"#,
            r#"<div style="position: absolute; top: 100px; left: 0; width: 20px; height: 20px; background-color: red;"></div>"#,
            "</div></body></html>",
        ),
        400.0,
    );
    let info = find_laid_with_scroll(&laid).unwrap().scroll.unwrap();
    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (w, h) = (200u32, 200u32);
    let mut before = vec![0u32; (w * h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut before, w, h, 0.0, None);
    assert!(!before.contains(&0xFF0000), "absolute child at top:100px is below the 50px window");
    let mut offsets = HashMap::new();
    offsets.insert(info.id, 80.0);
    let mut after = vec![0u32; (w * h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &offsets, &mut after, w, h, 0.0, None);
    assert!(after.contains(&0xFF0000), "scrolling by 80 should bring the absolute child (now at y=20) into view");
}

#[test]
fn q24_nested_scroll_regions_route_the_wheel_to_the_innermost() {
    let laid = measure_fixture(
        concat!(
            r#"<html><body style="margin:0"><div style="overflow: scroll; height: 100px; width: 100px;">"#,
            r#"<div style="overflow: scroll; height: 50px;"><div style="height: 150px;"></div></div>"#,
            r#"<div style="height: 200px;"></div>"#,
            "</div></body></html>",
        ),
        400.0,
    );
    let mut all = Vec::new();
    find_all_laid_with_scroll(&laid, &mut all);
    assert_eq!(all.len(), 2, "outer and inner regions");
    let outer = all[0].scroll.unwrap();
    let inner = all[1].scroll.unwrap();
    let none = HashMap::new();
    let id_at = |y: f32, offs: &HashMap<usize, f32>| hit_test_scroll_region(&laid, 5.0, y, offs, 0.0, 600.0).map(|i| i.id);
    assert_eq!(id_at(25.0, &none), Some(inner.id), "over the inner region");
    assert_eq!(id_at(75.0, &none), Some(outer.id), "below the inner region: outer");
    let mut offs = HashMap::new();
    offs.insert(outer.id, 30.0);
    assert_eq!(id_at(10.0, &offs), Some(inner.id), "outer scrolled 30: y=10 is content y=40, still inner");
    assert_eq!(id_at(25.0, &offs), Some(outer.id), "outer scrolled 30: y=25 is content y=55, past inner");
}

#[test]
fn wheel_routing_picks_the_scroll_region_under_the_cursor_not_the_page() {
    // Two independent (sibling, not nested) `overflow` regions plus
    // one plain, non-scrollable paragraph — enough to confirm
    // `hit_test_scroll_region` tells the two scrollable regions
    // apart by position and correctly falls through to `None`
    // (→ the page's own top-level scroll) outside both.
    let laid = measure_fixture(
        concat!(
            r#"<html><body>"#,
            r#"<div style="overflow: scroll; height: 50px; width: 100px;">first region text</div>"#,
            r#"<div style="overflow: auto; height: 50px; width: 100px;">second region needs more text than fits so it overflows its own fifty pixel height for real this time yes</div>"#,
            r#"<p style="height: 30px; margin: 0;">not scrollable at all</p>"#,
            "</body></html>",
        ),
        400.0,
    );
    let mut regions = Vec::new();
    find_all_laid_with_scroll(&laid, &mut regions);
    assert_eq!(regions.len(), 2, "both overflow regions should each get their own ScrollInfo");
    let (first, second) = (regions[0], regions[1]);
    assert_ne!(first.scroll.unwrap().id, second.scroll.unwrap().id, "each region should get a distinct id");

    let scroll_offsets = HashMap::new();
    let viewport_h = 600.0;

    // A point inside the first region routes to it, not the second.
    let (fx, fy) = (first.x + 5.0, first.y + 5.0);
    let hit_first = hit_test_scroll_region(&laid, fx, fy, &scroll_offsets, 0.0, viewport_h);
    assert_eq!(
        hit_first.map(|i| i.id),
        Some(first.scroll.unwrap().id),
        "a point inside the first scrollable region should route to it"
    );

    // A point inside the second region routes to *that* one instead.
    let (sx, sy) = (second.x + 5.0, second.y + 5.0);
    let hit_second = hit_test_scroll_region(&laid, sx, sy, &scroll_offsets, 0.0, viewport_h);
    assert_eq!(
        hit_second.map(|i| i.id),
        Some(second.scroll.unwrap().id),
        "a point inside the second scrollable region should route to it, not the first"
    );

    // A point over the plain, non-scrollable `<p>` falls through to
    // `None` — the caller's own cue to fall back to the page's own
    // top-level scroll instead. Found by its own distinctive 30px
    // height (chosen to not collide with either 50px scroll region,
    // or with the root/html/body wrapper boxes' own, much taller,
    // whole-document heights).
    let p = find_laid_by_height(&laid, 30.0).expect("the plain, non-scrollable <p> should be findable by its own height");
    let (px, py) = (p.x + 5.0, p.y + 5.0);
    assert!(
        px != fx || py != fy,
        "sanity: the <p>'s own point shouldn't accidentally coincide with the first region's"
    );
    let hit_none = hit_test_scroll_region(&laid, px, py, &scroll_offsets, 0.0, viewport_h);
    assert!(
        hit_none.is_none(),
        "a point over plain, non-scrollable content should fall through to the page's own scroll"
    );
}

#[test]
fn scrollbar_paints_for_scroll_even_when_not_overflowing_but_auto_only_paints_when_it_does() {
    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    // Taller than any one fixture's own box (body's 8px margin +
    // up to 200px box height) so the scan below never runs past the
    // buffer's own bounds.
    let (win_w, win_h) = (200u32, 260u32);
    // A translucent black-on-white pixel near the box's own right
    // edge (within `TRACK_W` of it) is the scrollbar's own track or
    // thumb — nothing else in this fixture paints anything but white
    // background and (deliberately) no text color near that edge,
    // so any non-white pixel there is unambiguously the indicator.
    let has_scrollbar_pixel = |pixels: &[u32], box_right: f32, box_top: f32, box_bottom: f32, win_w: u32| {
        let x = (box_right - 4.0) as u32;
        (box_top as u32..box_bottom as u32).any(|y| pixels[(y * win_w + x) as usize] != 0x00FFFFFF)
    };

    // `overflow: scroll` with content that fits entirely — real CSS
    // `scroll` always shows a scrollbar regardless.
    let scroll_fits = measure_fixture(
        r#"<html><body><div style="overflow: scroll; height: 200px; width: 100px;">short</div></body></html>"#,
        400.0,
    );
    let scroll_box = find_laid_with_scroll(&scroll_fits).expect("overflow: scroll box not found");
    let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
    paint(
        &scroll_fits, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w,
        win_h, 0.0, None,
    );
    assert!(
        has_scrollbar_pixel(&pixels, scroll_box.x + scroll_box.w, scroll_box.y, scroll_box.y + scroll_box.h, win_w),
        "overflow: scroll should always paint a scrollbar indicator, even when nothing overflows"
    );

    // `overflow: auto` with content that fits — no scrollbar at all.
    let auto_fits = measure_fixture(
        r#"<html><body><div style="overflow: auto; height: 200px; width: 100px;">short</div></body></html>"#,
        400.0,
    );
    let auto_fits_box = find_laid_with_scroll(&auto_fits).expect("overflow: auto box not found");
    let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
    paint(
        &auto_fits, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h,
        0.0, None,
    );
    assert!(
        !has_scrollbar_pixel(&pixels, auto_fits_box.x + auto_fits_box.w, auto_fits_box.y, auto_fits_box.y + auto_fits_box.h, win_w),
        "overflow: auto shouldn't paint a scrollbar at all when nothing overflows"
    );

    // `overflow: auto` with content that genuinely overflows — now a
    // scrollbar should appear, same as `scroll`.
    let auto_overflows = measure_fixture(
        concat!(
            r#"<html><body><div style="overflow: auto; height: 20px; width: 100px;">"#,
            r#"<p style="margin: 0;">one</p><p style="margin: 0;">two</p><p style="margin: 0;">three</p>"#,
            "</div></body></html>",
        ),
        400.0,
    );
    let auto_overflow_box = find_laid_with_scroll(&auto_overflows).expect("overflow: auto box not found");
    let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
    paint(
        &auto_overflows, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w,
        win_h, 0.0, None,
    );
    assert!(
        has_scrollbar_pixel(
            &pixels, auto_overflow_box.x + auto_overflow_box.w, auto_overflow_box.y,
            auto_overflow_box.y + auto_overflow_box.h, win_w
        ),
        "overflow: auto should paint a scrollbar once its content actually overflows"
    );
}

#[test]
fn qy20_visibility_hidden_scroll_container_paints_no_scrollbar_and_takes_no_wheel() {
    // bbc.com/news: a drawer with `visibility:hidden` holds an
    // `overflow-y:scroll` child. Real CSS: the hidden box is painted
    // as if transparent and is not a hit-test target, so its own
    // scrollbar must not paint and the wheel must not scroll it. A
    // descendant with `visibility:visible` still does both.
    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let mut cache = ShapeCache::new();
    let (win_w, win_h) = (200u32, 260u32);
    let has_scrollbar_pixel = |pixels: &[u32], box_right: f32, box_top: f32, box_bottom: f32| {
        let x = (box_right - 4.0) as u32;
        (box_top as u32..box_bottom as u32).any(|y| pixels[(y * win_w + x) as usize] != 0x00FFFFFF)
    };
    let mut paint_white = |laid: &Laid| {
        let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
        paint(laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
        pixels
    };

    // Control: the same box, visible, paints its scrollbar and takes the wheel.
    let visible = measure_fixture(
        r#"<html><body><div style="overflow-y: scroll; height: 200px; width: 100px;">short</div></body></html>"#,
        400.0,
    );
    let vbox = find_laid_with_scroll(&visible).expect("overflow-y: scroll box not found");
    let pixels = paint_white(&visible);
    assert!(
        has_scrollbar_pixel(&pixels, vbox.x + vbox.w, vbox.y, vbox.y + vbox.h),
        "a visible overflow-y: scroll box must still paint its scrollbar"
    );
    assert!(
        hit_test_scroll_region(&visible, vbox.x + 10.0, vbox.y + 10.0, &HashMap::new(), 0.0, 600.0).is_some(),
        "a visible scroll box must still take the wheel under the cursor"
    );

    // The same box under `visibility: hidden` (inherited by its
    // content): no scrollbar pixels, and the wheel does not land on it.
    let hidden = measure_fixture(
        r#"<html><body><div style="visibility: hidden; overflow-y: scroll; height: 200px; width: 100px;"><div style="height: 500px"></div></div></body></html>"#,
        400.0,
    );
    let hbox = find_laid_with_scroll(&hidden).expect("hidden overflow-y: scroll box not found");
    let pixels = paint_white(&hidden);
    assert!(
        !has_scrollbar_pixel(&pixels, hbox.x + hbox.w, hbox.y, hbox.y + hbox.h),
        "a visibility:hidden scroll container must not paint its scrollbar"
    );
    assert!(
        hit_test_scroll_region(&hidden, hbox.x + 10.0, hbox.y + 10.0, &HashMap::new(), 0.0, 600.0).is_none(),
        "a visibility:hidden scroll container must not take the wheel"
    );

    // A `visibility: visible` scroll box nested inside the hidden one
    // overrides the inherited value: its own scrollbar paints again.
    let nested = measure_fixture(
        r#"<html><body><div style="visibility: hidden; height: 200px; width: 100px;"><div style="visibility: visible; overflow-y: scroll; height: 200px; width: 100px;">short</div></div></body></html>"#,
        400.0,
    );
    let nbox = find_laid_with_scroll(&nested).expect("nested overflow-y: scroll box not found");
    let pixels = paint_white(&nested);
    assert!(
        has_scrollbar_pixel(&pixels, nbox.x + nbox.w, nbox.y, nbox.y + nbox.h),
        "a visibility:visible scroll box inside a hidden ancestor must still paint its scrollbar"
    );
}

// ---------- `box-shadow: none` ----------

#[test]
fn box_shadow_none_overrides_a_rule_level_shadow_not_just_fail_silently() {
    let laid = measure_fixture(
        r#"<html><head><style>div { box-shadow: 2px 2px 0 #000000; }</style></head><body><div style="box-shadow: none; width: 123px;">x</div></body></html>"#,
        400.0,
    );
    let div = find_by_width(&laid, 123.0).expect("div not found");
    assert!(
        div.style.box_shadow[0].is_none(),
        "box-shadow: none should win the cascade and clear the rule-level shadow, not silently fail to parse and let the rule's shadow leak through"
    );
}

#[test]
fn box_shadow_without_none_still_resolves_normally() {
    let laid = measure_fixture(
        r#"<html><body><div style="box-shadow: 2px 3px 0 #ff0000; width: 50px;">x</div></body></html>"#,
        400.0,
    );
    let div = find_by_width(&laid, 50.0).expect("div not found");
    let shadow = div.style.box_shadow[0].expect("a real box-shadow declaration should still resolve normally");
    assert_eq!((shadow.offset_x, shadow.offset_y), (2.0, 3.0));
}

// ---------- Q25: box-shadow blur, multiple, inset, currentColor ----------

#[test]
fn q25_box_shadow_parses_multiple_inset_blur_and_current_color() {
    let laid = measure_fixture(
        r#"<html><body><div style="color: #00ff00; width: 50px; box-shadow: 1px 2px 3px 4px red, inset 0 0 5px blue, 5px 5px;">x</div></body></html>"#,
        400.0,
    );
    let div = find_by_width(&laid, 50.0).expect("div not found");
    let b = &div.style.box_shadow;
    let first = b[0].expect("first shadow");
    assert_eq!((first.offset_x, first.offset_y, first.blur, first.spread), (1.0, 2.0, 3.0, 4.0));
    assert!(!first.inset);
    let second = b[1].expect("second shadow");
    assert!(second.inset && second.blur == 5.0);
    let third = b[2].expect("third shadow");
    assert_eq!((third.color.g(), third.color.r()), (255, 0), "a missing color means currentColor");
    assert!(b[3].is_none());
}

#[test]
fn q25_blurred_outer_shadow_fades_and_skips_the_border_box() {
    let (px, w, _) = paint_fixture(
        r#"<html><body style="margin:0;background:#fff"><div style="margin:50px;width:60px;height:60px;box-shadow:0 0 20px #000000"></div></body></html>"#,
    );
    // Inside the (transparent) box: no shadow drawn.
    assert_eq!(px_at(&px, w, 80, 80), 0xFFFFFF);
    // Just outside the edge is darker than farther out; far away is untouched.
    let near = px_at(&px, w, 47, 80) & 0xFF;
    let far = px_at(&px, w, 35, 80) & 0xFF;
    assert!(near < far, "shadow should fade with distance ({near} vs {far})");
    assert!(near < 0xF0, "shadow should be visible right at the edge");
    assert_eq!(px_at(&px, w, 5, 80), 0xFFFFFF);
}

#[test]
fn q25_inset_shadow_paints_inside_and_multiple_layers_stack() {
    let (px, w, _) = paint_fixture(
        r#"<html><body style="margin:0;background:#fff"><div style="margin:20px;width:100px;height:100px;background:#ffffff;box-shadow:inset 10px 10px 0 #ff0000"></div></body></html>"#,
    );
    assert_eq!(px_at(&px, w, 25, 25), 0xFF0000, "top-left band is inset-shadowed");
    assert_eq!(px_at(&px, w, 100, 100), 0xFFFFFF, "the rest is untouched");
    assert_eq!(px_at(&px, w, 15, 25), 0xFFFFFF, "nothing outside the box");
    let (px, w, _) = paint_fixture(
        r#"<html><body style="margin:0;background:#fff"><div style="margin:50px;width:20px;height:20px;box-shadow:5px 0 0 #ff0000, 10px 0 0 #0000ff"></div></body></html>"#,
    );
    assert_eq!(px_at(&px, w, 73, 60), 0xFF0000, "first layer is on top");
    assert_eq!(px_at(&px, w, 78, 60), 0x0000FF);
}

// ---------- QA8: background-attachment: fixed ----------

#[test]
fn qa8_background_attachment_parses() {
    let src = tiny_png_data_uri(4, 4);
    let fixed_of = |css: &str| {
        let html = format!(r#"<html><body><div style="width: 13px; height: 13px; {css}"></div></body></html>"#);
        let laid = measure_fixture(&html, 200.0);
        find_by_width(&laid, 13.0).expect("div not found").style.bg_layout.fixed
    };
    assert!(!fixed_of("background: red;"));
    assert!(fixed_of("background-attachment: fixed;"));
    assert!(!fixed_of("background-attachment: scroll;"));
    assert!(!fixed_of("background-attachment: local;"));
    assert!(!fixed_of("background-attachment: bogus;"));
    assert!(fixed_of(&format!("background: url('{src}') fixed no-repeat;")));
    assert!(!fixed_of(&format!("background-attachment: fixed; background: url('{src}') no-repeat;")), "shorthand resets");
    assert!(fixed_of(&format!("background: url('{src}') local; background-attachment: fixed;")));
}

#[test]
fn qa8_fixed_background_positions_against_viewport() {
    // 1x20 image whose row n has red = n*10. A 10x10 box at y=7 with a
    // fixed background: box row 0 shows image row 7 (viewport y), not 0.
    let pixels_img: Vec<TextColor> = (0..20u8).map(|n| TextColor::rgb(n * 10, 0, 0)).collect();
    let image = DecodedImage { width: 1, height: 20, pixels: pixels_img, svg: None, mask: None };
    let repeat_x = BgRepeat { x: true, y: false };
    let paint_at = |fixed: bool| {
        let layout = BgLayout { repeat: repeat_x, fixed, ..BgLayout::default() };
        let mut px = vec![0u32; 20 * 20];
        paint_background_image(&image, &layout, &mut px, 20, 20, 5.0, 7.0, 10.0, 10.0, 0.0, None);
        px
    };
    let red = |px: &[u32], x: usize, y: usize| (px[y * 20 + x] >> 16) & 0xFF;
    let scroll = paint_at(false);
    assert_eq!(red(&scroll, 5, 7), 0, "scroll: image row 0 at the box top");
    assert_eq!(red(&scroll, 5, 8), 10);
    let fixed = paint_at(true);
    assert_eq!(red(&fixed, 5, 7), 70, "fixed: image row 7 (viewport y) at the box top");
    assert_eq!(red(&fixed, 5, 16), 160);
    // Still clipped to the box.
    assert_eq!(fixed[7 * 20 + 4], 0, "left of the box");
    assert_eq!(fixed[6 * 20 + 5], 0, "above the box");
    assert_eq!(fixed[17 * 20 + 5], 0, "below the box");
}

#[test]
fn qy2_performance_timing_and_visibility() {
    let log = run_collecting(&[src(
        "console.log(performance.timing.responseStart > 0 && document.visibilityState === 'visible' && document.hidden === false);\
         console.log(performance.timeOrigin > 0 && performance.timing.navigationStart <= performance.timing.loadEventEnd);\
         console.log(performance.getEntries().length + ',' + performance.getEntriesByType('resource').length + ',' + performance.getEntriesByName('x').length);\
         console.log(typeof performance.now() + typeof performance.mark);",
    )]);
    assert_eq!(log, ["log:true", "log:true", "log:0,0,0", "log:numberfunction"]);
}

// ---------- QW6: canvas background propagation ----------

fn qw6_pixels(html: &str) -> Vec<u32> {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 400.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 400.0, 300.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let mut pixels = vec![0xFFFFFFu32; 400 * 300];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, 400, 300, 0.0, None);
    pixels
}

#[test]
fn body_background_paints_the_canvas_qw6() {
    let px = qw6_pixels(r#"<html><body style="background:#123;max-width:300px;margin:40px auto"><p>x</p></body></html>"#);
    assert_eq!(px[5 * 400 + 5] & 0xFFFFFF, 0x112233, "canvas corner takes the body's colour");
    assert_eq!(px[5 * 400 + 395] & 0xFFFFFF, 0x112233, "also outside the body's column");
}

#[test]
fn html_background_wins_and_body_keeps_its_own_qw6() {
    let px = qw6_pixels(r#"<html style="background:red"><body style="background:#123;max-width:300px;margin:40px auto"><p>x</p></body></html>"#);
    assert_eq!(px[5 * 400 + 5] & 0xFFFFFF, 0xFF0000, "canvas stays red");
    assert_eq!(px[50 * 400 + 200] & 0xFFFFFF, 0x112233, "body paints its own background");
}

#[test]
fn qy29_data_and_form_props_live_on_their_interfaces() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        "var d = document.getElementById('d'), t = document.createTextNode('a'), c = document.createComment('b');\
         console.log('data' in d, 'data' in document.createElement('x-foo'), 'data' in document.createElement('object'), 'data' in t && 'data' in c);\
         t.data = 'zz'; console.log(t.data, c.data, t.nodeValue);\
         var b = document.createElement('button'), i = document.createElement('input');\
         console.log('disabled' in d, 'disabled' in b, 'required' in d, 'required' in i, 'checked' in d, 'checked' in i, 'disabled' in document.createElement('x-foo'));\
         i.checked = true; i.required = true; b.disabled = true; console.log(i.hasAttribute('checked'), i.hasAttribute('required'), b.hasAttribute('disabled'), i.checked);",
    );
    assert_eq!(
        log,
        ["log:false false true true", "log:zz b zz", "log:false true false true false true false", "log:false true true true"],
        "{log:?}"
    );
}

fn find_any_by_width(laid: &Laid, w: f32) -> Option<&Laid> {
    if laid.style.width.map(BoxLength::px) == Some(w) {
        return Some(laid);
    }
    if let LaidContent::Blocks(cs) = &laid.content {
        for c in cs {
            if let Some(f) = find_any_by_width(c, w) {
                return Some(f);
            }
        }
    }
    laid.positioned.iter().find_map(|c| find_any_by_width(c, w))
}

// QY32: `position: fixed` inside an absolutely positioned box used to be
// dropped (the nested pending list was only debug_assert!ed empty).
#[test]
fn fixed_inside_absolute_is_laid_out_at_viewport_position_qy32() {
    let laid = measure_document_fixture(
        r#"<html><body style="margin:0">
<div style="position:absolute;top:200px;left:50px;width:300px;height:100px">
<div style="position:fixed;top:7px;left:9px;width:11px;height:10px"></div>
<div style="position:fixed;top:8px;left:9px;width:12px;height:10px;z-index:5"></div>
<div style="position:fixed;top:6px;left:9px;width:13px;height:10px;transform:translateY(0)"></div>
</div></body></html>"#,
        400.0,
        600.0,
    );
    for (w, top) in [(11.0, 7.0), (12.0, 8.0), (13.0, 6.0)] {
        let f = find_any_by_width(&laid, w).unwrap_or_else(|| panic!("fixed box of width {w} was dropped"));
        assert!(f.fixed, "width {w} must be flagged fixed");
        assert_eq!((f.x, f.y), (9.0, top), "fixed box of width {w} stays at its viewport position, not the absolute parent's");
    }
}

/// QY30: a rAF is always due strictly after "now", and a self-rescheduling
/// rAF loop advances one frame per callback with monotonic timestamps.
#[test]
fn qy30_raf_due_time_strictly_after_now() {
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let engine = crate::js::start_scripts_clocked(&parse_html(""), &[src("")], Rc::new(|_, _| {}), Some(clock.clone())).unwrap();
    let frame = 1000.0 / 60.0;
    for t in [0.0, 100.0, frame * 6.0, 1e6, 16.666666666666668, 33.333333333333336] {
        clock.set(t);
        engine.eval_value::<bool>("t", "requestAnimationFrame(function(){}); true");
        let due = engine.next_due_ms().unwrap();
        assert!(due > t, "rAF at {t} due {due}");
        clock.set(due);
        engine.run_due();
    }
    // A self-rescheduling loop, pumped like the report does.
    clock.set(100.0);
    engine.eval_value::<bool>(
        "t",
        "globalThis.__ts=[]; (function f(ts){ __ts.push(ts); if (__ts.length<10) requestAnimationFrame(f); })(0); true",
    );
    for _ in 0..50 {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    let ts: String = engine.eval_value("t", "__ts.join(',')").unwrap();
    let v: Vec<f64> = ts.split(',').map(|x| x.parse().unwrap()).collect();
    assert_eq!(v.len(), 10, "{ts}");
    for w in v[1..].windows(2) {
        assert!(w[1] > w[0], "timestamps must strictly advance: {ts}");
    }
    assert!(clock.get() - 100.0 >= frame * 8.0, "clock advanced {}", clock.get());
}

#[test]
fn qy40_document_all_is_htmldda() {
    let doc = parse_html("<html><head></head><body><div id=a></div><form name=f></form></body></html>");
    let o = Rc::new(RefCell::new(Vec::<String>::new()));
    let o2 = o.clone();
    let code = "var out=[]; var t=function(n,v){out.push(n+':'+v)};
      t('typeof', typeof document.all === 'undefined'); t('falsy', !document.all); t('eqnull', document.all == null);
      t('notundef', document.all !== undefined);
      t('len', document.all.length > 0 && document.all.length === document.getElementsByTagName('*').length);
      t('idx0', document.all[0] === document.documentElement); t('item0', document.all.item(0) === document.documentElement);
      t('named', document.all.namedItem('a') === document.getElementById('a'));
      t('namedname', document.all.namedItem('f') === document.forms[0]);
      t('call', document.all('a') === document.getElementById('a'));
      var n = document.all.length; document.body.appendChild(document.createElement('p')); t('live', document.all.length === n + 1);
      function f(T){ if(!T && T!==document.all) return T; return 'zClosurez'; }
      t('resin', f(undefined) === undefined && f(null) === null && f(document.all) === 'zClosurez' && f(document.body) === 'zClosurez');
      var r='no'; if (document.all) r='ie'; t('sniff', r === 'no');
      console.log(out.join(','));";
    let _e = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o2.borrow_mut().push(format!("{level}:{text}"))));
    let log = o.borrow().join("\n");
    assert!(log.contains("log:") && !log.contains(":false") && !log.contains("error:"), "{log}");
    assert!(log.contains("sniff:true") && log.contains("resin:true") && log.contains("live:true"), "{log}");
}

/// QY41: a `%` width inside an intrinsic (max-content) probe is cyclic and
/// behaves as `auto`; it must not resolve against the 1e6 probe width.
#[test]
fn qy41_percent_width_inline_block_does_not_inflate_max_content() {
    for (parent, pct) in [
        ("display:inline-block", "100%"),
        ("display:block", "100%"),
        ("display:inline-block", "50%"),
        ("display:inline-block", "calc(100% - 4px)"),
    ] {
        let html = format!(
            r#"<html><body style="margin:0"><div style="display:flex;width:500px"><div style="flex:none"><span style="{parent};width:40px"><span style="display:inline-block;width:{pct}"></span></span></div><div style="width:10px;height:5px"></div></div></body></html>"#
        );
        let laid = measure_fixture(&html, 600.0);
        let marker = find_by_width(&laid, 10.0).expect("marker item");
        assert!(marker.x <= 41.0 && marker.x >= 39.0, "{parent} {pct}: marker x = {}, expected 40", marker.x);
    }
}

/// QY41 regression guard: definite px widths still size max-content.
#[test]
fn qy41_definite_px_width_inline_block_still_sizes_max_content() {
    let html = r#"<html><body style="margin:0"><div style="display:flex;width:500px"><div style="flex:none"><span style="display:inline-block;width:70px;height:3px"></span></div><div style="width:10px;height:5px"></div></div></body></html>"#;
    let laid = measure_fixture(html, 600.0);
    let marker = find_by_width(&laid, 10.0).expect("marker item");
    assert!((marker.x - 70.0).abs() < 1.0, "marker x = {}", marker.x);
}
