#[allow(unused_imports)]
use super::*;

#[test]
fn img_with_no_css_or_attrs_uses_intrinsic_size() {
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body><img src="{src}"></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (40.0, 20.0), "auto width/height should fall back to the decoded intrinsic size");
}

#[test]
fn img_css_width_only_scales_height_by_intrinsic_aspect_ratio() {
    let src = tiny_png_data_uri(40, 20); // 2:1
    let html = format!(r#"<html><body><img src="{src}" style="width: 100px"></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (100.0, 50.0), "height: auto should scale proportionally from the explicit width");
}

#[test]
fn img_css_height_only_scales_width_by_intrinsic_aspect_ratio() {
    let src = tiny_png_data_uri(40, 20); // 2:1
    let html = format!(r#"<html><body><img src="{src}" style="height: 50px"></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (100.0, 50.0), "width: auto should scale proportionally from the explicit height");
}

#[test]
fn img_html_width_height_attrs_used_when_css_leaves_both_auto() {
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body><img src="{src}" width="80" height="90"></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (80.0, 90.0), "HTML width/height attributes should size the image when no CSS size is declared");
}

#[test]
fn img_html_percent_width_attr_is_percentage_of_container() {
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body style="margin:0"><div style="width:200px"><img src="{src}" width="50%"></div></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (100.0, 50.0), "width=50% should be half the container, height by aspect");
}

#[test]
fn img_css_size_overrides_html_attrs() {
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body><img src="{src}" width="80" height="90" style="width: 10px; height: 10px"></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let img = find_image_laid(&laid).expect("img box not found");
    assert_eq!((img.w, img.h), (10.0, 10.0), "a declared CSS size should win over the HTML width/height attributes");
}

fn find_blocks_containing_image(b: &LayoutBox) -> Option<&[LayoutBox]> {
    if let BoxContent::Blocks(children) = &b.content {
        if children.iter().any(|c| matches!(c.content, BoxContent::Image(_))) {
            return Some(children);
        }
        for c in children {
            if let Some(found) = find_blocks_containing_image(c) {
                return Some(found);
            }
        }
    }
    None
}

fn find_inline_run_with_image(b: &LayoutBox) -> Option<&[InlineItem]> {
    if let BoxContent::Inline(items) = &b.content {
        if items.iter().any(|i| matches!(i, InlineItem::Image { .. })) {
            return Some(items);
        }
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_inline_run_with_image(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn img_with_default_display_shares_the_inline_run_with_surrounding_text() {
    // Pre-v48 behavior: a default-`display: inline` `<img>` always
    // broke surrounding text into separate sibling boxes. v48
    // reverses this — see TODO.txt/NOTES.md — so this now pins the
    // *opposite*: one `BoxContent::Inline` run holding text before,
    // the image, and text after, in source order, ready for real
    // inline-level flow in `measure_mixed_inline`.
    let src = tiny_png_data_uri(2, 2);
    let html = format!(r#"<html><body><p>before <img src="{src}"> after</p></body></html>"#);
    let doc = parse_html(&html);
    let tree = build_layout_tree(&doc, 1920.0);
    let items = find_inline_run_with_image(&tree).expect("<p> should be one inline run containing the image");
    // Q59: the source space after the image is its own (hanging) text item.
    assert_eq!(items.len(), 4, "text before/the image/hanging space/text after should be 4 items in the SAME inline run");
    let InlineItem::Text(before, _) = &items[0] else { panic!("expected text before the image") };
    assert!(before.trim_start().starts_with("before"), "unexpected text before the image: {before:?}");
    assert!(matches!(items[1], InlineItem::Image { .. }));
    let InlineItem::Text(after, _) = &items[3] else { panic!("expected text after the image") };
    assert!(after.trim_start().starts_with("after"), "unexpected text after the image: {after:?}");
}

#[test]
fn img_with_explicit_display_block_still_gets_its_own_box() {
    // The one case that should still behave like pre-v48: an author
    // explicitly overriding `<img>`'s default `display: inline` to
    // `block` is real block-level markup, not inline flow — same
    // treatment as any other block element, not folded into the
    // surrounding text run.
    let src = tiny_png_data_uri(2, 2);
    let html = format!(r#"<html><body><p>before <img src="{src}" style="display: block"> after</p></body></html>"#);
    let doc = parse_html(&html);
    let tree = build_layout_tree(&doc, 1920.0);
    let children = find_blocks_containing_image(&tree).expect("display:block <img> should still split into sibling boxes");
    assert_eq!(children.len(), 3);
    assert!(matches!(children[1].content, BoxContent::Image(_)));
}

fn find_inline_text(b: &LayoutBox) -> Option<&str> {
    if let BoxContent::Inline(items) = &b.content {
        for item in items {
            if let InlineItem::Text(text, _) = item {
                return Some(text.as_str());
            }
        }
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_inline_text(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn img_with_missing_src_falls_back_to_alt_text() {
    let doc = parse_html(r#"<html><body><img alt="a fallback"></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let text = find_inline_text(&tree).expect("no text found");
    assert!(text.contains("a fallback"), "a missing src should fall back to rendering alt text: {text:?}");
}

#[test]
fn img_with_undecodable_src_falls_back_to_alt_text() {
    let doc = parse_html(r#"<html><body><img src="data:image/png;base64,not-valid-base64!!" alt="broken img"></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let text = find_inline_text(&tree).expect("no text found");
    assert!(text.contains("broken img"), "undecodable image data should fall back to rendering alt text: {text:?}");
}

// ---------- v47: `background-image` ----------

#[test]
fn tile_image_repeats_the_source_image_across_a_larger_box() {
    const W: u32 = 8;
    const H: u32 = 2;
    let mut pixels = vec![0u32; (W * H) as usize];
    let red = TextColor::rgb(255, 0, 0);
    let blue = TextColor::rgb(0, 0, 255);
    let image = DecodedImage { width: 2, height: 1, pixels: vec![red, blue], svg: None, mask: None };
    tile_image(&image, &mut pixels, W, H, 0.0, 0.0, 8.0, 1.0, None);
    let px = |x: u32| pixels[x as usize] & 0x00FFFFFF;
    assert_eq!(px(0), 0xFF0000);
    assert_eq!(px(1), 0x0000FF);
    assert_eq!(px(2), 0xFF0000, "a 2px-wide image should repeat, not stretch, across a wider box");
    assert_eq!(px(3), 0x0000FF);
    assert_eq!(px(6), 0xFF0000);
    assert_eq!(px(7), 0x0000FF);
}

#[test]
fn q28_object_fit_and_position_parse_and_place_the_picture() {
    let style_of = |css: &str| {
        let html = format!(r#"<html><body><div style="width: 13px; height: 13px; {css}"></div></body></html>"#);
        let laid = measure_fixture(&html, 200.0);
        let st = find_by_width(&laid, 13.0).expect("div not found").style;
        (st.object_fit, st.object_pos)
    };
    assert_eq!(style_of(""), (ObjectFit::Fill, ((0.5, 0.0), (0.5, 0.0))));
    assert_eq!(style_of("object-fit: cover; object-position: left 3px top;"), (ObjectFit::Cover, ((0.0, 3.0), (0.0, 0.0))));
    assert_eq!(style_of("object-fit: scale-down;").0, ObjectFit::ScaleDown);
    assert_eq!(style_of("object-fit: bogus;").0, ObjectFit::Fill);
    // Painting: a 2x1 image (left red, right blue) in a 10x10 box.
    let red = TextColor::rgb(255, 0, 0);
    let blue = TextColor::rgb(0, 0, 255);
    let image = DecodedImage { width: 2, height: 1, pixels: vec![red, blue], svg: None, mask: None };
    let paint = |fit: ObjectFit, pos: ((f32, f32), (f32, f32))| {
        let mut px = vec![0u32; 100];
        blit_image(&image, fit, pos, &mut px, 10, 10, 0.0, 0.0, 10.0, 10.0, None);
        px
    };
    let center = ((0.5, 0.0), (0.5, 0.0));
    let set = |px: &[u32], x: usize, y: usize| px[y * 10 + x] & 0xFFFFFF != 0;
    let px = paint(ObjectFit::Fill, center);
    assert!(set(&px, 0, 0) && set(&px, 9, 9), "fill covers the whole box");
    let px = paint(ObjectFit::Contain, center);
    assert!(set(&px, 0, 5) && set(&px, 9, 5) && !set(&px, 5, 0) && !set(&px, 5, 9), "contain letterboxes a 2:1 picture");
    let px = paint(ObjectFit::None, center);
    assert!(set(&px, 4, 4) && set(&px, 5, 4) && !set(&px, 3, 4) && !set(&px, 6, 4), "none keeps the 2x1 intrinsic size, centred");
    let px = paint(ObjectFit::None, ((0.0, 0.0), (0.0, 0.0)));
    assert!(set(&px, 0, 0) && set(&px, 1, 0) && !set(&px, 2, 0) && !set(&px, 0, 1), "object-position 0 0 pins it top-left");
    let px = paint(ObjectFit::Cover, center);
    assert!(set(&px, 0, 0) && set(&px, 9, 9), "cover fills the box");
    assert!(((px[5 * 10 + 1] >> 16) & 0xFF) > (px[5 * 10 + 1] & 0xFF) * 2, "cover crops: the left half is still mostly red");
    let px = paint(ObjectFit::ScaleDown, center);
    assert!(set(&px, 4, 4) && !set(&px, 3, 4), "scale-down never upscales past intrinsic size");
    // Bilinear: scaling the 2x1 image up blends red into blue mid-way.
    let px = paint(ObjectFit::Fill, center);
    let mid = px[5 * 10 + 5];
    assert!(((mid >> 16) & 0xFF) > 20 && (mid & 0xFF) > 20, "bilinear blends across the seam: {mid:06x}");
    // Downscaling box-averages instead of picking one pixel.
    let checker = DecodedImage {
        width: 4, height: 4,
        pixels: (0..16).map(|i| if (i % 4 + i / 4) % 2 == 0 { TextColor::rgb(255, 255, 255) } else { TextColor::rgb(0, 0, 0) }).collect(),
        svg: None, mask: None,
    };
    let mut px = vec![0u32; 4];
    blit_image(&checker, ObjectFit::Fill, center, &mut px, 2, 2, 0.0, 0.0, 1.0, 1.0, None);
    let g = px[0] & 0xFF;
    assert!((120..=135).contains(&g), "a 4x4 checkerboard downscaled to 1px averages to grey, got {g}");
}

#[test]
fn q29_mask_image_clips_the_box_by_the_masks_alpha() {
    // Mask: an SVG whose left half is opaque, right half empty.
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 2 1'><rect width='1' height='1'/></svg>";
    let uri = format!("data:image/svg+xml,{}", svg.replace('<', "%3C").replace('>', "%3E").replace('\'', "%27"));
    let render = |css: &str| {
        let html = format!(
            r#"<html><body style="margin: 0"><div style="width: 40px; height: 20px; background-color: #ff0000; {css}"></div></body></html>"#
        );
        let doc = parse_html(&html);
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
        let mut pixels = vec![0u32; 60 * 30];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, 60, 30, 0.0, None);
        (laid, pixels)
    };
    let px = |p: &[u32], x: u32, y: u32| p[(y * 60 + x) as usize] & 0xFFFFFF;
    let (_, plain) = render("");
    assert_eq!((px(&plain, 5, 10), px(&plain, 35, 10)), (0xFF0000, 0xFF0000));
    for prop in ["-webkit-mask-image", "mask-image"] {
        let (_, p) = render(&format!("{prop}: url('{uri}'); mask-size: 40px 20px; mask-repeat: no-repeat;"));
        assert_eq!(px(&p, 5, 10), 0xFF0000, "{prop}: inside the mask stays painted");
        assert_eq!(px(&p, 35, 10), 0, "{prop}: outside the mask is cut away");
    }
    // A later `none` wins; then the `-webkit-mask` shorthand.
    let (_, p) = render(&format!("mask-image: url('{uri}'); mask-image: none;"));
    assert_eq!(px(&p, 35, 10), 0xFF0000);
    let (_, p) = render(&format!("-webkit-mask: url('{uri}') no-repeat center / 40px 20px;"));
    assert_eq!((px(&p, 5, 10), px(&p, 35, 10)), (0xFF0000, 0));
    // Position moves the stencil: a 20px mask pinned right covers x 20..40.
    let (laid, p) = render(&format!("mask-image: url('{uri}'); mask-size: 20px 20px; mask-repeat: no-repeat; mask-position: right;"));
    assert_eq!((px(&p, 5, 10), px(&p, 25, 10), px(&p, 35, 10)), (0, 0xFF0000, 0));
    let div = find_by_width(&laid, 40.0).expect("div not found");
    assert!(div.style.mask_layout.repeat == BgRepeat { x: false, y: false });
}

#[test]
fn background_image_css_property_paints_behind_border() {
    let src = tiny_png_data_uri(4, 4); // solid red, see `tiny_png_data_uri`
    let html = format!(
        r#"<html><body><div style="width: 40px; height: 40px; border: 4px solid blue; background-image: url('{src}')"></div></body></html>"#
    );
    let doc = parse_html(&html);
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
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);

    // `body { margin: 8px }` (default.css) offsets everything by 8px —
    // border-box spans roughly x/y 8..56 (4px border + 40px content +
    // 4px border), content (background-image) area roughly 12..52.
    let px = |x: u32, y: u32| pixels[(y * win_w + x) as usize] & 0x00FFFFFF;
    assert_eq!(px(30, 30), 0xFF0000, "background-image should paint inside the border");
    assert_eq!(px(9, 30), 0x0000FF, "border itself should still paint on top of/around the background-image");
}

#[test]
fn list_style_none_shorthand_suppresses_the_marker() {
    let laid = measure_fixture(
        r#"<html><body><ul style="list-style: none;"><li style="width: 42px;">Item</li></ul></body></html>"#,
        200.0,
    );
    let li = find_by_width(&laid, 42.0).expect("li not found");
    assert!(
        li.style.list_style_type == ListStyleType::None,
        "list-style: none on the ul should cascade/inherit down to no marker on the li"
    );
}

#[test]
fn list_style_type_none_longhand_also_suppresses_the_marker() {
    let laid = measure_fixture(
        r#"<html><body><ul><li style="width: 42px; list-style-type: none;">Item</li></ul></body></html>"#,
        200.0,
    );
    let li = find_by_width(&laid, 42.0).expect("li not found");
    assert!(li.style.list_style_type == ListStyleType::None);
}

#[test]
fn list_style_disc_is_still_the_default_without_none() {
    let laid = measure_fixture(
        r#"<html><body><ul><li style="width: 42px;">Item</li></ul></body></html>"#,
        200.0,
    );
    let li = find_by_width(&laid, 42.0).expect("li not found");
    assert!(li.style.list_style_type == ListStyleType::Disc, "sanity: an ordinary <li> should still default to a disc marker");
}

#[test]
fn background_shorthand_color_only_sets_background_color() {
    let html = r#"<html><body style="margin:0"><div style="width: 40px; height: 40px; background: #ff0000;"></div></body></html>"#;
    let doc = parse_html(html);
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
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    let px = pixels[20 * win_w as usize + 20];
    assert_eq!(px & 0x00FF_FFFF, 0x00FF_0000, "`background: #ff0000` shorthand should paint the same red as `background-color` would");
}

#[test]
fn q17_opacity_multiplies_down_the_tree_and_fades_paint() {
    let html = r#"<html><body style="margin:0"><div style="width: 40px; height: 40px; background: #ff0000; opacity: .5;"><div style="width: 20px; height: 20px; background: #0000ff; opacity: 50%;"></div></div></body></html>"#;
    let doc = parse_html(html);
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
    assert_eq!(find_by_width(&laid, 40.0).unwrap().style.opacity, 0.5);
    assert_eq!(find_by_width(&laid, 20.0).unwrap().style.opacity, 0.25, "child multiplies the parent's");
    // Out-of-range values clamp; invalid ones are ignored.
    assert_eq!(parse_declarations("opacity: 3").opacity, Some(1.0));
    assert_eq!(parse_declarations("opacity: -1").opacity, Some(0.0));
    assert_eq!(parse_declarations("opacity: red").opacity, None);
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0xFFFFFFu32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    let outer = pixels[30 * win_w as usize + 30] & 0xFFFFFF;
    assert!((outer >> 16) & 0xFF == 255 && ((outer >> 8) & 0xFF).abs_diff(127) <= 1, "red at 50% over white: {outer:06x}");
    let inner = pixels[10 * win_w as usize + 10] & 0xFFFFFF;
    assert!((inner & 0xFF).abs_diff(0x9f) <= 2 && ((inner >> 16) & 0xFF).abs_diff(0xbf) <= 2, "double-blended blue over faded red: {inner:06x}");
    assert_eq!(PAINT_OPACITY.with(|o| o.get()), 1.0, "paint restores the ambient opacity");
}

#[test]
fn q16_background_longhands_parse_and_paint() {
    let src = tiny_png_data_uri(4, 4);
    let layout_of = |css: &str| {
        let html = format!(r#"<html><body><div style="width: 13px; height: 13px; {css}"></div></body></html>"#);
        let laid = measure_fixture(&html, 200.0);
        find_by_width(&laid, 13.0).expect("div not found").style.bg_layout
    };
    let l = layout_of("background-position: right 10px bottom 20px; background-size: cover; background-repeat: no-repeat;");
    assert_eq!((l.pos_x, l.pos_y), ((1.0, -10.0), (1.0, -20.0)));
    assert_eq!(l.size, BgSize::Cover);
    assert_eq!(l.repeat, BgRepeat { x: false, y: false });
    let l = layout_of("background-position: top; background-size: 50% auto; background-repeat: repeat-x;");
    assert_eq!((l.pos_x, l.pos_y), ((0.5, 0.0), (0.0, 0.0)));
    assert_eq!(l.size, BgSize::Explicit(BgDim::Pct(0.5), BgDim::Auto));
    assert_eq!(l.repeat, BgRepeat { x: true, y: false });
    let l = layout_of(&format!("background: url('{src}') center / contain no-repeat red;"));
    assert_eq!((l.pos_x, l.pos_y), ((0.5, 0.0), (0.5, 0.0)));
    assert_eq!(l.size, BgSize::Contain);
    assert_eq!(l.repeat, BgRepeat { x: false, y: false });
    let l = layout_of("background: red; background-position: 3px 4px;");
    assert_eq!((l.pos_x, l.pos_y), ((0.0, 3.0), (0.0, 4.0)));
    assert_eq!(layout_of("background: red;"), BgLayout::default(), "shorthand resets the longhands");

    // Painting: a 2x2 red image in a 10x10 box.
    let red = TextColor::rgb(255, 0, 0);
    let image = DecodedImage { width: 2, height: 2, pixels: vec![red; 4], svg: None, mask: None };
    let paint = |layout: BgLayout, radius: f32| {
        let mut px = vec![0u32; 100];
        paint_background_image(&image, &layout, &mut px, 10, 10, 0.0, 0.0, 10.0, 10.0, radius, None);
        px
    };
    let on = |px: &[u32], x: usize, y: usize| px[y * 10 + x] & 0xFFFFFF == 0xFF0000;
    let base = BgLayout { repeat: BgRepeat { x: false, y: false }, ..BgLayout::default() };
    let px = paint(base, 0.0);
    assert!(on(&px, 1, 1) && !on(&px, 2, 0) && !on(&px, 0, 2), "no-repeat paints one 2x2 tile");
    let px = paint(BgLayout { pos_x: (1.0, 0.0), pos_y: (1.0, -1.0), ..base }, 0.0);
    assert!(on(&px, 9, 7) && on(&px, 8, 8) && !on(&px, 9, 9), "right/bottom minus 1px offset");
    let px = paint(BgLayout { size: BgSize::Cover, ..base }, 0.0);
    assert!(on(&px, 0, 0) && on(&px, 9, 9), "cover fills the box");
    let px = paint(BgLayout { size: BgSize::Explicit(BgDim::Px(6.0), BgDim::Auto), ..base }, 0.0);
    assert!(on(&px, 5, 5) && !on(&px, 6, 0) && !on(&px, 0, 6), "one explicit axis keeps the aspect");
    let px = paint(BgLayout { repeat: BgRepeat { x: true, y: false }, ..base }, 0.0);
    assert!(on(&px, 9, 1) && !on(&px, 0, 2), "repeat-x repeats only horizontally");
    let px = paint(BgLayout { size: BgSize::Cover, ..base }, 5.0);
    assert!(!on(&px, 0, 0) && on(&px, 5, 5), "rounded corners clip the image");
}

#[test]
fn background_shorthand_url_sets_background_image() {
    let src = tiny_png_data_uri(4, 4);
    let html = format!(r#"<html><body><div style="width: 13px; height: 13px; background: url('{src}');"></div></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    let div = find_by_width(&laid, 13.0).expect("div not found");
    assert!(div.background_image.is_some(), "`background: url(...)` shorthand should resolve a background-image the same as the longhand would");
}

#[test]
fn q35b_background_shorthand_takes_first_url_layer_after_gradient() {
    let src = tiny_png_data_uri(4, 4);
    for decl in [
        format!("background: linear-gradient(transparent, transparent), url('{src}')"),
        format!("background: url('{src}'), linear-gradient(red, blue)"),
        format!("background: linear-gradient(red, blue) no-repeat, url('{src}') repeat-x #fff"),
    ] {
        let html = format!(r#"<html><body><div style="width: 13px; height: 13px; {decl};"></div></body></html>"#);
        let laid = measure_fixture(&html, 200.0);
        let div = find_by_width(&laid, 13.0).expect("div not found");
        assert!(div.background_image.is_some(), "shorthand `{decl}` should keep the url layer's image");
    }
    // No url layer at all: gradient-only stays imageless; strict parsing still rejects junk.
    let html = r#"<html><body><div style="width: 13px; height: 13px; background: linear-gradient(red, blue), linear-gradient(blue, red);"></div></body></html>"#;
    let laid = measure_fixture(html, 200.0);
    assert!(find_by_width(&laid, 13.0).expect("div").background_image.is_none());
    let mut p = CssParser::new("linear-gradient(red,blue), url(x.png) repeat-x 3px 4px");
    let bg = parse_background_shorthand(&mut p, true).ok().expect("parses");
    assert_eq!(bg.image.as_deref(), Some("x.png"));
    assert!(bg.repeat.is_some() && bg.position.is_some(), "position/repeat come from the url layer");
    let mut p = CssParser::new("linear-gradient(red,blue) no-repeat, url(x.png)");
    let bg = parse_background_shorthand(&mut p, true).ok().expect("parses");
    assert!(bg.repeat.is_none(), "the gradient layer's repeat does not apply to the url layer");
    let mut p = CssParser::new("linear-gradient(red,blue), url(x.png) bogus");
    assert!(parse_background_shorthand(&mut p, true).is_err(), "QA4 strict parsing still drops junk");
}

#[test]
fn background_shorthand_ignores_unrecognized_components_but_still_sets_color() {
    let html = r#"<html><body style="margin:0"><div style="width: 40px; height: 40px; background: #0000ff no-repeat center / cover;"></div></body></html>"#;
    let doc = parse_html(html);
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
    let (win_w, win_h) = (100u32, 100u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    let px = pixels[20 * win_w as usize + 20];
    assert_eq!(
        px & 0x00FF_FFFF,
        0x0000_00FF,
        "a repeat/position/size component this file doesn't implement shouldn't throw away the color that did parse"
    );
}

#[test]
fn whitespace_between_block_siblings_generates_no_line_box() {
    // Real newlines/indentation between tags, the shape every real
    // page has (and most other fixtures here deliberately avoid).
    let laid = measure_document_fixture(
        "<html><body style=\"margin: 0\">\n  <div style=\"width: 300px\">\n    <p style=\"width: 200px; margin: 0\">A</p>\n    <p style=\"width: 100px; margin: 0\">B</p>\n  </div>\n</body></html>",
        400.0,
        400.0,
    );
    let div = find_by_width(&laid, 300.0).expect("div not found");
    let p1 = find_by_width(&laid, 200.0).expect("first p not found");
    let p2 = find_by_width(&laid, 100.0).expect("second p not found");
    assert_eq!(div.y, 0.0, "whitespace before the div shouldn't push it down");
    assert_eq!(p1.y, div.y, "whitespace before the first p shouldn't push it down");
    assert_eq!(p2.y, p1.y + p1.h, "whitespace between the two p's shouldn't take up a line");
    assert_eq!(div.h, p1.h + p2.h, "whitespace after the last p shouldn't add height");
}

#[test]
fn white_space_pre_keeps_whitespace_only_lines_between_block_siblings() {
    let laid = measure_document_fixture(
        "<html><body style=\"margin: 0\"><div style=\"width: 300px; white-space: pre\"><p style=\"width: 200px; margin: 0\">A</p>\n\n<p style=\"width: 100px; margin: 0\">B</p></div></body></html>",
        400.0,
        400.0,
    );
    let p1 = find_by_width(&laid, 200.0).expect("first p not found");
    let p2 = find_by_width(&laid, 100.0).expect("second p not found");
    assert!(
        p2.y > p1.y + p1.h,
        "white-space: pre whitespace is preserved, so it must still produce its own line(s) (p1 bottom {}, p2 top {})",
        p1.y + p1.h,
        p2.y
    );
}

#[test]
fn white_space_nowrap_keeps_a_single_line_height_even_in_a_narrow_box() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 30px; white-space: nowrap;">a long run of many words here</div></body></html>"#,
        400.0,
    );
    let div = find_by_width(&laid, 30.0).expect("div not found");
    assert!(
        div.h < 25.0,
        "white-space: nowrap should keep this text on one line even though the box is far narrower than the text, not wrap it onto several lines (got height {})",
        div.h
    );
}

#[test]
fn without_nowrap_the_same_narrow_box_wraps_onto_several_lines() {
    let laid = measure_fixture(
        r#"<html><body><div style="width: 30px;">a long run of many words here</div></body></html>"#,
        400.0,
    );
    let div = find_by_width(&laid, 30.0).expect("div not found");
    assert!(
        div.h > 60.0,
        "sanity: without nowrap, this long text in a narrow box should wrap onto several lines (got height {})",
        div.h
    );
}

#[test]
fn white_space_nowrap_also_prevents_wrapping_in_a_mixed_run_with_an_inline_image() {
    // `measure_mixed_inline` (the greedy wrapper a run containing an
    // `<img>`/inline-block takes, instead of the plain-text fast
    // path `shape_spans` uses) never looked at `white_space` at all
    // before this — nowrap text sharing a line with an image would
    // still wrap at the available width. This also matters beyond
    // the mixed-run rendering path itself: `cell_natural_width`'s
    // own min/max-content probing always goes through this same
    // wrapper (see its own docs), so a `nowrap` flex/inline-block/
    // table-cell item's *natural width* was wrong too, not just its
    // real on-page wrapping — see the flex `automatic minimum size`
    // test above, which depends on this fix for a multi-word nowrap
    // item's min-content to be computed correctly.
    let src = tiny_png_data_uri(4, 4);
    let html = format!(
        r#"<html><body><div style="width: 20px; white-space: nowrap;">a long run of many words <img src="{src}" width="4" height="4"> more words here too</div></body></html>"#
    );
    let laid = measure_fixture(&html, 400.0);
    let div = find_by_width(&laid, 20.0).expect("div not found");
    assert!(find_mixed_inline(&laid).is_some(), "sanity: an inline <img> should route this run through measure_mixed_inline, not the plain-text fast path");
    assert!(
        div.h < 25.0,
        "white-space: nowrap should keep this mixed text+image run on one line even in a box far narrower than the content, not wrap across several lines (got height {})",
        div.h
    );
}

#[test]
fn no_background_image_declared_resolves_to_none() {
    let laid = measure_fixture(r#"<html><body><div style="width: 13px; height: 13px; background-color: #00ff00;"></div></body></html>"#, 200.0);
    let div = find_by_width(&laid, 13.0).expect("div not found");
    assert!(div.background_image.is_none(), "a div with no background-image declared shouldn't resolve one, even with a background-color set");
    assert!(div.style.background_color.is_some(), "sanity: background-color itself should still resolve normally");
}

#[test]
fn background_image_none_overrides_a_rule_level_image_not_just_fail_silently() {
    let src = tiny_png_data_uri(4, 4);
    let html = format!(
        r#"<html><head><style>div {{ background-image: url('{src}'); }}</style></head>
        <body><div style="width: 13px; height: 13px; background-image: none;"></div></body></html>"#
    );
    let laid = measure_fixture(&html, 200.0);
    let div = find_by_width(&laid, 13.0).expect("div not found");
    assert!(
        div.background_image.is_none(),
        "an inline `background-image: none` should win the cascade and suppress the less-specific rule's real image, not be silently skipped as if undeclared"
    );
}

#[test]
fn background_image_without_none_still_resolves_normally() {
    let src = tiny_png_data_uri(4, 4);
    let html = format!(
        r#"<html><body><div style="width: 13px; height: 13px; background-image: url('{src}');"></div></body></html>"#
    );
    let laid = measure_fixture(&html, 200.0);
    let div = find_by_width(&laid, 13.0).expect("div not found");
    assert!(div.background_image.is_some(), "a real url() background-image should still resolve normally");
}

#[test]
fn background_image_layer_list_with_gradient_first_resolves_url_qw1() {
    // wikipedia.org's `.sprite`: gradient layer first, sprite url second.
    let uri = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'%3E%3Crect width='4' height='4' fill='red'/%3E%3C/svg%3E";
    let css = format!(".sprite{{background-image:linear-gradient(transparent,transparent),url(\"{uri}\");background-repeat:no-repeat;display:inline-block}} .wm{{background-position:0 -2px;width:17px;height:9px;color:transparent;text-indent:-10000px;overflow:hidden}}");
    let doc = parse_html("<html><body><span class=\"sprite wm\">Wikipedia</span></body></html>");
    let rules = parse_stylesheet(&css);
    let sp = find_element_by_tag(&doc, "span").expect("span");
    assert!(
        resolve_background_image(&sp, &rules, 800.0, None, &HashMap::new()).is_some(),
        "gradient,url() layer list must resolve the url layer (wikipedia.org wordmark sprite)"
    );
}

#[test]
fn mixed_inline_keeps_items_on_one_line_when_they_fit() {
    let src = tiny_png_data_uri(4, 4);
    let html = format!(r#"<html><body><p>x <img src="{src}" style="width: 10px; height: 10px"> y</p></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let items = mixed_items(run);
    // Q59: word, image, hanging space after the image, word.
    assert_eq!(items.len(), 4, "word, image, space, word");
    // One line's height is `max(word line-height, image height)` —
    // the word's own `Metrics::new(16.0, 20.0)` line-height (20px)
    // dominates a 10px-tall image, so the whole run's box should be
    // ~20px tall. Two separate lines would instead be ~40px+ (each
    // line at least 20px, image bottom-aligned within whichever is
    // taller) — a far coarser, implementation-detail-free way to
    // confirm "on one line" than comparing exact `y` offsets (the
    // word is top-aligned and the image bottom-aligned *within* a
    // shared line, see `measure_mixed_inline`'s own docs, so their
    // `y` values legitimately differ even on the same line).
    assert!(run.h < 25.0, "word+image should fit on one ~20px line, got height {}", run.h);
    let positions: Vec<(f32, f32)> = items.iter().map(mixed_item_xy).collect();
    assert!(positions[0].0 < positions[1].0, "image should sit after the first word, not stacked below it");
    assert!(positions[1].0 < positions[3].0, "trailing word should sit after the image");
}

#[test]
fn mixed_inline_wraps_when_content_does_not_fit_on_one_line() {
    // Two 50px-wide images in a 60px container can't possibly share
    // a line (50 + 50 > 60) regardless of font metrics, so this
    // pins real wrapping deterministically without depending on how
    // wide any shaped word measures.
    let src = tiny_png_data_uri(4, 4);
    let html = format!(
        r#"<html><body><div style="width: 60px"><p><img src="{src}" style="width: 50px; height: 10px"><img src="{src}" style="width: 50px; height: 10px"></p></div></body></html>"#
    );
    let laid = measure_fixture(&html, 500.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let items = mixed_items(run);
    assert_eq!(items.len(), 2);
    let positions: Vec<(f32, f32)> = items.iter().map(mixed_item_xy).collect();
    assert_ne!(positions[0].1, positions[1].1, "two 50px images in a 60px container should wrap onto separate lines");
    assert!(run.h >= 20.0 - 0.01, "two wrapped 10px-tall image lines should report roughly 20px total, got {}", run.h);
}

#[test]
fn mixed_inline_respects_text_align_center() {
    let src = tiny_png_data_uri(4, 4);
    let left_html = format!(r#"<html><body><p><img src="{src}" style="width: 10px; height: 10px"></p></body></html>"#);
    let center_html = format!(r#"<html><body><p style="text-align: center"><img src="{src}" style="width: 10px; height: 10px"></p></body></html>"#);
    let left_laid = measure_fixture(&left_html, 500.0);
    let center_laid = measure_fixture(&center_html, 500.0);
    let (left_x, _) = mixed_item_xy(&mixed_items(find_mixed_inline(&left_laid).unwrap())[0]);
    let (center_x, _) = mixed_item_xy(&mixed_items(find_mixed_inline(&center_laid).unwrap())[0]);
    assert!(center_x > left_x, "text-align: center should shift a mixed line's content rightward compared to default left alignment");
}

#[test]
fn mixed_inline_text_only_paints_once_the_whole_runs_shaped() {
    // Regression for the "text trickles in a few characters at a
    // time" symptom real-world testing (a real page, through a
    // merged PR) surfaced: `measure_mixed_inline` shapes one
    // `Buffer` per *word*, each its own `ShapeCache` id — needed so
    // its own manual wrap pass has each word's real width to wrap
    // around the image with — so a background shaping tick can
    // land with only *some* of a run's words shaped. `paint` must
    // not reveal the ones that happen to be ready yet; otherwise a
    // run's words pop in individually instead of as one chunk, the
    // way an ordinary paragraph already does.
    let src = tiny_png_data_uri(4, 4);
    let html = format!(r#"<html><body><p>x <img src="{src}" style="width: 10px; height: 10px"> y</p></body></html>"#);
    let (laid, mut cache) = measure_fixture_with_cache(&html, 500.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let word_ids: Vec<usize> = mixed_items(run)
        .iter()
        .filter_map(|item| match item {
            MixedPositioned::Word { id, .. } => Some(*id),
            MixedPositioned::Embedded(_) => None,
        })
        .collect();
    // Q59: "x ", the hanging space after the image, and "y".
    assert_eq!(word_ids.len(), 3, "expected the words (\"x \", \" \", \"y\") to each get their own id");

    let mut font_system = new_font_system();
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (200u32, 200u32);
    const BLANK: u32 = 0x00FF_FFFF;

    // Baseline: nothing in this run shaped at all (the very first
    // tick, before any word has been shaped) — only the image
    // should paint.
    let mut empty_cache = ShapeCache::new();
    let mut image_only_pixels = vec![BLANK; (win_w * win_h) as usize];
    paint(
        &laid, &mut font_system, &mut swash_cache, &mut empty_cache, &HashMap::new(), &mut image_only_pixels,
        win_w, win_h, 0.0, None,
    );
    assert!(image_only_pixels.iter().any(|&p| p != BLANK), "the image itself should still paint with no words shaped at all");

    // Only the *first* word shaped (the second removed, as if a
    // background tick hadn't reached it yet) — before this fix,
    // the first word's own text would have painted on its own;
    // now the whole run should still wait, producing pixels
    // identical to the "nothing shaped yet" baseline above.
    cache.remove(&word_ids[1]);
    let mut partial_pixels = vec![BLANK; (win_w * win_h) as usize];
    paint(
        &laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut partial_pixels, win_w,
        win_h, 0.0, None,
    );
    assert_eq!(
        partial_pixels, image_only_pixels,
        "no text should paint while any word in the run is still unshaped, even an already-shaped one"
    );
}

// ---------- Q35b2: image arrivals are debounced ----------

#[test]
fn q35b2_image_relayout_wait_is_pure_debounce() {
    let ms = std::time::Duration::from_millis;
    // First arrival on an idle page waits the whole window, no more.
    assert_eq!(image_relayout_wait(None, false), Some(IMAGE_RELAYOUT_DEBOUNCE));
    assert!(IMAGE_RELAYOUT_DEBOUNCE <= ms(100));
    // Part-way through: the remainder, never extended by later arrivals.
    assert_eq!(image_relayout_wait(Some(ms(30)), false), Some(IMAGE_RELAYOUT_DEBOUNCE - ms(30)));
    // Window passed: rebuild now (the timer flushes even with no more events).
    assert_eq!(image_relayout_wait(Some(IMAGE_RELAYOUT_DEBOUNCE), false), None);
    assert_eq!(image_relayout_wait(Some(ms(5000)), false), None);
    // A rebuild is already queued (first layout, chunk, resize): nothing to defer.
    assert_eq!(image_relayout_wait(None, true), None);
}

#[test]
fn q35b2_image_burst_collapses_and_flushes() {
    // Without a display, only the pure decision above can be checked.
    let event_loop = match test_event_loop() {
        Ok(l) => l,
        Err(_) => {
            assert_eq!(image_relayout_wait(None, false), Some(IMAGE_RELAYOUT_DEBOUNCE));
            return;
        }
    };
    let mut app = App::new_static(parse_html("<html><body><img src=\"a.png\"><img src=\"b.png\"></body></html>"), event_loop.create_proxy(), None);
    let img = || Some(DecodedImage { width: 1, height: 1, pixels: vec![TextColor::rgb(1, 2, 3)], svg: None, mask: None });
    let generation = app.nav_generation;

    // First layout still queued: arrivals add nothing extra and are not deferred.
    assert!(app.layout_dirty);
    app.handle_image_fetched(generation, "a.png".into(), img());
    assert!(app.image_relayout_since.is_none());
    assert!(app.image_cache.contains_key("a.png"));

    // Idle page with a finished layout: the image is cached at once but the
    // rebuild waits for the window.
    app.layout_dirty = false;
    app.layout_in_flight = false;
    let before = app.layout_generation;
    app.handle_image_fetched(generation, "b.png".into(), img());
    assert!(app.image_cache.contains_key("b.png"));
    assert!(!app.layout_dirty, "rebuild is deferred by the debounce window");
    assert!(app.image_relayout_since.is_some());
    assert_eq!(app.layout_generation, before, "no stale-making bump until the flush");

    // A burst member inside the window does not rebuild either.
    app.handle_image_fetched(generation, "c.png".into(), img());
    assert!(!app.layout_dirty);

    // Window passed but a pass is in flight: still deferred (QB2: no stale frame).
    app.image_relayout_since = Some(std::time::Instant::now() - IMAGE_RELAYOUT_DEBOUNCE * 2);
    app.layout_in_flight = true;
    app.flush_image_relayout();
    assert!(!app.layout_dirty);
    // The pass completes (handle_shape_result calls flush): one rebuild.
    app.layout_in_flight = false;
    app.flush_image_relayout();
    assert!(app.layout_dirty);
    assert!(app.image_relayout_since.is_none());
    assert_eq!(app.layout_generation, before + 1);

    // A stale-generation image is dropped entirely (R7).
    app.layout_dirty = false;
    app.handle_image_fetched(generation + 1, "stale".into(), img());
    assert!(!app.image_cache.contains_key("stale"));
    assert!(app.image_relayout_since.is_none());

    // A navigation clears any pending debounce.
    app.handle_image_fetched(generation, "d.png".into(), img());
    assert!(app.image_relayout_since.is_some());
    app.replace_document(parse_html("<html></html>"), String::new());
    assert!(app.image_relayout_since.is_none());
}

#[test]
fn qa18a_create_html_document_has_skeleton_and_own_tree() {
    let log = run_in_html(
        "<html><head><title>P</title></head><body><p id=x>page</p></body></html>",
        "var d=document.implementation.createHTMLDocument('T');\
         console.log(d.nodeType, d.nodeName, d!==document, d instanceof Document, d.title, d.documentElement.tagName,\
           d.head.tagName, d.body.tagName, d.doctype.name, d.contentType, d.URL, d.defaultView, d.location);\
         var e=d.createElement('div'); e.id='in'; d.body.appendChild(e);\
         console.log(d.getElementById('in')===e, document.getElementById('in'), d.getElementById('x'),\
           d.querySelector('#in')===e, d.querySelectorAll('div').length, document.querySelectorAll('div').length);\
         console.log(e.ownerDocument===d, document.body.ownerDocument===document, d.createElement('b').ownerDocument===d, document.createElement('b').ownerDocument===document);\
         d.title='N'; console.log(d.title, document.title, d.body.parentNode===d.documentElement, d.parentNode, d.childNodes.length);",
    );
    assert_eq!(
        log,
        [
            "log:9 #document true true T HTML HEAD BODY html text/html about:blank null null",
            "log:true null null true 1 0",
            "log:true true true true",
            "log:N  true null 2"
        ],
        "{log:?}"
    );
}

#[test]
fn qa18a_import_and_adopt_node_and_create_document_type() {
    let log = run_in_html(
        "<html><body><div id=s><b>x</b></div></body></html>",
        "var d=document.implementation.createHTMLDocument('');\
         var s=document.getElementById('s'); var c=d.importNode(s,true);\
         console.log(c.ownerDocument===d, c.parentNode, c.innerHTML, s.ownerDocument===document, c!==s);\
         var u=document.createElement('u'); var a=d.adoptNode(u); d.body.appendChild(a);\
         console.log(a===u, u.ownerDocument===d, d.body.firstChild===u, u.parentNode===d.body);\
         var dt=document.implementation.createDocumentType('svg','pub','sys');\
         console.log(dt.nodeType, dt.name, dt.publicId, dt.systemId, dt.nodeName, dt instanceof DocumentType);\
         var x=document.implementation.createDocument('http://www.w3.org/2000/svg','svg',null);\
         console.log(x.documentElement.tagName, x.contentType, x.body, document.implementation.hasFeature('x','1.0'));\
         var thrown=''; try { document.implementation.createDocumentType('1 bad','','') } catch (e) { thrown=e.name }\
         console.log(thrown);",
    );
    assert_eq!(
        log,
        [
            "log:true null <b>x</b> true true",
            "log:true true true true",
            "log:10 svg pub sys svg true",
            "log:svg image/svg+xml null true",
            "log:InvalidCharacterError"
        ],
        "{log:?}"
    );
}

// ---------- QY56: images that appear after load ----------

const QY56_PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

#[test]
fn qy56_script_set_and_created_imgs_are_requested_with_state_and_events() {
    set_page_url(Url::parse("https://page.test/dir/").ok().as_ref());
    let (engine, _doc, log) = events_engine(
        "<html><body><img id=s></body></html>",
        "var s=document.getElementById('s');
         s.onload=function(){console.log('s-load '+s.complete+' '+s.naturalWidth+'x'+s.naturalHeight)};
         s.onerror=function(){console.log('s-error')};
         s.src='a.png';
         var c=new Image();
         if(!(c instanceof HTMLImageElement)) throw new Error('not an img');
         c.addEventListener('load',function(){console.log('c-load '+c.complete)});
         c.addEventListener('error',function(){console.log('c-error '+c.complete+' '+c.naturalWidth)});
         c.setAttribute('src','b.png');
         document.body.appendChild(c);
         console.log('before '+s.complete+' '+s.naturalWidth+' '+c.complete);",
    );
    let reqs = engine.take_requests();
    assert_eq!(reqs.imgs, ["a.png", "b.png"]);
    assert_eq!(log.borrow().iter().filter(|l| l.contains("before false 0 false")).count(), 1, "{:?}", log.borrow());
    crate::js_dom::note_image_state("a.png", Some((7, 5)));
    engine.eval("done-a", "__scratchy_img_done('a.png')");
    crate::js_dom::note_image_state("b.png", None);
    engine.eval("done-b", "__scratchy_img_done('b.png')");
    // A second notification must not fire again.
    engine.eval("done-a2", "__scratchy_img_done('a.png')");
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| x.contains("s-load true 7x5")).count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| x.contains("c-error true 0")).count(), 1, "{l:?}");
    assert!(!l.iter().any(|x| x.contains("s-error") || x.contains("c-load")), "{l:?}");
    crate::js_dom::clear_image_states();
}

#[test]
fn qy56_data_url_image_is_complete_at_once_and_loads_as_a_task() {
    let (engine, _doc, log) = events_engine(
        "<html><body></body></html>",
        &format!(
            "var i=new Image(); i.onload=function(){{console.log('load '+i.naturalWidth)}};
             i.src='{QY56_PIXEL}';
             console.log('now '+i.complete+' '+i.naturalWidth+' '+i.naturalHeight);
             i.decode().then(function(){{console.log('decoded')}});
             var bad=new Image(); bad.src='data:image/png;base64,AAAA';
             bad.decode().catch(function(e){{console.log('rejected '+e.name)}});"
        ),
    );
    assert!(engine.take_requests().imgs.is_empty(), "data: URLs need no fetch");
    assert!(log.borrow().iter().any(|l| l.contains("now true 1 1")), "{:?}", log.borrow());
    assert!(!log.borrow().iter().any(|l| l.contains("load 1")), "load is asynchronous");
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| x.contains("load 1")).count(), 1, "{l:?}");
    assert!(l.iter().any(|x| x.contains("decoded")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("rejected EncodingError")), "{l:?}");
}

#[test]
fn qy56_decode_waits_for_the_fetch_and_img_without_src_is_complete() {
    let (engine, _doc, log) = events_engine(
        "<html><body><img id=n></body></html>",
        "var n=document.getElementById('n'); console.log('nosrc '+n.complete);
         var i=new Image(); i.src='late.png'; i.decode().then(function(){console.log('decode-ok '+i.naturalWidth)});",
    );
    assert!(log.borrow().iter().any(|l| l.contains("nosrc true")));
    assert!(!log.borrow().iter().any(|l| l.contains("decode-ok")));
    crate::js_dom::note_image_state("late.png", Some((3, 2)));
    engine.eval("d", "__scratchy_img_done('late.png')");
    engine.drain_after_task();
    assert!(log.borrow().iter().any(|l| l.contains("decode-ok 3")), "{:?}", log.borrow());
    crate::js_dom::clear_image_states();
}

#[test]
fn qy56_inserted_and_parsed_imgs_get_events_via_the_sweep() {
    let (engine, _doc, log) = events_engine(
        "<html><body><img id=p src=static.png></body></html>",
        "var d=document.createElement('div');
         d.innerHTML='<img id=h src=\"html.png\" onload=\"console.log(\\'h-load\\')\">';
         document.body.appendChild(d);
         document.getElementById('p').addEventListener('load',function(){console.log('p-load')});",
    );
    assert!(engine.take_requests().imgs.is_empty(), "parser and innerHTML images are fetched via the layout path");
    crate::js_dom::note_image_state("static.png", Some((2, 2)));
    engine.eval("d1", "__scratchy_img_done('static.png')");
    crate::js_dom::note_image_state("html.png", Some((2, 2)));
    engine.eval("d2", "__scratchy_img_done('html.png')");
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| x.contains("p-load")).count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| x.contains("h-load")).count(), 1, "{l:?}");
    crate::js_dom::clear_image_states();
}

#[test]
fn qy56_layout_build_records_missed_img_srcs_once() {
    let _ = crate::images::take_wanted_img_srcs();
    let doc = parse_html(&format!(
        "<html><body><img src=\"x/one.png\"><img src=\"x/one.png\"><img src=\"two.png\"><img src=\"{QY56_PIXEL}\"><img></body></html>"
    ));
    let mut cache = HashMap::new();
    cache.insert("two.png".to_string(), Arc::new(DecodedImage { width: 1, height: 1, pixels: vec![TextColor::rgb(1, 2, 3)], svg: None, mask: None }));
    let _ = build_layout_tree_with_resources(&doc, 800.0, Some(600.0), "", &cache);
    assert_eq!(crate::images::take_wanted_img_srcs(), ["x/one.png"]);
    assert!(crate::images::take_wanted_img_srcs().is_empty(), "drained");
}

#[test]
fn qy56_image_fetch_claims_dedupe_per_generation() {
    let g = 9_100_001;
    assert!(crate::net::claim_image_fetch(g, "a.png"));
    assert!(!crate::net::claim_image_fetch(g, "a.png"), "the same URL is fetched once");
    assert!(crate::net::claim_image_fetch(g, "b.png"));
    assert!(crate::net::image_fetch_claimed(g, "b.png"));
    assert!(crate::net::claim_image_fetch(g + 1, "a.png"), "a new page starts over");
    assert!(!crate::net::claim_image_fetch(g, "zzz.png"), "a stale generation claims nothing");
}

#[test]
fn qy56_late_image_cap_is_honoured_and_extra_images_error() {
    let Ok(event_loop) = test_event_loop() else { return };
    let mut app = App::new_static(parse_html("<html><body></body></html>"), event_loop.create_proxy(), Some(PathBuf::from("/nonexistent/qy56.sock")));
    app.nav_generation = 9_200_001;
    let srcs: Vec<String> = (0..MAX_LATE_IMAGE_FETCHES + 10).map(|i| format!("cap{i}.png")).collect();
    let (p, s2) = (app.proxy.clone(), app.socket_path.clone());
    let go = |app: &mut App, v: Vec<String>| fetch_late_images(&p, s2.as_deref(), None, &mut app.late_images_requested, &mut app.late_image_errors, 9_200_001, v);
    go(&mut app, srcs.clone());
    assert_eq!(app.late_images_requested, MAX_LATE_IMAGE_FETCHES + 1, "cap reached, logged once");
    assert_eq!(app.late_image_errors.len(), 10, "the capped ones fail with error");
    // Repeating the same URLs requests nothing new.
    go(&mut app, srcs);
    assert_eq!(app.late_images_requested, MAX_LATE_IMAGE_FETCHES + 1);
    crate::js_dom::clear_image_states();
}

// QY61: Element.insertAdjacentElement/Text/HTML (en.wikipedia skins.vector.js
// movePinnableElement died with "TypeError: not a function").
#[test]
fn qy61_insert_adjacent_element_text_and_html() {
    let (_engine, _doc, log) = events_engine(
        "<html><body><div id=p><b id=m>m</b></div><div id=q></div></body></html>",
        "var p=document.getElementById('p'), m=document.getElementById('m'), q=document.getElementById('q');
         var r=q.insertAdjacentElement('beforeend', m);
         console.log('ret '+(r===m)+' q='+q.innerHTML+' p='+p.innerHTML);
         p.insertAdjacentText('afterbegin','a'); p.insertAdjacentText('beforeEnd','z');
         p.insertAdjacentHTML('beforebegin','<i>x</i>'); p.insertAdjacentHTML('afterend','<u>y</u><s>w</s>');
         console.log('p='+p.innerHTML+' body='+document.body.innerHTML);
         var d=document.createElement('div'); console.log('orphan '+d.insertAdjacentElement('afterend', m));
         try { p.insertAdjacentText('bogus','t'); } catch (e) { console.log('err '+e.name); }",
    );
    let l = log.borrow().join("\n");
    assert!(l.contains("ret true q=<b id=\"m\">m</b> p="), "{l}");
    assert!(l.contains("p=az "), "{l}");
    assert!(l.contains("<i>x</i><div id=\"p\">az</div><u>y</u><s>w</s><div id=\"q\">"), "{l}");
    assert!(l.contains("orphan null"), "{l}");
    assert!(l.contains("err SyntaxError"), "{l}");
}

fn qinc1k2_images(b: &LayoutBox, out: &mut Vec<Arc<DecodedImage>>) {
    match &b.content {
        BoxContent::Image(i) => out.push(i.clone()),
        BoxContent::Inline(items) => {
            for i in items {
                match i {
                    InlineItem::Image { image, .. } => out.push(image.clone()),
                    InlineItem::Embedded(b) => qinc1k2_images(b, out),
                    _ => {}
                }
            }
        }
        BoxContent::Blocks(bs) | BoxContent::TableRow(bs) => bs.iter().for_each(|b| qinc1k2_images(b, out)),
    }
}

fn qinc1k2_svgs(h: &Handle, out: &mut Vec<Handle>) {
    if matches!(h.data, NodeData::Element { .. }) && element_tag(h) == "svg" {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        qinc1k2_svgs(c, out);
    }
}

/// QINC1k2: two mounts of the same inline-SVG component that differ only in generated ids (React
/// `useId`: github.com's card background grids) share one rasterization through the inline-SVG cache,
/// and the pixels are those of the original markup (the pattern still resolves after the renaming).
#[test]
fn qinc1k2_inline_svg_ids_canonicalized_and_rasterized_once() {
    let grid = |id: &str| {
        format!(
            r##"<svg width="8" height="8" viewBox="0 0 8 8"><defs><pattern id="grid-{id}" width="2" height="2" patternUnits="userSpaceOnUse"><rect width="1" height="2" fill="#00ff00"></rect></pattern></defs><rect width="8" height="8" fill="url(#grid-{id})"></rect></svg>"##
        )
    };
    let html = format!(r#"<html><body style="margin:0">{}<div>{}</div><p>#grid-_r_1_ stays text</p></body></html>"#, grid("_r_1_"), grid("_r_2_"));
    let doc = parse_html(&html);
    let mut svgs = Vec::new();
    qinc1k2_svgs(&doc, &mut svgs);
    assert_eq!(svgs.len(), 2);
    let black = TextColor::rgba(0, 0, 0, 255);
    let raw = crate::images::serialize_inline_svg(&svgs[0], black);
    let a = crate::images::canonical_svg_ids(raw.clone());
    let b = crate::images::canonical_svg_ids(crate::images::serialize_inline_svg(&svgs[1], black));
    assert_eq!(a, b, "only the generated ids differed");
    assert!(a.contains(r#"id="scratchy-svg-id-0""#) && a.contains("url(#scratchy-svg-id-0)"), "{a}");
    assert!(!a.contains("_r_1_"), "{a}");
    crate::images::inline_svg_cache_clear();
    let one = crate::images::decode_inline_svg(a).expect("decodes");
    let two = crate::images::decode_inline_svg(b).expect("decodes");
    assert!(Arc::ptr_eq(&one, &two), "the second mount is a cache hit");
    assert_eq!(crate::images::inline_svg_cache_mem().0, 1);
    // Same pixels as the original markup; the renamed pattern still paints (green first column of
    // each 2 px tile, transparent second column).
    let orig = crate::images::decode_svg_bytes(raw.as_bytes()).expect("decodes");
    assert_eq!(one.pixels, orig.pixels);
    assert_eq!(one.pixels[0], TextColor::rgba(0, 255, 0, 255));
    assert_eq!(one.pixels[1].a(), 0);
    // The box build goes through the cache: both boxes carry the one image.
    let tree = build_layout_tree(&doc, 400.0);
    let mut imgs = Vec::new();
    qinc1k2_images(&tree, &mut imgs);
    assert_eq!(imgs.len(), 2);
    assert!(Arc::ptr_eq(&imgs[0], &imgs[1]));
}

/// QINC1k2: the renaming is conservative: markup it cannot prove self-contained is left alone.
#[test]
fn qinc1k2_canonical_svg_ids_leaves_unprovable_markup_alone() {
    let c = crate::images::canonical_svg_ids;
    for s in [
        // No ids.
        r#"<svg><rect width="1" height="1"></rect></svg>"#,
        // A <style> element may select by id.
        r#"<svg><style>#g1 { fill: red }</style><rect id="g1"></rect></svg>"#,
        // Mentioned by something that is not a recognised reference (SMIL timing).
        r#"<svg><rect id="a1"></rect><animate begin="a1.end"></animate></svg>"#,
        // `#id` in text content is never rewritten, so the id stays mentioned.
        r##"<svg><rect id="lbl" fill="url(#lbl)"></rect><text>#lbl </text></svg>"##,
        // Already uses the prefix.
        r#"<svg><rect id="scratchy-svg-id-0"></rect></svg>"#,
    ] {
        assert_eq!(c(s.to_string()), s);
    }
    // An id that is a prefix of another: each reference goes to its own id.
    let out = c(r##"<svg><g id="k"></g><g id="kq"></g><use href="#kq"></use><use href="#k"></use></svg>"##.to_string());
    assert_eq!(
        out,
        r##"<svg><g id="scratchy-svg-id-0"></g><g id="scratchy-svg-id-1"></g><use href="#scratchy-svg-id-1"></use><use href="#scratchy-svg-id-0"></use></svg>"##
    );
}
