use super::*;

// ---------- v36: `rem` resolves against `<html>`'s real font-size ----------

fn find_deepest_nonzero_margin_top(b: &LayoutBox) -> Option<&LayoutBox> {
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_deepest_nonzero_margin_top(c) {
                return Some(found);
            }
        }
    }
    if b.style.margin.top.px() != 0.0 {
        return Some(b);
    }
    None
}

#[test]
fn rem_resolves_against_html_font_size() {
    let doc = parse_html(
        r#"<html style="font-size: 40px"><body><div style="margin-top: 2rem">x</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
    assert_eq!(div.style.margin.top.px(), 80.0, "2rem should resolve against html's 40px, giving 80px");
}

#[test]
fn rem_defaults_to_16_without_html_override() {
    let doc = parse_html(r#"<html><body><div style="margin-top: 2rem">x</div></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
    assert_eq!(div.style.margin.top.px(), 32.0);
}

// ---------- v37: `:link`/`:visited` pseudo-classes ----------

fn find_first_span_color(b: &LayoutBox) -> Option<TextColor> {
    if let BoxContent::Inline(items) = &b.content {
        if let Some(InlineItem::Text(_, s)) = items.first() {
            return Some(s.color);
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_first_span_color(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn link_pseudo_matches_anchor_with_href() {
    let doc = parse_html(
        r#"<html><body><style>a:link { color: #ff0000; } a:visited { color: #00ff00; }</style><a href="x">hi</a></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_eq!(color, TextColor::rgb(0xff, 0, 0), ":link should match an <a href> and apply red");
}

#[test]
fn link_pseudo_does_not_match_anchor_without_href() {
    let doc = parse_html(r#"<html><body><style>a:link { color: #ff0000; }</style><a>hi</a></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_ne!(color, TextColor::rgb(0xff, 0, 0), ":link should not match an <a> without href");
}

#[test]
fn visited_never_matches() {
    let doc = parse_html(
        r#"<html><body><style>a:visited { color: #00ff00; }</style><a href="x">hi</a></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_ne!(color, TextColor::rgb(0, 0xff, 0), ":visited should never match");
}

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

// ---------- v42: `width`/`height` CSS properties ----------

fn measure_fixture(html: &str, available_width: f32) -> Laid {
    measure_fixture_h(html, available_width, None)
}

/// Same as `measure_fixture`, but also lets a test supply a definite
/// containing-block height for the document root (see `measure`'s
/// own `containing_height` docs) — needed to exercise a `%` `height`/
/// `min-height`/`max-height` declared on a top-level element, which
/// `measure_fixture`'s plain `None` (indefinite) deliberately can't
/// resolve, matching a real UA only ever having one via its viewport.
fn measure_fixture_h(html: &str, available_width: f32, containing_height: Option<f32>) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure(
        &tree, &mut font_system, 0.0, 0.0, available_width, containing_height, &mut next_id, &mut cache, deadline,
        &mut incomplete, &mut Vec::new(), &Floats::default(), None,
    )
}

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

fn find_laid_by_margin_top(laid: &Laid, target: f32) -> Option<&Laid> {
    if laid.style.margin.top.px() == target {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_laid_by_margin_top(c, target) {
                return Some(found);
            }
        }
    }
    None
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

fn find_by_height(laid: &Laid, target_h: f32) -> Option<&Laid> {
    if laid.style.height.map(BoxLength::px) == Some(target_h) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_by_height(c, target_h) {
                return Some(found);
            }
        }
    }
    None
}

fn find_by_width(laid: &Laid, target_w: f32) -> Option<&Laid> {
    if laid.style.width.map(BoxLength::px) == Some(target_w) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_by_width(c, target_w) {
                return Some(found);
            }
        }
    }
    None
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

// ---------- v44: structural pseudo-classes (`:not()`, `:nth-child()`, `:first-child`) ----------
// These aren't `OurPseudoClass` variants at all — `:link`/`:visited` need custom
// state scratchy tracks itself, but `:not()`/`:nth-child()`/`:first-child` are
// built into the `selectors` crate's own matching, driven purely by the
// `SelectorsElement` methods (`prev_sibling_element`, `next_sibling_element`,
// `is_root`, ...) already implemented for `El`. Never actually verified before.

fn find_all_span_colors(b: &LayoutBox, out: &mut Vec<TextColor>) {
    if let BoxContent::Inline(items) = &b.content {
        for item in items {
            if let InlineItem::Text(_, s) = item {
                out.push(s.color);
            }
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            find_all_span_colors(c, out);
        }
    }
}

#[test]
fn not_pseudo_excludes_matching_element() {
    let doc = parse_html(
        r#"<html><head><style>p:not(.skip) { color: #ff0000; }</style></head><body><p>one</p><p class="skip">two</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert_eq!(colors.len(), 2, "expected two paragraphs' worth of spans");
    assert_eq!(colors[0], TextColor::rgb(0xff, 0, 0), ":not(.skip) should match the first <p>");
    assert_ne!(colors[1], TextColor::rgb(0xff, 0, 0), ":not(.skip) should not match the second <p>");
}

#[test]
fn nth_child_matches_correct_position() {
    let doc = parse_html(
        r#"<html><head><style>p:nth-child(2) { color: #00ff00; }</style></head><body><p>one</p><p>two</p><p>three</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert_eq!(colors.len(), 3);
    assert_ne!(colors[0], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should not match the 1st <p>");
    assert_eq!(colors[1], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should match the 2nd <p>");
    assert_ne!(colors[2], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should not match the 3rd <p>");
}

#[test]
fn first_child_matches_only_first() {
    let doc = parse_html(
        r#"<html><head><style>p:first-child { color: #0000ff; }</style></head><body><p>one</p><p>two</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert_eq!(colors.len(), 2);
    assert_eq!(colors[0], TextColor::rgb(0, 0, 0xff), ":first-child should match the 1st <p>");
    assert_ne!(colors[1], TextColor::rgb(0, 0, 0xff), ":first-child should not match the 2nd <p>");
}

// ---------- Q2: dropped pseudo-classes now parse ----------

#[test]
fn q2_state_pseudo_classes_parse_and_match() {
    let red = TextColor::rgb(0xff, 0, 0);
    let run = |html: &str| {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc, 1920.0);
        let mut colors = Vec::new();
        find_all_span_colors(&tree, &mut colors);
        colors
    };
    // With no interaction state (Q31), :hover never matches, and doesn't
    // drop the other selector in the list.
    set_interaction(&InteractionState::default());
    let c = run(r#"<html><head><style>p:hover, p.a { color: #ff0000; } p:not(:focus).b { color: #ff0000; }</style></head><body><p>x</p><p class="a">y</p><p class="b">z</p></body></html>"#);
    assert_ne!(c[0], red);
    assert_eq!(c[1], red);
    assert_eq!(c[2], red, ":not(:focus) should match");
    // :checked / :disabled / :enabled
    // :dir / :lang
    let c = run(r#"<html lang="en-GB"><head><style>p:dir(ltr) { color: #ff0000; } .r:dir(rtl) { color: #00ff00; } .e:lang(en) { color: #0000ff; } .f:lang(fr) { color: #00ff00; }</style></head><body><p class="r e f">x</p></body></html>"#);
    assert_ne!(c[0], TextColor::rgb(0, 0xff, 0));
    assert_eq!(c[0], TextColor::rgb(0, 0, 0xff), ":lang(en) matches en-GB, last matching rule wins");
}

fn find_by_tag(h: &Handle, tag: &str) -> Option<Handle> {
    if element_tag(h) == tag {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_tag(c, tag))
}

#[test]
fn q31b_target_fragment_geometry_and_focus_ring() {
    let red = TextColor::rgb(0xff, 0, 0);
    let html = r#"<html><head><style>p:target { color: #ff0000; }</style></head><body>
        <div style="height:900px">spacer</div>
        <p id="sec two">section</p><a name="old">anchor</a>
        <p><a href="/x" id="lnk">link text</a></p></body></html>"#;
    let doc = parse_html(html);
    set_interaction(&InteractionState::default());
    let colors = || {
        let tree = build_layout_tree(&doc, 400.0);
        let mut c = Vec::new();
        find_all_span_colors(&tree, &mut c);
        c
    };
    assert!(!colors().contains(&red));
    assert_eq!(state_rules_mask(), STATE_TARGET);
    // Fragment lookup: id, <a name>, percent-decoding, empty.
    let p = find_fragment_target(&doc, &percent_decode_fragment("sec%20two")).expect("by id");
    assert_eq!(element_tag(&p), "p");
    assert!(find_fragment_target(&doc, "old").is_some(), "by <a name>");
    assert!(find_fragment_target(&doc, "nope").is_none() && find_fragment_target(&doc, "").is_none());
    assert_eq!(percent_decode_fragment("a%2"), "a%2", "malformed escapes are left alone");
    // :target matches exactly that element.
    let mut st = InteractionState::default();
    st.target = Some(node_id(&p));
    set_interaction(&st);
    assert!(colors().contains(&red), "p:target applies");
    set_interaction(&InteractionState::default());
    // Geometry: the block's box and the inline link's glyph run are
    // found below the 900px spacer, which is what scrolling uses.
    let tree = build_layout_tree(&doc, 400.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let pg = node_geometry(&laid, &cache, node_id(&p));
    assert!(pg.rects.first().is_some_and(|r| r.1 >= 900.0), "block target: {:?}", pg.rects);
    let a = find_fragment_target(&doc, "lnk").unwrap();
    let ag = node_geometry(&laid, &cache, node_id(&a));
    assert!(ag.rects.first().is_some_and(|r| r.1 > pg.rects[0].1 && r.2 > 0.0), "inline link glyph run: {:?}", ag.rects);
    assert!(!ag.outline);
    assert!(node_geometry(&laid, &cache, 0).rects.is_empty());
    // The default ring is painted just outside the rect, not inside it.
    let mut px = vec![0x00FFFFFFu32; 100 * 100];
    paint_focus_ring(&mut px, 100, 100, 40.0, 40.0, 20.0, 20.0);
    assert_eq!(px[50 * 100 + 50], 0x00FFFFFF, "inside untouched");
    assert_ne!(px[50 * 100 + 38], 0x00FFFFFF, "left edge of the ring");
    assert_ne!(px[38 * 100 + 50], 0x00FFFFFF, "top edge of the ring");
}

#[test]
fn q31_hover_active_focus_state_matches_and_tracks_the_element_chain() {
    let red = TextColor::rgb(0xff, 0, 0);
    let blue = TextColor::rgb(0, 0, 0xff);
    let html = r#"<html><head><style>
        a:hover { color: #ff0000; } a:active { color: #0000ff; }
        div:hover > p { color: #00ff00; } a:focus-visible { color: #ff0000; }
        div:focus-within { color: #0000ff; }
        </style></head><body><div><p>plain</p><a href="/x" tabindex="0">link</a></div><b tabindex="-1">no</b></body></html>"#;
    let doc = parse_html(html);
    let color_of = |tag: &str| {
        let tree = build_layout_tree(&doc, 1920.0);
        let mut colors = Vec::new();
        let _ = tag;
        find_all_span_colors(&tree, &mut colors);
        colors
    };
    set_interaction(&InteractionState::default());
    let base = color_of("a");
    assert!(!base.contains(&red), "no state: nothing hovered");
    assert_eq!(state_rules_mask(), STATE_HOVER | STATE_ACTIVE | STATE_FOCUS);
    let a = find_by_tag(&doc, "a").unwrap();
    let div = find_by_tag(&doc, "div").unwrap();
    // Hover: the element and its ancestors, found through the real hit test.
    let doc_tree = build_layout_tree(&doc, 400.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &doc_tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let (gx, gy) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("link glyph");
    let hit = hit_test_element(&laid, gx, gy, &cache, &HashMap::new(), 0.0, 600.0);
    assert_eq!(hit, node_id(&a), "the hit test names the <a> under the mouse");
    let mut st = InteractionState::default();
    st.hover = ancestor_ids(&a).into_iter().collect();
    assert!(st.hover.contains(&node_id(&div)), "ancestors are part of the chain");
    set_interaction(&st);
    let c = color_of("a");
    assert!(c.contains(&red), "a:hover applies: {c:?}");
    assert!(c.contains(&TextColor::rgb(0, 0xff, 0)), "div:hover > p applies via the ancestor chain");
    // Active beats hover (later rule, same specificity).
    st.active = st.hover.clone();
    set_interaction(&st);
    assert!(color_of("a").contains(&blue));
    // Focus: :focus-visible only after keyboard focus; :focus-within on the ancestor.
    let mut st = InteractionState::default();
    let ids = ancestor_ids(&a);
    st.focus = Some(ids[0]);
    st.focus_within = ids.into_iter().collect();
    set_interaction(&st);
    let c = color_of("a");
    assert!(!c.contains(&red), "mouse focus is not :focus-visible");
    assert!(c.contains(&blue), "div:focus-within applies");
    st.focus_visible = true;
    set_interaction(&st);
    assert!(color_of("a").contains(&red), ":focus-visible after keyboard focus");
    set_interaction(&InteractionState::default());
}

#[test]
fn q31_focusable_elements_and_tab_order() {
    let doc = parse_html(
        r#"<html><body><a href="/1">1</a><a>no href</a><span tabindex="0" id="s">2</span><b tabindex="-1">3</b><i tabindex="1" id="first">4</i></body></html>"#,
    );
    let order = focusable_in_tab_order(&doc);
    let tags: Vec<&str> = order.iter().map(|h| element_tag(h)).collect();
    assert_eq!(tags, ["i", "a", "span"], "positive tabindex first, then document order, -1 and href-less skipped");
}

#[test]
fn q31_state_rule_mask_is_zero_when_no_stylesheet_uses_state() {
    let doc = parse_html("<html><head><style>p { color: red }</style></head><body><p>x</p></body></html>");
    let _ = build_layout_tree(&doc, 400.0);
    assert_eq!(state_rules_mask(), 0, "no restyle will ever be needed for state on such a page");
}

#[test]
fn q2_checked_disabled_enabled_match_elements() {
    let doc = parse_html(
        r#"<html><head><style>button:disabled { color: #ff0000; } button:enabled { color: #0000ff; }</style></head><body><p><button disabled>a</button><button>b</button></p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert!(colors.contains(&TextColor::rgb(0xff, 0, 0)), ":disabled should match: {colors:?}");
    assert!(colors.contains(&TextColor::rgb(0, 0, 0xff)), ":enabled should match: {colors:?}");
    let parse = |sel: &str| {
        let mut p = CssParser::new(sel);
        SelectorList::parse(&OurSelectorParser, &mut p, ParseRelative::No).is_ok()
    };
    for s in [
        "a:checked", "a:disabled", "a:enabled", "a:dir(rtl)", "a:lang(en)", "a:focus-within",
        "a:target", "a:focus-visible", "a:active",
    ] {
        assert!(parse(s), "{s} should parse");
    }
    assert!(!parse("a:dir(up)"));
}

#[test]
fn q3_pseudo_elements_parse_but_never_match_elements() {
    let parse = |sel: &str| {
        let mut p = CssParser::new(sel);
        SelectorList::parse(&OurSelectorParser, &mut p, ParseRelative::No).is_ok()
    };
    for s in [
        "a::before", "a::after", "li::marker", "input::placeholder", "p::selection",
        "a:hover::after", "div::-webkit-scrollbar", "a, a::after", "p::BEFORE",
    ] {
        assert!(parse(s), "{s} should parse");
    }
    assert!(!parse("a::bogus"));
    let doc = parse_html(
        r#"<html><head><style>a, a::after { color: #0000ff; } p::before { color: #ff0000; }</style></head><body><a href="x">l</a><p>t</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert!(colors.contains(&TextColor::rgb(0, 0, 0xff)), "`a` keeps its blue: {colors:?}");
    assert!(!colors.contains(&TextColor::rgb(0xff, 0, 0)), "p::before must not color p: {colors:?}");
}

fn q19_texts(b: &LayoutBox, out: &mut Vec<(String, TextColor)>) {
    match &b.content {
        BoxContent::Inline(items) => {
            for item in items {
                match item {
                    InlineItem::Text(t, s) => out.push((t.clone(), s.color)),
                    InlineItem::Embedded(e) => q19_texts(e, out),
                    _ => {}
                }
            }
        }
        BoxContent::Blocks(children) => {
            for c in children {
                q19_texts(c, out);
            }
        }
        _ => {}
    }
}

#[test]
fn q19_before_after_generate_string_content() {
    let doc = parse_html(
        r#"<html><head><style>
        q::before { content: "<<"; color: #ff0000; }
        q::after { content: ">>" "!"; }
        q { color: #0000ff; }
        p::before { content: none; }
        p:nth-child(2)::after { content: "FIRST"; }
        .b::before { content: ""; display: block; width: 33px; height: 7px; }
        </style></head><body><q>mid</q><p>a</p><p>b</p><div class="b">x</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    let pos = |t: &str| texts.iter().position(|(x, _)| x == t).unwrap_or_else(|| panic!("{t} missing: {texts:?}"));
    let (b, m, a) = (pos("<<"), pos("mid "), pos(">>!"));
    assert!(b < m && m < a, "order before, content, after: {texts:?}");
    assert_eq!(texts[b].1, TextColor::rgb(0xff, 0, 0), "pseudo's own color");
    assert_eq!(texts[a].1, TextColor::rgb(0, 0, 0xff), "inherits from the element");
    assert!(texts.iter().any(|(t, _)| t == "FIRST"), "{texts:?}");
    assert_eq!(texts.iter().filter(|(t, _)| t == "FIRST").count(), 1, "only the first p");
    let laid = measure_document_fixture(
        r#"<html><head><style>.b::before { content: ''; display: block; width: 33px; height: 7px; }</style></head><body><div class="b">x</div></body></html>"#,
        800.0,
        600.0,
    );
    assert_eq!(find_by_width(&laid, 33.0).expect("generated block").h, 7.0);
    assert_eq!(parse_declarations("content: url(x.png)").content, Some(ContentDecl::Image("x.png".into())));
    assert_eq!(parse_declarations("content: url(x.png) 'a'").content, None);
    assert_eq!(parse_declarations("content: normal").content, Some(ContentDecl::None));
    assert_eq!(parse_declarations("content: 'a' \"b\"").content, Some(ContentDecl::Text("ab".into())));
}

#[test]
fn q19d_content_url_renders_an_image() {
    fn count_images(b: &LayoutBox) -> usize {
        match &b.content {
            BoxContent::Image(_) => 1,
            BoxContent::Inline(items) => items
                .iter()
                .map(|i| match i {
                    InlineItem::Image { .. } => 1,
                    InlineItem::Embedded(e) => count_images(e),
                    _ => 0,
                })
                .sum(),
            BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().map(count_images).sum(),
            _ => 0,
        }
    }
    let uri = tiny_png_data_uri(7, 5);
    let html = format!(
        r#"<html><head><style>
        p::before {{ content: url({uri}); }}
        div::after {{ content: url("{uri}"); display: block; width: 40px; height: 20px; }}
        i::before {{ content: url(not-fetched.png); }}
        </style></head><body><p>a</p><div>b</div><i>c</i></body></html>"#
    );
    let tree = build_layout_tree(&parse_html(&html), 800.0);
    assert_eq!(count_images(&tree), 2, "p::before inline + div::after block; unfetched url is skipped");
    let laid = measure_document_fixture(&html, 800.0, 600.0);
    assert_eq!(find_by_width(&laid, 40.0).expect("block image").h, 20.0);
    let mut cache = HashMap::new();
    cache.insert(
        "not-fetched.png".to_string(),
        Arc::new(DecodedImage { width: 3, height: 3, pixels: vec![TextColor::rgb(9, 9, 9); 9], svg: None, mask: None }),
    );
    let tree = build_layout_tree_with_resources(&parse_html(&html), 800.0, None, "", &cache);
    assert_eq!(count_images(&tree), 3, "a fetched (cached) url now renders too");
}

#[test]
fn q19e_first_letter_styles_the_leading_letter() {
    let doc = parse_html(
        r#"<html><head><style>
        p::first-letter { color: #ff0000; font-size: 40px; }
        </style></head><body><p>"Hello world</p><div>plain</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    let red = TextColor::rgb(0xff, 0, 0);
    assert!(texts.iter().any(|(t, c)| t == "\"H" && *c == red), "{texts:?}");
    assert!(texts.iter().any(|(t, c)| t == "ello world " && *c != red), "{texts:?}");
    assert!(texts.iter().any(|(t, c)| t == "plain " && *c != red), "{texts:?}");
}

#[test]
fn q19f_counter_polish_quotes_and_element_content() {
    assert_eq!(parse_declarations("quotes: none").quotes, Some(Vec::new()));
    assert_eq!(
        parse_declarations(r#"quotes: "<" ">" "[" "]""#).quotes,
        Some(vec![("<".into(), ">".into()), ("[".into(), "]".into())])
    );
    assert_eq!(parse_declarations("counter-reset: inherit").counter_reset, None);
    assert_eq!(parse_declarations("counter-set: a 5").counter_set, Some(vec![("a".into(), 5)]));
    let doc = parse_html(
        r#"<html><head><style>
        li::before { content: counter(list-item) ". "; }
        .skip { display: none; counter-increment: n; }
        .skip::before { content: "HIDDEN"; }
        body { counter-reset: n; }
        p::before { content: counter(n) "|"; }
        .set { counter-set: n 7; }
        q { quotes: "<" ">" "[" "]"; }
        q::before { content: open-quote; }
        .rep { content: "REPLACED"; }
        </style></head><body>
        <ol><li>a</li><li>b</li></ol>
        <div class="skip">x</div><p>one</p><p class="set">two</p>
        <p><q>a<q>b</q></q></p>
        <span class="rep">orig</span></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    for want in ["1. ", "2. ", "0|", "7|", "REPLACED", "<", "["] {
        assert!(texts.iter().any(|(t, _)| t == want), "{want:?} missing: {texts:?}");
    }
    assert!(!texts.iter().any(|(t, _)| t.contains("HIDDEN") || t.starts_with("orig")), "{texts:?}");
}

#[test]
fn q19e2_first_line_styles_only_the_first_laid_out_line() {
    let doc = parse_html(
        r#"<html><head><style>
        p { width: 120px; margin: 0; }
        p::first-line { color: #ff0000; }
        </style></head><body><p>aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, 400.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let red = TextColor::rgb(0xff, 0, 0);
    let mut lines: Vec<bool> = Vec::new();
    for entry in cache.values() {
        let guard = entry.lock().unwrap();
        let runs: Vec<_> = guard.0.layout_runs().collect();
        if runs.len() < 2 {
            continue;
        }
        for run in runs {
            lines.push(run.glyphs.iter().all(|g| g.color_opt == Some(red)));
        }
    }
    assert!(lines.len() >= 2, "paragraph should wrap: {lines:?}");
    assert!(lines[0], "first line red: {lines:?}");
    assert!(lines[1..].iter().all(|r| !r), "later lines not red: {lines:?}");
}

#[test]
fn q21_overflow_axes_and_text_overflow_ellipsis() {
    assert!(parse_declarations("overflow-x: hidden").overflow == Some(Overflow::Hidden));
    assert!(parse_declarations("overflow-x: hidden").overflow_y.is_none());
    let d = parse_declarations("overflow: hidden scroll");
    assert!(d.overflow == Some(Overflow::Hidden) && d.overflow_y == Some(Overflow::Scroll));
    let d = parse_declarations("overflow: auto");
    assert!(d.overflow == Some(Overflow::Auto) && d.overflow_y == Some(Overflow::Auto));
    assert!(parse_declarations("text-overflow: ellipsis").text_overflow == Some(TextOverflow::Ellipsis));
    let ink_width = |css: &str| -> (f32, bool) {
        let doc = parse_html(&format!(
            r#"<html><head><style>div {{ width: 100px; margin: 0; {css} }}</style></head><body><div>aaaa bbbb cccc dddd eeee ffff gggg</div></body></html>"#
        ));
        let tree = build_layout_tree(&doc, 1920.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure_document(&tree, &mut font_system, 400.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let mut best = (0.0f32, false);
        for entry in cache.values() {
            let g = entry.lock().unwrap();
            for run in g.0.layout_runs() {
                if run.glyphs.iter().any(|gl| gl.end > 0) && run.line_w > best.0 {
                    let text = run.text;
                    best = (run.line_w, text.contains('\u{2026}'));
                }
            }
        }
        best
    };
    let (plain_w, _) = ink_width("white-space: nowrap; overflow: hidden;");
    assert!(plain_w > 150.0, "unclipped nowrap line is wide: {plain_w}");
    let (w, has) = ink_width("white-space: nowrap; overflow-x: hidden; text-overflow: ellipsis;");
    assert!(w <= 100.5 && has, "ellipsized line fits the box and ends in an ellipsis: {w}");
    let (w, _) = ink_width("white-space: nowrap; overflow: visible; text-overflow: ellipsis;");
    assert!(w > 150.0, "no ellipsis without clipping overflow: {w}");
}

#[test]
fn q19c_marker_pseudo_styles_and_replaces_the_marker() {
    let doc = parse_html(
        r#"<html><head><style>
        li::marker { color: #00ff00; }
        li.c::marker { content: "→ "; color: #ff0000; }
        li.n::marker { content: none; }
        </style></head><body><ul><li>a</li><li class="c">b</li><li class="n">c</li></ul></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    let find = |t: &str| texts.iter().find(|(x, _)| x == t).map(|(_, c)| *c);
    assert_eq!(find("\u{2022} "), Some(TextColor::rgb(0, 0xff, 0)), "{texts:?}");
    assert_eq!(find("→ "), Some(TextColor::rgb(0xff, 0, 0)), "{texts:?}");
    assert!(texts.iter().filter(|(t, _)| t.starts_with('\u{2022}')).count() == 1, "none removes it: {texts:?}");
    assert!(find("c ").is_some());
}

#[test]
fn q20a_font_shorthand() {
    let d = parse_declarations("font: italic bold 20px/1.5 Georgia, serif");
    assert!(d.font_style == Some(FontStyle::Italic));
    assert!(d.font_weight == Some(FontWeightValue::Absolute(FontWeight::BOLD)));
    assert!(d.font_size == Some(Length::Px(20.0)));
    assert!(d.line_height == Some(LineHeight::Multiplier(1.5)));
    assert!(d.font_family.is_some());
    let d = parse_declarations("font: 12px sans-serif");
    assert!(d.font_style == Some(FontStyle::Normal));
    assert!(d.font_weight == Some(FontWeightValue::Absolute(FontWeight::NORMAL)));
    assert!(d.line_height == Some(LineHeight::Normal));
    let d = parse_declarations("font: 600 small/20px \"Foo Bar\", serif");
    assert!(d.font_weight == Some(FontWeightValue::Absolute(FontWeight(600))));
    assert!(d.font_size == Some(Length::Px(13.0)));
    assert!(d.line_height == Some(LineHeight::Len(Length::Px(20.0))));
    assert!(parse_declarations("font: menu").font_size.is_none());
    assert!(parse_declarations("font: bold").font_size.is_none());
}

#[test]
fn q20b_prefixed_aliases_are_plain_synonyms() {
    let d = parse_declarations("-webkit-box-sizing: border-box; -moz-box-sizing: content-box");
    assert!(d.box_sizing == Some(BoxSizing::ContentBox));
    let d = parse_declarations("-webkit-box-sizing: border-box");
    assert!(d.box_sizing == Some(BoxSizing::BorderBox));
    let d = parse_declarations("display: -webkit-flex; -webkit-flex: 2 1 10px; -webkit-box-shadow: 1px 1px 2px red");
    assert!(d.display == Some(Display::Flex));
    assert!(d.flex_grow.is_some() && d.box_shadow.is_some());
    assert!(parse_declarations("display: -webkit-inline-flex").display == Some(Display::InlineFlex));
    assert!(parse_declarations("display: -webkit-box").display.is_none());
}

#[test]
fn q20c_alpha_roman_greek_markers_and_display_list_item() {
    assert!(parse_declarations("list-style-type: lower-alpha").list_style_type == Some(ListStyleType::LowerAlpha));
    assert!(parse_declarations("list-style: upper-roman inside").list_style_type == Some(ListStyleType::UpperRoman));
    let doc = parse_html(
        r#"<html><head><style>
        .a { list-style-type: lower-alpha; } .g { list-style-type: lower-greek; }
        .r { list-style-type: upper-roman; }
        div.i { display: list-item; list-style-type: decimal; }
        div.i::marker { color: #ff0000; }
        </style></head><body>
        <ol class="a"><li>x</li><li>y</li></ol><ol class="g"><li>x</li><li>y</li></ol>
        <ol class="r"><li>x</li><li>y</li><li>z</li><li>w</li></ol>
        <div class="i">p</div><div>q</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    let has = |t: &str| texts.iter().any(|(x, _)| x == t);
    assert!(has("a. ") && has("b. "), "{texts:?}");
    assert!(has("\u{3b1}. ") && has("\u{3b2}. "), "{texts:?}");
    assert!(has("IV. "), "{texts:?}");
    let m = texts.iter().find(|(x, _)| x == "1. ").map(|(_, c)| *c);
    assert_eq!(m, Some(TextColor::rgb(0xff, 0, 0)), "list-item div gets a styled marker: {texts:?}");
    assert!(texts.iter().filter(|(x, _)| x.ends_with(". ")).count() == 9, "plain div gets none: {texts:?}");
}

#[test]
fn q20def_word_break_text_transform_and_white_space_variants() {
    assert!(parse_declarations("word-break: break-all").word_break == Some(WordBreak::BreakAll));
    assert!(parse_declarations("overflow-wrap: anywhere").word_break == Some(WordBreak::BreakWord));
    assert!(parse_declarations("word-wrap: break-word").word_break == Some(WordBreak::BreakWord));
    assert!(parse_declarations("text-transform: capitalize").text_transform == Some(TextTransform::Capitalize));
    assert!(parse_declarations("white-space: pre-wrap").white_space == Some(WhiteSpace::PreWrap));
    assert!(parse_declarations("white-space: pre-line").white_space == Some(WhiteSpace::PreLine));
    assert_eq!(inline_text("hello wORLD it's", WhiteSpace::Normal, TextTransform::Capitalize), "Hello WORLD It's ");
    assert_eq!(inline_text("a  b", WhiteSpace::Normal, TextTransform::Uppercase), "A B ");
    assert_eq!(inline_text("A  b\n  c ", WhiteSpace::PreLine, TextTransform::Lowercase), "a b\nc");
    assert_eq!(inline_text("a  b\n", WhiteSpace::PreWrap, TextTransform::None), "a  b\n");

    let doc = parse_html(
        r#"<html><head><style>
        .u { text-transform: uppercase; } .l { text-transform: lowercase; }
        </style></head><body><p class="u">shout <b>loud</b></p><p class="l">QUIET</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    assert!(texts.iter().any(|(t, _)| t == "SHOUT "), "{texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "LOUD "), "inherited into <b>: {texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "quiet "), "{texts:?}");

    // Shaping: pre-wrap wraps (several lines), nowrap does not; pre-line
    // honors a newline; break-all splits a single long word.
    let mut fs = new_font_system();
    let mut run = |ws: WhiteSpace, wb: WordBreak, text: &str| {
        let mut span = SpanStyle::from(ROOT_STYLE);
        span.word_break = wb;
        let items = [InlineItem::Text(inline_text(text, ws, TextTransform::None), span)];
        let pre = matches!(ws, WhiteSpace::Pre | WhiteSpace::Nowrap);
        let shaped = shape_spans(&mut fs, &items, 60.0, pre, None);
        shaped.0.layout_runs().count()
    };
    let long = "word word word word word word word word";
    assert!(run(WhiteSpace::PreWrap, WordBreak::Normal, long) > 1);
    assert_eq!(run(WhiteSpace::Pre, WordBreak::Normal, long), 1);
    assert_eq!(run(WhiteSpace::PreLine, WordBreak::Normal, "a\nb"), 2);
    assert!(run(WhiteSpace::Normal, WordBreak::BreakAll, "abcdefghijklmnopqrstuvwxyz") > 1);
}

#[test]
fn q20f_intrinsic_width_keywords_size_from_content() {
    assert!(parse_declarations("width: max-content").width == Some(Length::Intrinsic(IntrinsicSize::MaxContent)));
    assert!(parse_declarations("min-width: fit-content").min_width == Some(Length::Intrinsic(IntrinsicSize::FitContent)));
    assert!(parse_declarations("margin: min-content").margin_top.is_none());
    fn widths(l: &Laid, out: &mut Vec<f32>) {
        out.push(l.w);
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                widths(k, out);
            }
        }
    }
    let width_of = |style: &str| {
        let laid = measure_fixture(
            &format!(r#"<html><body><div id="t" style="{style}">alpha beta gamma delta</div></body></html>"#),
            600.0,
        );
        let mut ws = Vec::new();
        widths(&laid, &mut ws);
        // the test div's inline child is the last box in document order
        *ws.last().unwrap()
    };
    let max = width_of("width: max-content");
    let min = width_of("width: min-content");
    let fit = width_of("width: fit-content");
    let minw = width_of("min-width: max-content; width: 10px");
    assert!(min > 20.0 && min < max, "min {min} max {max}");
    assert!(max < 400.0, "max-content hugs the text: {max}");
    assert!((fit - max).abs() < 0.5, "fit-content with room equals max-content: {fit} vs {max}");
    assert!((minw - max).abs() < 0.5, "min-width: max-content overrides a small width: {minw} vs {max}");
}

#[test]
fn q19b_attr_counters_quotes_and_flex_pseudo_items() {
    assert_eq!(
        parse_declarations("content: attr(title) counter(n, lower-roman)").content,
        Some(ContentDecl::Parts(vec![
            ContentPart::Attr("title".into()),
            ContentPart::Counter { name: "n".into(), style: "lower-roman".into() },
        ]))
    );
    assert_eq!(format_counter(4, "lower-roman"), "iv");
    assert_eq!(format_counter(28, "upper-alpha"), "AB");
    assert_eq!(format_counter(3, "decimal-leading-zero"), "03");
    let doc = parse_html(
        r#"<html><head><style>
        body { counter-reset: sec; }
        h2 { counter-increment: sec; }
        h2::before { content: counter(sec) ". "; }
        ul { counter-reset: item; }
        li { counter-increment: item 2; }
        li::after { content: "[" counters(item, "-") "]"; }
        a::before { content: attr(href) open-quote; }
        a::after { content: close-quote; }
        </style></head><body><h2>A</h2><h2>B</h2><ul><li>x</li><li>y</li></ul>
        <a href="u">l</a></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    for want in ["1. ", "2. ", "[2]", "[4]", "u\u{201C}", "\u{201D}"] {
        assert!(texts.iter().any(|(t, _)| t == want), "{want:?} missing: {texts:?}");
    }
    // ::before / ::after are their own flex items.
    let laid = measure_document_fixture(
        r#"<html><head><style>
        .f { display: flex; width: 300px; }
        .f::before { content: ""; width: 40px; height: 9px; }
        .f::after { content: ""; width: 50px; height: 9px; }
        </style></head><body><div class="f"><span style="width:100px">m</span></div></body></html>"#,
        800.0,
        600.0,
    );
    let before = find_by_width(&laid, 40.0).expect("before item");
    let after = find_by_width(&laid, 50.0).expect("after item");
    assert!(after.x > before.x + 40.0, "after item sits to the right: {} vs {}", after.x, before.x);
}

// ---------- v45: CSS custom properties (`:root`/`html`-scoped) + `var()` ----------

#[test]
fn root_custom_prop_resolves_via_var() {
    let doc = parse_html(
        r#"<html><head><style>:root { --c: #ff0000; } p { color: var(--c); }</style></head><body><p>hi</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_eq!(color, TextColor::rgb(0xff, 0, 0), "var(--c) should resolve to the :root-declared value");
}

#[test]
fn html_selector_also_scopes_a_custom_prop() {
    let doc = parse_html(
        r#"<html><head><style>html { --c: #0000ff; } p { color: var(--c); }</style></head><body><p>hi</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_eq!(color, TextColor::rgb(0, 0, 0xff), "var(--c) should resolve via a plain `html` selector too");
}

#[test]
fn var_fallback_used_when_prop_undeclared() {
    let doc = parse_html(
        r#"<html><head><style>p { color: var(--undefined, #00ff00); }</style></head><body><p>hi</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_eq!(color, TextColor::rgb(0, 0xff, 0), "an undeclared custom property should fall back to the fallback value");
}

#[test]
fn nested_var_fallback_resolves() {
    let doc = parse_html(
        r#"<html><head><style>p { color: var(--a, var(--b, #112233)); }</style></head><body><p>hi</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let color = find_first_span_color(&tree).expect("no span found");
    assert_eq!(color, TextColor::rgb(0x11, 0x22, 0x33), "a nested var() inside a fallback should itself resolve");
}

fn first_span_color(html: &str) -> TextColor {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    find_first_span_color(&tree).expect("no span found")
}

/// Builds with a real 800x600 viewport and measures via
/// `measure_document` (the path the app uses).
fn measure_viewport_fixture(html: &str) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree_sized(&doc, 800.0, 600.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete)
}

#[test]
fn q14_viewport_units_and_icb_height() {
    // vh / vw / vmin / vmax, plain and inside calc().
    let laid = measure_viewport_fixture(
        r#"<html><body><div style="height:50vh;width:25vw"></div><div style="height:calc(10vh + 5px);width:10vmin"></div><div style="height:10px;width:10vmax"></div></body></html>"#,
    );
    assert_eq!(find_by_height(&laid, 300.0).expect("50vh").style.width.map(BoxLength::px), Some(200.0));
    let calc = find_by_height(&laid, 65.0).expect("calc(10vh + 5px)");
    assert_eq!(calc.style.width.map(BoxLength::px), Some(60.0));
    assert!(find_by_width(&laid, 80.0).is_some(), "10vmax of 800x600");
    // %-width inside calc with vw resolves against the carried viewport.
    let laid = measure_viewport_fixture(
        r#"<html><body style="margin:0"><div style="width:calc(50% + 10vw)"></div></body></html>"#,
    );
    assert!(find_by_width(&laid, 480.0).is_some());

    // html,body{height:100%} + child 50%.
    let laid = measure_viewport_fixture(
        r#"<html><head><style>html,body{height:100%;margin:0}</style></head><body><div style="height:50%"></div></body></html>"#,
    );
    assert!(find_by_height(&laid, 600.0).is_some(), "html/body fill the window");
    assert!(find_by_height(&laid, 300.0).is_some(), "child 50% of the body");
    assert_eq!(laid.h, 600.0);

    // Content taller than the window still scrolls: root h is the
    // content extent, not pinned to the viewport.
    let laid = measure_viewport_fixture(
        r#"<html><head><style>html,body{height:100%;margin:0}</style></head><body><div style="height:1500px"></div></body></html>"#,
    );
    assert!(laid.h >= 1500.0, "scroll height {} must cover overflowing content", laid.h);
}

#[test]
fn custom_prop_on_ancestor_inherits_to_subtree() {
    let c = first_span_color(
        r#"<html><head><style>.dark{--bg:#000001} .dark p{color:var(--bg)}</style></head><body><div class="dark"><section><p>hi</p></section></div></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(0, 0, 1));
}

#[test]
fn custom_prop_not_visible_outside_its_subtree() {
    let c = first_span_color(
        r#"<html><head><style>.scope { --c: #ff0000; } p { color: var(--c, #00ff00); }</style></head><body><div class="scope"></div><p>hi</p></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(0, 0xff, 0));
}

#[test]
fn descendant_overrides_custom_prop() {
    let c = first_span_color(
        r#"<html><head><style>:root{--c:#010101} .x{--c:#020202} p{color:var(--c)}</style></head><body><div class="x"><p>hi</p></div></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(2, 2, 2));
}

#[test]
fn chained_custom_props_resolve() {
    let c = first_span_color(
        r#"<html><head><style>:root{--b:var(--a);--a:#030303} p{color:var(--b)}</style></head><body><p>hi</p></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(3, 3, 3));
}

#[test]
fn custom_prop_cycle_uses_fallback() {
    let c = first_span_color(
        r#"<html><head><style>:root{--a:var(--b);--b:var(--a)} p{color:var(--a,#040404)}</style></head><body><p>hi</p></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(4, 4, 4));
}

#[test]
fn var_inside_quoted_string_is_not_substituted() {
    let vars = |n: &str| if n == "--x" { Some("BAD".to_string()) } else { None };
    assert_eq!(substitute_css_vars("\"var(--x)\" var(--x)", &vars).unwrap(), "\"var(--x)\" BAD");
}

#[test]
fn var_inside_calc_resolves_per_element() {
    let doc = parse_html(
        r#"<html><head><style>.a{--w:30px} div{width:calc(var(--w) + 10px)}</style></head><body><div class="a" id="d">x</div></body></html>"#,
    );
    let rules = parse_stylesheet(".a{--w:30px} div{width:calc(var(--w) + 10px)}");
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    assert!(st.width.is_some());
}

#[test]
fn background_image_var_resolves() {
    let uri = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'%3E%3Crect width='4' height='4' fill='red'/%3E%3C/svg%3E";
    let css = format!(":root{{--bg:url(\"{uri}\")}} p{{background-image:var(--bg)}}");
    let doc = parse_html("<html><body><p>x</p></body></html>");
    let rules = parse_stylesheet(&css);
    let p = find_element_by_tag(&doc, "p").expect("p");
    let html = find_html_element(&doc).unwrap();
    let hs = resolve_style(&html, &rules, ROOT_STYLE, 800.0);
    assert!(resolve_background_image(&p, &rules, 800.0, hs.custom_props).is_some());
}

#[test]
fn inline_style_custom_prop_and_var() {
    let c = first_span_color(
        r#"<html><body><div style="--c:#050505"><p style="color:var(--c)">hi</p></div></body></html>"#,
    );
    assert_eq!(c, TextColor::rgb(5, 5, 5));
}

// ---------- v46: `<img>` rendering ----------

/// Builds a solid-color PNG in memory and returns it as a `data:` URI
/// — avoids hardcoding a base64 blob in test source; exercises the
/// real `image`-crate decode path the same way a real `<img src>`
/// would, just generated instead of fetched.
fn tiny_png_data_uri(width: u32, height: u32) -> String {
    use base64::Engine;
    let img = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |_, _| {
        image::Rgba([255u8, 0, 0, 255])
    }));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))
}

fn find_image_laid(laid: &Laid) -> Option<&Laid> {
    if matches!(laid.content, LaidContent::Image(_)) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_image_laid(c) {
                return Some(found);
            }
        }
    }
    // A default-`display: inline` `<img>` (the common case since
    // v48) lands inside a `MixedInline` run instead of its own
    // top-level `LaidContent::Image` — see `MixedPositioned`'s own
    // docs.
    if let LaidContent::MixedInline(items) = &laid.content {
        for item in items {
            // `Embedded` can now also hold a `display: inline-block`/
            // `inline-flex` element's own subtree, not just an
            // inline `<img>` — check this entry's own `content` so a
            // run mixing both kinds doesn't return whichever comes
            // first regardless of which one a test actually wants.
            if let MixedPositioned::Embedded(embedded_laid) = item
                && matches!(embedded_laid.content, LaidContent::Image(_))
            {
                return Some(embedded_laid);
            }
        }
    }
    None
}

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
    assert_eq!(items.len(), 3, "text before/the image/text after should be 3 items in the SAME inline run");
    let InlineItem::Text(before, _) = &items[0] else { panic!("expected text before the image") };
    assert!(before.trim_start().starts_with("before"), "unexpected text before the image: {before:?}");
    assert!(matches!(items[1], InlineItem::Image { .. }));
    let InlineItem::Text(after, _) = &items[2] else { panic!("expected text after the image") };
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

// ---------- v48: real inline-level `<img>` flow ----------

fn find_mixed_inline(laid: &Laid) -> Option<&Laid> {
    if matches!(laid.content, LaidContent::MixedInline(_)) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_mixed_inline(c) {
                return Some(found);
            }
        }
    }
    None
}

fn mixed_items(laid: &Laid) -> &Vec<MixedPositioned> {
    let LaidContent::MixedInline(items) = &laid.content else { panic!("not a MixedInline Laid") };
    items
}

fn mixed_item_xy(item: &MixedPositioned) -> (f32, f32) {
    match item {
        MixedPositioned::Embedded(laid) => (laid.x, laid.y),
        MixedPositioned::Word { x, y, .. } => (*x, *y),
    }
}

#[test]
fn mixed_inline_keeps_items_on_one_line_when_they_fit() {
    let src = tiny_png_data_uri(4, 4);
    let html = format!(r#"<html><body><p>x <img src="{src}" style="width: 10px; height: 10px"> y</p></body></html>"#);
    let laid = measure_fixture(&html, 500.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let items = mixed_items(run);
    assert_eq!(items.len(), 3, "word, image, word");
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
    assert!(positions[1].0 < positions[2].0, "trailing word should sit after the image");
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
    assert_eq!(word_ids.len(), 2, "expected the two words (\"x \", \"y\") to each get their own id");

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

// ---------- v49: `%` lengths, and confirming unitless-non-zero stays rejected ----------

#[test]
fn percent_width_resolves_against_containing_block_width() {
    // `body { margin: 8px }` (default.css) eats 16px total, so a
    // 416px window gives body a 400px content width — the div's own
    // containing block.
    let laid = measure_fixture(r#"<html><body><div style="width: 50%;">x</div></body></html>"#, 416.0);
    let div = find_by_width(&laid, 200.0).expect("div's width: 50% should resolve to 200px");
    assert_eq!(div.w, 200.0);
}

#[test]
fn percent_margin_resolves_against_containing_block_width_on_every_side() {
    // Real CSS resolves a margin percentage against the containing
    // block's *width* on every side, even top/bottom — not against
    // the margined box's own (much narrower, unrelated) width. The
    // outer div pins the containing block's content width at
    // exactly 1000px so there's an unambiguous basis to check
    // against.
    let laid = measure_fixture(
        r#"<html><body><div style="width: 1000px;"><div style="width: 20px; margin-top: 10%; margin-left: 10%;">inner</div></div></body></html>"#,
        2000.0,
    );
    let inner = find_by_width(&laid, 20.0).expect("inner div not found");
    assert_eq!(
        inner.style.margin.top.px(),
        100.0,
        "margin-top: 10% should resolve against the containing block's 1000px width, not inner's own 20px"
    );
    assert_eq!(inner.style.margin.left.px(), 100.0);
}

#[test]
fn percent_padding_top_resolves_against_containing_block_width_not_height() {
    // Same point as the margin test above, isolated for `padding`
    // specifically (a separate `Declarations`/cascade path) and
    // against a box whose own *height* could plausibly be
    // (wrongly) used as the basis instead, if padding-top's real
    // CSS basis (width, same as every other side) weren't followed.
    let laid = measure_fixture(
        r#"<html><body><div style="width: 1000px;"><div style="height: 10px; padding-top: 10%;">inner</div></div></body></html>"#,
        2000.0,
    );
    let inner = find_by_height(&laid, 10.0).expect("inner div not found");
    assert_eq!(
        inner.style.padding.top.px(),
        100.0,
        "padding-top: 10% must resolve against the containing block's 1000px width, not inner's own 10px height"
    );
}

#[test]
fn percent_height_resolves_against_definite_containing_block_height() {
    let laid = measure_fixture(
        r#"<html><body><div style="height: 300px;"><div style="height: 50%;">inner</div></div></body></html>"#,
        500.0,
    );
    let inner = find_by_height(&laid, 150.0)
        .expect("inner div's height: 50% should resolve to 150px against its 300px containing block");
    assert_eq!(inner.h, 150.0);
}

#[test]
fn percent_height_computes_to_auto_when_containing_block_height_is_indefinite() {
    // Neither `html` nor `body` declares an explicit height in
    // `default.css`, so the div's containing block height is
    // indefinite (content-driven) here — real CSS Level 2's
    // documented rule for exactly this case: the percentage
    // computes to `auto`, i.e. exactly as if `height` weren't
    // declared at all, not some value resolved against a bogus
    // implicit basis. Checked by comparing against the identical
    // markup with the declaration removed entirely — if `%` were
    // wrongly resolving against something (e.g. `0`, or this box's
    // own width), the two reported heights would differ.
    let with_percent = measure_fixture(
        r#"<html><body><div style="height: 50%; padding: 5px;">hello</div></body></html>"#,
        500.0,
    );
    let without = measure_fixture(r#"<html><body><div style="padding: 5px;">hello</div></body></html>"#, 500.0);
    assert_eq!(
        with_percent.h, without.h,
        "height: 50% against an indefinite containing block should have no effect at all, same as auto"
    );
}

#[test]
fn unitless_nonzero_number_is_still_rejected_for_margin() {
    // This TODO item's other half: confirming a plain `<length>`
    // property (unlike `line-height`, which legitimately accepts a
    // bare number) still only accepts unitless *zero*, not a
    // general unitless number — `margin-top: 5` (no unit) must drop
    // the whole declaration, falling back to the real initial value
    // `0`, not be (wrongly) treated as `5px`.
    let laid =
        measure_fixture(r#"<html><body><div style="width: 123px; margin-top: 5;">x</div></body></html>"#, 500.0);
    let div = find_by_width(&laid, 123.0).expect("div not found");
    assert_eq!(div.style.margin.top.px(), 0.0, "a unitless non-zero margin-top must be dropped, not treated as 5px");
}

#[test]
fn line_height_percent_resolves_against_own_font_size() {
    // `line-height`'s `%` form resolves against the *declaring*
    // element's own font-size (same timing as its `em` form, and
    // unlike its bare-`<number>` form, which stays symbolic and
    // re-resolves per descendant — see `LineHeight`'s own docs).
    let laid = measure_fixture(
        r#"<html><body><div style="font-size: 20px; line-height: 150%;">x</div></body></html>"#,
        500.0,
    );
    let div = find_by_laid_font_size(&laid, 20.0).expect("div with font-size: 20px not found");
    match div.style.line_height {
        LineHeight::Len(l) => assert_eq!(l.resolve(div.style.font_size, div.style.root_font_size), 30.0),
        _ => panic!("line-height: 150% should parse as LineHeight::Len(Length::Percent), not Normal/Multiplier"),
    }
}

fn find_by_laid_font_size(laid: &Laid, target: f32) -> Option<&Laid> {
    if laid.style.font_size == target {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_by_laid_font_size(c, target) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn border_radius_percent_resolves_against_own_border_box_width() {
    // `border-radius`'s `%` basis is this box's own border-box
    // width (unlike `width`/`margin`/`padding`, whose basis is the
    // *containing block's* width) — resolved directly in
    // `paint_box_decoration` against its own `w`, not in `measure`.
    let laid = measure_fixture(
        r#"<html><body><div style="width: 200px; height: 50px; border-radius: 25%;">x</div></body></html>"#,
        500.0,
    );
    let div = find_by_width(&laid, 200.0).expect("div not found");
    assert_eq!(div.style.border_radius.resolve(div.w), 50.0, "border-radius: 25% of a 200px-wide box should resolve to 50px");
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

// ---------- table layout: real CSS auto table layout (min/max content width per column) ----------

/// The first `LaidContent::TableRow`'s own cells, found by walking
/// through any number of wrapping `Blocks` boxes (a `<table>`'s own
/// content, or `<body>`/`<html>` above it) — same recursive-search
/// shape as this file's other `find_*` test helpers.
fn find_table_row_cells(laid: &Laid) -> Option<&[Laid]> {
    if let LaidContent::TableRow(cells) = &laid.content {
        return Some(cells);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_table_row_cells(c) {
                return Some(found);
            }
        }
    }
    None
}

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
        r#"<table><tr><td>a</td><td>a much much longer second column of text here</td></tr></table>"#,
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

// ---------- v51: named `font-family` fallback lists ----------

fn find_first_span_font_family(b: &LayoutBox) -> Option<FontFamily> {
    if let BoxContent::Inline(items) = &b.content {
        if let Some(InlineItem::Text(_, s)) = items.first() {
            return Some(s.font_family);
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_first_span_font_family(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn font_family_parses_quoted_multiword_and_generic_fallback_list() {
    // A quoted name (`'Georgia'`), an unquoted multi-word name
    // (`Helvetica Neue` — real CSS grammar joins bare idents with
    // spaces into one family name), and a trailing bare generic
    // keyword, all in one comma-separated list.
    let doc = parse_html(
        r#"<html><body><p style="font-family: 'Georgia', Helvetica Neue, monospace">x</p></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let family = find_first_span_font_family(&tree).expect("no span found");
    assert_eq!(
        family.0,
        [
            FontFamilyItem::Name("Georgia"),
            FontFamilyItem::Name("Helvetica Neue"),
            FontFamilyItem::Monospace,
        ]
        .as_slice(),
        "expected the quoted name, the joined multi-word name, and the trailing generic, in order"
    );
}

#[test]
fn font_family_single_generic_keyword_still_works() {
    // v16's original scope (a single bare generic, no list at all)
    // must still parse to exactly one item — no regression from
    // generalizing the grammar to a full list.
    let doc = parse_html(r#"<html><body><code>x</code></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let family = find_first_span_font_family(&tree).expect("no span found");
    assert_eq!(family.0, [FontFamilyItem::Monospace].as_slice());
}

#[test]
fn font_family_inherits_through_the_cascade() {
    // Real CSS inheritance (same family as color/font-size/etc.,
    // per PROMPTS.md v16) — a descendant with no `font-family` rule
    // of its own takes its ancestor's resolved fallback list, not
    // the default.
    let doc = parse_html(
        r#"<html><body><div style="font-family: 'Georgia', serif"><span>x</span></div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let family = find_first_span_font_family(&tree).expect("no span found");
    assert_eq!(family.0, [FontFamilyItem::Name("Georgia"), FontFamilyItem::Serif].as_slice());
}

fn collect_text_shape_ids(laid: &Laid, ids: &mut Vec<usize>) {
    match &laid.content {
        LaidContent::Text(id) => ids.push(*id),
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children {
                collect_text_shape_ids(c, ids);
            }
        }
        LaidContent::Image(_) | LaidContent::MixedInline(_) => {}
    }
}

#[test]
fn font_family_fallback_resolves_through_the_real_font_system() {
    // Verified past the cascade, through to what actually gets
    // shaped — same diligence v16's own (since-removed, pre-v41)
    // test used: a field can cascade to the right enum value and
    // still not affect a single rendered glyph if the shaping path
    // never reads it correctly. This runs the full pipeline
    // (`build_layout_tree` + `measure`) on three paragraphs —
    // `monospace`, `sans-serif`, and a fallback list naming a family
    // that can't possibly be installed before a real `monospace`
    // keyword — and inspects the actually-shaped `cosmic_text::
    // Buffer`'s own `fontdb::ID` per paragraph (not scratchy's own
    // `FontFamily` value, which would only prove the enum carries
    // the right data, not that it reached a real font).
    // Deliberately no whitespace between the `<p>`s — each gets its
    // own anonymous inline box/shaped paragraph, but a newline
    // between them would be its own (4th, 5th, ...) whitespace-only
    // inline run with its own shape id too, breaking the assumption
    // that `collect_text_shape_ids` finds exactly one id per `<p>`.
    let html = r#"<html><body><p style="font-family: monospace">A</p><p style="font-family: sans-serif">A</p><p style="font-family: 'NoSuchFontXYZ123', monospace">A</p></body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    );

    let mut ids = Vec::new();
    collect_text_shape_ids(&laid, &mut ids);
    assert_eq!(ids.len(), 3, "expected one shaped paragraph per <p>");

    let font_id_of = |id: usize| -> fontdb::ID {
        let entry = cache.get(&id).expect("missing shaped paragraph");
        let guard = entry.lock().unwrap();
        guard
            .0
            .layout_runs()
            .next()
            .and_then(|r| r.glyphs.first())
            .map(|g| g.font_id)
            .expect("no glyph was shaped")
    };

    let mono_id = font_id_of(ids[0]);
    let sans_id = font_id_of(ids[1]);
    let fallback_id = font_id_of(ids[2]);

    assert_ne!(mono_id, sans_id, "monospace and sans-serif should resolve to different fonts on this system");
    assert_eq!(
        fallback_id, mono_id,
        "an unavailable named family should fall through to the next entry in the list (monospace), not silently stay on cosmic-text's default sans-serif"
    );
}

// ---------- v52: positioning model (`position`/`z-index`) ----------

/// Like `measure_fixture`, but through `measure_document` — the real
/// top-level entry point, needed for anything that exercises `fixed`
/// or an orphaned `absolute` (both only ever resolved at the document
/// root; see `measure_document`'s own docs). Plain `measure_fixture`
/// calls `measure` directly, which would silently *drop* either case
/// (whatever bubbles out of its own disposable `pending` `Vec` is
/// simply never resolved) — the right behavior for `measure` itself
/// (it has no viewport to resolve them against), wrong for a test
/// that wants to see the fully-resolved tree.
fn measure_document_fixture(html: &str, width: f32, viewport_h: f32) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, width, viewport_h, &mut next_id, &mut cache, deadline, &mut incomplete)
}

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

// ---------- v53: `<head>`/`<title>`/`<meta>`/`<link>`/`<base>` are hidden by default ----------

fn collect_all_text(b: &LayoutBox, out: &mut String) {
    match &b.content {
        BoxContent::Blocks(children) | BoxContent::TableRow(children) => {
            for c in children {
                collect_all_text(c, out);
            }
        }
        BoxContent::Inline(items) => {
            for item in items {
                if let InlineItem::Text(t, _) = item {
                    out.push_str(t);
                }
            }
        }
        BoxContent::Image(_) => {}
    }
}

#[test]
fn title_text_does_not_render() {
    let doc = parse_html(
        r#"<html><head><title>Secret Page Title</title><meta charset="utf-8"><link rel="stylesheet" href="x.css"></head><body>Visible Body Text</body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut text = String::new();
    collect_all_text(&tree, &mut text);
    assert!(!text.contains("Secret Page Title"), "title text must not render in the document body: {text:?}");
    assert!(text.contains("Visible Body Text"), "body text should still render normally: {text:?}");
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

// ---------- v56: real CSS2.1 Appendix E stacking-context semantics ----------

/// Shared plumbing for this section's tests: parse, build, measure at
/// a 200x200 viewport (`measure_document`, so `position: absolute`/
/// `fixed` resolve the same way they would in the real app), paint
/// into a real pixel buffer, and hand back just the buffer plus the
/// window size needed to index it. `<body style="margin:0">` in every
/// fixture below sidesteps `default.css`'s own `body { margin: 8px }`
/// — these tests sample hand-computed pixel coordinates against
/// in-flow boxes (unlike the pre-existing `z_index_controls_paint_
/// order_for_overlapping_absolute_boxes` test above, whose `position:
/// absolute; top/left: 0` boxes anchor to the *viewport*, immune to
/// body's own margin either way).
fn paint_fixture(html: &str) -> (Vec<u32>, u32, u32) {
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
    (pixels, win_w, win_h)
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
    let summarize = |h: &Handle, pseudo: Option<&str>| -> Vec<u32> {
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

fn px_at(pixels: &[u32], win_w: u32, x: u32, y: u32) -> u32 {
    pixels[(y * win_w + x) as usize] & 0x00FF_FFFF
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
    let html = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div></div></body></html>"#;
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
    let html = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div><div style="position: absolute; top: 0px; left: 0px; width: 30px; height: 30px; background-color: #ffff00; z-index: 5;"></div></div></body></html>"#;
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
    assert_eq!(
        px_at(&pixels, win_w, 10, 60),
        0x0000_00FF,
        "z-index:auto should not establish a stacking context — the later sibling keeps winning the overlap by plain document/paint order"
    );
}

// ---------- v57: flexbox (`display: flex`) ----------

/// The flex container's own direct `LaidContent::Blocks` children,
/// in document order — exactly the flex items `measure_flex`
/// positioned, found by locating the container itself (via its own
/// distinctive `width`, same `find_by_width` helper every other
/// section already uses) rather than a bespoke lookup.
fn flex_items(laid: &Laid, container_w: f32) -> Vec<&Laid> {
    let container = find_by_width(laid, container_w).expect("flex container not found");
    let LaidContent::Blocks(children) = &container.content else {
        panic!("expected the flex container's content to be Blocks (its own flex items)");
    };
    children.iter().collect()
}

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

// ---------- v59: `word-spacing` ----------

/// Every distinct `y` among a `MixedInline` run's own positioned
/// items, rounded to avoid float-equality flakiness — the number of
/// *lines* a `measure_mixed_inline` call actually produced, used by
/// this section's wrap-regression tests the same way `line_height`-
/// based line counting would, but without depending on real font
/// metrics for line height itself.
fn mixed_inline_line_count(laid: &Laid) -> usize {
    let items = mixed_items(laid);
    let ys: std::collections::BTreeSet<i64> =
        items.iter().map(|it| (mixed_item_xy(it).1 * 100.0).round() as i64).collect();
    ys.len()
}

#[test]
fn word_spacing_normal_keeps_the_fast_cosmic_text_path() {
    // The overwhelming common case (`word-spacing` never declared,
    // or explicitly `normal`) must stay on the existing single-
    // `Buffer`-per-paragraph path — zero regression risk/cost for
    // every page that doesn't use this property, same guarantee
    // v48/v54 already give for a run with no image and no active
    // float.
    let laid = measure_fixture(r#"<html><body><p style="word-spacing: normal">several plain words here</p></body></html>"#, 500.0);
    assert!(
        find_mixed_inline(&laid).is_none(),
        "`word-spacing: normal` should not route a plain-text run through the slower per-word path"
    );
    let mut ids = Vec::new();
    collect_text_shape_ids(&laid, &mut ids);
    assert!(!ids.is_empty(), "sanity: the paragraph should still have shaped via the ordinary whole-buffer `LaidContent::Text` path");
}

#[test]
fn nonzero_word_spacing_routes_through_the_per_word_path_with_no_image_or_float() {
    // The core routing change this item adds: unlike v48 (image
    // present) and v54 (an active float), a plain-text run with a
    // declared `word-spacing` has neither — it still has to take
    // `measure_mixed_inline`'s hand-rolled path, since that's the
    // only place this property is actually applied (see
    // `ComputedStyle::word_spacing`'s own docs).
    let laid = measure_fixture(r#"<html><body><p style="word-spacing: 10px">foo bar</p></body></html>"#, 500.0);
    let run = find_mixed_inline(&laid)
        .expect("a run with nonzero word-spacing and no image/float should still route through measure_mixed_inline");
    assert_eq!(mixed_items(run).len(), 2, "two words, each its own MixedPositioned::Word");
}

#[test]
fn word_spacing_increases_the_gap_between_two_words() {
    // The real geometry proof, not just "doesn't crash": the second
    // word's own starting `x` should shift further right as the
    // declared `word-spacing` grows, by exactly the difference
    // between the two declared values — proving this is a real,
    // precisely-sized per-word position change, not a no-op that
    // merely avoids a crash/mis-wrap.
    //
    // Deliberately *not* computed as "word 1's right edge to word
    // 2's left edge": the extra spacing is baked directly into word
    // 1's own stored `w` (see `measure_mixed_inline`'s own docs) —
    // the same "a trailing space counts toward this word's own
    // width" treatment this file already gives the word's ordinary
    // trailing space, just extended uniformly to cover the extra
    // word-spacing too — so that naive gap would always measure ~0
    // regardless of the declared value. The second word's absolute
    // `x` is where the extra space actually becomes observable.
    let second_word_x = |spacing_px: f32| -> f32 {
        let html = format!(r#"<html><body><p style="margin: 0; word-spacing: {spacing_px}px">foo bar</p></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
        let items = mixed_items(run);
        assert_eq!(items.len(), 2);
        match &items[1] {
            MixedPositioned::Word { x, .. } => *x,
            MixedPositioned::Embedded(_) => panic!("expected a word"),
        }
    };
    let x1_10 = second_word_x(10.0);
    let x1_30 = second_word_x(30.0);
    assert!(
        (x1_30 - x1_10 - 20.0).abs() < 0.5,
        "raising word-spacing from 10px to 30px should shift the second word exactly 20px further right (only the extra spacing differs between these two calls — the words themselves shape identically); got {x1_10} then {x1_30} (delta {})",
        x1_30 - x1_10
    );
}

#[test]
fn word_spacing_does_not_shatter_a_short_paragraph_the_way_letter_spacing_did() {
    // Direct regression test against the exact failure shape that
    // broke the reverted `letter-spacing` attempt (see TODO.txt):
    // a short paragraph that trivially fits on one line at a wide
    // (900px) container shattered into one line per word (worse at
    // larger values) once that attempt's spacing crossed roughly
    // 2-3px, because it wired spacing into cosmic-text's own
    // `Attrs`/shaping and cosmic-text's *internal* wrap-width
    // computation got the word width wrong. `word-spacing` never
    // touches `Attrs` at all — only this hand-rolled path's own
    // position math changes — so the exact same scenario, at the
    // exact value (8px) that broke letter-spacing into 4 lines,
    // should still fit on one line here.
    for spacing_px in [2.0, 8.0] {
        let html = format!(
            r#"<html><body><p style="margin: 0; word-spacing: {spacing_px}px">Wide word spacing</p></body></html>"#
        );
        let laid = measure_fixture(&html, 900.0);
        let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
        let lines = mixed_inline_line_count(run);
        assert_eq!(
            lines, 1,
            "a short 3-word paragraph at a 900px container should still fit on one line with {spacing_px}px word-spacing, not shatter into {lines} lines the way letter-spacing's own attempt did at comparable values"
        );
    }
}

#[test]
fn word_spacing_does_not_break_realistic_multi_word_wrapping() {
    // The other half of the same reproduction: a longer, realistic
    // paragraph that genuinely needs to wrap across several lines at
    // a normal article width should still group multiple words per
    // line with a typical `word-spacing` value — not degrade toward
    // one word (or worse) per line the way letter-spacing's own bug
    // did at a real paragraph width.
    let paragraph = "The quick brown fox jumps over the lazy dog while the sun sets \
         behind the distant mountains slowly and quietly each evening";
    let word_count = paragraph.split_whitespace().count();
    let html = format!(r#"<html><body><p style="margin: 0; word-spacing: 3px">{paragraph}</p></body></html>"#);
    let laid = measure_fixture(&html, 600.0);
    let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
    let lines = mixed_inline_line_count(run);
    assert!(lines >= 1, "sanity: should produce at least one line");
    assert!(
        lines * 3 < word_count,
        "a {word_count}-word paragraph wrapping into {lines} lines at a realistic 600px width should still average well over 3 words per line, not shatter down toward one word per line the way letter-spacing's own bug did"
    );
}

// ---------- v60: `text-decoration-line`/`-style`/`-color` longhands, `wavy`, `double` for line-through/overline ----------

/// Same shape as v39's own `collect_decorations`, but also captures
/// `decoration_style` alongside `text_decoration` — needed by this
/// section's tests, which care about the *style* (solid/dashed/
/// double/wavy) a span resolved to, not just which lines are on.
fn collect_decoration_values(b: &LayoutBox, out: &mut Vec<(TextDecoration, DecorationStyle)>) {
    if let BoxContent::Inline(items) = &b.content {
        for item in items {
            if let InlineItem::Text(_, s) = item {
                out.push((s.text_decoration, s.decoration_style));
            }
        }
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            collect_decoration_values(c, out);
        }
    }
}

#[test]
fn text_decoration_longhands_combine_like_the_shorthand() {
    // The core claim of this item's gap #1: `text-decoration-line`/
    // `-style`/`-color`, declared as three *separate* properties with
    // no `text-decoration` shorthand anywhere, should combine into
    // exactly the same effective decoration a single `text-decoration:
    // underline dashed #ff0000` shorthand declaration would produce.
    let longhand_doc = parse_html(
        r#"<html><body><span style="text-decoration-line: underline; text-decoration-style: dashed; text-decoration-color: #ff0000">x</span></body></html>"#,
    );
    let shorthand_doc = parse_html(
        r#"<html><body><span style="text-decoration: underline dashed #ff0000">x</span></body></html>"#,
    );
    let mut longhand_values = Vec::new();
    collect_decoration_values(&build_layout_tree(&longhand_doc, 1920.0), &mut longhand_values);
    let mut shorthand_values = Vec::new();
    collect_decoration_values(&build_layout_tree(&shorthand_doc, 1920.0), &mut shorthand_values);
    assert_eq!(longhand_values.len(), 1, "expected exactly one span: {:?}", longhand_values);
    assert_eq!(shorthand_values.len(), 1, "expected exactly one span: {:?}", shorthand_values);
    let (ld, ls) = longhand_values[0];
    let (sd, ss) = shorthand_values[0];
    assert_eq!(ls, ss, "the three longhands together should resolve to the same DecorationStyle as the shorthand");
    assert_eq!(ld.underline, sd.underline, "both should turn underline on");
    assert_eq!(ld.underline_color_opt, sd.underline_color_opt, "both should attach the same color to it");
    assert_eq!(ld.underline_color_opt, Some(TextColor::rgb(0xff, 0, 0)), "sanity: the color should actually be red, not just equal-to-each-other-and-wrong");
}

#[test]
fn text_decoration_style_longhand_overrides_shorthands_style_on_the_same_element() {
    // A longhand declared *alongside* the shorthand on the very same
    // element should override just its own one component — real
    // CSS's "combine at used-value time" semantics, not an
    // all-or-nothing replace the way the shorthand's own cascade
    // already works against an *ambient* value.
    let doc = parse_html(
        r#"<html><body><span style="text-decoration: underline red; text-decoration-style: dashed">x</span></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut values = Vec::new();
    collect_decoration_values(&tree, &mut values);
    assert_eq!(values.len(), 1);
    let (deco, style) = values[0];
    assert_eq!(deco.underline, UnderlineStyle::Single, "the shorthand's own line should still apply");
    assert_eq!(deco.underline_color_opt, Some(TextColor::rgb(0xff, 0, 0)), "the shorthand's own color should still apply");
    assert_eq!(style, DecorationStyle::Dashed, "the longhand text-decoration-style should override the shorthand's own implicit `solid`");
}

#[test]
fn text_decoration_color_longhand_alone_has_no_effect_without_any_line() {
    // `text-decoration-color` (or `-style`) declared with no line
    // active anywhere (no shorthand, no `-line` longhand, no ambient
    // decoration from an ancestor) must stay a no-op — same "don't
    // leave a stray color/style on an inactive line" rule
    // `parse_text_decoration` already enforces for the shorthand
    // itself, now also true across the three longhands combined.
    let doc = parse_html(r#"<html><body><span style="text-decoration-color: red; text-decoration-style: dashed">x</span></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut values = Vec::new();
    collect_decoration_values(&tree, &mut values);
    assert_eq!(values.len(), 1);
    let (deco, _style) = values[0];
    assert_eq!(deco, TextDecoration::new(), "no line was ever turned on, so color/style alone should paint nothing: {:?}", deco);
}

#[test]
fn text_decoration_line_longhand_propagates_ambiently_like_the_shorthand() {
    // The combined *result* (not the raw longhand) is what threads on
    // as the new ambient value for children — a descendant that
    // declares none of these four properties should still see its
    // ancestor's longhand-only decoration, same ambient-propagation
    // shape the shorthand already had since v17/v33.
    let doc = parse_html(
        r#"<html><body><div style="text-decoration-line: underline"><div>nested</div></div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    let mut values = Vec::new();
    collect_decoration_values(&tree, &mut values);
    assert!(
        values.iter().any(|(d, _)| d.underline == UnderlineStyle::Single),
        "a nested block descendant should inherit the ambient `text-decoration-line: underline`: {:?}",
        values
    );
}

/// A minimal `Renderer` test double that just records every
/// `rectangle` call verbatim — lets `paint_decoration_line` be
/// exercised directly, with no real `cosmic_text::Buffer`/font
/// shaping needed, the same way v33's own (since-deleted, per the
/// since-reverted pre-v41 policy) test double worked.
#[derive(Default)]
struct CollectingRenderer {
    rects: Vec<(i32, i32, u32, u32, TextColor)>,
}

impl Renderer for CollectingRenderer {
    fn rectangle(&mut self, x: i32, y: i32, w: u32, h: u32, color: TextColor) {
        self.rects.push((x, y, w, h, color));
    }
    fn glyph(&mut self, _physical_glyph: cosmic_text::PhysicalGlyph, _color: TextColor) {}
}

#[test]
fn wavy_decoration_produces_a_real_sine_wave_not_a_straight_line() {
    // The core claim of this item's gap #2: `Wavy` must paint a
    // genuine sine-wave pattern — not just "doesn't crash" and not
    // just "paints something" — verified by checking the painted
    // segments' own `y` actually rises *and* falls across the
    // stroke's width (a monotonic ramp would also vary, but isn't a
    // wave), unlike `Solid`, which paints one single flat rectangle.
    let mut renderer = CollectingRenderer::default();
    paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(0, 0, 0), DecorationStyle::Wavy);
    assert!(
        renderer.rects.len() > 4,
        "a wavy stroke across 60px should paint several small segments tracing the curve, not one rectangle: {:?}",
        renderer.rects
    );
    let ys: Vec<i32> = renderer.rects.iter().map(|r| r.1).collect();
    let min_y = *ys.iter().min().unwrap();
    let max_y = *ys.iter().max().unwrap();
    assert!(
        max_y - min_y >= 2,
        "a real sine-wave stroke should vary its y position across its width, not stay flat the way a Solid line's single rectangle does: observed y range {min_y}..{max_y}"
    );
    let mut rose = false;
    let mut fell = false;
    for i in 1..ys.len() {
        if ys[i] > ys[i - 1] {
            rose = true;
        }
        if ys[i] < ys[i - 1] {
            fell = true;
        }
    }
    assert!(rose && fell, "a sine wave should both rise and fall across its own extent, not move monotonically like a diagonal line: {:?}", ys);
}

#[test]
fn solid_decoration_stays_perfectly_flat_unlike_wavy() {
    // The control case for the test just above — confirms the flat-
    // line assertion wavy is contrasted against is actually true of
    // `Solid`, not an assumption.
    let mut renderer = CollectingRenderer::default();
    paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(0, 0, 0), DecorationStyle::Solid);
    assert_eq!(renderer.rects.len(), 1, "Solid should paint exactly one rectangle spanning the whole width");
    assert_eq!(renderer.rects[0].1, 100, "Solid's one rectangle should sit at exactly the requested y, with no wave offset");
}

#[test]
fn double_style_paints_two_parallel_solid_strokes_via_the_shared_primitive() {
    // `paint_decoration_line` is the one shared primitive `paint_
    // decorations` calls identically for underline/strikethrough/
    // overline (see its own docs) — proving `Double` paints two
    // separate `Solid` rectangles here, once, covers all three line
    // types at once; the two pixel tests below additionally confirm
    // it end-to-end through real text shaping for line-through/
    // overline specifically (this item's own gap #3).
    let mut renderer = CollectingRenderer::default();
    paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(255, 0, 0), DecorationStyle::Double);
    assert_eq!(renderer.rects.len(), 2, "Double should paint exactly two rectangles: {:?}", renderer.rects);
    assert_eq!(renderer.rects[0].1, 100, "first stroke at the requested y");
    assert_eq!(renderer.rects[1].1, 104, "second stroke offset by thickness (2) + an equal gap (2) = 4px below the first");
    for r in &renderer.rects {
        assert_eq!(r.4, TextColor::rgb(255, 0, 0));
        assert_eq!(r.2, 60, "each stroke should span the full requested width");
    }
}

/// Finds every contiguous run ("band") of exactly `color` pixels down
/// column `x` of a painted `(pixels, win_w, win_h)` fixture — used by
/// this section's end-to-end double-line-through/overline tests to
/// confirm *two* separate bands paint, not one, proving `Double`
/// reaches real text rendering (not just the primitive tested above
/// in isolation).
fn color_bands_in_column(pixels: &[u32], win_w: u32, win_h: u32, x: u32, color: u32) -> usize {
    let mut bands = 0;
    let mut in_band = false;
    for y in 0..win_h {
        let matches = pixels[(y * win_w + x) as usize] & 0x00FF_FFFF == color;
        if matches && !in_band {
            bands += 1;
        }
        in_band = matches;
    }
    bands
}

#[test]
fn double_line_through_paints_two_distinct_strokes_end_to_end() {
    // This item's own gap #3: cosmic-text 0.19 has no double variant
    // for strikethrough at all (only underline) — before this item,
    // `text-decoration: line-through double` silently painted one
    // plain line. Verified with real text shaping/painting (`paint_
    // fixture`), not just the primitive-level test above, so this
    // actually proves the full pipeline (span style → `paint_
    // decorations` → `paint_decoration_line`) wires `Double` through
    // for strikethrough, not only underline.
    let (pixels, win_w, win_h) = paint_fixture(
        r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: line-through double red">HELLO</p></body></html>"#,
    );
    let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
    assert_eq!(bands, 2, "a double line-through should paint two distinct red strokes down this column, not one");
}

#[test]
fn double_overline_paints_two_distinct_strokes_end_to_end() {
    // Same gap, the other previously-unreachable line: cosmic-text
    // 0.19 has no double variant for overline either.
    let (pixels, win_w, win_h) = paint_fixture(
        r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: overline double red">HELLO</p></body></html>"#,
    );
    let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
    assert_eq!(bands, 2, "a double overline should paint two distinct red strokes down this column, not one");
}

#[test]
fn double_underline_still_paints_two_distinct_strokes_after_the_double_refactor() {
    // Regression guard for this item's refactor: doubling used to be
    // driven by cosmic-text's own native `UnderlineStyle::Double`
    // (set directly by the old `parse_text_decoration`), now it's
    // driven uniformly by `DecorationStyle::Double` for all three
    // lines instead (see `paint_decorations`'s own docs on why that's
    // safe) — confirms underline's own pre-existing v33 behavior
    // wasn't lost in that switch.
    let (pixels, win_w, win_h) = paint_fixture(
        r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: underline double red">HELLO</p></body></html>"#,
    );
    let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
    assert_eq!(bands, 2, "a double underline should still paint two distinct red strokes, same as before this item's refactor");
}

// ---------- v61: extended `cursor` keyword set (TODO.txt autonomous pass) ----------

fn find_first_span_cursor(b: &LayoutBox) -> Option<CursorStyle> {
    if let BoxContent::Inline(items) = &b.content {
        if let Some(InlineItem::Text(_, s)) = items.first() {
            return Some(s.cursor);
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_first_span_cursor(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn every_cursor_keyword_parses_and_cascades_to_the_right_style() {
    // Covers the original two keywords (v32, re-added here since v32
    // predates v41's "keep tests permanently" policy and its own were
    // written-then-deleted) plus every keyword this item added — one
    // table-driven test through the real `style="cursor: ..."`
    // parsing path (`DeclParser`/`cursor_style_from_keyword`), not
    // just the keyword-mapping helper in isolation.
    let cases: &[(&str, CursorStyle)] = &[
        ("auto", CursorStyle::Auto),
        ("pointer", CursorStyle::Pointer),
        ("default", CursorStyle::Default),
        ("none", CursorStyle::None),
        ("context-menu", CursorStyle::ContextMenu),
        ("help", CursorStyle::Help),
        ("progress", CursorStyle::Progress),
        ("wait", CursorStyle::Wait),
        ("cell", CursorStyle::Cell),
        ("crosshair", CursorStyle::Crosshair),
        ("text", CursorStyle::Text),
        ("vertical-text", CursorStyle::VerticalText),
        ("alias", CursorStyle::Alias),
        ("copy", CursorStyle::Copy),
        ("move", CursorStyle::Move),
        ("no-drop", CursorStyle::NoDrop),
        ("not-allowed", CursorStyle::NotAllowed),
        ("grab", CursorStyle::Grab),
        ("grabbing", CursorStyle::Grabbing),
        ("e-resize", CursorStyle::EResize),
        ("n-resize", CursorStyle::NResize),
        ("ne-resize", CursorStyle::NeResize),
        ("nw-resize", CursorStyle::NwResize),
        ("s-resize", CursorStyle::SResize),
        ("se-resize", CursorStyle::SeResize),
        ("sw-resize", CursorStyle::SwResize),
        ("w-resize", CursorStyle::WResize),
        ("ew-resize", CursorStyle::EwResize),
        ("ns-resize", CursorStyle::NsResize),
        ("nesw-resize", CursorStyle::NeswResize),
        ("nwse-resize", CursorStyle::NwseResize),
        ("col-resize", CursorStyle::ColResize),
        ("row-resize", CursorStyle::RowResize),
        ("all-scroll", CursorStyle::AllScroll),
        ("zoom-in", CursorStyle::ZoomIn),
        ("zoom-out", CursorStyle::ZoomOut),
    ];
    for (keyword, expected) in cases {
        let html = format!(r#"<html><body><span style="cursor: {keyword}">x</span></body></html>"#);
        let doc = parse_html(&html);
        let tree = build_layout_tree(&doc, 1920.0);
        let cursor = find_first_span_cursor(&tree).unwrap_or_else(|| panic!("no span found for cursor: {keyword}"));
        assert_eq!(cursor, *expected, "cursor: {keyword} should cascade to {expected:?}, got {cursor:?}");
    }
}

#[test]
fn unrecognized_cursor_keyword_is_dropped_not_guessed() {
    assert!(cursor_style_from_keyword("bogus-keyword-xyz").is_none());
}

#[test]
fn cursor_inherits_through_a_nested_span_for_a_new_keyword() {
    // Same real-CSS-inheritance shape v32's own doc comment described
    // for `pointer`/`<a>` — exercised here with one of this item's new
    // keywords to confirm inheritance wasn't special-cased to just the
    // two original variants.
    let doc = parse_html(r#"<html><body><div style="cursor: wait"><span>inherited</span></div></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let cursor = find_first_span_cursor(&tree).expect("no span found");
    assert_eq!(cursor, CursorStyle::Wait, "a nested span with no cursor of its own should inherit the ancestor's wait");
}

#[test]
fn to_winit_maps_new_keywords_onto_matching_cursor_icons() {
    // Direct `CursorStyle` -> `CursorIcon` mapping tests for a
    // representative spread of the new variants (every arm is a
    // literal 1:1 rename, so this isn't exhaustive over all 30+, just
    // enough to prove the match arms aren't transposed/miswired).
    assert_eq!(CursorStyle::Default.to_winit(), Some(CursorIcon::Default));
    assert_eq!(CursorStyle::Text.to_winit(), Some(CursorIcon::Text));
    assert_eq!(CursorStyle::Wait.to_winit(), Some(CursorIcon::Wait));
    assert_eq!(CursorStyle::Progress.to_winit(), Some(CursorIcon::Progress));
    assert_eq!(CursorStyle::Crosshair.to_winit(), Some(CursorIcon::Crosshair));
    assert_eq!(CursorStyle::Move.to_winit(), Some(CursorIcon::Move));
    assert_eq!(CursorStyle::Grab.to_winit(), Some(CursorIcon::Grab));
    assert_eq!(CursorStyle::Grabbing.to_winit(), Some(CursorIcon::Grabbing));
    assert_eq!(CursorStyle::NotAllowed.to_winit(), Some(CursorIcon::NotAllowed));
    assert_eq!(CursorStyle::Help.to_winit(), Some(CursorIcon::Help));
    assert_eq!(CursorStyle::ZoomIn.to_winit(), Some(CursorIcon::ZoomIn));
    assert_eq!(CursorStyle::ZoomOut.to_winit(), Some(CursorIcon::ZoomOut));
    assert_eq!(CursorStyle::NResize.to_winit(), Some(CursorIcon::NResize));
    assert_eq!(CursorStyle::EwResize.to_winit(), Some(CursorIcon::EwResize));
    assert_eq!(CursorStyle::ColResize.to_winit(), Some(CursorIcon::ColResize));
    assert_eq!(CursorStyle::RowResize.to_winit(), Some(CursorIcon::RowResize));
    assert_eq!(CursorStyle::AllScroll.to_winit(), Some(CursorIcon::AllScroll));
}

#[test]
fn cursor_none_maps_to_hide_not_to_any_icon() {
    // The one real keyword with no `CursorIcon` equivalent at all —
    // `to_winit()` signals "hide the cursor" via `None`, which the
    // live `CursorMoved` handler turns into `window.set_cursor_
    // visible(false)` instead of `window.set_cursor(..)`.
    assert_eq!(CursorStyle::None.to_winit(), None, "cursor: none should signal \"hide,\" not any particular icon");
}

fn measure_fixture_with_cache(html: &str, available_width: f32) -> (Laid, ShapeCache) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, available_width, None, &mut next_id, &mut cache, deadline,
        &mut incomplete, &mut Vec::new(), &Floats::default(), None,
    );
    (laid, cache)
}

/// Finds the pixel-space center of the first glyph whose own per-span
/// cursor (via `glyph.metadata`, the same lookup `hit_test_cursor`
/// itself does) matches `want` — lets a test drive `hit_test_cursor`
/// at a real, specific coordinate instead of guessing one.
fn find_glyph_center(laid: &Laid, cache: &ShapeCache, want: CursorStyle) -> Option<(f32, f32)> {
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            children.iter().find_map(|c| find_glyph_center(c, cache, want))
        }
        LaidContent::Text(id) => {
            let entry = cache.get(id)?;
            let guard = entry.lock().unwrap();
            let (buffer, _, _, _, _, cursors, _, _) = &*guard;
            let content_x = laid.x + laid.style.border_left() + laid.style.padding.left.px();
            let content_y = laid.y + laid.style.border_top() + laid.style.padding.top.px();
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    if cursors.get(glyph.metadata).copied() == Some(want) {
                        return Some((content_x + glyph.x + glyph.w / 2.0, content_y + run.line_top + run.line_height / 2.0));
                    }
                }
            }
            None
        }
        LaidContent::Image(_) | LaidContent::MixedInline(_) => None,
    }
}

#[test]
fn hit_test_cursor_generalizes_to_a_new_keyword_at_glyph_precision() {
    // The requirement this item's task explicitly called out: v32's
    // `hit_test_cursor` is keyed on "which `CursorStyle` does this
    // glyph's span resolve to," not hardcoded to `Pointer` — so a
    // brand-new keyword (`grab`, picked arbitrarily, not special) needs
    // zero changes to `hit_test_cursor` itself to hit-test correctly at
    // real glyph precision, same diligence v32's own pipeline test used
    // for `Pointer` vs. `Auto`.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><span style="cursor: grab">drag me</span> plain text</p></body></html>"#,
        400.0,
    );
    let (gx, gy) = find_glyph_center(&laid, &cache, CursorStyle::Grab).expect("no grab-styled glyph found");
    let (ax, ay) = find_glyph_center(&laid, &cache, CursorStyle::Auto).expect("no plain-text glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(
        hit_test_cursor(&laid, gx, gy, &cache, &scroll_offsets, 0.0, 600.0).0,
        CursorStyle::Grab,
        "a glyph inside the styled span should report Grab"
    );
    assert_eq!(
        hit_test_cursor(&laid, ax, ay, &cache, &scroll_offsets, 0.0, 600.0).0,
        CursorStyle::Auto,
        "an adjacent plain-text glyph should still report Auto"
    );
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

// ---------- v64: `@media` (min-width/max-width, and/comma lists, print, viewport resize) ----------

/// Walks the tree for the first box whose own resolved style declares
/// a `background-color` — the simplest single signal to check whether
/// a `@media`-wrapped rule actually applied or not, reused by every
/// test below instead of a bespoke finder per test.
fn find_box_with_background(b: &LayoutBox) -> Option<TextColor> {
    if b.style.background_color.is_some() {
        return b.style.background_color;
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_box_with_background(c) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn media_min_width_applies_only_at_or_above_the_breakpoint() {
    let doc = parse_html(
        r#"<html><body><style>@media (min-width: 600px) { .box { background-color: #ff0000; } }</style><div class="box">x</div></body></html>"#,
    );
    let wide = build_layout_tree(&doc, 800.0);
    assert_eq!(
        find_box_with_background(&wide),
        Some(TextColor::rgb(0xff, 0, 0)),
        "a min-width: 600px rule should apply at an 800px viewport"
    );
    let narrow = build_layout_tree(&doc, 400.0);
    assert_eq!(
        find_box_with_background(&narrow),
        None,
        "a min-width: 600px rule should NOT apply at a 400px viewport"
    );
    // Exactly at the breakpoint counts as matching (min-width is
    // inclusive, real CSS `>=` semantics).
    let exact = build_layout_tree(&doc, 600.0);
    assert_eq!(
        find_box_with_background(&exact),
        Some(TextColor::rgb(0xff, 0, 0)),
        "min-width: 600px should match exactly at 600px (inclusive)"
    );
}

#[test]
fn media_max_width_applies_only_at_or_below_the_breakpoint() {
    let doc = parse_html(
        r#"<html><body><style>@media (max-width: 600px) { .box { background-color: #00ff00; } }</style><div class="box">x</div></body></html>"#,
    );
    let narrow = build_layout_tree(&doc, 400.0);
    assert_eq!(
        find_box_with_background(&narrow),
        Some(TextColor::rgb(0, 0xff, 0)),
        "a max-width: 600px rule should apply at a 400px viewport"
    );
    let wide = build_layout_tree(&doc, 800.0);
    assert_eq!(
        find_box_with_background(&wide),
        None,
        "a max-width: 600px rule should NOT apply at an 800px viewport — this is the exact inverse of min-width, confirming the comparison direction isn't flipped"
    );
}

#[test]
fn media_and_requires_every_feature_to_hold() {
    // `screen and (min-width: 600px)` — both the type and the width
    // feature must hold; screen always matches here, so this is
    // really exercising that `and` doesn't silently drop the width
    // condition (a bug that would make this behave like a bare
    // `screen` query with no width gating at all).
    let doc = parse_html(
        r#"<html><body><style>@media screen and (min-width: 600px) { .box { background-color: #0000ff; } }</style><div class="box">x</div></body></html>"#,
    );
    let wide = build_layout_tree(&doc, 800.0);
    assert_eq!(find_box_with_background(&wide), Some(TextColor::rgb(0, 0, 0xff)));
    let narrow = build_layout_tree(&doc, 400.0);
    assert_eq!(
        find_box_with_background(&narrow),
        None,
        "`screen and (min-width: 600px)` must fail at 400px even though `screen` alone would match"
    );
}

#[test]
fn media_comma_list_is_or_either_branch_matching_is_enough() {
    // `(min-width: 900px), (max-width: 300px)` — two mutually
    // exclusive branches at any viewport between them; a width of
    // 500px should match NEITHER, confirming comma is OR (not
    // silently treated as AND, which would make this never match at
    // any width, or silently treated as the same branch twice).
    let doc = parse_html(
        r#"<html><body><style>@media (min-width: 900px), (max-width: 300px) { .box { background-color: #ff00ff; } }</style><div class="box">x</div></body></html>"#,
    );
    let neither = build_layout_tree(&doc, 500.0);
    assert_eq!(find_box_with_background(&neither), None, "500px matches neither comma branch");
    let first_branch = build_layout_tree(&doc, 1000.0);
    assert_eq!(
        find_box_with_background(&first_branch),
        Some(TextColor::rgb(0xff, 0, 0xff)),
        "1000px should match the first (min-width: 900px) branch"
    );
    let second_branch = build_layout_tree(&doc, 200.0);
    assert_eq!(
        find_box_with_background(&second_branch),
        Some(TextColor::rgb(0xff, 0, 0xff)),
        "200px should match the second (max-width: 300px) branch"
    );
}

#[test]
fn media_print_never_matches_regardless_of_width() {
    // scratchy has no print/paged-media concept at all — `@media
    // print` is intentionally always inert, a documented design
    // choice (see TODO.txt), not a gap. Checked at a width that
    // would satisfy the rule's own min-width if the type weren't
    // print, to make sure `print` itself is what's rejecting it,
    // not the width condition.
    let doc = parse_html(
        r#"<html><body><style>@media print and (min-width: 0px) { .box { background-color: #123456; } }</style><div class="box">x</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    assert_eq!(find_box_with_background(&tree), None, "@media print should never match — scratchy has no print concept");
}

#[test]
fn media_calc_feature_value_is_evaluated() {
    // Q11: `calc()` in a media feature is now evaluated (1120-1 = 1119).
    let doc = parse_html(
        r#"<html><body><style>@media (max-width: calc(1120px - 1px)) { .box { background-color: #abcdef; } }</style><div class="box">x</div></body></html>"#,
    );
    let narrow = build_layout_tree(&doc, 1119.0);
    assert_eq!(find_box_with_background(&narrow), Some(TextColor::rgb(0xab, 0xcd, 0xef)));
    let wide = build_layout_tree(&doc, 1120.0);
    assert_eq!(find_box_with_background(&wide), None);
}

#[test]
fn media_rule_safely_coexists_with_an_unconditioned_rule() {
    // A plain, non-`@media` rule (empty `media` list) must keep
    // applying regardless of viewport width, unaffected by this
    // item's own changes — the lowest-risk possible regression
    // check for "did wiring viewport width into matching_
    // declarations somehow make ordinary rules width-sensitive too."
    let doc = parse_html(
        r#"<html><body><style>.always { background-color: #111111; }</style><div class="always">x</div></body></html>"#,
    );
    let narrow = build_layout_tree(&doc, 10.0);
    assert_eq!(find_box_with_background(&narrow), Some(TextColor::rgb(0x11, 0x11, 0x11)));
    let wide = build_layout_tree(&doc, 10000.0);
    assert_eq!(find_box_with_background(&wide), Some(TextColor::rgb(0x11, 0x11, 0x11)));
}

#[test]
fn media_rematches_on_simulated_resize_with_no_new_mechanism() {
    // `build_layout_tree` is the exact function `App::redraw` already
    // reruns whenever the live window width changes — calling it
    // twice with two different widths from the same parsed document
    // is a faithful simulation of that real resize path, not just a
    // unit-level check of `MediaQuery::matches` in isolation.
    let doc = parse_html(
        r#"<html><body><style>@media (min-width: 500px) { .box { background-color: #fedcba; } }</style><div class="box">x</div></body></html>"#,
    );
    let before_resize = build_layout_tree(&doc, 300.0);
    assert_eq!(find_box_with_background(&before_resize), None);
    let after_resize = build_layout_tree(&doc, 700.0);
    assert_eq!(
        find_box_with_background(&after_resize),
        Some(TextColor::rgb(0xfe, 0xdc, 0xba)),
        "the same document, rebuilt at a wider width, must pick up a min-width rule it didn't before"
    );
}

#[test]
fn prefers_color_scheme_defaults_to_light_never_dark() {
    // scratchy has no OS theme detection — documented fixed default:
    // `light` always matches, `dark` never does.
    let doc = parse_html(
        r#"<html><body><style>
            @media (prefers-color-scheme: light) { .box { background-color: #00ffff; } }
            @media (prefers-color-scheme: dark) { .other { background-color: #ff0000; } }
        </style><div class="box">x</div><div class="other">y</div></body></html>"#,
    );
    let tree = build_layout_tree(&doc, 1920.0);
    assert_eq!(
        find_box_with_background(&tree),
        Some(TextColor::rgb(0, 0xff, 0xff)),
        "prefers-color-scheme: light should match (the only color found should be .box's cyan, not .other's red)"
    );
}

// ---------- v65: link navigation ----------

// -- href resolution against the page's own <base href> --

#[test]
fn resolve_href_relative_joins_against_base() {
    let base = Url::parse("https://example.com/articles/foo").unwrap();
    let resolved = resolve_href(Some(&base), "bar").unwrap();
    assert_eq!(resolved.as_str(), "https://example.com/articles/bar");
}

#[test]
fn resolve_href_absolute_ignores_base_entirely() {
    // Real `Url::join` behavior, same as a real browser: an already-
    // absolute href resolves to itself regardless of what base the
    // page declares.
    let base = Url::parse("https://example.com/articles/foo").unwrap();
    let resolved = resolve_href(Some(&base), "https://other.example/bar").unwrap();
    assert_eq!(resolved.as_str(), "https://other.example/bar");
}

#[test]
fn resolve_href_relative_with_no_base_is_a_clear_error() {
    let err = resolve_href(None, "relative/path").unwrap_err();
    assert!(
        err.contains("no <base href>"),
        "error should explain there's nothing to resolve a relative link against: {err}"
    );
}

#[test]
fn resolve_href_absolute_with_no_base_still_resolves() {
    // No `<base>` on the page at all doesn't block an *already*-
    // absolute href — only a relative one has nothing to join
    // against.
    let resolved = resolve_href(None, "https://example.com/standalone").unwrap();
    assert_eq!(resolved.as_str(), "https://example.com/standalone");
}

#[test]
fn classify_input_recognizes_http_and_https_urls() {
    assert!(matches!(classify_input("http://example.com"), Input::Url(_)));
    assert!(matches!(classify_input("https://example.com/wiki/Foo"), Input::Url(_)));
}

#[test]
fn classify_input_treats_everything_else_as_a_file_path() {
    assert!(matches!(classify_input("page.html"), Input::File(_)));
    assert!(matches!(classify_input("/home/user/page.html"), Input::File(_)));
    // A scheme scratchy has no `wire`-backed fetch path for — same
    // "parsed successfully isn't the same question as fetchable"
    // reasoning `classify_input`'s own docs give.
    assert!(matches!(classify_input("file:///etc/hosts"), Input::File(_)));
    assert!(matches!(classify_input("data:text/plain,hi"), Input::File(_)));
}

#[test]
fn find_base_href_finds_a_real_base_tag() {
    let doc = parse_html(
        r#"<html><head><base href="https://example.com/articles/"></head><body>hi</body></html>"#,
    );
    assert_eq!(find_base_href(&doc), Some("https://example.com/articles/".to_string()));
}

#[test]
fn find_base_href_is_none_when_the_page_has_no_base_tag() {
    let doc = parse_html("<html><head></head><body>hi</body></html>");
    assert_eq!(find_base_href(&doc), None);
}

// -- v68: a fetched page with no <base> of its own falls back to the
// real URL it was fetched from, instead of leaving base_url at None
// and breaking every relative link on that page (found live — see
// PROMPTS.md's own v68 entry) --

#[test]
fn resolve_page_base_url_prefers_the_pages_own_base_tag_when_present() {
    let doc = parse_html(r#"<html><head><base href="https://example.com/real/"></head><body>hi</body></html>"#);
    let fetched_url = Url::parse("https://example.com/different-url").unwrap();
    assert_eq!(resolve_page_base_url(&doc, fetched_url).as_str(), "https://example.com/real/");
}

#[test]
fn resolve_page_base_url_falls_back_to_the_fetched_url_when_the_page_has_no_base_tag() {
    // The exact real-world shape that broke live: a page with no
    // <base> at all (the common case) must still let a relative —
    // including a same-page fragment, e.g. MediaWiki's own
    // "#bodyContent" skip-link — link resolve, by falling back to
    // wherever the page was actually fetched from.
    let doc = parse_html("<html><head></head><body><a href=\"#bodyContent\">skip</a></body></html>");
    let fetched_url = Url::parse("https://en.wikipedia.org/wiki/Example").unwrap();
    let base = resolve_page_base_url(&doc, fetched_url.clone());
    assert_eq!(base, fetched_url);
    // And confirms the actual downstream effect: a relative href on
    // this page now resolves at all, instead of erroring with "no
    // <base href> on this page to resolve it against".
    assert_eq!(
        resolve_href(Some(&base), "#bodyContent").unwrap().as_str(),
        "https://en.wikipedia.org/wiki/Example#bodyContent"
    );
}

// -- click-vs-drag distinction --

#[test]
fn is_click_accepts_a_tiny_move_between_down_and_up() {
    // Ordinary hand tremor between a real mouse-down and mouse-up
    // over the same link shouldn't be mistaken for a drag.
    assert!(is_click((100.0, 100.0), (102.0, 101.0), CLICK_DRAG_THRESHOLD_PX));
}

#[test]
fn is_click_rejects_a_real_drag() {
    assert!(!is_click((100.0, 100.0), (260.0, 100.0), CLICK_DRAG_THRESHOLD_PX));
}

#[test]
fn is_click_boundary_is_inclusive() {
    // Exactly at the threshold distance still counts as a click, not
    // a drag — consistent with this file's other boundary-inclusive
    // conventions (see e.g. `@media`'s own min-width tests).
    assert!(is_click((0.0, 0.0), (CLICK_DRAG_THRESHOLD_PX, 0.0), CLICK_DRAG_THRESHOLD_PX));
}

// -- href recovered at glyph precision (reuses `cursor`'s own v32/
// v61 hit-testing machinery — same `hit_test_cursor` function, now
// returning the href half of its result too) --

#[test]
fn hit_test_cursor_recovers_the_real_href_of_a_clicked_link() {
    // `default.css` already declares `a { cursor: pointer }` (v32),
    // so `find_glyph_center`'s existing `CursorStyle::Pointer` lookup
    // doubles as "find the link's own glyph" here with no new test
    // helper needed.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a href="/page">link text</a> plain text</p></body></html>"#,
        400.0,
    );
    let (lx, ly) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
    let (px, py) = find_glyph_center(&laid, &cache, CursorStyle::Auto).expect("no plain-text glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(
        hit_test_cursor(&laid, lx, ly, &cache, &scroll_offsets, 0.0, 600.0).1,
        Some("/page"),
        "a glyph inside the link should report its real href"
    );
    assert_eq!(
        hit_test_cursor(&laid, px, py, &cache, &scroll_offsets, 0.0, 600.0).1,
        None,
        "an adjacent plain-text glyph should report no href at all"
    );
}

#[test]
fn href_ambiently_propagates_into_a_nested_element_with_no_href_of_its_own() {
    // Not a CSS property (see `SpanStyle::href`'s own docs), so this
    // isn't real CSS inheritance — but a `<b>` nested inside a real
    // `<a href>`, with no `href` of its own, still needs to report
    // the ancestor link's href, the same ambient-propagation shape
    // `background-color`/`text-decoration` already use.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a href="/x"><b>bold link</b></a></p></body></html>"#,
        400.0,
    );
    let (bx, by) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, bx, by, &cache, &scroll_offsets, 0.0, 600.0).1, Some("/x"));
}

#[test]
fn an_anchor_with_no_href_is_not_a_link() {
    // A fragment-target `<a name="...">`/`<a>` with no `href` at all
    // is real HTML, not a link — same distinction `OurPseudoClass::
    // Link`'s own `:link` matching already makes (v37). Note this
    // doesn't mean no cursor: `default.css`'s `a { cursor: pointer
    // }` rule is a plain tag selector, not `:link`-scoped, so it
    // still applies here — `find_glyph_center` has to look for
    // `Pointer`, same as a real link, even though `href` itself must
    // come back `None`. This is exactly why `href` needs its own
    // tracking rather than piggybacking on `cursor`.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a>not a link</a></p></body></html>"#,
        400.0,
    );
    let (x, y) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, x, y, &cache, &scroll_offsets, 0.0, 600.0).1, None);
}

// -- building the absolute-form request `wire` itself parses --

#[test]
fn build_get_request_text_uses_absolute_form_and_a_bare_host_header() {
    let url = Url::parse("https://example.com/path?q=1").unwrap();
    assert_eq!(
        build_get_request_text(&url),
        "GET https://example.com/path?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n"
    );
}

#[test]
fn build_get_request_text_includes_an_explicit_port_in_the_host_header() {
    let url = Url::parse("http://example.com:8080/x").unwrap();
    assert_eq!(build_get_request_text(&url), "GET http://example.com:8080/x HTTP/1.1\r\nHost: example.com:8080\r\n\r\n");
}

// -- parsing wire's own normalized response shape --

#[test]
fn parse_wire_response_extracts_content_type_and_body() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";
    let (content_type, body) = parse_wire_response(raw).expect("parse");
    assert_eq!(content_type, Some("text/html".to_string()));
    assert_eq!(body, b"hello");
}

#[test]
fn parse_wire_response_content_type_lookup_is_case_insensitive_and_ignores_other_headers() {
    let raw = b"HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nServer: wire\r\n\r\n\x89PNG";
    let (content_type, body) = parse_wire_response(raw).expect("parse");
    assert_eq!(content_type, Some("image/png".to_string()));
    assert_eq!(body, b"\x89PNG");
}

#[test]
fn parse_wire_response_errors_on_a_missing_header_terminator() {
    assert!(parse_wire_response(b"not a real response at all").is_err());
}

// -- dispatching by Content-Type, per `scratchy/TODO.txt`'s own
// settled response-framing contract --

#[test]
fn q8_charset_decoding_meta_header_bom_and_streaming() {
    // windows-1252 bytes, <meta charset=iso-8859-1> -> "é" (0xE9).
    let mut doc = b"<meta charset=iso-8859-1><p>caf".to_vec();
    doc.push(0xE9);
    assert!(decode_html_bytes(&doc, None).ends_with("caf\u{e9}"));
    // http-equiv form, and windows-1252 specific 0x80 = euro sign.
    let doc2 = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\">\x80";
    assert!(decode_html_bytes(doc2, None).ends_with('\u{20ac}'));
    // Header charset wins over meta; wire path transcodes to UTF-8.
    let out = dispatch_wire_response(Some("text/html; charset=ISO-8859-1".to_string()), vec![b'a', 0xE9]).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "a\u{e9}");
    // BOM'd UTF-8 has the BOM stripped; no BOM/meta stays UTF-8.
    assert_eq!(decode_html_bytes(b"\xEF\xBB\xBFhi \xC3\xA9", None), "hi \u{e9}");
    assert_eq!(decode_html_bytes("h\u{e9}".as_bytes(), None), "h\u{e9}");
    // Streaming: split chunks, sniff buffered, multi-byte split ok.
    let mut t = StreamTranscoder::new();
    let mut out = t.decode(b"<meta charset=latin1>", false);
    assert!(out.is_empty());
    out.extend(t.decode(&[0xE9; 1500], false));
    out.extend(t.decode(&[], true));
    let s = String::from_utf8(out).unwrap();
    assert!(s.starts_with("<meta") && s.ends_with("\u{e9}") && s.matches('\u{e9}').count() == 1500);
    // Short stream flushed at EOF.
    let mut t = StreamTranscoder::new();
    assert!(t.decode(b"<meta charset=latin1>\xE9", false).is_empty());
    assert!(String::from_utf8(t.decode(&[], true)).unwrap().ends_with('\u{e9}'));
}

#[test]
fn dispatch_wire_response_passes_html_through_untouched() {
    let body = b"<html><body>hi</body></html>".to_vec();
    assert_eq!(dispatch_wire_response(Some("text/html; charset=utf-8".to_string()), body.clone()), Ok(body));
}

#[test]
fn dispatch_wire_response_wraps_a_real_image_in_a_synthetic_html_document() {
    // A real tiny PNG, built the same way `tiny_png_data_uri` builds
    // one for the pre-existing `<img>`-rendering tests just above —
    // decoded back out of its own data: URI purely to get the raw
    // bytes a real `wire` response body would carry. Round-tripped
    // through the *real* `decode_data_uri` this produces an
    // `<img src>` for — not just a string-shape check — to confirm
    // the wrapped document is actually renderable, not merely
    // well-formed-looking HTML.
    let png_bytes = decode_data_uri(&tiny_png_data_uri(1, 1)).expect("decode the fixture's own data: URI");
    let html = dispatch_wire_response(Some("image/png".to_string()), png_bytes.clone()).expect("dispatch");
    let html = String::from_utf8(html).expect("utf8 html");
    assert!(html.starts_with("<html><body><img src=\"data:image/png;base64,"), "got: {html}");
    let data_uri_start = html.find("src=\"").unwrap() + "src=\"".len();
    let data_uri_end = html[data_uri_start..].find('"').unwrap() + data_uri_start;
    let decoded = decode_data_uri(&html[data_uri_start..data_uri_end]).expect("decode_data_uri");
    assert_eq!(decoded, png_bytes, "the wrapped data: URI must decode back to the exact original bytes");
}

#[test]
fn dispatch_wire_response_errors_on_an_unsupported_content_type() {
    let err = dispatch_wire_response(Some("application/pdf".to_string()), vec![1, 2, 3]).unwrap_err();
    assert!(err.contains("application/pdf"), "error should name the unsupported content type: {err}");
}

#[test]
fn dispatch_wire_response_errors_clearly_when_there_is_no_content_type_at_all() {
    assert!(dispatch_wire_response(None, vec![1, 2, 3]).is_err());
}

// -- the real wire-socket protocol end to end, against a tiny
// in-process mock listener (same spirit as `wire`'s own test module,
// which does the mirror image of this: a real client against a real
// listener, rather than touching the real internet) --

fn unique_test_socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scratchy-test-{name}-{}-{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn fetch_raw_via_wire_round_trips_a_real_response_over_a_real_unix_socket() {
    use std::os::unix::net::UnixListener;
    let path = unique_test_socket_path("roundtrip");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind mock wire socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut received = Vec::new();
        // The client always closes its write side (or just drops
        // the connection after reading) once it has what it wants —
        // reading to EOF here mirrors exactly what the real `wire`
        // daemon's own `handle_connection` does on its side of this
        // same protocol.
        let mut buf = [0u8; 4096];
        loop {
            match Read::read(&mut stream, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    received.extend_from_slice(&buf[..n]);
                    if received.ends_with(b"\r\n\r\n") {
                        break; // a GET request has no body to wait for
                    }
                }
            }
        }
        assert_eq!(
            received,
            build_get_request_text(&Url::parse("https://example.com/page").unwrap()).into_bytes(),
            "scratchy must send the real absolute-form request wire itself parses"
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 29\r\n\r\n<html><body>hi</body></html>")
            .expect("write response");
        // Dropping `stream` here closes the connection — the real
        // `wire` protocol's own "done" signal (see `wire/src/
        // main.rs`'s own `handle_connection`).
    });
    let url = Url::parse("https://example.com/page").unwrap();
    let request_text = build_get_request_text(&url);
    let result = fetch_raw_via_wire(&path, &request_text);
    server.join().expect("mock server thread panicked");
    let _ = std::fs::remove_file(&path);
    match result {
        WireFetchRaw::Responded { content_type, body } => {
            assert_eq!(content_type, Some("text/html".to_string()));
            assert_eq!(body, b"<html><body>hi</body></html>");
        }
        WireFetchRaw::Unreachable => panic!("expected a successful response, got Unreachable"),
    }
}

#[test]
fn fetch_raw_via_wire_reports_unreachable_when_nothing_is_listening() {
    let path = unique_test_socket_path("missing");
    let _ = std::fs::remove_file(&path);
    let request_text = build_get_request_text(&Url::parse("https://example.com/").unwrap());
    assert!(matches!(fetch_raw_via_wire(&path, &request_text), WireFetchRaw::Unreachable));
}

// -- sub-resource fetching (v67): external stylesheets and images --

#[test]
fn find_external_resources_collects_stylesheet_links_and_non_data_img_srcs() {
    let doc = parse_html(
        r#"<html><head>
            <link rel="stylesheet" href="a.css">
            <link rel="icon" href="favicon.ico">
            <link rel="alternate stylesheet" href="b.css">
        </head><body>
            <img src="photo.jpg">
            <img src="data:image/png;base64,iVBORw0KGgo=">
        </body></html>"#,
    );
    let mut resources = ExternalResources::default();
    find_external_resources(&doc, &mut resources);
    assert_eq!(resources.stylesheets, vec!["a.css".to_string(), "b.css".to_string()]);
    assert_eq!(resources.images, vec!["photo.jpg".to_string()]);
}

/// A tiny single-shot mock `wire` listener: binds `path`, accepts
/// exactly one connection, writes `response` verbatim, then returns
/// (dropping the stream, closing the connection — the real `wire`
/// protocol's own "done" signal, same shape every other mock server
/// in this file already uses). Doesn't bother reading/validating the
/// request at all — unlike `fetch_raw_via_wire`'s own round-trip
/// test, these tests are about what `fetch_stylesheet_via_wire`/
/// `fetch_image_via_wire` do with a *response*, not about re-proving
/// the request framing that's already covered elsewhere.
fn spawn_mock_wire_once(path: &std::path::Path, response: &'static [u8]) {
    use std::os::unix::net::UnixListener;
    let listener = UnixListener::bind(path).expect("bind mock wire socket");
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let mut discard = [0u8; 4096];
        let _ = Read::read(&mut stream, &mut discard);
        let _ = stream.write_all(response);
    });
}

#[test]
fn q22_file_page_gets_a_file_url_base_and_relative_hrefs_resolve_against_it() {
    let dir = std::env::temp_dir().join(format!("scratchy-q22-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("page.html");
    std::fs::write(&page, "<html></html>").unwrap();
    let base = file_page_url(page.to_str().unwrap()).expect("file url");
    assert_eq!(base.scheme(), "file");
    assert!(base.path().ends_with("/page.html"));
    let css = resolve_href(Some(&base), "style/a.css").unwrap();
    assert_eq!(css.scheme(), "file");
    assert!(css.path().ends_with("/style/a.css"));
    // The request `wire` receives is plain absolute-form.
    assert!(build_get_request_text(&css).starts_with("GET file:///"));
    assert!(file_page_url("/nonexistent/q22.html").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn q22_stylesheet_over_a_file_url_goes_through_wire() {
    let path = unique_test_socket_path("q22-css");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\np{margin:0}");
    let url = Url::parse("file:///tmp/a.css").unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &url), Some("p{margin:0}".to_string()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn q22b_streamed_page_stylesheets_resolve_against_its_base_href() {
    let path = unique_test_socket_path("q22b-css");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\nh1{color:red}");
    let doc = parse_html("<html><head><base href=\"file:///tmp/site/\"><link rel=stylesheet href=a.css></head><body><h1>x</h1></body></html>");
    let base = find_base_url(&doc).unwrap();
    let mut res = ExternalResources::default();
    find_external_resources(&doc, &mut res);
    assert_eq!(resolve_href(Some(&base), &res.stylesheets[0]).unwrap().as_str(), "file:///tmp/site/a.css");
    assert_eq!(fetch_stylesheets(&path, &base, &res.stylesheets), "h1{color:red}\n");
    // And a page with no <base> has nothing to resolve against.
    assert!(find_base_url(&parse_html("<link rel=stylesheet href=a.css>")).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_stylesheet_via_wire_only_accepts_real_css() {
    let path = unique_test_socket_path("css-ok");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 15\r\n\r\nbody{color:red}");
    let url = Url::parse("https://example.com/style.css").unwrap();
    // A real connection, over a real socket — not a direct call into
    // `fetch_sub_resource_via_wire`'s own internals — the same
    // "real I/O, not just the pure logic" bar `fetch_raw_via_wire`'s
    // own test already holds to.
    assert_eq!(fetch_stylesheet_via_wire(&path, &url), Some("body{color:red}".to_string()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn q18_expand_css_imports_in_place_media_depth_and_cycles() {
    let base = Url::parse("https://e.com/css/main.css").unwrap();
    let mut fetched: Vec<String> = Vec::new();
    let mut fetch = |u: &Url| -> Option<String> {
        fetched.push(u.to_string());
        match u.path() {
            "/css/a.css" => Some("@import 'b.css';\na{color:red}".to_string()),
            "/css/b.css" => Some("@import url(main.css);\nb{color:blue}".to_string()),
            "/css/p.css" => Some("p{margin:0}".to_string()),
            _ => None,
        }
    };
    let css = "@charset \"utf-8\";\n/* c */ @import url(\"a.css\");\n@import p.css;\n@import \"p.css\" print and (min-width:1px);\n@import \"missing.css\";\nbody{x:y}";
    let mut stack = vec![base.clone()];
    let out = expand_css_imports(css, &base, &mut stack, &mut fetch);
    // `@import p.css` (unquoted) is not valid CSS and is dropped.
    let b = out.find("b{color:blue}").unwrap();
    let a = out.find("a{color:red}").unwrap();
    let m = out.find("@media print and (min-width:1px) {").unwrap();
    assert!(b < a && a < m && m < out.find("body{x:y}").unwrap());
    assert!(!out.contains("@import"));
    // The cycle back to main.css was skipped, not fetched.
    assert!(!fetched.iter().any(|u| u.ends_with("main.css")));
    // Depth limit: a self-chaining fetcher stops.
    let mut n = 0;
    let mut chain = |_: &Url| -> Option<String> {
        n += 1;
        Some(format!("@import \"x{n}.css\"; i{n}{{}}"))
    };
    let out = expand_css_imports("@import \"x0.css\";", &base, &mut vec![base.clone()], &mut chain);
    assert_eq!(out.matches("{}").count(), MAX_IMPORT_DEPTH - 1);
}

#[test]
fn q18_stylesheet_charset_decoding() {
    assert_eq!(decode_css_bytes(b"a{content:'\xE9'}", Some(encoding_rs::WINDOWS_1252)), "a{content:'é'}");
    assert_eq!(decode_css_bytes(b"@charset \"iso-8859-1\"; a{content:'\xE9'}", None), "@charset \"iso-8859-1\"; a{content:'é'}");
    assert_eq!(decode_css_bytes(b"\xEF\xBB\xBFa{}", None), "a{}");
}

#[test]
fn fetch_stylesheet_via_wire_rejects_a_non_css_content_type() {
    let path = unique_test_socket_path("css-wrong-type");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\n<p/>");
    let url = Url::parse("https://example.com/not-css").unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &url), None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_image_via_wire_decodes_a_real_fetched_png() {
    use base64::Engine;
    let data_uri = tiny_png_data_uri(2, 2);
    let payload = data_uri.strip_prefix("data:image/png;base64,").unwrap();
    let png_bytes = base64::engine::general_purpose::STANDARD.decode(payload).unwrap();
    let mut raw = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png_bytes.len()).into_bytes();
    raw.extend_from_slice(&png_bytes);
    let path = unique_test_socket_path("image-ok");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, raw.leak());
    let url = Url::parse("https://example.com/pixel.png").unwrap();
    let decoded = fetch_image_via_wire(&path, &url).expect("decode");
    assert_eq!((decoded.width, decoded.height), (2, 2));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_image_via_wire_rejects_a_non_image_content_type() {
    let path = unique_test_socket_path("image-wrong-type");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello");
    let url = Url::parse("https://example.com/not-an-image").unwrap();
    assert!(fetch_image_via_wire(&path, &url).is_none());
    let _ = std::fs::remove_file(&path);
}

// -- the rendering side: does a cached live-fetched resource
// actually reach the page, not just the fetch functions above --

fn find_element_by_tag(handle: &Handle, tag: &str) -> Option<Handle> {
    if element_tag(handle) == tag {
        return Some(handle.clone());
    }
    handle.children.borrow().iter().find_map(|c| find_element_by_tag(c, tag))
}

#[test]
fn resolve_img_renders_a_cached_live_fetched_image_for_a_non_data_uri_src() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
    let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
    let decoded = DecodedImage { width: 4, height: 4, pixels: vec![TextColor::rgb(1, 2, 3); 16], svg: None, mask: None };
    let mut image_cache = HashMap::new();
    image_cache.insert("https://example.com/photo.jpg".to_string(), Arc::new(decoded));
    let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &image_cache);
    assert!(found.is_some(), "a src present in image_cache must be used, not fall back to alt text");
    let found = found.unwrap();
    assert_eq!((found.width, found.height), (4, 4));
}

#[test]
fn resolve_img_is_a_cache_miss_for_a_non_data_uri_src_not_yet_fetched() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/not-fetched-yet.jpg"></body></html>"#);
    let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
    let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &HashMap::new());
    assert!(found.is_none(), "an empty image_cache must be a clean miss, not a panic or a guess");
}

#[test]
fn build_layout_tree_with_resources_applies_extra_css_to_the_real_cascade() {
    let doc = parse_html(r#"<html><body><div class="box">x</div></body></html>"#);
    let extra_css = ".box { background-color: #00ff00; }";
    let tree = build_layout_tree_with_resources(&doc, 1920.0, None, extra_css, &HashMap::new());
    assert_eq!(
        find_box_with_background(&tree),
        Some(TextColor::rgb(0, 0xff, 0)),
        "CSS fetched from an external stylesheet must cascade exactly like an inline <style> block's own rules"
    );
}

#[test]
fn build_layout_tree_with_resources_renders_a_cached_image_as_a_real_image_box() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
    let mut image_cache = HashMap::new();
    image_cache.insert(
        "https://example.com/photo.jpg".to_string(),
        Arc::new(DecodedImage { width: 2, height: 2, pixels: vec![TextColor::rgb(9, 9, 9); 4], svg: None, mask: None }),
    );
    let tree = build_layout_tree_with_resources(&doc, 1920.0, None, "", &image_cache);
    fn find_image_box(b: &LayoutBox) -> Option<&LayoutBox> {
        if matches!(b.content, BoxContent::Image(_)) {
            return Some(b);
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_image_box(c) {
                    return Some(found);
                }
            }
        }
        None
    }
    assert!(
        find_image_box(&tree).is_some(),
        "a non-data: <img> with its src already in image_cache must render as a real image box, not fall back to alt text"
    );
}

// `process_external_resources` itself isn't unit-tested directly —
// it takes a real `EventLoopProxy<AppEvent>`, and this file's own
// established precedent (see NOTES.md's v65/v62/v55/v32 entries) is
// to never construct a real winit `EventLoop` inside `#[cfg(test)]`
// just to get one; its two real pieces (`find_external_resources`,
// `fetch_stylesheet_via_wire`, `fetch_image_via_wire`, all above)
// are each independently covered, and the thin orchestration between
// them (resolve each href, fetch/spawn) rests on code review plus
// the live interactive verification `PROMPTS.md`'s own v67 entry
// describes, same category as `start_navigation`'s own click-side
// wiring already is.

// -- `App::show_message`'s HTML-escaping, since it echoes back
// arbitrary page/OS-controlled text (a clicked href, an io::Error)
// into synthetic markup --

#[test]
fn escape_html_escapes_the_characters_that_would_corrupt_the_synthetic_markup() {
    assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
}

// ---------- vim mode: `resolve_binding`/`TextEditBuffer` ----------
// (TODO.txt's own plan — the mode enum, real keyboard wiring, the
// `:` command bar, `:set vim`/`:set novim`, `:go`, `:help`, `:q`/
// `:quit`, Tab completion, and the `SIGUSR1` bootstrap for turning
// vim mode back on once off.)

// ---------- command bar: Tab completion, `:q`/`:quit`/`:help`/`:go` ----------

#[test]
fn resolve_binding_maps_bare_jk_to_scroll_in_normal_mode() {
    let key = Key::Character("j".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollDown));
    let key = Key::Character("k".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollUp));
}

#[test]
fn resolve_binding_requires_control_for_half_page_scroll() {
    let key = Key::Character("d".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
    assert_eq!(
        resolve_binding(&key, ModifiersState::CONTROL, VimMode::Normal),
        Some(Action::HalfPageDown)
    );
}

#[test]
fn resolve_binding_colon_enters_command_mode() {
    let key = Key::Character(":".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::EnterCommandMode));
}

#[test]
fn resolve_binding_maps_bare_hl_to_history_back_and_front() {
    let key = Key::Character("h".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::HistoryBack));
    let key = Key::Character("l".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::HistoryFront));
}

#[test]
fn action_ignores_repeat_exempts_scroll_but_not_history_navigation() {
    // The regression this closes: holding `h` fires `Action::
    // HistoryBack` on every OS key-repeat tick, and `navigate_
    // history` isn't gated on its own fetch completing, so an
    // un-gated repeat would race straight through the whole
    // history stack before a single page even loads.
    assert!(action_ignores_repeat(Action::HistoryBack));
    assert!(action_ignores_repeat(Action::HistoryFront));
    assert!(!action_ignores_repeat(Action::ScrollDown));
    assert!(!action_ignores_repeat(Action::ScrollUp));
    assert!(!action_ignores_repeat(Action::HalfPageDown));
    assert!(!action_ignores_repeat(Action::HalfPageUp));
}

#[test]
fn resolve_binding_never_fires_outside_normal_mode() {
    let key = Key::Character("j".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Command), None);
}

#[test]
fn resolve_binding_ignores_unbound_keys() {
    let key = Key::Character("x".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
}

#[test]
fn key_insertion_text_handles_the_named_space_key_not_just_character_keys() {
    // The exact regression reported post-merge: `winit` reports
    // space as `Key::Named(NamedKey::Space)`, so a match arm that
    // only handles `Key::Character` silently drops it.
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Space)), Some(" "));
    assert_eq!(key_insertion_text(&Key::Character("a".into())), Some("a"));
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Enter)), None);
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Escape)), None);
}

#[test]
fn text_edit_buffer_insert_backspace_and_move_track_a_real_cursor() {
    let mut buf = TextEditBuffer::default();
    buf.insert('v');
    buf.insert('i');
    buf.insert('m');
    assert_eq!(buf.text, "vim");
    assert_eq!(buf.cursor, 3);

    buf.move_left();
    buf.move_left();
    assert_eq!(buf.cursor, 1);

    buf.insert('!');
    assert_eq!(buf.text, "v!im");
    assert_eq!(buf.cursor, 2);

    buf.backspace();
    assert_eq!(buf.text, "vim");
    assert_eq!(buf.cursor, 1);
}

#[test]
fn text_edit_buffer_backspace_and_move_left_at_start_are_no_ops() {
    let mut buf = TextEditBuffer::default();
    buf.backspace();
    buf.move_left();
    assert_eq!(buf.text, "");
    assert_eq!(buf.cursor, 0);
}

#[test]
fn text_edit_buffer_move_right_at_end_is_a_no_op() {
    let mut buf = TextEditBuffer::default();
    buf.insert('x');
    buf.move_right();
    assert_eq!(buf.cursor, 1);
}

#[test]
fn text_edit_buffer_moves_by_whole_codepoints_not_raw_bytes() {
    let mut buf = TextEditBuffer::default();
    buf.insert('é'); // 2 UTF-8 bytes, one char
    assert_eq!(buf.cursor, 2);
    buf.move_left();
    assert_eq!(buf.cursor, 0);
    buf.backspace(); // no-op at start, must not panic on a mid-codepoint byte index
    assert_eq!(buf.text, "é");
}

#[test]
fn text_edit_buffer_insert_str_drops_control_characters() {
    // The exact real-world shape this guards against: a clipboard
    // paste ending in a trailing newline, which `App::
    // paste_into_command_line` hands straight to `insert_str`.
    let mut buf = TextEditBuffer::default();
    buf.insert_str("https://example.com/\n");
    assert_eq!(buf.text, "https://example.com/");
    assert_eq!(buf.cursor, buf.text.len());
}

#[test]
fn text_edit_buffer_insert_str_inserts_at_the_cursor_not_just_the_end() {
    let mut buf = TextEditBuffer::default();
    buf.insert_str("go https://b/");
    buf.cursor = 3; // right after "go "
    buf.insert_str("https://a/ ");
    assert_eq!(buf.text, "go https://a/ https://b/");
}

#[test]
fn token_start_before_finds_the_last_whitespace_boundary() {
    assert_eq!(token_start_before("go", 2), 0, "single token: starts at 0");
    assert_eq!(token_start_before("set vi", 6), 4, "second token starts right after the space");
    assert_eq!(token_start_before("set ", 4), 4, "cursor right after a trailing space: empty token there");
    assert_eq!(token_start_before("", 0), 0);
}

#[test]
fn command_completions_offers_command_names_for_the_first_token() {
    let candidates = command_completions("g", 0, 1);
    assert_eq!(candidates, vec!["go"]);
}

#[test]
fn command_completions_offers_set_args_only_for_sets_own_second_token() {
    let candidates = command_completions("set v", 4, 5);
    assert_eq!(candidates, vec!["vim"]);
}

#[test]
fn command_completions_has_nothing_to_offer_for_an_arbitrary_argument() {
    // `:go`'s own URL argument — no finite candidate pool exists for
    // it, by design (see `command_completions`'s own docs).
    assert_eq!(command_completions("go ht", 3, 5), Vec::<&str>::new());
}

#[test]
fn command_completions_prefix_matches_only_not_substrings() {
    // "o" is a substring of "go" but not a prefix of it — must not
    // match.
    assert_eq!(command_completions("o", 0, 1), Vec::<&str>::new());
}

/// DIAGNOSTIC, not a real regression test (no behavior assertion
/// beyond "didn't run out of the deadline") — kept permanently,
/// `#[ignore]`d, as the measured evidence `measure_mixed_inline`'s
/// own docs cite for why it shapes a whole text item at once rather
/// than one `Buffer` per word. Compares the *same total text* shaped
/// two ways: as plain paragraphs, and as the same paragraphs each
/// with one small inline image dropped in the middle (real-world
/// trigger: Wikipedia's own inline icons). Run with `cargo test
/// --release mixed_inline_overhead_diagnostic -- --ignored --nocapture`.
#[test]
#[ignore]
fn mixed_inline_overhead_diagnostic() {
    let src = tiny_png_data_uri(4, 4);
    let words_per_paragraph = 40;
    let paragraph_count = 300;
    let word = "lorem ";

    let mut plain_html = String::from("<!doctype html><html><body>\n");
    let mut mixed_html = String::from("<!doctype html><html><body>\n");
    for _ in 0..paragraph_count {
        plain_html.push_str("<p>");
        mixed_html.push_str("<p>");
        for w in 0..words_per_paragraph {
            plain_html.push_str(word);
            mixed_html.push_str(word);
            if w == words_per_paragraph / 2 {
                mixed_html.push_str(&format!(r#"<img src="{src}" style="width: 12px; height: 12px">"#));
            }
        }
        plain_html.push_str("</p>\n");
        mixed_html.push_str("</p>\n");
    }
    plain_html.push_str("</body></html>");
    mixed_html.push_str("</body></html>");

    let time_it = |html: &str| -> std::time::Duration {
        let doc = parse_html(html);
        // Wide enough that a 20-word chunk (half of each paragraph,
        // split by the image at the midpoint) comfortably fits on
        // one line regardless of font metrics — isolating the
        // common "sparse icon in flowing prose" case from the rare
        // "chunk itself wraps" fallback this function's own docs
        // also handle.
        let tree = build_layout_tree(&doc, 5000.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let start = std::time::Instant::now();
        measure_document(&tree, &mut font_system, 5000.0, 5000.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let elapsed = start.elapsed();
        assert!(!incomplete, "diagnostic deadline too short");
        elapsed
    };

    let plain_time = time_it(&plain_html);
    let mixed_time = time_it(&mixed_html);
    eprintln!(
        "plain: {:?} ({} paragraphs, {} words each, no inline images)",
        plain_time, paragraph_count, words_per_paragraph
    );
    eprintln!(
        "mixed: {:?} ({} paragraphs, {} words each, one inline image per paragraph)",
        mixed_time, paragraph_count, words_per_paragraph
    );
    eprintln!("ratio (mixed / plain): {:.2}x", mixed_time.as_secs_f64() / plain_time.as_secs_f64());
}

/// DIAGNOSTIC, not a real regression test — kept permanently,
/// `#[ignore]`d, alongside `mixed_inline_overhead_diagnostic` as the
/// measured evidence for the same doc comment. Isolates `shape_
/// spans`'s own per-call overhead directly: N calls, one word each
/// (the shape of `measure_mixed_inline`'s old per-word path), versus
/// 1 call with all N words as a single span (its current whole-
/// chunk path) — same font system, same total text, same function.
/// Run with `cargo test --release
/// shape_spans_per_call_overhead_diagnostic -- --ignored --nocapture`.
#[test]
#[ignore]
fn shape_spans_per_call_overhead_diagnostic() {
    let word_count = 12_000; // matches 300 paragraphs * 40 words in the other diagnostic
    let span = SpanStyle::from(ROOT_STYLE);
    let mut font_system = new_font_system();

    let start = std::time::Instant::now();
    for _ in 0..word_count {
        let items = [InlineItem::Text("lorem ".to_string(), span)];
        shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
    }
    let per_word_time = start.elapsed();

    let one_big_text = "lorem ".repeat(word_count);
    let start = std::time::Instant::now();
    let items = [InlineItem::Text(one_big_text, span)];
    shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
    let one_call_time = start.elapsed();

    eprintln!("{word_count} separate 1-word shape_spans calls: {per_word_time:?}");
    eprintln!("1 shape_spans call with all {word_count} words: {one_call_time:?}");
    eprintln!("ratio: {:.2}x", per_word_time.as_secs_f64() / one_call_time.as_secs_f64());
}

/// Regression test for a real hang: a span with `line-height: 0`
/// (legitimate, real-world CSS — e.g. Wikipedia's own thumbnail-figure
/// styling) used to make `shape_spans`'s call to cosmic-text's
/// `Buffer::shape_until_scroll` loop forever, because cosmic-text
/// never returns for a zero line height. `shape_spans` now floors the
/// line height just above zero before handing it to cosmic-text. Runs
/// the call on a background thread with a watchdog so a regression
/// fails loudly (panic) instead of hanging the whole test suite.
#[test]
fn shape_spans_zero_line_height_does_not_hang() {
    let mut span = SpanStyle::from(ROOT_STYLE);
    span.line_height = 0.0;
    let items = [InlineItem::Text("Map".to_string(), span)];

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut font_system = new_font_system();
        shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("shape_spans hung with line_height == 0.0");
}

// ---------- `SCRATCHY_CSS_DEBUG` ----------

/// `parse_value`'s own before/after `Declarations` comparison relies
/// on `PartialEq` genuinely detecting "a real property changed" vs.
/// "nothing changed at all" — the one directly testable piece of
/// this feature on stable Rust; the env-var-gated `eprintln!` side
/// effect itself isn't practically capturable in-process the same
/// way `timing_enabled`'s own env-var gate has no test either.
#[test]
fn declarations_partial_eq_detects_a_real_difference_and_matches_when_unchanged() {
    let a = Declarations::default();
    let b = Declarations::default();
    assert!(a == b, "two freshly-defaulted Declarations should compare equal — the 'nothing changed' case `parse_value` reports on");
    let mut c = Declarations::default();
    c.display = Some(Display::Block);
    assert!(a != c, "a real property difference should make them compare unequal — the 'this one actually applied' case that must stay silent");
}

/// The real, end-to-end premise the before/after comparison above
/// stands in for, exercised through the actual production parser
/// (`parse_stylesheet`), not a hand-built `Declarations` — a
/// recognized-but-unsupported value shape (`grid-area`'s own
/// fuller numeric `<line> / <line> / <line> / <line>` form, not
/// parsed — see `parse_grid_area_name`'s own docs) leaves its field
/// untouched, while a supported declaration in the very same rule
/// still applies normally, confirming a real declaration-level
/// failure genuinely produces no change for `SCRATCHY_CSS_DEBUG`'s
/// own comparison to catch, not just the synthetic structs above.
#[test]
fn parse_stylesheet_leaves_an_unsupported_declaration_with_no_effect() {
    let rules = parse_stylesheet("div { color: red; grid-area: 1 / 2 / 3 / 4 / 5; }");
    assert_eq!(rules.len(), 1);
    assert!(rules[0].decls.color.is_some(), "the supported declaration in the same rule should still apply");
    assert!(
        rules[0].decls.grid_area.is_none(),
        "an invalid grid-area form (five lines) should leave the field unset, not guess at a value"
    );
}

/// Q1: duplicate-valued declarations stay silent; dropped selectors
/// and at-rules are reported.
#[test]
fn css_debug_distinguishes_duplicates_dropped_selectors_and_at_rules() {
    CSS_DEBUG_CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    parse_stylesheet(
        "p { color: red; color: red; frobnicate: 3; }\
         p::bogus { color: blue; }\
         @font-face { font-family: x; }",
    );
    let lines = CSS_DEBUG_CAPTURE.with(|c| c.borrow_mut().take().unwrap());
    assert!(lines.iter().any(|l| l == "scratchy: css: ignored `frobnicate: 3`"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("ignored `color")), "{lines:?}");
    assert!(lines.iter().any(|l| l == "scratchy: css: dropped selector `p::bogus`"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "scratchy: css: dropped @font-face"), "{lines:?}");
}

/// Q10: `@supports` conditions gate their block.
#[test]
fn q10_supports_conditions_gate_blocks() {
    let n = |css: &str| parse_stylesheet(css).len();
    assert_eq!(n("@supports (display:grid){p{color:red}}"), 1);
    assert_eq!(n("@supports (display:bogus){p{color:red}}"), 0);
    assert_eq!(n("@supports not (display:bogus){p{color:red}}"), 1);
    assert_eq!(n("@supports not (display:grid){p{color:red}}"), 0);
    assert_eq!(n("@supports (display:grid) and (color:red){p{color:red}}"), 1);
    assert_eq!(n("@supports (display:grid) and (display:bogus){p{color:red}}"), 0);
    assert_eq!(n("@supports (display:bogus) or (color:red){p{color:red}}"), 1);
    assert_eq!(n("@supports (display:bogus) or (display:grid) and (color:red){p{color:red}}"), 0);
    assert_eq!(n("@supports ((display:bogus) or (color:red)){p{color:red}}"), 1);
    assert_eq!(n("@supports selector(a > b){p{color:red}}"), 1);
    assert_eq!(n("@supports selector(a::bogus){p{color:red}}"), 0);
    assert_eq!(n("@supports foo(bar){p{color:red}}"), 0);
    assert_eq!(n("@supports garbage{p{color:red}}p{color:blue}"), 1);
    // Applies through the cascade.
    let r = parse_stylesheet("@supports (display:grid){p{color:red}}");
    assert!(r[0].decls.color.is_some());
}

#[test]
fn q12a_svg_decodes_to_pixels() {
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='red'/></svg>";
    let img = decode_image_bytes(svg).expect("svg decodes");
    assert_eq!((img.width, img.height), (10, 10));
    assert_eq!(img.pixels[55], TextColor::rgba(255, 0, 0, 255));
    // percent-encoded data URI, viewBox-only size
    let uri = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4 6'%3E%3Crect width='4' height='6' fill='%230f0'/%3E%3C/svg%3E";
    let img = decode_image_bytes(&decode_data_uri(uri).unwrap()).expect("uri svg decodes");
    assert_eq!((img.width, img.height), (4, 6));
    assert_eq!(img.pixels[0], TextColor::rgba(0, 255, 0, 255));
    assert!(decode_image_bytes(b"<svg").is_none());
}

#[test]
fn q12b_inline_svg_is_a_replaced_box_with_current_color() {
    let imgs = |html: &str| {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc, 400.0);
        fn collect(b: &LayoutBox, out: &mut Vec<Arc<DecodedImage>>) {
            match &b.content {
                BoxContent::Image(i) => out.push(i.clone()),
                BoxContent::Inline(items) => {
                    for i in items {
                        match i {
                            InlineItem::Image { image, .. } => out.push(image.clone()),
                            InlineItem::Embedded(b) => collect(b, out),
                            _ => {}
                        }
                    }
                }
                BoxContent::Blocks(bs) | BoxContent::TableRow(bs) => bs.iter().for_each(|b| collect(b, out)),
            }
        }
        let mut out = Vec::new();
        collect(&tree, &mut out);
        out
    };
    // Sized by its width/height attributes; the title text must not render.
    let v = imgs(r#"<html><body style="margin:0"><svg width="10" height="10" viewBox="0 0 10 10"><title>hi</title><rect width="10" height="10" fill="red"/></svg></body></html>"#);
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].width, v[0].height), (10, 10));
    assert_eq!(v[0].pixels[0], TextColor::rgba(255, 0, 0, 255));
    // currentColor follows the computed (inherited) color.
    let v = imgs(r#"<html><body style="margin:0;color:#0000ff"><svg width="4" height="4"><rect width="4" height="4" fill="currentColor"/></svg></body></html>"#);
    assert_eq!(v[0].pixels[0], TextColor::rgba(0, 0, 255, 255));
    // CSS size wins; block svg is its own box.
    let v = imgs(r#"<html><body style="margin:0"><svg viewBox="0 0 2 2" style="display:block;width:20px;height:20px"><rect width="2" height="2" fill="lime"/></svg></body></html>"#);
    assert_eq!((v[0].width, v[0].height), (20, 20));
}

#[test]
fn q12a2_svg_rasterizes_at_css_size() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10' fill='red'/></svg>";
    let uri = format!("data:image/svg+xml,{}", svg.replace('<', "%3C").replace('>', "%3E"));
    let img_of = |attrs: &str| {
        let html = format!(r#"<html><body style="margin:0"><img src="{uri}" {attrs}></body></html>"#);
        let doc = parse_html(&html);
        let tree = build_layout_tree(&doc, 400.0);
        fn find(b: &LayoutBox) -> Option<Arc<DecodedImage>> {
            match &b.content {
                BoxContent::Image(i) => Some(i.clone()),
                BoxContent::Inline(items) => items.iter().find_map(|i| match i {
                    InlineItem::Image { image, .. } => Some(image.clone()),
                    _ => None,
                }),
                BoxContent::Blocks(bs) | BoxContent::TableRow(bs) => bs.iter().find_map(find),
            }
        }
        find(&tree).expect("image box")
    };
    let a = img_of(r#"width="40" height="20""#);
    assert_eq!((a.width, a.height), (40, 20));
    assert_eq!(a.pixels[0], TextColor::rgba(255, 0, 0, 255));
    assert_eq!(a.pixels[40 * 20 - 1], TextColor::rgba(255, 0, 0, 255));
    // One dimension: the other follows the aspect ratio.
    let b = img_of(r#"width="30""#);
    assert_eq!((b.width, b.height), (30, 30));
    // No size: intrinsic.
    let c = img_of("");
    assert_eq!((c.width, c.height), (10, 10));
    // No width/height/viewBox: 300x150 fallback.
    let d = decode_image_bytes(b"<svg xmlns='http://www.w3.org/2000/svg'></svg>").expect("fallback size");
    assert_eq!((d.width, d.height), (300, 150));
}

#[test]
fn q11_calc_min_max_clamp() {
    let w = |v: &str, vw: f32| {
        let laid = measure_fixture(
            &format!(r#"<html><body style="margin:0"><div style="width:{v};height:7px">x</div></body></html>"#),
            vw,
        );
        find_by_height(&laid, 7.0).map(|l| l.w)
    };
    // Percentage resolved against the containing block.
    assert_eq!(w("calc(100% - 20px)", 200.0), Some(180.0));
    assert_eq!(w("calc(50% + 1em)", 200.0), Some(116.0));
    assert_eq!(w("calc(2 * 1em)", 200.0), Some(32.0));
    assert_eq!(w("calc((1px + 2px) * 3 - 4px / 2)", 200.0), Some(7.0));
    assert_eq!(w("calc(-1 * 8px + 10px)", 200.0), Some(2.0));
    assert_eq!(w("min(10px, 5px)", 200.0), Some(5.0));
    assert_eq!(w("max(10px, 5px, 1rem)", 200.0), Some(16.0));
    assert_eq!(w("min(100%, 150px)", 200.0), Some(150.0));
    assert_eq!(w("min(100%, 150px)", 100.0), Some(100.0));
    assert_eq!(w("clamp(50px, 10%, 100px)", 200.0), Some(50.0));
    assert_eq!(w("clamp(50px, 40%, 100px)", 200.0), Some(80.0));
    assert_eq!(w("clamp(50px, 90%, 100px)", 200.0), Some(100.0));
    // Invalid: division by zero, length + bare number, length * length.
    // The declaration drops, leaving the auto width (full container).
    assert_eq!(w("calc(10px / 0)", 200.0), Some(200.0));
    assert_eq!(w("calc(10px + 2)", 200.0), Some(200.0));
    assert_eq!(w("calc(10px * 2px)", 200.0), Some(200.0));
    assert_eq!(w("calc(10px / 2px)", 200.0), Some(200.0));
    assert_eq!(w("calc(1px + )", 200.0), Some(200.0));
    // Margins and font-size take it too.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="margin-left:calc(10% + 5px);height:7px">x</div></body></html>"#,
        200.0,
    );
    assert_eq!(find_by_height(&laid, 7.0).map(|l| l.x), Some(25.0));
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
    assert!(s.overflow == Overflow::Hidden);
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
    assert!(resolve_background_image(&p, &rules, 800.0, None).is_some());
    let rules = parse_stylesheet(&format!("div{{background-image:url({uri})}} p{{background-image:initial}}"));
    assert!(resolve_background_image(&p, &rules, 800.0, None).is_none());
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

// ---------- `pt` unit ----------

#[test]
fn pt_unit_on_a_box_length_converts_at_the_exact_css_96_72_ratio() {
    let doc = parse_html(r#"<html><body><div style="margin-top: 12pt">x</div></body></html>"#);
    let tree = build_layout_tree(&doc, 1920.0);
    let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
    assert_eq!(div.style.margin.top.px(), 16.0, "12pt should convert to exactly 16px (CSS's own fixed 96px-per-inch, 72pt-per-inch ratio)");
}

#[test]
fn pt_unit_on_font_size() {
    let laid = measure_fixture(r#"<html><body><div style="font-size: 12pt;">x</div></body></html>"#, 500.0);
    let div = find_by_laid_font_size(&laid, 16.0).expect("font-size: 12pt should resolve to exactly 16px, not be dropped as unsupported");
    let _ = div;
}

#[test]
fn pt_unit_on_line_height() {
    let laid = measure_fixture(r#"<html><body><div style="font-size: 20px; line-height: 24pt;">x</div></body></html>"#, 500.0);
    let div = find_by_laid_font_size(&laid, 20.0).expect("div not found");
    match div.style.line_height {
        LineHeight::Len(l) => assert_eq!(l.resolve(div.style.font_size, div.style.root_font_size), 32.0, "24pt should convert to exactly 32px"),
        _ => panic!("line-height: 24pt should parse as LineHeight::Len, not be dropped as unsupported"),
    }
}

// ---------- Q40a: WPT reftest runner ----------

#[test]
fn q40a_wpt_expectation_logic() {
    use wpt::{Status, compare_expectations, format_expected, images_match, parse_expected};
    assert!(images_match(&[0xFF112233, 1], &[0x00112233, 1]));
    assert!(!images_match(&[1, 2], &[1, 3]));
    assert!(!images_match(&[1], &[1, 1]));
    let exp = parse_expected("# c\nPASS a\nFAIL b\nFAIL c\n");
    let mut got = std::collections::BTreeMap::new();
    got.insert("a".to_string(), Status::Fail);
    got.insert("b".to_string(), Status::Pass);
    got.insert("c".to_string(), Status::Fail);
    got.insert("d".to_string(), Status::Pass);
    let ch = compare_expectations(&got, &exp);
    assert_eq!(ch.len(), 3);
    assert!(ch[0].starts_with("REGRESSION a"));
    assert!(ch[1].starts_with("NEWLY PASSING b"));
    assert!(ch[2].starts_with("UNRECORDED PASS d"));
    assert_eq!(parse_expected(&format_expected(&got)), got);
}

#[test]
fn q40a_wpt_runner_renders_match_mismatch_and_skips() {
    use wpt::{Outcome, Renderer, Status};
    let dir = std::env::temp_dir().join(format!("scratchy-q40a-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let w = |name: &str, body: &str| fs::write(dir.join(name), body).unwrap();
    w("ref.html", "<div style=\"width:50px;height:50px;background:#f00\"></div>");
    w("pass.html", "<link rel=match href=ref.html><div style=\"width:50px;height:25px;background:#f00;border-bottom:25px solid #f00\"></div>");
    w("fail.html", "<link rel=match href=ref.html><div style=\"width:60px;height:50px;background:#f00\"></div>");
    w("mm.html", "<link rel=mismatch href=ref.html><div style=\"width:60px;height:50px;background:#f00\"></div>");
    w("js.html", "<link rel=match href=ref.html><script>1</script>");
    let mut r = Renderer::new(&[]);
    assert_eq!(r.run_test(&dir.join("pass.html"), None), Outcome::Ran(Status::Pass));
    assert_eq!(r.run_test(&dir.join("fail.html"), None), Outcome::Ran(Status::Fail));
    assert_eq!(r.run_test(&dir.join("mm.html"), None), Outcome::Ran(Status::Pass));
    assert_eq!(r.run_test(&dir.join("js.html"), None), Outcome::Skip("script"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn q40b_wpt_strip_cdata_only_for_xhtml() {
    let src = "<style><![CDATA[ a{} ]]></style>".to_string();
    assert_eq!(wpt::strip_cdata(std::path::Path::new("t.xht"), src.clone()), "<style> a{} </style>");
    assert_eq!(wpt::strip_cdata(std::path::Path::new("t.html"), src.clone()), src);
}

#[test]
fn q40b_wpt_root_relative_reference() {
    use wpt::{Outcome, Renderer, Status};
    let root = std::env::temp_dir().join(format!("scratchy-q40b-{}", std::process::id()));
    fs::create_dir_all(root.join("css/reference")).unwrap();
    fs::create_dir_all(root.join("css/t")).unwrap();
    fs::write(root.join("css/reference/r.html"), "<div style=\"width:50px;height:50px;background:#f00\"></div>").unwrap();
    fs::write(
        root.join("css/t/t.html"),
        "<link rel=match href=\"/css/reference/r.html \"><div style=\"width:50px;height:50px;background:#f00\"></div>",
    )
    .unwrap();
    let mut r = Renderer::new(&[]);
    r.root = fs::canonicalize(&root).ok();
    assert_eq!(r.run_test(&root.join("css/t/t.html"), None), Outcome::Ran(Status::Pass));
    let _ = fs::remove_dir_all(&root);
}
