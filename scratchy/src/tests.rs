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
            match item {
                InlineItem::Text(_, s) => out.push(s.color),
                // QW3: `<button>` is an inline-block (embedded box).
                InlineItem::Embedded(e) => find_all_span_colors(e, out),
                _ => {}
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
    let (b, m, a) = (pos("<<"), pos("mid"), pos(">>!"));
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
    assert!(texts.iter().any(|(t, c)| t == "ello world" && *c != red), "{texts:?}");
    assert!(texts.iter().any(|(t, c)| t == "plain" && *c != red), "{texts:?}");
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
    assert!(find("c").is_some());
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
    assert_eq!(inline_text("hello wORLD it's", WhiteSpace::Normal, TextTransform::Capitalize), "Hello WORLD It's");
    assert_eq!(inline_text("a  b", WhiteSpace::Normal, TextTransform::Uppercase), "A B");
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
    assert!(texts.iter().any(|(t, _)| t == "LOUD"), "inherited into <b>: {texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "quiet"), "{texts:?}");

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
    assert_eq!(substitute_css_vars("\"var(--x)\" var(--x)", &vars).unwrap(), "\"var(--x)\"  BAD ");
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
    assert!(resolve_background_image(&p, &rules, 800.0, hs.custom_props, &HashMap::new()).is_some());
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
fn block_display_table_without_width_shrinks_to_fit() {
    // Q50 (W14): no float, no width -> shrink-to-fit, not container fill.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:600px"><table><tr><td>a</td><td>b</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 600.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    assert!(cs[0].w < 100.0, "auto-width table must shrink to its cells, w={}", cs[0].w);
}

#[test]
fn q51_table_width_includes_border_and_padding() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:400px"><table style="width:100%;border:5px solid black;padding:10px"><tr><td>a</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 400.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    assert_eq!(cs[0].w, 400.0, "table width:100% is its border box");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><table style="width:200px;border:5px solid black;padding:10px"><tr><td>a</td></tr></table></body></html>"#,
        700.0,
    );
    let t = find_by_width(&laid, 200.0).expect("table must be 200px wide including border+padding");
    assert_eq!(t.w, 200.0);
}

#[test]
fn shrink_to_fit_table_is_sum_of_column_maxima_so_no_cell_wraps() {
    // Q50: rows with different per-column widths must not make cells wrap.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:600px"><table><tr><td>Founded</td><td>1993</td></tr><tr><td>HQ</td><td>Santa Clara</td></tr></table></div></body></html>"#,
        700.0,
    );
    let container = find_by_width(&laid, 600.0).expect("container not found");
    let LaidContent::Blocks(cs) = &container.content else { panic!("expected Blocks") };
    let LaidContent::Blocks(rows) = &cs[0].content else { panic!("expected rows") };
    assert!(rows[0].h < 30.0, "first row must stay on one line, h={}", rows[0].h);
    assert!(rows[1].h < 30.0, "second row must stay on one line, h={}", rows[1].h);
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
fn qb7_text_plain_is_wrapped_in_an_escaped_pre() {
    let out = dispatch_wire_response(Some("text/plain; charset=utf-8".to_string()), b"a<b\n  c".to_vec()).unwrap();
    let html = String::from_utf8(out).unwrap();
    assert!(html.contains("a&lt;b\n  c</pre>"), "{html}");
    let out = dispatch_wire_response(Some("text/plain; charset=ISO-8859-1".to_string()), vec![b'a', 0xE9]).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("a\u{e9}</pre>"));
    let out = dispatch_wire_response(Some("text/plain".to_string()), b"\nx".to_vec()).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("<pre style=\"white-space:pre-wrap\">\n\nx"));
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
        WireFetchRaw::Responded { content_type, body, .. } => {
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
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), Some("p{margin:0}".to_string()));
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
    assert_eq!(fetch_stylesheets(&path, &base, &base, &res.stylesheets), "h1{color:red}\n");
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
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), Some("body{color:red}".to_string()));
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
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), None);
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
    let decoded = fetch_image_via_wire(&path, &url, &url).expect("decode");
    assert_eq!((decoded.width, decoded.height), (2, 2));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_image_via_wire_rejects_a_non_image_content_type() {
    let path = unique_test_socket_path("image-wrong-type");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello");
    let url = Url::parse("https://example.com/not-an-image").unwrap();
    assert!(fetch_image_via_wire(&path, &url, &url).is_none());
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
fn resolve_binding_maps_capital_g_and_home_end_to_scroll_extremes() {
    let n = VimMode::Normal;
    let m = ModifiersState::empty();
    assert_eq!(resolve_binding(&Key::Character("G".into()), m, n), Some(Action::ScrollBottom));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::End), m, n), Some(Action::ScrollBottom));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::Home), m, n), Some(Action::ScrollTop));
    assert_eq!(resolve_binding(&Key::Character("G".into()), m, VimMode::Command), None);
    // bare `g` alone is not a binding; it goes through `resolve_pending_g`.
    assert_eq!(resolve_binding(&Key::Character("g".into()), m, n), None);
}

#[test]
fn pending_g_double_g_scrolls_top_and_other_keys_clear() {
    let m = ModifiersState::empty();
    let g = Key::Character("g".into());
    assert_eq!(resolve_pending_g(false, &g, m), (None, true, true));
    assert_eq!(resolve_pending_g(true, &g, m), (Some(Action::ScrollTop), false, true));
    for other in [Key::Character("j".into()), Key::Named(NamedKey::Escape), Key::Character(":".into())] {
        assert_eq!(resolve_pending_g(true, &other, m), (None, false, false));
        assert_eq!(resolve_pending_g(false, &other, m), (None, false, false));
    }
    // `g`, `j`, `g` must not fire: the `j` cleared the flag.
    let (_, p, _) = resolve_pending_g(false, &g, m);
    let (_, p, _) = resolve_pending_g(p, &Key::Character("j".into()), m);
    assert_eq!(resolve_pending_g(p, &g, m), (None, true, true));
    // Ctrl-g is not `g`.
    assert_eq!(resolve_pending_g(false, &g, ModifiersState::CONTROL), (None, false, false));
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
    assert!(resolve_background_image(&p, &rules, 800.0, None, &HashMap::new()).is_some());
    let rules = parse_stylesheet(&format!("div{{background-image:url({uri})}} p{{background-image:initial}}"));
    assert!(resolve_background_image(&p, &rules, 800.0, None, &HashMap::new()).is_none());
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

#[test]
fn q40c_wpt_root_relative_stylesheet_and_about_blank_reference() {
    use wpt::{Outcome, Renderer, Status};
    let root = std::env::temp_dir().join(format!("scratchy-q40c-{}", std::process::id()));
    fs::create_dir_all(root.join("css/t")).unwrap();
    fs::create_dir_all(root.join("support")).unwrap();
    fs::write(root.join("support/s.css"), "div{width:50px;height:50px;background:#f00}").unwrap();
    fs::write(root.join("css/t/ref.html"), "<div style=\"width:50px;height:50px;background:#f00\"></div>").unwrap();
    // The root-relative stylesheet link must resolve inside the checkout.
    fs::write(root.join("css/t/t.html"), "<link rel=match href=ref.html><link rel=stylesheet href=\"/support/s.css\"><div></div>").unwrap();
    // `about:blank` is an empty white page: an empty body matches, a box does not.
    fs::write(root.join("css/t/blank-pass.html"), "<link rel=match href=\"about:blank\"><p></p>").unwrap();
    fs::write(root.join("css/t/blank-fail.html"), "<link rel=match href=\"about:blank\"><div style=\"width:5px;height:5px;background:#000\"></div>").unwrap();
    let mut r = Renderer::new(&[]);
    r.root = fs::canonicalize(&root).ok();
    assert_eq!(r.run_test(&root.join("css/t/t.html"), None), Outcome::Ran(Status::Pass));
    assert_eq!(r.run_test(&root.join("css/t/blank-pass.html"), None), Outcome::Ran(Status::Pass));
    assert_eq!(r.run_test(&root.join("css/t/blank-fail.html"), None), Outcome::Ran(Status::Fail));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn q41_br_forces_a_line_break() {
    let h = |body: &str| measure_fixture(&format!("<html><body style=\"margin:0\">{body}</body></html>"), 800.0).h;
    let one = h("<div>a b</div>");
    assert!(one > 0.0);
    // Plain text run.
    assert!((h("<div>a<br>b</div>") - 2.0 * one).abs() < 1.0, "a<br>b must be two lines");
    assert!((h("<div>a<br><br>b</div>") - 3.0 * one).abs() < 1.0, "double br leaves an empty line");
    // A trailing <br> does not add a line.
    assert!((h("<div>a<br></div>") - one).abs() < 1.0);
    // Mixed run (inline image forces the hand-rolled breaker).
    let src = tiny_png_data_uri(10, 10);
    let mixed_one = h(&format!("<div>a <img src=\"{src}\"> b</div>"));
    let mixed_two = h(&format!("<div>a <img src=\"{src}\"><br>b</div>"));
    assert!(mixed_two > mixed_one + 1.0, "br after an inline image breaks the line: {mixed_one} vs {mixed_two}");
    // Whitespace after a <br> never forms a line of its own.
    assert!((h("<div>a<br>\n</div>") - one).abs() < 1.0);
    assert!((h("<div>a<br> b</div>") - 2.0 * one).abs() < 1.0);
    // Inside an inline-block.
    let ib = h("<div><span style=\"display:inline-block\">a<br>b</span></div>");
    assert!(ib > one + 1.0, "br inside inline-block: {ib} vs {one}");
}

#[test]
fn q41a_br_trailing_space_and_display_block() {
    // Trailing spaces before a <br> do not count toward the line width, so
    // right-aligned text sits flush against the edge.
    let doc = |body: &str| format!("<html><body style=\"margin:0\">{body}</body></html>");
    let one = measure_fixture(&doc("<div>a b</div>"), 800.0).h;
    let tight = measure_fixture(&doc("<div style=\"width:40px\">aaaa   <br>b</div>"), 800.0).h;
    assert!((tight - 2.0 * one).abs() < 1.0, "spaces before br must not wrap an extra line: {tight}");
    // `display:block` on a <br> is still just a line break.
    let blk = measure_fixture(&doc("<div>a<br style=\"display:block\">b</div>"), 800.0).h;
    assert!((blk - 2.0 * one).abs() < 1.0, "display:block br: {blk}");
}

#[test]
fn q42_physical_and_font_relative_units() {
    let h = |style: &str| measure_fixture(&format!("<html><body style=\"margin:0\"><div style=\"{style}\"></div></body></html>"), 800.0).h;
    assert!((h("height:96px") - 96.0).abs() < 0.5);
    for (u, v) in [("1in", 96.0), ("2.54cm", 96.0), ("25.4mm", 96.0), ("6pc", 96.0), ("101.6Q", 96.0), ("1IN", 96.0)] {
        assert!((h(&format!("height:{u}")) - v).abs() < 0.5, "{u}");
    }
    assert!((h("height:calc(1in + 4px)") - 100.0).abs() < 0.5);
    // ex/ch fall back to half an em (font-size 20px -> 10px).
    assert!((h("font-size:20px;height:1ex") - 10.0).abs() < 0.5);
    assert!((h("font-size:20px;height:2ch") - 20.0).abs() < 0.5);
    // Negative padding is invalid and dropped (padding-bottom-056).
    assert!(h("padding-bottom:-1in").abs() < 0.5);
    assert!((h("padding:1in") - 192.0).abs() < 0.5);
}

#[test]
fn q43_shrink_to_fit_excludes_trailing_collapsible_space() {
    // Ahem (1em-wide squares) is only available next to a WPT checkout;
    // the trailing-space bug is font-independent, so fall back to a
    // relative comparison when it is missing.
    let ahem = std::path::PathBuf::from("/home/loupax/src/wpt/fonts/Ahem.ttf");
    let fonts: Vec<std::path::PathBuf> = if ahem.exists() { vec![ahem] } else { vec![] };
    let run = |body: &str| {
        let doc = parse_html(&format!("<html><body style=\"margin:0;font:20px Ahem\">{body}</body></html>"));
        let tree = build_layout_tree(&doc, 1920.0);
        let mut fs = new_font_system_with(&fonts);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure(&tree, &mut fs, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None)
    };
    fn collect(l: &Laid, out: &mut Vec<(f32, f32)>) {
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                out.push((k.x, k.w));
                collect(k, out);
            }
        }
    }
    let mut v = Vec::new();
    collect(&run("<div style=\"float:left\">X</div><div style=\"float:left\">XX</div>"), &mut v);
    let floats: Vec<_> = v.iter().filter(|(_, w)| *w < 700.0).collect();
    // Source whitespace/trailing collapsible space must not widen a float:
    // the second float starts exactly where the first ends.
    assert!((floats[0].0 - 0.0).abs() < 0.01);
    assert!((floats[2].0 - (floats[0].0 + floats[0].1)).abs() < 0.01, "{floats:?}");
    if !fonts.is_empty() {
        assert!((floats[0].1 - 20.0).abs() < 0.01, "{floats:?}");
        assert!((floats[2].1 - 40.0).abs() < 0.01, "{floats:?}");
    }
    // Inline-block, same.
    if !fonts.is_empty() {
        let laid = run("<div><span style=\"display:inline-block\">XX</span></div>");
        let items = mixed_items(find_mixed_inline(&laid).expect("mixed run"));
        assert!(
            items.iter().any(|m| matches!(m, MixedPositioned::Embedded(l) if (l.w - 40.0).abs() < 0.01)),
            "inline-block of XX at 20px should be 40px wide"
        );
    }
}

#[test]
fn q44_abspos_auto_width_is_shrink_to_fit() {
    let abs_w = |style: &str| {
        let laid = measure_fixture(
            &format!(r#"<html><body><div style="position: relative; width: 300px; height: 100px;"><div style="position: absolute; {style}">ab</div></div></body></html>"#),
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("container");
        container.positioned[0].w
    };
    let w = abs_w("left: 10px; top: 0;");
    assert!(w > 0.0 && w < 100.0, "left-only abspos shrinks to its text, got {w}");
    let w = abs_w("right: 10px; top: 0;");
    assert!(w > 0.0 && w < 100.0, "right-only abspos shrinks to its text, got {w}");
    let w = abs_w("top: 0;");
    assert!(w > 0.0 && w < 100.0, "static-position abspos shrinks to its text, got {w}");
    assert_eq!(abs_w("left: 10px; right: 20px; top: 0;"), 270.0, "left+right stretches");
    assert_eq!(abs_w("left: 10px; width: 50px; top: 0;"), 50.0, "explicit width wins");
}

#[test]
fn qa7_abspos_shrink_to_fit_counts_float_children() {
    let abs_w = |inner: &str, style: &str| {
        let laid = measure_fixture(
            &format!(r#"<html><body><div style="position: relative; width: 300px; height: 100px;"><div style="position: absolute; left: 0; top: 0; {style}">{inner}</div></div></body></html>"#),
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("container");
        container.positioned[0].w
    };
    let fl = r#"<div style="float:right;width:48px;height:12px"></div>"#;
    assert_eq!(abs_w(fl, ""), 48.0, "abspos holding only a 48px float is 48 wide");
    assert_eq!(
        abs_w(fl, "border-left:24px solid;border-right:24px solid;"),
        96.0,
        "borders add to the float's width"
    );
    let two = r#"<div style="float:left;width:30px;height:5px"></div><div style="float:left;width:20px;height:5px"></div>"#;
    assert_eq!(abs_w(two, ""), 50.0, "adjacent floats sum for max-content");
    let nested = r#"<div><div style="float:left;width:40px;height:5px"></div></div>"#;
    assert_eq!(abs_w(nested, ""), 40.0, "float inside a plain block child");
}

#[test]
fn abspos_box_contains_its_floats_including_bottom_margin() {
    // Q45 / W11: an abspos box is a BFC, so its auto height includes floats.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="position:relative; width:300px;"><div style="position:absolute; width:96px; background:black;"><div style="float:left; margin-bottom:48px; height:48px; width:100%;"></div></div></div></body></html>"#,
        400.0,
    );
    let outer = find_by_width(&laid, 300.0).expect("container not found");
    let c = &outer.positioned[0];
    assert_eq!(c.h, 96.0, "abspos auto height must include float and its bottom margin");
}

#[test]
fn q46_flex_item_with_explicit_width_takes_flexed_size() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; width: 300px;"><div style="width: 100px; flex-grow: 1;">a</div><div style="width: 100px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!(items[0].w, 200.0, "explicit width item must paint at the grown size");
    assert_eq!(items[1].x, 200.0);
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; width: 100px;"><div style="width: 100px; min-width: 0;">a</div><div style="width: 100px; min-width: 0;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 100.0);
    assert_eq!(items[0].w, 50.0);
    assert_eq!(items[1].w, 50.0);
}

#[test]
fn q46_flex_item_margins_count_against_free_space() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; width: 300px;"><div style="width: 100px; flex-grow: 1; margin-left: 10px; margin-right: 10px;">a</div><div style="width: 100px;">b</div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 300.0);
    assert_eq!((items[0].x, items[0].w, items[1].x, items[1].w), (10.0, 180.0, 200.0, 100.0));
}

#[test]
fn q47_single_line_flex_cross_size_is_container_height() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="display: flex; width: 100px; height: 100px;"><div style="align-self: flex-end; height: 50px; width: 25px;"></div><div style="align-self: center; height: 20px; width: 25px;"></div><div style="width: 25px;"></div></div></body></html>"#,
        500.0,
    );
    let items = flex_items(&laid, 100.0);
    assert_eq!((items[0].y, items[0].h), (50.0, 50.0));
    assert_eq!((items[1].y, items[1].h), (40.0, 20.0));
    assert_eq!((items[2].y, items[2].h), (0.0, 100.0));
}

fn q49_find(laid: &Laid, h: f32) -> Option<&Laid> {
    if laid.style.height.map(BoxLength::px) == Some(h) {
        return Some(laid);
    }
    match &laid.content {
        LaidContent::Blocks(cs) => cs.iter().find_map(|c| q49_find(c, h)),
        LaidContent::MixedInline(items) => items.iter().find_map(|i| match i {
            MixedPositioned::Embedded(l) => q49_find(l, h),
            _ => None,
        }),
        _ => None,
    }
}

fn q49_first_embedded(l: &Laid) -> Option<&Laid> {
    match &l.content {
        LaidContent::Blocks(cs) => cs.iter().find_map(q49_first_embedded),
        LaidContent::MixedInline(items) => items.iter().find_map(|i| match i {
            MixedPositioned::Embedded(e) => Some(&**e),
            _ => None,
        }),
        _ => None,
    }
}

// Q49: a float after inline content on the same line goes at the top of
// that line, and the inline content shifts beside it when it fits.
#[test]
fn q49_float_after_inline_block_goes_at_line_top_and_content_shifts() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:200px"><div style="display:inline-block;width:100px;height:96px"></div> <div style="float:left;width:100px;height:77px"></div></div></body></html>"#,
        500.0,
    );
    let f = q49_find(&laid, 77.0).expect("float");
    assert_eq!((f.x, f.y), (0.0, 0.0));
    let ib = q49_find(&laid, 96.0).expect("inline-block");
    assert_eq!((ib.x, ib.y), (100.0, 0.0));
}

#[test]
fn q49_float_that_does_not_fit_beside_the_line_stays_below() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:150px"><div style="display:inline-block;width:100px;height:96px"></div> <div style="float:left;width:100px;height:77px"></div></div></body></html>"#,
        500.0,
    );
    let f = q49_find(&laid, 77.0).expect("float");
    assert_eq!((f.x, f.y), (0.0, 96.0));
    let ib = q49_find(&laid, 96.0).expect("inline-block");
    assert_eq!((ib.x, ib.y), (0.0, 0.0));
}

#[test]
fn q49_float_after_plain_text_goes_at_line_top() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:300px">some text <div style="float:left;width:50px;height:77px"></div></div></body></html>"#,
        500.0,
    );
    let f = q49_find(&laid, 77.0).expect("float");
    assert_eq!((f.x, f.y), (0.0, 0.0));
}

#[test]
fn q49_leading_and_trailing_markup_space_does_not_push_float_down() {
    let laid = measure_fixture(
        r#"<html><head><style>#div1{border:solid 5px black;height:1in;width:2in} div div{display:inline-block;height:1in;width:1in} #div3{background-color:blue;float:left}</style></head><body style="margin:0"><div id="div1">
           <div id="div2"></div>
           <div id="div3"></div>
        </div></body></html>"#,
        800.0,
    );
    // Markup whitespace around the inline-block must not widen line 1.
    let ib = q49_first_embedded(&laid).expect("inline-block");
    assert_eq!((ib.x, ib.y), (101.0, 5.0));
}

// Q48: vertical margins collapse through an empty block (CSS2.1 8.3.1).
fn q48_y_of(html: &str, h: f32) -> f32 {
    let laid = measure_fixture(html, 900.0);
    find_by_height(&laid, h).expect("marker div not found").y
}

#[test]
fn margin_collapses_through_empty_auto_block() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px"></div><div style="margin-bottom:50px"></div><div></div><div style="margin-top:30px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 60.0);
}

#[test]
fn margin_collapses_through_zero_height_block() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px"></div><div style="margin-bottom:50px"></div><div style="height:0"></div><div style="margin-top:30px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 60.0);
}

#[test]
fn margin_collapses_through_empty_block_own_margins() {
    // 20, 40, 10, 30 collapse to 40.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px"></div><div style="margin-bottom:20px"></div><div style="margin:40px 0 10px"></div><div style="margin-top:30px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 50.0);
}

#[test]
fn margin_collapse_through_mixed_signs_combines_max_pos_and_min_neg() {
    // Acid2 .empty: 75, -72, 75 collapse to 75 + -72 = 3 (not 78).
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px"></div><div style="margin:75px 0 -72px"></div><div style="margin-top:75px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 13.0);
}

#[test]
fn clearance_prevents_collapse_through_empty_block() {
    // A cleared empty block is not collapsed through: its margins stay
    // separate from the next sibling's.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;height:50px;width:10px"></div><div style="clear:left;margin-bottom:20px"></div><div style="margin-top:30px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 80.0);
}

#[test]
fn padding_blocks_margin_collapse_through() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px"></div><div style="margin-bottom:50px"></div><div style="padding-top:1px"></div><div style="margin-top:30px;height:77px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 91.0);
}

// Q53: negative values for non-negative-only properties drop the declaration.
#[test]
fn q53_negative_sizes_are_dropped() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="margin-top:1px;height:40px;max-height:-1px;min-height:-3px;padding-right:-1px;border-top-width:-4px;border-top-style:solid"></div><div style="margin-top:2px;width:-10px;height:7px"></div></body></html>"#,
        500.0,
    );
    let a = find_laid_by_margin_top(&laid, 1.0).expect("a");
    assert_eq!(a.w, 500.0);
    // The dropped `border-top-width:-4px` leaves the initial `medium` (3px) now that
    // a style is set (QA11a), which the border-box height includes.
    assert_eq!(a.style.border_width[0], 3.0, "border-top-width:-4px dropped -> medium");
    assert_eq!(a.h, 43.0, "max-height:-1px dropped; 40px content + 3px medium border");
    let b = find_laid_by_margin_top(&laid, 2.0).expect("b");
    assert_eq!(b.w, 500.0, "width:-10px dropped -> auto");
}

// Q54: width:auto with borders+padding wider than the container overflows at full border width.
#[test]
fn q54_overconstrained_auto_width_overflows() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:48px"><div style="border-left:32px solid;border-right:32px solid;height:9px"></div></div></body></html>"#,
        500.0,
    );
    let d = find_by_height(&laid, 9.0).expect("inner");
    assert_eq!(d.w, 64.0);
}

// Q55 (W17): a negative-z-index abspos child of a positioned parent with
// z-index:auto (not a stacking context) paints below that parent's
// background; with z-index:0 (a stacking context) it paints above it.
#[test]
fn q55_negative_z_child_of_non_stacking_parent_paints_below_parent_background() {
    let auto = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(auto);
    assert_eq!(px_at(&pixels, win_w, 50, 50), 0x0000_FF00, "z-auto parent: child goes below parent bg");
    let sc = auto.replace("position: relative;", "position: relative; z-index: 0;");
    let (pixels, win_w, _) = paint_fixture(&sc);
    assert_eq!(px_at(&pixels, win_w, 50, 50), 0x00FF_0000, "z-index:0 parent: child above parent bg");
    // Nested abspos chain (WPT fixed-pos-stacking-001 shape).
    let nested = r#"<html><body style="margin:0"><div style="position: absolute; left: 0px; top: 0px; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; width: 50px; height: 50px;"><div style="position: absolute; z-index: -1; width: 50px; height: 50px; background-color: #ff0000;"></div></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(nested);
    assert_eq!(px_at(&pixels, win_w, 20, 20), 0x0000_FF00, "nested non-SC chain: negative child below ancestor bg");
}

// QA9 (R17): z-index orders stacking contexts across parents. A
// `position:relative; z-index:2` box nested in a static wrapper must paint
// above an unrelated later `position:fixed` (z auto) box.
#[test]
fn qa9_z_indexed_relative_paints_above_unrelated_fixed() {
    let html = r#"<html><body style="margin:0"><div><div style="position: relative; z-index: 2; width: 100px; height: 100px; background-color: #00ff00;"></div></div><div style="position: fixed; top: 0px; left: 0px; width: 60px; height: 60px; background-color: #000000;"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    assert_eq!(px_at(&pixels, win_w, 30, 30), 0x0000_FF00, "z-index:2 lime wins over later fixed z:auto black");
    // Without the z-index the later fixed box wins (normal order).
    let (pixels, win_w, _) = paint_fixture(&html.replace("z-index: 2;", ""));
    assert_eq!(px_at(&pixels, win_w, 30, 30), 0x0000_0000, "no z-index: fixed black on top");
    // A positive-z abspos nested in a z:auto relative parent escapes to the
    // root context and beats a later positioned box with a lower z-index.
    let nested = r#"<html><body style="margin:0"><div style="position: relative;"><div style="position: absolute; z-index: 5; top: 0px; left: 0px; width: 50px; height: 50px; background-color: #00ff00;"></div></div><div style="position: absolute; z-index: 1; top: 0px; left: 0px; width: 50px; height: 50px; background-color: #000000;"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(nested);
    assert_eq!(px_at(&pixels, win_w, 20, 20), 0x0000_FF00, "z:5 abspos nested in z-auto parent beats z:1 elsewhere");
}

// QA9 / Q55a: a negative-z in-flow relative child of a non-stacking
// positioned parent paints below that parent's background.
#[test]
fn qa9_negative_z_relative_child_below_non_stacking_parent_background() {
    let html = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: relative; z-index: -1; width: 100px; height: 100px; background-color: #ff0000;"></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    assert_eq!(px_at(&pixels, win_w, 50, 50), 0x0000_FF00, "z:-1 relative child under z-auto parent bg");
}

// QY1 (R20): one stacking order per stacking context, tree-order tie-break.
// An earlier `position:absolute` z-auto box must not cover a later
// positioned box (the YouTube `ytd-app` vs skeleton shape).
fn qy1_repro(second_style: &str) -> String {
    format!(
        r#"<html><body style="margin:0"><div style="position:absolute;top:0;left:0;right:0;min-height:100%;background:#ffff00"></div><div style="{second_style};background:#808080;height:100px;margin-top:56px;cursor:pointer"></div></body></html>"#
    )
}

#[test]
fn qy1_later_positioned_box_paints_over_earlier_abspos_sheet() {
    for second in ["position:relative;z-index:0", "position:relative;z-index:1", "position:relative"] {
        let (pixels, win_w, _) = paint_fixture(&qy1_repro(second));
        assert_eq!(px_at(&pixels, win_w, 50, 100), 0x0080_8080, "gray wins for {second}");
        assert_eq!(px_at(&pixels, win_w, 50, 20), 0x00FF_FF00, "yellow above the bar for {second}");
    }
    // Reverse order: relative first, abspos second: abspos wins.
    let rev = r#"<html><body style="margin:0"><div style="position:relative;z-index:0;background:#808080;height:100px"></div><div style="position:absolute;top:0;left:0;right:0;height:100px;background:#ffff00"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(rev);
    assert_eq!(px_at(&pixels, win_w, 50, 50), 0x00FF_FF00, "later abspos paints over earlier relative");
    // An explicit higher z-index still beats a later z-auto abspos.
    let hi = r#"<html><body style="margin:0"><div style="position:relative;z-index:3;background:#808080;height:100px"></div><div style="position:absolute;top:0;left:0;right:0;height:100px;background:#ffff00"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(hi);
    assert_eq!(px_at(&pixels, win_w, 50, 50), 0x0080_8080, "z:3 beats later z-auto abspos");
}

#[test]
fn qy1_hit_test_follows_paint_order() {
    let html = qy1_repro("position:relative;z-index:0");
    let doc = parse_html(&html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let so = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, 50.0, 100.0, &cache, &so, 0.0, 200.0).0, CursorStyle::Pointer);
    assert_ne!(hit_test_cursor(&laid, 50.0, 20.0, &cache, &so, 0.0, 200.0).0, CursorStyle::Pointer);
}

// QA9/R17 Acid2 shape: `html{overflow:hidden}` is propagated to the viewport
// and must not turn `<html>` into a clipping stacking boundary that traps
// the z-indexed intro away from the root-level fixed bar.
#[test]
fn qa9_html_overflow_hidden_does_not_trap_z_indexed_box() {
    let html = r#"<html style="overflow:hidden"><body style="margin:0"><div style="position:relative;z-index:2;width:100px;height:100px;background:#00ff00"></div><div><div style="position:fixed;top:0;left:0;width:60px;height:60px;background:#000000"></div></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    assert_eq!(px_at(&pixels, win_w, 30, 30), 0x0000_FF00, "z:2 intro covers the fixed bar even with html overflow hidden");
}

// QY1 / R20 (YouTube): `body{overflow-y:scroll}` is propagated to the viewport
// too, so an earlier abspos sheet must not cover a later z-indexed box inside body.
#[test]
fn qy1_body_overflow_scroll_does_not_trap_later_positioned_box() {
    let html = r#"<html><body style="margin:0;overflow-y:scroll"><div style="position:absolute;top:0;left:0;right:0;min-height:100%;background:#ffff00"></div><div style="position:relative;z-index:0;background:#808080;height:100px;margin-top:56px"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    assert_eq!(px_at(&pixels, win_w, 50, 100), 0x0080_8080, "gray skeleton wins over the earlier abspos sheet");
}

// ---------- Q57: var() correctness ----------

fn q57_color(css: &str) -> TextColor {
    let html = format!("<html><head><style>body{{color:#008000}} {css}</style></head><body><p>hi</p></body></html>");
    let doc = parse_html(&html);
    let tree = build_layout_tree(&doc, 1920.0);
    find_first_span_color(&tree).expect("no span found")
}

#[test]
fn q57_var_correctness() {
    let green = TextColor::rgb(0, 0x80, 0);
    // (a) invalid custom property values are dropped at parse.
    assert_eq!(q57_color("p{--a:#008000;--b:#dc143c;--a:var(--b,!);color:var(--a)}"), green);
    assert_eq!(q57_color("p{--a:#008000;--b:#dc143c;--a:var(--b) !important !important;color:var(--a)}"), green);
    assert_eq!(q57_color("p{--a:#008000;--b:#dc143c;--a:var(--b,;);color:var(--a)}"), green);
    // (b) invalid at computed-value time is unset (inherits from body), not dropped.
    assert_eq!(q57_color("p{color:#ffa500;--a:red;--a:var(--b) <!--;color:var(--a)}"), green);
    assert_eq!(q57_color("p{color:#ffa500;color:var(--nope)}"), green);
    // (c) escaped names.
    assert_eq!(q57_color("p{color:#ffa500;--0:#008000;color:var(--\\30)}"), green);
    // (d) adjacent substitutions do not fuse into one token.
    assert_eq!(q57_color("p{color:#ffa500;--a:var(--b)var(--c);--b:orange;--c:red;color:var(--a)}"), green);
    // @supports conditions containing var().
    assert_eq!(q57_color("p{color:#ff0000} @supports (--a: a) and (not (--a: var(--b,!))){p{color:#008000}}"), green);
}

// ---------- Q58: multi-value box shorthands ----------

fn q58_style(css: &str) -> ComputedStyle {
    let doc = parse_html(&format!(r#"<html><body><div style="{css}">x</div></body></html>"#));
    let tree = build_layout_tree(&doc, 1920.0);
    // The div is the only box with any non-zero margin/padding/offset.
    fn find(b: &LayoutBox) -> Option<ComputedStyle> {
        let s = &b.style;
        if s.margin.top.px() != 0.0
            || s.margin.right.px() != 0.0
            || s.margin.bottom.px() != 0.0
            || s.margin.left.px() != 0.0
            || s.padding.top.px() != 0.0
            || s.padding.right.px() != 0.0
            || s.padding.bottom.px() != 0.0
            || s.padding.left.px() != 0.0
        {
            // Prefer a descendant match (body has its own default margin).
            if let BoxContent::Blocks(c) = &b.content
                && let Some(d) = c.iter().find_map(find)
            {
                return Some(d);
            }
            return Some(s.clone());
        }
        match &b.content {
            BoxContent::Blocks(c) => c.iter().find_map(find),
            _ => None,
        }
    }
    find(&tree).unwrap_or_else(|| tree.style.clone())
}

#[test]
fn q58_margin_three_values() {
    let s = q58_style("margin:5px 30px 7px");
    assert_eq!(
        (s.margin.top.px(), s.margin.right.px(), s.margin.bottom.px(), s.margin.left.px()),
        (5.0, 30.0, 7.0, 30.0)
    );
}

#[test]
fn q58_padding_two_and_four_values_and_negative_rejected() {
    let s = q58_style("padding:1px 2px");
    assert_eq!((s.padding.top.px(), s.padding.right.px(), s.padding.bottom.px(), s.padding.left.px()), (1.0, 2.0, 1.0, 2.0));
    let s = q58_style("padding:1px 2px 3px 4px");
    assert_eq!((s.padding.top.px(), s.padding.right.px(), s.padding.bottom.px(), s.padding.left.px()), (1.0, 2.0, 3.0, 4.0));
    let s = q58_style("padding:1px -2px");
    assert_eq!(s.padding.right.px(), 0.0);
}

#[test]
fn q58_logical_pairs() {
    let s = q58_style("margin-block:3px 9px;padding-inline:4px 6px");
    assert_eq!((s.margin.top.px(), s.margin.bottom.px()), (3.0, 9.0));
    assert_eq!((s.padding.left.px(), s.padding.right.px()), (4.0, 6.0));
}

// ---------- Q59p: horizontal `margin: auto` ----------

fn find_box_at(laid: &Laid, x: f32, w: f32) -> Option<&Laid> {
    if laid.w == w && laid.x == x {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        return children.iter().find_map(|c| find_box_at(c, x, w));
    }
    None
}

#[test]
fn auto_margins_centre_and_right_align_blocks_and_images() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 50.0, 100.0).is_some(), "width box centred");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:100px;margin-left:auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 100.0, 100.0).is_some(), "right-aligned");
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="max-width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 50.0, 100.0).is_some(), "max-width box centred");
    let src = tiny_png_data_uri(40, 20);
    let html = format!(r#"<html><body style="margin:0"><img src="{src}" style="display:block;margin:0 auto"></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    let img = find_image_laid(&laid).expect("img");
    assert_eq!((img.x, img.w), (80.0, 40.0), "block img centred");
    // Floats keep auto margins at 0.
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="float:left;width:100px;margin:0 auto">a</div></body></html>"#,
        200.0,
    );
    assert!(find_box_at(&laid, 0.0, 100.0).is_some(), "float not centred");
}

#[test]
fn q59_block_img_with_explicit_width_centres_with_auto_margins() {
    // Regression: auto margins on a block <img> with a width were resolved
    // twice and the second pass zeroed them (block-replaced-width-006).
    let src = tiny_png_data_uri(15, 15);
    for (body, want_x) in [
        (format!(r#"<div style="width:200px"><img src="{src}" width="50%" style="display:block;margin-left:auto;margin-right:auto"></div>"#), 50.0),
        (format!(r#"<div style="width:200px"><img src="{src}" width="100" style="display:block;margin:0 auto"></div>"#), 50.0),
        (format!(r#"<img src="{src}" width="100" style="display:block;margin:0 auto">"#), 350.0),
    ] {
        let laid = measure_fixture(&format!(r#"<html><body style="margin:0">{body}</body></html>"#), 800.0);
        let img = find_image_laid(&laid).expect("img");
        assert_eq!((img.x, img.w), (want_x, 100.0));
    }
}

#[test]
fn q59_text_node_trailing_space_only_where_source_had_whitespace() {
    let ahem = std::path::PathBuf::from("/home/loupax/src/wpt/fonts/Ahem.ttf");
    let fonts: Vec<std::path::PathBuf> = if ahem.exists() { vec![ahem] } else { vec![] };
    fn collect(l: &Laid, out: &mut Vec<f32>) {
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                out.push(k.w);
                collect(k, out);
            }
        }
    }
    let float_w = |inner: &str| -> f32 {
        let doc = parse_html(&format!(
            "<html><body style=\"margin:0;font:20px Ahem\"><div style=\"float:left\">{inner}</div></body></html>"
        ));
        let tree = build_layout_tree(&doc, 1920.0);
        let mut fs = new_font_system_with(&fonts);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        let mut ws = Vec::new();
        collect(&laid, &mut ws);
        ws.into_iter().filter(|w| *w < 700.0).fold(0.0, f32::max)
    };
    let tight = float_w("a<span>b</span>");
    let spaced = float_w("a <span>b</span>");
    assert!(spaced > tight + 1.0, "real space preserved: {tight} vs {spaced}");
    // Punctuation after an element adds no gap.
    assert!(float_w("<b>x</b>,") <= float_w("<b>x</b> ,") - 1.0);
    // Collapsing across node boundaries: one space, not two.
    assert!((float_w("a <span> b</span>") - spaced).abs() < 0.01);
    if !fonts.is_empty() {
        assert!((tight - 40.0).abs() < 0.01, "{tight}");
        assert!((spaced - 60.0).abs() < 0.01, "{spaced}");
    }
}

// Q48a: adjacent in-flow sibling vertical margins collapse (CSS2.1 8.3.1).
fn q48a_pair(a: &str, b: &str) -> f32 {
    q48_y_of(
        &format!(r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:{a}"></div><div style="height:77px;margin-top:{b}"></div></body></html>"#),
        77.0,
    )
}

#[test]
fn sibling_margins_collapse_positive() {
    assert_eq!(q48a_pair("20px", "30px"), 40.0);
    assert_eq!(q48a_pair("30px", "20px"), 40.0);
}

#[test]
fn sibling_margins_collapse_negative() {
    assert_eq!(q48a_pair("-20px", "-30px"), 10.0 - 30.0);
}

#[test]
fn sibling_margins_collapse_mixed() {
    // 30 + (-10) = 20
    assert_eq!(q48a_pair("30px", "-10px"), 30.0);
    assert_eq!(q48a_pair("-10px", "30px"), 30.0);
}

#[test]
fn sibling_margins_percent_top_collapses() {
    // 10% of 900px = 90 collapses with 20 -> 90
    assert_eq!(q48a_pair("20px", "10%"), 100.0);
}

#[test]
fn float_between_siblings_does_not_break_or_join_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:20px"></div><div style="float:left;width:5px;height:5px;margin:100px"></div><div style="height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
}

#[test]
fn abspos_between_siblings_is_transparent_to_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="height:10px;margin-bottom:20px"></div><div style="position:absolute;height:5px;margin:100px"></div><div style="height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
}

#[test]
fn clearance_prevents_sibling_margin_collapse() {
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:50px"></div><div style="height:10px;margin-bottom:20px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    // Q48d: clearance puts the cleared box's BORDER edge at the float's
    // bottom (50); its margin-top is absorbed, not added on top.
    assert_eq!(y, 50.0);
}

#[test]
fn q48d_clear_without_clearance_still_collapses_and_clear_on_first_child() {
    // Float only 5px tall: the collapsed hypothetical border edge (10 + max(20,30)
    // = 40) is already below it, so no clearance is added.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:5px"></div><div style="height:10px;margin-bottom:20px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 40.0);
    // First in-flow child cleared past a 50px float with margin-top 30: border edge at 50.
    let y = q48_y_of(
        r#"<html><body style="margin:0"><div style="float:left;width:5px;height:50px"></div><div style="clear:left;height:77px;margin-top:30px"></div></body></html>"#,
        77.0,
    );
    assert_eq!(y, 50.0);
}

#[test]
fn q52a_iframe_is_a_replaced_box_default_300x150_honouring_attrs_and_css() {
    fn collect(l: &Laid, out: &mut Vec<(f32, f32)>) {
        out.push((l.w, l.h));
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                collect(k, out);
            }
        }
    }
    let sizes = |html: &str| -> Vec<(f32, f32)> {
        let doc = parse_html(&format!("<html><body style=\"margin:0\">{html}</body></html>"));
        let tree = build_layout_tree(&doc, 1000.0);
        let mut fs = new_font_system_with(&[]);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 1000.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        let mut v = Vec::new();
        collect(&laid, &mut v);
        v
    };
    assert!(sizes("<iframe style=\"display:block\"></iframe>").contains(&(300.0, 150.0)));
    assert!(sizes("<iframe style=\"display:block\" width=\"200\" height=\"80\"></iframe>").contains(&(200.0, 80.0)));
    assert!(sizes("<iframe style=\"display:block;width:120px\" width=\"200\" height=\"80\"></iframe>").contains(&(120.0, 80.0)));
    let v = sizes("<iframe>hello world fallback</iframe>");
    assert!(v.iter().any(|&(_, h)| h == 150.0), "{v:?}");
}

// ---------- Q33a: running scripts ----------

/// Serializes tests that flip the process-wide JS flag.
static JS_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_collecting(scripts: &[ScriptSource]) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_with(scripts, Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

fn src(code: &str) -> ScriptSource {
    ScriptSource { name: "t".into(), code: code.into(), index: None, ..Default::default() }
}

#[test]
fn q33a_scripts_run_in_order_then_dom_content_loaded_then_load() {
    let log = run_collecting(&[
        src("console.log('a'); document.addEventListener('DOMContentLoaded', function(){ console.log('dcl:' + document.readyState); });"),
        src("window.addEventListener('load', function(){ console.log('load:' + document.readyState); }); window.onload = function(){ console.log('onload'); }; console.log('b');"),
    ]);
    assert_eq!(log, ["log:a", "log:b", "log:dcl:interactive", "log:load:complete", "log:onload"]);
}

#[test]
fn q33a_script_errors_are_logged_and_not_fatal() {
    let log = run_collecting(&[
        src("throw new Error('boom');"),
        src("syntax error here ("),
        src("Promise.resolve().then(function(){ console.log('job'); }); console.warn('still', 1, {x: 2});"),
    ]);
    assert!(log[0].starts_with("error:t: Error: boom"), "{log:?}");
    assert!(log[1].starts_with("error:t: "), "{log:?}");
    assert_eq!(&log[2..], ["warn:still 1 {\"x\":2}", "log:job"]);
}

#[test]
fn qj1_classic_scripts_run_in_sloppy_mode() {
    let log = run_collecting(&[
        src("x = 1; console.log(typeof x)"),
        src("console.log((function(){ return this === globalThis })())"),
        src("\"use strict\"; y = 1"),
    ]);
    assert_eq!(&log[..2], ["log:number", "log:true"], "{log:?}");
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(log[2].starts_with("error:t: "), "{log:?}");
}

#[test]
fn qj2_error_lines_carry_name_message_and_script_number() {
    let log = run_collecting(&[
        src("window.addEventListener('load', function(){ throw new TypeError('boom'); });"),
        ScriptSource { name: "inline script #2".into(), code: "throw new RangeError('r');".into(), index: None, ..Default::default() },
        src("throw 'plain'"),
        src("throw {a: 1}"),
    ]);
    assert!(log.iter().any(|l| l.contains("Uncaught TypeError: boom")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("inline script #2: RangeError: r") && l.contains("inline script #2:1")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("t: Uncaught plain")), "{log:?}");
    assert!(log.iter().all(|l| !l.contains("Value(")), "{log:?}");
}

#[test]
fn qj2_inline_scripts_are_numbered_by_document_position() {
    let doc = parse_html("<script src=a.js></script><script>1</script><script>2</script>");
    let names: Vec<_> = inline_scripts(&doc).into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["inline script #2", "inline script #3"]);
}

#[test]
fn q33a_runaway_script_is_interrupted() {
    let start = std::time::Instant::now();
    let log = run_collecting(&[src("while(true){}"), src("console.log('after');")]);
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    assert!(log[0].starts_with("error:js: t stopped"), "{log:?}");
}

#[test]
fn q33a_classic_script_types_and_collection_order() {
    let doc = parse_html(
        "<html><head><script>var a=1;</script><script src=\"x.js\"></script>\
         <script type=\"module\">import 'z';</script><script type=\"application/json\">{}</script>\
         <script type=\"text/javascript\">var b=2;</script><script> </script></head></html>",
    );
    let mut res = ExternalResources::default();
    find_external_resources(&doc, &mut res);
    assert_eq!(
        res.scripts,
        [
            ScriptRef::Inline("var a=1;".into()),
            ScriptRef::External("x.js".into()),
            // QY15: module scripts are collected now.
            ScriptRef::ModuleInline("import 'z';".into()),
            ScriptRef::Inline("var b=2;".into())
        ]
    );
    assert_eq!(inline_scripts(&doc).len(), 3);
}

#[test]
fn q33a_no_js_flag_and_set_command_toggle() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(SET_ARGS.contains(&"js") && SET_ARGS.contains(&"nojs"));
    set_js_enabled(false);
    assert!(!js_enabled());
    let base = Url::parse("http://example.test/").unwrap();
    assert!(fetch_scripts(std::path::Path::new("/nonexistent"), &base, &base, vec![ScriptRef::Inline("1".into())]).is_empty());
    set_js_enabled(true);
    assert!(js_enabled());
    assert_eq!(fetch_scripts(std::path::Path::new("/nonexistent"), &base, &base, vec![ScriptRef::Inline("1".into())]).len(), 1);
}

#[test]
fn q33a_noscript_content_hidden_only_when_js_is_on() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fn has_size(l: &Laid, w: f32, h: f32) -> bool {
        (l.w == w && l.h == h) || matches!(&l.content, LaidContent::Blocks(c) if c.iter().any(|k| has_size(k, w, h)))
    }
    let visible = |js: bool| -> bool {
        set_js_enabled(js);
        let doc = parse_html("<html><body style=\"margin:0\"><noscript><div style=\"width:123px;height:45px\"></div></noscript></body></html>");
        let tree = build_layout_tree(&doc, 1000.0);
        let mut fs = new_font_system_with(&[]);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 1000.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        has_size(&laid, 123.0, 45.0)
    };
    let (on, off) = (visible(true), visible(false));
    set_js_enabled(true);
    assert!(!on, "noscript content must be hidden with JS on");
    assert!(off, "noscript content must show with JS off");
}

#[test]
fn qj3_js_report_counts_scripts_and_errors() {
    let html = "<html><body><script>console.log('ok');</script><script>throw new Error('boom');</script></body></html>";
    let report = js::js_report_for_html(html, None, "none.html");
    assert!(report.contains("scripts run: 2\n"), "{report}");
    assert!(report.contains("errors total: 1\n"), "{report}");
    assert!(report.contains("boom"), "{report}");
}

// ---------- Q33b: DOM read API ----------

fn run_in_html(html: &str, code: &str) -> Vec<String> {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_in(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

#[test]
fn q33b_get_element_by_id_and_attributes() {
    let log = run_in_html(
        "<html><body><div id=a class='x y' data-k=v>hi <b>there</b></div></body></html>",
        "var e=document.getElementById('a');\
         console.log(e.tagName, e.id, e.className, e.getAttribute('data-k'), e.hasAttribute('nope'), e.getAttribute('nope'), e.textContent);\
         console.log(document.getElementById('zz'), e===document.getElementById('a'), e.classList.contains('y'), e.classList.contains('q'));",
    );
    assert_eq!(log, ["log:DIV a x y v false null hi there", "log:null true true false"], "{log:?}");
}

#[test]
fn q33b_query_selector_and_tree_walk() {
    let log = run_in_html(
        "<html><head></head><body><ul id=l><li class=i>1</li><li>2</li><li class=i>3</li></ul></body></html>",
        "var l=document.querySelector('#l');\
         console.log(document.querySelectorAll('li.i').length, l.querySelector('li').textContent, l.children.length);\
         console.log(l.parentNode===document.body, document.documentElement.tagName, document.head.tagName, l.firstElementChild.nextSibling.textContent);\
         console.log(l.querySelector('li:nth-child(2)').matches('li'), l.closest('body')===document.body, document.querySelector('p'));\
         try { document.querySelector('!!'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        ["log:2 1 3", "log:true HTML HEAD 2", "log:true true null", "log:SyntaxError"],
        "{log:?}"
    );
}
// ---------- Q33b1: startup globals ----------

#[test]
fn q33b1_globals_feature_test_and_polyfill() {
    set_page_url(Url::parse("https://example.org:8080/a/b?x=1#frag").ok().as_ref());
    let out = run_collecting(&[src(
        "var r = [];\
         r.push(typeof Node, typeof Element, typeof HTMLElement, typeof Document, typeof Event, typeof CustomEvent, typeof Image);\
         Element.prototype.polyfilled = Element.prototype.polyfilled || function () { return true; };\
         r.push(Element.prototype.polyfilled() === true);\
         r.push(new Image() instanceof Element, new Image(3,4).width, document instanceof Document);\
         var e = new CustomEvent('x', { detail: 5, cancelable: true }); e.preventDefault();\
         r.push(e.type, e.detail, e.defaultPrevented, e instanceof Event);\
         r.push(navigator.userAgent.indexOf('scratchy') > 0, navigator.language);\
         r.push(innerWidth > 0, window.innerHeight > 0);\
         r.push(document.readyState, document.URL, document.cookie); document.cookie = 'a=b'; r.push(document.cookie === '');\
         r.push(location.href, location.protocol, location.host, location.hostname, location.port, location.pathname, location.search, location.hash, location.origin, String(location));\
         console.log(r.join('|'));",
    )]);
    assert_eq!(
        out,
        vec![
            "log:function|function|function|function|function|function|function|true|true|3|true|x|5|true|true|true|en-US|true|true|loading|https://example.org:8080/a/b?x=1#frag||true|https://example.org:8080/a/b?x=1#frag|https:|example.org:8080|example.org|8080|/a/b|?x=1|#frag|https://example.org:8080|https://example.org:8080/a/b?x=1#frag"
        ]
    );
}

#[test]
fn q33b1_no_url_gives_about_blank() {
    set_page_url(None);
    let out = run_collecting(&[src("console.log(location.href, location.protocol, typeof document.title)")]);
    assert_eq!(out, vec!["log:about:blank about: string"]);
}

#[test]
fn q33b1_wrappers_are_instances_of_the_constructors() {
    let out = run_collecting(&[src(
        "var b = document.body;\
         Element.prototype.myPoly = function () { return this.tagName; };\
         console.log([b instanceof Element, b instanceof HTMLElement, b instanceof Node, document.documentElement instanceof Element,\
           document.head.myPoly(), document instanceof Node, typeof document.getElementById].join('|'));",
    )]);
    assert_eq!(out, vec!["log:true|true|true|true|HEAD|true|function"]);
}

// ---------- Q33t: timers ----------

/// Runs `code` and keeps the engine, then pumps its timers on a manual
/// clock (jumping straight to each next due time, so no real sleeping
/// and no dependence on machine load) until none are pending; returns
/// everything logged. Every registration made while the script runs
/// happens at t=0 ms, which makes due-time ordering fully deterministic.
fn run_with_timers(code: &str) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(code)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    // Bounded: an interval would otherwise never run dry.
    for _ in 0..10_000 {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    let v = out.borrow().clone();
    v
}

#[test]
fn q33t_set_timeout_runs_after_the_turn() {
    let log = run_with_timers("setTimeout(()=>console.log('a'),0); console.log('b');");
    assert_eq!(log, vec!["log:b", "log:a"]);
}

#[test]
fn q33t_clear_timeout_cancels() {
    let log = run_with_timers(
        "var t = setTimeout(()=>console.log('no'),0); clearTimeout(t); window.setTimeout(()=>console.log('yes'),1);",
    );
    assert_eq!(log, vec!["log:yes"]);
}

#[test]
fn q33t_interval_runs_n_times_then_clears_itself() {
    let log = run_with_timers(
        "var n=0; var id=setInterval(function(){ n++; console.log('i'+n); if(n==3) clearInterval(id); }, 1);",
    );
    assert_eq!(log, vec!["log:i1", "log:i2", "log:i3"]);
}

#[test]
fn q33t_timer_args_string_handler_raf_and_order() {
    let log = run_with_timers(
        "setTimeout(function(a,b){console.log('args '+a+b)},30,'x','y');\
         setTimeout(\"console.log('str')\",1);\
         requestAnimationFrame(function(ts){console.log(typeof ts)});\
         var c=requestAnimationFrame(function(){console.log('cancelled')}); cancelAnimationFrame(c);",
    );
    assert_eq!(log, vec!["log:str", "log:number", "log:args xy"]);
}

#[test]
fn q33t_timer_error_is_logged_and_does_not_stop_others() {
    let log = run_with_timers("setTimeout(()=>{throw new Error('boom')},0); setTimeout(()=>console.log('ok'),1);");
    assert!(log[0].starts_with("error:Uncaught Error: boom"), "{log:?}");
    assert_eq!(log[1], "log:ok");
}

#[test]
fn q33t_no_timers_means_no_deadline() {
    let engine = start_scripts(&parse_html(""), &[src("1+1")], Rc::new(|_, _| {})).unwrap();
    assert!(engine.next_deadline().is_none());
}

#[test]
fn q33t_runaway_timer_callback_is_interrupted_with_its_own_budget() {
    let log = run_with_timers("setTimeout(()=>{ for(;;){} },0); setTimeout(()=>console.log('after'),1);");
    assert!(log.iter().any(|l| l.starts_with("error:")), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// R7: an image fetched for the current navigation can arrive before
/// that navigation's `NavResult` (scripts are fetched synchronously
/// before `NavResult` is sent); the document swap must not drop it,
/// while a stale-generation image must still be dropped.
#[test]
fn r7_image_fetched_before_nav_result_survives_document_swap() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    let img = || Some(DecodedImage { width: 1, height: 1, pixels: vec![TextColor::rgb(1, 2, 3)], svg: None, mask: None });
    app.image_cache.insert("stale-from-old-page".into(), Arc::new(img().unwrap()));
    app.fetch_url_inner(Url::parse("http://127.0.0.1:9/").unwrap());
    assert!(app.image_cache.is_empty(), "starting a navigation clears the old page images");
    let generation = app.nav_generation;
    app.handle_image_fetched(generation - 1, "old".into(), img());
    app.handle_image_fetched(generation, "globe.png".into(), img());
    let page = FetchedPage {
        html: b"<html><body><img src=\"globe.png\"></body></html>".to_vec(),
        extra_css: String::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert!(app.image_cache.contains_key("globe.png"), "image that beat NavResult must survive");
    assert!(!app.image_cache.contains_key("old"), "stale-generation image must be dropped");
}

// ---------- Q33c: DOM writes ----------

/// Runs `code` against `html`, returns the log and the body's markup
/// afterwards, read from the Rust tree (not through JS).
fn mutate(html: &str, code: &str) -> (Vec<String>, String) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let _engine = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let mut body = None;
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        if element_tag(&n) == "body" {
            body = Some(n.clone());
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    let mut s = String::new();
    for c in body.unwrap().children.borrow().iter() {
        let mut buf = Vec::new();
        let _ = html5ever::serialize::serialize(
            &mut buf,
            &markup5ever_rcdom::SerializableHandle::from(c.clone()),
            html5ever::serialize::SerializeOpts {
                traversal_scope: html5ever::serialize::TraversalScope::IncludeNode,
                ..Default::default()
            },
        );
        s.push_str(&String::from_utf8_lossy(&buf));
    }
    let log = out.borrow().clone();
    (log, s)
}

#[test]
fn q33c_class_attribute_and_classlist_writes_reach_the_tree() {
    let (log, body) = mutate(
        "<html class=no-js><body><div id=a class=x>t</div></body></html>",
        "var a=document.getElementById('a'); a.setAttribute('data-k','v'); a.id='b';\
         a.classList.add('y','z'); a.classList.remove('x'); console.log(a.classList.toggle('q'), a.classList.toggle('q'), a.className);\
         a.removeAttribute('data-k'); a.title='T';",
    );
    assert_eq!(log, ["log:true false y z"], "{log:?}");
    assert_eq!(body, "<div id=\"b\" class=\"y z\" title=\"T\">t</div>");
    let doc = parse_html("<html class=no-js><body></body></html>");
    let _ = start_scripts(&doc, &[src("document.documentElement.className = 'js-enabled'")], Rc::new(|_, _| {}));
    let html = doc.children.borrow().iter().find(|c| element_tag(c) == "html").cloned().unwrap();
    assert_eq!(element_attr(&html, "class").as_deref(), Some("js-enabled"));
}

#[test]
fn q33c_text_create_append_insert_remove() {
    let (log, body) = mutate(
        "<html><body><ul id=l><li>1</li><li id=two>2</li></ul><p id=p>old <b>x</b></p></body></html>",
        "var l=document.getElementById('l');\
         var n=document.createElement('LI'); n.textContent='new'; l.appendChild(n);\
         var f=document.createElement('li'); f.appendChild(document.createTextNode('first')); l.insertBefore(f, l.firstChild);\
         l.removeChild(document.getElementById('two'));\
         document.getElementById('p').textContent='plain';\
         var c=n.cloneNode(true); c.textContent='clone'; l.append(c);\
         try { n.appendChild(l); } catch (e) { console.log(e.name); }\
         var fr=document.createDocumentFragment(); fr.appendChild(document.createElement('hr')); fr.appendChild(document.createElement('br'));\
         document.body.appendChild(fr); console.log(fr.childNodes.length, l.children.length, n.parentNode===l);",
    );
    assert_eq!(log, ["log:HierarchyRequestError", "log:0 4 true"], "{log:?}");
    assert_eq!(body, "<ul id=\"l\"><li>first</li><li>1</li><li>new</li><li>clone</li></ul><p id=\"p\">plain</p><hr><br>");
}

// Acid3 (real page): `span.firstChild.data = score` must write the text node.
#[test]
fn acid3_text_node_data_setter_writes_the_tree() {
    let (log, body) = mutate(
        "<html><body><p id=p><span id=s>JS</span></p></body></html>",
        "var t=document.getElementById('s').firstChild; t.data='42'; console.log(t.data, t.nodeValue, t.length);",
    );
    assert_eq!(log, ["log:42 42 2"], "{log:?}");
    assert_eq!(body, "<p id=\"p\"><span id=\"s\">42</span></p>");
}

#[test]
fn q33c_inner_html_and_remove() {
    let (log, body) = mutate(
        "<html><body><div id=d>gone</div><span id=s>x</span></body></html>",
        "var d=document.getElementById('d'); d.innerHTML='<p class=k>a<b>b</b></p><i>c</i>';\
         console.log(d.innerHTML, d.firstChild.className, d.outerHTML.slice(0,15));\
         document.getElementById('s').remove();",
    );
    assert_eq!(log, ["log:<p class=\"k\">a<b>b</b></p><i>c</i> k <div id=\"d\"><p "], "{log:?}");
    assert_eq!(body, "<div id=\"d\"><p class=\"k\">a<b>b</b></p><i>c</i></div>");
}

#[test]
fn q33c_style_property_writes_the_style_attribute() {
    let (log, body) = mutate(
        "<html><body><div id=a style='color: red'>t</div></body></html>",
        "var a=document.getElementById('a'); a.style.backgroundColor='blue'; a.style.color='';\
         a.style.setProperty('margin-top','3px'); console.log(a.style.backgroundColor, a.style.marginTop, JSON.stringify(a.style.color));\
         a.style.display='none';",
    );
    assert_eq!(log, ["log:blue 3px \"\""], "{log:?}");
    assert_eq!(body, "<div id=\"a\" style=\"background-color: blue; margin-top: 3px; display: none;\">t</div>");
}

#[test]
fn q33c_many_writes_mark_dirty_once_and_reads_stay_clean() {
    let doc = parse_html("<html><body><div id=a></div></body></html>");
    let engine = start_scripts(
        &doc,
        &[src("var a=document.getElementById('a'); for (var i=0;i<50;i++) a.className='c'+i;")],
        Rc::new(|_, _| {}),
    )
    .unwrap();
    assert!(engine.take_dirty());
    assert!(!engine.take_dirty(), "the flag clears once drained");
    let clean = start_scripts(
        &parse_html("<html><body><div id=a></div></body></html>"),
        &[src("document.getElementById('a').className")],
        Rc::new(|_, _| {}),
    )
    .unwrap();
    assert!(!clean.take_dirty(), "reads never dirty the page");
}

#[test]
fn q33c2_closed_dialog_is_not_rendered_open_dialog_is() {
    // QB4: an open dialog is now out of flow (UA `position: absolute`), so
    // it shows up in `positioned` and with its own height rather than
    // growing the document.
    let p = |d: &str| measure_document_fixture(&format!("<html><body><p>a</p>{d}</body></html>"), 500.0, 400.0).positioned.len();
    assert_eq!(p("<dialog><div style=\"height:100px\">x</div></dialog>"), 0, "closed dialog renders nothing");
    let laid = measure_document_fixture("<html><body><p>a</p><dialog open><div style=\"height:100px\">x</div></dialog></body></html>", 500.0, 400.0);
    let d = laid.positioned.first().expect("open dialog still renders");
    assert!(d.h >= 100.0, "open dialog renders its content, h {}", d.h);
}

#[test]
fn q33c3_select_color_transparent_paints_no_option_text() {
    // Wikipedia portal: `.styled-select.js-enabled select` overlays a
    // `color:transparent` select on a visible label; the select's option
    // text must not paint.
    let (pixels, _, _) = paint_fixture(
        r#"<html><head><style>.s select{background:transparent;border:0;margin:0;padding:0 24px;font-size:40px;color:transparent;color:hsla(0,0%,100%,0)}</style></head><body style="margin:0;background:#fff"><div class="s"><select><option selected>Afrikaans</option><option>English</option></select></div></body></html>"#,
    );
    // Unpainted pixels are 0, the white body fill is ffffff; any anti-aliased
    // glyph pixel is something else.
    let stray = pixels.iter().filter(|&&p| p & 0x00FF_FFFF != 0 && p & 0x00FF_FFFF != 0x00FF_FFFF).count();
    assert_eq!(stray, 0, "transparent select text must not paint");
    assert_eq!(
        to_text_color(&parse_declarations("color: hsl(120, 100%, 50%)").color.unwrap()),
        Some(TextColor::rgb(0, 255, 0))
    );
    assert_eq!(
        to_text_color(&parse_declarations("color: hsla(0,0%,100%,0)").color.unwrap()),
        Some(TextColor::rgba(255, 255, 255, 0))
    );
    // Partial alpha text is scaled, not painted opaque.
    let (half, _, _) = paint_fixture(
        r#"<html><body style="margin:0;background:#fff"><p style="margin:0;font-size:40px;color:rgba(0,0,0,0.2)">HHHH</p></body></html>"#,
    );
    let darkest = half.iter().filter(|&&p| p != 0).map(|&p| p & 0xFF).min().unwrap();
    assert!(darkest > 0xA0, "20% black text must stay light, darkest channel {darkest:#x}");
}

#[test]
fn q33c3_option_in_select_uses_the_selects_colour() {
    // The portal's `select option{color:grey}` must not paint the closed
    // select's text when the select itself is transparent.
    let (pixels, _, _) = paint_fixture(
        r#"<html><head><style>select{color:transparent;font-size:40px;border:0}select option{color:#777}</style></head><body style="margin:0;background:#fff"><select><option selected>Afrikaans</option></select></body></html>"#,
    );
    let stray = pixels.iter().filter(|&&p| p & 0x00FF_FFFF != 0 && p & 0x00FF_FFFF != 0x00FF_FFFF).count();
    assert_eq!(stray, 0, "option colour must not override the select's transparent text");
}

// ---------- Q33d: events ----------

/// Starts `code` against `html` and returns the engine, the document and
/// the shared log (so tests can dispatch native events afterwards).
fn events_engine(html: &str, code: &str) -> (JsEngine, Handle, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    (engine, doc, out)
}

fn by_id(doc: &Handle, id: &str) -> Handle {
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        if element_attr(&n, "id").as_deref() == Some(id) {
            return n;
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    panic!("no #{id}");
}

#[test]
fn q33d_capture_target_bubble_order_and_stop_propagation() {
    let (_e, _d, log) = events_engine(
        "<html><body><div id=o><p id=i>x</p></div></body></html>",
        "var o=document.getElementById('o'), i=document.getElementById('i');\
         window.addEventListener('click', function(e){ console.log('w-cap', e.eventPhase) }, true);\
         o.addEventListener('click', function(e){ console.log('o-cap', e.eventPhase) }, true);\
         i.addEventListener('click', function(e){ console.log('i-bub', e.eventPhase, e.target===i, e.currentTarget===i) });\
         i.addEventListener('click', function(e){ console.log('i-cap') }, {capture:true});\
         o.addEventListener('click', function(e){ console.log('o-bub', e.eventPhase) });\
         document.addEventListener('click', function(e){ console.log('doc'); e.stopPropagation(); });\
         window.addEventListener('click', function(e){ console.log('w-bub') });\
         i.dispatchEvent(new Event('click', {bubbles:true}));",
    );
    assert_eq!(
        *log.borrow(),
        ["log:w-cap 1", "log:o-cap 1", "log:i-cap", "log:i-bub 2 true true", "log:o-bub 3", "log:doc"]
    );
}

#[test]
fn q33d_prevent_default_once_remove_passive_onprop_and_throwing_listener() {
    let (_e, _d, log) = events_engine(
        "<html><body><div id=o><p id=i>x</p></div></body></html>",
        "var o=document.getElementById('o'), i=document.getElementById('i');\
         o.addEventListener('ping', function(){ console.log('o') });\
         var f=function(){ console.log('f') }; i.addEventListener('ping', f); i.removeEventListener('ping', f);\
         i.addEventListener('ping', function(e){ console.log('once'); e.preventDefault(); }, {once:true});\
         console.log(i.dispatchEvent(new Event('ping')));\
         console.log(i.dispatchEvent(new Event('ping', {cancelable:true})));\
         i.addEventListener('pong', function(e){ e.preventDefault(); console.log(e.defaultPrevented) }, {passive:true});\
         i.dispatchEvent(new Event('pong', {cancelable:true}));\
         i.onclick = function(){ console.log('onclick'); return false; };\
         console.log(i.dispatchEvent(new Event('click', {cancelable:true})));\
         i.addEventListener('boom', function(){ throw new Error('x') });\
         i.addEventListener('boom', function(){ console.log('after boom') });\
         i.dispatchEvent(new Event('boom'));",
    );
    let log = log.borrow();
    assert_eq!(log[..5], ["log:once", "log:true", "log:true", "log:false", "log:onclick"]);
    assert_eq!(log[5], "log:false");
    assert!(log[6].starts_with("error:Uncaught Error: x"), "{log:?}");
    assert_eq!(log[7], "log:after boom");
}

#[test]
fn q33d_native_click_reaches_page_handler_and_prevent_default_is_reported() {
    let (engine, doc, log) = events_engine(
        "<html><body><a id=a href='/x'><b id=b>t</b></a></body></html>",
        "document.getElementById('a').addEventListener('click', function(e){\
           console.log('click', e.target.id, e.isTrusted, e.clientX, e.button, e.ctrlKey); e.preventDefault(); });",
    );
    let b = by_id(&doc, "b");
    let init = [
        ("bubbles", EventField::Bool(true)),
        ("cancelable", EventField::Bool(true)),
        ("clientX", EventField::Num(12.0)),
        ("button", EventField::Num(0.0)),
    ];
    assert!(!engine.dispatch_event(Some(&b), "click", &init), "handler cancelled the click");
    assert_eq!(*log.borrow(), ["log:click b true 12 0 false"]);
    // A page with no handler lets the default action proceed.
    let (engine2, doc2, _) = events_engine("<html><body><a id=a href='/x'>t</a></body></html>", "1");
    assert!(engine2.dispatch_event(Some(&by_id(&doc2, "a")), "click", &init));
}

#[test]
fn q33d_native_key_and_focus_events_carry_their_fields() {
    let (engine, doc, log) = events_engine(
        "<html><body><a id=a href='/x'>t</a></body></html>",
        "var a=document.getElementById('a');\
         document.addEventListener('keydown', function(e){ console.log('kd', e.key, e.repeat, e.shiftKey, e.target.id); });\
         a.addEventListener('focus', function(e){ console.log('focus', e.type, e.bubbles); });\
         document.addEventListener('focusin', function(e){ console.log('focusin', e.target.id); });\
         a.addEventListener('blur', function(e){ console.log('blur'); });\
         document.body.addEventListener('keyup', function(e){ console.log('ku', e.key, e.target.tagName); e.preventDefault(); });",
    );
    let a = by_id(&doc, "a");
    let key = |k: &str, shift: bool| {
        [
            ("bubbles", EventField::Bool(true)),
            ("cancelable", EventField::Bool(true)),
            ("key", EventField::Str(k.to_string())),
            ("shiftKey", EventField::Bool(shift)),
        ]
    };
    assert!(engine.dispatch_event(Some(&a), "keydown", &key("\u{e9}", true)));
    assert!(!engine.dispatch_event(None, "keyup", &key("Enter", false)), "None targets <body>");
    engine.dispatch_event(Some(&a), "focus", &[("bubbles", EventField::Bool(false))]);
    engine.dispatch_event(Some(&a), "focusin", &[("bubbles", EventField::Bool(true))]);
    engine.dispatch_event(Some(&a), "blur", &[]);
    assert_eq!(
        *log.borrow(),
        ["log:kd \u{e9} false true a", "log:ku Enter BODY", "log:focus focus false", "log:focusin a", "log:blur"]
    );
}

#[test]
fn q33d_load_events_and_event_target_subclass() {
    let log = run_collecting(&[src(
        "document.addEventListener('DOMContentLoaded', function(e){ console.log('dcl', e.target===document, e.bubbles); });\
         window.addEventListener('DOMContentLoaded', function(e){ console.log('dcl-window'); });\
         window.addEventListener('load', function(e){ console.log('load', e.target===window); });\
         var t = new EventTarget(); t.addEventListener('x', function(e){ console.log('x', e instanceof CustomEvent, e.detail); });\
         t.dispatchEvent(new CustomEvent('x', {detail: 7}));\
         addEventListener('y', function(){ console.log('global y'); }); dispatchEvent(new Event('y'));",
    )]);
    assert_eq!(
        log,
        ["log:x true 7", "log:global y", "log:dcl true true", "log:dcl-window", "log:load true"]
    );
}

#[test]
fn qb1_event_loop_exit_message() {
    use winit::error::EventLoopError;
    assert_eq!(event_loop_exit_message(&Ok(())), None);
    let m = event_loop_exit_message(&Err(EventLoopError::ExitFailure(1))).unwrap();
    assert!(m.contains("Wayland compositor"), "{m}");
    assert!(!m.contains(char::from(10)));
}

// Q49a: a float's shrink-to-fit width is min(max(min-content, available),
// max-content), so a float in a container narrower than its longest word is
// as wide as the word, not squeezed to the container.
#[test]
fn q49a_float_in_narrow_container_is_at_least_min_content() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:5px"><div style="float:left;height:0">WWWWWWWWWW</div></div></body></html>"#,
        500.0,
    );
    fn find_float(l: &Laid) -> Option<&Laid> {
        if l.style.float != Float::None {
            return Some(l);
        }
        match &l.content {
            LaidContent::Blocks(cs) => cs.iter().find_map(find_float),
            _ => None,
        }
    }
    let f = find_float(&laid).expect("float");
    assert!(f.w > 20.0, "float w {} should be >= min-content, not the 5px container", f.w);
    assert!(shrink_to_fit(30.0, 80.0, 5.0) == 30.0 && shrink_to_fit(30.0, 80.0, 50.0) == 50.0 && shrink_to_fit(30.0, 80.0, 500.0) == 80.0);
}

// ---------- QB2: JS DOM writes use the restyle path ----------

/// An `App` with a complete cached frame and a JS engine whose script has
/// already written the DOM (engine dirty). `None` without a display.
fn qb2_app(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) -> App {
    let doc = parse_html("<html><body><p>hello</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(&doc, &[src("document.body.setAttribute('data-tick', '1')")], None);
    app.cached = Some(measure_fixture("<p>hello</p>", 400.0));
    app.layout_dirty = false;
    app.restyle_requested = false;
    app.keep_scroll_on_rebuild = false;
    app
}

/// One process can build only one event loop, so both scenarios share it.
#[test]
fn qb2_js_dom_writes_use_restyle_path() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let proxy = event_loop.create_proxy();
    qb2_restyle_and_keep_old_frame(qb2_app(&proxy));
    qb2_coalesce_into_one_rebuild(qb2_app(&proxy));
    qb2_first_layout_not_made_stale(qb2_app(&proxy));
    qb5_app_applies_requests(&proxy);
    qb6_help_suspends_js(&proxy);
    qf7_app_net(&proxy);
    qf14b_app_initiator(&proxy);
}

// ---------- QB6: `:help` suspends the page's JS ----------

fn qb6_counter(app: &App) -> f64 {
    app.js.as_ref().unwrap().eval_value::<f64>("t", "window.__n").unwrap()
}

fn qb6_help_suspends_js(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let doc = parse_html("<html><body><p>page</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(
        &doc,
        &[src("window.__n = 0; setInterval(function(){ window.__n++; document.body.setAttribute('data-n', window.__n); }, 1);")],
        None,
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.js.as_ref().unwrap().run_due();
    let before = qb6_counter(&app);
    assert!(before >= 1.0, "interval should have fired");

    app.show_help();
    assert!(app.js.is_none(), "help page must have no JS engine");
    assert!(app.pre_help_page.as_ref().unwrap().js.is_some(), "engine parked in SavedPage");
    // Key dispatch and sync reach nothing: the old page's listeners are gone.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(app.js_dispatch(None, "keydown", &[]), "no engine: dispatch returns true");
    app.js_sync();
    // Re-running :help keeps the parked engine.
    app.show_help();
    assert!(app.pre_help_page.as_ref().unwrap().js.is_some());

    app.restore_saved_page();
    assert!(app.js.is_some(), "engine restored");
    assert_eq!(qb6_counter(&app), before, "timers did not run while help was shown");
    app.js.as_ref().unwrap().run_due();
    assert!(qb6_counter(&app) > before, "overdue timer fires after return");

    // show_message also drops the engine.
    app.show_message("oops");
    assert!(app.js.is_none());
}

fn qb2_restyle_and_keep_old_frame(mut app: App) {
    let generation = app.layout_generation;
    app.js_sync();
    assert!(app.layout_dirty && app.restyle_requested && app.keep_scroll_on_rebuild);
    assert_eq!(app.layout_generation, generation, "redraw bumps the generation, not js_sync");
    // Simulate what redraw does when it queues the rebuild.
    app.restyle_requested = false;
    app.hold_old_frame = true;
    app.layout_in_flight = true;
    app.layout_dirty = false;
    app.layout_generation += 1;
    let h_before = app.cached.as_ref().unwrap().h;
    let partial = measure_fixture("<p>hello</p><p>world</p><p>more</p><p>text</p>", 400.0);
    app.handle_shape_result(app.layout_generation, Box::new(partial), ShapeCache::new(), true);
    assert_eq!(app.cached.as_ref().unwrap().h, h_before, "incomplete pass must not replace the old frame");
    assert!(app.hold_old_frame);
}

fn qb2_coalesce_into_one_rebuild(mut app: App) {
    app.hold_old_frame = true;
    app.layout_in_flight = true;
    let generation = app.layout_generation;
    app.js_sync();
    assert!(!app.layout_dirty && !app.restyle_requested, "no rebuild queued while a restyle is in flight");
    assert!(app.js_restyle_pending);
    assert_eq!(app.layout_generation, generation, "in-flight result must not go stale");
    // A second write while still holding changes nothing more.
    app.js_sync();
    assert!(!app.layout_dirty && app.js_restyle_pending);
    // The complete pass lands: exactly one follow-up rebuild is requested.
    app.handle_shape_result(generation, Box::new(measure_fixture("<p>hello</p>", 400.0)), ShapeCache::new(), false);
    assert!(!app.hold_old_frame);
    assert!(!app.js_restyle_pending);
    assert!(app.layout_dirty && app.restyle_requested && app.keep_scroll_on_rebuild);
    assert!(!app.layout_in_flight);
}

/// A write while the very first layout is still being shaped (no complete
/// frame yet) must not make that pass stale either.
fn qb2_first_layout_not_made_stale(mut app: App) {
    app.cached = None;
    app.layout_in_flight = true;
    let generation = app.layout_generation;
    app.js_sync();
    assert!(app.js_restyle_pending && !app.layout_dirty);
    assert_eq!(app.layout_generation, generation);
    app.handle_shape_result(generation, Box::new(measure_fixture("<p>hello</p>", 400.0)), ShapeCache::new(), false);
    assert!(app.cached.is_some());
    assert!(app.layout_dirty && app.restyle_requested);
}

// ---------- QB4: HTMLDialogElement ----------

#[test]
fn qb4_dialog_show_modal_close_return_value_and_event() {
    let (log, body) = mutate(
        "<html><body><dialog id=d><button id=b>x</button></dialog></body></html>",
        "var d=document.getElementById('d'); var b=document.getElementById('b');\
         console.log(typeof d.showModal, typeof d.close, d.open, JSON.stringify(d.returnValue));\
         d.addEventListener('close', function(e){ console.log('closed', d.open, d.returnValue, e.type); });\
         b.addEventListener('click', function(){ d.close('x'); });\
         d.showModal(); console.log(d.open, d.hasAttribute('open'));\
         b.dispatchEvent(new Event('click',{bubbles:true}));\
         console.log(d.open, d.returnValue, d instanceof HTMLDialogElement);\
         d.show(); console.log(d.open); d.open = false; console.log(d.open);",
    );
    assert_eq!(
        log,
        ["log:function function false \"\"", "log:true true", "log:closed false x close", "log:false x true", "log:true", "log:false"],
        "{log:?}"
    );
    assert!(!body.contains("open") && !body.contains("modal"), "{body}");
}

#[test]
fn qb4_modal_dialog_is_centred_in_the_viewport() {
    let w = |attrs: &str| {
        measure_document_fixture(
            &format!("<html><body style=\"margin:0\"><dialog {attrs} style=\"width:100px;height:50px;padding:0;border:0\">x</dialog></body></html>"),
            400.0,
            300.0,
        )
    };
    let closed = w("");
    assert!(closed.positioned.is_empty(), "closed dialog renders nothing");
    let laid = w("open data-scratchy-modal");
    let d = laid.positioned.first().expect("modal dialog is out of flow");
    assert_eq!((d.x, d.y), (150.0, 125.0), "centred in 400x300");
}

#[test]
fn qb4_clicks_inside_a_modal_dialog_hit_its_content() {
    let html = "<html><body style=\"margin:0\"><p>outer</p><dialog open data-scratchy-modal style=\"width:100px;height:50px;padding:0;border:0\"><div style=\"height:50px\">in</div></dialog></body></html>";
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 400.0, 300.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let so = HashMap::new();
    let node = hit_test_element(&laid, 200.0, 150.0, &cache, &so, 0.0, 300.0);
    let h = crate::dom::find_node_by_id(&doc, node).unwrap_or_else(|| panic!("hit node {node}"));
    assert_eq!(element_tag(&h), "div", "click at the dialog's centre lands on its content");
}

// ---------- QB5: focus()/blur(), autofocus, window.scroll* ----------

fn qb5_engine(html: &str, code: &str) -> JsEngine {
    let doc = parse_html(html);
    start_scripts(&doc, &[src(code)], Rc::new(|_, _| {})).unwrap()
}

#[test]
fn qb5_focus_and_blur_are_functions_and_record_requests() {
    let engine = qb5_engine(
        "<html><body><input id=i><div id=d></div></body></html>",
        "var i=document.getElementById('i'), d=document.getElementById('d');
         if (typeof i.focus !== 'function' || typeof i.blur !== 'function') throw new Error('missing');
         d.focus(); // not focusable: ignored
         i.focus();",
    );
    let r = engine.take_requests();
    let Some(crate::js_dom::FocusReq::Focus(h)) = r.focus else { panic!("no focus request: {r:?}") };
    assert_eq!(element_attr(&h, "id").as_deref(), Some("i"));
    assert!(engine.take_requests().focus.is_none(), "drained");
    engine.eval("t2", "document.getElementById('i').blur()");
    assert!(matches!(engine.take_requests().focus, Some(crate::js_dom::FocusReq::Blur(_))));
}

#[test]
fn qb5_autofocus_and_boolean_attributes_reflect() {
    let engine = qb5_engine(
        "<html><body><input id=i autofocus><input id=j></body></html>",
        "var i=document.getElementById('i'), j=document.getElementById('j');
         var r=[i.autofocus, j.autofocus, j.disabled]; j.autofocus=true; j.disabled=true; i.autofocus=false;
         r.push(j.hasAttribute('autofocus'), j.disabled, i.hasAttribute('autofocus'), j.required, j.checked);
         // the portal's idiom: void 0===t.autofocus?t.focus():window.scroll(0,0)
         void 0===j.autofocus?j.focus():window.scroll(0,0);
         globalThis.out = r.join();",
    );
    assert_eq!(engine.eval_value::<String>("t", "out").unwrap(), "true,false,false,true,true,false,false,false");
}

#[test]
fn qb5_window_scroll_family_records_a_request() {
    let engine = qb5_engine("<html><body></body></html>", "window.scrollTo(0, 100);");
    let s = engine.take_requests().scroll.expect("scroll request");
    assert_eq!((s.x, s.y, s.relative), (Some(0.0), Some(100.0), false));
    engine.eval("t", "window.scroll({top: 40}); window.scrollBy(0, 5);");
    let s = engine.take_requests().scroll.unwrap();
    assert_eq!((s.y, s.relative), (Some(5.0), true), "last request wins");
    engine.eval("t", "window.scroll({top: 40})");
    let s = engine.take_requests().scroll.unwrap();
    assert_eq!((s.x, s.y), (None, Some(40.0)), "missing left keeps the axis");
    assert_eq!(engine.eval_value::<f64>("t", "window.scrollY").unwrap(), 40.0);
}

/// App side: `js_sync` applies a scroll request and a focus request.
fn qb5_app_applies_requests(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let doc = parse_html("<html><body><input id=i><p>hello</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(
        &doc,
        &[src("window.scrollTo(0, 120); document.getElementById('i').focus();")],
        None,
    );
    app.js_sync();
    assert_eq!(app.scroll_offset, 120.0);
    let i = crate::dom::find_node_by_id(&doc, app.interaction.focus.expect("focused")).unwrap();
    assert_eq!(element_attr(&i, "id").as_deref(), Some("i"));
    app.js.as_ref().unwrap().eval("t", "window.scrollBy(0, -500); document.getElementById('i').blur();");
    app.js_sync();
    assert_eq!(app.scroll_offset, 0.0, "clamped at the top");
    assert!(app.interaction.focus.is_none(), "blur() of the focused element clears focus");
}

/// QB3: the pointer-idle gate for hover restyles is a pure function of
/// the last build time and the time since the pointer last moved.
#[test]
fn qb3_hover_idle_gate() {
    use std::time::Duration as D;
    let ms = D::from_millis;
    // Fast builds never wait, however recently the pointer moved.
    assert_eq!(hover_idle_wait(Some(ms(20)), Some(ms(0))), None);
    assert_eq!(hover_idle_wait(Some(HOVER_SLOW_BUILD), Some(ms(0))), None);
    // No build measured yet: nothing to protect against.
    assert_eq!(hover_idle_wait(None, Some(ms(0))), None);
    // Slow build, pointer just moved: wait out the idle window.
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(0))), Some(HOVER_POINTER_IDLE));
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(100))), Some(HOVER_POINTER_IDLE - ms(100)));
    // Slow build, pointer still long enough (or never moved): go ahead.
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(HOVER_POINTER_IDLE)), None);
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(5000))), None);
    assert_eq!(hover_idle_wait(Some(ms(400)), None), None);
}

/// QB3: `hover_restyle_delay` uses the gate: a slow page with a moving
/// pointer defers the restyle, a still pointer or a fast page does not.
#[test]
fn qb3_app_defers_hover_while_pointer_moves_on_slow_pages() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.layout_dirty = false;
    app.hold_old_frame = false;
    app.last_build_time = Some(std::time::Duration::from_millis(400));
    app.last_pointer_move = Some(std::time::Instant::now());
    assert!(app.hover_restyle_delay().is_some(), "moving pointer on a slow page defers hover");
    app.last_pointer_move = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    assert!(app.hover_restyle_delay().is_none(), "still pointer applies hover");
    app.last_pointer_move = Some(std::time::Instant::now());
    app.last_build_time = Some(std::time::Duration::from_millis(5));
    assert!(app.hover_restyle_delay().is_none(), "fast page applies hover immediately");
}

/// QB3b: a burst of pointer-driven hover changes on a slow page, then a
/// final move onto a link, must end with the link (the last target)
/// hovered: the pending flush re-arms while the pointer moves and applies
/// the final `hover_node`, never an earlier one. (The "link stays
/// un-highlighted" symptom came from the dev injector dropping its last
/// pointer motion, not from this path; this pins the app side.)
#[test]
fn qb3b_final_hover_target_applied_after_burst() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let doc = parse_html("<html><head><style>a:hover{color:red}</style></head><body><p>x</p><div>y</div><a href=\"/x\">z</a></body></html>");
    let a = node_id(&find_by_tag(&doc, "a").unwrap());
    let p = node_id(&find_by_tag(&doc, "p").unwrap());
    let div = node_id(&find_by_tag(&doc, "div").unwrap());
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    app.layout_dirty = false;
    app.hold_old_frame = false;
    app.state_rules = STATE_HOVER;
    app.last_build_time = Some(std::time::Duration::from_millis(400));
    // The burst: the pointer keeps moving, so every flush is deferred.
    for i in 0..100 {
        app.last_pointer_move = Some(std::time::Instant::now());
        app.update_hover(if i % 2 == 0 { p } else { div });
        assert!(app.hover_restyle_delay().is_some());
    }
    // The last move lands on the link immediately after the burst.
    app.last_pointer_move = Some(std::time::Instant::now());
    app.update_hover(a);
    assert_eq!(app.hover_node, a);
    assert!(app.interaction.hover.is_empty(), "still deferred: pointer not idle");
    // Pointer goes still; the flush that was already pending fires.
    app.last_pointer_move = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    assert!(app.hover_restyle_delay().is_none());
    app.apply_hover();
    assert!(app.interaction.hover.contains(&a), "the final target is hovered");
    assert!(app.layout_dirty, "and a restyle was requested");
}

/// QB3: timer/event callbacks get ~500 ms, not the 5 s page-load budget.
#[test]
fn qb3_callback_budget_is_much_shorter_than_load_budget() {
    let log = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    let engine = start_scripts(
        &parse_html(""),
        &[src("setTimeout(()=>{ for(;;){} },0);")],
        Rc::new(move |lvl, t| l.borrow_mut().push(format!("{lvl}:{t}"))),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let t = std::time::Instant::now();
    engine.run_due();
    let took = t.elapsed();
    assert!(took < std::time::Duration::from_millis(2500), "callback ran {took:?}");
    assert!(took >= std::time::Duration::from_millis(400), "callback ran {took:?}");
    assert!(log.borrow().iter().any(|m| m.contains("budget")), "{:?}", log.borrow());
}

/// QB9: a writer whose every call fails with EPIPE, which is what
/// `scratchy … 2>&1 | head -1` gives once `head` has exited.
struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// QB9: diagnostic output on a closed pipe returns normally. The old
/// `eprintln!`/`println!` call sites panicked here (exit 101).
#[test]
fn qb9_write_line_to_ignores_closed_pipe() {
    let mut pipe = ClosedPipe;
    write_line_to(&mut pipe, format_args!("scratchy: could not {}: {}", "draw", 1));
    write_line_to(&mut pipe, format_args!("plain"));
}

/// QB9: the helper writes exactly what `eprintln!` wrote (text plus `\n`).
#[test]
fn qb9_write_line_to_writes_text_and_newline() {
    let mut buf: Vec<u8> = Vec::new();
    write_line_to(&mut buf, format_args!("usage: scratchy {}", "[file|url]"));
    assert_eq!(buf, b"usage: scratchy [file|url]\n");
}

// ---------- QA13: document.write / writeln during page load ----------

fn qa13_run(html: &str) -> Vec<String> {
    let (_engine, out) = qb8_start(html);
    let v = out.borrow().clone();
    v
}

#[test]
fn qa13_write_inserts_after_the_running_script() {
    let out = qa13_run(
        "<html><body><p id=a>x<script>document.write('<b id=w>hi</b>')</script><i id=z></i></p>\
         <script>var s=document.getElementById('a').getElementsByTagName('script')[0];\
         console.log(s.nextSibling.id, s.nextSibling.textContent, s.nextSibling.nextSibling.id);</script></body></html>",
    );
    assert_eq!(out, ["log:w hi z"]);
}

#[test]
fn qa13_two_scripts_each_write_and_consecutive_writes_keep_order() {
    let out = qa13_run(
        "<html><body><script>document.write('<u id=a1></u>'); document.write('<u id=a2></u>', '<u id=a3></u>')</script>\
         <script>document.write('<u id=b1></u>')</script>\
         <script>console.log(Array.prototype.map.call(document.body.children, function(e){return e.id||e.tagName}).join())</script></body></html>",
    );
    assert_eq!(out, ["log:SCRIPT,a1,a2,a3,SCRIPT,b1,SCRIPT"]);
}

#[test]
fn qa13_write_inside_head_and_writeln() {
    let out = qa13_run(
        "<html><head><script>document.writeln('<meta id=m name=k>'); document.write('<meta id=n>')</script></head>\
         <body><script>var m=document.getElementById('m'); console.log(document.head.children.length, m.nextSibling.nodeType, m.nextSibling.data === '\\n');</script></body></html>",
    );
    assert_eq!(out, ["log:3 3 true"]);
}

#[test]
fn qa13_write_after_load_is_ignored_with_a_warning() {
    let (engine, out) = qb8_start(
        "<body><script>setTimeout(function(){ document.write('<b id=late></b>'); console.log(String(document.getElementById('late'))); }, 0)</script></body>",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    let out = out.borrow();
    assert!(out.iter().any(|l| l.contains("ignored")), "{out:?}");
    assert!(out.iter().any(|l| l == "log:null"), "{out:?}");
}

// ---------- QB8: document.currentScript, named eval frames ----------

fn qb8_start(html: &str) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(&doc, &inline_scripts(&doc), Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    (engine, out)
}

#[test]
fn qb8_current_script_is_the_executing_element_then_null() {
    let (_engine, out) = qb8_start(
        "<script src=x.js></script><script id=a>console.log(document.currentScript.id, document.currentScript.tagName);
         document.addEventListener('DOMContentLoaded', function(){ console.log('dcl', document.currentScript); });</script>
         <script id=b>console.log(document.currentScript.id)</script>",
    );
    assert_eq!(*out.borrow(), ["log:a SCRIPT", "log:b", "log:dcl null"]);
}

#[test]
fn qb8_timer_callback_sees_null_current_script() {
    let (engine, out) = qb8_start("<script id=a>setTimeout(function(){ console.log('t:' + document.currentScript); }, 0)</script>");
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    assert_eq!(*out.borrow(), ["log:t:null"]);
}

#[test]
fn qb8_event_and_timer_errors_name_their_frame() {
    let (engine, out) = qb8_start(
        "<body><script>document.body.addEventListener('click', function(){ throw new Error('boom'); }); setTimeout(function(){ throw new Error('tick'); }, 0);</script></body>",
    );
    engine.dispatch_event(None, "click", &[]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    let out = out.borrow();
    assert!(out.iter().any(|l| l.contains("boom") && l.contains("event:click")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("tick") && l.contains("timer")), "{out:?}");
    assert!(out.iter().all(|l| !l.contains("(timers:1:1)")), "{out:?}");
}

#[test]
fn qa1_border_style_initial_value_is_none() {
    let div_of = |style: &str, width: f32| {
        let html = format!(r#"<html><body><div style="width: {width}px; {style}">x</div></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        find_by_width(&laid, width).expect("div not found").style
    };
    let s = div_of("border: 4px blue;", 311.0);
    assert_eq!(s.border_width, [0.0; 4]);
    assert_eq!(s.border_style, [BorderStyle::None; 4]);
    let s = div_of("border-width: 4px;", 312.0);
    assert_eq!(s.border_width, [0.0; 4]);
    let s = div_of("border: 4px solid blue;", 313.0);
    assert_eq!(s.border_width, [4.0; 4]);
    let s = div_of("border-top: 4px solid blue;", 314.0);
    assert_eq!(s.border_width, [4.0, 0.0, 0.0, 0.0]);
    // The UA sheet's own borders still name a style.
    let laid = measure_fixture(r#"<html><body><hr style="width: 321px"></body></html>"#, 500.0);
    let hr = find_by_width(&laid, 321.0).expect("hr not found").style;
    assert_eq!(hr.border_width, [1.0; 4]);
}

// ---------- QA2: U+00A0 is not collapsible white space ----------

fn qa2_body_h(inner: &str) -> f32 {
    let html = format!(r#"<html><body style="margin:0;font:12px sans-serif;line-height:1em">{inner}</body></html>"#);
    measure_fixture(&html, 400.0).h
}

#[test]
fn qa2_nbsp_only_line_has_height() {
    assert!(qa2_body_h("<div>&nbsp;</div>") >= 11.0, "{}", qa2_body_h("<div>&nbsp;</div>"));
    assert!(qa2_body_h("<div>&nbsp;&nbsp;&nbsp;</div>") >= 11.0);
    // A plain space still collapses to nothing.
    assert_eq!(qa2_body_h("<div> </div>"), 0.0);
}

// ---------- QA4: `background` shorthand resets omitted longhands, rejects junk ----------

/// Computed background colour of the 13x13 div under the stylesheet `css`.
fn qa4_bg(css: &str) -> Option<TextColor> {
    let html = format!(r#"<html><head><style>{css}</style></head><body><div style="width:13px;height:13px"></div></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    find_by_width(&laid, 13.0).expect("div not found").style.background_color
}

fn qa4_rgb(c: Option<TextColor>) -> (u8, u8, u8) {
    let c = c.expect("a background colour");
    (c.r(), c.g(), c.b())
}

fn qa4_visible_red(c: Option<TextColor>) -> bool {
    c.is_some_and(|c| c.a() > 0 && c.r() == 255 && c.g() == 0)
}

#[test]
fn qa4_background_none_resets_earlier_color() {
    assert!(qa4_visible_red(qa4_bg("div{background:red}")));
    assert!(!qa4_visible_red(qa4_bg("div{background:red} div{background:none}")));
    assert!(!qa4_visible_red(qa4_bg("div{background:red; background:none}")));
    // Omitted colour resets to transparent, even with other parts given.
    assert!(!qa4_visible_red(qa4_bg("div{background-color:red; background:no-repeat}")));
    // Resets the image too.
    let src = tiny_png_data_uri(4, 4);
    let html = format!(r#"<html><head><style>div{{background-image:url('{src}')}} div{{background:red}}</style></head><body><div style="width:13px;height:13px"></div></body></html>"#);
    let laid = measure_fixture(&html, 200.0);
    assert!(find_by_width(&laid, 13.0).unwrap().background_image.is_none());
}

#[test]
fn qa4_background_junk_drops_whole_declaration() {
    // Acid2 parser line: two colours is invalid, the earlier yellow stays.
    assert_eq!(qa4_rgb(qa4_bg("div{background:yellow; background: red pink}")), (255, 255, 0));
    assert_eq!(qa4_rgb(qa4_bg("div{background:yellow; background: red bogus}")), (255, 255, 0));
    // A colour before a comma (not the last layer) is invalid.
    assert_eq!(qa4_rgb(qa4_bg("div{background:yellow; background: red, url(x.png)}")), (255, 255, 0));
}

#[test]
fn qa4_background_accepts_attachment_and_box_keywords() {
    assert_eq!(qa4_rgb(qa4_bg("div{background: blue fixed padding-box content-box}")), (0, 0, 255));
    assert_eq!(qa4_rgb(qa4_bg("div{background: scroll border-box no-repeat center / cover #00f}")), (0, 0, 255));
    assert_eq!(qa4_rgb(qa4_bg("div{background: linear-gradient(red, blue) green !important}")), (0, 128, 0));
}

#[test]
fn qa2_inline_text_keeps_nbsp_collapses_ascii_ws() {
    let n = TextTransform::None;
    assert_eq!(inline_text("a\u{a0}\u{a0}b", WhiteSpace::Normal, n), "a\u{a0}\u{a0}b");
    assert_eq!(inline_text("a  \t\n b", WhiteSpace::Normal, n), "a b");
    assert_eq!(inline_text("\u{a0}", WhiteSpace::Normal, n), "\u{a0}");
    assert_eq!(inline_text(" a\u{a0}", WhiteSpace::Normal, n), " a\u{a0}");
    assert_eq!(inline_text("a\u{a0}\u{a0}b  c", WhiteSpace::PreLine, n), "a\u{a0}\u{a0}b c");
    assert_eq!(normalize_whitespace("\u{a0}x  y\u{a0}"), "\u{a0}x y\u{a0}");
}

#[test]
fn qa2_css_ws_helpers_and_flex_item() {
    assert!(is_css_ws(' ') && is_css_ws('\x0c') && !is_css_ws('\u{a0}'));
    assert_eq!(trim_css_ws(" \u{a0} "), "\u{a0}");
    assert!(qa2_body_h("<div style=\"display:flex\">&nbsp;</div>") >= 11.0);
}

/// QA3: margins on out-of-flow boxes (CSS2.1 §10.3.7 / §10.6.4).
#[test]
fn qa3_abspos_margins_offset_from_insets() {
    let laid = measure_fixture(
        r#"<html><body><div style="position: relative; width: 300px; height: 100px;">
<div style="position: absolute; top: 0; left: 0; margin: 36px 0 0 60px; width: 48px; height: 12px;"></div>
<div style="position: absolute; right: 0; bottom: 0; margin: 0 10px 7px 0; width: 40px; height: 20px;"></div>
</div></body></html>"#,
        400.0,
    );
    let c = find_by_width(&laid, 300.0).expect("container");
    let a = c.positioned.iter().find(|l| l.w == 48.0).expect("a");
    assert_eq!((a.x - c.x, a.y - c.y), (60.0, 36.0));
    let b = c.positioned.iter().find(|l| l.w == 40.0).expect("b");
    assert_eq!((b.x - c.x, b.y - c.y), (300.0 - 10.0 - 40.0, 100.0 - 7.0 - 20.0));
}

#[test]
fn qa3_fixed_margin_and_static_position_margin() {
    let laid = measure_document_fixture(
        r#"<html><body style="margin:0"><div style="height: 50px;"></div>
<div style="position: fixed; top: 0; left: 0; margin: 36px 0 0 60px; width: 48px; height: 12px;"></div>
<div style="position: absolute; margin: 5px 0 0 9px; width: 10px; height: 10px;"></div>
</body></html>"#,
        400.0,
        600.0,
    );
    let f = laid.positioned.iter().find(|l| l.fixed).expect("fixed");
    assert_eq!((f.x, f.y), (60.0, 36.0));
    let s = laid.positioned.iter().find(|l| !l.fixed).expect("static abs");
    assert_eq!((s.x, s.y), (9.0, 55.0), "static position is the margin edge, margins add");
}

#[test]
fn qa3_auto_margins_centre_with_fixed_margins_subtracted() {
    let laid = measure_fixture(
        r#"<html><body><div style="position: relative; width: 300px; height: 100px;">
<div style="position: absolute; left: 0; right: 0; margin-left: auto; margin-right: 20px; width: 80px; height: 10px;"></div>
</div></body></html>"#,
        400.0,
    );
    let c = find_by_width(&laid, 300.0).expect("container");
    let a = c.positioned.first().expect("a");
    assert_eq!(a.x - c.x, 300.0 - 20.0 - 80.0, "left auto margin absorbs all free space");
}

#[test]
fn qa5_data_url_percent_decodes_before_base64() {
    use base64::Engine;
    let bytes: Vec<u8> = (0u8..=255).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let escaped: String = b64.replace('/', "%2F").replace('+', "%2B").replace('=', "%3D");
    assert!(escaped.contains("%2F") && escaped.contains("%3D"));
    let got = decode_data_uri(&format!("data:image/png;base64,{escaped}")).expect("escaped base64 decodes");
    assert_eq!(got, bytes);
    // A real PNG round-trips through the escaped form and decodes to an image.
    let uri = tiny_png_data_uri(2, 2).replace('/', "%2F").replace('=', "%3D");
    assert!(decode_image_bytes(&decode_data_uri(&uri).unwrap()).is_some());
}

#[test]
fn qa5_data_url_forgiving_base64_and_text_forms() {
    // Acid3 T97: spaces and CR/LF inside the payload are dropped, padding optional.
    let t = decode_data_uri_text("data:text/javascript;base64,%20ZD%20Qg%0D%0APS%20An%20Zm91cic%0D%0A%207%20").unwrap();
    assert_eq!(t, "d4 = 'four';");
    assert_eq!(decode_data_uri_text("data:text/css,p%7Bcolor:red%7D").unwrap(), "p{color:red}");
    // Default mediatype, params, charset.
    let d = parse_data_url("data:,a%20b").unwrap();
    assert_eq!((d.mime.as_str(), d.body.as_slice()), ("text/plain", &b"a b"[..]));
    assert_eq!(decode_data_uri_text("data:text/plain;charset=iso-8859-1,caf%E9").unwrap(), "caf\u{e9}");
    assert_eq!(decode_data_uri_text("data:text/plain;charset=utf-8;base64,w6k=").unwrap(), "\u{e9}");
    // Malformed: stray `=`, length 1 mod 4, other scheme, no comma.
    assert!(decode_data_uri("data:;base64,a=bc").is_none());
    assert!(decode_data_uri("data:;base64,abcde").is_none());
    assert!(decode_data_uri("http://x/").is_none());
    assert!(decode_data_uri("data:text/plain").is_none());
}

#[test]
fn qa5_data_script_and_stylesheet_are_decoded_locally() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_js_enabled(true);
    let base = Url::parse("http://example.test/").unwrap();
    let nowhere = std::path::Path::new("/nonexistent");
    let srcs = fetch_scripts(
        nowhere,
        &base,
        &base,
        vec![ScriptRef::External("data:text/javascript;base64,%20ZD%20Qg%0D%0APS%20An%20Zm91cic%0D%0A%207%20".into())],
    );
    assert_eq!(srcs.len(), 1);
    assert_eq!(srcs[0].code, "d4 = 'four';");
    let css = fetch_stylesheets(nowhere, &base, &base, &["data:text/css,p%7Bcolor:red%7D".to_string()]);
    assert!(css.contains("p{color:red}"), "{css:?}");
    // @import of a data: URL inside a stylesheet.
    let out = expand_css_imports("@import url(\"data:text/css,b%7Bx:y%7D\");a{}", &base, &mut vec![base.clone()], &mut |_| None);
    assert!(out.contains("b{x:y}") && out.contains("a{}"), "{out:?}");
}

// ---------- QA6: fragment of the first page load ----------

/// Lays out `app.document` (so node ids match) and delivers it as a
/// `ShapeResult` for the current layout generation.
fn qa6_deliver_layout(app: &mut App, incomplete: bool) {
    let tree = build_layout_tree(&app.document, 400.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let generation = app.layout_generation;
    app.handle_shape_result(generation, Box::new(laid), cache, incomplete);
}

#[test]
fn qa6_fragment_waits_for_real_page_and_target() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.fetch_url(Url::parse("http://127.0.0.1:9/p#sec").unwrap());
    // The "Loading…" page lays out first: it must not consume the fragment.
    qa6_deliver_layout(&mut app, false);
    assert_eq!(app.nav_fragment.as_deref(), Some("sec"), "still held for the real page");
    assert!(app.pending_fragment.is_none());
    let generation = app.nav_generation;
    let page = FetchedPage {
        html: b"<html><body><div style=\"height:900px\">x</div><p id=\"sec\">target</p></body></html>".to_vec(),
        extra_css: String::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/p").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert_eq!(app.pending_fragment.as_deref(), Some("sec"));
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());
    assert!(app.scroll_offset >= 900.0, "scrolled to target, got {}", app.scroll_offset);
    assert!(app.interaction.target.is_some());

    // An existing target on an incomplete layout is applied but kept, so
    // the scroll follows content that is still growing above it.
    app.pending_fragment = Some("sec".into());
    app.scroll_offset = 0.0;
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("sec"), "kept while incomplete");
    assert!(app.scroll_offset >= 900.0);
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());

    // Target absent from an incomplete layout: keep waiting; a complete
    // layout without it gives up.
    app.pending_fragment = Some("late".into());
    app.scroll_offset = 0.0;
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("late"), "incomplete layout keeps waiting");
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none(), "complete layout without the id gives up");

    // Target that appears only in a later layout generation.
    app.pending_fragment = Some("late".into());
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("late"));
    let doc = parse_html("<html><body><div style=\"height:700px\">x</div><p id=\"late\">t</p></body></html>");
    app.replace_document(doc, String::new());
    app.pending_fragment = Some("late".into());
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());
    assert!(app.scroll_offset >= 700.0, "got {}", app.scroll_offset);
}

/// Every command the command bar accepts (`COMMAND_NAMES`, also what Tab
/// completes) must be documented on the `:help` page. Fails when a new
/// command is added without a row in `src/help.html`.
#[test]
fn help_page_documents_every_command() {
    for name in COMMAND_NAMES {
        let needle = format!(":{name}");
        assert!(
            HELP_PAGE_HTML.contains(&needle),
            "`:{name}` is a command-bar command but is not documented in src/help.html"
        );
    }
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

// ---------- QY3: HTMLCanvasElement ----------

#[test]
fn qy3_canvas_element_interface_without_drawing() {
    let (log, _body) = mutate(
        "<html><body><canvas id=c width=40></canvas></body></html>",
        "var c=document.createElement('canvas'); var d=document.createElementNS('http://www.w3.org/1999/xhtml','canvas');\
         console.log(c.getContext('2d'), d.getContext('webgl'), c.width, c.height, c instanceof HTMLCanvasElement, c instanceof HTMLElement);\
         c.width=20; console.log(c.width, c.getAttribute('width'), c.toDataURL());\
         var p=document.getElementById('c'); console.log(p.width, p.height, p.getContext('2d'), typeof p.toBlob);",
    );
    assert_eq!(log, ["log:null null 300 150 true true", "log:20 20 data:,", "log:40 150 null function"], "{log:?}");
}

#[test]
fn qy4_create_event_and_constructor_globals() {
    let log = run_collecting(&[src(
        "var e=document.createEvent('UIEvents');e.initUIEvent('t',true,false,null,6);\
         var d=-1;document.addEventListener('t',function(x){d=x.detail+':'+x.bubbles;});document.dispatchEvent(e);console.log(d);\
         var m=document.createEvent('MouseEvents');m.initMouseEvent('c',true,true,window,1,0,0,7,8,false,false,false,false,0,null);console.log(m.clientX+','+m.clientY+','+(m instanceof MouseEvent));\
         var c=document.createEvent('CustomEvent');c.initCustomEvent('k',false,false,{a:1});console.log(c.detail.a);\
         var v=document.createEvent('Event');v.initEvent('z',true,true);console.log(v.type+v.cancelable);\
         try{document.createEvent('Bogus')}catch(x){console.log(x.name)}\
         console.log(['DocumentFragment','CharacterData','Text','Comment','Attr','NodeList','HTMLCollection','ErrorEvent'].every(function(n){return typeof window[n]==='function'}));\
         var er=new ErrorEvent('error',{message:'m',filename:'f',lineno:3,colno:4,error:5});console.log(er.message+er.filename+er.lineno+er.colno+er.error);\
         console.log((document.body.ownerDocument===document)+','+(document.ownerDocument===null)+','+(document.defaultView===window));\
         console.log((document.createTextNode('x') instanceof Text)+','+(document.createComment('x') instanceof Comment)+','+(document.createDocumentFragment() instanceof DocumentFragment)+','+(document.createTextNode('x') instanceof CharacterData));\
         console.log(NodeFilter.SHOW_ELEMENT+','+NodeFilter.FILTER_ACCEPT+','+NodeFilter.SHOW_ALL);",
    )]);
    assert_eq!(log, ["log:6:true", "log:7,8,true", "log:1", "log:ztrue", "log:NotSupportedError",
        "log:true", "log:mf345", "log:true,true,true", "log:true,true,true,true", "log:1,1,4294967295"]);
}

/// QA17 fixture: root div#R > a(span) > ["x", b], c(i), "t", d(p) > e(em).
const QA17_TREE: &str = "var R=document.createElement('div');R.id='R';\
    function mk(t,id){var e=document.createElement(t);e.id=id;return e}\
    var a=mk('span','a'),b=mk('b','b'),c=mk('i','c'),d=mk('p','d'),e=mk('em','e');\
    a.appendChild(document.createTextNode('x'));a.appendChild(b);R.appendChild(a);R.appendChild(c);\
    R.appendChild(document.createTextNode('t'));d.appendChild(e);R.appendChild(d);\
    function nm(n){return n?(n.id||('#'+n.nodeValue)):'null'}";

#[test]
fn qa17_tree_walker_show_element_walks_forward_and_back() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT,null);\
         var f=[],n;while((n=w.nextNode()))f.push(n.id);console.log(f.join(','));\
         var r=[];while((n=w.previousNode()))r.push(n.id);console.log(r.join(','));\
         console.log(w.root===R, w.whatToShow, w.filter===null, w.currentNode===R);"
    ))]);
    assert_eq!(log, ["log:a,b,c,d,e", "log:d,c,b,a,R", "log:true 1 true true"], "{log:?}");
}

#[test]
fn qa17_tree_walker_filter_skip_vs_reject() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} function walk(code){{var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT,\
           function(n){{return n.id==='a'?code:NodeFilter.FILTER_ACCEPT}});var o=[],n;while((n=w.nextNode()))o.push(n.id);return o.join(',')}}\
         console.log(walk(NodeFilter.FILTER_SKIP));console.log(walk(NodeFilter.FILTER_REJECT));\
         var w=document.createTreeWalker(R,NodeFilter.SHOW_ALL,{{acceptNode:function(n){{return n.nodeType===3?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}}}});\
         var o=[],n;while((n=w.nextNode()))o.push(nm(n));console.log(o.join(','));"
    ))]);
    assert_eq!(log, ["log:b,c,d,e", "log:c,d,e", "log:a,b,c,d,e"], "{log:?}");
}

#[test]
fn qa17_tree_walker_current_node_and_relatives() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT);\
         var o=[];o.push(nm(w.firstChild()));o.push(nm(w.nextSibling()));o.push(nm(w.nextSibling()));\
         o.push(nm(w.nextSibling()));o.push(nm(w.previousSibling()));o.push(nm(w.lastChild()));o.push(nm(w.parentNode()));\
         o.push(w.currentNode===d);w.currentNode=b;o.push(nm(w.nextNode()));o.push(nm(w.parentNode()));\
         w.currentNode=R;o.push(nm(w.nextSibling()));o.push(nm(w.parentNode()));\
         w.currentNode=d;o.push(nm(w.nextSibling()));w.currentNode=e;o.push(nm(w.nextNode()));\
         console.log(o.join(','));"
    ))]);
    assert_eq!(log, ["log:a,c,d,null,c,null,R,false,c,R,null,null,null,null"], "{log:?}");
}

#[test]
fn qa17_node_iterator_next_and_previous() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var it=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT|NodeFilter.SHOW_TEXT);\
         console.log(it.root===R, it.whatToShow, it.referenceNode===R, it.pointerBeforeReferenceNode);\
         var f=[],n;while((n=it.nextNode()))f.push(nm(n));console.log(f.join(','));\
         console.log(it.pointerBeforeReferenceNode, nm(it.referenceNode));\
         var r=[];while((n=it.previousNode()))r.push(nm(n));console.log(r.join(','));\
         console.log(typeof it.detach, it.detach());\
         var o=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT,function(n){{return n.id==='b'?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}});\
         var g=[];while((n=o.nextNode()))g.push(n.id);console.log(g.join(','));"
    ))]);
    assert_eq!(log, ["log:true 5 true true", "log:R,a,#x,b,c,#t,d,e", "log:false e",
        "log:e,d,#t,c,b,#x,a,R", "log:function undefined", "log:R,a,c,d,e"], "{log:?}");
}

#[test]
fn qa17_node_iterator_survives_removal_and_filter_reentry_throws() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var it=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT);\
         it.nextNode();it.nextNode();it.nextNode();console.log(nm(it.referenceNode));\
         R.removeChild(a);console.log(nm(it.referenceNode),it.pointerBeforeReferenceNode,nm(it.nextNode()));\
         var w;w=document.createTreeWalker(R,NodeFilter.SHOW_ALL,function(){{try{{w.nextNode()}}catch(x){{console.log(x.name)}}return 1}});w.nextNode();\
         try{{document.createTreeWalker()}}catch(x){{console.log(x.name)}}"
    ))]);
    assert_eq!(log, ["log:b", "log:R false c", "log:InvalidStateError", "log:TypeError"], "{log:?}");
}

/// QY5: a runaway page-load script is stopped with one clear line naming
/// the script and the budget, not a bare `InternalError: interrupted`.
#[test]
fn qy5_interrupted_page_script_logs_a_clear_message() {
    let log = run_collecting(&[ScriptSource { name: "big.js".into(), code: "for(;;){}".into(), index: None, ..Default::default() }, src("console.log('after');")]);
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("big.js stopped: exceeded the 5 s"), "{log:?}");
    assert!(errs[0].contains("budget") && errs[0].contains("KB of source"), "{log:?}");
    assert!(!errs[0].contains("InternalError"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// QY5: same for a timer callback, with the callback budget.
#[test]
fn qy5_interrupted_callback_logs_a_clear_message() {
    let log = run_with_timers("setTimeout(()=>{ for(;;){} },0);");
    assert!(log.iter().any(|l| l.starts_with("error:js: timer stopped: exceeded the 500 ms")), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("InternalError")), "{log:?}");
}

/// QY5: the budget clock is thread CPU time: it advances when the thread
/// works and not while it sleeps (a slow-to-compile or merely waiting
/// script on a loaded machine is not killed for wall-clock time).
#[test]
fn qy5_budget_clock_is_thread_cpu_time() {
    let a = crate::js::thread_cpu_nanos();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let slept = crate::js::thread_cpu_nanos() - a;
    assert!(slept < 100_000_000, "sleeping used {slept} ns of CPU");
    let t = std::time::Instant::now();
    let mut x = 0u64;
    while t.elapsed() < std::time::Duration::from_millis(100) {
        x = x.wrapping_add(1);
        std::hint::black_box(x);
    }
    let spun = crate::js::thread_cpu_nanos() - a - slept;
    assert!(spun > 20_000_000, "spinning used {spun} ns of CPU");
}

#[test]
fn absolutize_css_urls_resolves_relative_against_sheet_q35a() {
    let sheet = Url::parse("https://en.wikipedia.org/w/load.php?x=1").unwrap();
    let css = ".a{background:url(img/a.png)} .b{mask-image:url( '../m.svg' )} .c{background-image:url(\"data:image/png;base64,AAAA\")} .d{background:url(https://other/z.png)} .e{filter:url(#f)} .f{background:URL(/abs.png) no-repeat} .g{x:myurl(rel)}";
    let out = absolutize_css_urls(css, &sheet);
    assert!(out.contains("url(\"https://en.wikipedia.org/w/img/a.png\")"), "{out}");
    assert!(out.contains("url(\"https://en.wikipedia.org/m.svg\")"), "{out}");
    assert!(out.contains("url(\"data:image/png;base64,AAAA\")"), "{out}");
    assert!(out.contains("url(https://other/z.png)"), "{out}");
    assert!(out.contains("url(#f)"), "{out}");
    assert!(out.contains("url(\"https://en.wikipedia.org/abs.png\") no-repeat"), "{out}");
    assert!(out.contains("myurl(rel)"), "{out}");
    // a data: sheet has no base: left alone
    let data = Url::parse("data:text/css,").unwrap();
    assert_eq!(absolutize_css_urls(css, &data), css);
    // imported sheets resolve against their own url
    let mut stack = vec![sheet.clone()];
    let expanded = expand_css_imports("@import url(sub/x.css);\n.k{background:url(k.png)}", &sheet, &mut stack, &mut |u| {
        assert_eq!(u.as_str(), "https://en.wikipedia.org/w/sub/x.css");
        Some(".i{background:url(i.png)}".to_string())
    });
    assert!(expanded.contains("https://en.wikipedia.org/w/sub/i.png"), "{expanded}");
    assert!(expanded.contains("https://en.wikipedia.org/w/k.png"), "{expanded}");
}

#[test]
fn external_background_and_mask_urls_use_image_cache_q35a() {
    let _ = take_wanted_css_images();
    let doc = parse_html(r#"<html><body><span class="s">x</span><i class="m">y</i></body></html>"#);
    let rules = parse_stylesheet(
        r#".s{background-image:url("https://h.test/sprite.svg")} .m{background-color:red;mask-image:url("https://h.test/icon.png")}"#,
    );
    let s = find_element_by_tag(&doc, "span").unwrap();
    let m = find_element_by_tag(&doc, "i").unwrap();
    let mut cache: HashMap<String, Arc<DecodedImage>> = HashMap::new();
    // Not fetched yet: nothing resolves, but both urls are reported as wanted.
    assert!(resolve_background_image(&s, &rules, 800.0, None, &cache).is_none());
    assert!(resolve_background_image(&m, &rules, 800.0, None, &cache).is_none());
    let mut wanted = take_wanted_css_images();
    wanted.sort();
    assert_eq!(wanted, ["https://h.test/icon.png", "https://h.test/sprite.svg"]);
    // Fetched: the cached images resolve, as a background and as a mask.
    let img = decode_image_bytes(&decode_data_uri(&tiny_png_data_uri(3, 2)).unwrap()).unwrap();
    cache.insert("https://h.test/sprite.svg".into(), Arc::new(img.clone()));
    cache.insert("https://h.test/icon.png".into(), Arc::new(img));
    let bg = resolve_background_image(&s, &rules, 800.0, None, &cache).expect("cached background resolves");
    assert_eq!((bg.width, bg.height), (3, 2));
    assert!(bg.mask.is_none());
    let masked = resolve_background_image(&m, &rules, 800.0, None, &cache).expect("cached mask resolves");
    assert!(masked.mask.is_some(), "mask-image url must come from the cache");
    assert!(take_wanted_css_images().is_empty());
}

#[test]
fn layout_build_reports_wanted_css_images_and_uses_cache_q35a() {
    let _ = take_wanted_css_images();
    let doc = parse_html(r#"<html><body><div style="width:20px;height:20px;background-image:url(http://h.test/a.png)">x</div></body></html>"#);
    let mut cache: HashMap<String, Arc<DecodedImage>> = HashMap::new();
    let _ = build_layout_tree_with_resources(&doc, 400.0, Some(300.0), "", &cache);
    assert_eq!(take_wanted_css_images(), ["http://h.test/a.png"]);
    let img = decode_image_bytes(&decode_data_uri(&tiny_png_data_uri(2, 2)).unwrap()).unwrap();
    cache.insert("http://h.test/a.png".into(), Arc::new(img));
    let tree = build_layout_tree_with_resources(&doc, 400.0, Some(300.0), "", &cache);
    assert!(take_wanted_css_images().is_empty());
    fn any_bg(b: &LayoutBox) -> bool {
        b.background_image.is_some()
            || match &b.content {
                BoxContent::Blocks(c) => c.iter().any(any_bg),
                _ => false,
            }
    }
    assert!(any_bg(&tree), "the cached image must be attached to the div's box");
}

// ---------- QW2: `text-indent` (wikipedia.org search-button label) ----------

fn red_pixel_count_of(html: &str) -> usize {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, 300.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    );
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (300u32, 100u32);
    let mut pixels = vec![0xFFFFFFu32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    pixels.iter().filter(|&&p| ((p >> 16) & 0xFF) > 200 && ((p >> 8) & 0xFF) < 80 && (p & 0xFF) < 80).count()
}

#[test]
fn negative_text_indent_moves_the_label_off_the_icon_qw2() {
    // wikipedia.org: `<button><i class="sprite svg-search-icon">Search</i></button>`
    // with `.svg-search-icon{width:22px;height:22px;display:inline-block}` and
    // `button .svg-search-icon{text-indent:-9999px}`: the label must be hidden
    // off to the left, not spill out of the 22px icon box ("Searc" clipped).
    let page = |indent: &str| {
        format!(
            r#"<html><body><button style="white-space:nowrap;font-weight:700;font-style:italic;color:#f00;padding:8px 16px"><i style="display:inline-block;width:22px;height:22px;vertical-align:middle;{indent}">Search</i></button></body></html>"#
        )
    };
    let without = red_pixel_count_of(&page(""));
    let with = red_pixel_count_of(&page("text-indent:-9999px"));
    assert!(without > 20, "control: the label must paint without text-indent (got {without} red px)");
    assert_eq!(with, 0, "text-indent:-9999px must push the first line off-screen (got {with} red px)");
}

#[test]
fn negative_text_indent_applies_on_the_mixed_inline_path_too_qw2() {
    // wikipedia.org has a float earlier in the BFC, which routes the label
    // through `measure_mixed_inline` (per-word `Word` items) instead of the
    // single-buffer path; the indent must apply there as well.
    let page = |indent: &str| {
        format!(
            r#"<html><body><div style="float:left;width:10px;height:200px"></div><button style="white-space:nowrap;font-weight:700;font-style:italic;color:#f00;padding:8px 16px"><i style="display:inline-block;width:22px;height:22px;vertical-align:middle;{indent}">Search</i></button></body></html>"#
        )
    };
    let without = red_pixel_count_of(&page(""));
    let with = red_pixel_count_of(&page("text-indent:-9999px"));
    assert!(without > 20, "control: the label must paint without text-indent (got {without} red px)");
    assert_eq!(with, 0, "text-indent:-9999px must push the first mixed-inline line off-screen (got {with} red px)");
}

// ---------- Q35b3: HN vote arrow column ----------

fn hn_vote_row_html(arrow_display: &str) -> String {
    format!(
        r#"<style>.votearrow{{width:10px;height:10px;border:0;margin:3px 2px 6px;background:url('grayarrow.gif') no-repeat;{arrow_display}}} td{{font-size:10pt}}</style>
<table border="0" cellpadding="0" cellspacing="0" width="85%"><tr class="athing"><td valign="top" class="votelinks"><center><a href="vote?id=1"><div class="votearrow" title="upvote"></div></a></center></td><td class="title"><span class="titleline"><a href="https://example.com/">Old Bitly Links Are Being Hijacked</a></span></td></tr></table>"#
    )
}

#[test]
fn hn_vote_arrow_div_in_center_anchor_gets_its_own_column_wide_enough() {
    // Real HN: block-level `div.votearrow` (10px + 2px side margins) inside
    // `<center><a>` in a narrow cell; the column must fit its margin box and
    // the title cell must start to its right (Q35b3).
    for d in ["", "display:inline-block"] {
        let laid = measure_fixture(&hn_vote_row_html(d), 900.0);
        let cells = find_table_row_cells(&laid).expect("table row not found");
        assert_eq!(cells.len(), 2);
        assert!(cells[0].w >= 14.0 - 0.01, "vote column {} fits 10px + 2*2px margins ({d})", cells[0].w);
        assert!(cells[1].x >= cells[0].x + 14.0 - 0.01, "title cell x {} starts right of the arrow ({d})", cells[1].x);
    }
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

// ---------- Q52b: `var()` declarations respect source order ----------

#[test]
fn var_declaration_does_not_override_later_plain_declaration() {
    // `border-style: var(--s)` is expanded per element; it used to land after
    // every plain declaration of the rule, so it beat the later
    // `border-top: none`.
    let laid = measure_fixture(
        r#"<html><head><style>
        #a { --s: solid; width: 301px; border-style: var(--s); border-width: 2px; border-top: none; }
        #b { --s: solid; width: 302px; border-top: none; border-style: var(--s); border-width: 2px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#,
        900.0,
    );
    let a = find_by_width(&laid, 301.0).expect("a").style;
    assert_eq!(a.border_width, [0.0, 2.0, 2.0, 2.0], "plain border-top:none comes after the var()");
    let b = find_by_width(&laid, 302.0).expect("b").style;
    assert_eq!(b.border_width, [2.0; 4], "var() after border-top:none re-enables the style");
}

#[test]
fn var_declarations_interleave_with_plain_ones_in_source_order() {
    // plain, var, plain, var, plain: each plain run beats the earlier var()
    // and loses to the later one.
    let laid = measure_fixture(
        r#"<html><head><style>
        #a { --w: 5px; --h: 7px; width: 1px; width: var(--w); height: 2px; width: 303px; height: var(--h); height: 9px; }
        </style></head><body><div id="a">x</div></body></html>"#,
        900.0,
    );
    let a = find_by_width(&laid, 303.0).expect("plain width after var() wins");
    assert_eq!(a.style.height.map(BoxLength::px), Some(9.0), "plain height after the last var() wins");
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

// ---------- QA11a: border width defaults, keywords, currentColor ----------

#[test]
fn border_width_defaults_to_medium_and_keywords_parse() {
    let div_of = |style: &str, width: f32| {
        let html = format!(r#"<html><body><div style="width: {width}px; {style}">x</div></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        find_by_width(&laid, width).expect("div not found").style
    };
    let s = div_of("border-top: solid black;", 311.0);
    assert_eq!(s.border_width, [3.0, 0.0, 0.0, 0.0], "style without a width is medium (3px)");
    let s = div_of("border: black solid medium;", 312.0);
    assert_eq!(s.border_width, [3.0; 4], "`medium` is parsed, not dropped");
    let s = div_of("border-style: solid; border-width: thin thick 7px medium;", 313.0);
    assert_eq!(s.border_width, [1.0, 5.0, 7.0, 3.0]);
    let s = div_of("border: 4px blue;", 314.0);
    assert_eq!(s.border_width, [0.0; 4], "no style stays none");
}

#[test]
fn border_color_current_color_resolves_to_own_color() {
    let laid = measure_fixture(
        r#"<html><head><style>
        .p { color: red; border: none; }
        .c { width: 315px; color: green; border-color: inherit; border-style: solid; }
        .d { width: 316px; color: blue; border: 2px solid currentColor; }
        </style></head><body><div class="p"><div class="c">x</div></div><div class="d">y</div></body></html>"#,
        500.0,
    );
    let c = find_by_width(&laid, 315.0).expect("c").style;
    assert_eq!(c.border_color[0], Some(TextColor::rgb(0, 0x80, 0)), "inherited currentColor uses the child's colour");
    let d = find_by_width(&laid, 316.0).expect("d").style;
    assert_eq!(d.border_color[0], Some(TextColor::rgb(0, 0, 0xff)));
}

#[test]
fn qa14_domexception_constructor_and_constants() {
    let (log, _) = mutate(
        "<html><body></body></html>",
        "var e=new DOMException('m','NotFoundError');\
         console.log(e.name, e.message, e.code, e instanceof Error, e instanceof DOMException, String(e));\
         console.log(DOMException.INVALID_CHARACTER_ERR, DOMException.prototype.HIERARCHY_REQUEST_ERR, DOMException.NAMESPACE_ERR, DOMException.DATA_CLONE_ERR);\
         console.log(new DOMException('x').code, new DOMException('x').name);",
    );
    assert_eq!(log, ["log:NotFoundError m 8 true true NotFoundError: m", "log:5 3 14 25", "log:0 Error"], "{log:?}");
}

#[test]
fn qa14_dom_methods_throw_domexception() {
    let (log, _) = mutate(
        "<html><body><div id=a><span id=s></span></div><p id=p></p></body></html>",
        "var t=function(f){try{f();console.log('none')}catch(e){console.log(e.name+' '+e.code+' '+(e instanceof DOMException))}};\
         var a=document.getElementById('a'), s=document.getElementById('s'), p=document.getElementById('p');\
         t(function(){document.createElement('<bad>')});\
         t(function(){document.createElement('')});\
         t(function(){s.appendChild(a)});\
         t(function(){a.removeChild(p)});\
         t(function(){a.insertBefore(p, p)});\
         t(function(){document.querySelector('a[')});\
         t(function(){p.setAttribute('a b','x')});\
         t(function(){document.createElementNS('http://www.w3.org/2000/svg','a:b:c')});\
         t(function(){document.createElementNS(null,'a:b')});\
         t(function(){document.createEvent('Bogus')});\
         t(function(){document.createElement('ok-name')});\
         console.log(document.createElementNS('http://www.example.org/ns','PREFIX:LOCALNAME').tagName);",
    );
    assert_eq!(
        log,
        ["log:InvalidCharacterError 5 true", "log:InvalidCharacterError 5 true", "log:HierarchyRequestError 3 true",
         "log:NotFoundError 8 true", "log:NotFoundError 8 true", "log:SyntaxError 12 true",
         "log:InvalidCharacterError 5 true", "log:NamespaceError 14 true", "log:NamespaceError 14 true",
         "log:NotSupportedError 9 true", "log:none", "log:PREFIX:LOCALNAME"],
        "{log:?}"
    );
}

// ---------- QA16: getComputedStyle ----------

#[test]
fn qa16_get_computed_style_cascade_inheritance_and_unknowns() {
    let (_e, _d, log) = events_engine(
        "<html><head><style>#a{white-space:pre-wrap;position:relative;z-index:3;display:flex;clear:both}\
         .c{color:#102030;font-size:2em;opacity:.5;margin-left:7px;border:2px solid red}\
         #k{white-space:pre-line;white-space:x-bogus}</style></head>\
         <body style='font-size:10px'><div id=a>x</div><p id=b class=c style='background-color:rgb(1,2,3)'>y<span id=s>z</span></p>\
         <div id=h style='display:none'></div><i id=k></i></body></html>",
        "var a=document.getElementById('a'), b=document.getElementById('b'), cs=getComputedStyle(a);\
         console.log(cs.whiteSpace, cs.zIndex, cs.display, cs.cssFloat, cs['float'], cs.position, cs.getPropertyValue('white-space'));\
         var cb=getComputedStyle(b);\
         console.log(cb.color, cb.fontSize, cb.opacity, cb.marginLeft, cb.borderTopWidth, cb.getPropertyValue('border-top-style'), cb['background-color'], cb.zIndex, cb.display);\
         console.log(getComputedStyle(document.getElementById('s')).color, getComputedStyle(document.getElementById('s')).fontSize);\
         console.log(getComputedStyle(document.getElementById('h')).display, JSON.stringify(cs.bogus), JSON.stringify(cs.getPropertyValue('bogus')), cs.length > 10, typeof cs.item(0));\
         b.style.display='none'; console.log(getComputedStyle(b).display);\
         var d=document.createElement('div'); console.log(JSON.stringify(getComputedStyle(d).display), window.document.defaultView === window);\
         console.log(getComputedStyle(document.getElementById('k')).whiteSpace); try { getComputedStyle(null) } catch (e) { console.log('te', e instanceof TypeError) }",
    );
    let l = log.borrow();
    assert_eq!(l[0], "log:pre-wrap 3 flex none none relative pre-wrap", "{l:?}");
    assert_eq!(l[1], "log:rgb(16, 32, 48) 20px 0.5 7px 2px solid rgb(1, 2, 3) auto block", "{l:?}");
    assert_eq!(l[2], "log:rgb(16, 32, 48) 20px", "{l:?}");
    assert_eq!(l[3], "log:none \"\" \"\" true string", "{l:?}");
    assert_eq!(l[4], "log:none", "write invalidates the cache: {l:?}");
    assert_eq!(l[5], "log:\"\" true", "{l:?}");
    assert_eq!(l[6], "log:pre-line", "invalid later keyword must not override: {l:?}");
    assert_eq!(l[7], "log:te true", "{l:?}");
}

// QW3: wikipedia.org language-button divider. A later z-auto relative 1px
// line pulled up by a negative margin must be hidden by the earlier
// z-index:1 inline button's white background / outline, not drawn over it.
#[test]
fn qw3_divider_line_is_hidden_behind_z_indexed_inline_button() {
    let html = r#"<html><body style="margin:0;background:#fff"><div style="text-align:center;font-size:14px"><button style="display:inline;position:relative;z-index:1;margin:0 auto;padding:10px 20px;outline:20px solid #fff;border:2px solid #0000ff;font-family:inherit;background-color:#f8f9fa;font-size:14px;line-height:1"><i style="display:inline-block;vertical-align:middle;width:10px;height:10px"></i><span style="padding:0 6px;vertical-align:middle">Read</span><i style="display:inline-block;vertical-align:middle;width:10px;height:10px"></i></button></div><div style="background-color:#ff0000;display:block;position:relative;width:80%;margin:-20px auto 20px;height:1px"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    let mut red_in_button = 0;
    for x in 50..150u32 {
        for y in 0..40u32 {
            if px_at(&pixels, win_w, x, y) == 0x00FF_0000 {
                red_in_button += 1;
            }
        }
    }
    assert_eq!(red_in_button, 0, "divider must not paint over the button");
    // A `<button>` with `display:inline` computes to inline-block, so its
    // border paints (a plain inline box paints none).
    let mut blue = 0;
    for x in 0..200u32 {
        for y in 0..60u32 {
            if px_at(&pixels, win_w, x, y) == 0x0000_00FF {
                blue += 1;
            }
        }
    }
    assert!(blue > 20, "button border must paint, got {blue} blue pixels");
}

// ---------- QA20: table and form DOM interfaces ----------

#[test]
fn qa20_table_rows_cells_and_sections() {
    let log = run_in_html(
        "<html><body><table id=t><caption>c</caption><thead><tr><th>h</th></tr></thead>\
         <tbody><tr id=r1><td>a</td><td colspan=3 rowspan=2>b</td></tr><tr id=r2><td>c</td></tr></tbody>\
         <tfoot><tr><td>f</td></tr></tfoot></table></body></html>",
        "var t=document.getElementById('t'), r2=document.getElementById('r2'), r1=document.getElementById('r1');\
         console.log(t.rows.length, t.tBodies.length, t.tHead.tagName, t.tFoot.tagName, t.caption.tagName);\
         console.log(r2.rowIndex, r2.sectionRowIndex, r1.cells.length, r1.cells[1].cellIndex, r1.cells[1].colSpan, r1.cells[1].rowSpan, r1.cells[0].colSpan);\
         console.log(t.rows[0].parentNode.tagName, t.rows[3].parentNode.tagName, t.rows.item(1)===r1, t instanceof HTMLTableElement, r1 instanceof HTMLTableRowElement, r1.cells[0] instanceof HTMLTableCellElement);",
    );
    assert_eq!(log, ["log:4 1 THEAD TFOOT CAPTION", "log:2 1 2 1 3 2 1", "log:THEAD TFOOT true true true true"], "{log:?}");
}

#[test]
fn qa20_table_insert_delete_and_create() {
    let log = run_in_html(
        "<html><body><table id=t></table></body></html>",
        "var t=document.getElementById('t');\
         var r=t.insertRow(); var c=r.insertCell(); c.textContent='x'; r.insertCell(0).textContent='y';\
         console.log(t.rows.length, t.tBodies.length, r.cells.length, r.cells[0].textContent, r.cells[1].textContent);\
         t.insertRow(0); console.log(t.rows.length, t.rows[1]===r);\
         t.deleteRow(0); console.log(t.rows.length, t.rows[0]===r); r.deleteCell(-1); console.log(r.cells.length);\
         var h=t.createTHead(); console.log(h===t.createTHead(), t.tHead===h, t.firstChild===h);\
         var cap=t.createCaption(); console.log(t.firstChild===cap, t.caption===cap);\
         var f=t.createTFoot(); console.log(t.lastChild===f); t.deleteTHead(); t.deleteCaption(); t.deleteTFoot();\
         console.log(t.tHead, t.caption, t.tFoot);\
         try { t.insertRow(9); } catch (e) { console.log(e.name); }\
         try { t.deleteRow(9); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        ["log:1 1 2 y x", "log:2 true", "log:1 true", "log:1", "log:true true true", "log:true true", "log:true", "log:null null null", "log:IndexSizeError", "log:IndexSizeError"],
        "{log:?}"
    );
}

#[test]
fn qa20_forms_collection_and_elements_named_access() {
    let log = run_in_html(
        "<html><body><form id=f1 name=first action=/go method=post><input name=a value=1><select name=s><option>x<option selected value=v>y</select>\
         <textarea name=ta>hi</textarea><button name=b>go</button><input type=image name=img></form>\
         <form id=f2><input id=z></form></body></html>",
        "var f=document.forms[0];\
         console.log(document.forms.length, f.id, document.forms.f2.id, document.forms.namedItem('first')===f, f.length, f.elements.length);\
         console.log(f.elements.a.value, f.elements.namedItem('s').tagName, f.elements[2].name, f.elements.img, f.action, f.method);\
         console.log(document.forms.f2.elements.z.form===document.forms.f2, typeof f.submit, typeof f.reset);",
    );
    assert_eq!(log, ["log:2 f1 f2 true 4 4", "log:1 SELECT ta undefined /go post", "log:true function function"], "{log:?}");
}

#[test]
fn qa20_form_control_reflections() {
    let log = run_in_html(
        "<html><body><form id=f><input id=i name=n value=v><input id=c type=checkbox checked><input id=bad type=bogus>\
         <button id=b></button><button id=b2 type=button></button>\
         <select id=s><option id=o1>one</option><option id=o2 value=2 selected>two</option></select>\
         <label id=l for=i>L</label><textarea id=ta>t</textarea><input id=d disabled></form></body></html>",
        "var $=function(i){return document.getElementById(i)};\
         console.log($('i').type, $('i').name, $('i').value, $('c').type, $('c').checked, $('bad').type, $('d').disabled, $('i').form===$('f'));\
         console.log($('b').type, $('b2').type, $('s').type, $('s').options.length, $('s').selectedIndex, $('s').value, $('o1').value, $('o2').value, $('o2').selected, $('o2').defaultSelected, $('o2').index);\
         $('s').selectedIndex=0; console.log($('s').value, $('o2').selected);\
         $('i').type='password'; $('i').name='m'; $('c').checked=false; console.log($('i').getAttribute('type'), $('i').name, $('c').checked);\
         console.log($('l').htmlFor, $('l').control===$('i'), $('ta').value, $('ta').type);\
         var o=document.createElement('option'); o.text='three'; $('s').add(o); console.log($('s').length, $('s').options[2].value);",
    );
    assert_eq!(
        log,
        [
            "log:text n v checkbox true text true true",
            "log:submit button select-one 2 1 2 one 2 true true 1",
            "log:one false",
            "log:password m false",
            "log:i true t textarea",
            "log:3 three"
        ],
        "{log:?}"
    );
}

#[test]
fn qa20_map_areas_and_form_global_constructors() {
    let log = run_in_html(
        "<html><body><map id=m name=mm><area id=a1 shape=rect coords='0,0,1,1' href=x><area id=a2></map><img usemap='#mm'></body></html>",
        "var m=document.getElementById('m');\
         console.log(m.areas.length, m.areas[0].shape, m.areas[0].coords, m.name, m.images.length, m instanceof HTMLMapElement, m instanceof HTMLElement, typeof HTMLFormElement);",
    );
    assert_eq!(log, ["log:2 rect 0,0,1,1 mm 1 true true function"], "{log:?}");
}

#[test]
fn qa20_input_value_property_is_separate_from_the_attribute_and_meta_http_equiv() {
    let log = run_in_html(
        "<html><body></body></html>",
        "var i=document.createElement('input'); i.value='test';\
         console.log(i.hasAttribute('value'), i.value, i.defaultValue);\
         var m=document.createElement('meta'); m.setAttribute('http-equiv','x'); console.log(m.httpEquiv, m.hasAttribute('httpEquiv'));",
    );
    assert_eq!(log, ["log:false test ", "log:x false"], "{log:?}");
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
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    // Without a display, only the pure decision above can be checked.
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
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

// ---------- QA18b: iframe.contentDocument / contentWindow ----------

#[test]
fn qa18b_iframe_content_document_and_window() {
    let log = run_with_timers(
        "var f=document.createElement('iframe');\
         console.log(f.contentDocument, f.contentWindow);\
         f.onload=function(){ console.log('onload', this===f); };\
         f.addEventListener('load', function(){ console.log('listener'); });\
         document.body.appendChild(f);\
         var d=f.contentDocument; var w=f.contentWindow;\
         console.log(d!==document, d.nodeType, d.documentElement.tagName, d.body.tagName, d.head.tagName,\
           w.document===d, d.defaultView===w, w.parent===window, w.top===window, w.frameElement===f, f.contentDocument===d);\
         var e=d.createElement('p'); d.body.appendChild(e); console.log(d.getElementsByTagName('p').length, document.getElementsByTagName('p').length, e.ownerDocument===d);\
         var n=document.createElement('iframe'); n.src='http://example.org/x.html'; document.body.appendChild(n);\
         console.log(n.contentDocument, n.contentWindow);\
         var u=document.createElement('iframe'); u.setAttribute('src','about:blank'); document.body.appendChild(u);\
         var dd=document.createElement('iframe'); dd.src='data:text/html,hi'; document.body.appendChild(dd);\
         console.log(u.contentDocument.nodeType, dd.contentDocument.nodeType, f instanceof HTMLIFrameElement);\
         var r=document.createElement('iframe'); document.body.appendChild(r); document.body.removeChild(r); console.log(r.contentDocument);\
         console.log('sync-end');",
    );
    assert_eq!(
        log,
        [
            "log:null null",
            "log:true 9 HTML BODY HEAD true true true true true true",
            "log:1 0 true",
            "log:null null",
            "log:9 9 true",
            "log:null",
            "log:sync-end",
            "log:listener",
            "log:onload true",
        ],
        "{log:?}"
    );
}

// ---------- QA18b1 / QA18c: iframe src documents, styles in second documents ----------

#[test]
fn qa18b1_iframe_src_document_is_loaded_and_styled() {
    let mut pages = HashMap::new();
    pages.insert(
        "http://qa18b1.test/dir/empty.html".to_string(),
        "<!doctype html><html><head><style>p{white-space:pre;color:#102030}</style><title>T</title></head><body><p id=a>hi</p></body></html>".to_string(),
    );
    crate::net::set_iframe_pages(pages);
    crate::js::set_page_url(Url::parse("http://qa18b1.test/dir/page.html").ok().as_ref());
    let (_e, _d, log) = events_engine(
        "<html><body><iframe id=f src='empty.html'></iframe><iframe id=g src='missing.html'></iframe>\
         <iframe id=h src='http://other.test/empty.html'></iframe></body></html>",
        "var f=document.getElementById('f'), d=f.contentDocument, w=f.contentWindow;\
         console.log(d!==null, d.title, d.getElementById('a').textContent, d.URL, w.location.href, d.defaultView===w, w.parent===window);\
         var cs=getComputedStyle(d.getElementById('a'));\
         console.log(cs.whiteSpace, cs.color, getComputedStyle(document.body).whiteSpace);\
         console.log(document.getElementById('g').contentDocument, document.getElementById('h').contentDocument);\
         console.log(document.getElementById('a'), document.getElementsByTagName('p').length);\
         var e=d.createElement('div'); d.body.appendChild(e); e.style.whiteSpace='pre-wrap';\
         console.log(getComputedStyle(e).whiteSpace);",
    );
    assert_eq!(
        *log.borrow(),
        [
            "log:true T hi http://qa18b1.test/dir/empty.html http://qa18b1.test/dir/empty.html true true",
            "log:pre rgb(16, 32, 48) normal",
            "log:null null",
            "log:null 0",
            "log:pre-wrap",
        ],
        "{:?}",
        log.borrow()
    );
}

// ---------- QY8: O(1) sibling access ----------

#[test]
fn qy8_walk_counts_and_stays_correct_after_mutations() {
    let log = run_in_html(
        "<div id=p></div>",
        "var p=document.getElementById('p');
         for(var i=0;i<50;i++){var s=document.createElement('span');s.id='s'+i;p.appendChild(s);}
         var w=function(){var o=[];for(var n=p.firstChild;n;n=n.nextSibling)o.push(n.id||'#');return o.join(',');};
         var b=function(){var o=[];for(var n=p.lastChild;n;n=n.previousSibling)o.push(n.id||'#');return o.join(',');};
         var s5=document.getElementById('s5');
         console.log(s5.nextSibling.id+' '+s5.previousSibling.id);
         p.insertBefore(document.createElement('i'),s5);
         console.log(s5.previousSibling.tagName+' '+s5.nextSibling.id);
         p.removeChild(document.getElementById('s6'));
         console.log(s5.nextSibling.id);
         s5.before(document.createTextNode('t'));
         console.log(s5.previousSibling.nodeType+' '+s5.previousElementSibling.tagName+' '+s5.nextElementSibling.id);
         s5.replaceWith(document.createElement('b'));
         console.log(s5.nextSibling+' '+s5.parentNode);
         var c=0;for(var n=p.firstChild;n;n=n.nextSibling)c++;
         console.log(c+' '+(w().split(',').length)+' '+(b().split(',').reverse().join(',')===w()));
         p.innerHTML='<a></a><b></b>';
         console.log(p.firstChild.tagName+p.lastChild.tagName+p.firstChild.nextSibling.tagName+p.lastElementChild.previousElementSibling.tagName);
         var x=document.createElement('div');console.log(x.nextSibling+' '+x.firstChild+' '+x.nextElementSibling);",
    );
    assert_eq!(
        log,
        vec![
            "log:s6 s4",
            "log:I s6",
            "log:s7",
            "log:3 I s7",
            "log:null null",
            "log:51 51 true",
            "log:ABBA",
            "log:null null null",
        ]
    );
}

#[test]
fn qy8_sibling_hint_is_o1_when_walking() {
    use markup5ever_rcdom::Node;
    let kids: Vec<_> = (0..2000).map(|_| Node::new(markup5ever_rcdom::NodeData::Comment { contents: "x".into() })).collect();
    let mut steps = 0;
    let mut hint = 0;
    for (i, k) in kids.iter().enumerate() {
        let got = crate::js_dom::child_index(&kids, k, hint, &mut steps);
        assert_eq!(got, Some(i));
        hint = i + 1; // the native stores the sibling's index when it returns it
    }
    // One probe per step when the hint is right (a scan would be ~2M).
    assert!(steps <= 2 * 2000, "steps = {steps}");
    // A stale hint still gives the right answer (falls back to a scan).
    assert_eq!(crate::js_dom::child_index(&kids, &kids[10], 1999, &mut steps), Some(10));
    assert_eq!(crate::js_dom::child_index(&kids, &kids[10], 99999, &mut steps), Some(10));
}

// ---------- QY9: URL and URLSearchParams ----------

fn qy9_run(code: &str) -> Vec<String> {
    run_in_html("<html><body></body></html>", code)
}

#[test]
fn qy9_url_parse_and_relative() {
    let log = qy9_run(
        "var u=new URL('https://u:p@Example.org:8080/a/b?x=1#frag');\
         console.log(u.href,u.protocol,u.host,u.hostname,u.port,u.pathname,u.search,u.hash,u.origin,u.username,u.password);\
         console.log(new URL('../a?b=1#c','https://x.org/p/q').href, new URL('/z','https://x.org/p/q').href, String(new URL('a','https://x.org/')), JSON.stringify({u:new URL('https://x.org')}));\
         console.log(URL.canParse('nope'), URL.canParse('a','https://x.org'), URL.parse('nope'));",
    );
    assert_eq!(
        log,
        [
            "log:https://u:p@example.org:8080/a/b?x=1#frag https: example.org:8080 example.org 8080 /a/b ?x=1 #frag https://example.org:8080 u p",
            "log:https://x.org/a?b=1#c https://x.org/z https://x.org/a {\"u\":\"https://x.org/\"}",
            "log:false true null",
        ],
        "{log:?}"
    );
}

#[test]
fn qy9_url_invalid_throws_type_error() {
    let log = qy9_run("try{new URL('not a url');console.log('no')}catch(e){console.log(e instanceof TypeError, e.name)}");
    assert_eq!(log, ["log:true TypeError"], "{log:?}");
}

#[test]
fn qy9_search_params_live_link_both_ways() {
    let log = qy9_run(
        "var u=new URL('https://x.org/p?a=1&a=2');\
         u.searchParams.set('z','1 2');\
         console.log(u.search, u.href);\
         u.search='?q=%C3%A9&w=a+b';\
         console.log(u.searchParams.get('q'), u.searchParams.get('w'), u.searchParams.size);\
         u.searchParams.delete('q'); u.searchParams.delete('w');\
         console.log(JSON.stringify(u.search), u.href);\
         u.pathname='/new path'; u.hash='h'; u.port='81';\
         console.log(u.href);",
    );
    assert_eq!(
        log,
        [
            "log:?a=1&a=2&z=1+2 https://x.org/p?a=1&a=2&z=1+2",
            "log:é a b 2",
            "log:\"\" https://x.org/p",
            "log:https://x.org:81/new%20path#h",
        ],
        "{log:?}"
    );
}

#[test]
fn qy9_search_params_encoding_and_iteration() {
    let log = qy9_run(
        "var p=new URLSearchParams('a=1&a=2&b=%20&c');\
         console.log(p.getAll('a').length, JSON.stringify(p.get('b')), p.has('c'), p.get('nope'), p.size);\
         var q=new URLSearchParams({k:'a b', u:'h\\u00e9llo&='});\
         console.log(q.toString());\
         var r=new URLSearchParams([['x','1'],['y','2']]); r.append('x','3'); r.sort();\
         var out=[]; for (var e of r) out.push(e.join(':'));\
         console.log(out.join(','), Array.from(r.keys()).join(''), Array.from(r.values()).join(''));\
         var n=[]; r.forEach(function(v,k){n.push(k+v)}); console.log(n.join(','));\
         console.log(new URLSearchParams('?a=1').toString());",
    );
    assert_eq!(
        log,
        [
            "log:2 \" \" true null 4",
            "log:k=a+b&u=h%C3%A9llo%26%3D",
            "log:x:1,x:3,y:2 xxy 132",
            "log:x1,x3,y2",
            "log:a=1",
        ],
        "{log:?}"
    );
}

// ---------- QY10: window.matchMedia ----------

#[test]
fn qy10_match_media_features_and_combinations() {
    let out = run_collecting(&[src(
        "var q = function (s) { return window.matchMedia(s).matches ? 1 : 0; };\
         console.log([q('(min-width: 1px)'), q('(max-width: 1px)'), q('(min-width: 1024px)'), q('(max-width: 1023px)'),\
           q('(min-width: 600px) and (max-width: 2000px)'), q('(min-width: 600px) and (max-width: 800px)'),\
           q('(prefers-color-scheme: light)'), q('(prefers-color-scheme: dark)'), q('print'), q('screen'),\
           q('not all'), q('not print'), q('(min-height: 700px)'), q('(orientation: landscape)'), q('(orientation: portrait)'),\
           q('(hover: hover)'), q('(pointer: coarse)'), q('(prefers-reduced-motion: reduce)'), q('(min-resolution: 2dppx)'),\
           q('(min-resolution: 1dppx)'), q('print, (min-width: 1px)'), q('(width >= 600px)'), q('(width < 600px)'),\
           q('screen and (min-width: 1px)'), q('(forced-colors: active)'), q('')].join(','));",
    )]);
    assert_eq!(out, vec!["log:1,0,1,0,1,0,1,0,0,1,0,1,1,1,0,1,0,0,0,1,1,1,0,1,0,1"]);
}

#[test]
fn qy10_match_media_invalid_is_not_all_and_media_is_reported() {
    let out = run_collecting(&[src(
        "var a = matchMedia('(((garbage'), b = matchMedia('(MIN-WIDTH: 10px)'), c = matchMedia('screen and');\
         console.log(a.matches, a.media, b.matches, b.media, c.matches, c.media, matchMedia('').media === '');",
    )]);
    assert_eq!(out, vec!["log:false not all true (min-width: 10px) false not all true"]);
}

#[test]
fn qy10_match_media_listeners_and_class() {
    let out = run_collecting(&[src(
        "var m = matchMedia('(min-width: 1px)'), n = 0, f = function () { n++; };\
         var k = 0, g2 = function () { k++; }; m.addListener(g2);\
         m.addListener(f); m.addEventListener('change', f); m.removeListener(f);\
         m.dispatchEvent(new Event('change')); if (k !== 1) n += 100; m.removeListener(g2);\
         m.dispatchEvent(new Event('change')); if (k !== 1) n += 1000;\
         m.removeEventListener('change', f);\
         m.dispatchEvent(new Event('change'));\
         m.onchange = function () { n += 10; }; m.dispatchEvent(new Event('change'));\
         console.log(m instanceof MediaQueryList, m instanceof EventTarget, typeof m.addListener, m.onchange !== undefined, n);",
    )]);
    assert_eq!(out, vec!["log:true true function true 10"]);
}

// ---------- QY11: reflected URL attributes, dataset, attributes, classList ----------

#[test]
fn qy11_url_reflection_is_absolute_and_honours_base() {
    set_page_url(Url::parse("https://page.test/dir/index.html?q=1").ok().as_ref());
    let log = run_in_html(
        "<html><head><base href=https://x.org/d/></head><body><script src=a.js id=s></script><img src=../p.png id=i>\
         <a href='sub/x?y#z' id=a></a><form action=go id=f></form><link rel=stylesheet href=/c.css id=l>\
         <iframe src=about:blank id=fr></iframe><a id=n></a></body></html>",
        "var g=function(i){return document.getElementById(i)};\
         var r=[g('s').src,g('i').src,g('i').currentSrc,g('a').href,g('f').action,g('l').href,g('fr').src,g('n').href,document.baseURI];\
         g('i').src='/z.png'; r.push(g('i').getAttribute('src'), g('i').src);\
         var a=g('a'); r.push(a.origin,a.protocol,a.host,a.hostname,a.port,a.pathname,a.search,a.hash);\
         a.pathname='/new'; r.push(a.href);\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:https://x.org/d/a.js|https://x.org/p.png|https://x.org/p.png|https://x.org/d/sub/x?y#z|https://x.org/d/go|https://x.org/c.css|about:blank||https://x.org/d/|/z.png|https://x.org/z.png|https://x.org|https:|x.org|x.org||/d/sub/x|?y|#z|https://x.org/new?y#z"],
        "{log:?}"
    );
    set_page_url(None);
}

#[test]
fn qy11_url_reflection_without_base_uses_page_url() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_in_html(
        "<html><body><script src=a.js id=s></script></body></html>",
        "console.log(document.getElementById('s').src, document.baseURI)",
    );
    assert_eq!(log, ["log:https://page.test/dir/a.js https://page.test/dir/index.html"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qy11_dataset_roundtrip() {
    let log = run_in_html(
        "<html><body><div id=a data-foo-bar=1 data-x=2></div></body></html>",
        "var e=document.getElementById('a'), d=e.dataset, r=[];\
         r.push(d.fooBar, d.x, d.nope, 'fooBar' in d, 'zip' in d);\
         d.newThing=5; r.push(e.getAttribute('data-new-thing'));\
         e.setAttribute('data-late','L'); r.push(d.late);\
         delete d.x; r.push(e.hasAttribute('data-x'), 'x' in d);\
         r.push(Object.keys(d).join(','));\
         var k=[]; for (var p in d) k.push(p); r.push(k.join(','));\
         console.log(r.join('|'));",
    );
    assert_eq!(log, ["log:1|2||true|false|5|L|false|false|fooBar,newThing,late|fooBar,newThing,late"], "{log:?}");
}

#[test]
fn qy11_attributes_and_attribute_names() {
    let log = run_in_html(
        "<html><body><p id=a class='c d' title=t></p><p id=e></p></body></html>",
        "var e=document.getElementById('a'), m=e.attributes, r=[];\
         r.push(m.length, m[0].name, m.item(1).value, m.item(9), m.getNamedItem('title').value, m.title.name, m.id.specified, m.id.ownerElement===e, m[2].nodeName);\
         r.push(e.getAttributeNames().join(','), e.hasAttributes(), document.createElement('i').hasAttributes());\
         var n=[]; for (var x of m) n.push(x.name+'='+x.value); r.push(n.join(';'));\
         m.title.value='T2'; r.push(e.getAttribute('title'), e.getAttributeNode('id').value, e.getAttributeNode('zz'));\
         e.setAttribute('lang','en'); r.push(m.length, e.getAttributeNames().pop());\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:3|id|c d||t|title|true|true|title|id,class,title|true|false|id=a;class=c d;title=t|T2|a||4|lang"],
        "{log:?}"
    );
}

#[test]
fn qy11_classlist_iterable_and_consistent() {
    let log = run_in_html(
        "<html><body><div id=a class='x y'></div></body></html>",
        "var e=document.getElementById('a'), l=e.classList, r=[];\
         r.push([...l].join(','), l.length, l.item(1), l[0], l.value, l.supports('x'), e.classList===l);\
         var f=[]; l.forEach(function(v,i){f.push(i+v)}); r.push(f.join(','));\
         r.push([...l.keys()].join(','), [...l.values()].join(','), [...l.entries()].join(';'));\
         for (var c of l) r.push('c'+c);\
         l.add('z'); r.push(e.className); e.className='q'; r.push(l.value, l.length);\
         l.value='m n'; r.push(e.getAttribute('class'), l.toggle('n'), e.className);\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:x,y|2|y|x|x y|true|true|0x,1y|0,1|x,y|0,x;1,y|cx|cy|x y z|q|1|m n|false|m"],
        "{log:?}"
    );
}

#[test]
fn qy11_style_string_reflection() {
    let log = run_in_html(
        "<html><body><div id=a></div></body></html>",
        "var e=document.getElementById('a'); e.style.color='red'; e.style.marginTop='1px';\
         console.log(e.getAttribute('style'), '|', e.style.cssText, '|', e.style.color);",
    );
    assert_eq!(log, ["log:color: red; margin-top: 1px; | color: red; margin-top: 1px; | red"], "{log:?}");
}

#[test]
fn qy23_style_assignment_puts_forward_to_css_text() {
    let log = run_in_html(
        "<html><body><div id=a></div></body></html>",
        "'use strict'; var d=document.getElementById('a'); d.style='color: red';\
         console.log(d.getAttribute('style'), '|', d.style.color);\
         d.style=null; console.log(d.getAttribute('style'), '|', d.style.cssText === '');",
    );
    assert_eq!(log, ["log:color: red | red", "log:null | true"], "{log:?}");
}

#[test]
fn qy24_class_list_reads_the_class_attribute_not_class_name() {
    let log = run_in_html(
        "<html><body><x-q id=a class='p q'></x-q></body></html>",
        "'use strict'; class XQ extends HTMLElement { get className() { return undefined; } set className(v) {} }\
         customElements.define('x-q', XQ); var d=document.getElementById('a');\
         d.classList.add('r'); console.log(d.classList.contains('p'), d.classList.toggle('p'), d.getAttribute('class'), d.classList.length, String(d.classList));",
    );
    assert_eq!(log, ["log:true false q r 2 q r"], "{log:?}");
}

#[test]
fn qy12_window_interface_has_its_own_prototype() {
    let out = run_collecting(&[src(
        "console.log(window instanceof Window, window instanceof EventTarget, Window.prototype !== Object.prototype,\
         Object.getPrototypeOf(Window.prototype) === EventTarget.prototype, ({}) instanceof Window,\
         window.top === window, window.parent === window, window.frames === window, window.opener === null);\
         try { new Window(); } catch (e) { console.log(e.name); }",
    )]);
    assert_eq!(out, vec!["log:true true true true false true true true true", "log:TypeError"]);
}

#[test]
fn qy12_html_element_interfaces_and_instanceof() {
    let out = run_in_html(
        "<html><body><div id=d></div><span></span><a href=x></a><ul><li></li></ul><h3></h3><foo-bar></foo-bar><blink></blink><template></template><video></video></body></html>",
        "var t = function (s) { return document.querySelector(s); };\
         console.log(t('div') instanceof HTMLDivElement, t('span') instanceof HTMLSpanElement, t('a') instanceof HTMLAnchorElement,\
           t('li') instanceof HTMLLIElement, t('ul') instanceof HTMLUListElement, t('h3') instanceof HTMLHeadingElement,\
           document.body instanceof HTMLBodyElement, document.documentElement instanceof HTMLHtmlElement,\
           document.createElement('img') instanceof HTMLImageElement, t('template') instanceof HTMLTemplateElement,\
           t('div') instanceof HTMLElement, t('div') instanceof HTMLSpanElement);\
         console.log(t('blink') instanceof HTMLUnknownElement, t('foo-bar') instanceof HTMLUnknownElement, t('foo-bar') instanceof HTMLElement,\
           t('div') instanceof HTMLUnknownElement, t('div').constructor === HTMLDivElement);\
         var c = [HTMLDivElement, HTMLScriptElement, HTMLStyleElement, HTMLLinkElement, HTMLMetaElement, HTMLTitleElement, HTMLHeadElement, HTMLSlotElement, HTMLParagraphElement, HTMLOListElement, HTMLButtonElement, HTMLInputElement, HTMLFormElement, HTMLLabelElement];\
         var bad = 0; c.forEach(function (C) { try { C(); bad++; } catch (e) { if (!(e instanceof TypeError)) bad++; } try { new C(); bad++; } catch (e) {} });\
         console.log(bad, Object.getPrototypeOf(HTMLDivElement.prototype) === HTMLElement.prototype);",
    );
    assert_eq!(out, vec!["log:true true true true true true true true true true true false", "log:true false true false true", "log:0 true"], "{out:?}");
}

#[test]
fn qy12_svg_interfaces_by_namespace() {
    let out = run_in_html(
        "<html><body><svg id=s><path d='M0 0'></path><g></g></svg></body></html>",
        "var svg = 'http://www.w3.org/2000/svg';\
         var p = document.createElementNS(svg, 'path'), r = document.createElementNS(svg, 'rect'), x = document.createElementNS(svg, 'foo');\
         var s = document.getElementById('s');\
         console.log(s instanceof SVGSVGElement, s instanceof SVGGraphicsElement, s instanceof SVGElement, s instanceof Element, s instanceof HTMLElement,\
           s.firstChild instanceof SVGPathElement, p instanceof SVGGeometryElement, r instanceof SVGRectElement, x instanceof SVGElement, x instanceof SVGGraphicsElement,\
           document.createElement('div') instanceof SVGElement, typeof SVGUseElement, typeof SVGGElement);",
    );
    assert_eq!(out, vec!["log:true true true true false true true true true false false function function"], "{out:?}");
}

#[test]
fn qy12_cdata_pi_range_and_fragment_children() {
    let out = run_in_html(
        "<html><body></body></html>",
        "var f = document.createDocumentFragment(); var a = document.createElement('p'); f.appendChild(document.createTextNode('t')); f.appendChild(a); f.appendChild(document.createElement('i'));\
         console.log(f.children.length, f.childElementCount, f.firstElementChild === a, f.lastElementChild.tagName, Array.from(f.children).length);\
         f.prepend(document.createElement('b')); f.append(document.createElement('u')); console.log(f.children.length, f.firstElementChild.tagName);\
         console.log(typeof CDATASection, typeof ProcessingInstruction, CDATASection.prototype instanceof Text, ProcessingInstruction.prototype instanceof CharacterData, typeof Range);\
         try { new Range(); } catch (e) { console.log(e.name); }\
         try { Selection(); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(out, vec!["log:2 2 true I 2", "log:4 B", "log:function function true true function", "log:TypeError"], "{out:?}");
}

#[test]
fn qy12_event_subclasses_construct_with_init_dicts() {
    let out = run_collecting(&[src(
        "var p = new PointerEvent('pointerdown', { pointerId: 3, pointerType: 'mouse', clientX: 5, bubbles: true });\
         var w = new WheelEvent('wheel', { deltaY: 100 }), h = new HashChangeEvent('hashchange', { oldURL: 'a', newURL: 'b' });\
         var m = new MessageEvent('message', { data: { x: 1 }, origin: 'o' }), pr = new ProgressEvent('progress', { loaded: 2, total: 4, lengthComputable: true });\
         var t = new TouchEvent('touchstart'), s = new StorageEvent('storage', { key: 'k' });\
         console.log(p instanceof MouseEvent, p instanceof Event, p.pointerId, p.pointerType, p.clientX, p.bubbles, w.deltaY, w.deltaX, h.newURL, m.data.x, m.origin, pr.loaded, pr.total, pr.lengthComputable, t.touches.length, s.key, s.oldValue);\
         var n = 0; ['TransitionEvent', 'AnimationEvent', 'PromiseRejectionEvent', 'PopStateEvent', 'ClipboardEvent', 'DragEvent', 'CompositionEvent'].forEach(function (k) { if (new window[k]('x') instanceof Event) n++; });\
         console.log(n, new PopStateEvent('popstate', { state: 7 }).state, new TransitionEvent('transitionend', { propertyName: 'width' }).propertyName);",
    )]);
    assert_eq!(out[0], "log:true true 3 mouse 5 true 100 0 b 1 o 2 4 true 0 k null", "{out:?}");
    assert_eq!(out[1], "log:7 7 width", "{out:?}");
}

#[test]
fn qy12_media_interfaces_are_inert_placeholders() {
    let out = run_in_html(
        "<html><body><video id=v></video><audio></audio></body></html>",
        "var v = document.getElementById('v');\
         console.log(v instanceof HTMLVideoElement, v instanceof HTMLMediaElement, document.querySelector('audio') instanceof HTMLAudioElement,\
           typeof HTMLSourceElement, typeof HTMLTrackElement, v.canPlayType('video/mp4') === '');\
         v.pause(); v.load();\
         var pr = v.play(); console.log(pr instanceof Promise);\
         pr.catch(function (e) { console.log('rejected', e.name); });",
    );
    assert!(out.iter().any(|l| l == "log:true true true function function true"), "{out:?}");
    assert!(out.iter().filter(|l| l.starts_with("error:") && l.contains("not implemented")).count() >= 3, "{out:?}");
    assert!(out.iter().any(|l| l == "log:rejected NotSupportedError"), "{out:?}");
}

// ---------- QY13: history, AbortController, MessageChannel, storage, ... ----------

#[test]
fn qy13_history_push_replace() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_with_timers(
        "var n0=history.length; history.pushState({a:1},'','/p?q=1#h');\
         console.log(n0, history.length, JSON.stringify(history.state), location.pathname, location.search, location.hash, document.URL.indexOf('/p?q=1#h')>0);\
         history.replaceState({b:2},'','/r'); console.log(history.length, history.state.b, location.pathname, history.scrollRestoration);\
         window.addEventListener('popstate', function(e){ console.log('pop', e.state, location.pathname); });\
         history.back();",
    );
    assert_eq!(log, ["log:1 2 {\"a\":1} /p ?q=1 #h true", "log:2 2 /r auto", "log:pop null /dir/index.html"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qy13_abort_controller() {
    let log = run_with_timers(
        "var c=new AbortController(), n=0; c.signal.addEventListener('abort',()=>n++); c.signal.onabort=()=>n+=10;\
         console.log(c.signal.aborted); c.abort('why'); c.abort('again');\
         console.log(c.signal.aborted, c.signal.reason, n);\
         try { c.signal.throwIfAborted(); } catch (e) { console.log('threw', e); }\
         console.log(AbortSignal.abort().reason.name);\
         var t=AbortSignal.timeout(5); t.onabort=()=>console.log('timeout', t.reason.name);\
         var a=new AbortController(); var any=AbortSignal.any([a.signal]); a.abort(); console.log(any.aborted);",
    );
    assert_eq!(log, ["log:false", "log:true why 11", "log:threw why", "log:AbortError", "log:true", "log:timeout TimeoutError"], "{log:?}");
}

#[test]
fn qy13_message_channel_and_post_message() {
    let log = run_with_timers(
        "var ch=new MessageChannel(); ch.port1.onmessage=function(e){console.log('port', e.data.x)};\
         ch.port2.postMessage({x:5}); console.log('sync');\
         window.addEventListener('message', function(e){ console.log('win', e.data, e.source===window); });\
         window.postMessage('hi','*'); window.postMessage('no','http://other.example');\
         setTimeout(()=>console.log('t10'),10);",
    );
    assert_eq!(log, ["log:sync", "log:port 5", "log:win hi true", "log:t10"], "{log:?}");
}

#[test]
fn qy13_storage_and_misc() {
    let log = run_with_timers(
        "localStorage.setItem('a',1); localStorage.b='two'; sessionStorage.setItem('c','3');\
         console.log(localStorage.getItem('a'), localStorage.a, localStorage.length, localStorage.key(1), localStorage.getItem('zz'), sessionStorage.length);\
         delete localStorage.a; localStorage.removeItem('b'); console.log(localStorage.length); localStorage.clear();\
         var o=structuredClone({d:new Date(5),m:new Map([[1,2]]),s:new Set([1]),arr:[1,{z:2}]}); console.log(o.d.getTime(), o.m.get(1), o.s.has(1), o.arr[1].z);\
         try { structuredClone(function(){}) } catch(e) { console.log(e.name) }\
         queueMicrotask(()=>console.log('micro')); console.log('sync');\
         requestIdleCallback(function(d){ console.log('idle', d.didTimeout, typeof d.timeRemaining()); });\
         var id=requestIdleCallback(function(){console.log('never')}); cancelIdleCallback(id);",
    );
    assert_eq!(log, ["log:1 1 2 b null 1","log:0", "log:5 2 true 2", "log:DataCloneError", "log:sync", "log:micro", "log:idle false number"], "{log:?}");
}

// ---------- QY14: JS diagnostics ----------

#[test]
fn qy14_rejected_promise_job_and_unhandled_rejection_log_message() {
    // A throw inside a `.then` job rejects the derived promise: one line.
    let log = run_in_html("", "Promise.resolve().then(function () { throw new TypeError('job-boom'); });");
    let hits: Vec<_> = log.iter().filter(|l| l.contains("job-boom")).collect();
    assert_eq!(hits.len(), 1, "{log:?}");
    assert!(hits[0].starts_with("error:Uncaught (in promise) TypeError: job-boom"), "{log:?}");
    // Async function throwing, and a plain reject of a non-Error value.
    let log = run_in_html("", "(async function () { throw new RangeError('async-boom'); })(); Promise.reject('plain-reason');");
    assert!(log.iter().any(|l| l.starts_with("error:Uncaught (in promise) RangeError: async-boom")), "{log:?}");
    assert!(log.iter().any(|l| l == "error:Uncaught (in promise) plain-reason"), "{log:?}");
}

#[test]
fn qy14_handled_rejections_stay_silent_and_events_fire() {
    let log = run_in_html("", "Promise.reject(new Error('quiet')).catch(function () {}); (async function () { try { await Promise.reject(1); } catch (e) {} })();");
    assert!(!log.iter().any(|l| l.starts_with("error:")), "{log:?}");
    let log = run_in_html(
        "",
        "var late = Promise.reject(new Error('later'));\
         addEventListener('unhandledrejection', function (e) { console.log('unhandled', e.reason.message, e instanceof PromiseRejectionEvent); });\
         addEventListener('rejectionhandled', function (e) { console.log('handled', e.reason.message); });\
         setTimeout(function () { late.catch(function () {}); }, 0);\
         Promise.reject(new Error('stopped'));",
    );
    assert!(log.iter().any(|l| l == "log:unhandled stopped true"), "{log:?}");
    assert_eq!(log.iter().filter(|l| l.contains("Uncaught (in promise)")).count(), 2, "{log:?}");
}

#[test]
fn qy14_prevent_default_suppresses_unhandled_rejection_log() {
    let log = run_in_html("", "addEventListener('unhandledrejection', function (e) { e.preventDefault(); }); Promise.reject(new Error('muted'));");
    assert!(!log.iter().any(|l| l.contains("muted")), "{log:?}");
}

#[test]
fn qy14_js_report_run_timers_finds_timer_and_channel_errors() {
    let html = "<html><body><script>\
        setTimeout(function () { throw new Error('late-timer'); }, 1000);\
        var ch = new MessageChannel(); ch.port1.onmessage = function () { throw new Error('chan-boom'); }; ch.port2.postMessage(1);\
        requestIdleCallback(function () { throw new Error('idle-boom'); });\
        setTimeout(function () { Promise.reject(new Error('timer-rejection')); }, 50);\
        console.error('page-console-error');\
        </script></body></html>";
    // Default: unchanged format, nothing from the timer phase.
    let plain = js::js_report_for_html(html, None, "none.html");
    assert!(!plain.contains("late-timer") && !plain.contains("timers ("), "{plain}");
    assert!(plain.contains("errors total: 0\n") && plain.contains("console.error calls: 1\n"), "{plain}");
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(2000));
    for needle in ["late-timer", "chan-boom", "idle-boom", "timer-rejection"] {
        assert!(report.contains(needle), "{needle}: {report}");
    }
    assert!(report.contains("js error (timers): "), "{report}");
    assert!(report.contains("errors in timer phase: 4\n"), "{report}");
    assert!(report.contains("errors total: 0\n"), "{report}");
    let short = js::js_report_for_html_timers(html, None, "none.html", Some(500));
    assert!(!short.contains("late-timer") && short.contains("chan-boom"), "{short}");
}

#[test]
fn qy31_js_report_keeps_pumping_after_a_tripped_callback() {
    let html = "<html><body><script>\
        setTimeout(function () { for (;;) {} }, 10);\
        setTimeout(function () { console.error('after-trip-1000'); }, 1000);\
        setTimeout(function () { throw new Error('after-trip-3000'); }, 3000);\
        </script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(5000));
    assert!(report.contains("CPU-time callback budget"), "{report}");
    assert!(report.contains("after-trip-1000") && report.contains("after-trip-3000"), "{report}");
    assert!(report.contains("--- timers (simulated 3000 ms, 3 callbacks) ---\n"), "{report}");
}

#[test]
fn qy14_js_report_run_timers_is_bounded() {
    let html = "<html><body><script>setInterval(function () { throw new Error('tick'); }, 1);</script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(100000));
    assert!(report.contains("errors in timer phase: 5000\n"), "{}", &report[report.len().saturating_sub(300)..]);
}

#[test]
fn qy14_console_methods_exist() {
    let log = run_in_html(
        "",
        "console.assert(true, 'no'); console.assert(false, 'yes'); console.group('g'); console.table([1]); console.trace('t'); console.groupEnd(); console.dir({a:1}); console.count(); console.time('x'); console.timeEnd('x'); console.error('e'); console.info('i'); console.debug('d'); console.warn('w');",
    );
    assert!(log.iter().any(|l| l == "console.error:Assertion failed: yes"), "{log:?}");
    assert!(log.iter().any(|l| l == "console.error:e"), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("log:Trace: t")), "{log:?}");
    assert!(log.iter().any(|l| l == "log:default: 1") && log.iter().any(|l| l == "warn:w") && log.iter().any(|l| l == "info:i"), "{log:?}");
    assert!(!log.iter().any(|l| l.contains(":no")), "{log:?}");
}

// ---------- QY16: minimal en-US Intl ----------

#[test]
fn qy16_number_format() {
    let log = run_with_timers(
        "var nf=new Intl.NumberFormat('en-US');\
         console.log(nf.format(1234.5), nf.format(1234567.891), nf.format(-0.5), new Intl.NumberFormat('en',{maximumFractionDigits:2}).format(1.005), nf.format(NaN));\
         console.log(new Intl.NumberFormat('en',{minimumFractionDigits:2}).format(5), new Intl.NumberFormat('en',{maximumFractionDigits:0}).format(2.5), new Intl.NumberFormat('en',{useGrouping:false}).format(12345));\
         console.log(new Intl.NumberFormat('en',{style:'percent'}).format(0.256), new Intl.NumberFormat('en',{style:'percent',maximumFractionDigits:1}).format(0.2567));\
         var c=function(cur,v,o){return new Intl.NumberFormat('en-US',Object.assign({style:'currency',currency:cur},o)).format(v)};\
         console.log(c('USD',1234.5), c('EUR',-3), c('GBP',0.5), c('JPY',1234.5), c('USD',-5,{currencySign:'accounting'}));\
         var k=function(v,o){return new Intl.NumberFormat('en',Object.assign({notation:'compact'},o)).format(v)};\
         console.log(k(999), k(1234), k(12345), k(1500000), k(2.5e9), k(999999), k(1234,{compactDisplay:'long'}));\
         console.log(new Intl.NumberFormat('en',{maximumSignificantDigits:3}).format(123456), new Intl.NumberFormat('en',{minimumIntegerDigits:3}).format(5));\
         console.log(JSON.stringify(nf.formatToParts(-1234.5).map(function(p){return p.type+'='+p.value})));\
         var r=new Intl.NumberFormat('de-DE').resolvedOptions(); console.log(r.locale, r.minimumFractionDigits, r.maximumFractionDigits, r.style);\
         var f=nf.format; console.log(f(7), typeof Intl, Object.prototype.toString.call(Intl));",
    );
    assert_eq!(
        log,
        [
            "log:1,234.5 1,234,567.891 -0.5 1.01 NaN",
            "log:5.00 3 12345",
            "log:26% 25.7%",
            "log:$1,234.50 -€3.00 £0.50 ¥1,235 ($5.00)",
            "log:999 1.2K 12K 1.5M 2.5B 1M 1.2 thousand",
            "log:123,000 005",
            "log:[\"minusSign=-\",\"integer=1\",\"group=,\",\"integer=234\",\"decimal=.\",\"fraction=5\"]",
            "log:en-US 0 3 decimal",
            "log:7 object [object Intl]",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_date_time_format() {
    let log = run_with_timers(
        "var t=Date.UTC(2024,0,2,15,4,5,67);\
         var f=function(o,l){return new Intl.DateTimeFormat(l||'en-US',Object.assign({timeZone:'UTC'},o)).format(t)};\
         console.log(f({}), f({dateStyle:'short'}), f({dateStyle:'medium'}), f({dateStyle:'long'}), f({dateStyle:'full'}));\
         console.log(f({timeStyle:'short'}), f({timeStyle:'medium'}), f({timeStyle:'long'}), f({timeStyle:'full'}));\
         console.log(f({dateStyle:'medium',timeStyle:'short'}), '|', f({dateStyle:'long',timeStyle:'short'}));\
         console.log(f({year:'numeric',month:'long',day:'numeric'}), f({month:'short',day:'numeric'}), f({year:'numeric',month:'2-digit',day:'2-digit'}), f({weekday:'long'}), f({month:'long',year:'numeric'}));\
         console.log(f({hour:'numeric',minute:'2-digit',hour12:false}), f({hour:'numeric',hour12:true}), f({hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false}));\
         console.log(JSON.stringify(new Intl.DateTimeFormat('en-US',{timeZone:'UTC',month:'short',day:'numeric'}).formatToParts(t)));\
         var r=new Intl.DateTimeFormat('fr-FR').resolvedOptions(); console.log(r.locale, r.timeZone, r.calendar);\
         var d=new Date(t); console.log(d.toLocaleDateString('en-US',{timeZone:'UTC'}), '|', d.toLocaleTimeString('en-US',{timeZone:'UTC'}), '|', d.toLocaleString('en-US',{timeZone:'UTC'}), '|', d.toLocaleDateString(undefined,{month:'long',day:'numeric'}));\
         console.log(new Date(NaN).toLocaleString(), (function(){try{new Intl.DateTimeFormat().format(NaN)}catch(e){return e.name}})());",
    );
    assert_eq!(
        log,
        [
            "log:1/2/2024 1/2/24 Jan 2, 2024 January 2, 2024 Tuesday, January 2, 2024",
            "log:3:04 PM 3:04:05 PM 3:04:05 PM UTC 3:04:05 PM Coordinated Universal Time",
            "log:Jan 2, 2024, 3:04 PM | January 2, 2024 at 3:04 PM",
            "log:January 2, 2024 Jan 2 01/02/2024 Tuesday January 2024",
            "log:15:04 3 PM 15:04:05",
            "log:[{\"type\":\"month\",\"value\":\"Jan\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"day\",\"value\":\"2\"}]",
            "log:en-US UTC gregory",
            "log:1/2/2024 | 3:04:05 PM | 1/2/2024, 3:04:05 PM | January 2",
            "log:Invalid Date RangeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_plural_relative_list_collator() {
    let log = run_with_timers(
        "var pr=new Intl.PluralRules('en'), po=new Intl.PluralRules('en-US',{type:'ordinal'});\
         console.log(pr.select(0), pr.select(1), pr.select(2), pr.select(1.5), [1,2,3,4,11,12,13,21,22,23,101].map(function(n){return po.select(n)}).join());\
         var rt=new Intl.RelativeTimeFormat('en'), ra=new Intl.RelativeTimeFormat('en',{numeric:'auto'});\
         console.log(rt.format(3,'day'), rt.format(-3,'days'), rt.format(1,'hour'), rt.format(-1,'month'), rt.format(2,'year'));\
         console.log(ra.format(-1,'day'), ra.format(1,'day'), ra.format(0,'day'), ra.format(1,'year'), ra.format(-2,'day'), ra.format(0,'second'));\
         var lf=function(a,o){return new Intl.ListFormat('en',o).format(a)};\
         console.log(lf(['a','b','c']), '|', lf(['a','b']), '|', lf(['a']), '|', lf(['a','b','c'],{type:'disjunction'}), '|', lf(['a','b','c'],{style:'short'}), '|', lf(['a','b','c'],{type:'unit',style:'narrow'}));\
         var co=new Intl.Collator();\
         console.log(['b','a','C','B','c','A'].sort(co.compare).join(''), co.compare('a','b'), co.compare('b','a'), co.compare('a','a'), 'a'.localeCompare('B'), 'résumé'.localeCompare('resume'));\
         console.log(['a10','a2','a1'].sort(new Intl.Collator(undefined,{numeric:true}).compare).join(), ['a10','a2','a1'].sort(new Intl.Collator().compare).join(), new Intl.Collator('en',{sensitivity:'base'}).compare('a','Á'));",
    );
    assert_eq!(
        log,
        [
            "log:other one other other one,two,few,other,other,other,other,one,two,few,one",
            "log:in 3 days 3 days ago in 1 hour 1 month ago in 2 years",
            "log:yesterday tomorrow today next year 2 days ago now",
            "log:a, b, and c | a and b | a | a, b, or c | a, b, & c | a b c",
            "log:aAbBcC -1 1 0 -1 1",
            "log:a1,a2,a10 a1,a10,a2 0",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_locale_fallback_and_misc_apis() {
    let log = run_with_timers(
        "var mk=[Intl.NumberFormat,Intl.DateTimeFormat,Intl.PluralRules,Intl.RelativeTimeFormat,Intl.ListFormat,Intl.Collator,Intl.Segmenter];\
         console.log(mk.map(function(F){return new F('de-DE-u-nu-latn').resolvedOptions().locale}).join());\
         console.log(mk.map(function(F){return new F(['xx','ja'],{bogusOption:1}).resolvedOptions().locale}).join());\
         try { new Intl.NumberFormat('en_US') } catch (e) { console.log(e.name) }\
         try { new Intl.NumberFormat('') } catch (e) { console.log(e.name) }\
         console.log(Intl.getCanonicalLocales('EN-us').join(), JSON.stringify(Intl.supportedValuesOf('currency')), Intl.NumberFormat.supportedLocalesOf(['en-GB','de']).join());\
         var l=new Intl.Locale('en-Latn-US'); console.log(l.language, l.script, l.region, l.baseName, String(l), new Intl.Locale('fr').maximize().toString(), new Intl.Locale('en-Latn-US').minimize().toString(), new Intl.Locale('en-US-u-hc-h23').hourCycle);\
         console.log(new Intl.DisplayNames('en',{type:'region'}).of('US'), Array.from(new Intl.Segmenter('en').segment('a\\u0301b')).length, Array.from(new Intl.Segmenter('en',{granularity:'word'}).segment('Hi there!')).filter(function(s){return s.isWordLike}).map(function(s){return s.segment}).join());\
         console.log((1234.5).toLocaleString(), (1234.5).toLocaleString('en-US',{style:'currency',currency:'USD'}), (0.5).toLocaleString(undefined,{style:'percent'}), [1234,5].toLocaleString(), (12345678901234567890n).toLocaleString());",
    );
    assert_eq!(
        log,
        [
            "log:en-US,en-US,en-US,en-US,en-US,en-US,en-US",
            "log:en-US,en-US,en-US,en-US,en-US,en-US,en-US",
            "log:RangeError",
            "log:RangeError",
            "log:en-US [] en-GB",
            "log:en Latn US en-Latn-US en-Latn-US fr-Latn-FR en h23",
            "log:US 2 Hi,there",
            "log:1,234.5 $1,234.50 50% 1,234,5 12,345,678,901,234,567,890",
        ],
        "{log:?}"
    );
}

// ---- QY15: ES modules

/// Runs `html` as a page whose module fetches are served from `files`
/// (url -> source); returns the log lines.
fn run_module_page(html: &str, files: &[(&str, &str)]) -> Vec<String> {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_js_enabled(true);
    let dom = parse_html(html);
    let mut res = ExternalResources::default();
    find_external_resources(&dom, &mut res);
    let base = Url::parse("https://example.test/dir/page.html").unwrap();
    let files: Vec<(String, String)> = files.iter().map(|(u, s)| (u.to_string(), s.to_string())).collect();
    let fetch = move |u: &Url| files.iter().find(|(k, _)| k == u.as_str()).map(|(_, s)| s.clone());
    let scripts = fetch_scripts_with(&base, res.scripts, &fetch);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_in(&dom, &scripts, Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

#[test]
fn qy15_inline_module_imports_prefetched_module_default_and_named() {
    let log = run_module_page(
        "<script type=module>import d, { n } from './b.js'; console.log(d + n);</script>",
        &[("https://example.test/dir/b.js", "export default 'D'; export const n = 'N';")],
    );
    assert_eq!(log, ["log:DN"], "{log:?}");
}

#[test]
fn qy15_external_module_and_transitive_imports_and_import_meta_url() {
    let log = run_module_page(
        "<script type=module src=\"/m/a.js\"></script>",
        &[
            ("https://example.test/m/a.js", "import { b } from './b.js'; console.log(b, import.meta.url);"),
            ("https://example.test/m/b.js", "export { c as b } from '../c.js';"),
            ("https://example.test/c.js", "export const c = 'C';"),
        ],
    );
    assert_eq!(log, ["log:C https://example.test/m/a.js"], "{log:?}");
}

#[test]
fn qy15_inline_import_meta_url_is_document_url() {
    let log = run_module_page("<script type=module>console.log(import.meta.url);</script>", &[]);
    assert_eq!(log, ["log:https://example.test/dir/page.html"], "{log:?}");
}

#[test]
fn qy15_nomodule_scripts_are_skipped_and_classic_runs_first() {
    let log = run_module_page(
        "<script type=module>console.log('module');</script>\
         <script nomodule>console.log('nomodule');</script>\
         <script>console.log('classic');</script>\
         <script>document.addEventListener('DOMContentLoaded', function(){ console.log('dcl'); });</script>",
        &[],
    );
    assert_eq!(log, ["log:classic", "log:module", "log:dcl"], "{log:?}");
}

#[test]
fn qy15_module_scripts_run_in_document_order() {
    let log = run_module_page(
        "<script type=module src=a.js></script><script type=module>console.log('inline');</script><script type=module src=b.js></script>",
        &[("https://example.test/dir/a.js", "console.log('a');"), ("https://example.test/dir/b.js", "console.log('b');")],
    );
    assert_eq!(log, ["log:a", "log:inline", "log:b"], "{log:?}");
}

#[test]
fn qy15_importmap_resolves_bare_specifiers_and_prefixes() {
    let log = run_module_page(
        "<script type=importmap>{\"imports\":{\"lib\":\"/vendor/lib.js\",\"u/\":\"/utils/\"}}</script>\
         <script type=module>import { l } from 'lib'; import { x } from 'u/x.js'; console.log(l + x);</script>",
        &[("https://example.test/vendor/lib.js", "export const l = 'L';"), ("https://example.test/utils/x.js", "export const x = 'X';")],
    );
    assert_eq!(log, ["log:LX"], "{log:?}");
}

#[test]
fn qy15_bare_specifier_without_importmap_is_an_error() {
    let log = run_module_page("<script type=module>import 'nope';</script>", &[]);
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("error:") && log[0].contains("nope"), "{log:?}");
}

#[test]
fn qy15_failed_fetch_logs_once_and_other_scripts_still_run() {
    let log = run_module_page(
        "<script type=module src=gone.js></script><script type=module>import './gone2.js';</script><script>console.log('classic');</script><script type=module>console.log('ok');</script>",
        &[],
    );
    assert_eq!(log.iter().filter(|l| l.starts_with("error:") && l.contains("gone.js")).count(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|l| l.starts_with("error:") && l.contains("gone2.js")).count(), 1, "{log:?}");
    assert!(log.contains(&"log:classic".to_string()) && log.contains(&"log:ok".to_string()), "{log:?}");
}

#[test]
fn qy15_module_is_strict_scoped_and_this_is_undefined() {
    let log = run_module_page(
        "<script type=module>var leaked = 1; console.log(this === undefined, typeof globalThis.leaked, (function(){ return this; })() === undefined);</script>\
         <script type=module>try { undeclared = 1; } catch (e) { console.log(e.name); }</script>",
        &[],
    );
    assert_eq!(log, ["log:true undefined true", "log:ReferenceError"], "{log:?}");
}

#[test]
fn qy15_dynamic_import_of_prefetched_and_data_modules() {
    let log = run_module_page(
        "<script>import('./d.js').then(function(m){ console.log('classic ' + m.v); });</script>\
         <script type=module>const m = await import('data:text/javascript,export default 7'); console.log('data ' + m.default);</script>",
        &[("https://example.test/dir/d.js", "export const v = 'V';")],
    );
    assert!(log.contains(&"log:data 7".to_string()), "{log:?}");
    assert!(log.iter().any(|l| l == "log:classic V"), "{log:?}");
}

#[test]
fn qy15_dynamic_import_of_unfetched_url_rejects_with_clear_error() {
    let log = run_module_page(
        "<script type=module>try { await import('https://other.test/' + 'x.js'); } catch (e) { console.log('rejected ' + /not pre-fetched/.test(String(e.message))); }</script>",
        &[],
    );
    assert_eq!(log, ["log:rejected true"], "{log:?}");
}

#[test]
fn qy15_module_syntax_error_and_throw_are_logged_with_url() {
    let log = run_module_page(
        "<script type=module src=bad.js></script><script type=module src=thrower.js></script>",
        &[("https://example.test/dir/bad.js", "export const = ;"), ("https://example.test/dir/thrower.js", "throw new Error('boom');")],
    );
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].starts_with("error:https://example.test/dir/bad.js: "), "{log:?}");
    assert!(log[1].starts_with("error:https://example.test/dir/thrower.js: ") && log[1].contains("boom"), "{log:?}");
}

#[test]
fn qy15_module_runaway_loop_hits_the_cpu_budget() {
    let log = run_module_page("<script type=module>for(;;){}</script><script type=module>console.log('after');</script>", &[]);
    assert!(log[0].starts_with("error:js: ") && log[0].contains("stopped"), "{log:?}");
    assert_eq!(log[1], "log:after", "{log:?}");
}

#[test]
fn qy15_import_scanner_finds_specifiers_and_ignores_strings_and_comments() {
    let src = "import a from 'a'; import {b as c, d} from \"b\"; import * as ns from './ns.js';\n\
               export * from 'e'; export { x } from 'f'; import 'side';\n\
               // import 'commented'\n/* import 'block' */ var s = \"import 'instr'\"; var t = `import 'tpl'`;\n\
               var r = /import 're'/; import('dyn'); const u = import.meta.url; export const k = 1;";
    assert_eq!(crate::js_modules::scan_import_specifiers(src), ["a", "b", "./ns.js", "e", "f", "side", "dyn"]);
}

#[test]
fn qy15_graph_prefetch_terminates_on_import_cycles() {
    let log = run_module_page(
        "<script type=module>import './a.js'; console.log('done');</script>",
        &[
            ("https://example.test/dir/a.js", "import './b.js'; export const a = 1;"),
            ("https://example.test/dir/b.js", "import './a.js'; export const b = 2;"),
        ],
    );
    assert_eq!(log, ["log:done"], "{log:?}");
}

#[test]
fn qy15_literal_dynamic_import_is_prefetched_only_from_entries() {
    // The entry's own `import('./lazy1.js')` is pre-fetched; the one inside a
    // statically imported module (a lazy chunk of a big app) is not.
    let log = run_module_page(
        "<script type=module>import './a.js'; const m = await import('./lazy1.js'); console.log(m.v); try { await import('./lazy' + '2.js'); } catch (e) { console.log('lazy2 rejected'); }</script>",
        &[
            ("https://example.test/dir/a.js", "export const a = 1; export const f = () => import('./lazy2.js');"),
            ("https://example.test/dir/lazy1.js", "export const v = 'L1';"),
            ("https://example.test/dir/lazy2.js", "export const v = 'L2';"),
        ],
    );
    assert_eq!(log, ["log:L1", "log:lazy2 rejected"], "{log:?}");
}

// ---------- QY16b: Node.prototype.isEqualNode / isSameNode ----------

#[test]
fn qy16b_is_equal_node_compares_subtrees() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function mk(attrs, kids) {\
           var e = document.createElement('div');\
           attrs.forEach(function (p) { e.setAttribute(p[0], p[1]); });\
           kids.forEach(function (k) { e.appendChild(k); });\
           return e;\
         }\
         function p(word) {\
           var q = document.createElement('p');\
           q.appendChild(document.createTextNode('hi '));\
           var b = document.createElement(word);\
           b.appendChild(document.createTextNode('there'));\
           q.appendChild(b);\
           return q;\
         }\
         function tree(comment) {\
           return mk([['class', 'x'], ['data-k', '1']], [p('b'), document.createComment(comment || 'c')]);\
         }\
         var a = tree();\
         console.log(a.isEqualNode(tree()), a.isEqualNode(a), a.isEqualNode(a.firstChild));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '2']], [p('b'), document.createComment('c')])));\
         console.log(a.isEqualNode(tree('d')), a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('i'), document.createComment('c')])));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('b'), document.createComment('c'), document.createElement('span')])));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('b')])));\
         console.log(document.createTextNode('x').isEqualNode(document.createTextNode('x')), document.createTextNode('x').isEqualNode(document.createTextNode('y')));\
         console.log(document.createComment('x').isEqualNode(document.createTextNode('x')), document.createElement('a').isEqualNode(document.createElement('b')));\
         console.log(a.isEqualNode(null), a.isEqualNode(undefined), document.createDocumentFragment().isEqualNode(document.createDocumentFragment()));\
         var t1 = document.implementation.createDocumentType('html', '-//A', 's.dtd'), t2 = document.implementation.createDocumentType('html', '-//A', 't.dtd'), t3 = document.implementation.createDocumentType('html', '-//A', 's.dtd');\
         console.log(t1.isEqualNode(t2), t1.isEqualNode(t3));\
         console.log(a.isSameNode(a), a.isSameNode(tree()), a.isSameNode(null));",
    );
    assert_eq!(
        log,
        [
            "log:true true false",
            "log:false",
            "log:false false",
            "log:false",
            "log:false",
            "log:true false",
            "log:false false",
            "log:false false true",
            "log:false true",
            "log:true false false",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16b_is_equal_node_attributes_are_a_set() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function mk(attrs) {\
           var e = document.createElement('a');\
           attrs.forEach(function (p) { e.setAttribute(p[0], p[1]); });\
           return e;\
         }\
         var a = mk([['href', 'x'], ['title', 't']]);\
         console.log(a.isEqualNode(mk([['title', 't'], ['href', 'x']])), a.isEqualNode(mk([['href', 'y'], ['title', 't']])), a.isEqualNode(mk([['href', 'x']])));\
         var e = mk([['href', 'x'], ['title', 't'], ['data-z', '1']]);\
         console.log(e.isEqualNode(mk([['data-z', '1'], ['href', 'x'], ['title', 't']])), a.isEqualNode(e), a.isEqualNode(mk([['href', 'x'], ['alt', 't']])));\
         console.log(document.createElementNS('http://www.w3.org/2000/svg', 'a').isEqualNode(document.createElement('a')));",
    );
    assert_eq!(
        log,
        ["log:true false false", "log:true false false", "log:false"],
        "{log:?}"
    );
}

// ---------- QY15b: TextEncoder / TextDecoder / atob / btoa ----------

#[test]
fn qy15b_text_encoder_utf8_bytes() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"var e = new TextEncoder();
         function b(s) { return Array.from(e.encode(s)).join(','); }
         console.log(e.encoding, b('abc'), e.encode() instanceof Uint8Array, e.encode().length);
         console.log(b('é€'));
         console.log(b('😀'));
         console.log(b('a\uD800b'), b('\uDC00'));"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-8 97,98,99 true 0",
            "log:195,169,226,130,172",
            "log:240,159,152,128",
            "log:97,239,191,189,98 239,191,189",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_encoder_encode_into_read_and_written() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"var e = new TextEncoder();
         var b4 = new Uint8Array(4), r = e.encodeInto('aéb', b4);
         console.log(r.read, r.written, Array.from(b4).join(','));
         var b3 = new Uint8Array(3);
         r = e.encodeInto('aéb', b3);
         console.log(r.read, r.written, Array.from(b3).join(','));
         r = e.encodeInto('€', new Uint8Array(2));
         console.log(r.read, r.written);
         r = e.encodeInto('😀', new Uint8Array(3));
         console.log(r.read, r.written);
         r = e.encodeInto('x😀', new Uint8Array(5));
         console.log(r.read, r.written);
         try { e.encodeInto('a', 'nope'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:3 4 97,195,169,98",
            "log:2 3 97,195,169",
            "log:0 0",
            "log:0 0",
            "log:3 5",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_utf8_bom_invalid_and_fatal() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var d = new TextDecoder();
         console.log(d.encoding, d.fatal, d.ignoreBOM);
         console.log(d.decode(new Uint8Array([0xEF, 0xBB, 0xBF, 0x68, 0x69])));
         console.log(new TextDecoder('utf-8', { ignoreBOM: true }).decode(new Uint8Array([0xEF, 0xBB, 0xBF, 0x68])).length);
         console.log(cs(d.decode(new Uint8Array([0x61, 0xFF, 0x62]))));
         console.log(cs(d.decode(new Uint8Array([0xE2, 0x82, 0x61]))));
         console.log(cs(d.decode(new Uint8Array([0xED, 0xA0, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xC0, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xF0, 0x9F, 0x98, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xE2, 0x82]))));
         var f = new TextDecoder('utf-8', { fatal: true });
         console.log(f.fatal, f.decode(new Uint8Array([0xF0, 0x9F, 0x98, 0x80])).length);
         try { f.decode(new Uint8Array([0x61, 0xFF])); console.log('no'); } catch (err) { console.log(err.name); }
         var buf = new Uint8Array([0x78, 0x79, 0x7a, 0x7a]).buffer;
         console.log(d.decode(buf), d.decode(new DataView(buf, 1, 2)), d.decode().length);"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-8 false false",
            "log:hi",
            "log:2",
            "log:97,65533,98",
            "log:65533,97",
            "log:65533,65533,65533",
            "log:65533,65533",
            "log:55357,56832",
            "log:65533",
            "log:true 2",
            "log:TypeError",
            "log:xyzz yz 0",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_streaming_splits_characters() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var s = new TextDecoder();
         var a1 = s.decode(new Uint8Array([0x61, 0xE2, 0x82]), { stream: true });
         var a2 = s.decode(new Uint8Array([0xAC, 0x62]));
         console.log(cs(a1), cs(a2));
         var t = new TextDecoder(), out = '';
         [0xF0, 0x9F, 0x98, 0x80].forEach(function (x) { out += t.decode(new Uint8Array([x]), { stream: true }); });
         out += t.decode();
         console.log(cs(out));
         var u = new TextDecoder();
         var p = u.decode(new Uint8Array([0xEF]), { stream: true });
         console.log(p.length, u.decode(new Uint8Array([0xBB, 0xBF, 0x78])));
         var bad = new TextDecoder('utf-8', { fatal: true });
         try { bad.decode(new Uint8Array([0xE2]), { stream: true }); bad.decode(new Uint8Array([0x61])); console.log('no'); } catch (err) { console.log(err.name); }
         console.log(bad.decode(new Uint8Array([0x62])));"#,
    );
    assert_eq!(
        log,
        [
            "log:97 8364,98",
            "log:55357,56832",
            "log:0 x",
            "log:TypeError",
            "log:b",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_utf16le_windows1252_and_labels() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var u = new TextDecoder('utf-16le');
         console.log(u.encoding, cs(u.decode(new Uint8Array([0x61, 0x00, 0x3d, 0xd8, 0x00, 0xde]))));
         console.log(cs(u.decode(new Uint8Array([0xff, 0xfe, 0x62, 0x00]))));
         console.log(cs(u.decode(new Uint8Array([0x61]))));
         console.log(new TextDecoder('utf8').encoding, new TextDecoder('unicode-1-1-utf-8').encoding, new TextDecoder(' LATIN1 ').encoding, new TextDecoder('iso-8859-1').encoding, new TextDecoder('windows-1252').encoding);
         console.log(cs(new TextDecoder('latin1').decode(new Uint8Array([0x41, 0xe9, 0x80]))));
         try { new TextDecoder('klingon'); console.log('no'); } catch (err) { console.log(err.name); }
         try { TextDecoder('utf-8'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-16le 97,55357,56832",
            "log:98",
            "log:65533",
            "log:utf-8 utf-8 windows-1252 windows-1252 windows-1252",
            "log:65,233,8364",
            "log:RangeError",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_atob_btoa_and_invalid_character() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"console.log(btoa(''), btoa('f'), btoa('fo'), btoa('foo'), btoa('foob'), btoa('fooba'), btoa('foobar'));
         console.log(btoa('ÿ'));
         console.log(atob('Zm9vYmFy'), atob('Zm9vYg'), atob(' Zm9v YmE= '), atob('Zg'));
         try { btoa('Ā'); console.log('no'); } catch (err) { console.log(err.name); }
         try { atob('a'); console.log('no'); } catch (err) { console.log(err.name); }
         try { atob('@@@@'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log: Zg== Zm8= Zm9v Zm9vYg== Zm9vYmE= Zm9vYmFy",
            "log:/w==",
            "log:foobar foob fooba f",
            "log:InvalidCharacterError",
            "log:InvalidCharacterError",
            "log:InvalidCharacterError",
        ],
        "{log:?}"
    );
}

// ---------- QF2: Blob, File, FileReader, blob: URL store ----------

#[test]
fn qf2_blob_size_type_slice() {
    let log = run_with_timers(
        "var b = new Blob(['héllo ', new Uint8Array([1, 2, 3]), new ArrayBuffer(2), new DataView(new ArrayBuffer(4))], {type: 'Text/Plain'});\
         console.log(b.size, b.type);\
         console.log(new Blob([], {type: 'a\\u00e9'}).type === '', new Blob().size);\
         var s = b.slice(-3); console.log(s.size, s.type === '');\
         var t = new Blob(['abcdef']).slice(1, -1, 'X/Y'); console.log(t.size, t.type);\
         console.log(new Blob(['abc']).slice(5).size, new Blob(['abc']).slice(2, 1).size);\
         console.log(new Blob(['a\\r\\nb\\rc'], {endings: 'native'}).size, new Blob(['a\\r\\nb']).size);\
         console.log(Object.prototype.toString.call(b), b instanceof Blob);",
    );
    assert_eq!(
        log,
        ["log:16 text/plain", "log:true 0", "log:3 true", "log:4 x/y", "log:0 0", "log:5 4", "log:[object Blob] true"],
        "{log:?}"
    );
}

#[test]
fn qf2_blob_text_arraybuffer_and_nested() {
    let log = run_with_timers(
        "var inner = new Blob(['é', '€']);\
         var outer = new Blob(['x', inner, new Uint8Array([0x41])]);\
         outer.text().then(function (t) { console.log('text', t, outer.size); });\
         outer.arrayBuffer().then(function (ab) { console.log('ab', Array.prototype.join.call(new Uint8Array(ab), ',')); });\
         outer.bytes().then(function (u) { console.log('bytes', u.length); });\
         new Blob([new Uint8Array([0xEF, 0xBB, 0xBF, 0x68, 0xFF])]).text().then(function (t) { console.log('bom', t.length, t.charCodeAt(1)); });",
    );
    assert_eq!(
        log,
        ["log:text xé€A 7", "log:ab 120,195,169,226,130,172,65", "log:bytes 7", "log:bom 2 65533"],
        "{log:?}"
    );
}

#[test]
fn qf2_file_properties() {
    let log = run_with_timers(
        "var f = new File(['hi'], 'a.txt', {type: 'text/plain', lastModified: 1234});\
         console.log(f.name, f.size, f.type, f.lastModified, JSON.stringify(f.webkitRelativePath), f instanceof Blob, f instanceof File);\
         console.log(typeof new File([], 'x').lastModified, f.slice(1).size);\
         try { new File([]); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(log, ["log:a.txt 2 text/plain 1234 \"\" true true", "log:number 1", "log:TypeError"], "{log:?}");
}

#[test]
fn qf2_filereader_events_and_results() {
    let log = run_with_timers(
        "var b = new Blob(['hé'], {type: 'text/plain'});\
         var r = new FileReader(); var ev = [];\
         console.log(r.readyState, r.result);\
         ['loadstart', 'progress', 'load', 'loadend'].forEach(function (n) { r.addEventListener(n, function (e) { ev.push(n + ':' + r.readyState + ':' + e.loaded); }); });\
         r.onload = function () { var res = r.result; setTimeout(function () { console.log('onload', res, ev.join(' ')); }, 1); };\
         r.readAsText(b); console.log(r.readyState, ev.length);\
         try { r.readAsText(b); } catch (e) { console.log(e.name); }\
         var r2 = new FileReader(); r2.onloadend = function () { console.log(r2.result); }; r2.readAsDataURL(b);\
         var r3 = new FileReader(); r3.onload = function () { console.log(r3.result.byteLength, r3.result instanceof ArrayBuffer); }; r3.readAsArrayBuffer(b);\
         var r4 = new FileReader(); r4.onload = function () { console.log(r4.result.length); }; r4.readAsBinaryString(b);\
         var r5 = new FileReader(); var e5 = []; r5.onabort = function () { e5.push('abort'); }; r5.onloadend = function () { e5.push('end'); }; r5.onload = function () { e5.push('load'); };\
         r5.readAsText(b); r5.abort(); console.log(r5.readyState, r5.result);\
         setTimeout(function () { console.log(e5.join(',')); }, 10);",
    );
    assert_eq!(
        log,
        [
            "log:0 null",
            "log:1 0",
            "log:InvalidStateError",
            "log:2 null",
            "log:data:text/plain;base64,aMOp",
            "log:3 true",
            "log:3",
            "log:onload h\u{e9} loadstart:1:0 progress:1:3 load:2:3 loadend:2:3",
            "log:abort,end",
        ],
        "{log:?}"
    );
}

#[test]
fn qf2_object_url_registered_revoked_and_lookup() {
    let log = run_with_timers(
        "var b = new Blob(['zz']); var u = URL.createObjectURL(b); var v = URL.createObjectURL(b);\
         console.log(/^blob:.*\\/[0-9a-f-]{36}$/.test(u) || u, u !== v);\
         console.log(__blob_lookup(u) === b, __blob_lookup(u + '#frag') === b, __blob_lookup('blob:nope'));\
         URL.revokeObjectURL(u); console.log(__blob_lookup(u), __blob_lookup(v) === b);\
         try { URL.createObjectURL({}); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(log, ["log:true true", "log:true true undefined", "log:undefined true", "log:TypeError"], "{log:?}");
}

// ---------- QF4: ReadableStream, Headers, Request, Response ----------

#[test]
fn qf4_headers_normalise_combine_and_iterate_sorted() {
    let log = run_with_timers(
        "var h = new Headers({'X-B': ' two ', 'a-a': '1'});\
         h.append('x-b', '3'); h.append('Set-Cookie', 'a=1'); h.append('set-cookie', 'b=2');\
         console.log(JSON.stringify(Array.from(h)));\
         console.log(h.get('X-B'), h.get('nope'), h.has('A-A'), JSON.stringify(h.getSetCookie()));\
         h.set('X-B', 'only'); h.delete('a-a');\
         console.log(JSON.stringify(Array.from(h.keys())), JSON.stringify(Array.from(h.values())));\
         var seen = []; h.forEach(function (v, k, o) { seen.push(k + '=' + v + (o === h)); }); console.log(seen.join(';'));\
         console.log(JSON.stringify(Array.from(new Headers([['b', '1'], ['A', '2']]))), JSON.stringify(Array.from(new Headers(h))));\
         var bad = [function () { new Headers({'bad name': 'x'}); }, function () { new Headers().append('a', 'x\\ny'); },\
           function () { new Headers([['a']]); }, function () { new Headers().append('a', '\\u0100'); }, function () { new Headers(5); }];\
         console.log(bad.map(function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } }).join());\
         console.log(Object.prototype.toString.call(h), typeof h[Symbol.iterator], new Headers().get('x'));",
    );
    assert_eq!(
        log,
        [
            "log:[[\"a-a\",\"1\"],[\"set-cookie\",\"a=1\"],[\"set-cookie\",\"b=2\"],[\"x-b\",\"two, 3\"]]",
            "log:two, 3 null true [\"a=1\",\"b=2\"]",
            "log:[\"set-cookie\",\"set-cookie\",\"x-b\"] [\"a=1\",\"b=2\",\"only\"]",
            "log:set-cookie=a=1true;set-cookie=b=2true;x-b=onlytrue",
            "log:[[\"a\",\"2\"],[\"b\",\"1\"]] [[\"set-cookie\",\"a=1\"],[\"set-cookie\",\"b=2\"],[\"x-b\",\"only\"]]",
            "log:TypeError,TypeError,TypeError,TypeError,TypeError",
            "log:[object Headers] function null",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_headers_guards_forbidden_and_immutable() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/', {headers: {Cookie: 'x=1', Host: 'h', 'Sec-Fetch-Mode': 'x', 'Proxy-Foo': '1', Origin: 'o', 'X-Ok': '1', 'Content-Length': '3'}});\
         console.log(JSON.stringify(Array.from(r.headers)));\
         r.headers.set('Referer', 'x'); r.headers.append('Via', 'x'); console.log(r.headers.has('referer'), r.headers.has('via'));\
         var s = new Response('x', {headers: {'Set-Cookie': 'a=1', 'X-Ok': '2'}});\
         console.log(JSON.stringify(Array.from(s.headers)), s.headers.getSetCookie().length);\
         var e = Response.error();\
         try { e.headers.set('a', 'b'); } catch (x) { console.log(x.name); }\
         var net = __make_response(200, 'OK', [['Set-Cookie', 'a=1'], ['Content-Type', 'text/x'], ['X', '1'], ['x', '2']], new Uint8Array([104, 105]), 'http://a.test/p', true, 'basic');\
         console.log(JSON.stringify(Array.from(net.headers)), net.headers.get('set-cookie'), net.url, net.redirected, net.type);\
         try { net.headers.append('a', 'b'); } catch (x) { console.log(x.name); }\
         var op = __make_response(200, 'OK', [['a', 'b']], new Uint8Array([1]), 'http://a.test/', false, 'opaque');\
         console.log(op.status, op.url === '', op.body, Array.from(op.headers).length);\
         net.text().then(function (t) { console.log(t, net.bodyUsed); });",
    );
    assert_eq!(
        log,
        [
            "log:[[\"x-ok\",\"1\"]]",
            "log:false false",
            "log:[[\"content-type\",\"text/plain;charset=UTF-8\"],[\"x-ok\",\"2\"]] 0",
            "log:TypeError",
            "log:[[\"content-type\",\"text/x\"],[\"x\",\"1, 2\"]] null http://a.test/p true basic",
            "log:TypeError",
            "log:0 true null 0",
            "log:hi true",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_readablestream_reader_read_done_cancel() {
    let log = run_with_timers(
        "var ctl, cancelled = [], rs = new ReadableStream({start: function (c) { ctl = c; c.enqueue('a'); }, cancel: function (r) { cancelled.push(r); }});\
         console.log(rs.locked, ctl.desiredSize, typeof rs.tee, typeof rs.pipeTo, typeof TransformStream);\
         var rd = rs.getReader(); console.log(rs.locked);\
         try { rs.getReader(); } catch (e) { console.log(e.name); }\
         rd.read().then(function (r) { console.log(JSON.stringify(r)); ctl.enqueue('b'); ctl.close(); return rd.read(); })\
           .then(function (r) { console.log(JSON.stringify(r)); return rd.read(); })\
           .then(function (r) { console.log(r.done, r.value); return rd.closed; })\
           .then(function () { rd.releaseLock(); console.log(rs.locked); return rd.read().catch(function (e) { console.log(e.name); }); })\
           .then(function () {\
              var s2 = new ReadableStream({pull: function (c) { c.enqueue(1); }, cancel: function (r) { cancelled.push(r); }}, {highWaterMark: 0});\
              var r2 = s2.getReader();\
              return r2.read().then(function (x) { console.log('pulled', x.value); return r2.cancel('bye'); }).then(function () { console.log(cancelled.join()); return r2.read(); });\
           }).then(function (x) { console.log('after cancel', x.done);\
              var s3 = new ReadableStream({start: function (c) { c.error(new Error('boom')); }});\
              return s3.getReader().read().catch(function (e) { console.log(e.message); });\
           }).then(function () {\
              var s4 = new ReadableStream({start: function (c) { c.enqueue('x'); c.enqueue('y'); c.close(); }}), out = [];\
              return (async function () { for await (var v of s4) out.push(v); console.log(out.join(), s4.locked); })();\
           });",
    );
    assert_eq!(
        log,
        [
            "log:false 0 undefined undefined undefined",
            "log:true",
            "log:TypeError",
            "log:{\"value\":\"a\",\"done\":false}",
            "log:{\"value\":\"b\",\"done\":false}",
            "log:true undefined",
            "log:false",
            "log:TypeError",
            "log:pulled 1",
            "log:bye",
            "log:after cancel true",
            "log:boom",
            "log:x,y false",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_request_method_url_headers_and_body_rules() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/p?q#f', {method: 'post', body: 'héllo', headers: {A: '1'}});\
         console.log(r.url, r.method, r.mode, r.credentials, r.cache, r.redirect, r.referrer, r.keepalive, JSON.stringify(Array.from(r.headers)));\
         console.log(new Request('http://a.test/', {method: 'patch'}).method, new Request('http://a.test/', {method: 'Options'}).method);\
         var t = ['CONNECT', 'trace', 'TRACK', 'bad method'].map(function (m) { try { new Request('http://a.test/', {method: m}); return 'ok'; } catch (e) { return e.name; } });\
         console.log(t.join());\
         var g = [function () { new Request('http://a.test/', {body: 'x'}); }, function () { new Request('http://a.test/', {method: 'HEAD', body: 'x'}); },\
           function () { new Request('http://u:p@a.test/'); }, function () { new Request('/rel'); }, function () { new Request(); },\
           function () { new Request('http://a.test/', {mode: 'navigate'}); }, function () { new Request('http://a.test/', {cache: 'zzz'}); }];\
         console.log(g.map(function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } }).join());\
         var c = new Request(r, {method: 'PUT'}); console.log(c.method, c.url, r.bodyUsed, c.headers.get('a'), c.headers.get('content-type'));\
         var ab = new AbortController(), q = new Request('http://a.test/', {signal: ab.signal});\
         console.log(q.signal.aborted, q.signal === ab.signal); ab.abort(); console.log(q.signal.aborted);\
         console.log(new Request('http://a.test/', {signal: AbortSignal.abort()}).signal.aborted, Object.prototype.toString.call(q));\
         var u = new Request('http://a.test/', {method: 'POST', body: new URLSearchParams({a: '1 2'})});\
         console.log(u.headers.get('content-type'));\
         u.text().then(function (s) { console.log(s, u.bodyUsed); });\
         new Request('http://a.test/', {method: 'POST', body: new Blob(['xy'], {type: 'text/Foo'})}).text().then(function (s) { console.log(s); });\
         new Request('http://a.test/', {method: 'POST', body: new Uint8Array([104, 105])}).arrayBuffer().then(function (b) { console.log(b.byteLength); });\
         try { new Request('http://a.test/', {method: 'POST', body: {}}).text().then(function (s) { console.log(s); }); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:http://a.test/p?q#f POST cors same-origin default follow about:client false [[\"a\",\"1\"],[\"content-type\",\"text/plain;charset=UTF-8\"]]",
            "log:patch OPTIONS",
            "log:TypeError,TypeError,TypeError,TypeError",
            "log:TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "log:PUT http://a.test/p?q#f true 1 text/plain;charset=UTF-8",
            "log:false false",
            "log:true",
            "log:true [object Request]",
            "log:application/x-www-form-urlencoded;charset=UTF-8",
            "log:a=1+2 true",
            "log:xy",
            "log:2",
            "log:[object Object]",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_request_relative_url_resolves_against_base_href() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_in_html(
        "<html><head><base href=https://x.org/d/></head><body></body></html>",
        "console.log(new Request('').url, new Request('sub/x?y').url, new Request('/abs').url);\
         console.log(Response.redirect('r', 301).headers.get('location'));",
    );
    assert_eq!(log, ["log:https://x.org/d/ https://x.org/d/sub/x?y https://x.org/abs", "log:https://x.org/d/r"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qf4_request_clone_and_body_used() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/', {method: 'POST', body: 'abc'});\
         var c = r.clone(); console.log(r.bodyUsed, c.bodyUsed, c.method, c.url, c.headers === r.headers, c.headers.get('content-type'));\
         r.text().then(function (t) {\
           console.log(t, r.bodyUsed, c.bodyUsed);\
           return r.text().then(function () { console.log('no'); }, function (e) { console.log(e.name, /already used/.test(e.message)); });\
         }).then(function () {\
           try { r.clone(); } catch (e) { console.log('clone', e.name); }\
           try { new Request(r); } catch (e) { console.log('ctor', e.name); }\
           return c.json().catch(function (e) { console.log('json', e.name); });\
         }).then(function () {\
           var n = new Request('http://a.test/', {method: 'POST', body: 'zz'}), m = new Request(n);\
           console.log(n.bodyUsed, m.bodyUsed); return m.text();\
         }).then(function (t) { console.log(t);\
           var ab = new Request('http://a.test/', {method: 'POST', body: 'qq'}).body;\
           console.log(ab instanceof ReadableStream, new Request('http://a.test/').body);\
           return ab.getReader().read();\
         }).then(function (x) { console.log(x.value.length, x.done); });",
    );
    assert_eq!(
        log,
        [
            "log:false false POST http://a.test/ false text/plain;charset=UTF-8",
            "log:abc true false",
            "log:TypeError true",
            "log:clone TypeError",
            "log:ctor TypeError",
            "log:json SyntaxError",
            "log:true false",
            "log:zz",
            "log:true null",
            "log:2 false",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_response_status_json_text_clone_error_redirect() {
    let log = run_with_timers(
        "var r = new Response('hi', {status: 201, statusText: 'Made', headers: {X: '1'}});\
         console.log(r.status, r.statusText, r.ok, r.type, JSON.stringify(r.url), r.redirected, r.headers.get('content-type'), r.headers.get('x'));\
         var bad = [99, 600, 0, 1000].map(function (s) { try { new Response('', {status: s}); return 'ok'; } catch (e) { return e.name; } });\
         console.log(bad.join(), new Response().status, new Response(null, {status: 404}).ok, new Response().statusText === '');\
         try { new Response('x', {status: 204}); } catch (e) { console.log(e.name); }\
         try { new Response('x', {statusText: 'a\\nb'}); } catch (e) { console.log(e.name); }\
         var e = Response.error(); console.log(e.type, e.status, e.ok, e.body);\
         var d = Response.redirect('http://a.test/x', 307); console.log(d.status, d.headers.get('location'), d.type);\
         try { Response.redirect('http://a.test/', 200); } catch (x) { console.log(x.name); }\
         var j = Response.json({a: [1, 'é']}, {status: 202, headers: {'x-y': 'z'}});\
         console.log(j.status, j.headers.get('content-type'), j.headers.get('x-y'));\
         try { Response.json(undefined); } catch (x) { console.log(x.name); }\
         j.clone().text().then(function (t) { console.log(t, j.bodyUsed); return j.json(); })\
          .then(function (v) { console.log(JSON.stringify(v), j.bodyUsed); return j.text().catch(function (x) { console.log(x.name, /already used/.test(x.message)); }); })\
          .then(function () { try { j.clone(); } catch (x) { console.log('clone', x.name); }\
            return new Response('héllo', {headers: {'content-type': 'Text/HTML'}}).blob(); })\
          .then(function (b) { console.log(b.size, b.type, b instanceof Blob);\
            return new Response(new Uint8Array([1, 2, 3])).arrayBuffer(); })\
          .then(function (b) { console.log(b.byteLength); return new Response(new Blob(['xyz'], {type: 'a/b'})).bytes(); })\
          .then(function (b) { console.log(Array.from(b).join(), new Response(new Blob(['x'], {type: 'a/b'})).headers.get('content-type'));\
            return new Response('a=1&b=2').formData().catch(function (x) { console.log('fd', x.name === 'NotSupportedError' || x.name === 'TypeError'); }); })\
          .then(function () { return new Response(new ReadableStream({start: function (c) { c.enqueue(new Uint8Array([104])); c.enqueue(new Uint8Array([105])); c.close(); }})).text(); })\
          .then(function (t) { console.log('stream', t); });",
    );
    assert_eq!(
        log,
        [
            "log:201 Made true default \"\" false text/plain;charset=UTF-8 1",
            "log:RangeError,RangeError,RangeError,RangeError 200 false true",
            "log:TypeError",
            "log:TypeError",
            "log:error 0 false null",
            "log:307 http://a.test/x default",
            "log:RangeError",
            "log:202 application/json z",
            "log:TypeError",
            "log:{\"a\":[1,\"é\"]} false",
            "log:{\"a\":[1,\"é\"]} true",
            "log:TypeError true",
            "log:clone TypeError",
            "log:6 text/html true",
            "log:3",
            "log:120,121,122 a/b",
            "log:fd true",
            "log:stream hi",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_blob_stream_and_body_stream_read_marks_used() {
    let log = run_with_timers(
        "var b = new Blob(['abc']); console.log(typeof b.stream);\
         b.stream().getReader().read().then(function (x) { console.log(x.value.length, x.done); });\
         var r = new Response('abc'); var rd = r.body.getReader(); console.log(r.bodyUsed);\
         rd.read().then(function () { console.log(r.bodyUsed); return r.text().catch(function (e) { console.log(e.name); }); });",
    );
    assert_eq!(log, ["log:function", "log:false", "log:3 false", "log:true", "log:TypeError"], "{log:?}");
}

// ---- QY6e: Custom Elements.

#[test]
fn qy6e_define_upgrades_existing_runs_ctor_once_and_connected() {
    let log = run_in_html(
        "<html><body><x-a id=a1 k=v></x-a><div><x-a id=a2></x-a></div></body></html>",
        r#"let n = 0;
         class XA extends HTMLElement {
           static get observedAttributes() { return ['k']; }
           constructor() { super(); n++; this.tag = 'ctor:' + this.id; }
           connectedCallback() { console.log('conn ' + this.id); }
           attributeChangedCallback(a, o, v) { console.log('attr ' + this.id + ' ' + a + ' ' + o + ' ' + v); }
         }
         const a1 = document.getElementById('a1');
         console.log(a1 instanceof XA, a1 instanceof HTMLElement, a1.tag);
         customElements.define('x-a', XA);
         console.log(n, a1.tag, document.getElementById('a1') === a1, a1 instanceof XA);
         console.log(customElements.get('x-a') === XA, customElements.getName(XA), customElements.get('x-b'));"#,
    );
    assert_eq!(
        log,
        [
            "log:false true undefined",
            "log:attr a1 k null v",
            "log:conn a1",
            "log:conn a2",
            "log:2 ctor:a1 true true",
            "log:true x-a undefined",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_create_element_new_and_innerhtml_construct() {
    let log = run_in_html(
        "<html><body><div id=host></div></body></html>",
        r#"let made = [];
         class XB extends HTMLElement {
           constructor() { super(); made.push('ctor'); }
           connectedCallback() { made.push('conn'); }
           disconnectedCallback() { made.push('disc'); }
         }
         customElements.define('x-b', XB);
         const e = document.createElement('x-b');
         console.log(e instanceof XB, e.localName, made.join());
         const n = new XB();
         console.log(n instanceof XB, n.localName, n.parentNode === null, made.join());
         made.length = 0;
         const host = document.getElementById('host');
         host.innerHTML = '<p><x-b></x-b></p>';
         console.log(host.querySelector('x-b') instanceof XB, made.join());
         made.length = 0;
         host.appendChild(e);
         console.log(made.join());
         made.length = 0;
         host.removeChild(e);
         console.log(made.join());
         made.length = 0;
         host.innerHTML = '';
         console.log(made.join());
         const frag = document.createDocumentFragment();
         frag.appendChild(document.createElement('x-b'));
         made.length = 0;
         host.appendChild(frag);
         console.log(made.join());
         made.length = 0;
         const c = host.firstChild.cloneNode(true);
         console.log(c instanceof XB, made.join());"#,
    );
    assert_eq!(
        log,
        [
            "log:true x-b ctor",
            "log:true x-b true ctor,ctor",
            "log:true ctor,conn",
            "log:conn",
            "log:disc",
            "log:disc",
            "log:conn",
            "log:true ctor",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_attribute_changed_only_for_observed() {
    let log = run_in_html(
        "<html><body><x-c id=c></x-c></body></html>",
        r#"class XC extends HTMLElement {
           static get observedAttributes() { return ['title', 'class']; }
           attributeChangedCallback(a, o, v) { console.log(a + ':' + o + '>' + v); }
         }
         customElements.define('x-c', XC);
         const c = document.getElementById('c');
         c.setAttribute('title', 'a');
         c.setAttribute('lang', 'x');
         c.setAttribute('title', 'b');
         c.className = 'k';
         c.classList.add('m');
         c.removeAttribute('title');
         c.removeAttribute('title');
         c.toggleAttribute('title');
         c.id = 'd';"#,
    );
    assert_eq!(
        log,
        [
            "log:title:null>a",
            "log:title:a>b",
            "log:class:null>k",
            "log:class:k>k m",
            "log:title:b>null",
            "log:title:null>",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_when_defined_resolves_and_errors() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"class XD extends HTMLElement {}
         customElements.whenDefined('x-d').then(function (c) { console.log('resolved ' + (c === XD)); });
         customElements.whenDefined('x-d').then(function () { console.log('second'); });
         console.log('before');
         customElements.define('x-d', XD);
         customElements.whenDefined('x-d').then(function (c) { console.log('already ' + (c === XD)); });
         customElements.whenDefined('bad').catch(function (e) { console.log('rej ' + e.name); });
         const names = [];
         const tryDef = function (n, C, o) { try { customElements.define(n, C, o); names.push('ok'); } catch (e) { names.push(e.name); } };
         tryDef('nohyphen', class extends HTMLElement {});
         tryDef('Has-Upper', class extends HTMLElement {});
         tryDef('font-face', class extends HTMLElement {});
         tryDef('x-d', class extends HTMLElement {});
         tryDef('x-e', XD);
         tryDef('x-f', 5);
         console.log(names.join());"#,
    );
    assert_eq!(
        log,
        [
            "log:before",
            "log:SyntaxError,SyntaxError,SyntaxError,NotSupportedError,NotSupportedError,TypeError",
            "log:resolved true",
            "log:second",
            "log:already true",
            "log:rej SyntaxError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_upgrade_and_is_and_attach_shadow() {
    let log = run_in_html(
        "<html><body><button id=b is=x-btn></button></body></html>",
        r#"class XBtn extends HTMLButtonElement { constructor() { super(); this.mark = 'built'; } connectedCallback() { console.log('conn'); } }
         customElements.define('x-btn', XBtn, { extends: 'button' });
         const b = document.getElementById('b');
         console.log(b instanceof XBtn, b.mark);
         const d = document.createElement('div');
         d.innerHTML = '<x-g></x-g>';
         class XG extends HTMLElement { constructor() { super(); this.up = 1; } }
         customElements.define('x-g', XG);
         console.log(d.firstChild.up);
         customElements.upgrade(d);
         console.log(d.firstChild.up);
         try { d.attachShadow({ mode: 'open' }); console.log(d.shadowRoot.mode); } catch (e) { console.log(e.name); }
         try { new HTMLElement(); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:conn",
            "log:true built",
            "log:undefined",
            "log:1",
            "error:scratchy: js: not implemented: shadow root rendering (QSD6); shadow content is not rendered, only the host's light children",
            "log:open",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_zero_cost_without_define() {
    let log = run_in_html(
        "<html><body><div id=h></div></body></html>",
        r#"const h = document.getElementById('h');
         for (let i = 0; i < 20; i++) { const e = document.createElement('x-z'); e.setAttribute('a', i); h.appendChild(e); }
         h.innerHTML = '<x-z></x-z>'; h.textContent = 'x'; h.removeChild(h.firstChild);
         console.log(__ce_stats.hooks);
         customElements.define('x-y', class extends HTMLElement {});
         h.appendChild(document.createElement('div'));
         console.log(__ce_stats.hooks > 0);"#,
    );
    assert_eq!(log, ["log:0", "log:true"], "{log:?}");
}

#[test]
fn qy6e_document_write_upgrades() {
    let out = qa13_run(
        "<html><body><script>\
         class XW extends HTMLElement { connectedCallback() { console.log('w conn'); } }\
         customElements.define('x-w', XW);\
         document.write('<x-w></x-w>');\
         console.log('end');</script></body></html>",
    );
    assert_eq!(out, ["log:w conn", "log:end"], "{out:?}");
}

// ---------- QY19: <meta http-equiv=refresh> ----------

#[test]
fn qy19_parse_meta_refresh() {
    let base = Url::parse("https://example.com/a/b").unwrap();
    let p = |c: &str| parse_meta_refresh(c, Some(&base)).map(|(d, u)| (d, u.to_string()));
    assert_eq!(p("0; url='x'"), Some((0, "https://example.com/a/x".into())));
    assert_eq!(p("5;URL=x"), Some((5, "https://example.com/a/x".into())));
    assert_eq!(p("  2 , url = \"/y?z=1\""), Some((2, "https://example.com/y?z=1".into())));
    assert_eq!(p("0;url=\"https://html.duckduckgo.com/html\""), Some((0, "https://html.duckduckgo.com/html".into())));
    assert_eq!(p("1; x"), Some((1, "https://example.com/a/x".into())), "url= prefix optional");
    assert_eq!(p("999;url=x").map(|r| r.0), Some(10), "delay clamped");
    assert_eq!(p("0; url=x#frag"), Some((0, "https://example.com/a/x#frag".into())));
    // Bare reload, junk, empty URL: never followed.
    assert_eq!(p("3"), None);
    assert_eq!(p("3;"), None);
    assert_eq!(p("0; url="), None);
    assert_eq!(p("url=x"), None);
    assert_eq!(p("-1; url=x"), None);
    assert_eq!(p(""), None);
}

#[test]
fn qy19_find_meta_refresh_and_chain_cap() {
    let base = Url::parse("https://example.com/").unwrap();
    let doc = parse_html(r#"<html><head><meta HTTP-EQUIV="Refresh" content="0; url=/next"></head><body></body></html>"#);
    let (d, u) = meta_refresh_action(&doc, Some(&base), 0).expect("found");
    assert_eq!((d, u.as_str()), (0, "https://example.com/next"));
    assert!(meta_refresh_action(&doc, Some(&base), META_REFRESH_MAX_CHAIN).is_none(), "chain capped");
    let other = parse_html(r#"<html><head><meta http-equiv="content-type" content="0; url=/x"></head></html>"#);
    assert!(meta_refresh_action(&other, Some(&base), 0).is_none());
}

#[test]
fn qy19_noscript_meta_only_with_js_off() {
    let html = r#"<html><head><noscript><meta http-equiv="refresh" content="0; url=/h"></noscript></head><body></body></html>"#;
    let was = js_enabled();
    set_js_enabled(false);
    let off = new_parser().read_from(&mut html.as_bytes()).unwrap().document;
    set_js_enabled(true);
    let on = new_parser().read_from(&mut html.as_bytes()).unwrap().document;
    set_js_enabled(was);
    assert!(find_meta_refresh_content(&off).is_some(), "JS off: noscript meta is an element");
    assert!(find_meta_refresh_content(&on).is_none(), "JS on: noscript meta ignored");
}

#[test]
fn qy19_meta_redirect_replaces_history_and_is_chained() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.history = vec![Url::parse("http://127.0.0.1:9/start").unwrap(), Url::parse("http://127.0.0.1:9/redir").unwrap()];
    app.history_index = 1;
    app.base_url = Some(Url::parse("http://127.0.0.1:9/redir").unwrap());
    app.fetch_url(app.history[1].clone());
    let generation = app.nav_generation;
    let page = FetchedPage {
        html: br#"<html><head><meta http-equiv="refresh" content="0; url=/final#sec"></head><body>old</body></html>"#.to_vec(),
        extra_css: String::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/redir").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert_eq!(app.history.len(), 2, "no extra back step");
    assert_eq!(app.history[1].as_str(), "http://127.0.0.1:9/final#sec", "entry replaced");
    assert_eq!(app.history_index, 1);
    assert_eq!(app.meta_chain, 1);
    assert!(app.nav_generation > generation, "a new fetch was started");
    assert_eq!(app.nav_fragment.as_deref(), Some("sec"), "fragment of the target kept");
    // A stale delayed refresh (user navigated meanwhile) is ignored.
    let g = app.nav_generation;
    app.handle_meta_refresh(g - 1, Url::parse("http://127.0.0.1:9/zzz").unwrap());
    assert_eq!(app.nav_generation, g);
    // A current one pushes a new entry.
    app.handle_meta_refresh(g, Url::parse("http://127.0.0.1:9/zzz").unwrap());
    assert_eq!(app.history.len(), 3);
}

fn qy17_style(css: &str, html: &str) -> ComputedStyle {
    let doc = parse_html(html);
    let p = find_element_by_tag(&doc, "p").expect("no <p>");
    resolve_style(&p, &parse_stylesheet(css), ROOT_STYLE, 800.0)
}

#[test]
fn qy17_layered_rules_apply_unlayered_beat_them_and_later_layers_win() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = r#"<p id="a" class="c">x</p>"#;
    // A layered rule applies when nothing else does (it was dropped before).
    assert_eq!(qy17_style("@layer a{p{color:red}}", h).color, red);
    // Unlayered beats layered regardless of specificity and order.
    assert_eq!(qy17_style("@layer a{#a{color:red}} p{color:green}", h).color, green);
    assert_eq!(qy17_style("p{color:green} @layer a{#a.c{color:red}}", h).color, green);
    // Later layer beats earlier, regardless of specificity.
    assert_eq!(qy17_style("@layer a{p{color:green}} @layer b{p{color:red}}", h).color, red);
    assert_eq!(qy17_style("@layer a{#a{color:red}} @layer b{p{color:green}}", h).color, green);
    // Same layer: specificity then order as usual.
    assert_eq!(qy17_style("@layer a{#a{color:green} p{color:red}}", h).color, green);
    // Anonymous layers are separate, ordered layers.
    assert_eq!(qy17_style("@layer{p{color:red}} @layer{p{color:green}}", h).color, green);
}

#[test]
fn qy17_layer_statement_fixes_order_and_blocks_reopen() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    assert_eq!(qy17_style("@layer a, b; @layer b{p{color:green}} @layer a{p{color:red}}", h).color, green);
    // Without the statement the first block mentioned is the earlier layer.
    assert_eq!(qy17_style("@layer b{p{color:green}} @layer a{p{color:red}}", h).color, red);
    // A block reopens an earlier layer without moving it.
    assert_eq!(qy17_style("@layer a{p{color:green}} @layer b{p{color:red}} @layer a{p{color:green}}", h).color, red);
}

#[test]
fn qy17_important_reverses_layer_order() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    // Earlier layer's important beats a later layer's important.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} @layer b{p{color:red!important}}", h).color, green);
    // Layered important beats unlayered important.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} p{color:red!important}", h).color, green);
    // Any important beats a normal unlayered declaration.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} p{color:red}", h).color, green);
    // Specificity within a layer still orders important declarations.
    assert_eq!(qy17_style("@layer a{#x{color:green!important} p{color:red!important}}", r#"<p id="x">x</p>"#).color, green);
}

#[test]
fn qy17_nested_layers_and_media_supports_nesting() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    // `a.b` is a sub-layer of `a`: it loses to rules directly in `a`.
    assert_eq!(qy17_style("@layer a{@layer b{p{color:red}} p{color:green}}", h).color, green);
    assert_eq!(qy17_style("@layer a.b{p{color:red}} @layer a{p{color:green}}", h).color, green);
    // Sub-layers order within their parent; the parent's order decides across parents.
    assert_eq!(qy17_style("@layer x.q, y; @layer y{p{color:green}} @layer x.q{p{color:red}}", h).color, green);
    assert_eq!(qy17_style("@layer a.p, a.q; @layer a.q{p{color:green}} @layer a.p{p{color:red}}", h).color, green);
    // Layer inside @media / @supports, and @media inside a layer.
    assert_eq!(qy17_style("@media screen{@layer a{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@media print{@layer a{p{color:red}}}", h).color, ROOT_STYLE.color);
    assert_eq!(qy17_style("@supports (display:block){@layer a{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@layer a{@media screen{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@layer a{@media print{p{color:red}}}", h).color, ROOT_STYLE.color);
    assert_eq!(qy17_style("@layer a{@media screen{p{color:red}}} p{color:green}", h).color, green);
}

#[test]
fn qy17_hidden_attribute_important_inside_a_layer_hides_the_element() {
    let css = "@layer primer{template,[hidden]{display:none!important}} .x{display:flex}";
    let s = qy17_style(css, r#"<p hidden class="x">x</p>"#);
    assert!(matches!(s.display, Display::None));
    let s = qy17_style(css, r#"<p class="x">x</p>"#);
    assert!(!matches!(s.display, Display::None));
}

#[test]
fn qy17_user_agent_rules_stay_below_author_rules() {
    // The UA sheet gives <p> margins; a layered, low-specificity author rule
    // still beats it (UA is its own, lower origin).
    let doc = parse_html("<p>x</p>");
    let p = find_element_by_tag(&doc, "p").unwrap();
    let mut rules = parse_ua_stylesheet("p{margin-top:20px}");
    rules.extend(parse_stylesheet("@layer a{p{margin-top:3px}}"));
    let s = resolve_style(&p, &rules, ROOT_STYLE, 800.0);
    assert!(matches!(s.margin.top, BoxLength::Px(v) if v == 3.0));
}

#[test]
fn qy17_import_layer_wraps_the_imported_sheet() {
    let base = Url::parse("https://e.com/css/main.css").unwrap();
    let mut fetch = |u: &Url| -> Option<String> {
        match u.path() {
            "/css/a.css" => Some("p{color:red}".to_string()),
            _ => None,
        }
    };
    let out = expand_css_imports("@import url(a.css) layer(x.y) screen;\n@import \"a.css\" layer;", &base, &mut vec![base.clone()], &mut fetch);
    assert!(out.contains("@layer x.y {\n@media screen {\np{color:red}"), "{out}");
    assert!(out.contains("@layer {\np{color:red}"), "{out}");
    // Layered import loses to an unlayered rule even when written after it.
    let out = expand_css_imports("@import url(a.css) layer(x);\np{color:green}", &base, &mut vec![base.clone()], &mut fetch);
    assert_eq!(qy17_style(&out, "<p>x</p>").color, TextColor::rgb(0, 128, 0));
}

// ---------- QF3: FormData (data class; no form reading, no submission) ----------

#[test]
fn qf3_formdata_append_get_getall_has_set_delete() {
    let log = run_with_timers(
        "var f = new FormData(); f.append('a', '1'); f.append('b', 2); f.append('a', true);\
         console.log(f.get('a'), f.get('b'), f.get('zz'), f.getAll('a').join('|'), f.getAll('nope').length);\
         console.log(f.has('a'), f.has('c'));\
         f.set('a', 'x'); console.log(f.getAll('a').join('|'), f.get('b'));\
         f.set('new', 'n'); console.log(f.get('new'));\
         f.delete('a'); console.log(f.has('a'), f.getAll('a').length);\
         f.append('wh\\ud800', 'v\\udc00'); console.log(f.get('wh\\ufffd'), f.get('wh\\ufffd').length, f.get('wh\\ud800') === 'v\\ufffd');\
         try { f.append('only'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:1 2 null 1|true 0",
            "log:true false",
            "log:x 2",
            "log:n",
            "log:false 0",
            "log:v\u{fffd} 2 true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_iteration_order_and_for_each() {
    let log = run_with_timers(
        "var f = new FormData(); f.append('b', '1'); f.append('a', '2'); f.append('b', '3');\
         console.log(Array.from(f.keys()).join(','), Array.from(f.values()).join(','));\
         console.log(Array.from(f).map(function (e) { return e[0] + '=' + e[1]; }).join('&'));\
         console.log(f[Symbol.iterator] === f.entries, Object.prototype.toString.call(f), Object.prototype.toString.call(f.entries()));\
         var seen = []; f.forEach(function (v, k, fd) { seen.push(k + ':' + v + ':' + (fd === f)); }, null); console.log(seen.join(' '));\
         var it = f.keys(); console.log(it.next().value, it.next().value, it.next().value, it.next().done, it.next().done);\
         try { f.forEach(1); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:b,a,b 1,2,3",
            "log:b=1&a=2&b=3",
            "log:true [object FormData] [object FormData Iterator]",
            "log:b:1:true a:2:true b:3:true",
            "log:b a b true true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_blob_value_becomes_file() {
    let log = run_with_timers(
        "var fd = new FormData();\
         fd.append('p', new Blob(['abc'], {type: 'text/plain'}));\
         var v = fd.get('p'); console.log(v instanceof File, v.name, v.size, v.type);\
         v.text().then(function (t) { console.log('text', t); });\
         fd.append('q', new File(['xy'], 'orig.txt', {type: 'a/b'})); console.log(fd.get('q').name);\
         fd.append('r', new File(['xy'], 'orig.txt'), 'renamed.txt'); console.log(fd.get('r').name, fd.get('r').size);\
         fd.append('s', new Blob(['z']), 'named'); console.log(fd.get('s').name, fd.get('s') instanceof File);\
         fd.append('t', 42, 'ignored'); console.log(typeof fd.get('t'), fd.get('t'));\
         fd.append('u', new Blob(['w'])); console.log(fd.getAll('u')[0].name);",
    );
    assert_eq!(
        log,
        [
            "log:true blob 3 text/plain",
            "log:orig.txt",
            "log:renamed.txt 2",
            "log:named true",
            "log:string 42",
            "log:blob",
            "log:text abc",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_form_argument_logs_and_is_empty() {
    // Q30e: FormData(form) now reads the form (an empty form gives an empty set, no log).
    let log = run_with_timers(
        "var none = new FormData();\
         var fd = new FormData(document.createElement('form'), null);\
         console.log(fd.has('x'), Array.from(fd).length, none.keys().next().done);",
    );
    assert_eq!(
        log,
        ["log:false 0 true"],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_request_and_response_bodies() {
    let log = run_with_timers(
        "var fd = new FormData(); fd.append('a', 'x\\ny\"z'); fd.append('f', new File(['hi'], 'h.txt', {type: 'text/plain'}));\
         var req = new Request('http://x.test/', {method: 'POST', body: fd}), ct = req.headers.get('content-type');\
         req.text().then(function (t) {\
           var bd = ct.split('boundary=')[1];\
           console.log(ct.indexOf('multipart/form-data; boundary=----scratchyBoundary') === 0, t.indexOf('--' + bd + '\\r\\n') === 0,\
             t.indexOf('name=\"a\"') > 0, t.indexOf('x\\r\\ny\"z') > 0,\
             t.indexOf('name=\"f\"; filename=\"h.txt\"\\r\\nContent-Type: text/plain\\r\\n\\r\\nhi\\r\\n') > 0,\
             t.endsWith('--' + bd + '--\\r\\n'), t.split('--' + bd).length === 4);\
         });\
         new Response(new URLSearchParams('a=1&a=2&b=%20')).formData().then(function (f) { console.log(f.getAll('a').join('|'), f.get('b').length, f instanceof FormData); });\
         new Response(fd).formData().catch(function (e) { console.log(e.name); });",
    );
    assert_eq!(
        log,
        [
            "log:true true true true true true true",
            "log:1|2 1 true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

// ---------- QF5: net policy + wire codec ----------

fn qf5_url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn qf5_is_public_ip_classes() {
    let no = [
        "127.0.0.1", "127.9.9.9", "10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "169.254.0.1",
        "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3", "224.0.0.1", "255.255.255.255", "192.0.2.1", "198.51.100.1",
        "203.0.113.1", "198.18.0.1", "240.0.0.1", "::1", "::", "fe80::1", "febf::1", "fc00::1", "fd12::1", "ff02::1",
        "2001:db8::1", "::ffff:127.0.0.1", "::ffff:10.1.2.3", "::ffff:169.254.169.254", "64:ff9b::7f00:1",
    ];
    for s in no {
        assert!(!is_public_ip(s.parse().unwrap()), "{s} must not be public");
    }
    let yes = ["8.8.8.8", "1.1.1.1", "172.15.0.1", "172.32.0.1", "100.63.255.255", "100.128.0.1", "2606:4700::1111", "::ffff:8.8.8.8"];
    for s in yes {
        assert!(is_public_ip(s.parse().unwrap()), "{s} must be public");
    }
}

fn qf5_block_policy() -> RequestPolicy {
    let mut p = RequestPolicy::default();
    p.private_targets = PrivatePolicy::Block;
    p.file_targets = PrivatePolicy::Block;
    p
}

#[test]
fn qf5_warn_and_allow_modes_and_redirect_hops() {
    let p = RequestPolicy::default();
    assert_eq!((p.private_targets, p.file_targets), (PrivatePolicy::Warn, PrivatePolicy::Warn));
    let page = qf5_url("https://example.com/a");
    let cases = [
        ("https://127.0.0.1/", "loopback"), ("https://localhost/", "loopback"), ("https://[::1]/", "loopback"),
        ("https://10.0.0.5/", "private"), ("https://192.168.0.1/", "private"), ("https://172.20.1.1/", "private"),
        ("https://169.254.169.254/", "link-local"), ("https://[fe80::1]/", "link-local"),
        ("https://100.64.0.9/", "cgnat"), ("https://[fd00::1]/", "ula"), ("https://[::ffff:10.0.0.1]/", "private"),
        ("https://nas.local/", "local-name"), ("https://db.internal/", "local-name"), ("file:///etc/os-release", "file"),
    ];
    for (u, class) in cases {
        let out = p.check_request(&page, &qf5_url(u), "GET", false).unwrap_or_else(|e| panic!("{u}: {e}"));
        assert_eq!(out.warning, Some(class), "{u}");
    }
    assert_eq!(p.check_request(&page, &qf5_url("https://example.org/"), "GET", false).unwrap().warning, None);
    // same origin never warns
    let dev = qf5_url("http://localhost:3000/");
    assert_eq!(p.check_request(&dev, &qf5_url("http://localhost:3000/x"), "GET", false).unwrap().warning, None);
    // still enforced in Warn mode: schemes, ports, mixed content (except loopback), methods
    assert!(p.check_request(&page, &qf5_url("ftp://x.test/"), "GET", false).is_err());
    assert_eq!(p.check_request(&page, &qf5_url("http://127.0.0.1:22/"), "GET", false), Err(PolicyError::BadPort(22)));
    assert_eq!(p.check_request(&page, &qf5_url("http://10.0.0.1/"), "GET", false), Err(PolicyError::MixedContent));
    assert_eq!(p.check_request(&page, &qf5_url("http://localhost:8080/"), "GET", false).unwrap().warning, Some("loopback"));
    assert!(p.check_request(&page, &qf5_url("https://10.0.0.1/"), "TRACE", false).is_err());
    // Allow: no warning
    let mut a = RequestPolicy::default();
    a.private_targets = PrivatePolicy::Allow;
    a.file_targets = PrivatePolicy::Allow;
    assert_eq!(a.check_request(&page, &qf5_url("https://10.0.0.1/"), "GET", false).unwrap().warning, None);
    assert_eq!(a.check_request(&page, &qf5_url("file:///x"), "GET", false).unwrap().warning, None);
    // the warning line
    assert_eq!(
        format_net_warning(&page, &qf5_url("https://10.0.0.1/x"), "private"),
        "scratchy: net warning: https://example.com/a -> https://10.0.0.1/x (private) allowed by policy"
    );
    // redirect hops re-run the policy
    let b = qf5_block_policy();
    let (m, body, _) = b.check_redirect_hop(&page, &qf5_url("https://cdn.example.org/y"), 302, "POST", true, 1).unwrap();
    assert_eq!((m.as_str(), body), ("GET", false));
    let (m, body, _) = b.check_redirect_hop(&page, &qf5_url("https://cdn.example.org/y"), 307, "POST", true, 1).unwrap();
    assert_eq!((m.as_str(), body), ("POST", true));
    assert!(matches!(b.check_redirect_hop(&page, &qf5_url("http://169.254.169.254/"), 302, "GET", false, 1), Err(_)));
    assert!(matches!(b.check_redirect_hop(&page, &qf5_url("https://10.0.0.1/"), 302, "GET", false, 2), Err(PolicyError::PrivateHost(_))));
    assert_eq!(p.check_redirect_hop(&page, &qf5_url("https://10.0.0.1/"), 302, "GET", false, 2).unwrap().2.warning, Some("private"));
    assert_eq!(p.check_redirect_hop(&page, &qf5_url("https://x.example/"), 302, "GET", false, 6).unwrap_err(), PolicyError::Redirects(5));
}

#[test]
fn qf5_check_request_rules() {
    let p = qf5_block_policy();
    let page = qf5_url("https://example.com/a");
    let ok = |u: &str, m: &str, body: bool| p.check_request(&page, &qf5_url(u), m, body);
    assert!(ok("https://api.example.org/x", "GET", false).is_ok());
    // address classes by IP literal and name
    for u in [
        "https://127.0.0.1/", "https://10.0.0.5/", "https://[::1]/", "https://169.254.169.254/latest", "https://[::ffff:7f00:1]/",
        "https://localhost/", "https://a.localhost/", "https://printer.local/", "https://db.internal/", "https://LOCALHOST./",
        "https://0x7f.1/",
    ] {
        assert!(matches!(ok(u, "GET", false), Err(PolicyError::PrivateHost(_))), "{u}");
    }
    // schemes
    assert_eq!(ok("file:///etc/passwd", "GET", false), Err(PolicyError::FileNotAllowed));
    assert!(matches!(ok("ftp://example.com/", "GET", false), Err(PolicyError::Scheme(_))));
    assert!(matches!(ok("data:text/plain,hi", "GET", false), Err(PolicyError::Scheme(_))));
    // ports
    for port in [1, 22, 25, 101, 104, 513, 6000, 6667, 10080] {
        assert_eq!(ok(&format!("https://example.org:{port}/"), "GET", false), Err(PolicyError::BadPort(port)));
    }
    assert!(ok("https://example.org:8443/", "GET", false).is_ok());
    // mixed content
    assert_eq!(ok("http://example.org/", "GET", false), Err(PolicyError::MixedContent));
    assert!(p.check_request(&qf5_url("http://example.com/"), &qf5_url("http://example.org/"), "GET", false).is_ok());
    // methods and bodies
    for (m, body) in [("POST", true), ("PUT", true), ("PATCH", true), ("DELETE", true), ("OPTIONS", false), ("HEAD", false), ("post", true)] {
        assert!(ok("https://example.org/", m, body).is_ok(), "{m}");
    }
    for m in ["CONNECT", "TRACE", "track", "TRACK"] {
        assert!(matches!(ok("https://example.org/", m, false), Err(PolicyError::MethodForbidden(_))), "{m}");
    }
    assert!(matches!(ok("https://example.org/", "GET", true), Err(PolicyError::BodyNotAllowed(_))));
    assert!(matches!(ok("https://example.org/", "HEAD", true), Err(PolicyError::BodyNotAllowed(_))));
    assert!(matches!(ok("https://example.org/", "bad method", false), Err(PolicyError::MethodForbidden(_))));
    // url length
    let long = format!("https://example.org/{}", "a".repeat(p.max_url_len));
    assert!(matches!(ok(&long, "GET", false), Err(PolicyError::UrlTooLong(_))));
    // configurable
    let mut q = qf5_block_policy();
    q.private_targets = PrivatePolicy::Allow;
    assert_eq!(q.check_request(&page, &qf5_url("https://localhost/"), "GET", false), Ok(PolicyOutcome::default()));
}

#[test]
fn qf5_same_origin_exception_and_file_flag() {
    let p = qf5_block_policy();
    let dev = qf5_url("http://localhost:3000/app");
    assert!(p.check_request(&dev, &qf5_url("http://localhost:3000/api"), "POST", true).is_ok());
    assert!(p.check_request(&dev, &qf5_url("http://localhost:3001/api"), "GET", false).is_err());
    assert!(p.check_request(&dev, &qf5_url("http://127.0.0.1:3000/api"), "GET", false).is_err());
    // the bad-port check still applies to a same-origin private target
    let bad = qf5_url("http://localhost:6000/");
    assert_eq!(p.check_request(&bad, &bad, "GET", false), Err(PolicyError::BadPort(6000)));
    // file:
    let file_page = qf5_url("file:///home/u/a.html");
    assert_eq!(p.check_request(&file_page, &qf5_url("file:///etc/passwd"), "GET", false), Err(PolicyError::FileNotAllowed));
    let mut q = qf5_block_policy();
    q.file_targets = PrivatePolicy::Warn;
    assert_eq!(q.check_request(&file_page, &qf5_url("file:///home/u/b.json"), "GET", false).unwrap().warning, Some("file"));
    // a file page is never same-origin, so private hosts stay blocked in Block mode
    assert!(q.check_request(&file_page, &qf5_url("http://localhost/"), "GET", false).is_err());
}

fn qf5_h(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn qf5_cors_classification_and_preflight() {
    let page = qf5_url("https://a.example/p");
    let same = qf5_url("https://a.example/x");
    let other = qf5_url("https://b.example/x");
    assert_eq!(classify_fetch(RequestMode::SameOrigin, &page, &same, "GET", &[]), Ok(FetchKind::Basic));
    assert!(classify_fetch(RequestMode::SameOrigin, &page, &other, "GET", &[]).is_err());
    assert_eq!(classify_fetch(RequestMode::NoCors, &page, &other, "GET", &[]), Ok(FetchKind::Opaque));
    assert!(classify_fetch(RequestMode::NoCors, &page, &other, "PUT", &[]).is_err());
    assert_eq!(classify_fetch(RequestMode::Cors, &page, &other, "GET", &[]), Ok(FetchKind::Cors { preflight: false }));
    assert_eq!(
        classify_fetch(RequestMode::Cors, &page, &other, "POST", &qf5_h(&[("Content-Type", "text/plain;charset=UTF-8"), ("Accept", "*/*")])),
        Ok(FetchKind::Cors { preflight: false })
    );
    assert_eq!(
        classify_fetch(RequestMode::Cors, &page, &other, "POST", &qf5_h(&[("Content-Type", "application/json")])),
        Ok(FetchKind::Cors { preflight: true })
    );
    assert!(needs_preflight("PUT", &[]) && needs_preflight("DELETE", &[]) && needs_preflight("PATCH", &[]));
    assert!(needs_preflight("GET", &qf5_h(&[("X-Token", "1")])));
    assert!(needs_preflight("GET", &qf5_h(&[("Authorization", "x")])));
    assert!(!needs_preflight("get", &qf5_h(&[("accept-language", "en")])));
    assert!(needs_preflight("GET", &qf5_h(&[("Accept", "a\"b")])));
    assert_eq!(cors_unsafe_request_headers(&qf5_h(&[("X-B", "1"), ("x-a", "1"), ("X-B", "2"), ("Accept", "*/*")])), ["x-a", "x-b"]);
}

#[test]
fn qf5_cors_response_decisions() {
    let o = "https://a.example";
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", "*")])).is_ok());
    // `*` with credentials is refused
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", "*")])).is_err());
    assert!(check_cors_response(o, false, &qf5_h(&[("access-control-allow-origin", o)])).is_ok());
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", "https://b.example")])).is_err());
    assert!(check_cors_response(o, false, &[]).is_err());
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", o), ("Access-Control-Allow-Origin", o)])).is_err());
    // credentials need an exact origin plus Allow-Credentials: true
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", o)])).is_err());
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", o), ("Access-Control-Allow-Credentials", "true")])).is_ok());
    // file pages send Origin: null
    assert_eq!(origin_header(&qf5_url("file:///a.html")), "null");
    assert_eq!(origin_header(&qf5_url("https://a.example:8443/x")), "https://a.example:8443");

    let p = RequestPolicy::default();
    let uh = vec!["x-token".to_string(), "content-type".to_string()];
    let good = qf5_h(&[
        ("Access-Control-Allow-Origin", o),
        ("Access-Control-Allow-Methods", "GET, PUT"),
        ("Access-Control-Allow-Headers", "X-Token, Content-Type"),
        ("Access-Control-Max-Age", "99999"),
    ]);
    assert_eq!(p.check_preflight_response(o, false, "PUT", &uh, 204, &good).unwrap().max_age_secs, p.preflight_max_age_cap_secs);
    assert!(p.check_preflight_response(o, false, "DELETE", &uh, 204, &good).is_err());
    assert!(p.check_preflight_response(o, false, "PUT", &["x-other".to_string()], 204, &good).is_err());
    assert!(p.check_preflight_response(o, false, "PUT", &uh, 404, &good).is_err());
    let star = qf5_h(&[("Access-Control-Allow-Origin", "*"), ("Access-Control-Allow-Methods", "*"), ("Access-Control-Allow-Headers", "*")]);
    let r = p.check_preflight_response(o, false, "PATCH", &uh, 200, &star).unwrap();
    assert_eq!(r.max_age_secs, 5);
    assert!(p.check_preflight_response(o, false, "PATCH", &["authorization".to_string()], 200, &star).is_err());
    assert!(p.check_preflight_response(o, true, "PATCH", &uh, 200, &star).is_err());
}

#[test]
fn qf5_forbidden_header_lists_match_js() {
    let js = include_str!("fetch_prelude.js");
    let start = js.find("var FORBIDDEN_REQ = [").unwrap() + "var FORBIDDEN_REQ = [".len();
    let end = start + js[start..].find(']').unwrap();
    let from_js: Vec<String> = js[start..end].split(',').map(|s| s.trim().trim_matches('\'').to_string()).collect();
    let from_rust: Vec<String> = FORBIDDEN_REQUEST_HEADERS.iter().map(|s| s.to_string()).collect();
    assert_eq!(from_js, from_rust);
    assert!(is_forbidden_request_header("Proxy-Authorization") && is_forbidden_request_header("Sec-Fetch-Mode"));
    assert!(is_forbidden_request_header("COOKIE") && !is_forbidden_request_header("X-Token"));
    assert!(is_forbidden_response_header("Set-Cookie") && is_forbidden_response_header("set-cookie2") && !is_forbidden_response_header("etag"));
    assert!(is_wire_reserved_header("X-Wire-Policy") && !is_wire_reserved_header("X-Token"));
}

#[test]
fn qf5_beacon_classification_and_log() {
    assert_eq!(classify_beacon(NetApi::SendBeacon, false, None), Some(BeaconKind::SendBeacon));
    assert_eq!(classify_beacon(NetApi::Fetch, true, Some(FetchKind::Basic)), Some(BeaconKind::Keepalive));
    assert_eq!(classify_beacon(NetApi::Fetch, false, Some(FetchKind::Opaque)), Some(BeaconKind::NoCorsOpaque));
    assert_eq!(classify_beacon(NetApi::Fetch, false, Some(FetchKind::Cors { preflight: false })), None);
    assert_eq!(classify_beacon(NetApi::Xhr, false, None), None);
    let p = RequestPolicy::default();
    assert!(!p.beacons_send && p.suppress_beacon(Some(BeaconKind::Keepalive)) && !p.suppress_beacon(None));
    let body = "x".repeat(300);
    let line = format_beacon_log(
        BeaconKind::SendBeacon,
        "POST",
        &qf5_url("https://ophan.example/img?a=1"),
        Some("text/plain"),
        body.as_bytes(),
        &qf5_url("https://page.example/"),
        false,
    );
    assert!(line.contains("navigator.sendBeacon POST https://ophan.example/img?a=1 content-type=text/plain body=300 bytes"), "{line}");
    assert!(line.contains(&format!("\"{}\"", "x".repeat(200))) && !line.contains(&"x".repeat(201)), "{line}");
    assert!(line.ends_with("page=https://page.example/: suppressed (not sent)"), "{line}");
    let bin = format_beacon_log(BeaconKind::Keepalive, "POST", &qf5_url("https://a.example/"), Some("image/png"), &[1, 2, 3], &qf5_url("https://p.example/"), false);
    assert!(bin.contains("body=3 bytes page=") && !bin.contains("body[.."), "{bin}");
}

#[test]
fn qf5_policy_defaults_are_the_user_numbers() {
    let p = RequestPolicy::default();
    assert_eq!(p.max_response_bytes, 16 << 20);
    assert_eq!(p.max_in_flight_bytes, 64 << 20);
    assert_eq!((p.max_concurrent_requests, p.max_requests_per_page), (6, 1000));
    assert_eq!(p.read_timeout, std::time::Duration::from_secs(30));
    assert_eq!(p.sync_xhr_timeout_ms, 5000);
    assert_eq!(p.redirect_limit, 5);
    let h = p.wire_cap_headers(Some(5000));
    assert!(h.contains(&("X-Wire-Max-Bytes".to_string(), (16usize << 20).to_string())));
    assert!(h.contains(&("X-Wire-Timeout-Ms".to_string(), "5000".to_string())));
    assert!(h.contains(&("X-Wire-Policy".to_string(), "public-only".to_string())));
    assert!(p.wire_cap_headers(None).contains(&("X-Wire-Timeout-Ms".to_string(), "30000".to_string())));
}

#[test]
fn qf5_codec_build_and_parse() {
    let url = qf5_url("https://example.org:8443/p?q=1");
    let req = build_wire_request(
        "POST",
        &url,
        &qf5_h(&[("Content-Type", "application/json"), ("Host", "evil"), ("Content-Length", "99"), ("Connection", "keep-alive"), ("X-A", "1")]),
        b"{\"a\":1}",
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(req).unwrap(),
        "POST https://example.org:8443/p?q=1 HTTP/1.1\r\nHost: example.org:8443\r\nContent-Type: application/json\r\nX-A: 1\r\nContent-Length: 7\r\n\r\n{\"a\":1}"
    );
    let get = String::from_utf8(build_wire_request("GET", &url, &[], b"").unwrap()).unwrap();
    assert!(!get.contains("Content-Length"));
    let put = String::from_utf8(build_wire_request("PUT", &url, &[], b"").unwrap()).unwrap();
    assert!(put.contains("Content-Length: 0\r\n"));
    // CR/LF injection and bad names are refused
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X-A", "1\r\nX-B: 2")]), b"").is_err());
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X-A", "1\nX")]), b"").is_err());
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X A", "1")]), b"").is_err());
    assert!(build_wire_request("GE T", &url, &[], b"").is_err());

    let raw = b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nX-Wire-Final-Url: https://x.test/final\r\n\r\nhello\r\n\r\nworld";
    let r = parse_wire_response_full(raw).unwrap();
    assert_eq!((r.status, r.status_text.as_str()), (201, "Created"));
    assert_eq!(r.body, b"hello\r\n\r\nworld");
    assert_eq!(r.header("Content-Type"), Some("text/plain"));
    let cookies: Vec<&str> = r.headers.iter().filter(|(n, _)| n == "set-cookie").map(|(_, v)| v.as_str()).collect();
    assert_eq!(cookies, ["a=1", "b=2"]);
    assert_eq!(r.final_url.as_deref(), Some("https://x.test/final"));
    assert_eq!(r.error, None);
    // today's synthesized 502 and the QF6 marker
    let e = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\n\r\nwire: upstream request failed: refused").unwrap();
    assert_eq!(e.error.as_deref(), Some("wire: upstream request failed: refused"));
    let e2 = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\nX-Wire-Error: 1\r\n\r\nblocked: x").unwrap();
    assert_eq!(e2.error.as_deref(), Some("blocked: x"));
    let real502 = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\n\r\nnginx says no").unwrap();
    assert_eq!(real502.error, None);
    assert!(parse_wire_response_full(b"HTTP/1.1 200 OK\r\nno terminator").is_err());
    assert!(parse_wire_response_full(b"garbage\r\n\r\n").is_err());
    assert!(parse_wire_response_full(b"HTTP/1.1 999 X\r\n\r\n").is_err());
    assert_eq!(parse_wire_response_full(b"HTTP/1.1 204\r\n\r\n").unwrap().status_text, "");
}

/// In-memory fake wire: a thread on a temp unix socket. `respond` gets the
/// full request bytes and writes the answer; the received request is sent back.
fn qf5_fake_wire(
    tag: &str,
    respond: impl FnOnce(&[u8], &mut UnixStream) + Send + 'static,
) -> (PathBuf, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
    let dir = std::env::temp_dir().join(format!("scratchy-qf5-{}-{tag}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("wire.sock");
    let _ = fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let (tx, rx) = mpsc::channel();
    let h = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = s.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                let cl = head.split("content-length:").nth(1).and_then(|v| v.lines().next()).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                if buf.len() >= p + 4 + cl {
                    break;
                }
            }
        }
        let _ = tx.send(buf.clone());
        respond(&buf, &mut s);
    });
    (path, rx, h)
}

#[test]
fn qf5_fake_wire_round_trip() {
    let (path, rx, h) = qf5_fake_wire("rt", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: k=v\r\n\r\n{\"ok\":true}");
    });
    let url = qf5_url("https://example.org/api");
    let mut headers = qf5_h(&[("Content-Type", "application/json")]);
    headers.extend(RequestPolicy::default().wire_cap_headers(None));
    let req = build_wire_request("POST", &url, &headers, b"{\"a\":1}").unwrap();
    let resp = fetch_full_via_wire(&path, &req, &WireLimits::default()).unwrap();
    h.join().unwrap();
    let got = rx.recv().unwrap();
    assert_eq!(got, req);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"{\"ok\":true}");
    assert_eq!(resp.header("set-cookie"), Some("k=v"));
}

#[test]
fn qf5_fake_wire_size_cap() {
    let (path, _rx, h) = qf5_fake_wire("cap", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        let big = vec![b'a'; 1 << 20];
        for _ in 0..4 {
            if s.write_all(&big).is_err() {
                break;
            }
        }
    });
    let req = build_wire_request("GET", &qf5_url("https://example.org/"), &[], b"").unwrap();
    let mut limits = WireLimits::default();
    limits.max_body_bytes = 1 << 20;
    assert_eq!(fetch_full_via_wire(&path, &req, &limits), Err(NetError::TooLarge));
    h.join().unwrap();
    // exactly at the cap is fine
    let (path, _rx, h) = qf5_fake_wire("cap2", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        let _ = s.write_all(&vec![b'a'; 1000]);
    });
    limits.max_body_bytes = 1000;
    assert_eq!(fetch_full_via_wire(&path, &req, &limits).unwrap().body.len(), 1000);
    h.join().unwrap();
}

#[test]
fn qf5_fake_wire_timeouts_and_connect_errors() {
    let (path, _rx, h) = qf5_fake_wire("to", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\npartial");
        thread::sleep(std::time::Duration::from_millis(1500));
    });
    let req = build_wire_request("GET", &qf5_url("https://example.org/"), &[], b"").unwrap();
    let mut limits = WireLimits::default();
    limits.read_timeout = std::time::Duration::from_millis(300);
    let t = std::time::Instant::now();
    assert_eq!(fetch_full_via_wire(&path, &req, &limits), Err(NetError::Timeout));
    assert!(t.elapsed() < std::time::Duration::from_millis(1400));
    h.join().unwrap();
    // the legacy caller now has a read timeout too
    let (path, _rx, h) = qf5_fake_wire("to2", |_req, _s| thread::sleep(std::time::Duration::from_millis(1200)));
    let t = std::time::Instant::now();
    let r = fetch_raw_via_wire_with_timeout(&path, "GET http://x.test/ HTTP/1.1\r\nHost: x.test\r\n\r\n", std::time::Duration::from_millis(250));
    assert!(matches!(r, WireFetchRaw::Unreachable));
    assert!(t.elapsed() < std::time::Duration::from_millis(1100));
    h.join().unwrap();
    // nothing listening
    let missing = std::env::temp_dir().join(format!("scratchy-qf5-{}-none.sock", std::process::id()));
    assert!(matches!(fetch_full_via_wire(&missing, &req, &WireLimits::default()), Err(NetError::Connect(_))));
}


// ---------- QF7: page JS network queue (no fetch/XHR globals yet) ----------

use crate::js_net::NetExecutor;

/// A canned executor: answers by URL substring, records every request it
/// receives (url, thread, read timeout) and can sleep first.
struct QfFake {
    calls: Mutex<Vec<(String, thread::ThreadId, std::time::Duration)>>,
    answers: Vec<(&'static str, Result<WireResponse, NetError>)>,
    delay_ms: u64,
    raw: Mutex<Vec<Vec<u8>>>,
}

impl QfFake {
    fn new(answers: Vec<(&'static str, Result<WireResponse, NetError>)>) -> Arc<Self> {
        Arc::new(QfFake { calls: Mutex::new(Vec::new()), answers, delay_ms: 0, raw: Mutex::new(Vec::new()) })
    }
    fn urls(&self) -> Vec<String> {
        self.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }
}

impl NetExecutor for QfFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        self.raw.lock().unwrap().push(job.request.clone());
        let line = String::from_utf8_lossy(&job.request).lines().next().unwrap_or("").to_string();
        let url = line.split(' ').nth(1).unwrap_or("").to_string();
        self.calls.lock().unwrap().push((url.clone(), thread::current().id(), job.limits.read_timeout));
        if self.delay_ms > 0 {
            thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        }
        for (pat, r) in &self.answers {
            if url.contains(pat) {
                return r.clone();
            }
        }
        Err(NetError::Io("no canned answer".into()))
    }
}

fn qf7_ok(body: &str, headers: &[(&str, &str)]) -> Result<WireResponse, NetError> {
    Ok(WireResponse {
        status: 200,
        status_text: "OK".into(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.as_bytes().to_vec(),
        error: None,
        final_url: None,
    })
}

/// Starts an engine on `page` with the fake as the executor (no scripts yet).
fn qf7_engine(page: &str, fake: &Arc<QfFake>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}")))).unwrap();
    (engine, out)
}

fn qf7_drive(engine: &JsEngine, fake: &Arc<QfFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

/// JS that sends one GET and logs `ok <status> <type> <body>` or `err <name> <msg>`.
fn qf7_go(url: &str, extra: &str) -> String {
    format!(
        "var h = __net_request({{method:'GET', url:'{url}', headers:[], body:null{extra}}});\
         h.promise.then(function(r){{ return r.text().then(function(t){{ console.log('ok', r.status, r.type, t); }}); }},\
         function(e){{ console.log('err', e.name, e.message); }});"
    )
}

#[test]
fn qf7_same_origin_request_resolves_in_order() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[])), ("/b", qf7_ok("BBB", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", &format!("{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", "")));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls(), vec!["https://page.test/a", "https://page.test/b"]);
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:ok 200 basic AAA", "log:ok 200 basic BBB"], "answers arrive in request order");
    let e = engine.net_entries();
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].outcome, "200 OK");
    assert_eq!(e[0].bytes, 3);
}

#[test]
fn qf7_policy_refusal_never_reaches_the_executor_and_rejects_async() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", &format!("{}console.log('sync-done');", qf7_go("ftp://page.test/x", "")));
    assert_eq!(log.borrow().last().map(String::as_str), Some("log:sync-done"), "no rejection before the turn ends");
    qf7_drive(&engine, &fake);
    assert!(fake.urls().is_empty());
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:blocked GET ftp://page.test/x: scheme ftp:")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(engine.net_entries()[0].outcome.starts_with("blocked: "));
}

#[test]
fn qf7_private_target_warns_and_is_sent() {
    let fake = QfFake::new(vec![("127.0.0.1", qf7_ok("x", &[("access-control-allow-origin", "*")]))]);
    let (engine, log) = qf7_engine("http://page.test/", &fake);
    engine.eval("t", &qf7_go("http://127.0.0.1:8080/x", ""));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 1, "warn-and-allow");
    let l = log.borrow().clone();
    assert!(
        l.contains(&"net-warning:scratchy: net warning: http://page.test/ -> http://127.0.0.1:8080/x (loopback) allowed by policy".to_string()),
        "{l:?}"
    );
    assert_eq!(engine.net_entries()[0].warnings.len(), 1);
    assert!(l.contains(&"log:ok 200 cors x".to_string()), "{l:?}");
}

#[test]
fn qf7_cors_blocked_allowed_and_headers_filtered() {
    let fake = QfFake::new(vec![
        ("/closed", qf7_ok("secret", &[])),
        ("/open", qf7_ok("pub", &[("access-control-allow-origin", "*"), ("x-hidden", "1"), ("content-type", "text/plain")])),
    ]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{}{}{}",
            qf7_go("https://other.test/closed", ""),
            qf7_go("https://other.test/open", ""),
            "__net_request({method:'GET', url:'https://other.test/open', headers:[], body:null}).promise\
              .then(function(r){ console.log('hdr', r.headers.get('x-hidden'), r.headers.get('content-type')); });"
        ),
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"net:GET https://other.test/closed: CORS: no Access-Control-Allow-Origin header (blocked by CORS policy)".to_string()), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(l.contains(&"log:ok 200 cors pub".to_string()), "{l:?}");
    assert!(l.contains(&"log:hdr null text/plain".to_string()), "non-safelisted response headers are hidden from a cors response: {l:?}");
    assert_eq!(fake.urls().len(), 3);
}

#[test]
fn qf7_no_cors_is_opaque_and_beacon_is_logged_not_sent() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "__net_request({method:'GET', url:'https://track.test/p?x=1', headers:[], body:null, mode:'no-cors'}).promise\
           .then(function(r){ console.log('nc', r.status, r.type, JSON.stringify(r.url)); });\
         __net_request({method:'POST', url:'https://track.test/b', headers:[['Content-Type','text/plain']], body:new Uint8Array([104,105]), api:'beacon'}).promise\
           .then(function(r){ console.log('bc', r.type); });",
    );
    qf7_drive(&engine, &fake);
    assert!(fake.urls().is_empty(), "beacons are suppressed, not sent");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:nc 0 opaque \"\"".to_string()), "{l:?}");
    assert!(l.contains(&"log:bc opaque".to_string()), "{l:?}");
    assert!(
        l.contains(&"net:beacon navigator.sendBeacon POST https://track.test/b content-type=text/plain body=2 bytes body[..200]=\"hi\" page=https://page.test/: suppressed (not sent)".to_string()),
        "{l:?}"
    );
    assert!(l.iter().any(|x| x.contains("beacon fetch no-cors GET https://track.test/p?x=1") && x.ends_with("suppressed (not sent)")), "{l:?}");
}

#[test]
fn qf7_beacons_send_flag_sends_and_stays_opaque() {
    let fake = QfFake::new(vec![("track.test", qf7_ok("gif", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.beacons_send = true;
    engine.set_net_policy(p);
    engine.eval("t", &qf7_go("https://track.test/p", ", mode:'no-cors'"));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 1);
    assert!(log.borrow().iter().any(|x| x == "log:ok 0 opaque "), "{:?}", log.borrow());
}

#[test]
fn qf7_preflight_is_rejected_with_a_clear_log() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "__net_request({method:'PUT', url:'https://other.test/x', headers:[], body:null}).promise.catch(function(e){ console.log('err', e.name); });",
    );
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls(), vec!["https://other.test/x"], "QF13: only the preflight (which got no canned answer) was sent");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://other.test/x:")), "{l:?}");
    assert!(l.contains(&"log:err TypeError".to_string()), "{l:?}");
}

#[test]
fn qf7_abort_cancels_and_a_late_result_is_ignored() {
    let (engine, log) = qf7_engine("https://page.test/", &QfFake::new(vec![]));
    engine.eval(
        "t",
        "var c = new AbortController(); c.abort();\
         __net_request({method:'GET', url:'https://page.test/a', headers:[], body:null, signal:c.signal}).promise\
           .then(function(){ console.log('A resolved'); }, function(e){ console.log('A', e.name); });\
         var c2 = new AbortController();\
         __net_request({method:'GET', url:'https://page.test/b', headers:[], body:null, signal:c2.signal}).promise\
           .then(function(){ console.log('B resolved'); }, function(e){ console.log('B', e.name); });",
    );
    let reqs = engine.take_requests();
    assert_eq!(reqs.net.len(), 1, "the pre-aborted request is not even queued");
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let job = reqs.net.into_iter().next().unwrap();
    let id = job.id;
    sched.enqueue(job);
    let mut started = Vec::new();
    sched.pump(&mut |j| started.push(j));
    assert_eq!(started.len(), 1);
    // Abort while it is in flight, then deliver the answer anyway.
    engine.eval("t", "c2.abort();");
    assert_eq!(engine.take_requests().net_abort, vec![id]);
    engine.deliver_net(id, qf7_ok("B", &[]));
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:A AbortError", "log:B AbortError"], "{l:?}");
    // A job still queued is cancelled by its abort id.
    let mut s2 = NetScheduler::new(&RequestPolicy::default());
    s2.enqueue(started.into_iter().next().unwrap());
    assert!(s2.abort(id));
    assert_eq!(s2.counts(), (0, 0));
}

fn qf7_job(id: u64, reserve: usize) -> crate::js_net::NetJob {
    crate::js_net::NetJob { id, request: Vec::new(), limits: WireLimits::default(), reserve }
}

#[test]
fn qf7_scheduler_caps_parallelism_and_in_flight_bytes() {
    let mut p = RequestPolicy::default();
    let mut s = NetScheduler::new(&p);
    for i in 1..=10 {
        s.enqueue(qf7_job(i, 0));
    }
    let mut started = Vec::new();
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1, 2, 3, 4, 5, 6], "max_concurrent_requests (6) from the policy");
    s.done(1);
    s.done(999); // unknown ids are a no-op
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started.len(), 7);
    assert_eq!(s.counts(), (3, 6));
    // Byte cap, FIFO: a 60-byte request waits for the other 60-byte one.
    p.max_in_flight_bytes = 100;
    let mut s = NetScheduler::new(&p);
    s.enqueue(qf7_job(1, 60));
    s.enqueue(qf7_job(2, 60));
    s.enqueue(qf7_job(3, 10));
    let mut started = Vec::new();
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1]);
    s.done(1);
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1, 2, 3]);
    s.reset();
    assert_eq!(s.counts(), (0, 0));
}

#[test]
fn qf7_per_page_and_body_caps_come_from_the_policy() {
    let fake = QfFake::new(vec![("page.test", qf7_ok("x", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.max_requests_per_page = 2;
    p.max_in_flight_bytes = 8;
    engine.set_net_policy(p);
    let post = |n: usize| {
        format!("__net_request({{method:'POST', url:'https://page.test/p', headers:[], body:new Uint8Array({n})}});")
    };
    engine.eval("t", &format!("{}{}{}{}", post(9), post(1), post(1), post(1)));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 2, "the oversized body and the third request are refused");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("request body is too large (9 bytes, limit 8)")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("too many requests from this page (limit 2)")), "{l:?}");
}

#[test]
fn qf7_sync_blocks_on_the_engine_thread_with_the_short_timeout() {
    let fake = Arc::new(QfFake {
        calls: Mutex::new(Vec::new()),
        answers: vec![("/ok", qf7_ok("sync-body", &[])), ("/slow", Err(NetError::Timeout))],
        delay_ms: 60,
        raw: Mutex::new(Vec::new()),
    });
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let t = std::time::Instant::now();
    engine.eval(
        "t",
        "var r = __net_request({method:'GET', url:'https://page.test/ok', headers:[], body:null, sync:true}).response;\
         console.log('got', r.status, r.type);\
         try { __net_request({method:'GET', url:'https://page.test/slow', headers:[], body:null, sync:true}); }\
         catch (e) { console.log('threw', e.name); }",
    );
    assert!(t.elapsed() >= std::time::Duration::from_millis(120), "send() blocked for both requests");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:got 200 basic".to_string()), "{l:?}");
    assert!(l.contains(&"log:threw NetworkError".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:sync GET https://page.test/ok: blocking the page for up to 5000 ms")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:GET https://page.test/slow: request timed out")), "{l:?}");
    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.1 == thread::current().id()), "sync runs on the calling thread");
    assert!(calls.iter().all(|c| c.2 == std::time::Duration::from_millis(5000)), "sync_xhr_timeout_ms, not the 30 s default");
}

#[test]
fn qf7_js_report_has_a_network_section_and_uses_the_base_href() {
    let fake = QfFake::new(vec![("page.test/data", qf7_ok("12345", &[])), ("other.test", qf7_ok("no-acao", &[]))]);
    crate::js_net::set_net_executor(fake.clone());
    let html = "<html><head><base href='https://page.test/dir/'></head><body><script>\
        var q = function(u){ __net_request({method:'GET', url:u, headers:[], body:null}).promise.catch(function(){}); };\
        q('https://page.test/data'); q('https://other.test/x'); q('http://127.0.0.1:8080/');\
        </script></body></html>";
    set_js_enabled(true);
    let report = js::js_report_for_html(html, None, "/tmp/qf7-none.html");
    assert!(report.contains("--- network (3 requests, 12 bytes) ---"), "{report}");
    assert!(report.contains("GET https://page.test/data -> 200 OK 5 bytes"), "same-origin check uses the <base href>, not file://: {report}");
    assert!(report.contains("GET https://other.test/x -> blocked: CORS: no Access-Control-Allow-Origin header"), "{report}");
    assert!(report.contains("(loopback) allowed by policy"), "{report}");
    assert!(report.contains("errors total: 0"), "{report}");
}

/// App side: queued requests are started by `js_sync`, results of a
/// superseded page are dropped, current ones reach the page.
fn qf7_app_net(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[]))]);
    let doc = parse_html("<html><body></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    crate::js_net::set_net_executor(fake.clone());
    let page = Url::parse("https://page.test/").unwrap();
    let code = format!("{} h.promise.then(function(r){{ return r.text(); }}).then(function(t){{ window.__done = t; }});", qf7_go("https://page.test/a", ""));
    app.js = run_page_scripts(&doc, &[src(&code)], Some(&page));
    app.js_sync();
    for _ in 0..200 {
        if !fake.urls().is_empty() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(fake.urls(), vec!["https://page.test/a"], "js_sync started the request on a thread");
    let done = |app: &App| app.js.as_ref().unwrap().eval_value::<String>("t", "String(window.__done)");
    let id = app.js.as_ref().unwrap().eval_value::<f64>("t", "h.id").unwrap() as u64;
    // A stale generation is dropped: the promise stays pending.
    app.handle_js_net(app.nav_generation + 1, id, qf7_ok("STALE", &[]));
    assert_eq!(done(&app).as_deref(), Some("undefined"));
    app.handle_js_net(app.nav_generation, id, qf7_ok("AAA", &[]));
    assert_eq!(done(&app).as_deref(), Some("AAA"));
}

// ---------- QF8: window.fetch over the QF7 queue (fake executor, no network) ----------

fn qf8_run(page: &str, fake: &Arc<QfFake>, code: &str) -> Vec<String> {
    let (engine, log) = qf7_engine(page, fake);
    engine.eval("t", code);
    qf7_drive(&engine, fake);
    let l = log.borrow().clone();
    l
}

#[test]
fn qf8_fetch_exists_on_window_and_global_this() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "console.log(typeof fetch, typeof window.fetch, typeof globalThis.fetch, typeof self.fetch, fetch === window.fetch, fetch.length, fetch.name);",
    );
    assert_eq!(l, vec!["log:function function function function true 1 fetch"]);
}

#[test]
fn qf8_get_text_and_json_round_trip() {
    let fake = QfFake::new(vec![
        ("/t", qf7_ok("hello", &[("content-type", "text/plain")])),
        ("/j", qf7_ok("{\"a\":[1,2]}", &[("content-type", "application/json")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/t').then(function(r){ console.log('r', r.status, r.ok, r.type, r.url, r.redirected, r.headers.get('content-type')); return r.text(); }).then(function(t){ console.log('text', t); });\
         fetch('https://page.test/j').then(function(r){ return r.json(); }).then(function(j){ console.log('json', JSON.stringify(j)); });",
    );
    assert!(l.contains(&"log:r 200 true basic https://page.test/t false text/plain".to_string()), "{l:?}");
    assert!(l.contains(&"log:text hello".to_string()), "{l:?}");
    assert!(l.contains(&"log:json {\"a\":[1,2]}".to_string()), "{l:?}");
}

#[test]
fn qf8_post_json_body_and_headers_reach_the_executor() {
    let fake = QfFake::new(vec![("/p", qf7_ok("{}", &[]))]);
    let _ = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/p', {method:'post', headers:{'Content-Type':'application/json','X-Tok':'abc'}, body: JSON.stringify({k:1})});",
    );
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 1);
    let t = String::from_utf8_lossy(&raw[0]).to_string();
    assert!(t.starts_with("POST https://page.test/p"), "{t}");
    assert!(t.to_ascii_lowercase().contains("content-type: application/json"), "{t}");
    assert!(t.to_ascii_lowercase().contains("x-tok: abc"), "{t}");
    assert!(t.ends_with("{\"k\":1}"), "{t}");
}

#[test]
fn qf8_form_data_body_is_multipart() {
    let fake = QfFake::new(vec![("/u", qf7_ok("", &[]))]);
    let _ = qf8_run(
        "https://page.test/",
        &fake,
        "var fd = new FormData(); fd.append('a', 'one'); fd.append('f', new Blob(['xyz'], {type:'text/plain'}), 'n.txt'); fetch('/u', {method:'POST', body: fd});",
    );
    let raw = fake.raw.lock().unwrap().clone();
    let t = String::from_utf8_lossy(&raw[0]).to_string();
    assert!(t.to_ascii_lowercase().contains("content-type: multipart/form-data; boundary="), "{t}");
    assert!(t.contains("name=\"a\"") && t.contains("one") && t.contains("filename=\"n.txt\"") && t.contains("xyz"), "{t}");
}

#[test]
fn qf8_abort_before_and_after() {
    let fake = QfFake::new(vec![("/a", qf7_ok("x", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "var c = new AbortController(); c.abort();\
         fetch('/a', {signal: c.signal}).catch(function(e){ console.log('pre', e.name); });\
         var c2 = new AbortController(); c2.abort('why');\
         fetch('/a', {signal: c2.signal}).catch(function(e){ console.log('reason', e); });\
         var c3 = new AbortController();\
         fetch('/a', {signal: c3.signal}).then(function(){ console.log('B resolved'); }, function(e){ console.log('late', e.name); });\
         Promise.resolve().then(function(){ return Promise.resolve(); }).then(function(){ c3.abort(); });",
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:pre AbortError".to_string()), "{l:?}");
    assert!(l.contains(&"log:reason why".to_string()), "{l:?}");
    assert!(l.contains(&"log:late AbortError".to_string()), "{l:?}");
    assert!(!l.contains(&"log:B resolved".to_string()), "{l:?}");
}

#[test]
fn qf8_cors_blocked_allowed_and_opaque() {
    let fake = QfFake::new(vec![
        ("/closed", qf7_ok("secret", &[])),
        ("/open", qf7_ok("pub", &[("access-control-allow-origin", "*"), ("x-hidden", "1")])),
        ("/cred", qf7_ok("c", &[("access-control-allow-origin", "*")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://o.test/closed').catch(function(e){ console.log('closed', e.name, e.message); });\
         fetch('https://o.test/open').then(function(r){ console.log('open', r.type, r.headers.get('x-hidden')); return r.text(); }).then(function(t){ console.log('body', t); });\
         fetch('https://o.test/cred', {credentials:'include'}).catch(function(e){ console.log('cred', e.name); });\
         fetch('https://o.test/x', {mode:'no-cors'}).then(function(r){ console.log('nc', r.type, r.status, JSON.stringify(r.url)); });",
    );
    assert!(l.contains(&"log:closed TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(l.contains(&"log:open cors null".to_string()), "{l:?}");
    assert!(l.contains(&"log:body pub".to_string()), "{l:?}");
    assert!(l.contains(&"log:cred TypeError".to_string()), "credentials include rejects ACAO *: {l:?}");
    assert!(l.contains(&"log:nc opaque 0 \"\"".to_string()), "{l:?}");
}

#[test]
fn qf8_same_origin_mode_and_unsupported_schemes_reject() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://o.test/x', {mode:'same-origin'}).catch(function(e){ console.log('so', e.name); });\
         ['ftp://h/x', 'about:blank', 'javascript:1'].forEach(function(u){ fetch(u).catch(function(e){ console.log('bad', e.name, u); }); });\
         fetch().catch(function(e){ console.log('noargs', e.name); });",
    );
    assert!(fake.urls().is_empty(), "{:?} {l:?}", fake.urls());
    assert!(l.contains(&"log:so TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError ftp://h/x".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError about:blank".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError javascript:1".to_string()), "{l:?}");
    assert!(l.contains(&"log:noargs TypeError".to_string()), "{l:?}");
}

#[test]
fn qf8_redirect_error_and_follow_report_final_url() {
    let redirected = Ok(WireResponse {
        status: 200,
        status_text: "OK".into(),
        headers: vec![],
        body: b"final".to_vec(),
        error: None,
        final_url: Some("https://page.test/final".into()),
    });
    let fake = QfFake::new(vec![("/start", redirected)]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/start').then(function(r){ console.log('follow', r.url, r.redirected); });\
         fetch('/start', {redirect:'error'}).catch(function(e){ console.log('error', e.name); });",
    );
    assert!(l.contains(&"log:follow https://page.test/final true".to_string()), "{l:?}");
    assert!(l.contains(&"log:error TypeError".to_string()), "{l:?}");
}

#[test]
fn qf8_blob_and_data_urls_resolve_locally() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "var u = URL.createObjectURL(new Blob(['blobby'], {type:'text/x-t'}));\
         fetch(u).then(function(r){ console.log('blob', r.status, r.type, r.headers.get('content-type'), r.url === u); return r.text(); }).then(function(t){ console.log('bt', t); });\
         fetch('data:text/plain;base64,aGk=').then(function(r){ return r.text(); }).then(function(t){ console.log('b64', t); });\
         fetch('data:,a%20b%C3%A9').then(function(r){ console.log('ct', r.headers.get('content-type')); return r.text(); }).then(function(t){ console.log('pct', t); });\
         var u3 = URL.createObjectURL(new Blob(['q'])); URL.revokeObjectURL(u3); fetch(u3).catch(function(e){ console.log('revoked', e.name); });\
         var u2 = URL.createObjectURL(new Blob(['z'])); fetch(u2).then(function(r){ return r.arrayBuffer(); }).then(function(b){ console.log('ab', b.byteLength); });",
    );
    assert!(fake.urls().is_empty(), "local fetches never reach the network");
    assert!(l.contains(&"log:blob 200 basic text/x-t true".to_string()), "{l:?}");
    assert!(l.contains(&"log:bt blobby".to_string()), "{l:?}");
    assert!(l.contains(&"log:b64 hi".to_string()), "{l:?}");
    assert!(l.contains(&"log:ct text/plain;charset=US-ASCII".to_string()), "{l:?}");
    assert!(l.contains(&"log:pct a b\u{e9}".to_string()), "{l:?}");
    assert!(l.contains(&"log:revoked TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:ab 1".to_string()), "{l:?}");
}

#[test]
fn qf8_keepalive_beacon_is_logged_not_sent() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://track.test/b', {method:'POST', keepalive:true, body:'x=1', headers:{'Content-Type':'text/plain'}}).then(function(r){ console.log('ka', r.type, r.status); });",
    );
    assert!(fake.urls().is_empty());
    assert!(l.contains(&"log:ka opaque 0".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:") && x.contains("suppressed (not sent)")), "{l:?}");
}

#[test]
fn qf8_network_failure_rejects_failed_to_fetch() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run("https://page.test/", &fake, "fetch('/nothing').catch(function(e){ console.log('net', e.name, e.message); });");
    assert!(l.contains(&"log:net TypeError Failed to fetch".to_string()), "{l:?}");
}

// ---------- QF9: XMLHttpRequest over the QF7 queue (fake executor, no network) ----------

const QF9_TRACK: &str = "function track(x, tag) { tag = tag || ''; x.onreadystatechange = function () { console.log(tag + 'rs' + x.readyState); };\
  ['loadstart','progress','load','error','abort','timeout','loadend'].forEach(function (t) { x.addEventListener(t, function (e) { console.log(tag + t, e.loaded, e.total, e.lengthComputable); }); }); }\n";

fn qf9_run(page: &str, fake: &Arc<QfFake>, code: &str) -> Vec<String> {
    qf8_run(page, fake, &format!("{QF9_TRACK}{code}"))
}

#[test]
fn qf9_get_event_order_and_response_fields() {
    let fake = QfFake::new(vec![("/a", qf7_ok("hello", &[("content-type", "text/plain"), ("content-length", "5")]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); console.log(typeof XMLHttpRequest, XMLHttpRequest.DONE, x.UNSENT, x.readyState, x instanceof EventTarget);\
         track(x); x.open('get', '/a'); x.send();\
         x.addEventListener('loadend', function () { console.log('final', x.status, x.statusText, x.responseURL, x.responseText, x.response, x.getResponseHeader('Content-Type')); });",
    );
    let want = [
        "log:function 4 0 0 true",
        "log:rs1",
        "log:loadstart 0 0 false",
        "log:rs2",
        "log:rs3",
        "log:progress 5 5 true",
        "log:rs4",
        "log:load 5 5 true",
        "log:loadend 5 5 true",
        "log:final 200 OK https://page.test/a hello hello text/plain",
    ];
    assert_eq!(l, want, "{l:?}");
}

#[test]
fn qf9_post_body_and_headers_reach_the_executor_and_forbidden_is_ignored() {
    let fake = QfFake::new(vec![("/p", qf7_ok("{}", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('POST', '/p');\
         x.setRequestHeader('X-Tok', 'a'); x.setRequestHeader('x-tok', 'b'); x.setRequestHeader('Cookie', 'evil=1'); x.setRequestHeader('Host', 'evil');\
         try { x.send('payload'); x.setRequestHeader('X-Late', '1'); } catch (e) { console.log('late', e.name); }\
         try { new XMLHttpRequest().setRequestHeader('A', 'b'); } catch (e) { console.log('unopened', e.name); }",
    );
    assert!(l.contains(&"log:late InvalidStateError".to_string()), "{l:?}");
    assert!(l.contains(&"log:unopened InvalidStateError".to_string()), "{l:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    let lower = raw.to_lowercase();
    assert!(raw.starts_with("POST https://page.test/p"), "{raw}");
    assert!(lower.contains("x-tok: a, b"), "{raw}");
    assert!(lower.contains("content-type: text/plain;charset=utf-8"), "{raw}");
    assert!(raw.ends_with("payload"), "{raw}");
    assert!(!lower.contains("evil"), "{raw}");
}

#[test]
fn qf9_get_ignores_the_body_and_open_validates_the_method() {
    let fake = QfFake::new(vec![("/g", qf7_ok("x", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/g'); x.send('ignored');\
         ['TRACE','connect','Track'].forEach(function (m) { try { new XMLHttpRequest().open(m, '/'); } catch (e) { console.log(m, e.name); } });\
         try { new XMLHttpRequest().open('bad method', '/'); } catch (e) { console.log('bad', e.name); }",
    );
    assert!(l.contains(&"log:TRACE SecurityError".to_string()) && l.contains(&"log:connect SecurityError".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad SyntaxError".to_string()), "{l:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    assert!(raw.starts_with("GET ") && !raw.contains("ignored"), "{raw}");
}

#[test]
fn qf9_response_types() {
    let fake = QfFake::new(vec![("/j", qf7_ok("{\"a\":[1,2]}", &[("content-type", "application/json")]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "function go(t, f) { var x = new XMLHttpRequest(); x.open('GET', '/j'); x.responseType = t;\
           x.onload = function () { f(x); }; x.send(); }\
         go('json', function (x) { console.log('json', JSON.stringify(x.response), x.responseType); });\
         go('arraybuffer', function (x) { console.log('ab', x.response instanceof ArrayBuffer, x.response.byteLength); });\
         go('blob', function (x) { console.log('blob', x.response instanceof Blob, x.response.size, x.response.type); });\
         go('text', function (x) { console.log('text', x.response, x.responseText.length); });\
         go('json', function (x) { try { x.responseText; } catch (e) { console.log('rt', e.name); } });",
    );
    assert!(l.contains(&"log:json {\"a\":[1,2]} json".to_string()), "{l:?}");
    assert!(l.contains(&"log:ab true 11".to_string()), "{l:?}");
    assert!(l.contains(&"log:blob true 11 application/json".to_string()), "{l:?}");
    assert!(l.contains(&"log:text {\"a\":[1,2]} 11".to_string()), "{l:?}");
    assert!(l.contains(&"log:rt InvalidStateError".to_string()), "{l:?}");
}

#[test]
fn qf9_get_all_response_headers_format() {
    let fake = QfFake::new(vec![(
        "/h",
        qf7_ok("", &[("X-B", "2"), ("Content-Type", "text/plain"), ("Set-Cookie", "s=1"), ("x-a", "1"), ("X-B", "3")]),
    )]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); console.log(JSON.stringify(x.getAllResponseHeaders()), x.getResponseHeader('x-a'));\
         x.open('GET', '/h'); x.onload = function () { console.log(JSON.stringify(x.getAllResponseHeaders()), x.getResponseHeader('X-B'), x.getResponseHeader('set-cookie'), x.getResponseHeader('nope')); }; x.send();",
    );
    assert_eq!(l[0], "log:\"\" null");
    assert_eq!(l[1], "log:\"content-type: text/plain\\r\\nx-a: 1\\r\\nx-b: 2, 3\\r\\n\" 2, 3 null null", "{l:?}");
}

#[test]
fn qf9_abort_fires_abort_and_loadend_then_unsent() {
    let fake = QfFake::new(vec![("/a", qf7_ok("late", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); track(x); x.open('GET', '/a'); x.send(); x.abort(); console.log('after', x.readyState, x.status);\
         var y = new XMLHttpRequest(); y.abort(); console.log('idle', y.readyState);",
    );
    let want = ["log:rs1", "log:loadstart 0 0 false", "log:rs4", "log:abort 0 0 false", "log:loadend 0 0 false", "log:after 0 0", "log:idle 0"];
    assert_eq!(l, want, "{l:?}");
}

#[test]
fn qf9_timeout_fires_on_the_manual_clock() {
    let fake = QfFake::new(vec![]);
    set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(&format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', '/slow'); x.timeout = 250; track(x); x.send();\
             console.log('t', x.timeout);"
        ))],
        Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))),
        Some(clock.clone()),
    )
    .unwrap();
    // The request is queued but never delivered: only the timer can end it.
    let due = engine.next_due_ms().expect("a timeout timer");
    assert!((249.0..=251.0).contains(&due), "{due}");
    clock.set(due);
    engine.run_due();
    let l = out.borrow().clone();
    assert_eq!(l, ["log:loadstart 0 0 false", "log:t 250", "log:rs4", "log:timeout 0 0 false", "log:loadend 0 0 false"], "{l:?}");
}

#[test]
fn qf9_cors_failure_is_an_error_event_with_status_0() {
    let fake = QfFake::new(vec![("/closed", qf7_ok("secret", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', 'https://other.test/closed'); track(x); x.send();\
             x.addEventListener('loadend', function () {{ console.log('end', x.status, JSON.stringify(x.responseText)); }});\
             var p = new XMLHttpRequest(); p.open('PUT', 'https://other.test/pf'); p.onerror = function () {{ console.log('pf error'); }}; p.send('x');"
        ),
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:error 0 0 false".to_string()), "{l:?}");
    assert!(l.contains(&"log:end 0 \"\"".to_string()), "{l:?}");
    assert!(l.contains(&"log:pf error".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("GET https://other.test/closed: CORS: no Access-Control-Allow-Origin")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("CORS preflight failed: https://other.test/pf")), "{l:?}");
}

#[test]
fn qf9_404_still_fires_load() {
    let fake = QfFake::new(vec![(
        "/nf",
        Ok(WireResponse { status: 404, status_text: "Not Found".into(), headers: vec![], body: b"nope".to_vec(), error: None, final_url: None }),
    )]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/nf'); x.onload = function () { console.log('load', x.status, x.statusText, x.responseText); };\
         x.onerror = function () { console.log('error'); }; x.send();",
    );
    assert_eq!(l, ["log:load 404 Not Found nope"], "{l:?}");
}

#[test]
fn qf9_sync_returns_synchronously_and_logs() {
    let fake = QfFake::new(vec![("/s", qf7_ok("sync-body", &[("content-type", "text/plain")]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', '/s', false); track(x); x.send(); console.log('after-send', x.readyState, x.status, x.responseText);\
             var r = new XMLHttpRequest(); r.open('GET', '/s', false);\
             try {{ r.responseType = 'json'; }} catch (e) {{ console.log('rt', e.name); }}\
             try {{ r.timeout = 5; }} catch (e) {{ console.log('to', e.name); }}\
             var q = new XMLHttpRequest(); q.responseType = 'blob';\
             try {{ q.open('GET', '/s', false); }} catch (e) {{ console.log('open', e.name); }}"
        ),
    );
    let l = log.borrow().clone();
    assert!(l.contains(&"log:after-send 4 200 sync-body".to_string()), "{l:?}");
    let pos = |s: &str| l.iter().position(|x| x == s).unwrap_or_else(|| panic!("{s} in {l:?}"));
    assert!(pos("log:rs4") < pos("log:load 9 9 true") && pos("log:load 9 9 true") < pos("log:after-send 4 200 sync-body"), "{l:?}");
    assert!(l.iter().any(|x| x.contains("sync GET https://page.test/s: blocking the page")), "{l:?}");
    assert!(l.contains(&"log:rt InvalidAccessError".to_string()) && l.contains(&"log:to InvalidAccessError".to_string()), "{l:?}");
    assert!(l.contains(&"log:open InvalidAccessError".to_string()), "{l:?}");
    assert_eq!(fake.urls().len(), 1);
}

#[test]
fn qf9_sync_failure_throws_network_error() {
    let fake = QfFake::new(vec![]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/nothing', false);\
         try { x.send(); console.log('no throw'); } catch (e) { console.log('thrown', e.name, e instanceof DOMException, x.readyState, x.status); }",
    );
    assert!(l.contains(&"log:thrown NetworkError true 4 0".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("sync GET https://page.test/nothing")), "{l:?}");
}

#[test]
fn qf9_form_data_urlsearchparams_and_arraybuffer_bodies() {
    let fake = QfFake::new(vec![("/b", qf7_ok("", &[]))]);
    qf9_run(
        "https://page.test/",
        &fake,
        "var fd = new FormData(); fd.append('k', 'v');\
         [fd, new URLSearchParams('a=1'), new Uint8Array([65, 66]).buffer].forEach(function (b) { var x = new XMLHttpRequest(); x.open('POST', '/b'); x.send(b); });",
    );
    let raws: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_lowercase()).collect();
    assert!(raws[0].contains("content-type: multipart/form-data; boundary=") && raws[0].contains("name=\"k\""), "{}", raws[0]);
    assert!(raws[1].contains("application/x-www-form-urlencoded") && raws[1].ends_with("a=1"), "{}", raws[1]);
    assert!(raws[2].ends_with("ab") && !raws[2].contains("content-type"), "{}", raws[2]);
}

// ---- QY6a: MutationObserver ----

/// Shared JS helpers: `d(record)` summarises a record as
/// `type|target|added|removed|prev|next|attr|old`; `watch(node, opts)` returns
/// the observer and pushes one string per delivered callback (records joined
/// by ` ; `) onto `out`; `step(f, g, ...)` runs each function in its own
/// microtask turn, after the previous delivery.
const QY6A_JS: &str = r#"
const n = x => x ? (x.id || x.nodeName.toLowerCase()) : '-';
const ns = l => Array.from(l).map(n).join(',') || '-';
const d = r => [r.type, n(r.target), ns(r.addedNodes), ns(r.removedNodes), n(r.previousSibling), n(r.nextSibling), r.attributeName, r.oldValue].map(String).join('|');
const out = [];
function watch(t, o) { const mo = new MutationObserver(l => out.push(l.map(d).join(' ; '))); mo.observe(t, o); return mo; }
function step() { const fs = Array.from(arguments); const run = () => { const f = fs.shift(); if (f) { f(); Promise.resolve().then(run); } else Promise.resolve().then(() => globalThis.__fin && __fin()); }; Promise.resolve().then(run); }
const mk = (t, id) => { const e = document.createElement(t); if (id) e.id = id; return e; };
const $ = id => document.getElementById(id);
"#;

fn qy6a_run(body: &str) -> Vec<String> {
    run_in_html(
        "<html><body><div id=h><i id=a></i><b id=b></b></div><span id=s>text</span><!--c--></body></html>",
        &format!("{QY6A_JS}\n{body}"),
    )
}

/// Runs `body` and returns the delivered callbacks (one entry each).
fn qy6a_out(body: &str) -> Vec<String> {
    let mut log = qy6a_run(&format!("globalThis.__fin = () => console.log(out.join('\\n'));\n{body}"));
    assert_eq!(log.len(), 1, "{log:?}");
    let l = log.remove(0);
    let l = l.strip_prefix("log:").unwrap_or(&l).to_string();
    if l.is_empty() {
        Vec::new()
    } else {
        l.split('\n').map(String::from).collect()
    }
}

#[test]
fn qy6a_zero_cost_before_observe() {
    let log = qy6a_run(
        r#"const h = $('h');
         for (let i = 0; i < 20; i++) { const e = mk('p'); e.setAttribute('a', i); h.appendChild(e); }
         h.innerHTML = '<p></p>'; h.textContent = 'x'; h.removeChild(h.firstChild);
         console.log(__ce_stats.hooks);
         const mo = new MutationObserver(() => {});
         console.log(__ce_stats.hooks);
         mo.observe(h, { childList: true });
         h.appendChild(mk('u'));
         console.log(__ce_stats.hooks > 0);"#,
    );
    assert_eq!(log, ["log:0", "log:0", "log:true"], "{log:?}");
}

#[test]
fn qy6a_child_list_write_paths() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => $('h').appendChild(mk('p', 'p1')),
              () => $('h').insertBefore(mk('p', 'p2'), $('a')),
              () => $('h').removeChild($('b')),
              () => $('h').replaceChild(mk('p', 'p3'), $('a')),
              () => $('p1').remove());"#,
    );
    assert_eq!(
        o,
        [
            "childList|h|p1|-|b|-|null|null",
            "childList|h|p2|-|-|a|null|null",
            "childList|h|-|b|a|p1|null|null",
            "childList|h|p3|a|p2|p1|null|null",
            "childList|h|-|p1|p3|-|null|null",
        ]
    );
}

#[test]
fn qy6a_multi_node_apis_make_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => $('h').append(mk('p', 'p1'), 'txt', mk('p', 'p2')),
              () => $('h').prepend(mk('q', 'q1'), mk('q', 'q2')),
              () => $('a').before(mk('r', 'r1'), mk('r', 'r2')),
              () => $('b').after(mk('t', 't1'), mk('t', 't2')),
              () => $('a').replaceWith(mk('u', 'u1'), mk('u', 'u2')),
              () => $('h').replaceChildren(mk('v', 'v1')),
              () => { const f = document.createDocumentFragment(); f.appendChild(mk('w', 'w1')); f.appendChild(mk('w', 'w2')); $('h').appendChild(f); });"#,
    );
    assert_eq!(
        o,
        [
            "childList|h|p1,#text,p2|-|b|-|null|null",
            "childList|h|q1,q2|-|-|a|null|null",
            "childList|h|r1,r2|-|q2|a|null|null",
            "childList|h|t1,t2|-|b|p1|null|null",
            "childList|h|u1,u2|a|r2|b|null|null",
            "childList|h|v1|q1,q2,r1,r2,u1,u2,b,t1,t2,p1,#text,p2|-|-|null|null",
            "childList|h|w1,w2|-|v1|-|null|null",
        ]
    );
}

#[test]
fn qy6a_inner_html_and_text_content_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => { $('h').innerHTML = '<p id=p1></p><p id=p2></p>'; },
              () => { $('h').textContent = 'plain'; },
              () => { $('h').textContent = ''; },
              () => { $('h').innerHTML = ''; });"#,
    );
    // `#text` is the one text node textContent created; an empty -> empty write queues nothing.
    assert_eq!(
        o,
        [
            "childList|h|p1,p2|a,b|-|-|null|null",
            "childList|h|#text|p1,p2|-|-|null|null",
            "childList|h|-|#text|-|-|null|null",
        ]
    );
}

#[test]
fn qy6a_outer_html_replaces_with_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => { $('a').outerHTML = '<p id=x1></p><p id=x2></p>'; });"#,
    );
    assert_eq!(o, ["childList|h|x1,x2|a|-|b|null|null"]);
}

#[test]
fn qy6a_move_records_removal_and_insertion() {
    let o = qy6a_out(
        r#"const s = $('s'); watch($('h'), { childList: true }); watch(s, { childList: true });
         step(() => s.appendChild($('a')));"#,
    );
    assert_eq!(o, ["childList|h|-|a|-|b|null|null", "childList|s|a|-|#text|-|null|null"]);
}

#[test]
fn qy6a_attribute_records_and_old_value() {
    let o = qy6a_out(
        r#"const a = $('a'); watch(a, { attributes: true, attributeOldValue: true });
         step(() => a.setAttribute('x', '1'),
              () => a.setAttribute('x', '2'),
              () => a.removeAttribute('x'),
              () => a.removeAttribute('nope'),
              () => a.toggleAttribute('hidden'),
              () => { a.className = 'k'; },
              () => { a.classList.add('m'); },
              () => { a.id = 'a2'; },
              () => { a.dataset.foo = 'bar'; },
              () => { a.style.color = 'red'; });"#,
    );
    assert_eq!(
        o,
        [
            "attributes|a|-|-|-|-|x|null",
            "attributes|a|-|-|-|-|x|1",
            "attributes|a|-|-|-|-|x|2",
            "attributes|a|-|-|-|-|hidden|null",
            "attributes|a|-|-|-|-|class|null",
            "attributes|a|-|-|-|-|class|k",
            "attributes|a2|-|-|-|-|id|a",
            "attributes|a2|-|-|-|-|data-foo|null",
            "attributes|a2|-|-|-|-|style|null",
        ]
    );
}

#[test]
fn qy6a_old_value_only_when_requested() {
    let o = qy6a_out(
        r#"const a = $('a'); a.setAttribute('x', '1'); watch(a, { attributes: true });
         step(() => a.setAttribute('x', '2'));"#,
    );
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null"]);
}

#[test]
fn qy6a_attribute_filter() {
    let o = qy6a_out(
        r#"const a = $('a'); watch(a, { attributeFilter: ['x', 'y'], attributeOldValue: true });
         step(() => { a.setAttribute('z', '1'); a.setAttribute('x', '1'); a.setAttribute('w', '1'); a.setAttribute('y', '1'); });"#,
    );
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null ; attributes|a|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_character_data_records() {
    let o = qy6a_out(
        r#"const t = $('s').firstChild; watch(t, { characterData: true, characterDataOldValue: true });
         step(() => { t.data = 'one'; },
              () => { t.nodeValue = 'two'; },
              () => { t.textContent = 'three'; },
              () => t.appendData('+'),
              () => t.insertData(0, '>'),
              () => t.deleteData(0, 1),
              () => t.replaceData(0, 2, 'ZZ'));"#,
    );
    assert_eq!(
        o,
        [
            "characterData|#text|-|-|-|-|null|text",
            "characterData|#text|-|-|-|-|null|one",
            "characterData|#text|-|-|-|-|null|two",
            "characterData|#text|-|-|-|-|null|three",
            "characterData|#text|-|-|-|-|null|three+",
            "characterData|#text|-|-|-|-|null|>three+",
            "characterData|#text|-|-|-|-|null|three+",
        ]
    );
}

#[test]
fn qy6a_character_data_methods_edit_the_text() {
    let log = qy6a_run(
        r#"const t = document.createTextNode('hello');
         t.appendData(' world'); t.insertData(0, '>'); t.deleteData(1, 2); t.replaceData(0, 1, '<<');
         console.log(t.data, t.substringData(2, 3));
         try { t.insertData(99, 'x'); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(log, ["log:<<llo world llo", "log:IndexSizeError"], "{log:?}");
}

#[test]
fn qy6a_document_write_records_insertions() {
    let out = qa13_run(
        "<html><body><p id=a>x<script id=sc>\
         var mo = new MutationObserver(function (l) { l.forEach(function (r) { console.log(r.type, r.target.id, r.addedNodes[0].id, r.addedNodes.length, r.previousSibling.id, r.nextSibling && r.nextSibling.id); }); });\
         mo.observe(document.getElementById('a'), { childList: true });\
         document.write('<b id=w>hi</b><u id=v></u>');</script><i id=z></i></p></body></html>",
    );
    assert_eq!(out, ["log:childList a w 2 sc z"], "{out:?}");
}

#[test]
fn qy6a_subtree_observation() {
    let o = qy6a_out(
        r#"const h = $('h'); const mo = watch(h, { subtree: true, childList: true, attributes: true });
         step(() => $('a').appendChild(mk('p', 'p1')),
              () => $('p1').setAttribute('q', '1'),
              () => { $('s').appendChild(mk('z', 'z1')); });"#,
    );
    assert_eq!(o, ["childList|a|p1|-|-|-|null|null", "attributes|p1|-|-|-|-|q|null"]);
}

#[test]
fn qy6a_without_subtree_only_direct_children() {
    let o = qy6a_out(
        r#"const h = $('h'); watch(h, { childList: true, attributes: true });
         step(() => $('a').appendChild(mk('p', 'p1')), () => $('a').setAttribute('q', '1'), () => h.setAttribute('q', '1'));"#,
    );
    assert_eq!(o, ["attributes|h|-|-|-|-|q|null"]);
}

#[test]
fn qy6a_transient_observers_for_removed_subtree() {
    let o = qy6a_out(
        r#"const h = $('h'), a = $('a'); watch(h, { subtree: true, attributes: true });
         const kid = mk('p', 'p1'); a.appendChild(kid);
         step(() => { h.removeChild(a); a.setAttribute('x', '1'); kid.setAttribute('y', '1'); },
              () => { a.setAttribute('after', '1'); });"#,
    );
    // The removal inside the delivery turn: both writes still reach the observer
    // (transient registrations); once delivered they are dropped.
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null ; attributes|p1|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_batching_microtask_order_and_this() {
    let log = qy6a_run(
        r#"const h = $('h');
         const mo = new MutationObserver(function (recs, obs) { console.log('cb', recs.length, this === mo, obs === mo); });
         mo.observe(h, { childList: true, attributes: true });
         queueMicrotask(() => console.log('micro before'));
         h.appendChild(mk('p')); h.setAttribute('x', '1'); h.appendChild(mk('p'));
         console.log('sync done');
         Promise.resolve().then(() => console.log('micro after'));"#,
    );
    assert_eq!(log, ["log:sync done", "log:micro before", "log:cb 3 true true", "log:micro after"], "{log:?}");
}

#[test]
fn qy6a_observers_notified_in_creation_order() {
    let log = qy6a_run(
        r#"const h = $('h');
         const m1 = new MutationObserver(() => console.log('first')), m2 = new MutationObserver(() => console.log('second'));
         m2.observe(h, { childList: true }); m1.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert_eq!(log, ["log:first", "log:second"], "{log:?}");
}

#[test]
fn qy6a_records_made_in_a_callback_deliver_in_the_next_batch() {
    let log = qy6a_run(
        r#"const h = $('h'); let k = 0;
         const mo = new MutationObserver(recs => { console.log('cb', recs.length); if (k++ === 0) h.appendChild(mk('q')); });
         mo.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert_eq!(log, ["log:cb 1", "log:cb 1"], "{log:?}");
}

#[test]
fn qy6a_disconnect_and_take_records() {
    let log = qy6a_run(
        r#"const h = $('h');
         const mo = new MutationObserver(recs => console.log('delivered', recs.length));
         mo.observe(h, { childList: true });
         h.appendChild(mk('p'));
         const t = mo.takeRecords();
         console.log('taken', t.length, t[0].type, mo.takeRecords().length);
         h.appendChild(mk('p'));
         mo.disconnect();
         h.appendChild(mk('p'));
         const m2 = new MutationObserver(recs => console.log('m2', recs.length));
         m2.observe(h, { childList: true });
         h.appendChild(mk('p'));
         m2.disconnect();
         Promise.resolve().then(() => console.log('end'));"#,
    );
    assert_eq!(log, ["log:taken 1 childList 0", "log:end"], "{log:?}");
}

#[test]
fn qy6a_reobserve_replaces_options_and_other_targets_keep_theirs() {
    let o = qy6a_out(
        r#"const h = $('h'), a = $('a');
         const mo = new MutationObserver(l => out.push(l.map(d).join(' ; ')));
         mo.observe(h, { childList: true }); mo.observe(a, { attributes: true });
         mo.observe(h, { attributes: true });
         step(() => { h.appendChild(mk('p')); h.setAttribute('x', '1'); a.setAttribute('y', '1'); });"#,
    );
    assert_eq!(o, ["attributes|h|-|-|-|-|x|null ; attributes|a|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_observe_validates_options() {
    let log = qy6a_run(
        r#"const mo = new MutationObserver(() => {}), h = $('h');
         const t = o => { try { mo.observe(h, o); console.log('ok'); } catch (e) { console.log(e.name); } };
         t({}); t({ subtree: true }); t({ attributes: false });
         t({ attributeOldValue: true }); t({ attributeFilter: ['a'] }); t({ characterDataOldValue: true });
         t({ attributes: false, attributeOldValue: true }); t({ childList: true, attributeFilter: ['a'], attributes: false });
         t({ characterData: false, characterDataOldValue: true, childList: true });
         try { mo.observe(); } catch (e) { console.log(e.name); }
         try { mo.observe({}, { childList: true }); } catch (e) { console.log(e.name); }
         try { new MutationObserver(); } catch (e) { console.log(e.name); }
         try { new MutationObserver(1); } catch (e) { console.log(e.name); }
         try { MutationObserver(() => {}); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:TypeError", "log:TypeError", "log:TypeError", "log:ok", "log:ok", "log:ok", "log:TypeError", "log:TypeError",
            "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6a_globals_and_record_shape() {
    let log = qy6a_run(
        r#"console.log(window.MutationObserver === MutationObserver, WebKitMutationObserver === MutationObserver, typeof MutationRecord);
         console.log(Object.prototype.toString.call(new MutationObserver(() => {})), String(MutationRecord.prototype[Symbol.toStringTag]));
         const mo = new MutationObserver(recs => {
           const r = recs[0];
           console.log(r instanceof MutationRecord, Object.prototype.toString.call(r), r.type, r.attributeNamespace, Array.isArray(r.addedNodes), r.removedNodes.length, r.target === $('a'));
         });
         mo.observe($('a'), { attributes: true }); $('a').setAttribute('k', 'v');"#,
    );
    assert_eq!(
        log,
        [
            "log:true true function",
            "log:[object MutationObserver] MutationRecord",
            "log:true [object MutationRecord] attributes null true 0 true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6a_callback_error_does_not_stop_other_observers() {
    let log = qy6a_run(
        r#"const h = $('h');
         const m1 = new MutationObserver(() => { throw new Error('boom'); }), m2 = new MutationObserver(() => console.log('second ran'));
         m1.observe(h, { childList: true }); m2.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert!(log.iter().any(|l| l == "log:second ran"), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("error:") && l.contains("MutationObserver callback") && l.contains("boom")), "{log:?}");
}

#[test]
fn qy6a_custom_elements_and_observers_share_the_hooks() {
    let log = qy6a_run(
        r#"class XQ extends HTMLElement { connectedCallback() { console.log('conn'); } }
         customElements.define('x-q', XQ);
         const mo = new MutationObserver(l => console.log('records', l.length));
         mo.observe($('h'), { childList: true });
         $('h').appendChild(mk('x-q'));"#,
    );
    assert_eq!(log, ["log:conn", "log:records 1"], "{log:?}");
}

// ---- QY6g: IntersectionObserver / ResizeObserver / PerformanceObserver ----

/// Like `run_with_timers` but on a parsed document, so observers get targets.
fn qy6g_run(code: &str) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let html = "<html><body><div id=a></div><img id=i width=40 height=30>\
                <div id=hid style='display:none'><p id=inner></p></div><div id=b></div></body></html>";
    let engine = crate::js::start_scripts_clocked(
        &parse_html(html),
        &[src(&format!("const $ = id => document.getElementById(id);\n{code}"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..1000 {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    let v = out.borrow().clone();
    v
}

#[test]
fn qy6g_io_initial_entry_delivered_once_visible() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(function (es, o) {
             console.log('cb', es.length, this === io, o === io);
             es.forEach(e => console.log(e.target.id, e.isIntersecting, e.intersectionRatio, e instanceof IntersectionObserverEntry,
               e.boundingClientRect instanceof DOMRectReadOnly, e.rootBounds.width, e.rootBounds.height, typeof e.time));
           });
           io.observe($('a')); io.observe($('a')); io.observe($('i'));
           console.log('sync');"#,
    );
    assert_eq!(
        log,
        ["log:sync", "log:cb 2 true true", "log:a true 1 true true 1024 768 number", "log:i true 1 true true 1024 768 number"],
        "{log:?}"
    );
}

#[test]
fn qy6g_io_hidden_and_detached_report_not_intersecting() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log(e.target.id || 'det', e.isIntersecting, e.intersectionRatio)));
           io.observe($('inner')); io.observe($('hid')); io.observe(document.createElement('div')); io.observe($('b'));"#,
    );
    assert_eq!(log, ["log:inner false 0", "log:hid false 0", "log:det false 0", "log:b true 1"], "{log:?}");
}

#[test]
fn qy6g_io_unobserve_disconnect_take_records() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(es => console.log('A', es.map(e => e.target.id).join()));
           io.observe($('a')); io.observe($('b')); io.unobserve($('a'));
           const i2 = new IntersectionObserver(es => console.log('B'));
           i2.observe($('a')); i2.disconnect();
           const i3 = new IntersectionObserver(es => console.log('C'));
           i3.observe($('a')); const r = i3.takeRecords(); console.log('took', r.length, i3.takeRecords().length);"#,
    );
    assert_eq!(log, ["log:took 1 0", "log:A b"], "{log:?}");
}

#[test]
fn qy6g_io_arguments_and_properties() {
    let log = qy6g_run(
        r#"const t = f => { try { f(); return 'ok'; } catch (e) { return e.name; } };
           console.log(t(() => new IntersectionObserver(() => {}, { threshold: 2 })),
                       t(() => new IntersectionObserver(() => {}, { threshold: [0, -1] })),
                       t(() => new IntersectionObserver(() => {}, { rootMargin: '5em' })),
                       t(() => new IntersectionObserver(() => {}, { rootMargin: '1px 2px 3px 4px 5px' })),
                       t(() => new IntersectionObserver()),
                       t(() => new IntersectionObserver(1)),
                       t(() => IntersectionObserver(() => {})),
                       t(() => new IntersectionObserver(() => {}).observe({})),
                       t(() => new IntersectionObserver(() => {}, { root: 5 })),
                       t(() => new IntersectionObserver(() => {}, { threshold: [0.5, 0.25] })));
           const o = new IntersectionObserver(() => {}, { root: $('a'), rootMargin: '10px 20%', threshold: [1, 0.5] });
           console.log(o.root === $('a'), o.rootMargin, o.thresholds.join(), new IntersectionObserver(() => {}).thresholds.join(), new IntersectionObserver(() => {}).rootMargin,
                       new IntersectionObserver(() => {}, { threshold: 0.3 }).thresholds[0]);
           const r = new DOMRect(1, 2, 3, 4); console.log(r.right, r.bottom, JSON.stringify(DOMRectReadOnly.fromRect({ x: 5 })));"#,
    );
    assert_eq!(
        log,
        [
            "log:RangeError RangeError SyntaxError SyntaxError TypeError TypeError TypeError TypeError TypeError ok",
            "log:true 10px 20% 10px 20% 0.5,1 0 0px 0px 0px 0px 0.3",
            "log:4 6 {\"x\":5,\"y\":0,\"width\":0,\"height\":0,\"top\":0,\"right\":5,\"bottom\":0,\"left\":5}",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6g_io_one_batch_per_observer_in_creation_order() {
    let log = qy6g_run(
        r#"const x = new IntersectionObserver(es => console.log('x', es.length));
           const y = new IntersectionObserver(es => console.log('y', es.length));
           y.observe($('a')); x.observe($('a')); x.observe($('b')); y.observe($('b')); y.observe($('i'));"#,
    );
    assert_eq!(log, ["log:x 2", "log:y 3"], "{log:?}");
}

#[test]
fn qy6g_resize_observer_one_initial_entry() {
    let log = qy6g_run(
        r#"const ro = new ResizeObserver(function (es, o) {
             console.log('cb', es.length, this === ro, o === ro);
             es.forEach(e => console.log(e.target.id, e.contentRect.width, e.contentRect.height, e instanceof ResizeObserverEntry,
               e.borderBoxSize.length, e.contentBoxSize[0].inlineSize, e.contentBoxSize[0].blockSize, e.devicePixelContentBoxSize.length));
           });
           ro.observe($('a')); ro.observe($('i'), { box: 'border-box' }); ro.observe($('b')); ro.unobserve($('b'));
           try { ro.observe($('a'), { box: 'bogus' }); } catch (e) { console.log(e.name); }
           try { new ResizeObserver(); } catch (e) { console.log(e.name); }
           const r2 = new ResizeObserver(() => console.log('NO')); r2.observe($('a')); r2.disconnect();"#,
    );
    assert_eq!(
        log,
        ["log:TypeError", "log:TypeError", "log:cb 2 true true", "log:a 0 0 true 1 0 0 1", "log:i 40 30 true 1 40 30 1"],
        "{log:?}"
    );
}

#[test]
fn qy6g_performance_observer_is_inert() {
    let log = qy6g_run(
        r#"let n = 0;
           const po = new PerformanceObserver(() => n++);
           po.observe({ entryTypes: ['longtask', 'resource'] }); po.observe({ type: 'paint', buffered: true });
           const t = f => { try { f(); return 'ok'; } catch (e) { return e.name; } };
           console.log(t(() => po.observe({})), t(() => po.observe({ entryTypes: [], type: 'x' })), t(() => new PerformanceObserver()),
                       PerformanceObserver.supportedEntryTypes.length, po.takeRecords().length, typeof po.disconnect());
           setTimeout(() => console.log('n', n), 5);"#,
    );
    assert_eq!(log, ["log:TypeError TypeError TypeError 0 0 undefined", "log:n 0"], "{log:?}");
}

#[test]
fn qy6g_zero_cost_before_use() {
    // No timer exists until the first observe().
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts_clocked(
        &parse_html("<div id=a></div>"),
        &[src("new IntersectionObserver(() => {}); new ResizeObserver(() => {}); new PerformanceObserver(() => {}).observe({ entryTypes: ['mark'] });")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap();
    assert!(engine.next_due_ms().is_none(), "{:?}", out.borrow());
    assert!(out.borrow().is_empty(), "{:?}", out.borrow());
}

// ---------- QF14b: the scratchy half of the cookie jar (fake wire, no network) ----------

/// A fake wire on a unix socket that records every request (head + body) and
/// answers by URL extension like a cookie-capable wire 3.
fn qf14b_fake_wire(path: &std::path::Path) -> Arc<Mutex<Vec<String>>> {
    use std::os::unix::net::UnixListener;
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("bind fake wire");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let s = s.clone();
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let line = text.lines().next().unwrap_or("").to_string();
                let (ct, body): (&str, &str) = if line.contains(".css") {
                    ("text/css", "p{margin:0}")
                } else if line.contains(".js") {
                    ("application/javascript", "var a = 1;")
                } else if line.contains(".png") {
                    ("image/png", "not really a png")
                } else {
                    ("text/html", "<html><body>x</body></html>")
                };
                s.lock().unwrap().push(text);
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nX-Wire-Version: 3\r\nX-Wire-Cookies: on\r\n\r\n{body}");
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    seen
}

fn qf14b_has(req: &str, header: &str) -> bool {
    req.to_ascii_lowercase().contains(&format!("\r\n{}\r\n", header.to_ascii_lowercase()))
}

fn qf14b_wait(seen: &Arc<Mutex<Vec<String>>>, n: usize) -> Vec<String> {
    for _ in 0..300 {
        if seen.lock().unwrap().len() >= n {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    seen.lock().unwrap().clone()
}

#[test]
fn qf14b_site_of_is_scheme_plus_registrable_domain() {
    let site = |s: &str| site_of(&Url::parse(s).unwrap());
    assert_eq!(site("https://www.example.com/a?b").as_deref(), Some("https://example.com"));
    assert_eq!(site("https://news.bbc.co.uk/").as_deref(), Some("https://bbc.co.uk"));
    assert_eq!(site("http://127.0.0.1:8080/x").as_deref(), Some("http://127.0.0.1"));
    assert_eq!(site("http://localhost:1/").as_deref(), Some("http://localhost"));
    assert_eq!(site("https://a.b.github.io/").as_deref(), Some("https://b.github.io"));
    assert_eq!(site("http://example.com/").as_deref(), Some("http://example.com"), "scheme is part of the site");
    assert_eq!(site("file:///tmp/x.html"), None, "file: has no cookie origin");
    assert_eq!(site("data:text/plain,hi"), None);
}

#[test]
fn qf14b_cookie_headers_follow_the_fetch_spec_and_the_navigation_kind() {
    let sup = CookieSupport::new(true);
    let hs = |ctx: &CookieContext| wire_cookie_headers_with(&sup, ctx);
    let has = |h: &[(String, String)], n: &str, v: &str| h.iter().any(|(a, b)| a == n && b == v);
    let top = Url::parse("https://www.top.example/p").unwrap();
    // Sub-resource: jar + the page's site, even for a third party.
    let h = hs(&CookieContext::subresource(&top));
    assert!(has(&h, "X-Wire-Jar", "scratchy") && has(&h, "X-Wire-Top-Site", "https://top.example"), "{h:?}");
    assert!(!h.iter().any(|(n, _)| n == "X-Wire-Nav"));
    // Navigation: Nav + (page-initiated) initiator; top site = destination.
    let dest = Url::parse("https://shop.other.test/x").unwrap();
    let h = hs(&CookieContext::navigation(&dest, Some(&top)));
    assert!(has(&h, "X-Wire-Top-Site", "https://other.test") && has(&h, "X-Wire-Nav", "1"), "{h:?}");
    assert!(has(&h, "X-Wire-Initiator-Site", "https://top.example"), "{h:?}");
    let h = hs(&CookieContext::navigation(&dest, None));
    assert!(has(&h, "X-Wire-Nav", "1") && !h.iter().any(|(n, _)| n == "X-Wire-Initiator-Site"), "browser-initiated: no initiator {h:?}");
    // Fetch credentials modes.
    let f = |c: CredentialsMode, same: bool| hs(&CookieContext::fetch(&top, c, same));
    let off = vec![("X-Wire-Cookies".to_string(), "off".to_string())];
    assert_eq!(f(CredentialsMode::Omit, true), off);
    assert_eq!(f(CredentialsMode::SameOrigin, false), off, "cross-origin same-origin mode: no credentials");
    assert!(has(&f(CredentialsMode::SameOrigin, true), "X-Wire-Jar", "scratchy"));
    assert!(has(&f(CredentialsMode::Include, false), "X-Wire-Jar", "scratchy"));
    assert_eq!(parse_credentials("omit"), CredentialsMode::Omit);
    assert_eq!(parse_credentials(""), CredentialsMode::SameOrigin);
    // No cookie origin (file: page): nothing at all.
    let file = Url::parse("file:///tmp/p.html").unwrap();
    assert!(hs(&CookieContext::subresource(&file)).is_empty());
    assert!(hs(&CookieContext::fetch(&file, CredentialsMode::Include, true)).is_empty());
}

#[test]
fn qf14b_old_wire_is_detected_once_and_stops_the_jar_headers() {
    let top = Url::parse("https://top.example/").unwrap();
    let ctx = CookieContext::subresource(&top);
    // A pre-cookie wire: no X-Wire-Version.
    let sup = CookieSupport::new(true);
    assert!(!wire_cookie_headers_with(&sup, &ctx).is_empty(), "optimistic until proven otherwise");
    assert_eq!(sup.observe(None, None).as_deref(), Some("scratchy: cookies unavailable: wire is too old (restart it)"));
    assert_eq!(sup.observe(None, None), None, "logged ONCE");
    assert_eq!(sup.observe(Some("2"), None), None);
    assert!(wire_cookie_headers_with(&sup, &ctx).is_empty(), "no jar header to a wire that cannot use it");
    // --no-cookies.
    let sup = CookieSupport::new(true);
    assert_eq!(sup.observe(Some("3"), Some("off")).as_deref(), Some("scratchy: cookies unavailable: wire has cookies disabled"));
    assert!(!sup.usable());
    // A good wire.
    let sup = CookieSupport::new(true);
    assert_eq!(sup.observe(Some("3"), Some("on")), None);
    assert!(sup.usable());
    // Over a real socket: an old fake wire (no version) switches it off.
    let path = unique_test_socket_path("qf14b-old");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\np{}");
    let sup = CookieSupport::new(true);
    let req = build_get_request_ctx(&Url::parse("https://cdn.test/a.css").unwrap(), &CookieContext::subresource(&top));
    assert!(req.contains("X-Wire-Jar: scratchy"));
    let _ = fetch_raw_via_wire_with_support(&path, &req, std::time::Duration::from_secs(5), &sup);
    assert!(!sup.usable(), "learned from the first response");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qf14b_cookie_headers_are_built_in_one_place() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name == "net_policy.rs" || name == "tests.rs" || !name.ends_with(".rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap().to_ascii_lowercase();
        for lit in ["x-wire-jar", "x-wire-top-site", "x-wire-nav", "x-wire-initiator-site", "x-wire-cookie-op", "x-wire-cookie-url"] {
            // Only comments may name them (those lines start with `//`).
            let code_use = text.lines().any(|l| l.contains(lit) && !l.trim_start().starts_with("//") && !l.contains("`"));
            assert!(!code_use, "{name} builds a cookie header ({lit}) outside net_policy.rs");
        }
        checked += 1;
    }
    assert!(checked > 5);
}

#[test]
fn qf14b_every_loader_sends_the_jar_headers_with_the_pages_top_site() {
    set_js_enabled(true);
    let path = unique_test_socket_path("qf14b-loaders");
    let seen = qf14b_fake_wire(&path);
    let top = Url::parse("https://www.top.example/page").unwrap();
    let third = |p: &str| Url::parse(&format!("https://cdn.third.test/{p}")).unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &third("a.css"), &top).as_deref(), Some("p{margin:0}"));
    assert_eq!(fetch_script_via_wire(&path, &third("b.js"), &top).as_deref(), Some("var a = 1;"));
    let _ = fetch_image_via_wire(&path, &third("c.png"), &top);
    assert_eq!(fetch_stylesheets(&path, &top, &top, &["https://cdn.third.test/d.css".to_string()]), "p{margin:0}\n");
    let scripts = fetch_scripts(&path, &top, &top, vec![ScriptRef::ModuleExternal("https://cdn.third.test/e.js".into())]);
    assert_eq!(scripts.len(), 1);
    let doc = parse_html("<html><body><iframe src='https://www.top.example/f.html'></iframe></body></html>");
    prefetch_iframes(&path, &top, &top, &doc);
    let reqs = qf14b_wait(&seen, 6);
    assert_eq!(reqs.len(), 6, "{reqs:?}");
    for r in &reqs {
        assert!(qf14b_has(r, "X-Wire-Jar: scratchy"), "no jar header: {r}");
        assert!(qf14b_has(r, "X-Wire-Top-Site: https://top.example"), "wrong partition key: {r}");
        assert!(!r.to_ascii_lowercase().contains("x-wire-nav"), "sub-resources are not navigations: {r}");
    }
    // The same third party under ANOTHER top site gets another partition key.
    let top2 = Url::parse("https://other.example/").unwrap();
    let _ = fetch_stylesheet_via_wire(&path, &third("a.css"), &top2);
    let reqs = qf14b_wait(&seen, 7);
    assert!(qf14b_has(reqs.last().unwrap(), "X-Wire-Top-Site: https://other.example"));
    // A file: page has no cookie origin: no jar headers.
    let file = Url::parse("file:///tmp/p.html").unwrap();
    let _ = fetch_stylesheet_via_wire(&path, &third("a.css"), &file);
    let reqs = qf14b_wait(&seen, 8);
    assert!(!reqs.last().unwrap().to_ascii_lowercase().contains("x-wire-"), "{:?}", reqs.last());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qf14b_link_navigation_sends_nav_and_initiator_but_typed_go_does_not() {
    let dest = Url::parse("https://www.shop.example/cart").unwrap();
    let from = Url::parse("https://news.example/").unwrap();
    let click = build_get_request_ctx(&dest, &CookieContext::navigation(&dest, Some(&from)));
    assert!(click.starts_with("GET https://www.shop.example/cart HTTP/1.1\r\nHost: www.shop.example\r\n"), "{click}");
    for h in ["X-Wire-Jar: scratchy", "X-Wire-Top-Site: https://shop.example", "X-Wire-Nav: 1", "X-Wire-Initiator-Site: https://news.example"] {
        assert!(qf14b_has(&click, h), "{h} missing: {click}");
    }
    assert!(click.ends_with("\r\n\r\n"));
    let typed = build_get_request_ctx(&dest, &CookieContext::navigation(&dest, None));
    assert!(qf14b_has(&typed, "X-Wire-Nav: 1") && !typed.contains("Initiator"), "{typed}");
}

/// App side (needs a display like the other App tests): `start_navigation`
/// (a link) sends the current page as the initiator, `:go` does not.
fn qf14b_app_initiator(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let path = unique_test_socket_path("qf14b-app");
    let seen = qf14b_fake_wire(&path);
    let doc = parse_html("<html><body></body></html>");
    let mut app = App::new_static(doc, proxy.clone(), Some(path.clone()));
    app.base_url = Some(Url::parse("https://news.example/a").unwrap());
    app.start_navigation("https://shop.example/x");
    let reqs = qf14b_wait(&seen, 1);
    assert!(qf14b_has(&reqs[0], "X-Wire-Initiator-Site: https://news.example"), "{reqs:?}");
    app.dispatch_command("go", &["https://typed.example/y"]);
    let reqs = qf14b_wait(&seen, 2);
    let typed = reqs.iter().find(|r| r.contains("typed.example")).expect("typed request");
    assert!(qf14b_has(typed, "X-Wire-Nav: 1") && !typed.contains("Initiator"), "{typed}");
    let _ = std::fs::remove_file(&path);
}

/// A canned wire for the control channel: keeps a tiny jar (one HttpOnly
/// cookie the page must never see) and records requests.
struct JarFake {
    cookies: Mutex<Vec<(String, String, bool)>>,
    raw: Mutex<Vec<String>>,
    timeouts: Mutex<Vec<std::time::Duration>>,
    version: Option<&'static str>,
}

impl JarFake {
    fn new(version: Option<&'static str>) -> Arc<Self> {
        Arc::new(JarFake {
            cookies: Mutex::new(vec![("sid".into(), "SECRET".into(), true), ("seen".into(), "1".into(), false)]),
            raw: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            version,
        })
    }
}

impl NetExecutor for JarFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        let text = String::from_utf8_lossy(&job.request).to_string();
        self.raw.lock().unwrap().push(text.clone());
        self.timeouts.lock().unwrap().push(job.limits.read_timeout);
        let mut headers = vec![];
        if let Some(v) = self.version {
            headers.push(("x-wire-version".to_string(), v.to_string()));
            headers.push(("x-wire-cookies".to_string(), "on".to_string()));
        }
        let mk = |status: u16, body: String| Ok(WireResponse { status, headers: headers.clone(), body: body.into_bytes(), ..Default::default() });
        if text.contains("X-Wire-Cookie-Op: get") {
            let v: Vec<String> = self.cookies.lock().unwrap().iter().filter(|c| !c.2).map(|c| format!("{}={}", c.0, c.1)).collect();
            return mk(200, v.join("; "));
        }
        if text.contains("X-Wire-Cookie-Op: set") {
            let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            if !body.to_ascii_lowercase().contains("httponly") {
                if let Some((n, v)) = body.split(';').next().unwrap_or("").split_once('=') {
                    self.cookies.lock().unwrap().push((n.trim().to_string(), v.trim().to_string(), false));
                }
            }
            return mk(204, String::new());
        }
        mk(200, "ok".into())
    }
}

fn qf14b_engine(page: &str, fake: &Arc<JarFake>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}")))).unwrap();
    (engine, out)
}

#[test]
fn qf14b_document_cookie_round_trips_through_the_control_channel_and_hides_httponly() {
    let fake = JarFake::new(Some("3"));
    let (engine, log) = qf14b_engine("https://www.page.test/dir/x", &fake);
    engine.eval(
        "t",
        "var d = Object.getOwnPropertyDescriptor(Document.prototype, 'cookie');\
         console.log('proto', typeof d.get, typeof d.set, document.hasOwnProperty('cookie'));\
         console.log('before', document.cookie);\
         document.cookie = 'a=1; path=/';\
         console.log('after', document.cookie);\
         var second = document.implementation.createHTMLDocument('x');\
         second.cookie = 'zzz=9';\
         console.log('second', JSON.stringify(second.cookie));",
    );
    let l = log.borrow().clone();
    assert!(l.contains(&"log:proto function function false".to_string()), "{l:?}");
    assert!(l.contains(&"log:before seen=1".to_string()), "HttpOnly sid never reaches the page: {l:?}");
    assert!(l.contains(&"log:after seen=1; a=1".to_string()), "{l:?}");
    assert!(l.contains(&"log:second \"\"".to_string()), "second documents are inert: {l:?}");
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 3, "get, set, get; the second document made no request: {raw:?}");
    let set = &raw[1];
    for h in ["X-Wire-Jar: scratchy", "X-Wire-Top-Site: https://page.test", "X-Wire-Cookie-Op: set", "X-Wire-Cookie-Url: https://www.page.test/dir/x"] {
        assert!(qf14b_has(set, h), "{h}: {set}");
    }
    assert!(set.ends_with("\r\n\r\na=1; path=/"), "the assigned string is the body: {set}");
    assert!(fake.timeouts.lock().unwrap().iter().all(|t| *t == std::time::Duration::from_millis(2000)), "cookie_control_timeout_ms");
    assert!(!l.iter().any(|x| x.starts_with("error:")), "{l:?}");
}

#[test]
fn qf14b_document_cookie_is_inert_on_file_pages() {
    let fake = JarFake::new(Some("3"));
    let (engine, log) = qf14b_engine("file:///tmp/p.html", &fake);
    engine.eval("t", "document.cookie = 'a=1'; console.log(JSON.stringify(document.cookie));");
    assert!(log.borrow().contains(&"log:\"\"".to_string()), "{:?}", log.borrow());
    assert!(fake.raw.lock().unwrap().is_empty(), "no control request without a cookie origin");
}

#[test]
fn qf14b_document_cookie_failure_logs_once_and_reads_empty() {
    let fake = QfFake::new(vec![]); // every request fails
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", "document.cookie = 'a=1'; console.log(JSON.stringify(document.cookie)); console.log(JSON.stringify(document.cookie));");
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| x.starts_with("net:document.cookie:")).count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| *x == "log:\"\"").count(), 2, "{l:?}");
    assert_eq!(fake.urls().len(), 1, "after a failure the engine thread never blocks on wire again");
}

#[test]
fn qf14b_old_wire_for_document_cookie_logs_once_and_sends_no_more_jar_headers() {
    let fake = JarFake::new(None); // no X-Wire-Version: pre-cookie wire
    let (engine, log) = qf14b_engine("https://page.test/", &fake);
    engine.set_cookie_support(Box::leak(Box::new(CookieSupport::new(true))));
    engine.eval(
        "t",
        "document.cookie; document.cookie = 'a=1'; document.cookie;\
         __net_request({method:'GET', url:'https://page.test/x', headers:[], body:null});",
    );
    qf14b_drive(&engine, &fake);
    let l = log.borrow().clone();
    let warn = "net-warning:scratchy: cookies unavailable: wire is too old (restart it)";
    assert_eq!(l.iter().filter(|x| *x == warn).count(), 1, "{l:?}");
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 2, "one control probe, then the plain request: {raw:?}");
    assert!(!raw[1].to_ascii_lowercase().contains("x-wire-jar"), "{}", raw[1]);
}

fn qf14b_drive(engine: &JsEngine, fake: &Arc<JarFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

#[test]
fn qf14b_fetch_and_xhr_credentials_decide_whether_the_jar_is_used() {
    let ok = || qf7_ok("x", &[("access-control-allow-origin", "https://page.test"), ("access-control-allow-credentials", "true")]);
    let fake = QfFake::new(vec![("/same", ok()), ("/cross", ok())]);
    let (engine, _log) = qf7_engine("https://page.test/", &fake);
    let go = |url: &str, cred: &str| qf7_go(url, &format!(",credentials:'{cred}'"));
    engine.eval(
        "t",
        &[
            go("https://page.test/same?default", "same-origin"),
            go("https://page.test/same?omit", "omit"),
            go("https://page.test/same?include", "include"),
            go("https://other.test/cross?default", "same-origin"),
            go("https://other.test/cross?omit", "omit"),
            go("https://other.test/cross?include", "include"),
        ]
        .concat(),
    );
    qf7_drive(&engine, &fake);
    let raw: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_string()).collect();
    let find = |needle: &str| raw.iter().find(|r| r.contains(needle)).unwrap_or_else(|| panic!("{needle} not sent: {raw:?}")).clone();
    let jar = |r: &str| qf14b_has(r, "X-Wire-Jar: scratchy") && qf14b_has(r, "X-Wire-Top-Site: https://page.test");
    let off = |r: &str| qf14b_has(r, "X-Wire-Cookies: off") && !r.to_ascii_lowercase().contains("x-wire-jar");
    assert!(jar(&find("same?default")));
    assert!(off(&find("same?omit")), "omit sends no jar and says off");
    assert!(jar(&find("same?include")));
    assert!(off(&find("cross?default")), "cross-origin with the default same-origin mode: no credentials");
    assert!(off(&find("cross?omit")));
    assert!(jar(&find("cross?include")));
    let e = engine.net_entries();
    assert_eq!(e.iter().map(|x| x.jar).collect::<Vec<_>>(), vec![true, false, true, false, false, true]);
    assert!(raw.iter().all(|r| !r.to_ascii_lowercase().contains("\r\ncookie:")), "page JS never sets Cookie itself");
    let report = crate::js_net::format_net_report(&e);
    assert!(report.contains("jar: scratchy") && report.contains("jar: off"), "{report}");
}

#[test]
fn qf14b_xhr_with_credentials_maps_to_include() {
    let fake = QfFake::new(vec![("/x", qf7_ok("x", &[("access-control-allow-origin", "https://page.test"), ("access-control-allow-credentials", "true")]))]);
    let (engine, _log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "var a = new XMLHttpRequest(); a.open('GET', 'https://other.test/x?plain'); a.send();\
         var b = new XMLHttpRequest(); b.open('GET', 'https://other.test/x?creds'); b.withCredentials = true; b.send();",
    );
    qf7_drive(&engine, &fake);
    let raw: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_string()).collect();
    let plain = raw.iter().find(|r| r.contains("?plain")).unwrap();
    let creds = raw.iter().find(|r| r.contains("?creds")).unwrap();
    assert!(!plain.to_ascii_lowercase().contains("x-wire-jar"), "cross-origin XHR without withCredentials: no jar");
    assert!(qf14b_has(creds, "X-Wire-Jar: scratchy"));
}

#[test]
fn qf14b_js_report_has_a_cookie_smoke_line() {
    let fake = QfFake::new(vec![("page.test/data", qf7_ok("12345", &[]))]);
    crate::js_net::set_net_executor(fake.clone());
    let html = "<html><head><base href='https://page.test/'></head><body><script>\
        __net_request({method:'GET', url:'https://page.test/data', headers:[], body:null});\
        </script></body></html>";
    set_js_enabled(true);
    let report = js::js_report_for_html(html, None, "/tmp/qf14b-none.html");
    assert!(report.contains("GET https://page.test/data -> 200 OK 5 bytes"), "{report}");
    assert!(report.contains("jar: scratchy"), "{report}");
    assert!(report.contains("--- cookies ---\ndocument.cookie: "), "{report}");
    assert!(report.contains("errors total: 0"), "{report}");
}

// ---- QY6f: template.content, importNode.

#[test]
fn qy6f_parsed_template_content_innerhtml_and_outerhtml() {
    let log = run_in_html(
        "<html><body><template id=t><p class=x>a</p><b>b</b></template><div id=d></div></body></html>",
        r#"const t = document.getElementById('t');
         console.log(t.childNodes.length, t.content.childNodes.length, t.content === t.content, t.content.nodeType);
         console.log(t.innerHTML);
         console.log(t.outerHTML);
         console.log(t.content.querySelector('p.x').textContent, t.content.querySelectorAll('*').length);
         console.log(document.querySelector('p.x'), t.content.ownerDocument === document, t.content.ownerDocument.nodeType);
         t.innerHTML = '<i>z</i>';
         console.log(t.content.childNodes.length, t.content.firstChild.localName, t.innerHTML);
         console.log(t instanceof HTMLTemplateElement);"#,
    );
    assert_eq!(
        log,
        [
            "log:0 2 true 11",
            "log:<p class=\"x\">a</p><b>b</b>",
            "log:<template id=\"t\"><p class=\"x\">a</p><b>b</b></template>",
            "log:a 2",
            "log:null false 9",
            "log:1 i <i>z</i>",
            "log:true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6f_created_template_clone_import_and_append_fragment() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        r#"const t = document.createElement('template');
         const p = document.createElement('p'); p.textContent = 'hi';
         t.content.appendChild(p);
         console.log(t.content.childNodes.length, t.childNodes.length, t.innerHTML, p.ownerDocument === t.content.ownerDocument);
         const c = t.cloneNode(true);
         console.log(c.content !== t.content, c.content.childNodes.length, c.content.firstChild !== p, c.innerHTML);
         const shallow = t.cloneNode(false);
         console.log(shallow.content.childNodes.length);
         const fc = t.content.cloneNode(true);
         console.log(fc.childNodes.length, fc.firstChild.ownerDocument === t.content.ownerDocument);
         const imp = document.importNode(t.content, true);
         console.log(imp.childNodes.length, imp.firstChild.ownerDocument === document, t.content.childNodes.length);
         const d = document.getElementById('d');
         d.appendChild(imp);
         console.log(d.innerHTML, imp.childNodes.length);
         document.body.appendChild(t.content);
         console.log(t.content.childNodes.length, document.body.querySelectorAll('p').length, p.ownerDocument === document);"#,
    );
    assert_eq!(
        log,
        [
            "log:1 0 <p>hi</p> true",
            "log:true 1 true <p>hi</p>",
            "log:0",
            "log:1 true",
            "log:1 true 1",
            "log:<p>hi</p> 0",
            "log:0 2 true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6f_content_is_inert_no_custom_element_upgrade() {
    let log = run_in_html(
        "<html><body><template id=t><x-q></x-q></template></body></html>",
        r#"let n = 0;
         class XQ extends HTMLElement { constructor() { super(); n++; } }
         customElements.define('x-q', XQ);
         const t = document.getElementById('t');
         const e = t.content.firstChild;
         t.innerHTML = '<x-q></x-q>';
         const cl = t.content.cloneNode(true);
         console.log(n, e instanceof XQ, t.content.firstChild instanceof XQ, cl.firstChild instanceof XQ);
         const im = document.importNode(t.content, true);
         console.log(n, im.firstChild instanceof XQ);
         document.body.appendChild(t.content);
         console.log(n, document.body.lastChild instanceof XQ);"#,
    );
    assert_eq!(log, ["log:0 false false false", "log:1 true", "log:2 true"], "{log:?}");
}

#[test]
fn qy6f_template_is_not_rendered() {
    let doc = parse_html("<html><body><p>shown</p><template><p>TPLTEXT</p></template><template id=u></template></body></html>");
    run_scripts_in(&doc, &[src("document.getElementById('u').appendChild(document.createTextNode('OWNKID'));")], Rc::new(|_, _| {}));
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    assert!(texts.iter().any(|(t, _)| t.contains("shown")), "{texts:?}");
    assert!(!texts.iter().any(|(t, _)| t.contains("TPLTEXT") || t.contains("OWNKID")), "{texts:?}");
}

// ---- QSD0: fragment wrappers must not assign query methods per instance.

#[test]
fn qsd0_readonly_fragment_query_methods_do_not_break_wrappers() {
    let log = run_in_html(
        "<html><body><div id=a><p class=c>x</p></div></body></html>",
        r#"'use strict';
         Object.defineProperty(DocumentFragment.prototype, 'querySelectorAll',
           { value: function () { return ['page']; } });
         Object.freeze(DocumentFragment.prototype);
         const f = document.createDocumentFragment();
         const p = document.createElement('p');
         f.appendChild(p);
         console.log(f.querySelectorAll('p')[0], f.querySelector('p') === p, f.getElementsByTagName('p').length);
         console.log(document.querySelectorAll('p.c').length, document.getElementById('a').querySelector('.c').tagName);"#,
    );
    assert_eq!(log, ["log:page true 1", "log:1 P"], "{log:?}");
}


// ---------- QF13: all methods with bodies, CORS preflight + cache ----------

type PfHandler = Box<dyn Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync>;

/// Method-aware fake wire: records every raw request (in order) with its
/// read timeout; `handler(method, url, raw)` answers.
struct PfFake {
    seen: Mutex<Vec<(String, String, String, std::time::Duration)>>,
    handler: PfHandler,
}

impl PfFake {
    fn new(handler: impl Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(PfFake { seen: Mutex::new(Vec::new()), handler: Box::new(handler) })
    }
    /// `METHOD url` per request, in order.
    fn order(&self) -> Vec<String> {
        self.seen.lock().unwrap().iter().map(|(m, u, _, _)| format!("{m} {u}")).collect()
    }
    fn raw(&self, i: usize) -> String {
        self.seen.lock().unwrap()[i].2.clone()
    }
}

impl NetExecutor for PfFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        let raw = String::from_utf8_lossy(&job.request).to_string();
        let mut it = raw.lines().next().unwrap_or("").split(' ');
        let (m, u) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string());
        self.seen.lock().unwrap().push((m.clone(), u.clone(), raw.clone(), job.limits.read_timeout));
        (self.handler)(&m, &u, &raw)
    }
}

fn pf_resp(status: u16, headers: &[(&str, &str)], body: &str) -> Result<WireResponse, NetError> {
    Ok(WireResponse {
        status,
        status_text: if status == 204 { "No Content".into() } else { "OK".into() },
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.as_bytes().to_vec(),
        error: None,
        final_url: None,
    })
}

/// A cooperative CORS server for `https://page.test`: the preflight carries
/// `pre` (extra/override headers), everything else answers 200 with ACAO.
fn pf_server(pre: &'static [(&'static str, &'static str)]) -> impl Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync + 'static {
    move |m, _u, _raw| {
        if m == "OPTIONS" {
            pf_resp(204, pre, "")
        } else {
            pf_resp(200, &[("access-control-allow-origin", "https://page.test"), ("content-type", "text/plain")], "done")
        }
    }
}

const PF_ALLOW_ALL: &[(&str, &str)] = &[
    ("access-control-allow-origin", "https://page.test"),
    ("access-control-allow-methods", "POST, PUT, PATCH, DELETE"),
    ("access-control-allow-headers", "content-type, x-foo"),
];

fn pf_engine(page: &str, fake: &Arc<PfFake>, clock: Option<Rc<std::cell::Cell<f64>>>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts_clocked(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))), clock).unwrap();
    (engine, out)
}

fn pf_drive(engine: &JsEngine, fake: &Arc<PfFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

const PF_JSON_POST: &str = "fetch('https://api.test/x', {method:'POST', headers:{'Content-Type':'application/json','X-Foo':'1'}, body:'{\"k\":1}'})\
    .then(function(r){ return r.text(); }).then(function(t){ console.log('ok', t); }, function(e){ console.log('err', e.name, e.message); });";

#[test]
fn qf13_preflight_precedes_a_cross_origin_json_post_without_cookies() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", &PF_JSON_POST.replace("headers:", "credentials:'omit', headers:"));
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x", "POST https://api.test/x"]);
    let pre = fake.raw(0);
    assert!(pre.contains("Origin: https://page.test\r\n"), "{pre}");
    assert!(pre.contains("Access-Control-Request-Method: POST\r\n"), "{pre}");
    assert!(pre.contains("Access-Control-Request-Headers: content-type,x-foo\r\n"), "lowercased and sorted: {pre}");
    assert!(pre.contains("X-Wire-Cookies: off"), "{pre}");
    assert!(!pre.to_ascii_lowercase().contains("x-wire-jar"), "{pre}");
    assert!(!pre.to_ascii_lowercase().contains("content-length") && pre.ends_with("\r\n\r\n"), "no body: {pre}");
    let act = fake.raw(1);
    assert!(act.contains("{\"k\":1}") && act.contains("Origin: https://page.test"), "{act}");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:ok done".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:preflight OPTIONS https://api.test/x for POST (Access-Control-Request-Headers: content-type,x-foo)")), "{l:?}");
    let e = engine.net_entries();
    assert!(e.iter().any(|x| x.method == "OPTIONS (preflight)" && x.outcome.starts_with("204")), "{e:?}");
    let report = crate::js_net::format_net_report(&e);
    assert!(report.contains("OPTIONS (preflight) https://api.test/x -> 204"), "{report}");
}


#[test]
fn qf13_preflight_never_sends_the_jar_but_the_credentialed_request_does() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-credentials", "true"),
        ("access-control-allow-methods", "PUT"),
        ("access-control-allow-headers", "x-foo"),
    ]));
    let (engine, _log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('https://api.test/x', {method:'PUT', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.message); });",
    );
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x", "PUT https://api.test/x"], "{:?}", fake.order());
    assert!(!fake.raw(0).to_ascii_lowercase().contains("x-wire-jar"), "{}", fake.raw(0));
    assert!(fake.raw(0).contains("X-Wire-Cookies: off"));
    assert!(fake.raw(1).contains("X-Wire-Jar: scratchy"), "{}", fake.raw(1));
}

fn qf13_denied(pre: &'static [(&'static str, &'static str)], status: u16, code: &str) -> (Vec<String>, Vec<String>) {
    let fake = PfFake::new(move |m, _u, _r| {
        if m == "OPTIONS" {
            pf_resp(status, pre, "")
        } else {
            pf_resp(200, &[("access-control-allow-origin", "https://page.test")], "SECRET")
        }
    });
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", code);
    pf_drive(&engine, &fake);
    let l = log.borrow().clone();
    (fake.order(), l)
}

#[test]
fn qf13_each_missing_piece_denies_the_preflight_and_sends_nothing_else() {
    let cases: Vec<(&'static [(&'static str, &'static str)], u16, &str, &str)> = vec![
        // header missing
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type")], 204, PF_JSON_POST, "Access-Control-Allow-Headers missing 'x-foo'"),
        // method missing
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "GET"), ("access-control-allow-headers", "content-type, x-foo")], 204, "fetch('https://api.test/x', {method:'PUT', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Methods missing 'PUT'"),
        // origin missing / wrong
        (&[("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 204, PF_JSON_POST, "no Access-Control-Allow-Origin header"),
        (&[("access-control-allow-origin", "https://evil.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 204, PF_JSON_POST, "does not match the page origin"),
        // bad status
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 403, PF_JSON_POST, "preflight response has status 403"),
        // wildcard never covers Authorization
        (&[("access-control-allow-origin", "*"), ("access-control-allow-methods", "*"), ("access-control-allow-headers", "*")], 204, "fetch('https://api.test/x', {headers:{'Authorization':'t'}}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Headers missing 'authorization'"),
        // credentials: ACAO * with credentials, and ACAC missing
        (&[("access-control-allow-origin", "*"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "x-foo,content-type")], 204, "fetch('https://api.test/x', {method:'POST', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Origin is * but the request includes credentials"),
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "x-foo")], 204, "fetch('https://api.test/x', {method:'POST', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Credentials is not true"),
    ];
    for (pre, status, code, what) in cases {
        let (order, l) = qf13_denied(pre, status, code);
        assert_eq!(order, vec!["OPTIONS https://api.test/x"], "only the preflight is sent ({what}): {l:?}");
        assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://api.test/x: ") && x.contains(what)), "{what}: {l:?}");
        assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{what}: {l:?}");
        assert!(!l.iter().any(|x| x.contains("SECRET")), "{l:?}");
    }
}

#[test]
fn qf13_a_preflight_transport_error_fails_the_request_too() {
    let fake = PfFake::new(|_m, _u, _r| Err(NetError::Io("boom".into())));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://api.test/x: ") && x.contains("boom")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf13_cache_hit_skips_the_second_preflight_and_max_age_expires_on_the_clock() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-methods", "POST"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "10"),
    ]));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(9_000.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 1, "second request used the cache: {:?}", fake.order());
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("POST")).count(), 2);
    assert!(log.borrow().iter().any(|x| x.starts_with("net:preflight cache hit: OPTIONS https://api.test/x for POST")), "{:?}", log.borrow());
    let report = crate::js_net::format_net_report(&engine.net_entries());
    assert!(report.contains("OPTIONS (preflight) https://api.test/x -> cache hit"), "{report}");
    clock.set(10_500.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "expired after Max-Age: {:?}", fake.order());
}

#[test]
fn qf13_default_max_age_is_five_seconds_and_the_cap_comes_from_the_policy() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(4_900.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 1);
    clock.set(5_100.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "default 5 s");
    // a huge Max-Age is capped by the policy
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-methods", "POST"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "999999"),
    ]));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    let mut p = RequestPolicy::default();
    p.preflight_max_age_cap_secs = 20;
    engine.set_net_policy(p);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(21_000.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "cap of 20 s applied");
}

#[test]
fn qf13_cache_is_keyed_by_url_method_headers_and_credentials_and_bounded() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-credentials", "true"),
        ("access-control-allow-methods", "POST, PUT"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "100"),
    ]));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(Rc::new(std::cell::Cell::new(0.0))));
    let go = |url: &str, method: &str, cred: &str| {
        format!("fetch('{url}', {{method:'{method}', credentials:'{cred}', headers:{{'X-Foo':'1'}}, body:'b'}}).catch(function(){{}});")
    };
    engine.eval(
        "t",
        &[
            go("https://api.test/a", "POST", "omit"),
            go("https://api.test/a", "POST", "omit"),
            go("https://api.test/a", "PUT", "omit"),
            go("https://api.test/a", "POST", "include"),
            go("https://api.test/b", "POST", "omit"),
        ]
        .concat(),
    );
    pf_drive(&engine, &fake);
    // The first two POSTs are issued before the first preflight returns, so both
    // preflight; then PUT, include and /b are different keys.
    let pre = fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count();
    assert_eq!(pre, 5, "{:?}", fake.order());
    engine.eval("t", &go("https://api.test/a", "POST", "omit"));
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 5, "now cached");
    let mut c = crate::js_net::PreflightCache::default();
    let u = Url::parse("https://a.test/").unwrap();
    for i in 0..10 {
        let k = crate::js_net::PreflightCache::key("o", &u, &format!("M{i}"), &[], false);
        c.insert(k, 0.0, 10 + i, 4);
    }
    assert_eq!(c.len(), 4, "bounded by the policy's max entries");
    c.insert("zero".into(), 0.0, 0, 4);
    assert_eq!(c.len(), 4, "Max-Age 0 is not cached");
}

#[test]
fn qf13_put_patch_delete_with_bodies_reach_the_executor_after_their_preflights() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('https://api.test/r', {method:'PUT', body:'put-body'}).catch(function(e){ console.log('e1', e.message); });\
         fetch('https://api.test/r', {method:'PATCH', body:'patch-body'}).catch(function(e){ console.log('e2', e.message); });\
         fetch('https://api.test/r', {method:'DELETE', body:'del-body'}).catch(function(e){ console.log('e3', e.message); });\
         var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/x'); x.send('xhr-body');",
    );
    pf_drive(&engine, &fake);
    let seen = fake.seen.lock().unwrap().clone();
    for (m, body) in [("PUT", "put-body"), ("PATCH", "patch-body"), ("DELETE", "del-body")] {
        let (_, _, raw, _) = seen.iter().find(|(mm, u, _, _)| mm == m && u.ends_with("/r")).unwrap_or_else(|| panic!("{m} not sent: {seen:?}"));
        assert!(raw.ends_with(body), "{raw}");
    }
    assert!(seen.iter().any(|(m, u, r, _)| m == "PUT" && u.ends_with("/x") && r.ends_with("xhr-body")), "{seen:?}");
    // every actual request came after an OPTIONS for the same URL+method
    let opts = seen.iter().filter(|s| s.0 == "OPTIONS").count();
    assert!(opts >= 4, "{:?}", fake.order());
    assert!(!log.borrow().iter().any(|x| x.contains("blocked")), "{:?}", log.borrow());
}

#[test]
fn qf13_forbidden_methods_are_still_refused_before_any_preflight() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", "__net_request({method:'TRACE', url:'https://api.test/x', headers:[], body:null}).promise.catch(function(e){ console.log('err', e.name); });");
    pf_drive(&engine, &fake);
    assert!(fake.order().is_empty(), "{:?}", fake.order());
    assert!(log.borrow().iter().any(|x| x.starts_with("net:blocked TRACE")), "{:?}", log.borrow());
}

#[test]
fn qf13_same_origin_and_simple_requests_never_preflight() {
    let fake = PfFake::new(|_m, _u, _r| pf_resp(200, &[("access-control-allow-origin", "https://page.test")], "x"));
    let (engine, _log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('/same', {method:'PUT', headers:{'Content-Type':'application/json','X-Foo':'1'}, body:'{}'});\
         fetch('https://api.test/simple', {method:'POST', headers:{'Content-Type':'text/plain'}, body:'a'});\
         fetch('https://api.test/get');\
         var x = new XMLHttpRequest(); x.open('POST', 'https://api.test/form'); x.setRequestHeader('Content-Type', 'application/x-www-form-urlencoded'); x.send('a=b');",
    );
    pf_drive(&engine, &fake);
    let o = fake.order();
    assert_eq!(o.len(), 4, "{o:?}");
    assert!(o.iter().all(|x| !x.starts_with("OPTIONS")), "{o:?}");
}

#[test]
fn qf13_the_preflight_counts_toward_the_page_cap_and_the_in_flight_limits() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    let mut p = RequestPolicy::default();
    p.max_requests_per_page = 3;
    engine.set_net_policy(p);
    engine.eval(
        "t",
        "for (var i = 0; i < 3; i++) fetch('https://api.test/c' + i, {method:'PUT', body:'b'}).then(function(){ console.log('ok'); }, function(e){ console.log('err', e.message); });",
    );
    pf_drive(&engine, &fake);
    let o = fake.order();
    assert_eq!(o.len(), 3, "cap of 3 requests counts OPTIONS too: {o:?}");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("too many requests from this page (limit 3)")), "{l:?}");
    // in flight: a preflight is an ordinary job for the scheduler
    let mut sched = NetScheduler::new(&engine.net_policy());
    let (engine2, _l2) = pf_engine("https://page.test/", &fake, None);
    engine2.eval("t", "fetch('https://api.test/z', {method:'PUT', body:'b'});");
    let reqs = engine2.take_requests();
    assert_eq!(reqs.net.len(), 1, "only the preflight is queued first");
    for j in reqs.net {
        assert_eq!(j.reserve, 0, "a preflight reserves no body bytes");
        sched.enqueue(j);
    }
    assert_eq!(sched.counts().0, 1);
}

#[test]
fn qf13_aborting_during_the_preflight_sends_nothing() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", "var c = new AbortController(); fetch('https://api.test/a', {method:'PUT', body:'b', signal:c.signal}).catch(function(e){ console.log('err', e.name); }); c.abort();");
    pf_drive(&engine, &fake);
    assert!(fake.order().is_empty(), "{:?}", fake.order());
    assert!(log.borrow().contains(&"log:err AbortError".to_string()), "{:?}", log.borrow());
}

#[test]
fn qf13_xhr_fires_error_when_the_preflight_is_denied() {
    let fake = PfFake::new(pf_server(&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "GET")]));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/x');\
         ['load','error','loadend'].forEach(function(t){ x.addEventListener(t, function(){ console.log('ev', t, x.status, x.readyState); }); });\
         x.send('b');",
    );
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:ev error 0 4".to_string()) && !l.contains(&"log:ev load 0 4".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("CORS preflight failed: https://api.test/x: Access-Control-Allow-Methods missing 'PUT'")), "{l:?}");
}

#[test]
fn qf13_sync_xhr_preflights_inside_the_blocking_call_within_the_sync_budget() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('POST', 'https://api.test/s', false); x.setRequestHeader('X-Foo', '1');\
         x.send('sync-body'); console.log('after', x.status, x.responseText);",
    );
    // no driving at all: everything happened inside send()
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/s", "POST https://api.test/s"]);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:after 200 done".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("blocking the page for up to 5000 ms (including its preflight)")), "{l:?}");
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen[0].3 <= std::time::Duration::from_millis(5000) && seen[1].3 <= std::time::Duration::from_millis(5000), "{seen:?}");
    assert!(seen[1].2.contains("X-Wire-Timeout-Ms: "), "{}", seen[1].2);
    // denied: throws NetworkError, only the preflight was sent
    let fake = PfFake::new(pf_server(&[("access-control-allow-origin", "https://page.test")]));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/s', false);\
         try { x.send('b'); console.log('no throw'); } catch (e) { console.log('thrown', e.name); }",
    );
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/s"]);
    assert!(log.borrow().contains(&"log:thrown NetworkError".to_string()), "{:?}", log.borrow());
}

#[test]
fn qf13_a_redirect_of_a_preflighted_request_fails_closed_and_wire_is_told_not_to_follow() {
    let fake = PfFake::new(|m, _u, _r| {
        if m == "OPTIONS" {
            pf_resp(204, PF_ALLOW_ALL, "")
        } else {
            pf_resp(307, &[("location", "https://evil.test/steal"), ("access-control-allow-origin", "https://page.test")], "")
        }
    });
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert!(fake.raw(0).contains("X-Wire-Max-Redirects: 0") && fake.raw(1).contains("X-Wire-Max-Redirects: 0"));
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("of a preflighted request is not followed")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf13_a_preflight_that_redirects_fails() {
    let fake = PfFake::new(|_m, _u, _r| pf_resp(302, &[("location", "https://other.test/"), ("access-control-allow-origin", "*")], ""));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    assert!(log.borrow().iter().any(|x| x.contains("preflight response has status 302")), "{:?}", log.borrow());
}

#[test]
fn qf13_a_simple_cors_request_redirected_cross_origin_is_checked_against_origin_null() {
    // A GET that wire redirected from api.test to cdn.test: the final answer
    // only allows the page origin, which a browser (Origin: null after the
    // hop) would not accept.
    let mk = |acao: &'static str| {
        PfFake::new(move |_m, _u, _r| {
            Ok(WireResponse {
                status: 200,
                status_text: "OK".into(),
                headers: vec![("access-control-allow-origin".into(), acao.into())],
                body: b"x".to_vec(),
                error: None,
                final_url: Some("https://cdn.test/y".into()),
            })
        })
    };
    for (acao, ok) in [("https://page.test", false), ("null", true), ("*", true)] {
        let fake = mk(acao);
        let (engine, log) = pf_engine("https://page.test/", &fake, None);
        engine.eval("t", "fetch('https://api.test/x').then(function(){ console.log('ok'); }, function(e){ console.log('err', e.message); });");
        pf_drive(&engine, &fake);
        let l = log.borrow().clone();
        assert_eq!(l.contains(&"log:ok".to_string()), ok, "{acao}: {l:?}");
    }
}

#[test]
fn qf13_navigator_send_beacon_exists_logs_and_returns_true() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "console.log('t', typeof navigator.sendBeacon, navigator.sendBeacon('https://track.test/b', 'hi'), (function () { try { navigator.sendBeacon('ftp://x.test/'); return 'no-throw'; } catch (e) { return e.name; } })());",
    );
    pf_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:t function true TypeError".to_string()), "{l:?}");
    assert!(fake.order().is_empty(), "beacons are suppressed by default");
    assert!(l.iter().any(|x| x.starts_with("net:beacon navigator.sendBeacon POST https://track.test/b")), "{l:?}");
}

// ---- QSD1: document.domain, Document as ParentNode, getElementsByName.

#[test]
fn qsd1_document_domain_and_parent_node() {
    set_page_url(Url::parse("https://gist.github.com/a/b").ok().as_ref());
    let log = run_in_html(
        "<html><body><input name=n><p name=n></p><i name=m></i></body></html>",
        r#"const r = [];
         r.push(document.domain);
         document.domain = 'gist.github.com'; r.push(document.domain);
         document.domain = 'github.com'; r.push(document.domain);
         for (const bad of ['com', 'example.org', 'ub.com']) {
           try { document.domain = bad; r.push('no throw'); } catch (e) { r.push(e.name + ':' + (e instanceof DOMException)); }
         }
         r.push(document.firstElementChild.tagName, document.lastElementChild.tagName, document.childElementCount, document.children.length, document.children[0] === document.documentElement);
         const g = document.getElementsByName('n');
         r.push(g.length, g[0].tagName, document.getElementsByName('zz').length);
         console.log(r.join('|'));"#,
    );
    set_page_url(None);
    assert_eq!(
        log,
        ["log:gist.github.com|gist.github.com|github.com|SecurityError:true|SecurityError:true|SecurityError:true|HTML|HTML|1|1|true|2|INPUT|0"],
        "{log:?}"
    );
}

// ---- QSD1b: crypto.getRandomValues / randomUUID / subtle placeholder / isSecureContext.

#[test]
fn qsd1b_crypto_random_values_uuid_and_subtle_placeholder() {
    set_page_url(Url::parse("https://github.com/").ok().as_ref());
    let log = run_in_html(
        "<html><body></body></html>",
        r#"const r = [];
         r.push(window.crypto === globalThis.crypto, crypto instanceof Crypto, crypto.subtle instanceof SubtleCrypto, isSecureContext);
         const a = new Uint8Array(64); const b = crypto.getRandomValues(a);
         r.push(a === b, a.some(function (x) { return x !== 0; }));
         const u = new Uint32Array(4); crypto.getRandomValues(u); r.push(u.some(function (x) { return x !== 0; }));
         const id1 = crypto.randomUUID(), id2 = crypto.randomUUID();
         r.push(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id1), id1 !== id2);
         try { crypto.getRandomValues(new Float32Array(2)); r.push('no'); } catch (e) { r.push(e.name + ':' + (e instanceof DOMException)); }
         try { crypto.getRandomValues(new Uint8Array(65537)); r.push('no'); } catch (e) { r.push(e.name); }
         try { crypto.getRandomValues(new Uint8Array(65536)); r.push('ok'); } catch (e) { r.push(e.name); }
         try { crypto.getRandomValues([1, 2]); r.push('no'); } catch (e) { r.push(e.name); }
         console.log(r.join('|'));
         crypto.subtle.digest('SHA-256', new Uint8Array(1)).then(function () { console.log('resolved'); }, function (e) { console.log('rejected:' + e.name); });"#,
    );
    set_page_url(None);
    assert_eq!(log.len(), 3, "{log:?}");
    assert_eq!(
        log[0],
        "log:true|true|true|true|true|true|true|true|true|TypeMismatchError:true|QuotaExceededError|ok|TypeMismatchError"
    );
    assert!(log[1].starts_with("error:") && log[1].contains("not implemented: SubtleCrypto.digest"), "{log:?}");
    assert_eq!(log[2], "log:rejected:NotSupportedError");
}

#[test]
fn qsd1b_is_secure_context_follows_scheme_and_loopback() {
    let mut got = Vec::new();
    for u in ["https://example.org/", "http://example.org/", "http://localhost:8080/", "http://127.0.0.1/", "http://[::1]/"] {
        set_page_url(Url::parse(u).ok().as_ref());
        got.push(run_in_html("<html><body></body></html>", "console.log(String(isSecureContext))").remove(0));
    }
    set_page_url(None);
    assert_eq!(got, ["log:true", "log:false", "log:true", "log:true", "log:true"]);
}

// ---------- QF10: navigator.sendBeacon (fake executor, no network) ----------

#[test]
fn qf10_send_beacon_exists_on_navigator_and_prototype() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "console.log(typeof navigator.sendBeacon, typeof Navigator, Navigator.prototype.sendBeacon === navigator.sendBeacon, navigator instanceof Navigator, navigator.userAgent.length > 0);",
    );
    assert_eq!(l, vec!["log:function function true true true"], "{l:?}");
}

#[test]
fn qf10_send_beacon_is_logged_not_sent_and_returns_true() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/dir/",
        &fake,
        "console.log('r', navigator.sendBeacon('t?x=1', 'hello'));\
         console.log('u', navigator.sendBeacon('https://b.test/u', new URLSearchParams({a:'b c'})));\
         console.log('n', navigator.sendBeacon('https://b.test/n'));\
         console.log('a', navigator.sendBeacon('https://b.test/a', new Uint8Array([1,2,3])));\
         console.log('b', navigator.sendBeacon('https://b.test/b', new Blob(['{}'], {type:'application/json'})));\
         var fd = new FormData(); fd.append('k','v'); console.log('f', navigator.sendBeacon('https://b.test/f', fd));",
    );
    assert!(fake.urls().is_empty(), "suppressed beacons never reach the transport");
    for t in ["r", "u", "n", "a", "b", "f"] {
        assert!(l.contains(&format!("log:{t} true")), "{t}: {l:?}");
    }
    let net: Vec<&String> = l.iter().filter(|x| x.starts_with("net:beacon")).collect();
    assert_eq!(net.len(), 6, "{l:?}");
    assert!(net[0].contains("navigator.sendBeacon POST https://page.test/dir/t?x=1 content-type=text/plain;charset=UTF-8 body=5 bytes body[..200]=\"hello\" page=https://page.test/dir/"), "{}", net[0]);
    assert!(net[0].ends_with("suppressed (not sent)"), "{}", net[0]);
    assert!(net[1].contains("content-type=application/x-www-form-urlencoded;charset=UTF-8 body=5 bytes"), "{}", net[1]);
    assert!(net[2].contains("body=0 bytes"), "{}", net[2]);
    assert!(net[3].contains("body=3 bytes"), "{}", net[3]);
    assert!(net[4].contains("content-type=application/json body=2 bytes"), "{}", net[4]);
    assert!(net[5].contains("content-type=multipart/form-data; boundary="), "{}", net[5]);
}

#[test]
fn qf10_send_beacon_validates_the_url_and_the_size() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "['ftp://x.test/', 'javascript:1', 'data:text/plain,hi', 'http://[bad'].forEach(function (u) {\
           try { navigator.sendBeacon(u); console.log('no-throw', u); } catch (e) { console.log('thrown', e.name); }\
         });\
         try { navigator.sendBeacon(); } catch (e) { console.log('noarg', e.name); }\
         console.log('big', navigator.sendBeacon('https://b.test/x', new Uint8Array(65 * 1024)));\
         console.log('ok', navigator.sendBeacon('https://b.test/x', new Uint8Array(64 * 1024)));",
    );
    assert_eq!(l.iter().filter(|x| *x == "log:thrown TypeError").count(), 4, "{l:?}");
    assert!(l.contains(&"log:noarg TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:big false".to_string()), "{l:?}");
    assert!(l.contains(&"log:ok true".to_string()), "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("net:beacon")).count(), 1, "an oversized beacon is not even logged as sent: {l:?}");
}

#[test]
fn qf10_send_beacon_goes_through_the_policy_and_the_beacons_send_flag() {
    let fake = QfFake::new(vec![("track.test", qf7_ok("", &[])), ("127.0.0.1", qf7_ok("", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.beacons_send = true;
    engine.set_net_policy(p);
    engine.eval("t", "console.log(navigator.sendBeacon('https://track.test/b', 'hi')); console.log(navigator.sendBeacon('http://127.0.0.1:9/p', 'x'));");
    qf7_drive(&engine, &fake);
    let urls = fake.urls();
    assert!(urls.iter().any(|u| u.contains("track.test/b")), "beacons_send sends through the same executor: {urls:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    assert!(raw.contains("POST") && raw.to_ascii_lowercase().contains("content-type: text/plain;charset=utf-8"), "{raw}");
    let l = log.borrow().clone();
    assert!(!l.iter().any(|x| x.starts_with("net:beacon")), "{l:?}");
}

// ---- QSD2: getRootNode, isConnected, compareDocumentPosition.

#[test]
fn qsd2_get_root_node_and_is_connected() {
    let log = run_in_html(
        "<html><body><div id=a><p id=b>x</p></div><template id=t><i id=ti></i></template></body></html>",
        r#"const r = [];
         const a = document.getElementById('a'), b = document.getElementById('b');
         r.push(b.getRootNode() === document, b.getRootNode({ composed: true }) === document, document.getRootNode() === document, b.isConnected, document.isConnected);
         const d = document.createElement('div'), s = document.createElement('span'); d.appendChild(s);
         r.push(s.getRootNode() === d, d.getRootNode() === d, s.isConnected, d.isConnected);
         const f = document.createDocumentFragment(), q = document.createElement('q'); f.appendChild(q);
         r.push(q.getRootNode() === f, f.getRootNode() === f, q.isConnected, f.isConnected);
         document.body.appendChild(d);
         r.push(s.getRootNode() === document, s.isConnected);
         const ti = document.getElementById('t').content.firstChild, tc = document.getElementById('t').content;
         r.push(ti.getRootNode() === tc, ti.isConnected, document.getElementById('ti'));
         const d2 = document.implementation.createHTMLDocument('t');
         r.push(d2.body.getRootNode() === d2, d2.body.isConnected, d2.createElement('u').isConnected);
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log,
        ["log:true|true|true|true|true|true|true|false|false|true|true|false|false|true|true|true|false||true|true|false"],
        "{log:?}"
    );
}

#[test]
fn qsd2_compare_document_position() {
    let log = run_in_html(
        "<html><body><div id=a><p id=b>x</p><p id=c>y</p></div><span id=e></span></body></html>",
        r#"const r = [], $ = function (i) { return document.getElementById(i); };
         const a = $('a'), b = $('b'), c = $('c'), e = $('e');
         r.push(a.compareDocumentPosition(a), a.compareDocumentPosition(b), b.compareDocumentPosition(a));
         r.push(b.compareDocumentPosition(c), c.compareDocumentPosition(b), b.compareDocumentPosition(e), e.compareDocumentPosition(b));
         r.push(document.compareDocumentPosition(b), b.compareDocumentPosition(document));
         const x = document.createElement('x'), y = document.createElement('y');
         const dx = x.compareDocumentPosition(y), dy = y.compareDocumentPosition(x);
         r.push((dx & 1) !== 0, (dx & 32) !== 0, dx === 37 || dx === 35, (dx & 6) !== (dy & 6));
         r.push(b.compareDocumentPosition(x) & 33, Node.DOCUMENT_POSITION_CONTAINED_BY, Node.prototype.DOCUMENT_POSITION_PRECEDING);
         try { a.compareDocumentPosition(null); r.push('no'); } catch (err) { r.push(err.name); }
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log,
        ["log:0|20|10|4|2|4|2|20|10|true|true|true|true|33|16|2|TypeError"],
        "{log:?}"
    );
}

// ---------- QF7a: fail closed on an old wire, target-class warnings ----------

#[test]
fn qf7a_old_wire_fails_closed_with_one_log_line() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[])), ("/b", qf7_ok("BBB", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval("t", &format!("{}{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", ""), qf7_go("https://page.test/c", "")));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| *x == "net-warning:scratchy: page networking disabled: wire is too old (restart it)").count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("log:err")).count(), 3, "every request fails: {l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:ok")), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("net:")), "no further log lines: {l:?}");
    // Requests sent in the same batch are already in flight; a later one is refused before the executor.
    engine.eval("t2", &qf7_go("https://page.test/d", ""));
    qf7_drive(&engine, &fake);
    assert!(!fake.urls().iter().any(|u| u.ends_with("/d")), "nothing is sent after the old wire was found: {:?}", fake.urls());
    assert_eq!(log.borrow().iter().filter(|x| x.starts_with("log:err")).count(), 4);
    assert!(engine.net_entries().iter().all(|e| e.outcome.starts_with("blocked: page networking disabled")), "{:?}", engine.net_entries());
}

#[test]
fn qf7a_wire_v2_and_v3_keep_networking_working() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[("x-wire-version", "2")])), ("/b", qf7_ok("BBB", &[("x-wire-version", "3"), ("x-wire-cookies", "on")]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval("t", &format!("{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", "")));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:ok 200 basic AAA", "log:ok 200 basic BBB"], "{l:?}");
}

#[test]
fn qf7a_wire_control_headers_are_hidden_from_the_page() {
    let fake = QfFake::new(vec![("/a", qf7_ok("x", &[("x-wire-version", "3"), ("x-wire-target-class", "public"), ("x-custom", "1")]))]);
    let l = qf8_run("https://page.test/", &fake, "fetch('/a').then(function(r){ console.log('h', r.headers.get('x-wire-version'), r.headers.get('x-wire-target-class'), r.headers.get('x-custom')); });");
    assert!(l.contains(&"log:h null null 1".to_string()), "{l:?}");
}

#[test]
fn qf7a_dns_name_resolving_privately_warns_once() {
    let fake = QfFake::new(vec![
        ("rebind.test", qf7_ok("x", &[("x-wire-target-class", "private")])),
        ("fine.test", qf7_ok("x", &[("x-wire-target-class", "public")])),
        ("127.0.0.1", qf7_ok("x", &[("x-wire-target-class", "loopback")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://rebind.test/x'); fetch('https://fine.test/x'); fetch('http://127.0.0.1:8080/x');",
    );
    let w: Vec<&String> = l.iter().filter(|x| x.starts_with("net-warning:")).collect();
    assert_eq!(w.len(), 2, "{l:?}");
    assert!(w.iter().any(|x| x.contains("https://rebind.test/x (private) allowed by policy")), "{w:?}");
    assert_eq!(w.iter().filter(|x| x.contains("127.0.0.1")).count(), 1, "a literal IP is warned about by the URL check only: {w:?}");
}

#[test]
fn qf7a_block_mode_turns_wire_policy_refusal_into_a_clear_error() {
    let refusal = Ok(WireResponse {
        status: 403,
        status_text: "Forbidden".into(),
        error: Some("wire: blocked: rebind.test resolves to a non-public address (10.0.0.5)".into()),
        ..Default::default()
    });
    let fake = QfFake::new(vec![("rebind.test", refusal)]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.private_targets = PrivatePolicy::Block;
    engine.set_net_policy(p);
    engine.eval("t", &qf7_go("https://rebind.test/x", ""));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:") && x.contains("rebind.test resolves to a non-public address (private_targets is block)")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf7a_suppressed_beacons_and_refusals_do_not_trip_the_old_wire_check() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval(
        "t",
        "navigator.sendBeacon('https://track.test/b', 'x'); fetch('ftp://x.test/').catch(function(){ console.log('refused'); });\
         fetch('https://track.test/k', {method:'POST', keepalive:true, body:'y'}).then(function(r){ console.log('ka', r.type); });",
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(!l.iter().any(|x| x.contains("page networking disabled")), "{l:?}");
    assert!(l.contains(&"log:ka opaque".to_string()), "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("net:beacon")).count(), 2, "{l:?}");
}

// ---------- QSD4: constructable stylesheets / adoptedStyleSheets ----------

/// Runs `code`, then builds the layout tree the way the App does (page CSS
/// plus the engine's published adopted CSS) and returns the first text colour.
fn qsd4_color(html: &str, code: &str) -> (Option<TextColor>, Vec<String>, String) {
    let (engine, doc, log) = events_engine(html, code);
    let adopted = engine.take_requests().adopted_css.unwrap_or_default();
    let css = crate::images::with_adopted_css("", &adopted).into_owned();
    let tree = build_layout_tree_with_resources(&doc, 800.0, None, &css, &HashMap::new());
    let l = log.borrow().clone();
    (find_first_span_color(&tree), l, adopted)
}

const QSD4_HTML: &str = "<html><head><style>p{color:#0000ff}</style></head><body><p id=p>x</p></body></html>";

#[test]
fn qsd4_adopted_sheet_changes_used_colour_after_style() {
    let (c, l, _) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
    let (c, _, a) = qsd4_color(QSD4_HTML, "");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    assert_eq!(a, "", "zero cost when unused");
}

#[test]
fn qsd4_important_and_layer_interplay() {
    let html = "<html><head><style>p{color:#0000ff !important}</style></head><body><p>x</p></body></html>";
    let (c, ..) = qsd4_color(html, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, ..) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000 !important}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)));
    let (c, ..) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('@layer a { p{color:#ff0000} }'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
}

#[test]
fn qsd4_adoption_order_insert_delete_and_clear() {
    let (c, l, _) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(), b=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); b.replaceSync('p{color:#00ff00}'); document.adoptedStyleSheets=[a,b];");
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "later adopted sheet wins: {l:?}");
    let (c, l, _) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[a]; a.insertRule('p{color:#00ff00}',1); console.log(a.cssRules.length);");
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "{l:?}");
    assert_eq!(l[0], "log:2");
    let (c, ..) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000} p{color:#00ff00}'); document.adoptedStyleSheets=[a]; a.deleteRule(1);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)));
    let (c, _, a) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[a]; document.adoptedStyleSheets=[];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    assert_eq!(a, "");
    let (c, ..) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets.push(a); a.disabled=true;");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)), "disabled sheet does not apply");
}

#[test]
fn qsd4_api_surface() {
    let (_e, _d, log) = events_engine(
        QSD4_HTML,
        "var s=new CSSStyleSheet({media:'print',disabled:false});\
         s.replaceSync('/*c*/ @import url(x.css); a, b { color: red; margin: 0 !important } @media (min-width:1px){p{top:0}}');\
         var r=s.cssRules; console.log(r.length, r.item(0)===r[0], r[0].selectorText, r[0].type, r[0].style.color, r[0].style.getPropertyValue('margin'), r[0].style.getPropertyPriority('margin'));\
         console.log(r[0].cssText, '|', r[1].type);\
         console.log(s.media.mediaText, s.ownerNode, s.href, s.type, s.disabled, s.rules===s.cssRules);\
         try { document.adoptedStyleSheets = [{}]; } catch (e) { console.log('te', e instanceof TypeError); }\
         try { new CSSStyleSheet().insertRule('a{}', 5); } catch (e) { console.log(e.name); }\
         try { new CSSStyleSheet().insertRule('@import url(a.css)'); } catch (e) { console.log(e.name); }\
         try { new StyleSheet(); } catch (e) { console.log('ill', e instanceof TypeError); }\
         console.log(typeof CSSRule, typeof CSSRuleList, typeof CSSStyleRule, typeof MediaList, s instanceof StyleSheet, r[0] instanceof CSSStyleRule, r instanceof CSSRuleList);\
         var t=new CSSStyleSheet(); t.replace('p{color:red}').then(function(x){ console.log('rep', x===t, x.cssRules.length); });",
    );
    let l = log.borrow();
    assert!(l[0].contains("@import rules are not allowed"), "{l:?}");
    assert!(l[1].ends_with("2 true a, b 1 red 0 important"), "{l:?}");
    assert!(l[2].ends_with("a, b { color: red; margin: 0 !important; } | 4"), "{l:?}");
    assert!(l[3].ends_with("print null null text/css false true"), "{l:?}");
    assert!(l[4].ends_with("te true"), "{l:?}");
    assert!(l[5].ends_with("IndexSizeError"), "{l:?}");
    assert!(l[6].ends_with("SyntaxError"), "{l:?}");
    assert!(l[7].ends_with("ill true"), "{l:?}");
    assert!(l[8].ends_with("function function function function true true true"), "{l:?}");
    assert!(l[9].ends_with("rep true 1"), "{l:?}");
}

#[test]
fn qsd4_get_computed_style_sees_adopted_sheets() {
    let (_e, _d, log) = events_engine(
        QSD4_HTML,
        "var p=document.getElementById('p'); console.log(getComputedStyle(p).color);\
         var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];\
         console.log(getComputedStyle(p).color);\
         s.replaceSync('p{color:#00ff00}'); console.log(getComputedStyle(p).color);\
         document.adoptedStyleSheets=[]; console.log(getComputedStyle(p).color);",
    );
    let l = log.borrow();
    assert_eq!(
        *l,
        ["log:rgb(0, 0, 255)", "log:rgb(255, 0, 0)", "log:rgb(0, 255, 0)", "log:rgb(0, 0, 255)"],
        "{l:?}"
    );
}

/// Every command-line option a parser accepts must be documented on the
/// `:help` page, and `CLI_OPTIONS` must list every `"--option"` literal the
/// parsers (dom.rs `parse_args`, js.rs `--js-report`, wpt.rs `--wpt`, lib.rs
/// dispatch) mention, so a new option cannot slip past this test.
#[test]
fn cli_options_are_documented_in_help() {
    for opt in crate::dom::CLI_OPTIONS {
        assert!(HELP_PAGE_HTML.contains(opt), "`{opt}` is a command-line option but is not documented in src/help.html");
    }
    for (file, src) in [
        ("dom.rs", include_str!("dom.rs")),
        ("js.rs", include_str!("js.rs")),
        ("wpt.rs", include_str!("wpt.rs")),
        ("lib.rs", include_str!("lib.rs")),
    ] {
        let mut rest = src;
        while let Some(i) = rest.find("\"--") {
            rest = &rest[i + 1..];
            let lit: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            let closed = rest[lit.len()..].starts_with('"');
            if closed && lit[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                assert!(crate::dom::CLI_OPTIONS.contains(&lit.as_str()), "{file} accepts `{lit}`, which is missing from dom::CLI_OPTIONS");
            }
        }
    }
}

/// Lays out an already-parsed (and possibly script-mutated) document; the
/// returned height is the document height.
fn qw5_doc_height(doc: &Handle) -> f32 {
    let tree = build_layout_tree(doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, 500.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete).h
}

#[test]
fn qw5_hidden_attribute_is_display_none_author_display_wins() {
    let h = |body: &str| qw5_doc_height(&parse_html(&format!("<html><body style=\"margin:0\">{body}</body></html>")));
    let base = h("<div style=\"height:20px\"></div>");
    assert!(base >= 20.0);
    assert_eq!(h("<div style=\"height:20px\"></div><p hidden style=\"height:50px\">x</p>"), base, "hidden p is not laid out");
    assert_eq!(h("<div style=\"height:20px\"></div><p hidden=\"\" id=a class=tags>x</p>"), base);
    // author display wins over the UA rule
    assert!(h("<div style=\"height:20px\"></div><p hidden style=\"display:block;height:50px;margin:0\">x</p>") >= base + 50.0);
    // until-found is not display:none from this rule
    assert!(h("<p hidden=\"until-found\" style=\"height:50px;margin:0\">x</p>") >= 50.0);
    assert_eq!(h("<div style=\"height:20px\"></div><datalist><option>a</option></datalist><noembed>zz</noembed>"), base);
}

#[test]
fn qw5_js_hidden_toggle_relayouts_and_node_stays_in_dom() {
    let doc = parse_html("<html><body style=\"margin:0\"><p id=t hidden style=\"margin:0;height:40px\"><a href=\"/x\">x</a></p></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(
        &doc,
        &[src("var t=document.getElementById('t'); console.log(t.hidden, document.querySelectorAll('#t a[href]').length);\
                window.show=function(){t.hidden=false;}; window.hide=function(){t.hidden=true;};")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
    )
    .expect("engine");
    assert_eq!(out.borrow().as_slice(), ["log:true 1"]);
    assert_eq!(qw5_doc_height(&doc), 0.0, "hidden: nothing laid out");
    engine.eval("t", "show()");
    assert!(qw5_doc_height(&doc) >= 40.0, "after hidden=false it is laid out");
    engine.eval("t", "hide()");
    assert_eq!(qw5_doc_height(&doc), 0.0);
}

/// QW4: horizontal margin/padding on non-replaced inline elements advance
/// the line: three adjacent links differ in x by text width + margin +
/// padding on both sides.
#[test]
fn qw4_inline_margin_and_padding_advance_adjacent_links() {
    let html = r##"<!doctype html><meta charset=utf-8><style>a{margin-right:2rem;padding:0 1rem;background:#cde}</style>
<p><a href="#">one</a><a href="#">two</a><a href="#">three</a></p>"##;
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let words: Vec<(f32, f32)> = mixed_items(run)
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Word { x, w, .. } => Some((*x, *w)),
            _ => None,
        })
        .collect();
    assert_eq!(words.len(), 3, "three words");
    for pair in words.windows(2) {
        // end padding 16 + end margin 32 + next start padding 16 = 64
        let gap = pair[1].0 - (pair[0].0 + pair[0].1);
        assert!((gap - 64.0).abs() < 0.5, "gap between links should be 64px, got {gap}");
    }
    let spacers: Vec<f32> = mixed_items(run)
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Embedded(l) if l.style.background_color.is_some() => Some(l.w),
            _ => None,
        })
        .collect();
    assert!(spacers.iter().all(|w| (*w - 16.0).abs() < 0.5) && spacers.len() == 6, "padding backgrounds: {spacers:?}");
}

/// QW4: on a wrapped inline element the margin applies at the start of the
/// first fragment and the end of the last, not at every line.
#[test]
fn qw4_inline_margin_wraps_only_at_outer_edges() {
    let html = r##"<style>a{margin:0 20px}</style><div style="width:120px"><a href="#">aaaa bbbb cccc dddd</a></div>"##;
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let words: Vec<(f32, f32)> = mixed_items(run)
        .iter()
        .filter_map(|i| if let MixedPositioned::Word { x, y, .. } = i { Some((*x, *y)) } else { None })
        .collect();
    assert!(words.len() >= 2);
    assert!((words[0].0 - 28.0).abs() < 0.5, "first fragment starts after margin-left: {words:?}");
    let second_line_x = words.iter().filter(|p| (p.1 - words[0].1).abs() > 1.0).map(|p| p.0).fold(f32::MAX, f32::min);
    assert!((second_line_x - 8.0).abs() < 0.5, "wrapped line has no margin at its start: {second_line_x} {words:?}");
}

fn qy18_find_box(l: &Laid, w: f32, h: f32) -> Option<(f32, f32)> {
    if (l.w - w).abs() < 0.01 && (l.h - h).abs() < 0.01 {
        return Some((l.x, l.y));
    }
    let kids: Vec<&Laid> = match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().collect(),
        _ => Vec::new(),
    };
    kids.into_iter().chain(l.positioned.iter()).find_map(|k| qy18_find_box(k, w, h))
}

#[test]
fn qy18_transform_parses_translate_only() {
    let t = |s: &str| parse_declarations(s).transform;
    assert!(t("transform: none") == Some(vec![]));
    assert!(t("transform: translateX(100%)") == Some(vec![(Length::Percent(1.0), Length::Px(0.0))]));
    assert!(t("transform: translate(10px, 20px)") == Some(vec![(Length::Px(10.0), Length::Px(20.0))]));
    assert!(t("transform: translate(5px)") == Some(vec![(Length::Px(5.0), Length::Px(0.0))]));
    assert!(t("transform: translateY(-50%)") == Some(vec![(Length::Px(0.0), Length::Percent(-0.5))]));
    assert!(t("transform: translate3d(1px, 2px, 3px)") == Some(vec![(Length::Px(1.0), Length::Px(2.0))]));
    // Anything with an unsupported function is treated as none.
    assert!(t("transform: rotate(45deg)") == Some(vec![]));
    assert!(t("transform: translateX(10px) scale(2)") == Some(vec![]));
    assert!(t("transform: bogus").is_none());
}

#[test]
fn qy18_translate_offsets_paint_position_and_hit_test() {
    let base = measure_fixture(
        r#"<html><body style="margin:0"><div style="position:relative;width:400px;height:200px"><div style="position:absolute;right:0;top:0;width:100px;height:50px;background:red">x</div></div></body></html>"#,
        400.0,
    );
    let (bx, by) = qy18_find_box(&base, 100.0, 50.0).expect("box");
    // 100% of its own width pushes it fully off the right edge of the 400px viewport.
    let moved = measure_fixture(
        r#"<html><body style="margin:0"><div style="position:relative;width:400px;height:200px"><div style="position:absolute;right:0;top:0;width:100px;height:50px;background:red;transform:translateX(100%)">x</div></div></body></html>"#,
        400.0,
    );
    let (mx, my) = qy18_find_box(&moved, 100.0, 50.0).expect("box");
    assert!((mx - (bx + 100.0)).abs() < 0.01 && (my - by).abs() < 0.01, "{bx},{by} -> {mx},{my}");
    assert!(mx >= 400.0, "drawer is off-screen");
    // translate(10px,20px) on an in-flow box moves it by exactly (10,20) and not the flow.
    let html = |t: &str| format!(
        r#"<html><body style="margin:0"><div id="a" style="width:100px;height:50px;{t}">a</div><div id="b" style="width:60px;height:30px">b</div></body></html>"#
    );
    let plain = measure_fixture(&html(""), 400.0);
    let tr = measure_fixture(&html("transform:translate(10px,20px)"), 400.0);
    let (px, py) = qy18_find_box(&plain, 100.0, 50.0).unwrap();
    let (tx, ty) = qy18_find_box(&tr, 100.0, 50.0).unwrap();
    assert!((tx - px - 10.0).abs() < 0.01 && (ty - py - 20.0).abs() < 0.01);
    let (p2x, p2y) = qy18_find_box(&plain, 60.0, 30.0).unwrap();
    let (t2x, t2y) = qy18_find_box(&tr, 60.0, 30.0).unwrap();
    assert!((p2x - t2x).abs() < 0.01 && (p2y - t2y).abs() < 0.01, "layout of siblings is unchanged");
    // Hit test follows the offset: the new spot hits <a>, the old corner no longer does.
    let doc = parse_html(&html("transform:translate(10px,20px)"));
    let tree = build_layout_tree(&doc, 1920.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let a = find_by_tag(&doc, "div").unwrap();
    let hit_new = hit_test_element(&laid, 10.0 + 90.0, 20.0 + 40.0, &cache, &HashMap::new(), 0.0, 600.0);
    assert_eq!(hit_new, node_id(&a), "the translated position hits the element");
    let hit_old = hit_test_element(&laid, 3.0, 3.0, &cache, &HashMap::new(), 0.0, 600.0);
    assert_ne!(hit_old, node_id(&a), "the original top-left corner no longer hits it");
}

// ---- QSD3: real ShadowRoot (not rendered).

#[test]
fn qsd3_attach_shadow_basics() {
    let log = run_in_html(
        "<html><body><div id=h><b id=l>light</b></div><i id=bad></i></body></html>",
        r#"const r = [], h = document.getElementById('h');
         const sr = h.attachShadow({ mode: 'open', delegatesFocus: true });
         r.push(sr instanceof ShadowRoot, sr instanceof DocumentFragment, sr.nodeType, sr.host === h, sr.mode, sr.delegatesFocus, h.shadowRoot === sr, sr.activeElement);
         try { h.attachShadow({ mode: 'open' }); r.push('no'); } catch (e) { r.push(e.name); }
         try { document.getElementById('bad').attachShadow({ mode: 'open' }); r.push('no'); } catch (e) { r.push(e.name); }
         try { h.attachShadow({ mode: 'x' }); r.push('no'); } catch (e) { r.push(e.name); }
         const c = document.createElement('x-c'), cs = c.attachShadow({ mode: 'closed' });
         r.push(c.shadowRoot, cs.mode, cs.host === c);
         sr.innerHTML = '<p id=in>hi</p><span class=k>k</span>';
         r.push(sr.getElementById('in').localName, sr.querySelector('.k').localName, sr.querySelectorAll('*').length);
         r.push(document.querySelector('#in'), document.getElementById('in'), h.innerHTML, h.children.length, h.childNodes.length);
         r.push(sr.getRootNode() === sr, sr.firstChild.getRootNode() === sr, sr.getRootNode({ composed: true }) === document, sr.firstChild.getRootNode({ composed: true }) === document);
         r.push(sr.isConnected, sr.firstChild.isConnected, cs.isConnected);
         r.push(sr.adoptedStyleSheets.length);
         const sh = new CSSStyleSheet(); sr.adoptedStyleSheets = [sh]; r.push(sr.adoptedStyleSheets[0] === sh);
         try { sr.adoptedStyleSheets = [{}]; r.push('no'); } catch (e) { r.push(e.name); }
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log.last().map(String::as_str),
        Some("log:true|true|11|true|open|true|true||NotSupportedError|NotSupportedError|TypeError||closed|true|p|span|2|||<b id=\"l\">light</b>|1|1|true|true|true|true|true|true|false|0|true|TypeError"),
        "{log:?}"
    );
    assert_eq!(log.iter().filter(|l| l.contains("shadow root rendering")).count(), 1, "{log:?}");
}

#[test]
fn qsd3_custom_element_in_shadow_and_event() {
    let log = run_in_html(
        "<html><body><div id=h></div></body></html>",
        r#"class XC extends HTMLElement { connectedCallback() { console.log('conn'); } disconnectedCallback() { console.log('disc'); } }
         customElements.define('x-c', XC);
         const h = document.getElementById('h'), sr = h.attachShadow({ mode: 'open' });
         const x = document.createElement('x-c'); sr.appendChild(x);
         const seen = [];
         h.addEventListener('ping', function (e) { seen.push('host:' + e.target.localName); });
         document.addEventListener('ping', function () { seen.push('doc'); });
         x.dispatchEvent(new Event('ping', { bubbles: true, composed: true }));
         x.dispatchEvent(new Event('ping', { bubbles: true }));
         console.log(seen.join(','));
         sr.removeChild(x);
         const d = document.createElement('div'), ds = d.attachShadow({ mode: 'open' }), y = document.createElement('x-c');
         ds.appendChild(y); document.body.appendChild(d);"#,
    );
    assert!(log.contains(&"log:host:x-c,doc".to_string()), "{log:?}");
    assert!(log.contains(&"log:disc".to_string()), "{log:?}");
    assert_eq!(log.iter().filter(|l| *l == "log:conn").count(), 2, "{log:?}");
}

// ---------- QJ4: geometry reads (getBoundingClientRect, offset*, client*, scroll*) ----------

#[test]
fn qj4_geometry_reads_layout_backed_rects() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} article{display:block;height:100px;margin:0;border:2px solid red;padding:5px;width:300px}\
         #p{padding:0;border:0;height:0;position:relative;top:0}</style></head>\
         <body><article id=a1><p class=meta id=m1>one</p></article><article id=a2>two</article><article id=a3>three</article>\
         <div id=n style='display:none'><b id=nb>x</b></div><div id=p><i id=ch style='display:block;height:10px;margin-top:20px'></i></div></body></html>",
        r#"var a = ['a1','a2','a3'].map(function (i) { return document.getElementById(i).getBoundingClientRect(); });
         console.log('tops', a[0].top < a[1].top && a[1].top < a[2].top, a[0].height, a[0].width, a[0].top, a[0].left);
         console.log('sh', document.documentElement.scrollHeight >= a[0].height + a[1].height + a[2].height, document.documentElement.clientHeight === innerHeight, document.documentElement.clientWidth === innerWidth);
         var e1 = document.getElementById('a1');
         console.log('off', e1.offsetWidth, e1.offsetHeight, e1.clientWidth, e1.clientHeight, e1.clientLeft, e1.clientTop, e1.scrollHeight);
         var n = document.getElementById('n'), z = n.getBoundingClientRect();
         console.log('none', z.width, z.height, z.top, document.getElementById('nb').getBoundingClientRect().height, n.offsetHeight, n.getClientRects().length);
         var d = document.createElement('div'); console.log('det', d.getBoundingClientRect().height, d.offsetWidth, d.getClientRects().length, d.offsetParent);
         console.log('offp', document.getElementById('ch').offsetParent.id, document.getElementById('ch').offsetTop);
         document.documentElement.scrollTop = 0; e1.scrollTop = 7; console.log('st', e1.scrollTop, document.documentElement.scrollTop, e1.scrollLeft);
         console.log('rect', e1.getBoundingClientRect() instanceof DOMRect, e1.getClientRects().item(0).height);
         e1.style.height = '300px';
         console.log('after', e1.getBoundingClientRect().height);
         scrollTo(0, 50); console.log('scrolled', document.getElementById('a1').getBoundingClientRect().top);"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:tops true 114 314 0 0"), "{log:?}");
    assert!(has("log:sh true true true"), "{log:?}");
    assert!(has("log:off 314 114 310 110 2 2 110"), "{log:?}");
    assert!(has("log:none 0 0 0 0 0 0"), "{log:?}");
    assert!(has("log:det 0 0 0 null"), "{log:?}");
    assert!(has("log:offp p 20"), "{log:?}");
    // QJ6: a non-scrolling (overflow: visible) box keeps scrollTop 0, as in a browser.
    assert!(has("log:st 0 0 0"), "{log:?}");
    assert!(has("log:rect true 114"), "{log:?}");
    assert!(has("log:after 314"), "{log:?}");
    assert!(has("log:scrolled -50"), "{log:?}");
}

#[test]
fn qj4_forced_layout_time_is_not_charged_to_the_script_budget() {
    let before = crate::js::script_clock_nanos();
    let start = crate::js::thread_cpu_nanos();
    let mut x = 0u64;
    while crate::js::thread_cpu_nanos() - start < 20_000_000 {
        x = x.wrapping_add(1);
    }
    std::hint::black_box(x);
    crate::js::pause_budget_since(start);
    let charged = crate::js::script_clock_nanos() - before;
    assert!(charged < 10_000_000, "paused work still charged {charged} ns");
}

#[test]
fn qj4_scroll_height_sees_overflowing_content() {
    let log = run_in_html(
        "<html><body style='margin:0'><div id=b style='height:50px;overflow:hidden;width:200px'><div style='height:400px'></div></div></body></html>",
        "var b = document.getElementById('b'); console.log('s', b.clientHeight, b.scrollHeight);",
    );
    assert!(log.contains(&"log:s 50 400".to_string()), "{log:?}");
}

// ---- QY21: define() looks up only the new name; scans use the native candidate walk.

#[test]
fn qy21_define_is_linear_and_upgrade_order_unchanged() {
    let mut html = String::from("<html><body><div id=root>");
    for i in 0..1500 {
        if i % 5 == 0 {
            html.push_str(&format!("<x-n{} id=e{}></x-n{}>", i % 600, i, i % 600));
        } else {
            html.push_str(&format!("<span id=e{}></span>", i));
        }
    }
    html.push_str("<button id=b is=x-btn></button></div></body></html>");
    let log = run_in_html(
        &html,
        r#"const order = [];
         for (let i = 0; i < 600; i++) {
           customElements.define('x-n' + i, class extends HTMLElement { connectedCallback() { order.push(this.id); } });
         }
         customElements.define('x-btn', class extends HTMLButtonElement { connectedCallback() { order.push('btn'); } }, { extends: 'button' });
         console.log(__ce_stats.defineQueries, order.length);
         const root = document.getElementById('root');
         const frag = document.createElement('div');
         for (let i = 0; i < 300; i++) { const e = document.createElement('x-n' + i); e.id = 'n' + i; frag.appendChild(e); }
         root.appendChild(frag);
         console.log(order.length, order[order.length - 1]);
         const t = document.createElement('div');
         t.innerHTML = '<x-n1 id=h1></x-n1><p><x-n2 id=h2></x-n2></p>';
         root.appendChild(t);
         console.log(order.slice(-2).join(','));"#,
    );
    // 601 defines -> exactly 601 one-name queries (not 601 growing selectors).
    assert_eq!(log[0], "log:601 301", "{log:?}");
    assert_eq!(log[1], "log:601 n299", "{log:?}");
    assert_eq!(log[2], "log:h1,h2", "{log:?}");
}

/// QY22(a): compile time is not charged to the page-load run budget. A fake
/// clock makes the compile "take" 4.99 s of a 5 s budget; a script that then
/// needs a few ms of run time still completes.
#[test]
fn qy22_compile_time_is_not_counted_against_the_run_budget() {
    use crate::js::{AFTER_COMPILE, FAKE_CLOCK};
    FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 1_000_000))));
    AFTER_COMPILE.with(|c| c.set(Some(|| FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 4_990_000_000, s)))))));
    let log = run_collecting(&[src("var s=0;for(var i=0;i<300000;i++){s+=i}console.log('done');")]);
    FAKE_CLOCK.with(|f| f.set(None));
    AFTER_COMPILE.with(|c| c.set(None));
    assert!(log.iter().any(|l| l == "log:done"), "{log:?}");
    assert!(!log.iter().any(|l| l.starts_with("error:")), "{log:?}");
}

/// QY22(a): a trip in the run phase reports compile and run time apart.
#[test]
fn qy22_interrupt_message_splits_compile_and_run_time() {
    use crate::js::{AFTER_COMPILE, FAKE_CLOCK};
    FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 1_000_000))));
    AFTER_COMPILE.with(|c| c.set(Some(|| FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 3_000_000_000, s)))))));
    let log = run_collecting(&[src("for(;;){}")]);
    FAKE_CLOCK.with(|f| f.set(None));
    AFTER_COMPILE.with(|c| c.set(None));
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("compiled in 3.0 s, ran 5.0 s"), "{log:?}");
    assert!(errs[0].contains("5 s CPU-time page-load budget"), "{log:?}");
}

/// QY22(a): hard cap on the source size.
#[test]
fn qy22_oversized_script_is_not_compiled() {
    let big = format!("console.log('ran');/*{}*/", "x".repeat(33 * 1024 * 1024));
    let log = run_collecting(&[src(&big), src("console.log('after');")]);
    assert!(log.iter().any(|l| l.contains("not compiled") && l.contains("32 MB cap")), "{log:?}");
    assert!(!log.iter().any(|l| l == "log:ran"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// QY22(b): 1000 promise jobs queued behind a tripped busy loop log ONE
/// interrupt line (the task's), and the jobs get a fresh budget and run.
#[test]
fn qy22_promise_jobs_after_a_tripped_task_get_a_fresh_budget() {
    let log = run_with_timers(
        "var n=0;setTimeout(()=>{for(var i=0;i<1000;i++)Promise.resolve().then(()=>{n++});\
         for(;;){}},0);setTimeout(()=>console.log('n='+n),1);",
    );
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("timer stopped: exceeded the 500 ms"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:n=1000"), "{log:?}");
}

/// QY22(b): jobs that themselves overrun the fresh budget are counted in
/// one line, not logged once each.
#[test]
fn qy22_tripped_job_drain_logs_one_line_with_a_count() {
    let log = run_with_timers(
        "setTimeout(()=>{for(var i=0;i<1000;i++)Promise.resolve().then(()=>{var e=Date.now()+8;while(Date.now()<e){}});},0);",
    );
    // QY44a: a trip stops the drain and defers the rest, so one line per
    // turn that tripped (each names the kill and the deferral), never one per job.
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert!(!errs.is_empty() && errs.len() < 100, "{log:?}");
    assert!(errs.iter().all(|e| e.contains("promise jobs stopped") && e.contains("1 job killed")), "{log:?}");
    assert!(errs[0].contains("deferred to the next turn"), "{log:?}");
}

/// QY44a (R29): after one promise job is killed by the budget the drain stops
/// and the later jobs run in a later turn with a fresh budget, so their
/// `finally` blocks run (none starts on an expired deadline).
#[test]
fn qy44a_tripped_job_does_not_kill_later_jobs() {
    let log = run_with_timers(
        "var stack=[],ran=[];\
         function withConsumer(tag,f){stack.push(tag);try{return f();}finally{stack.pop();}}\
         setTimeout(function(){\
           Promise.resolve().then(function(){var x=0;for(;;){x++;}});\
           Promise.resolve().then(function(){withConsumer('job2',function(){var s=0;for(var i=0;i<200000;i++)s+=i;ran.push('job2');});});\
           Promise.resolve().then(function(){withConsumer('job3',function(){var s=0;for(var i=0;i<200000;i++)s+=i;ran.push('job3');});});\
         },0);\
         setTimeout(function(){console.log('completed:['+ran.join(',')+'] stack:['+stack.join(',')+']');},50);",
    );
    assert!(log.iter().any(|l| l == "log:completed:[job2,job3] stack:[]"), "{log:?}");
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("1 job killed, remaining jobs deferred"), "{log:?}");
}

// ---------- QJ5: Element.scrollIntoView ----------

#[test]
fn qj5_scroll_into_view_requests_window_scroll_from_layout_rect() {
    let doc = "<html><head><style>body{margin:0} div.s{height:2000px} #t{height:100px;background:red} #f{position:fixed;top:0;height:10px}</style></head>\
               <body><div class=s></div><div id=t></div><div class=s></div><div id=f></div></body></html>";
    let num = |log: &[String], key: &str| -> f64 { log.iter().find_map(|l| l.strip_prefix(key)).unwrap_or("NaN").parse().unwrap() };
    let vh = num(&run_in_html(doc, "console.log('vh', innerHeight);"), "log:vh ");
    let y = |js: &str| num(&run_in_html(doc, &format!("var t = document.getElementById('t'); {js}; console.log('y', scrollY);")), "log:y ");
    let top = 2000.0;
    assert_eq!(y("t.scrollIntoView()"), top);
    assert_eq!(y("t.scrollIntoView(true)"), top);
    assert_eq!(y("t.scrollIntoView({block:'start'})"), top);
    assert_eq!(y("t.scrollIntoView(false)"), top + 100.0 - vh);
    assert_eq!(y("t.scrollIntoView({block:'end'})"), top + 100.0 - vh);
    assert_eq!(y("t.scrollIntoView({block:'center'})"), top - (vh - 100.0) / 2.0);
    // nearest: below the viewport aligns the end; already visible does not move.
    assert_eq!(y("t.scrollIntoView({block:'nearest'})"), top + 100.0 - vh);
    assert_eq!(y("scrollTo(0, 2000); t.scrollIntoView({block:'nearest'})"), top);
    assert_eq!(y("scrollTo(0, 1950); t.scrollIntoView({block:'nearest'})"), 1950.0, "fully visible: no move");
    assert_eq!(y("scrollTo(0, 2050); t.scrollIntoView({block:'nearest'})"), top, "cut at the top: align start");
    // fixed and detached elements do nothing
    assert_eq!(y("document.getElementById('f').scrollIntoView()"), 0.0);
    assert_eq!(y("document.createElement('p').scrollIntoView()"), 0.0);
}

// ---------- QJ7: DOM Range (boundary points + client rects) ----------

#[test]
fn qj7_range_boundary_points_follow_the_spec() {
    let log = run_in_html(
        "<html><body><div id=d>ab<b id=b>cd</b>ef</div><p id=p></p></body></html>",
        r#"var d = document.getElementById('d'), b = document.getElementById('b'), t0 = d.firstChild, t1 = b.firstChild, t2 = d.lastChild;
         var r = document.createRange(), s = new Range();
         console.log('init', r.startContainer === document, r.startOffset, r.collapsed, s.endOffset, String(r));
         r.setStart(t0, 1); r.setEnd(t2, 1);
         console.log('set', r.startOffset, r.endOffset, r.collapsed, r.commonAncestorContainer === d, r.toString());
         r.selectNodeContents(b); console.log('snc', r.startContainer === b, r.endOffset, r.toString());
         r.selectNode(b); console.log('sn', r.startContainer === d, r.startOffset, r.endOffset, r.toString());
         r.collapse(true); console.log('col', r.collapsed, r.endOffset);
         r.setStart(t2, 2); console.log('after-end', r.startContainer === t2, r.endContainer === t2, r.endOffset, r.collapsed);
         var q = r.cloneRange(); q.setEnd(d, 3); console.log('clone', r.endContainer === t2, q.endContainer === d);
         var e = []; [function () { r.setStart(t0, 1); }, function () { r.setEnd(t0, 99); }, function () { r.setStart(document.implementation.createDocumentType('html', '', ''), 0); }, function () { r.selectNode(document); }].forEach(function (f) { try { f(); e.push('ok'); } catch (x) { e.push(x.name); } });
         console.log('err', e.join());
         var a = document.createRange(), c = document.createRange(); a.setStart(t0, 0); a.setEnd(t1, 1); c.setStart(t1, 0); c.setEnd(t2, 2);
         console.log('cmp', a.compareBoundaryPoints(Range.START_TO_START, c), a.compareBoundaryPoints(Range.END_TO_START, c), a.compareBoundaryPoints(Range.END_TO_END, c), c.compareBoundaryPoints(Range.START_TO_END, a));
         console.log('pt', a.isPointInRange(t0, 1), a.isPointInRange(t2, 0), a.comparePoint(t2, 0), a.comparePoint(t0, 0), a.intersectsNode(b), a.intersectsNode(document.getElementById('p')));
         var det = document.createElement('i'); det.appendChild(document.createTextNode('xy'));
         a.setStart(t0, 0); a.setEnd(det.firstChild, 1); console.log('root', a.startContainer === det.firstChild, a.collapsed);
         var w = document.createRange(); w.setStartBefore(b); w.setEndAfter(b); console.log('ba', w.startOffset, w.endOffset, w.toString());"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:init true 0 true 0 "), "{log:?}");
    assert!(has("log:set 1 1 false true bcde"), "{log:?}");
    assert!(has("log:snc true 1 cd"), "{log:?}");
    assert!(has("log:sn true 1 2 cd"), "{log:?}");
    assert!(has("log:col true 1"), "{log:?}");
    assert!(has("log:after-end true true 2 true"), "{log:?}");
    assert!(has("log:clone true true"), "{log:?}");
    assert!(has("log:err ok,IndexSizeError,InvalidNodeTypeError,InvalidNodeTypeError"), "{log:?}");
    assert!(has("log:cmp -1 -1 -1 1"), "{log:?}");
    assert!(has("log:pt true false 1 0 true false"), "{log:?}");
    assert!(has("log:root true true"), "{log:?}");
    assert!(has("log:ba 1 2 cd"), "{log:?}");
}

#[test]
fn qj7_range_client_rects_come_from_layout_per_character_and_line() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} #t{font:20px monospace;width:2000px;margin:0;padding:0} #w{font:20px monospace;width:60px;margin:0}\
         #c{font:20px monospace;margin:0;position:absolute;left:100px;top:300px}</style></head>\
         <body><p id=t>abc</p><p id=w>aaa bbb ccc</p><div id=c>x<b>yz</b></div><div id=n style='display:none'>hid</div></body></html>",
        r#"var t = document.getElementById('t').firstChild, r = document.createRange(), rc = [];
         for (var i = 0; i < 3; i++) { r.setStart(t, i); r.setEnd(t, i + 1); rc.push(r.getBoundingClientRect()); }
         var same = rc[0].top === rc[1].top && rc[1].top === rc[2].top && rc[0].height > 0;
         var order = rc[0].left < rc[1].left && rc[1].left < rc[2].left && rc[0].width > 5 && Math.abs(rc[0].width - rc[1].width) < 0.5;
         console.log('chars', same, order);
         r.selectNodeContents(t); var all = r.getClientRects(), bb = r.getBoundingClientRect();
         console.log('all', all.length, Math.abs(bb.width - (rc[0].width + rc[1].width + rc[2].width)) < 0.5, bb.left === rc[0].left, all.item(0).width === bb.width, all.item(5));
         var w = document.getElementById('w').firstChild; r.selectNodeContents(w);
         var lines = r.getClientRects(); console.log('wrap', lines.length >= 2, lines[0].top < lines[1].top);
         r.setStart(w, 4); r.setEnd(w, 7); console.log('part', r.getClientRects().length >= 1, r.getBoundingClientRect().width > 0);
         r.collapse(true); console.log('collapsed', r.getClientRects().length, r.getBoundingClientRect().width);
         var c = document.getElementById('c'); r.selectNodeContents(c); var cb = r.getBoundingClientRect();
         console.log('abs', Math.round(cb.left), Math.round(cb.top), cb.width > 25);
         r.selectNodeContents(document.getElementById('n')); console.log('hidden', r.getClientRects().length, r.getBoundingClientRect().height);
         var d = document.createTextNode('zz'), q = document.createRange(); q.selectNodeContents(d); console.log('detached', q.getClientRects().length);
         t.data = 'abcdef'; r.setStart(t, 0); r.setEnd(t, 6); console.log('mutated', r.getBoundingClientRect().width > rc[0].width * 5.5);"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:chars true true"), "{log:?}");
    assert!(has("log:all 1 true true true null"), "{log:?}");
    assert!(has("log:wrap true true"), "{log:?}");
    assert!(has("log:part true true"), "{log:?}");
    assert!(has("log:collapsed 0 0"), "{log:?}");
    assert!(has("log:abs 100 300 true"), "{log:?}");
    assert!(has("log:hidden 0 0"), "{log:?}");
    assert!(has("log:detached 0"), "{log:?}");
    assert!(has("log:mutated true"), "{log:?}");
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

// ---------- QY34: script-inserted <script> elements ----------

fn qy34_status(status: u16, body: &str) -> Result<WireResponse, NetError> {
    Ok(WireResponse { status, status_text: String::new(), headers: vec![], body: body.as_bytes().to_vec(), error: None, final_url: None })
}

fn qy34_run(page: &str, answers: Vec<(&'static str, Result<WireResponse, NetError>)>, code: &str) -> (Vec<String>, Arc<QfFake>) {
    let fake = QfFake::new(answers);
    let (engine, log) = qf7_engine(page, &fake);
    engine.eval("t", code);
    qf7_drive(&engine, &fake);
    engine.run_due();
    qf7_drive(&engine, &fake);
    engine.run_due();
    let l = log.borrow().clone();
    (l, fake)
}

#[test]
fn qy34_inline_script_runs_synchronously_with_current_script_in_insertion_order() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![],
        "var a = document.createElement('script'); a.id = 'a'; a.textContent = \"console.log('run a', document.currentScript && document.currentScript.id);\";\
         var b = document.createElement('script'); b.id = 'b'; b.text = \"console.log('run b', document.currentScript.id);\";\
         document.head.appendChild(a); console.log('between'); document.body.appendChild(b); console.log('after', document.currentScript);",
    );
    assert_eq!(l, vec!["log:run a a", "log:between", "log:run b b", "log:after null"], "{l:?}");
    assert!(fake.urls().is_empty());
}

#[test]
fn qy34_src_script_goes_through_the_queue_runs_async_and_fires_load() {
    let (l, fake) = qy34_run(
        "https://page.test/dir/",
        vec![("/x.js", qy34_status(200, "console.log('x ran', document.currentScript.id);"))],
        "var s = document.createElement('script'); s.id = 'sx'; s.src = 'x.js';\
         s.onload = function () { console.log('load', this.id); }; s.addEventListener('error', function () { console.log('error'); });\
         document.head.appendChild(s); console.log('appended');",
    );
    assert_eq!(l, vec!["log:appended", "log:x ran sx", "log:load sx"], "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/dir/x.js"]);
}

#[test]
fn qy34_http_error_and_policy_refusal_fire_error_and_log() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![("/missing.js", qy34_status(404, "nope"))],
        "['/missing.js', 'ftp://page.test/p.js'].forEach(function (u) {\
           var s = document.createElement('script'); s.src = u;\
           s.onload = function () { console.log('load', u); }; s.onerror = function () { console.log('error', u); };\
           document.head.appendChild(s); });",
    );
    assert!(l.contains(&"log:error /missing.js".to_string()), "{l:?}");
    assert!(l.contains(&"log:error ftp://page.test/p.js".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:load")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("error:https://page.test/missing.js: failed to load script (HTTP 404)")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:blocked GET ftp://page.test/p.js")), "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/missing.js"]);
}

#[test]
fn qy34_a_moved_script_is_not_run_again_and_innerhtml_scripts_never_run() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![],
        "var s = document.createElement('script'); s.textContent = \"console.log('once');\";\
         document.head.appendChild(s); document.body.appendChild(s); document.head.insertBefore(s, null);\
         var c = s.cloneNode(true); document.body.appendChild(c);\
         var d = document.createElement('div'); d.innerHTML = '<script>console.log(\"inner\")<\\/script>'; document.body.appendChild(d);\
         var late = document.createElement('script'); document.body.appendChild(late); late.textContent = \"console.log('late');\";\
         var det = document.createElement('div'); var ds = document.createElement('script'); ds.text = \"console.log('detached');\"; det.appendChild(ds);\
         console.log('before attach'); document.body.appendChild(det);",
    );
    assert_eq!(l, vec!["log:once", "log:late", "log:before attach", "log:detached"], "{l:?}");
}

#[test]
fn qy34_type_nomodule_and_language_checks() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![],
        "function add(attrs) { var s = document.createElement('script'); for (var k in attrs) s.setAttribute(k, attrs[k]); s.textContent = \"console.log('ran', '\" + JSON.stringify(attrs) + \"');\"; document.head.appendChild(s); }\
         add({type: 'text/plain'}); add({type: 'application/json'}); add({nomodule: ''}); add({type: ''}); add({type: ' Text/JavaScript '});\
         add({language: 'javascript'}); add({language: 'vbscript'}); add({type: '', language: 'vbscript'});",
    );
    let ran: Vec<&str> = l.iter().map(String::as_str).collect();
    assert_eq!(
        ran,
        vec![
            "log:ran {\"type\":\"\"}",
            "log:ran {\"type\":\" Text/JavaScript \"}",
            "log:ran {\"language\":\"javascript\"}",
            "log:ran {\"type\":\"\",\"language\":\"vbscript\"}",
        ],
        "{l:?}"
    );
}

#[test]
fn qy34_async_false_scripts_run_in_insertion_order() {
    let fake = QfFake::new(vec![("/slow.js", qy34_status(200, "console.log('slow');")), ("/fast.js", qy34_status(200, "console.log('fast');"))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "['slow', 'fast'].forEach(function (n) { var s = document.createElement('script'); s.async = false; s.src = '/' + n + '.js';\
         s.onload = function () { console.log('load', n); }; document.head.appendChild(s); });",
    );
    qf7_drive(&engine, &fake);
    assert_eq!(*log.borrow(), vec!["log:slow", "log:load slow", "log:fast", "log:load fast"]);
}

#[test]
fn qy34_module_script_inline_and_src_with_import() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![
            ("/m.js", qy34_status(200, "import { v } from './dep.js'; console.log('m', v, document.currentScript);")),
            ("/dep.js", qy34_status(200, "export const v = 42;")),
        ],
        "var s = document.createElement('script'); s.type = 'module'; s.src = '/m.js'; s.onload = function () { console.log('load m'); };\
         document.head.appendChild(s);\
         var i = document.createElement('script'); i.type = 'module'; i.textContent = \"console.log('inline module', typeof import.meta.url);\"; document.head.appendChild(i);",
    );
    assert!(l.contains(&"log:m 42 null".to_string()), "{l:?}");
    assert!(l.contains(&"log:load m".to_string()), "{l:?}");
    assert!(l.contains(&"log:inline module string".to_string()), "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/m.js", "https://page.test/dep.js"]);
}

#[test]
fn qy34_script_errors_are_logged_and_do_not_break_the_inserter() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/bad.js", qy34_status(200, "throw new Error('boom');"))],
        "var s = document.createElement('script'); s.textContent = 'null.x'; document.head.appendChild(s); console.log('still here');\
         var t = document.createElement('script'); t.src = '/bad.js'; t.onload = function () { console.log('load bad'); }; document.head.appendChild(t);",
    );
    assert!(l.iter().any(|x| x.starts_with("error:inline script (inserted): TypeError")), "{l:?}");
    assert!(l.contains(&"log:still here".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("error:https://page.test/bad.js: Error: boom")), "{l:?}");
    assert!(l.contains(&"log:load bad".to_string()), "{l:?}");
}

#[test]
fn qy34_cross_origin_script_runs_without_cors_and_is_not_a_suppressed_beacon() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![("cdn.test/a.js", qy34_status(200, "console.log('cdn ran');")), ("cdn.test/c.js", qy34_status(200, "console.log('cors ran');"))],
        "var s = document.createElement('script'); s.src = 'https://cdn.test/a.js'; s.onload = function () { console.log('load a'); }; document.head.appendChild(s);\
         var c = document.createElement('script'); c.src = 'https://cdn.test/c.js'; c.crossOrigin = 'anonymous'; c.onerror = function () { console.log('cors error'); }; document.head.appendChild(c);",
    );
    assert!(l.contains(&"log:cdn ran".to_string()), "{l:?}");
    assert!(l.contains(&"log:load a".to_string()), "{l:?}");
    // crossorigin=anonymous is a CORS request: no Access-Control-Allow-Origin, so it is blocked.
    assert!(l.contains(&"log:cors error".to_string()), "{l:?}");
    assert!(!l.contains(&"log:cors ran".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.contains("beacon")), "{l:?}");
    assert_eq!(fake.urls().len(), 2);
}

#[test]
fn qy34_runaway_fetched_script_hits_the_cpu_budget_and_later_scripts_still_run() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/loop.js", qy34_status(200, "for(;;){}")), ("/ok.js", qy34_status(200, "console.log('ok after');"))],
        "['loop', 'ok'].forEach(function (n) { var s = document.createElement('script'); s.src = '/' + n + '.js'; document.head.appendChild(s); });",
    );
    assert!(l.iter().any(|x| x.starts_with("error:js: https://page.test/loop.js stopped: exceeded the 5 s CPU-time page-load budget")), "{l:?}");
    assert!(l.contains(&"log:ok after".to_string()), "{l:?}");
}

/// QY42: a non-empty descendant's definite px `width` is its max-content
/// contribution (YouTube's 40 px kebab button around a 20 px icon).
#[test]
fn qy42_definite_width_descendant_non_empty() {
    let mk = |inner: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display:flex;width:500px"><div style="flex:none;display:flex">{inner}</div><div style="width:10px;height:5px"></div></div></body></html>"#
        )
    };
    let marker_x = |html: &str| {
        let laid = measure_fixture(html, 600.0);
        find_by_width(&laid, 10.0).expect("marker item").x
    };
    let base = r#"<span style="display:inline-block;width:40px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(base)) - 40.0).abs() < 1.0);
    let bb = r#"<span style="display:inline-block;width:40px;box-sizing:border-box;padding:0 5px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(bb)) - 40.0).abs() < 1.0);
    let mx = r#"<span style="display:inline-block;width:40px;max-width:30px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(mx)) - 30.0).abs() < 1.0);
    let mn = r#"<span style="display:inline-block;width:40px;min-width:50px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(mn)) - 50.0).abs() < 1.0);
    // YouTube chain: flex wrapper > flex > inline-flex > flex a > inline-block 40px > inline-block 100% > inline-block 24px
    let yt = r#"<div style="display:flex"><span style="display:inline-flex"><a style="display:flex"><span style="display:inline-block;width:40px;height:40px"><span style="display:inline-block;width:100%"><span style="display:inline-block;width:24px;height:24px"></span></span></span></a></span><span style="display:inline-block;width:95px">x</span></div>"#;
    assert!((marker_x(&mk(yt)) - 135.0).abs() < 1.0, "yt chain {}", marker_x(&mk(yt)));
    // max-width: fit-content must keep measuring the content (WPT).
    let fc = r#"<span style="display:inline-block;width:40px;max-width:fit-content"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(fc)) - 20.0).abs() < 1.0);
}

// ---------- QY44b: window.screen / visualViewport ----------

#[test]
fn qy44b_window_screen_and_visual_viewport() {
    let out = run_collecting(&[src(
        "var r = [];\
         r.push(typeof screen, screen instanceof Screen, screen === window.screen, screen.width >= innerWidth, screen.height >= innerHeight);\
         r.push(screen.availWidth > 0, screen.availHeight > 0, screen.availLeft, screen.availTop, screen.colorDepth, screen.pixelDepth);\
         r.push(typeof screen.orientation.type, screen.orientation.angle, typeof screen.orientation.addEventListener);\
         screen.orientation.addEventListener('change', function () {}); screen.orientation.removeEventListener('change', function () {});\
         var Jp = { height: window?.screen.height, width: window?.screen.width }; r.push(Jp.height === innerHeight);\
         r.push(screenX, screenY, screenLeft, screenTop);\
         r.push(visualViewport.width === innerWidth, visualViewport.height === innerHeight, visualViewport.scale, visualViewport.offsetLeft, visualViewport.offsetTop, visualViewport.pageLeft, visualViewport.pageTop);\
         visualViewport.addEventListener('resize', function () {}); visualViewport.removeEventListener('resize', function () {});\
         console.log(r.join('|'));",
    )]);
    assert_eq!(
        out,
        vec!["log:object|true|true|true|true|true|true|0|0|24|24|string|0|function|true|0|0|0|0|true|true|1|0|0|0|0"]
    );
}

/// QY43: a preserved newline inside a paragraph on the per-word path
/// (inline-block / padded inline present) is a forced break; a blank line
/// ("\n\n") is an empty line; no two word boxes on a line overlap.
type Qy43Words = Vec<(f32, f32, f32)>;
fn qy43_items(html: &str) -> (Qy43Words, Option<(f32, f32)>) {
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed-inline path");
    let items = mixed_items(run);
    let words: Vec<(f32, f32, f32)> = items
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Word { x, y, w, .. } => Some((*x, *y, *w)),
            _ => None,
        })
        .collect();
    let emb = items.iter().find_map(|i| match i {
        MixedPositioned::Embedded(l) => Some((l.x, l.y)),
        _ => None,
    });
    (words, emb)
}

fn qy43_assert_no_overlap(words: &[(f32, f32, f32)]) {
    for (i, a) in words.iter().enumerate() {
        for b in &words[i + 1..] {
            if (a.1 - b.1).abs() < 0.5 {
                assert!(a.0 + a.2 <= b.0 + 0.5 || b.0 + b.2 <= a.0 + 0.5, "overlap {a:?} {b:?}");
            }
        }
    }
}

#[test]
fn qy43_pre_wrap_newlines_with_inline_block_do_not_overprint() {
    let (words, emb) = qy43_items(
        "<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap;width:400px\">Aaa bbb.\n\nWenn du mehr <span style=\"display:inline-block\">Info</span> ccc</div></body></html>",
    );
    qy43_assert_no_overlap(&words);
    let first_y = words[0].1;
    let last_y = words.last().unwrap().1;
    // Line 1, an empty line, line 3: the last word sits two line heights down.
    let step = (last_y - first_y) / 2.0;
    assert!(step > 5.0, "blank line missing: {first_y} {last_y}");
    let (_, ey) = emb.expect("inline-block");
    assert!(ey > first_y + step, "inline-block on the third block: {ey}");
}

#[test]
fn qy43_pre_wrap_pre_line_pre_with_padded_inline_break_at_newline() {
    for ws in ["pre-wrap", "pre-line", "pre"] {
        let (words, _) = qy43_items(&format!(
            "<html><body style=\"margin:0\"><div style=\"white-space:{ws}\">one two\nthree <span style=\"padding:0 3px\">four</span></div></body></html>"
        ));
        qy43_assert_no_overlap(&words);
        let ys: Vec<f32> = words.iter().map(|w| w.1).collect();
        // A chunk that stays on one line is one merged word box.
        assert!(words.len() >= 2, "{ws}: {ys:?}");
        assert!(ys[1] > ys[0] + 5.0, "{ws}: second line lower {ys:?}");
        assert!((ys[1] - ys[2]).abs() < 0.5 || ys.len() == 2, "{ws}: three/four share a line {ys:?}");
    }
}

#[test]
fn qy43_inline_pre_wrap_span_in_normal_block_breaks_at_newline() {
    // YouTube: `yt-formatted-string{white-space:pre-wrap}` is an inline
    // element in a block whose own white-space is `normal`.
    let (words, emb) = qy43_items(
        "<html><body style=\"margin:0\"><div style=\"width:300px\"><span style=\"white-space:pre-wrap\">Aaa bbb.\n\nWenn du die Einstellung aendern moechtest, schalte den YouTube-Verlauf ein und bestaetige alles, </span><a style=\"display:inline-block\">Info</a></div></body></html>",
    );
    qy43_assert_no_overlap(&words);
    let first_y = words[0].1;
    assert!(words.iter().skip(1).all(|w| w.1 >= first_y + 2.0 * 10.0), "text after the blank line stays below: {words:?}");
    assert!(emb.unwrap().1 > words.last().unwrap().1 - 1.0);
}

#[test]
fn qy43_plain_pre_wrap_blank_line_and_normal_collapse_unchanged() {
    let two = measure_fixture("<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap\">a\n\nb</div></body></html>", 800.0);
    let one = measure_fixture("<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap\">a\nb</div></body></html>", 800.0);
    assert!(two.h > one.h + 5.0, "blank line adds height: {} vs {}", two.h, one.h);
    let (words, _) = qy43_items(
        "<html><body style=\"margin:0\"><div>one\ntwo <span style=\"display:inline-block\">x</span></div></body></html>",
    );
    assert!(words.iter().all(|w| (w.1 - words[0].1).abs() < 0.5), "{words:?}");
}

// ---------- QJ8: document.activeElement / hasFocus ----------

#[test]
fn qj8_active_element_follows_focus_blur_and_removal() {
    crate::js_dom::set_active_element(None);
    let log = run_in_html(
        "<html><body><input id=i><button id=b>x</button><div id=d></div></body></html>",
        "var i=document.getElementById('i'), b=document.getElementById('b');\
         console.log(document.activeElement===document.body, document.hasFocus());\
         i.focus(); console.log(document.activeElement===i);\
         b.focus(); console.log(document.activeElement===b);\
         i.blur(); console.log(document.activeElement===b);\
         b.blur(); console.log(document.activeElement===document.body);\
         i.focus(); i.remove(); console.log(document.activeElement===document.body);\
         var d2=document.implementation.createHTMLDocument('x'); var e=d2.createElement('input'); d2.body.appendChild(e);\
         b.focus(); console.log(d2.activeElement===d2.body, document.activeElement===b);",
    );
    assert_eq!(
        log,
        ["log:true true", "log:true", "log:true", "log:true", "log:true", "log:true", "log:true true"],
        "{log:?}"
    );
}

// ---------- QY44c: document.styleSheets, style/link .sheet, insertRule reaches the cascade ----------

const QY44C_HTML: &str = "<html><head><style id=s>p{color:#0000ff}</style><link rel=stylesheet href=x.css id=l></head><body><p id=p>x</p></body></html>";

#[test]
fn qy44c_insert_rule_into_empty_style_changes_colour() {
    let html = "<html><head><style id=s></style></head><body><p id=p>x</p></body></html>";
    let (c, l, _) = qsd4_color(html, "var sh=document.getElementById('s').sheet; sh.insertRule('p{color:#ff0000}',0); console.log(sh.cssRules.length, getComputedStyle(document.getElementById('p')).color);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
    assert_eq!(l[0], "log:1 rgb(255, 0, 0)");
}

#[test]
fn qy44c_delete_rule_and_text_unchanged() {
    let (c, l, _) = qsd4_color(QY44C_HTML, "var e=document.getElementById('s'),sh=e.sheet; sh.insertRule('p{color:#ff0000}',1); console.log(sh.cssRules.length, e.textContent); sh.deleteRule(1);");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)), "{l:?}");
    assert_eq!(l[0], "log:2 p{color:#0000ff}");
    let (c, _, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.deleteRule(0);");
    assert_ne!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, l, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.insertRule('p{color:#00ff00}'); sh.removeRule(1); sh.addRule('p','color:#ff0000'); console.log(sh.cssRules.length);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
}

#[test]
fn qy44c_text_content_change_replaces_sheet() {
    let (c, l, _) = qsd4_color(
        QY44C_HTML,
        "var e=document.getElementById('s'),a=e.sheet; a.insertRule('p{color:#ff0000}',1); e.textContent='p{color:#00ff00}'; var b=e.sheet; console.log(a===b, b.cssRules.length, b.cssRules[0].selectorText);",
    );
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "{l:?}");
    assert_eq!(l[0], "log:false 1 p");
}

#[test]
fn qy44c_style_sheets_list_and_owner() {
    let (_e, _d, l) = events_engine(
        QY44C_HTML,
        "var ss=document.styleSheets; var s=document.getElementById('s'),k=document.getElementById('l');\
         console.log(ss.length, ss[0].ownerNode===s, ss.item(1).ownerNode===k, ss[1].cssRules.length, s.sheet===ss[0], ss[0] instanceof CSSStyleSheet, ss.item(5));\
         var d=document.createElement('style'); console.log(d.sheet); document.head.appendChild(d); console.log(document.styleSheets.length, d.sheet.cssRules.length);\
         var c=new CSSStyleSheet(); document.adoptedStyleSheets=[c]; console.log(document.styleSheets.length, ss[0].disabled, ss[0].type);",
    );
    let l = l.borrow();
    assert_eq!(l[0], "log:2 true true 0 true true null", "{l:?}");
    assert_eq!(l[1], "log:null");
    assert_eq!(l[2], "log:3 0");
    assert_eq!(l[3], "log:3 false text/css");
}

#[test]
fn qy44c_disabled_sheet_and_rule_style_mutation() {
    let (c, _, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.disabled=true;");
    assert_ne!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, l, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.cssRules[0].style.color='#ff0000'; console.log(sh.cssRules[0].cssText);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
}

// ---------- QJ9: window.open / close / alert / confirm / prompt ----------

#[test]
fn qj9_window_open_close_dialogs_are_inert_and_log_once() {
    let log = run_in_html(
        "<html><body></body></html>",
        "console.log(typeof window.open, typeof window.close, typeof alert, typeof confirm, typeof prompt);\
         console.log(window.open('/x', '_blank', 'noopener'));\
         console.log(window.close());\
         console.log(alert('hi'), confirm('sure?'), prompt('name?'));\
         console.log('still running');",
    );
    assert_eq!(log[0], "log:function function function function function", "{log:?}");
    assert_eq!(log[1], "error:scratchy: js: not implemented: window.open \"/x target=_blank features=noopener\"", "{log:?}");
    assert_eq!(log[2], "log:null", "{log:?}");
    assert_eq!(log[3], "error:scratchy: js: not implemented: window.close", "{log:?}");
    assert_eq!(log[4], "log:undefined", "{log:?}");
    assert!(log[5].contains("window.alert \"hi\""), "{log:?}");
    assert!(log[6].contains("window.confirm"), "{log:?}");
    assert!(log[7].contains("window.prompt"), "{log:?}");
    assert_eq!(log[8], "log:undefined false null", "{log:?}");
    assert_eq!(log[9], "log:still running", "{log:?}");
    assert_eq!(log.len(), 10, "{log:?}");
}

// ---------- QJ6: element scrolling ----------

const QJ6_DOC: &str = "<html><head><style>body{margin:0} #box{height:100px;overflow:auto;position:relative;border:0}\
    .row{height:40px} #plain{height:100px;overflow:hidden}</style></head><body>\
    <div id=box><div class=row id=r0></div><div class=row id=r1></div><div class=row id=r2></div><div class=row id=r3></div><div class=row id=r4></div></div>\
    <div id=plain><div style='height:300px'></div></div></body></html>";

fn qj6_log(code: &str) -> Vec<String> {
    run_in_html(QJ6_DOC, &format!("var box=document.getElementById('box'), r4=document.getElementById('r4'); {code}"))
}

#[test]
fn qj6_scroll_top_set_clamps_and_reads_back() {
    // content 200 px, client 100 px: max scrollTop 100
    let log = qj6_log(
        "console.log(box.scrollHeight, box.clientHeight, box.scrollTop);\
         box.scrollTop = 50; console.log(box.scrollTop);\
         box.scrollTop = 9999; console.log(box.scrollTop);\
         box.scrollTop = -5; console.log(box.scrollTop);\
         var p=document.getElementById('plain'); p.scrollTop = 20; console.log('hidden', p.scrollTop);",
    );
    assert_eq!(log, ["log:200 100 0", "log:50", "log:100", "log:0", "log:hidden 0"], "{log:?}");
}

#[test]
fn qj6_scroll_methods_move_children_rects() {
    let log = qj6_log(
        "var y0=r4.getBoundingClientRect().top;\
         box.scrollBy(0, 30); console.log(box.scrollTop, y0 - r4.getBoundingClientRect().top);\
         box.scrollTo(0, 70); console.log(box.scrollTop, y0 - r4.getBoundingClientRect().top);\
         box.scroll({top: 10}); console.log(box.scrollTop);\
         box.scrollBy({top: 500}); console.log(box.scrollTop);\
         box.scrollTop = 0; console.log(y0 - r4.getBoundingClientRect().top);",
    );
    assert_eq!(log, ["log:30 30", "log:70 70", "log:10", "log:100", "log:0"], "{log:?}");
}

#[test]
fn qj6_scroll_event_fires_on_change_only() {
    let doc = parse_html(QJ6_DOC);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &doc,
        &[src("var box=document.getElementById('box'); var n=0; box.addEventListener('scroll', function(){ n++; console.log('scroll', box.scrollTop); });\
               box.scrollTop = 40; box.scrollTop = 40; box.scrollBy(0, 0);")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..100 {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    assert_eq!(*out.borrow(), ["log:scroll 40"]);
}

#[test]
fn qj6_scroll_into_view_scrolls_ancestor_then_window() {
    let doc = "<html><head><style>body{margin:0} .s{height:2000px} #box{height:100px;overflow:auto;position:relative}\
        .row{height:40px}</style></head><body><div class=s></div>\
        <div id=box><div class=row></div><div class=row></div><div class=row></div><div class=row></div><div class=row id=t></div></div>\
        <div class=s></div></body></html>";
    let log = run_in_html(doc, "var box=document.getElementById('box'), t=document.getElementById('t');\
        t.scrollIntoView({block:'start'}); console.log(box.scrollTop, scrollY);\
        console.log(t.getBoundingClientRect().top);");
    // the row starts at 160 inside a box that can scroll 100: clamped; the
    // window then brings the box's scrolled row to the top: 2000 + 160 - 100.
    assert_eq!(log, ["log:100 2060", "log:0"], "{log:?}");
}

#[test]
fn qj6_app_applies_element_scroll_request() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let doc = parse_html(QJ6_DOC);
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(&doc, &[src("document.getElementById('box').scrollBy(0, 50);")], None);
    app.js_sync();
    assert_eq!(app.node_scroll.values().copied().collect::<Vec<_>>(), [50.0]);
}

// ---------- QM1e: the media byte channel over a fake wire (no libmpv, no network) ----------

use crate::media_net::{MediaMode, MediaNetError, MediaSource, MediaStream, CHUNK_LARGE, CHUNK_SMALL};

#[derive(Default, Clone)]
struct Qm1eCfg {
    file: Vec<u8>,
    ignore_range: bool,
    /// Drop the second half of the body of the answer to request #1.
    truncate_once: bool,
    /// Report `Content-Range` starting this many bytes late.
    shift_start: u64,
    /// `X-Wire-Final-Url` per request index (0-based), else none.
    final_urls: Vec<(usize, String)>,
    acao: Option<String>,
    acac: bool,
    /// Accept requests and never answer.
    hold: bool,
}

struct Qm1eWire {
    path: std::path::PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
    cfg: Arc<Mutex<Qm1eCfg>>,
}

fn qm1e_wire(cfg: Qm1eCfg) -> Qm1eWire {
    use std::os::unix::net::UnixListener;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!("qm1e-{}-{}.sock", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind fake wire");
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let cfg = Arc::new(Mutex::new(cfg));
    let (s, c) = (seen.clone(), cfg.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (s, c) = (s.clone(), c.clone());
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let idx = {
                    let mut g = s.lock().unwrap();
                    g.push(text.clone());
                    g.len() - 1
                };
                let cfg = c.lock().unwrap().clone();
                if cfg.hold {
                    thread::sleep(std::time::Duration::from_secs(20));
                    return;
                }
                let hdr = |n: &str| {
                    text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()))
                };
                let mut extra = String::from("X-Wire-Version: 3\r\n");
                if let Some((_, u)) = cfg.final_urls.iter().find(|(i, _)| *i == idx) {
                    extra += &format!("X-Wire-Final-Url: {u}\r\n");
                }
                if let Some(a) = &cfg.acao {
                    extra += &format!("Access-Control-Allow-Origin: {a}\r\n");
                }
                if cfg.acac {
                    extra += "Access-Control-Allow-Credentials: true\r\n";
                }
                let max: Option<usize> = hdr("x-wire-max-bytes").and_then(|v| v.parse().ok());
                let total = cfg.file.len() as u64;
                let range = hdr("range").and_then(|r| {
                    let r = r.strip_prefix("bytes=")?.to_string();
                    let (a, b) = r.split_once('-')?;
                    Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))
                });
                let (head, body): (String, Vec<u8>) = match range {
                    Some((a, b)) if !cfg.ignore_range => {
                        if a >= total {
                            (format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\n{extra}"), vec![])
                        } else {
                            let b = b.min(total - 1);
                            let mut body = cfg.file[a as usize..=b as usize].to_vec();
                            if cfg.truncate_once && idx == 1 {
                                body.truncate(body.len() / 2);
                            }
                            (
                                format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{b}/{total}\r\n{extra}", a + cfg.shift_start),
                                body,
                            )
                        }
                    }
                    _ => match max {
                        Some(m) if cfg.file.len() > m => (
                            format!("HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nX-Wire-Error: too-large\r\n{extra}"),
                            b"wire: response too large".to_vec(),
                        ),
                        _ => (format!("HTTP/1.1 200 OK\r\n{extra}"), cfg.file.clone()),
                    },
                };
                let _ = stream.write_all(format!("{head}\r\n").as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    Qm1eWire { path, seen, cfg }
}

fn qm1e_file(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

fn qm1e_src(url: &str, page: &str, mode: MediaMode) -> MediaSource {
    MediaSource { url: Url::parse(url).unwrap(), page: Url::parse(page).unwrap(), mode }
}

fn qm1e_open(w: &Qm1eWire, url: &str, page: &str, mode: MediaMode) -> Result<MediaStream, MediaNetError> {
    MediaStream::open(w.path.clone(), qm1e_src(url, page, mode), RequestPolicy::default())
}

fn qm1e_read_all(s: &mut MediaStream, mut want: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    while want > 0 {
        let n = s.read(&mut buf[..want.min(65536)]).expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        want -= n;
    }
    out
}

const QM1E_URL: &str = "https://cdn.example/v.mp4";
const QM1E_PAGE: &str = "https://page.example/";

#[test]
fn qm1e_206_gives_total_from_content_range_and_exact_sequential_bytes() {
    let file = qm1e_file(3 * 1024 * 1024 - 100);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.size(), Some(file.len() as u64));
    assert!(s.seekable());
    let got = qm1e_read_all(&mut s, file.len() + 10);
    assert!(got == file, "bytes differ");
    assert_eq!(s.requests, 3, "1 MiB chunks: 3 requests for ~3 MiB");
    assert_eq!(s.read_cb(&mut [0u8; 8]), 0, "EOF");
    for r in w.seen.lock().unwrap().iter() {
        assert!(r.contains("\r\nRange: bytes="), "{r}");
        assert!(r.contains("\r\nX-Wire-Max-Bytes: "), "{r}");
        assert!(r.contains("\r\nX-Wire-Timeout-Ms: 30000\r\n"), "{r}");
    }
    assert!(qf14b_has(&w.seen.lock().unwrap()[0], "Range: bytes=0-1048575"));
}

#[test]
fn qm1e_200_fallback_is_forward_only_and_backward_seek_restarts() {
    let file = qm1e_file(3 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ignore_range: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert!(s.is_forward_only() && !s.seekable());
    assert_eq!(s.size(), None);
    assert_eq!(s.size_cb(), -1);
    // request 0: capped attempt (502 too-large), request 1: the streaming GET
    {
        let seen = w.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(!seen[1].contains("X-Wire-Max-Bytes"), "{}", seen[1]);
        assert!(seen[1].contains("X-Wire-Timeout-Ms: 3600000"), "{}", seen[1]);
    }
    assert!(qm1e_read_all(&mut s, 1000) == file[..1000]);
    s.seek(2_000_000).unwrap();
    assert!(qm1e_read_all(&mut s, 500) == file[2_000_000..2_000_500], "forward seek discards");
    s.seek(10).unwrap();
    assert!(qm1e_read_all(&mut s, 100) == file[10..110], "backward seek restarts from 0");
    assert_eq!(w.seen.lock().unwrap().len(), 3);
}

#[test]
fn qm1e_small_200_body_is_served_whole_and_seekable() {
    let file = qm1e_file(5000);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ignore_range: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert!(s.seekable());
    assert_eq!(s.size(), Some(5000));
    s.seek(4000).unwrap();
    assert!(qm1e_read_all(&mut s, 5000) == file[4000..]);
    s.seek(0).unwrap();
    assert!(qm1e_read_all(&mut s, 10) == file[..10]);
    assert_eq!(w.seen.lock().unwrap().len(), 1);
}

#[test]
fn qm1e_416_is_eof_even_when_the_file_shrinks() {
    let w = qm1e_wire(Qm1eCfg { file: vec![], ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.size(), Some(0));
    assert_eq!(s.read(&mut [0u8; 16]).unwrap(), 0);

    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(3 * 1024 * 1024), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    w.cfg.lock().unwrap().file = qm1e_file(100);
    s.seek(2 * 1024 * 1024).unwrap();
    assert_eq!(s.size(), Some(3 * 1024 * 1024));
    // The file shrank: the next fetch answers 416 `bytes */100`.
    assert_eq!(s.read(&mut [0u8; 16]).unwrap(), 0);
    assert_eq!(s.size(), Some(100));
}

#[test]
fn qm1e_short_206_body_is_retried_once_from_the_new_position() {
    let big = qm1e_file(2 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: big.clone(), truncate_once: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let got = qm1e_read_all(&mut s, big.len());
    assert!(got == big, "short body healed by one retry");
    // open + short second chunk + its retry
    assert_eq!(w.seen.lock().unwrap().len(), 3);
    let retry = w.seen.lock().unwrap()[2].clone();
    let line = retry.lines().find(|l| l.starts_with("Range:")).unwrap().to_string();
    let start: u64 = line["Range: bytes=".len()..].split('-').next().unwrap().parse().unwrap();
    assert!(start > CHUNK_SMALL && start < 2 * CHUNK_SMALL, "retry continues after the bytes that arrived: {line}");
}

#[test]
fn qm1e_short_206_on_a_seek_miss_also_heals() {
    // Every ranged answer from request #1 on is short: #1 truncated, retry (#2) also short.
    let big = qm1e_file(2 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: big.clone(), truncate_once: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    s.seek(CHUNK_SMALL).unwrap();
    // idx 1 is truncated, its retry (idx 2) is whole: heals. A third miss is whole too.
    assert!(qm1e_read_all(&mut s, 10) == big[CHUNK_SMALL as usize..CHUNK_SMALL as usize + 10]);
}

#[test]
fn qm1e_wrong_content_range_start_fails() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(5000), shift_start: 1, ..Default::default() });
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).err().expect("must fail");
    assert!(matches!(e, MediaNetError::BadRange(_)), "{e:?}");
}

#[test]
fn qm1e_redirect_is_pinned_and_a_cross_origin_later_final_url_fails() {
    let file = qm1e_file(3 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg {
        file: file.clone(),
        final_urls: vec![(0, "https://cdn2.example/real.mp4".into()), (1, "https://cdn2.example/real.mp4".into()), (2, "https://evil.example/x.mp4".into())],
        ..Default::default()
    });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut got = 0;
    while got < CHUNK_SMALL as usize {
        got += s.read(&mut buf[..CHUNK_SMALL as usize - got]).unwrap();
    }
    got = 0;
    while got < 10 {
        got += s.read(&mut buf[..10 - got]).unwrap();
    }
    // chunk 2 (request 1) is pinned to the first final URL
    let r1 = w.seen.lock().unwrap()[1].clone();
    assert!(r1.starts_with("GET https://cdn2.example/real.mp4 "), "{r1}");
    // chunk 3 (request 2) comes back from another origin
    let e = loop {
        match s.read(&mut buf) {
            Ok(0) => panic!("EOF before the failing chunk"),
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    assert!(matches!(e, MediaNetError::MixedOrigin { .. }), "{e:?}");
}

#[test]
fn qm1e_no_cors_sends_the_jar_and_anonymous_sends_cookies_off_and_origin() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(100), ..Default::default() });
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let r = w.seen.lock().unwrap()[0].clone();
    assert!(qf14b_has(&r, "X-Wire-Jar: scratchy") && qf14b_has(&r, "X-Wire-Top-Site: https://page.example"), "{r}");
    assert!(!r.contains("X-Wire-Cookies: off") && !r.contains("\r\nOrigin:"), "{r}");

    // CORS without ACAO: fails after the first response.
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::Anonymous).err().expect("no ACAO must fail");
    assert!(matches!(e, MediaNetError::Cors(_)), "{e:?}");
    let r = w.seen.lock().unwrap()[1].clone();
    assert!(qf14b_has(&r, "X-Wire-Cookies: off") && qf14b_has(&r, "Origin: https://page.example"), "{r}");
    assert!(!r.contains("X-Wire-Jar"), "{r}");

    w.cfg.lock().unwrap().acao = Some("*".into());
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::Anonymous).expect("ACAO * is fine without credentials");
    // use-credentials: the jar is on, `*` is not enough, exact origin + ACAC is.
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::UseCredentials).err().unwrap();
    assert!(matches!(e, MediaNetError::Cors(_)), "{e:?}");
    {
        let mut c = w.cfg.lock().unwrap();
        c.acao = Some("https://page.example".into());
        c.acac = true;
    }
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::UseCredentials).expect("exact ACAO + ACAC");
    let r = w.seen.lock().unwrap().last().unwrap().clone();
    assert!(qf14b_has(&r, "X-Wire-Jar: scratchy") && qf14b_has(&r, "Origin: https://page.example"), "{r}");
}

#[test]
fn qm1e_mixed_content_and_bad_ports_are_blocked_before_any_request() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(100), ..Default::default() });
    let e = qm1e_open(&w, "http://media.example/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    let e = qm1e_open(&w, "https://media.example:25/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    let e = qm1e_open(&w, "ftp://media.example/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    assert!(w.seen.lock().unwrap().is_empty(), "nothing reached wire");
}

#[test]
fn qm1e_chunks_grow_from_1_to_4_mib_and_shrink_back_after_a_seek() {
    let file = qm1e_file(12 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    qm1e_read_all(&mut s, CHUNK_SMALL as usize + 1); // second chunk fetched
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    qm1e_read_all(&mut s, CHUNK_SMALL as usize); // third contiguous miss
    assert_eq!(s.last_chunk_len, CHUNK_LARGE, "4 MiB after two contiguous fetches");
    let r = w.seen.lock().unwrap().last().unwrap().clone();
    assert!(r.contains(&format!("Range: bytes={}-{}", 2 * CHUNK_SMALL, 2 * CHUNK_SMALL + CHUNK_LARGE - 1)), "{r}");
    // a seek far away -> back to 1 MiB
    s.seek(11 * 1024 * 1024).unwrap();
    let mut b = [0u8; 100];
    s.read(&mut b).unwrap();
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    assert!(b == file[11 * 1024 * 1024..11 * 1024 * 1024 + 100]);
}

#[test]
fn qm1e_cache_hit_serves_without_a_request() {
    let file = qm1e_file(6 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(w.seen.lock().unwrap().len(), 1);
    // tail (moov-at-end), then back to 0: chunk 0 is still cached
    s.seek(file.len() as u64 - 4096).unwrap();
    assert!(qm1e_read_all(&mut s, 4096) == file[file.len() - 4096..]);
    assert_eq!(w.seen.lock().unwrap().len(), 2);
    s.seek(0).unwrap();
    assert!(qm1e_read_all(&mut s, 70000) == file[..70000]);
    s.seek(500_000).unwrap();
    assert!(qm1e_read_all(&mut s, 1000) == file[500_000..501_000]);
    assert_eq!(w.seen.lock().unwrap().len(), 2, "cache hits made no request");
}

#[test]
fn qm1e_cancel_stops_a_blocked_read_at_once() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(3 * 1024 * 1024), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    w.cfg.lock().unwrap().hold = true;
    s.seek(2 * 1024 * 1024).unwrap();
    let h = s.cancel_handle();
    let t0 = std::time::Instant::now();
    let th = thread::spawn(move || {
        let r = s.read(&mut [0u8; 16]);
        (r, s)
    });
    thread::sleep(std::time::Duration::from_millis(300));
    h.cancel();
    let (r, mut s) = th.join().unwrap();
    assert_eq!(r, Err(MediaNetError::Cancelled));
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(s.read_cb(&mut [0u8; 16]), -1, "later reads fail without a request");
    assert_eq!(w.seen.lock().unwrap().len(), 2);
}

#[test]
fn qm1e_other_status_is_a_plain_error() {
    use std::os::unix::net::UnixListener;
    let path = std::env::temp_dir().join(format!("qm1e-{}-status.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).unwrap();
    thread::spawn(move || {
        for st in l.incoming() {
            let Ok(mut st) = st else { return };
            let mut b = [0u8; 4096];
            let _ = Read::read(&mut st, &mut b);
            let _ = st.write_all(b"HTTP/1.1 404 Not Found\r\n\r\nnope");
        }
    });
    let e = MediaStream::open(path, qm1e_src(QM1E_URL, QM1E_PAGE, MediaMode::NoCors), RequestPolicy::default()).err().unwrap();
    assert_eq!(e, MediaNetError::Status(404));
}

// ---- QAN1a: @keyframes + animation longhands (parse only) ----

fn qan1a_anims(css: &str, body: &str) -> Vec<Animation> {
    let doc = parse_html(&format!("<html><body>{body}</body></html>"));
    let rules = parse_stylesheet(css);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    st.animations.map(|a| a.resolve()).unwrap_or_default()
}

#[test]
fn qan1a_shorthand_token_order() {
    let a = qan1a_anims("div{animation: 2s ease-in 500ms 3 alternate both paused foo}", "<div></div>");
    assert_eq!(a.len(), 1);
    let a = &a[0];
    assert_eq!(a.name.as_deref(), Some("foo"));
    assert_eq!(a.duration, 2000.0);
    assert_eq!(a.delay, 500.0);
    assert_eq!(a.iterations, 3.0);
    assert_eq!(a.direction, AnimDirection::Alternate);
    assert_eq!(a.fill, AnimFill::Both);
    assert_eq!(a.play_state, AnimPlayState::Paused);
    assert_eq!(a.timing, TimingFunction::CubicBezier(0.42, 0.0, 1.0, 1.0));
    // Name first, negative delay, infinite, cubic-bezier.
    let a = qan1a_anims("div{animation: bar 1s cubic-bezier(0,1,1,0) -500s infinite}", "<div></div>");
    assert_eq!(a[0].name.as_deref(), Some("bar"));
    assert_eq!(a[0].delay, -500000.0);
    assert!(a[0].iterations.is_infinite());
    assert_eq!(a[0].timing, TimingFunction::CubicBezier(0.0, 1.0, 1.0, 0.0));
    // Defaults.
    let a = qan1a_anims("div{animation: foo}", "<div></div>");
    assert_eq!(a[0].duration, 0.0);
    assert_eq!(a[0].timing, TimingFunction::EASE);
    assert_eq!(a[0].iterations, 1.0);
    assert_eq!(a[0].fill, AnimFill::None);
    // `none` and a keyword-like name.
    assert_eq!(qan1a_anims("div{animation: none}", "<div></div>")[0].name, None);
    assert_eq!(qan1a_anims("div{animation: foo none}", "<div></div>")[0].fill, AnimFill::None);
}

#[test]
fn qan1a_comma_lists_and_longhand_cycling() {
    let a = qan1a_anims("div{animation: a 1s, b 2s linear 1s}", "<div></div>");
    assert_eq!(a.len(), 2);
    assert_eq!((a[0].name.as_deref(), a[0].duration), (Some("a"), 1000.0));
    assert_eq!((a[1].name.as_deref(), a[1].duration, a[1].delay), (Some("b"), 2000.0, 1000.0));
    assert_eq!(a[1].timing, TimingFunction::Linear);
    let a = qan1a_anims(
        "div{animation-name: a, b, c; animation-duration: 1s, 2s; animation-direction: reverse}",
        "<div></div>",
    );
    assert_eq!(a.len(), 3);
    assert_eq!(a.iter().map(|x| x.duration).collect::<Vec<_>>(), [1000.0, 2000.0, 1000.0]);
    assert!(a.iter().all(|x| x.direction == AnimDirection::Reverse));
}

#[test]
fn qan1a_webkit_aliases_and_invalid_dropped() {
    let a = qan1a_anims("div{-webkit-animation: foo 1s; -webkit-animation-delay: 2s}", "<div></div>");
    assert_eq!((a[0].duration, a[0].delay), (1000.0, 2000.0));
    // Invalid declarations are dropped, the rest of the sheet survives.
    let a = qan1a_anims(
        "div{animation: foo 1s; animation-duration: -1s; animation-timing-function: cubic-bezier(2,0,1,1); animation: 1s 2s 3s x}",
        "<div></div>",
    );
    assert_eq!((a[0].name.as_deref(), a[0].duration), (Some("foo"), 1000.0));
    assert_eq!(a[0].timing, TimingFunction::EASE);
}

#[test]
fn qan1a_timing_functions() {
    let t = |v: &str| parse_animation_value("animation-timing-function", v).map(|d| d.timing.unwrap()[0]);
    assert_eq!(t("linear"), Ok(TimingFunction::Linear));
    assert_eq!(t("ease-out"), Ok(TimingFunction::CubicBezier(0.0, 0.0, 0.58, 1.0)));
    assert_eq!(t("ease-in-out"), Ok(TimingFunction::CubicBezier(0.42, 0.0, 0.58, 1.0)));
    assert_eq!(t("steps(4)"), Ok(TimingFunction::Steps(4, StepPos::JumpEnd)));
    assert_eq!(t("steps(2, start)"), Ok(TimingFunction::Steps(2, StepPos::JumpStart)));
    assert_eq!(t("steps(3, jump-both)"), Ok(TimingFunction::Steps(3, StepPos::JumpBoth)));
    assert_eq!(t("steps(1, jump-none)"), Err(()));
    assert_eq!(t("steps(0)"), Err(()));
    assert_eq!(t("step-start"), Ok(TimingFunction::Steps(1, StepPos::JumpStart)));
    assert_eq!(t("step-end"), Ok(TimingFunction::Steps(1, StepPos::JumpEnd)));
    assert_eq!(t("cubic-bezier(0.1, 0.7, 1, 0.1)"), Ok(TimingFunction::CubicBezier(0.1, 0.7, 1.0, 0.1)));
}

#[test]
fn qan1a_inherit_initial_per_longhand() {
    let doc = parse_html("<html><body><section><div></div></section></body></html>");
    let rules = parse_stylesheet(
        "section{animation: a 3s linear 2s} div{animation-name: inherit; animation-duration: inherit; animation-delay: initial; animation-timing-function: inherit}",
    );
    let s = find_element_by_tag(&doc, "section").unwrap();
    let ss = resolve_style(&s, &rules, ROOT_STYLE, 800.0);
    let d = find_element_by_tag(&doc, "div").unwrap();
    let inh = InheritedStyle { parent: Some(ss), ..ROOT_STYLE };
    let ds = resolve_style(&d, &rules, inh, 800.0);
    let a = ds.animations.unwrap().resolve();
    assert_eq!(a[0].name.as_deref(), Some("a"));
    assert_eq!(a[0].duration, 3000.0);
    assert_eq!(a[0].delay, 0.0);
    assert_eq!(a[0].timing, TimingFunction::Linear);
    assert_eq!(a[0].iterations, 1.0);
    // Not inherited by default.
    let rules = parse_stylesheet("section{animation: a 3s}");
    let ds = resolve_style(&d, &rules, InheritedStyle { parent: Some(ss), ..ROOT_STYLE }, 800.0);
    assert!(ds.animations.is_none());
}

#[test]
fn qan1a_keyframes_registry() {
    let (_, kf) = parse_stylesheet_with_keyframes(
        "@keyframes a { from { opacity: 0 } 0%, 50% { opacity: .5 } to { opacity: 1 !important; color: red } }
         @-webkit-keyframes w { 100% { opacity: 1 } }
         @keyframes \"s\" { to { opacity: 1 } }
         @keyframes none { to { opacity: 1 } }
         @keyframes bad { 150% { opacity: 1 } 10% { opacity: 1 } }
         @keyframes dup { to { opacity: 0.1 } } @keyframes dup { to { opacity: 0.9 } }
         p { color: red }",
    );
    let a = registry_get(&kf, "a", 800.0).expect("a");
    assert_eq!(a.iter().map(|k| k.offset).collect::<Vec<_>>(), [0.0, 0.0, 0.5, 1.0]);
    assert!(a[3].decls.opacity.is_none(), "!important inside a keyframe is dropped");
    assert!(a[3].decls.color.is_some());
    assert_eq!(a[2].decls.opacity, Some(0.5));
    assert!(registry_get(&kf, "w", 800.0).is_some() && registry_get(&kf, "s", 800.0).is_some());
    assert!(registry_get(&kf, "none", 800.0).is_none());
    assert_eq!(registry_get(&kf, "dup", 800.0).unwrap()[0].decls.opacity, Some(0.9), "last definition wins");
}

#[test]
fn qan1a_keyframes_inside_media_and_supports() {
    let (rules, kf) = parse_stylesheet_with_keyframes(
        "@media all { @keyframes yes { to { opacity: 1 } } }
         @media not all { @keyframes no { to { opacity: 1 } } }
         @supports (display: grid) { @keyframes sy { to { opacity: 1 } } }
         @supports (foo: bar) { @keyframes sn { to { opacity: 1 } } }
         div { color: blue }",
    );
    assert!(registry_get(&kf, "yes", 800.0).is_some() && registry_get(&kf, "sy", 800.0).is_some());
    assert!(registry_get(&kf, "no", 800.0).is_none() && registry_get(&kf, "sn", 800.0).is_none());
    assert_eq!(rules.len(), 1, "the sheet after the keyframes still parses");
}

#[test]
fn qan1a_registry_reaches_the_cascade_cache() {
    let doc = parse_html("<html><head><style>@keyframes k { to { opacity: 1 } } div { animation: k 1s }</style></head><body><div></div></body></html>");
    let _ = build_layout_tree(&doc, 800.0);
    assert!(lookup_keyframes("k", 800.0).is_some());
    let _ = build_layout_tree(&doc, 800.0);
    assert!(lookup_keyframes("k", 800.0).is_some(), "cache hit keeps the registry");
}

#[test]
fn qan1a_computed_style_stays_small() {
    fn is_copy<T: Copy>() {}
    is_copy::<ComputedStyle>();
    eprintln!("size_of ComputedStyle = {}", std::mem::size_of::<ComputedStyle>());
}

// ---------- Q30a: form controls get real boxes ----------

fn q30a_tree(html: &str) -> LayoutBox {
    let doc = parse_html(&format!("<html><body style=\"margin:0\">{html}</body></html>"));
    build_layout_tree(&doc, 1000.0)
}

fn q30a_texts(b: &LayoutBox, out: &mut Vec<(String, TextColor)>) {
    match &b.content {
        BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().for_each(|k| q30a_texts(k, out)),
        BoxContent::Inline(items) => {
            for i in items {
                match i {
                    InlineItem::Text(t, s) => out.push((t.clone(), s.color)),
                    InlineItem::Embedded(e) => q30a_texts(e, out),
                    _ => {}
                }
            }
        }
        BoxContent::Image(_) => {}
    }
}

fn q30a_all_text(html: &str) -> Vec<(String, TextColor)> {
    let mut v = Vec::new();
    q30a_texts(&q30a_tree(html), &mut v);
    v
}

/// (w, h) of every laid-out box under the body (inline-blocks included).
fn q30a_sizes(html: &str) -> Vec<(f32, f32)> {
    fn collect(l: &Laid, out: &mut Vec<(f32, f32)>) {
        out.push((l.w, l.h));
        match &l.content {
            LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|k| collect(k, out)),
            LaidContent::MixedInline(items) => {
                for it in items {
                    if let MixedPositioned::Embedded(e) = it {
                        collect(e, out);
                    }
                }
            }
            _ => {}
        }
    }
    let tree = q30a_tree(html);
    let mut fs = new_font_system_with(&[]);
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(&tree, &mut fs, 0.0, 0.0, 1000.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
    let mut v = Vec::new();
    collect(&laid, &mut v);
    v
}

#[test]
fn q30a_text_input_box_size_follows_size_attr_and_css() {
    // 20 chars default: 20 * 13.3333 * 0.55 content + 2*2 padding + 2*1 border.
    let default = q30a_sizes("<input>");
    let w20 = 20.0 * 13.3333 * 0.55 + 4.0 + 2.0;
    assert!(default.iter().any(|&(w, h)| (w - w20).abs() < 0.2 && h > 14.0 && h < 30.0), "{default:?} want w {w20}");
    let s10 = q30a_sizes("<input size=10>");
    let w10 = 10.0 * 13.3333 * 0.55 + 6.0;
    assert!(s10.iter().any(|&(w, _)| (w - w10).abs() < 0.2), "{s10:?}");
    let css = q30a_sizes("<input style=\"width:100px\">");
    assert!(css.iter().any(|&(w, _)| (w - 106.0).abs() < 0.2), "{css:?}");
    let bb = q30a_sizes("<input style=\"width:100px;box-sizing:border-box\">");
    assert!(bb.iter().any(|&(w, _)| (w - 100.0).abs() < 0.2), "{bb:?}");
    for ty in ["text", "search", "email", "url", "tel", "password", "number"] {
        let v = q30a_sizes(&format!("<input type={ty}>"));
        assert!(v.iter().any(|&(w, _)| (w - w20).abs() < 0.2), "{ty}: {v:?}");
    }
}

/// Q30d: the text of an editable control is no longer in the layout tree; the
/// painter reads what it shows (live value, placeholder, bullets) from forms_text.rs.
/// What `PaintText` would draw for the first `<input>`/`<textarea>` of `html` after a layout build.
fn q30a_shown(html: &str, password: bool) -> (String, bool, Option<TextColor>) {
    clear_text_state();
    let doc = parse_html(&format!("<html><body style=\"margin:0\">{html}</body></html>"));
    let _ = build_layout_tree(&doc, 1000.0);
    let h = find_element_by_tag(&doc, "input").or_else(|| find_element_by_tag(&doc, "textarea")).expect("control");
    let i = text_paint_info(node_id(&h), password).expect("registered by the build");
    (i.display, i.placeholder, i.ph_color)
}

#[test]
fn q30a_value_placeholder_and_password_text_in_the_tree() {
    let v = q30a_shown("<input value=\"hello\">", false);
    assert_eq!((v.0.as_str(), v.1), ("hello", false));
    let p = q30a_shown("<input placeholder=\"Search me\">", false);
    assert_eq!((p.0.as_str(), p.1, p.2), ("Search me", true, Some(TextColor::rgb(117, 117, 117))));
    let both = q30a_shown("<input value=\"v\" placeholder=\"ph\">", false);
    assert_eq!((both.0.as_str(), both.1), ("v", false));
    let pw = q30a_shown("<input type=password value=\"secret\">", true);
    assert_eq!(pw.0, "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}");
    clear_text_state();
}

fn q30a_dot_count(b: &LayoutBox, n: &mut usize) {
    if let BoxContent::Blocks(c) = &b.content {
        for k in c {
            if matches!(k.style.width, Some(BoxLength::Px(w)) if w == 5.0) && k.style.background_color.is_some() {
                *n += 1;
            }
            q30a_dot_count(k, n);
        }
    }
    if let BoxContent::Inline(items) = &b.content {
        for i in items {
            if let InlineItem::Embedded(e) = i {
                q30a_dot_count(e, n);
            }
        }
    }
}

#[test]
fn q30a_checkbox_and_radio_are_13px_and_checked_paints_a_mark() {
    for ty in ["checkbox", "radio"] {
        let v = q30a_sizes(&format!("<input type={ty}>"));
        assert!(v.contains(&(13.0, 13.0)), "{ty}: {v:?}");
    }
    // Q30c: the mark is painted from live state, so the tree only carries the
    // `check_widget` bits (kind + whether the `checked` attribute was set).
    fn bits(html: &str) -> Vec<u8> {
        fn walk(b: &LayoutBox, out: &mut Vec<u8>) {
            if b.style.check_widget != 0 {
                out.push(b.style.check_widget);
            }
            match &b.content {
                BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().for_each(|k| walk(k, out)),
                BoxContent::Inline(items) => {
                    for i in items {
                        if let InlineItem::Embedded(e) = i {
                            walk(e, out);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&q30a_tree(html), &mut out);
        out
    }
    assert_eq!(bits("<input type=checkbox>"), vec![CHECK_BOX]);
    assert_eq!(bits("<input type=checkbox checked>"), vec![CHECK_BOX | CHECK_ATTR]);
    assert_eq!(bits("<input type=radio checked>"), vec![CHECK_RADIO | CHECK_ATTR]);
    assert_eq!(bits("<input type=radio>"), vec![CHECK_RADIO]);
    let mut dots = 0;
    q30a_dot_count(&q30a_tree("<input type=radio checked>"), &mut dots);
    assert_eq!(dots, 0, "the radio dot is painted from live state, not built into the tree");
    assert!(bits("<input type=checkbox checked style=\"appearance:none\">").is_empty());
    assert!(bits("<input type=checkbox checked style=\"-webkit-appearance:none\">").is_empty());
}

#[test]
fn q30a_buttons_show_their_label() {
    let t = q30a_all_text("<input type=submit value=\"Go\"><input type=submit><input type=reset><input type=button value=\"Press\"><button>Label</button>");
    for want in ["Go", "Submit", "Reset", "Press", "Label"] {
        assert!(t.iter().any(|(s, _)| s == want), "missing {want}: {t:?}");
    }
    let v = q30a_sizes("<input type=submit value=\"Go\">");
    assert!(v.iter().any(|&(w, h)| w > 20.0 && w < 60.0 && h > 15.0 && h < 30.0), "{v:?}");
}

#[test]
fn q30a_hidden_input_is_not_laid_out() {
    let none = q30a_sizes("<input type=hidden value=\"x\">");
    let with = q30a_sizes("<input type=text>");
    assert!(with.len() > none.len(), "{none:?} vs {with:?}");
    let upper = q30a_sizes("<input type=HIDDEN>");
    assert_eq!(upper.len(), none.len());
    assert!(q30a_all_text("<input type=hidden value=\"secret\">").iter().all(|(t, _)| !t.contains("secret")));
}

#[test]
fn q30a_textarea_rows_cols_text_and_placeholder() {
    // content: cols * 13.3333*0.6, rows * round(13.3333*1.15)=15; + padding 4 + border 2.
    let v = q30a_sizes("<textarea rows=3 cols=30>hi</textarea>");
    let (w, h) = (30.0 * 13.3333 * 0.6 + 6.0, 45.0 + 6.0);
    assert!(v.iter().any(|&(a, b)| (a - w).abs() < 0.2 && (b - h).abs() < 0.2), "{v:?} want {w}x{h}");
    let d = q30a_sizes("<textarea></textarea>");
    let (w, h) = (20.0 * 13.3333 * 0.6 + 6.0, 30.0 + 6.0);
    assert!(d.iter().any(|&(a, b)| (a - w).abs() < 0.2 && (b - h).abs() < 0.2), "{d:?}");
    assert_eq!(q30a_shown("<textarea>Line one\nLine two</textarea>", false).0, "Line one\nLine two");
    let p = q30a_shown("<textarea placeholder=\"Write\"></textarea>", false);
    assert_eq!((p.0.as_str(), p.1, p.2), ("Write", true, Some(TextColor::rgb(117, 117, 117))));
    clear_text_state();
}

#[test]
fn q30a_select_shows_selected_option_and_arrow_not_the_others() {
    let t = q30a_all_text("<select><option>Alpha<option selected>Beta<option>Gamma</select>");
    assert!(t.iter().any(|(s, _)| s == "Beta"), "{t:?}");
    assert!(!t.iter().any(|(s, _)| s == "Alpha" || s == "Gamma"), "{t:?}");
    assert!(t.iter().any(|(s, _)| s.contains('\u{2304}')), "arrow: {t:?}");
    let first = q30a_all_text("<select><option>One<option>Two</select>");
    assert!(first.iter().any(|(s, _)| s == "One") && !first.iter().any(|(s, _)| s == "Two"));
    let none = q30a_all_text("<select style=\"appearance:none\"><option>One</select>");
    assert!(none.iter().any(|(s, _)| s == "One") && !none.iter().any(|(s, _)| s.contains('\u{2304}')));
    let lb = q30a_all_text("<select multiple size=3><option>A<option>B<option>C</select>");
    for o in ["A", "B", "C"] {
        assert!(lb.iter().any(|(s, _)| s == o), "{lb:?}");
    }
}

fn q30a_embedded_styles(b: &LayoutBox, out: &mut Vec<ComputedStyle>) {
    match &b.content {
        BoxContent::Blocks(c) => c.iter().for_each(|k| q30a_embedded_styles(k, out)),
        BoxContent::Inline(items) => {
            for i in items {
                if let InlineItem::Embedded(e) = i {
                    out.push(e.style);
                }
            }
        }
        _ => {}
    }
}

#[test]
fn q30a_disabled_style_and_appearance_none_keep_author_border() {
    let tree = q30a_tree("<input disabled><input style=\"border:0;appearance:none\">");
    let mut v = Vec::new();
    q30a_embedded_styles(&tree, &mut v);
    assert_eq!(v.len(), 2);
    // The :disabled rule in default.css greys the text.
    assert_eq!(v[0].color, TextColor::rgb(0x6d, 0x6d, 0x6d));
    assert_eq!(v[0].border_width, [1.0; 4]);
    // The author's border:0 beats the UA border: no double border.
    assert_eq!(v[1].border_width, [0.0; 4]);
    assert!(v[1].appearance_none);
}

#[test]
fn q30a_inline_form_row_shares_a_line() {
    let v = q30a_sizes("<p style=\"margin:0\">Search <input> <button>Go</button></p>");
    let tallest = v.iter().map(|&(_, h)| h).fold(0.0f32, f32::max);
    assert!(tallest < 40.0, "a one-line form row must stay one line high: {v:?}");
}

#[test]
fn q30a_rounded_border_keeps_its_bottom_row_at_fractional_height() {
    // Regression found by Q30a: a 1px bottom border of a rounded box vanished
    // when y/h were fractional (the last row was classed as an inner row).
    let (w, h) = (40u32, 30u32);
    let mut px = vec![0xFFFFFFFFu32; (w * h) as usize];
    fill_rounded_ring(&mut px, w, h, 2.0, 3.4, 30.0, 19.6, 2.0, 1.0, TextColor::rgb(0, 0, 0), None);
    let bottom_row = (3.4f32 + 19.6).round() as u32 - 1;
    let idx = (bottom_row * w + 15) as usize;
    assert_ne!(px[idx], 0xFFFFFFFF, "bottom border row {bottom_row} missing");
}

/// R32 / QP1: `shape_spans` must resolve font families with
/// `font_system.db()`, never `db_mut()`. cosmic-text's `db_mut()` clears the
/// font-match cache on every call (the cache cannot be read from outside
/// cosmic-text), so one `db_mut()` per span made every shape rebuild the
/// match list and reopen ~71 font files. `new_font_system_with` never calls
/// `db_mut`, so only that bug trips this guard. The needle is split so this
/// test's own source does not contain it (the guard reads layout.rs only).
#[test]
fn layout_shaping_never_wipes_font_cache() {
    let src = include_str!("layout.rs");
    let needle = concat!(".db", "_mut()");
    assert!(!src.contains(needle), "layout.rs calls `{needle}`, which clears cosmic-text's font-match cache (R32): use `font_system.db()`");
}

/// Q30b helper: lay out `doc` at width 600 and return the pieces a hit test needs.
fn q30b_layout(doc: &Handle) -> (Laid, ShapeCache) {
    let tree = build_layout_tree(doc, 600.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 600.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    (laid, cache)
}

fn q30b_by_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| *v == *id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| q30b_by_id(c, id))
}

#[test]
fn q30b_focusable_controls_and_tab_order() {
    let doc = parse_html(
        r#"<html><body><input id=a><input type=hidden id=h><input disabled id=d><button id=b>x</button>
        <select id=s><option>1</option></select><textarea id=t></textarea><input tabindex=-1 id=m>
        <fieldset disabled><legend><input id=lg></legend><input id=fs><button id=fb>y</button></fieldset>
        <a href=/x id=l>l</a></body></html>"#,
    );
    let ids: Vec<String> =
        focusable_in_tab_order(&doc).iter().map(|h| element_attr(h, "id").unwrap().to_string()).collect();
    assert_eq!(ids, ["a", "b", "s", "t", "lg", "l"], "disabled, hidden, tabindex=-1 and disabled-fieldset controls skipped");
    let m = q30b_by_id(&doc, "m").unwrap();
    assert!(!is_focusable(&m) && is_click_focusable(&m), "tabindex=-1: click-focusable only");
    assert!(is_disabled_control(&q30b_by_id(&doc, "fs").unwrap()));
    assert!(!is_click_focusable(&q30b_by_id(&doc, "d").unwrap()));
}

#[test]
fn q30b_label_forwarding_targets() {
    let doc = parse_html(
        r#"<html><body><label for=a id=l1>x</label><input id=a>
        <label id=l2><span id=sp>y</span><input id=b></label><label for=nope id=l3>z</label></body></html>"#,
    );
    let tgt = |l: &str| {
        label_control(&doc, &q30b_by_id(&doc, l).unwrap()).and_then(|h| element_attr(&h, "id").map(|v| v.to_string()))
    };
    assert_eq!(tgt("l1").as_deref(), Some("a"));
    assert_eq!(tgt("l2").as_deref(), Some("b"));
    assert_eq!(tgt("l3"), None);
    let sp = q30b_by_id(&doc, "sp").unwrap();
    assert!(enclosing_label(&sp).is_some());
}

#[test]
fn q30b_hit_test_returns_the_control_for_its_box() {
    let doc = parse_html(
        r#"<html><body><p>text</p><input id=i placeholder="search here"><button id=b>Go</button><textarea id=t>hello</textarea></body></html>"#,
    );
    let (laid, cache) = q30b_layout(&doc);
    for id in ["i", "b", "t"] {
        let h = q30b_by_id(&doc, id).unwrap();
        let geom = node_geometry(&laid, &cache, node_id(&h));
        let &(x, y, w, hh) = geom.rects.first().unwrap_or_else(|| panic!("no geometry for {id}"));
        for (px, py) in [(x + w / 2.0, y + hh / 2.0), (x + 2.0, y + 2.0)] {
            let hit = hit_test_element(&laid, px, py, &cache, &HashMap::new(), 0.0, 600.0);
            let hit_h = find_node_by_id(&doc, hit).unwrap_or_else(|| panic!("{id}: nothing at {px},{py}"));
            let c = enclosing_control(&hit_h).unwrap_or_else(|| panic!("{id}: hit {}", element_tag(&hit_h)));
            assert_eq!(node_id(&c), node_id(&h), "{id} at {px},{py}");
        }
    }
}

#[test]
fn q30b_label_click_focuses_control_and_disabled_is_ignored() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let doc = parse_html(
        r#"<html><body><input id=i><label for=i id=l>name</label>
        <label id=w>wrap <input id=wi></label><button id=b>Go</button></body></html>"#,
    );
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(
        &doc,
        &[src("window.log=[]; ['i','wi','l','b'].forEach(function(n){var e=document.getElementById(n); e.addEventListener('click',function(){log.push('click:'+n)}); e.addEventListener('focus',function(){log.push('focus:'+n)}); e.addEventListener('blur',function(){log.push('blur:'+n)});});")],
        None,
    );
    let id_of = |app: &App| {
        app.interaction.focus.and_then(|f| find_node_by_id(&doc, f)).and_then(|h| element_attr(&h, "id").map(|v| v.to_string()))
    };
    let by = |id: &str| q30b_by_id(&doc, id).unwrap();
    assert!(click_focus_is_visible(&by("i")), "a text input click-focus matches :focus-visible");
    assert!(!click_focus_is_visible(&by("b")), "a button click-focus has no ring");
    assert!(app.forward_label_click(&by("l"), (0.0, 0.0)));
    assert_eq!(id_of(&app).as_deref(), Some("i"));
    assert!(app.interaction.focus_visible);
    assert!(app.forward_label_click(&by("w"), (0.0, 0.0)));
    assert_eq!(id_of(&app).as_deref(), Some("wi"));
    let log: String = app.js.as_ref().unwrap().eval_value("t", "log.join(',')").expect("log");
    for want in ["click:i", "click:wi", "blur:i", "focus:wi"] {
        assert!(log.contains(want), "{want} in {log}");
    }
    let d_doc = parse_html(r#"<html><body><label for=d id=ld>x</label><input id=d disabled></body></html>"#);
    let mut app2 = App::new_static(d_doc.clone(), event_loop.create_proxy(), None);
    assert!(app2.forward_label_click(&q30b_by_id(&d_doc, "ld").unwrap(), (0.0, 0.0)));
    assert!(app2.interaction.focus.is_none(), "a disabled control is never focused by a label");
}

#[test]
fn qp2_js_thread_shape_memo_is_safe_and_cleared_on_resize() {
    use crate::js_geom::GeomCache;
    let doc = parse_html("<html><body style='margin:0'><p id=a>First paragraph of text</p><p id=b>Second paragraph</p></body></html>");
    fn find(h: &Handle, id: &str) -> Option<Handle> {
        let is_it = match &h.data {
            NodeData::Element { attrs, .. } => attrs.borrow().iter().any(|a| &*a.name.local == "id" && &*a.value == id),
            _ => false,
        };
        if is_it {
            return Some(h.clone());
        }
        h.children.borrow().iter().find_map(|c| find(c, id))
    }
    SHAPE_MEMO.with(|m| m.borrow_mut().clear());
    let mut g = GeomCache::default();
    let (a, b) = (find(&doc, "a").unwrap(), find(&doc, "b").unwrap());
    let r1 = g.rect_of(&doc, 1, &b, 400.0, 300.0);
    assert!(SHAPE_MEMO.with(|m| !m.borrow().is_empty()), "forced layout must fill the memo");
    // A write that changes no text: same rects, still memoised.
    let r2 = g.rect_of(&doc, 2, &b, 400.0, 300.0);
    assert_eq!(r1, r2);
    assert!(SHAPE_MEMO.with(|m| !m.borrow().is_empty()));
    // A text change must show up (no stale shapes): b moves down.
    let a_text = a.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &a_text.data {
        *contents.borrow_mut() = "First paragraph of text that is now long enough to wrap onto several lines when laid out in a narrow viewport of four hundred pixels wide, and then some more words to be sure".into();
    }
    let r3 = g.rect_of(&doc, 3, &b, 400.0, 300.0);
    assert!(r3[1] > r1[1] + 10.0, "b must move down: {r1:?} -> {r3:?}");
    // A viewport change clears the memo.
    SHAPE_MEMO.with(|m| {
        let mut m = m.borrow_mut();
        let any = m.values().next().unwrap().clone();
        m.insert(0xdead, any);
    });
    g.rect_of(&doc, 4, &b, 300.0, 300.0);
    assert!(SHAPE_MEMO.with(|m| !m.borrow().contains_key(&0xdead)), "resize must clear the memo");
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
}

// ---- QAN1b: evaluating animations ----

/// Style of the first `div` at document time `now` ms (a fresh start-time
/// map each call, so the animation starts at `now`'s own zero).
fn qan1b_style(css: &str, now: f64, body: &str) -> ComputedStyle {
    let doc = parse_html(&format!("<html><body>{body}</body></html>"));
    let (rules, kf) = parse_stylesheet_with_keyframes(css);
    set_keyframes(Rc::new(kf));
    set_document_clock(0.0);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let _ = resolve_style(&d, &rules, ROOT_STYLE, 800.0); // records the start at 0
    set_document_clock(now);
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    set_document_clock(0.0);
    finish_animation_pass();
    finish_animation_pass();
    st
}

#[test]
fn qan1b_timing_functions() {
    let f = TimingFunction::CubicBezier(0.0, 1.0, 1.0, 0.0);
    assert!((f.ease(0.5) - 0.5).abs() < 1e-3);
    assert!((TimingFunction::Linear.ease(0.3) - 0.3).abs() < 1e-6);
    assert!((TimingFunction::EASE.ease(0.5) - 0.8024).abs() < 1e-2);
    let end = TimingFunction::Steps(4, StepPos::JumpEnd);
    assert_eq!(end.ease(0.0), 0.0);
    assert_eq!(end.ease(0.3), 0.25);
    assert_eq!(end.ease(0.99), 0.75);
    assert_eq!(end.ease(1.0), 1.0);
    let start = TimingFunction::Steps(2, StepPos::JumpStart);
    assert_eq!(start.ease(0.0), 0.5);
    assert_eq!(start.ease(0.6), 1.0);
}

#[test]
fn qan1b_color_interpolates_in_premultiplied_srgb() {
    let st = qan1b_style(
        "@keyframes k{from{background-color:rgb(0,200,0)}to{background-color:rgb(200,0,0)}} div{animation:k 1000s linear 0s}",
        500_000.0,
        "<div></div>",
    );
    assert_eq!(st.background_color, Some(TextColor::rgba(100, 100, 0, 255)));
    // transparent -> red: premultiplied keeps the colour red, alpha halves
    let st = qan1b_style(
        "@keyframes k{from{background-color:transparent}to{background-color:rgb(200,0,0)}} div{animation:k 1000s linear}",
        500_000.0,
        "<div></div>",
    );
    assert_eq!(st.background_color, Some(TextColor::rgba(200, 0, 0, 128)));
}

#[test]
fn qan1b_negative_delay_and_cubic_bezier_body_case() {
    let st = qan1b_style(
        "@keyframes k{from{background-color:rgb(0,200,0)}to{background-color:rgb(200,0,0)}} div{animation:k 1000000s cubic-bezier(0,1,1,0) -500000s}",
        0.0,
        "<div></div>",
    );
    let c = st.background_color.unwrap();
    assert!((c.r() as i32 - 100).abs() <= 1 && (c.g() as i32 - 100).abs() <= 1);
}

#[test]
fn qan1b_opacity_length_and_discrete_fields() {
    let st = qan1b_style(
        "@keyframes k{from{opacity:0;width:100px;margin-left:10px;display:block;z-index:0}to{opacity:1;width:200px;margin-left:20px;display:none;z-index:10}} div{animation:k 1s linear}",
        250.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.25).abs() < 1e-3);
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if (w - 125.0).abs() < 1e-3));
    assert!(matches!(st.margin.left, BoxLength::Px(m) if (m - 12.5).abs() < 1e-3));
    assert!(st.display != Display::None, "display is discrete: still A at 25 %");
    let st = qan1b_style(
        "@keyframes k{from{display:block}to{display:none}} div{animation:k 1s linear}",
        600.0,
        "<div></div>",
    );
    assert_eq!(st.display, Display::None);
}

#[test]
fn qan1b_mixed_units_are_discrete() {
    let st = qan1b_style(
        "@keyframes k{from{width:100px}to{width:50%}} div{animation:k 1s linear}",
        250.0,
        "<div></div>",
    );
    assert!(matches!(st.width, Some(BoxLength::Px(_))));
}

#[test]
fn qan1b_fill_modes() {
    let css = |fill: &str| {
        format!("@keyframes k{{from{{opacity:0.2}}to{{opacity:0.8}}}} div{{animation:k 1s linear 1s {fill}}}")
    };
    // before the delay
    assert_eq!(qan1b_style(&css("none"), 500.0, "<div></div>").opacity, 1.0);
    assert!((qan1b_style(&css("backwards"), 500.0, "<div></div>").opacity - 0.2).abs() < 1e-3);
    assert!((qan1b_style(&css("both"), 500.0, "<div></div>").opacity - 0.2).abs() < 1e-3);
    // after the end
    assert_eq!(qan1b_style(&css("none"), 3000.0, "<div></div>").opacity, 1.0);
    assert_eq!(qan1b_style(&css("backwards"), 3000.0, "<div></div>").opacity, 1.0);
    assert!((qan1b_style(&css("forwards"), 3000.0, "<div></div>").opacity - 0.8).abs() < 1e-3);
}

#[test]
fn qan1b_iterations_direction_and_paused() {
    let css = |extra: &str| format!("@keyframes k{{from{{opacity:0}}to{{opacity:1}}}} div{{animation:k 1s linear {extra}}}");
    // second iteration of an alternate animation runs backwards
    let st = qan1b_style(&css("2 alternate"), 1250.0, "<div></div>");
    assert!((st.opacity - 0.75).abs() < 1e-3);
    let st = qan1b_style(&css("infinite"), 10_250.0, "<div></div>");
    assert!((st.opacity - 0.25).abs() < 1e-3);
    let st = qan1b_style(&css("reverse"), 250.0, "<div></div>");
    assert!((st.opacity - 0.75).abs() < 1e-3);
    // paused: time is frozen at the start
    let st = qan1b_style(&css("paused"), 5000.0, "<div></div>");
    assert_eq!(st.opacity, 0.0);
}

#[test]
fn qan1b_per_keyframe_timing_and_steps() {
    let st = qan1b_style(
        "@keyframes k{0%{opacity:0;animation-timing-function:steps(2,end)}100%{opacity:1}} div{animation:k 1s linear}",
        600.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.5).abs() < 1e-3);
}

#[test]
fn qan1b_missing_endpoint_uses_the_base_style() {
    let st = qan1b_style(
        "@keyframes k{to{opacity:0}} div{opacity:0.5;animation:k 1s linear}",
        500.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.25).abs() < 1e-3);
}

#[test]
fn qan1b_animation_beats_inline_but_not_important() {
    let css = "@keyframes k{from{width:100px}to{width:100px}} div{animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, r#"<div style="width:10px"></div>"#);
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 100.0));
    let css = "@keyframes k{from{width:100px}to{width:100px}} div{width:10px !important;animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, "<div></div>");
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 10.0));
}

#[test]
fn qan1b_important_in_keyframe_is_ignored() {
    let css = "@keyframes k{from{width:100px !important}to{width:100px !important}} div{width:10px;animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, "<div></div>");
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 10.0));
}

#[test]
fn qan1b_pseudo_elements_animate() {
    let doc = parse_html("<html><body><div></div></body></html>");
    let (rules, kf) = parse_stylesheet_with_keyframes(
        "@keyframes k{from{opacity:0}to{opacity:1}} div::after{content:'x';animation:k 1s linear -500ms both}",
    );
    set_keyframes(Rc::new(kf));
    set_document_clock(0.0);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style_for(&d, &rules, ROOT_STYLE, 800.0, Some("after"));
    assert!((st.opacity - 0.5).abs() < 1e-3);
    finish_animation_pass();
}

#[test]
fn qan1b_no_animation_means_no_start_bookkeeping() {
    let doc = parse_html("<html><body><div></div></body></html>");
    let rules = parse_stylesheet("div{opacity:0.5}");
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    assert!(st.animations.is_none());
    assert_eq!(st.opacity, 0.5);
}

#[test]
fn qan1b_computed_style_stays_copy_report_size() {
    fn is_copy<T: Copy>() {}
    is_copy::<ComputedStyle>();
    eprintln!("size_of::<ComputedStyle>() = {}", std::mem::size_of::<ComputedStyle>());
}

// ---- Q30c: checkbox/radio live state ----

fn q30c_by(doc: &Handle, id: &str) -> Handle {
    q30b_by_id(doc, id).unwrap_or_else(|| panic!("no #{id}"))
}

#[test]
fn q30c_state_is_seeded_from_the_attribute_then_dirty() {
    clear_check_state();
    let doc = parse_html("<html><body><input id=a type=checkbox checked><input id=b type=checkbox><input id=t type=text checked></body></html>");
    let (a, b) = (q30c_by(&doc, "a"), q30c_by(&doc, "b"));
    assert!(live_checked(&a) && !live_checked(&b));
    assert!(set_checked(&a, false), "a change is reported");
    assert!(!live_checked(&a), "once dirty the attribute no longer controls it");
    assert!(!set_checked(&a, false), "no change, no report");
    // While not dirty the attribute still controls it.
    crate::js_dom::set_attr(&b, "checked", "");
    assert!(live_checked(&b));
    set_checked(&b, false);
    crate::js_dom::set_attr(&b, "checked", "");
    assert!(!live_checked(&b), "dirty: the attribute is ignored");
    assert_eq!(checkable_kind(&q30c_by(&doc, "t")), None);
    set_checked(&q30c_by(&doc, "t"), true);
    assert!(live_checked(&q30c_by(&doc, "t")), "the property round-trips on any input");
    // Indeterminate is independent of checked.
    set_indeterminate(&a, true);
    assert!(live_indeterminate(&a) && !live_checked(&a));
    clear_check_state();
    assert!(live_checked(&a) && !live_indeterminate(&a), "navigation clears live state");
}

#[test]
fn q30c_radio_groups_are_per_form_and_name() {
    clear_check_state();
    let doc = parse_html(
        r#"<html><body>
        <form id=f1><input type=radio name=x id=a checked><input type=radio name=x id=b><input type=radio name=y id=c checked></form>
        <form id=f2><input type=radio name=x id=d checked></form>
        <input type=radio name=x id=e checked><input type=radio id=n1 checked><input type=radio id=n2 checked>
        <input type=radio name=x form=f1 id=g></body></html>"#,
    );
    let by = |i: &str| q30c_by(&doc, i);
    assert!(set_checked(&by("b"), true));
    assert!(!live_checked(&by("a")) && live_checked(&by("b")), "same form+name unchecks the old one");
    assert!(live_checked(&by("c")), "a different name is a different group");
    assert!(live_checked(&by("d")), "a different form is a different group");
    assert!(live_checked(&by("e")), "no form owner is its own group");
    assert!(live_checked(&by("n1")) && live_checked(&by("n2")), "nameless radios are not grouped");
    // `form=f1` joins f1's group from outside the element.
    set_checked(&by("g"), true);
    assert!(!live_checked(&by("b")) && live_checked(&by("g")));
    assert_eq!(node_id(&checked_radio_peer(&by("b")).unwrap()), node_id(&by("g")));
    clear_check_state();
}

#[test]
fn q30c_reset_restores_defaults_for_that_form_only() {
    clear_check_state();
    let doc = parse_html(
        r#"<html><body><form id=f><input type=checkbox id=a checked><input type=checkbox id=b></form>
        <input type=checkbox id=out></body></html>"#,
    );
    let (a, b, out, f) = (q30c_by(&doc, "a"), q30c_by(&doc, "b"), q30c_by(&doc, "out"), q30c_by(&doc, "f"));
    set_checked(&a, false);
    set_checked(&b, true);
    set_indeterminate(&b, true);
    set_checked(&out, true);
    assert!(reset_form_state(&f));
    assert!(live_checked(&a) && !live_checked(&b) && !live_indeterminate(&b));
    assert!(live_checked(&out), "a control outside the form is untouched");
    clear_check_state();
}

#[test]
fn q30c_checked_pseudo_class_uses_live_state() {
    clear_check_state();
    let doc = parse_html(
        "<html><head><style>input:checked{width:40px;height:20px} input:indeterminate{width:30px}</style></head><body style='margin:0'>\
         <input id=a type=checkbox checked><input id=b type=checkbox><input id=c type=radio></body></html>",
    );
    let width = |id: &str| {
        let (laid, cache) = q30b_layout(&doc);
        node_geometry(&laid, &cache, node_id(&q30c_by(&doc, id))).rects[0].2
    };
    assert_eq!(width("a"), 40.0);
    assert_ne!(width("b"), 40.0);
    set_checked(&q30c_by(&doc, "b"), true);
    set_checked(&q30c_by(&doc, "a"), false);
    assert_eq!(width("b"), 40.0, ":checked reads the live state");
    assert_ne!(width("a"), 40.0);
    set_indeterminate(&q30c_by(&doc, "a"), true);
    assert_eq!(width("a"), 30.0, ":indeterminate reads the live state");
    set_checked(&q30c_by(&doc, "c"), true);
    assert_eq!(width("c"), 40.0);
    // The stylesheet is what makes a toggle need a restyle.
    assert_ne!(state_rules_mask() & STATE_CHECKED, 0);
    clear_check_state();
}

#[test]
fn q30c_checked_state_is_not_part_of_the_layout_tree() {
    clear_check_state();
    fn shape(b: &LayoutBox) -> String {
        match &b.content {
            BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().map(shape).collect::<Vec<_>>().join(","),
            BoxContent::Inline(i) => format!("i{}", i.len()),
            _ => "o".into(),
        }
    }
    // Checked or not, the tree is identical (the painter draws the mark), which
    // is why a toggle needs no layout rebuild when no stylesheet uses :checked.
    let t1 = q30a_tree("<html><body><input type=checkbox><input type=radio></body></html>");
    let t2 = q30a_tree("<html><body><input type=checkbox checked><input type=radio checked></body></html>");
    assert_eq!(shape(&t1), shape(&t2));
}

#[test]
fn q30c_paint_uses_live_state() {
    clear_check_state();
    let doc = parse_html("<html><body><input id=a type=checkbox></body></html>");
    let a = q30c_by(&doc, "a");
    let mut st = anonymous_style(ROOT_STYLE);
    st.node = node_id(&a);
    st.check_widget = CHECK_BOX;
    st.border_width = [1.0; 4];
    st.border_style = [BorderStyle::Solid; 4];
    st.border_color = [Some(TextColor::rgb(118, 118, 118)); 4];
    st.background_color = Some(TextColor::rgb(255, 255, 255));
    let render = |st: &ComputedStyle| {
        let mut px = vec![0x00FFFFFFu32; 13 * 13];
        crate::paint::paint_checkable(st, None, &mut px, 13, 13, 0.0, 0.0, 13.0, 13.0, None);
        px
    };
    let accent = blend(0x00FFFFFF, TextColor::rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    let off = render(&st);
    assert!(!off.contains(&accent));
    set_checked(&a, true);
    let on = render(&st);
    assert_ne!(off, on, "a live toggle changes the pixels with no layout involved");
    assert!(on.contains(&accent), "accent fill");
    assert!(on.iter().filter(|p| **p == 0x00FFFFFF).count() > 3, "white check mark");
    set_checked(&a, false);
    set_indeterminate(&a, true);
    let ind = render(&st);
    assert!(ind.contains(&accent) && ind != on, "indeterminate dash differs from the check");
    set_indeterminate(&a, false);
    assert_eq!(render(&st), off);
    // A radio paints a dot in its middle, no fill.
    st.check_widget = CHECK_RADIO;
    st.border_radius = BoxLength::Percent(0.5);
    set_checked(&a, true);
    let radio = render(&st);
    assert_eq!(radio[6 * 13 + 6], accent, "dot in the middle");
    assert_ne!(radio[3 * 13 + 3], accent, "no fill around the dot");
    clear_check_state();
}

#[test]
fn q30c_js_reflections_events_preventdefault_and_reset() {
    clear_check_state();
    let doc = parse_html(
        r#"<html><body><form id=f><input id=a type=checkbox checked><input id=r1 type=radio name=g checked><input id=r2 type=radio name=g></form>
        <input id=o type=radio name=g></body></html>"#,
    );
    let js = run_page_scripts(
        &doc,
        &[src(
            r#"var L=[], $=function(i){return document.getElementById(i)};
            ['a','r1','r2'].forEach(function(n){ ['click','input','change'].forEach(function(t){ $(n).addEventListener(t,function(e){L.push(n+':'+t+':'+e.bubbles+':'+$(n).checked)}); }); });
            var out=[];
            out.push($('a').checked+','+$('a').defaultChecked);
            $('a').checked=false; out.push($('a').checked+','+$('a').defaultChecked+','+$('a').hasAttribute('checked'));
            $('a').click(); out.push('click1:'+$('a').checked);
            out.push('ev:'+L.join('|')); L.length=0;
            $('r2').click(); out.push('r:'+$('r1').checked+$('r2').checked+$('o').checked);
            out.push('revents:'+L.join('|')); L.length=0;
            $('o').checked=true; out.push('o:'+$('r2').checked+$('o').checked);
            $('r1').checked=true; out.push('set:'+$('r1').checked+$('r2').checked+$('o').checked);
            L.length=0;
            $('a').addEventListener('click',function(e){e.preventDefault()});
            $('a').click(); out.push('pd:'+$('a').checked+':'+L.join('|')); L.length=0;
            $('r2').addEventListener('click',function(e){e.preventDefault()});
            $('r2').click(); out.push('pdr:'+$('r1').checked+$('r2').checked+':'+L.join('|'));
            $('a').indeterminate=true; out.push('ind:'+$('a').indeterminate+$('a').matches(':indeterminate'));
            out.push('m:'+$('a').matches(':checked')+$('r1').matches(':checked'));
            $('a').checked=true; $('r2').checked=true;
            $('f').reset();
            out.push('reset:'+$('a').checked+$('a').indeterminate+$('r1').checked+$('r2').checked+$('o').checked);
            window.out=out.join(';');"#,
        )],
        None,
    )
    .expect("engine");
    let got: String = js.eval_value("t", "window.out").expect("out");
    let want = [
        "true,true",
        "false,true,true",
        "click1:true",
        "ev:a:click:true:true|a:input:true:true|a:change:true:true",
        "r:falsetruefalse",
        "revents:r2:click:true:true|r2:input:true:true|r2:change:true:true",
        "o:truetrue",
        "set:truefalsetrue",
        "pd:true:a:click:true:false",
        "pdr:truefalse:r2:click:true:true",
        "ind:truetrue",
        "m:truetrue",
        "reset:truefalsetruefalsetrue",
    ]
    .join(";");
    assert_eq!(got, want);
    clear_check_state();
}

#[test]
fn q30c_user_click_space_label_and_reset_button_through_the_app() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    clear_check_state();
    let doc = parse_html(
        r#"<html><body><form id=f><input id=a type=checkbox><input id=r1 type=radio name=g checked><input id=r2 type=radio name=g>
        <input id=b type=reset></form><label for=a id=la>text</label><label id=lw>wrap <input id=w type=checkbox></label>
        <input id=d type=checkbox disabled></body></html>"#,
    );
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(
        &doc,
        &[src(
            "window.L=[]; ['a','r1','r2','w'].forEach(function(n){var e=document.getElementById(n); ['click','input','change'].forEach(function(t){e.addEventListener(t,function(ev){L.push(n+':'+t+':'+ev.bubbles+':'+e.checked)})})});\
             document.addEventListener('change',function(e){L.push('doc:change:'+e.target.id)});",
        )],
        None,
    );
    let by = |id: &str| q30c_by(&doc, id);
    let log = |app: &App| -> String { app.js.as_ref().unwrap().eval_value("t", "L.join('|')").expect("log") };
    // A click toggles, fires click, input, change (bubbling to the document).
    assert!(app.click_activate(Some(&by("a")), (0.0, 0.0)));
    assert!(live_checked(&by("a")));
    assert_eq!(log(&app), "a:click:true:true|a:input:true:true|a:change:true:true|doc:change:a");
    app.js.as_ref().unwrap().eval_value::<bool>("t", "L.length=0,true");
    // Space on the focused checkbox is the same activation (key path calls click_activate).
    app.click_activate(Some(&by("a")), (0.0, 0.0));
    assert!(!live_checked(&by("a")));
    // Radios: selecting one unchecks the other, once; clicking the checked one changes nothing.
    app.js.as_ref().unwrap().eval_value::<bool>("t", "L.length=0,true");
    app.click_activate(Some(&by("r2")), (0.0, 0.0));
    assert!(live_checked(&by("r2")) && !live_checked(&by("r1")));
    app.js.as_ref().unwrap().eval_value::<bool>("t", "L.length=0,true");
    app.click_activate(Some(&by("r2")), (0.0, 0.0));
    assert!(live_checked(&by("r2")));
    assert_eq!(log(&app), "r2:click:true:true", "no input/change when nothing changed");
    // A label click toggles its control (for= and wrapping).
    assert!(app.forward_label_click(&by("la"), (0.0, 0.0)));
    assert!(live_checked(&by("a")));
    assert!(app.forward_label_click(&by("lw"), (0.0, 0.0)));
    assert!(live_checked(&by("w")));
    // A disabled control never toggles through its label.
    crate::js_dom::set_attr(&by("la"), "for", "d");
    assert!(app.forward_label_click(&by("la"), (0.0, 0.0)));
    assert!(!live_checked(&by("d")));
    // preventDefault on click restores the state.
    app.js.as_ref().unwrap().eval_value::<bool>("t", "document.getElementById('w').addEventListener('click',function(e){e.preventDefault()}),true");
    app.click_activate(Some(&by("w")), (0.0, 0.0));
    assert!(live_checked(&by("w")), "cancelled click keeps the old state");
    // The reset button restores the defaults of its form (r1 checked, a unchecked).
    app.click_activate(Some(&by("b")), (0.0, 0.0));
    assert!(!live_checked(&by("a")) && live_checked(&by("r1")) && !live_checked(&by("r2")));
    assert!(live_checked(&by("w")), "outside the form: untouched");
    clear_check_state();
}

// ---- QAN1c: animation start/end events and the settle-first restyles ----

struct Qan1c {
    rules: Vec<StyleRule>,
    div: Handle,
    tracker: AnimTracker,
    engine: JsEngine,
    log: Rc<RefCell<Vec<String>>>,
    builds: usize,
    last: ComputedStyle,
}

impl Qan1c {
    fn new(css: &str, body: &str, script: &str) -> Qan1c {
        let doc = parse_html(&format!("<html><body>{body}</body></html>"));
        let (rules, kf) = parse_stylesheet_with_keyframes(css);
        set_keyframes(Rc::new(kf));
        let div = find_element_by_tag(&doc, "div").expect("div");
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let engine = start_scripts(&doc, &[src(script)], Rc::new(move |level, text| l.borrow_mut().push(format!("{level}:{text}")))).expect("engine");
        set_app_clock_override(Some(0.0));
        set_app_clock(7);
        let last = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        let mut q = Qan1c { rules, div, tracker: AnimTracker::default(), engine, log, builds: 1, last };
        q.build(0.0);
        q
    }
    /// One restyle at app clock `ms` (what the App does in `redraw`).
    fn build(&mut self, ms: f64) {
        set_app_clock_override(Some(ms));
        set_app_clock(7);
        self.last = resolve_style(&self.div, &self.rules, ROOT_STYLE, 800.0);
        finish_animation_pass();
        self.tracker.after_build(7, take_animation_report(), document_clock_ms());
    }
    /// What `App::anim_tick` does at `ms`: dispatch, restyle if asked.
    fn tick(&mut self, ms: f64) -> Option<f64> {
        let t = self.tracker.tick(ms);
        for ev in t.events {
            let init = [
                ("bubbles", EventField::Bool(true)),
                ("animationName", EventField::Str(ev.name)),
                ("elapsedTime", EventField::Num(ev.elapsed_s)),
                ("pseudoElement", EventField::Str(ev.pseudo)),
            ];
            self.engine.dispatch_event(ev.node.as_ref(), ev.kind, &init);
        }
        if t.restyle {
            self.builds += 1;
            self.build(ms);
        }
        t.next
    }
    fn events(&self) -> Vec<String> {
        self.log.borrow().iter().filter(|l| l.contains("ev:")).cloned().collect()
    }
}

const QAN1C_LISTEN: &str = "['start','iteration','end','cancel'].forEach(function (k) { document.addEventListener('animation' + k, function (e) { console.log('ev:' + e.type + ':' + e.animationName + ':' + e.elapsedTime + ':' + e.pseudoElement + ':' + (e.target.tagName)); }); });";

#[test]
fn qan1c_fade_in_forwards_restyles_twice_and_ends_once() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 200ms forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.last.opacity, 0.0);
    // Starts at once (no delay): the event, no extra restyle, deadline at 200 ms.
    assert_eq!(q.tick(0.0), Some(200.0));
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    // Nothing happens before the end.
    assert_eq!(q.tick(100.0), Some(200.0));
    assert_eq!(q.builds, 1);
    assert_eq!(q.tick(200.0), None);
    assert_eq!(q.builds, 2, "initial build + one restyle at the end");
    assert_eq!(q.last.opacity, 1.0, "final state painted");
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV", "log:ev:animationend:a:0.2::DIV"]);
    // Later ticks are silent and cause no restyle.
    assert_eq!(q.tick(5000.0), None);
    assert_eq!(q.builds, 2);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_delay_restyles_at_start_and_at_end() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 200ms 300ms forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), Some(300.0));
    assert!(q.events().is_empty());
    assert_eq!(q.tick(300.0), Some(500.0));
    assert_eq!(q.builds, 2, "restyle when the delay elapses");
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    assert_eq!(q.tick(500.0), None);
    assert_eq!(q.builds, 3);
    assert_eq!(q.last.opacity, 1.0);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_name_change_cancels_the_animation() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 1s}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    q.tick(0.0);
    // The page changed the animation name: the next build no longer styles it.
    let (rules, _) = parse_stylesheet_with_keyframes("div{animation:none}");
    q.rules = rules;
    q.build(300.0);
    assert_eq!(q.tick(300.0), None);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV", "log:ev:animationcancel:a:0.3::DIV"]);
    // A cancelled animation never ends.
    q.tick(5000.0);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_infinite_gives_start_only_and_paused_never_ends() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms infinite}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), None, "no deadline for an infinite animation");
    q.tick(10_000.0);
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms paused}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), None);
    q.tick(10_000.0);
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    set_app_clock_override(None);
}

#[test]
fn qan1c_iteration_events_are_capped_to_one_per_second() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms 40 forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    let mut t = q.tick(0.0);
    let mut n = 0;
    while let Some(at) = t {
        t = q.tick(at);
        n += 1;
        assert!(n < 100, "deadline loop must terminate");
    }
    let it = q.events().iter().filter(|e| e.contains("animationiteration")).count();
    // 4 s of animation: about one per second, never one per 100 ms iteration.
    assert!((3..=5).contains(&it), "{:?}", q.events());
    assert_eq!(q.events().iter().filter(|e| e.contains("animationend")).count(), 1);
    set_app_clock_override(None);
}

#[test]
fn qan1c_onanimation_attribute_and_property_handlers_run() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 50ms forwards}";
    let mut q = Qan1c::new(css, "<div onanimationstart=\"console.log('ev:attr-start')\">x</div>", "document.querySelector('div').onanimationend = function (e) { console.log('ev:prop-end:' + e.animationName); };");
    q.tick(0.0);
    q.tick(50.0);
    assert_eq!(q.events(), vec!["log:ev:attr-start", "log:ev:prop-end:a"]);
    set_app_clock_override(None);
}

// ---- Q30d: text editing in input / textarea ----

fn q30d_buf(text: &str, cursor: usize) -> TextEditBuffer {
    TextEditBuffer { text: text.into(), cursor, anchor: None }
}

#[test]
fn q30d_buffer_insert_backspace_delete() {
    let mut b = TextEditBuffer::default();
    b.insert('a');
    b.insert('\u{e9}');
    assert_eq!((b.text.as_str(), b.cursor), ("a\u{e9}", 3));
    b.backspace();
    assert_eq!((b.text.as_str(), b.cursor), ("a", 1));
    b.delete_forward();
    assert_eq!(b.text, "a", "Delete at the end does nothing");
    b.move_left();
    b.delete_forward();
    assert_eq!((b.text.as_str(), b.cursor), ("", 0));
    b.backspace();
    assert_eq!(b.text, "", "Backspace at the start does nothing");
}

#[test]
fn q30d_buffer_selection_replace_collapse_and_extend() {
    let mut b = q30d_buf("hello world", 0);
    b.move_to(5, true);
    assert_eq!(b.selection(), Some((0, 5)));
    assert_eq!(b.selected_text(), Some("hello"));
    b.insert('X');
    assert_eq!((b.text.as_str(), b.cursor, b.anchor), ("X world", 1, None), "typing replaces the selection");
    b.select_all();
    b.backspace();
    assert_eq!(b.text, "", "Backspace deletes the selection");
    // Left/Right without Shift collapse to the selection edge.
    let mut b = q30d_buf("abcdef", 4);
    b.move_to(2, true);
    b.nav_horizontal(true, false);
    assert_eq!((b.cursor, b.selection()), (4, None));
    b.move_to(2, true);
    b.nav_horizontal(false, false);
    assert_eq!((b.cursor, b.selection()), (2, None));
    // Shift+Right extends from the anchor.
    b.nav_horizontal(true, true);
    b.nav_horizontal(true, true);
    assert_eq!(b.selection(), Some((2, 4)));
    b.nav_horizontal(false, true);
    assert_eq!(b.selection(), Some((2, 3)));
    // Pasting multi-line text into a single-line buffer drops the newlines.
    let mut b = TextEditBuffer::default();
    b.insert_str("a\nb");
    assert_eq!(b.text, "ab");
    let mut b = TextEditBuffer::default();
    b.insert_multiline("a\r\nb\rc\u{7}");
    assert_eq!(b.text, "a\nb\nc");
}

#[test]
fn q30d_buffer_word_home_end_and_lines() {
    let b = q30d_buf("foo bar  baz", 12);
    assert_eq!(b.word_left_pos(), 9);
    assert_eq!(q30d_buf("foo bar  baz", 9).word_left_pos(), 4);
    assert_eq!(q30d_buf("foo bar  baz", 4).word_left_pos(), 0);
    assert_eq!(q30d_buf("foo bar  baz", 0).word_right_pos(), 3);
    assert_eq!(q30d_buf("foo bar  baz", 3).word_right_pos(), 7);
    assert_eq!(q30d_buf("foo bar  baz", 7).word_right_pos(), 12);
    assert_eq!(q30d_buf("foo bar", 0).word_at(5), (4, 7));
    let t = "ab\ncdef\ng";
    assert_eq!((q30d_buf(t, 5).line_start_pos(), q30d_buf(t, 5).line_end_pos()), (3, 7));
    assert_eq!(q30d_buf(t, 5).vertical_pos(false), Some(2), "up keeps the column, clamped to the line");
    assert_eq!(q30d_buf(t, 5).vertical_pos(true), Some(9));
    assert_eq!(q30d_buf(t, 1).vertical_pos(false), None);
    assert_eq!(q30d_buf(t, 8).vertical_pos(true), None);
    assert_eq!(q30d_buf(t, 1).vertical_pos(true), Some(4));
}

#[test]
fn q30d_state_seed_dirty_selection_and_sanitizing() {
    clear_text_state();
    let doc = parse_html("<html><body><input id=a value=hi><textarea id=t>x&#10;y</textarea></body></html>");
    let (a, t) = (q30c_by(&doc, "a"), q30c_by(&doc, "t"));
    assert_eq!((text_value(&a), text_value(&t)), ("hi".into(), "x\ny".into()));
    // While not dirty the attribute still controls the live value.
    crate::js_dom::set_attr(&a, "value", "yo");
    assert_eq!(text_value(&a), "yo");
    assert!(set_text_value(&a, "zz"), "a change is reported");
    crate::js_dom::set_attr(&a, "value", "late");
    assert_eq!(text_value(&a), "zz", "dirty: the attribute is ignored");
    assert_eq!((text_selection(&a, false), text_selection(&a, true)), (2, 2), "setting .value puts the caret at the end");
    assert!(!set_text_value(&a, "zz"));
    // `value` never holds a newline in an input; a textarea keeps them (CRLF normalized).
    set_text_value(&a, "a\nb");
    assert_eq!(text_value(&a), "ab");
    set_text_value(&t, "p\r\nq");
    assert_eq!(text_value(&t), "p\nq");
    // Selection offsets are UTF-16 code units.
    set_text_value(&a, "a\u{1F600}b");
    set_text_selection(&a, 1, 3);
    assert_eq!((text_selection(&a, false), text_selection(&a, true)), (1, 3));
    set_text_selection(&a, 3, 4);
    assert!(text_paint_info(node_id(&a), false).is_some());
    clear_text_state();
    assert!(text_paint_info(node_id(&a), false).is_none());
    assert_eq!(text_value(&a), "late", "navigation clears the live value: the attribute rules again");
}

#[test]
fn q30d_text_is_not_in_the_layout_tree() {
    clear_text_state();
    fn find(b: &LayoutBox, out: &mut Vec<(u8, usize)>) {
        if b.style.text_widget != 0 {
            let n = match &b.content {
                BoxContent::Inline(i) => i.len(),
                _ => 99,
            };
            out.push((b.style.text_widget, n));
        }
        match &b.content {
            BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().for_each(|k| find(k, out)),
            BoxContent::Inline(items) => items.iter().for_each(|i| {
                if let InlineItem::Embedded(e) = i {
                    find(e, out)
                }
            }),
            _ => {}
        }
    }
    let shape = |html: &str| {
        let mut v = Vec::new();
        find(&q30a_tree(html), &mut v);
        v
    };
    let a = shape("<input value=a><input type=password value=secret><textarea>t</textarea>");
    let b = shape("<input value='a much longer value'><input type=password><textarea>\n\nlots\nof\ntext</textarea>");
    let c = shape("<input placeholder=hint><input type=password placeholder=pw><textarea placeholder=ph></textarea>");
    assert_eq!(a, b, "the value is not part of the tree");
    assert_eq!(a, c, "neither is the placeholder");
    assert_eq!(a, vec![(TEXT_SINGLE, 1), (TEXT_SINGLE | TEXT_PASSWORD, 1), (TEXT_MULTI, 0)]);
    // Date pickers and buttons stay static.
    assert!(shape("<input type=date><input type=checkbox><input type=submit>").is_empty());
}

fn q30d_render(h: &Handle, kind: u8, focused: bool, w: u32, hgt: u32) -> Vec<u32> {
    let mut st = anonymous_style(ROOT_STYLE);
    st.node = node_id(h);
    st.text_widget = kind;
    set_interaction(&InteractionState { focus: focused.then(|| node_id(h)), ..Default::default() });
    let mut fs = new_font_system();
    let mut sc = SwashCache::new();
    let mut px = vec![0x00FFFFFFu32; (w * hgt) as usize];
    crate::paint::paint_text_control(&st, &mut fs, &mut sc, &mut px, w, hgt, 0.0, 0.0, w as f32, hgt as f32, None);
    set_interaction(&InteractionState::default());
    px
}

#[test]
fn q30d_paint_value_placeholder_password_caret_selection() {
    clear_text_state();
    let doc = parse_html("<html><body><input id=a placeholder=hint><input id=p type=password><textarea id=t></textarea></body></html>");
    let (a, p, t) = (q30c_by(&doc, "a"), q30c_by(&doc, "p"), q30c_by(&doc, "t"));
    for h in [&a, &p, &t] {
        register_text_control(h, if node_id(h) == node_id(&a) { "hint".into() } else { String::new() }, TextColor::rgb(117, 117, 117));
    }
    let blank = vec![0x00FFFFFFu32; 200 * 24];
    let empty_unfocused = q30d_render(&a, TEXT_SINGLE, false, 200, 24);
    assert_ne!(empty_unfocused, blank, "the placeholder shows while the value is empty");
    set_text_value(&a, "x");
    let typed = q30d_render(&a, TEXT_SINGLE, false, 200, 24);
    assert_ne!(typed, empty_unfocused, "the placeholder is replaced by the typed value");
    assert!(typed.iter().filter(|p| **p != 0x00FFFFFF).count() < empty_unfocused.iter().filter(|p| **p != 0x00FFFFFF).count());
    set_text_value(&a, "xy");
    let more = q30d_render(&a, TEXT_SINGLE, false, 200, 24);
    assert_ne!(more, typed, "typing repaints with the new text, no layout involved");
    // The caret is painted only while focused; a selection adds a highlight.
    let focused = q30d_render(&a, TEXT_SINGLE, true, 200, 24);
    assert_ne!(focused, more);
    let accent_hl = blend(0x00FFFFFF, TextColor::rgba(ACCENT.0, ACCENT.1, ACCENT.2, 90));
    assert!(!focused.contains(&accent_hl));
    text_select_all(&a);
    let selected = q30d_render(&a, TEXT_SINGLE, true, 200, 24);
    assert!(selected.contains(&accent_hl), "selection highlight");
    let unfocused_selected = q30d_render(&a, TEXT_SINGLE, false, 200, 24);
    assert!(!unfocused_selected.contains(&accent_hl), "no highlight when unfocused");
    // A password shows bullets: the same length paints the same pixels for any text.
    set_text_value(&p, "abc");
    let p1 = q30d_render(&p, TEXT_SINGLE | TEXT_PASSWORD, false, 200, 24);
    set_text_value(&p, "xyz");
    let p2 = q30d_render(&p, TEXT_SINGLE | TEXT_PASSWORD, false, 200, 24);
    assert_eq!(p1, p2);
    assert_ne!(p1, blank);
    set_text_value(&p, "xyzw");
    assert_ne!(q30d_render(&p, TEXT_SINGLE | TEXT_PASSWORD, false, 200, 24), p2);
    // A textarea wraps: text longer than one line paints on more than one row.
    set_text_value(&t, "line one\nline two\nline three");
    let rows = q30d_render(&t, TEXT_MULTI, false, 120, 80);
    let ink_rows = (0..80usize).filter(|y| (0..120usize).any(|x| rows[y * 120 + x] != 0x00FFFFFF)).count();
    assert!(ink_rows > 30, "three lines of text, got {ink_rows} inked rows");
    clear_text_state();
}

#[test]
fn q30d_js_value_selection_roundtrip() {
    clear_text_state();
    let doc = parse_html(
        "<html><body><input id=a value=hi><textarea id=t>ab</textarea><input id=c type=checkbox><input id=d type=date></body></html>",
    );
    let js = run_page_scripts(
        &doc,
        &[src(
            "var a=document.getElementById('a'),t=document.getElementById('t'),c=document.getElementById('c'),o=[];\
             o.push(a.value,t.value);\
             a.value='hello'; o.push(a.value,a.selectionStart,a.selectionEnd);\
             a.setSelectionRange(1,3); o.push(a.selectionStart,a.selectionEnd);\
             a.select(); o.push(a.selectionStart,a.selectionEnd);\
             a.selectionStart=2; o.push(a.selectionStart,a.selectionEnd);\
             a.setAttribute('value','zz'); o.push(a.value,a.defaultValue);\
             t.value='l1\\nl2'; o.push(t.value.length,t.selectionEnd,t.defaultValue);\
             t.select(); o.push(t.selectionStart,t.selectionEnd);\
             a.value=null; o.push('['+a.value+']');\
             o.push(c.selectionStart===null);\
             window.out=o.join('|');",
        )],
        None,
    )
    .expect("js");
    let got: String = js.eval_value("t", "window.out").expect("out");
    assert_eq!(got, "hi|ab|hello|5|5|1|3|0|5|2|5|hello|zz|5|5|ab|0|5|[]|true");
    clear_text_state();
}

fn q30d_key(app: &mut App, h: &Handle, key: Key) -> bool {
    app.text_control_key(h, &key, false)
}

fn q30d_chr(app: &mut App, h: &Handle, s: &str) {
    q30d_key(app, h, Key::Character(s.into()));
}

#[test]
fn q30d_typing_through_the_app_events_maxlength_readonly_escape() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    clear_text_state();
    let doc = parse_html(
        r#"<html><body><input id=i maxlength=5><textarea id=t></textarea><input id=ro readonly value=ab><input id=pw type=password>
        <input id=off disabled value=x></body></html>"#,
    );
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(
        &doc,
        &[src(
            "window.L=[]; ['i','t','ro'].forEach(function(n){var e=document.getElementById(n);\
              ['beforeinput','input','change','blur'].forEach(function(k){e.addEventListener(k,function(ev){L.push(n+':'+k+':'+(ev.inputType||'')+':'+(ev.data===undefined?'':ev.data)+':'+e.value)})})});",
        )],
        None,
    );
    let by = |id: &str| q30c_by(&doc, id);
    let log = |app: &App| -> String { app.js.as_ref().unwrap().eval_value("t", "L.join('|')").expect("log") };
    let clear = |app: &App| {
        app.js.as_ref().unwrap().eval_value::<bool>("t", "L.length=0,true");
    };
    let i = by("i");
    app.set_focus_element(Some(&i), false);
    assert!(app.focused_text_control().is_some());
    q30d_chr(&mut app, &i, "a");
    assert_eq!(text_value(&i), "a");
    assert_eq!(log(&app), "i:beforeinput:insertText:a:|i:input:insertText:a:a", "beforeinput then one input per edit");
    clear(&app);
    // Space comes as a named key; ':' and uppercase are plain characters.
    q30d_key(&mut app, &i, Key::Named(NamedKey::Space));
    q30d_chr(&mut app, &i, ":");
    q30d_chr(&mut app, &i, "B");
    assert_eq!(text_value(&i), "a :B");
    // maxlength 5.
    q30d_chr(&mut app, &i, "c");
    q30d_chr(&mut app, &i, "d");
    assert_eq!(text_value(&i), "a :Bc");
    // Caret moves: Home, Right, Delete, End, Backspace.
    clear(&app);
    q30d_key(&mut app, &i, Key::Named(NamedKey::Home));
    q30d_key(&mut app, &i, Key::Named(NamedKey::ArrowRight));
    q30d_key(&mut app, &i, Key::Named(NamedKey::Delete));
    assert_eq!(text_value(&i), "a:Bc");
    q30d_key(&mut app, &i, Key::Named(NamedKey::End));
    q30d_key(&mut app, &i, Key::Named(NamedKey::Backspace));
    assert_eq!(text_value(&i), "a:B");
    assert_eq!(log(&app).matches(":input:").count(), 2, "one input event per edit, none for moves");
    // Backspace at the start changes nothing and fires no input.
    clear(&app);
    q30d_key(&mut app, &i, Key::Named(NamedKey::Home));
    q30d_key(&mut app, &i, Key::Named(NamedKey::Backspace));
    assert!(!log(&app).contains(":input:"), "no input event when nothing changed");
    // Ctrl+A then typing replaces everything; Ctrl+Backspace deletes a word.
    app.modifiers = ModifiersState::CONTROL;
    q30d_chr(&mut app, &i, "a");
    app.modifiers = ModifiersState::empty();
    q30d_chr(&mut app, &i, "z");
    assert_eq!(text_value(&i), "z");
    // Enter in an input is not a newline and only commits `change`.
    clear(&app);
    q30d_key(&mut app, &i, Key::Named(NamedKey::Enter));
    assert_eq!(text_value(&i), "z");
    assert_eq!(log(&app), "i:change:::z", "Enter commits: change since focus");
    // Shift+Left selects, copy then paste duplicates (internal or system clipboard).
    app.modifiers = ModifiersState::SHIFT;
    q30d_key(&mut app, &i, Key::Named(NamedKey::ArrowLeft));
    app.modifiers = ModifiersState::CONTROL;
    q30d_chr(&mut app, &i, "c");
    q30d_key(&mut app, &i, Key::Named(NamedKey::End));
    q30d_chr(&mut app, &i, "v");
    app.modifiers = ModifiersState::empty();
    assert_eq!(text_value(&i), "zz");
    // Escape blurs the control (back to scratchy's own keys) and fires `blur`.
    clear(&app);
    q30d_chr(&mut app, &i, "q");
    clear(&app);
    q30d_key(&mut app, &i, Key::Named(NamedKey::Escape));
    assert!(app.interaction.focus.is_none() && app.focused_text_control().is_none());
    assert_eq!(log(&app), "i:change:::zzq|i:blur:::zzq", "change fires before blur");
    // A textarea: Enter inserts a newline, Up/Down move by line.
    let t = by("t");
    app.set_focus_element(Some(&t), false);
    q30d_chr(&mut app, &t, "a");
    q30d_key(&mut app, &t, Key::Named(NamedKey::Enter));
    q30d_chr(&mut app, &t, "b");
    assert_eq!(text_value(&t), "a\nb");
    q30d_key(&mut app, &t, Key::Named(NamedKey::ArrowUp));
    q30d_chr(&mut app, &t, "X");
    assert_eq!(text_value(&t), "aX\nb");
    // readonly: no edits, but the selection still works; Ctrl+A selects.
    let ro = by("ro");
    app.set_focus_element(Some(&ro), false);
    clear(&app);
    q30d_chr(&mut app, &ro, "z");
    q30d_key(&mut app, &ro, Key::Named(NamedKey::Backspace));
    assert_eq!(text_value(&ro), "ab");
    assert_eq!(log(&app), "", "readonly fires no input events");
    app.modifiers = ModifiersState::CONTROL;
    q30d_chr(&mut app, &ro, "a");
    app.modifiers = ModifiersState::empty();
    assert_eq!((text_selection(&ro, false), text_selection(&ro, true)), (0, 2));
    // disabled controls take no focus-owning keys at all.
    app.set_focus_element(Some(&by("off")), false);
    assert!(app.focused_text_control().is_none());
    // A password edits like any input (its bullets are only painted).
    let pw = by("pw");
    app.set_focus_element(Some(&pw), false);
    q30d_chr(&mut app, &pw, "s");
    q30d_chr(&mut app, &pw, "3");
    assert_eq!(text_value(&pw), "s3");
    clear_text_state();
}

#[test]
fn q30d_page_keydown_preventdefault_is_reported_and_pages_see_the_focused_target() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return,
    };
    clear_text_state();
    let doc = parse_html("<html><body><input id=i></body></html>");
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(
        &doc,
        &[src(
            "window.T=[]; document.addEventListener('keydown',function(e){T.push(e.target.id+':'+e.key); if(e.key===':')e.preventDefault()});\
             document.getElementById('i').addEventListener('keypress',function(e){T.push('kp:'+e.key)});",
        )],
        None,
    );
    let i = q30c_by(&doc, "i");
    app.set_focus_element(Some(&i), false);
    // The App's KeyboardInput handler runs `js_key_for("keydown")` first and stops when the
    // page cancels; the text path only runs for keys the page left alone.
    assert!(app.js_key_for("keydown", &Key::Character("a".into()), false));
    assert!(!app.js_key_for("keydown", &Key::Character(":".into()), false), "preventDefault cancels the key");
    q30d_chr(&mut app, &i, "a");
    let got: String = app.js.as_ref().unwrap().eval_value("t", "T.join('|')").expect("log");
    assert_eq!(got, "i:a|i::|kp:a", "keydown targets the focused control; keypress follows it");
    clear_text_state();
}

<<<<<<< HEAD
// ---------- QY39b: --user-agent ----------

fn qy39b_args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn qy39b_flag_parsing_and_env() {
    assert_eq!(user_agent_from_args(&qy39b_args(&["--no-js", "x"])), Ok(None));
    assert_eq!(resolve_user_agent(&qy39b_args(&["--user-agent", "x-test/1.0", "u"]), None), Ok(Some("x-test/1.0".into())));
    // The flag wins over the environment; an empty variable is unset.
    assert_eq!(resolve_user_agent(&qy39b_args(&["--user-agent", "f"]), Some("e".into())), Ok(Some("f".into())));
    assert_eq!(resolve_user_agent(&[], Some("e".into())), Ok(Some("e".into())));
    assert_eq!(resolve_user_agent(&[], Some(String::new())), Ok(None));
    assert!(resolve_user_agent(&qy39b_args(&["--user-agent"]), None).unwrap_err().contains("needs a string"));
}

#[test]
fn qy39b_validation_errors() {
    assert!(validate_user_agent("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0").is_ok());
    assert!(validate_user_agent("").is_err());
    assert!(validate_user_agent("bad\r\nX: y").is_err());
    assert!(validate_user_agent("caf\u{e9}").is_err());
    assert!(validate_user_agent("tab\there").is_err());
    assert!(validate_user_agent(&"a".repeat(512)).is_ok());
    assert!(validate_user_agent(&"a".repeat(513)).is_err());
    assert!(resolve_user_agent(&qy39b_args(&["--user-agent", "bad\nx"]), None).is_err());
    assert!(resolve_user_agent(&[], Some("\u{7f}".into())).is_err());
}

fn qy39b_count(req: &[u8]) -> usize {
    String::from_utf8_lossy(req).lines().filter(|l| l.to_ascii_lowercase().starts_with("x-wire-user-agent:")).count()
}

#[test]
fn qy39b_header_added_once_per_request_kind_and_never_without_flag() {
    let url = Url::parse("https://page.test/a.png").unwrap();
    let on = UaOverride::new(Some("x-test/1.0".into()));
    let off = UaOverride::new(None);
    // Navigation / sub-resource builder (the ctx builder appends cookie headers to this text).
    let t = build_get_request_text_with(&url, &on);
    assert_eq!(qy39b_count(t.as_bytes()), 1, "{t}");
    assert!(t.contains("X-Wire-User-Agent: x-test/1.0\r\n") && t.ends_with("\r\n\r\n"));
    assert_eq!(qy39b_count(build_get_request_text_with(&url, &off).as_bytes()), 0);
    // fetch/XHR/beacon/media builder, even if a caller smuggled the header in.
    let smuggled = vec![("X-Wire-User-Agent".to_string(), "evil".to_string()), ("Accept".to_string(), "*/*".to_string())];
    for method in ["GET", "POST"] {
        let r = build_wire_request_with(method, &url, &smuggled, b"", &on).unwrap();
        assert_eq!(qy39b_count(&r), 1);
        let s = String::from_utf8_lossy(&r).to_string();
        assert!(s.contains("X-Wire-User-Agent: x-test/1.0\r\n") && !s.contains("evil") && s.contains("Accept: */*"), "{s}");
        assert_eq!(qy39b_count(&build_wire_request_with(method, &url, &[], b"", &off).unwrap()), 0);
    }
    // The global (no flag in tests) adds nothing anywhere.
    assert_eq!(qy39b_count(build_get_request_ctx(&url, &CookieContext::subresource(&url)).as_bytes()), 0);
    assert_eq!(qy39b_count(&build_wire_request("GET", &url, &[], b"").unwrap()), 0);
}

#[test]
fn qy39b_old_wire_warns_once_and_stops_sending() {
    let url = Url::parse("https://page.test/").unwrap();
    let ua = UaOverride::new(Some("x-test/1.0".into()));
    assert!(ua.header().is_some(), "sent until wire says otherwise");
    assert_eq!(ua.observe(None), Some(OLD_WIRE_WARNING));
    assert!(OLD_WIRE_WARNING.contains("wire does not support a User-Agent override (restart wire from a current build)"));
    assert_eq!(ua.observe(Some("")), None, "warned once");
    assert_eq!(qy39b_count(build_get_request_text_with(&url, &ua).as_bytes()), 0);
    // A current wire: nothing logged, still sent.
    let ok = UaOverride::new(Some("x".into()));
    assert_eq!(ok.observe(Some("foo, ua")), None);
    assert!(ok.header().is_some());
    // Without the flag nothing is ever learned or warned.
    assert_eq!(UaOverride::new(None).observe(None), None);
}

#[test]
fn qy39b_navigator_agrees_with_the_flag() {
    assert_eq!(navigator_user_agent_for(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0")), "Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0");
    assert_eq!(navigator_app_version_for(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0")), "5.0 (X11; Linux x86_64) Firefox/130.0");
    assert_eq!(navigator_app_version_for(Some("x-test/1.0")), "x-test/1.0");
    // Unchanged without the flag.
    assert!(navigator_user_agent_for(None).starts_with("Mozilla/5.0 (X11; Linux x86_64) scratchy/"));
    assert_eq!(navigator_app_version_for(None), "5.0 (X11)");
    // And the JS prelude reads both from the environment.
    let fake = QfFake::new(vec![]);
    let l = qf8_run("https://page.test/", &fake, "console.log(navigator.userAgent, '|', navigator.appVersion);");
    assert_eq!(l, vec![format!("log:{} | {}", navigator_user_agent(), navigator_app_version())], "{l:?}");
=======
// ---- Q30e: form submission ------------------------------------------------

fn q30e_form(html: &str) -> (Handle, Handle) {
    clear_check_state();
    clear_text_state();
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let f = q30c_by(&doc, "f");
    (doc, f)
}

fn q30e_set(f: &Handle, submitter: Option<&Handle>) -> String {
    encode_urlencoded(&form_data_set(f, submitter))
}

#[test]
fn q30e_data_set_every_control_type() {
    let (doc, f) = q30e_form(
        r#"<form id=f>
        <input name=t value="a b&c"><input type=password name=pw value=s3><input type=hidden name=h value=hid>
        <input type=checkbox name=c1 checked><input type=checkbox name=c2><input type=checkbox name=c3 value=yes checked>
        <input type=radio name=r value=1><input type=radio name=r value=2 checked><input type=radio name=r value=3>
        <input name=off disabled value=x><input value=nameless><input name=empty>
        <textarea name=ta>l1
l2</textarea>
        <select name=s><option value=a>A<option value=b selected>B<option value=c>C</select>
        <select name=s1><option disabled>D<option>Second one</select>
        <select name=m multiple><option selected>x<option>y<option selected value=z>Z</select>
        <input type=file name=fl><input type=range name=rg><input type=color name=co>
        <input id=sb type=submit name=go value=Go><button id=bb name=b value=bv>B</button><button type=button name=nb>N</button>
        <input type=reset name=rs><input type=image name=im>
        <fieldset disabled><input name=infs value=1></fieldset>
        </form>"#,
    );
    assert_eq!(
        q30e_set(&f, None),
        "t=a+b%26c&pw=s3&h=hid&c1=on&c3=yes&r=2&empty=&ta=l1%0D%0Al2&s=b&s1=Second+one&m=x&m=z&fl=&rg=50&co=%23000000"
    );
    let bb = q30c_by(&doc, "bb");
    assert!(q30e_set(&f, Some(&bb)).ends_with("&b=bv"));
    let sb = q30c_by(&doc, "sb");
    assert!(q30e_set(&f, Some(&sb)).ends_with("&go=Go"), "{}", q30e_set(&f, Some(&sb)));
    clear_check_state();
    clear_text_state();
}

#[test]
fn q30e_live_values_radio_groups_and_form_owner() {
    let (doc, f) = q30e_form(
        r#"<form id=f><input id=a name=a value=old><textarea id=t name=t></textarea>
        <input id=r1 type=radio name=r value=1 checked><input id=r2 type=radio name=r value=2></form>
        <input name=ext form=f value=outside><form id=g><input name=other form=f value=moved></form>
        <input name=nope value=no>"#,
    );
    set_text_value(&q30c_by(&doc, "a"), "new value");
    set_text_value(&q30c_by(&doc, "t"), "x\ny\r\nz");
    set_checked(&q30c_by(&doc, "r2"), true);
    assert_eq!(q30e_set(&f, None), "a=new+value&t=x%0D%0Ay%0D%0Az&r=2&ext=outside&other=moved");
    assert_eq!(q30e_set(&q30c_by(&doc, "g"), None), "");
    clear_check_state();
    clear_text_state();
}

#[test]
fn q30e_image_submitter_adds_xy() {
    let (_d, f) = q30e_form("<form id=f><input type=image name=im id=i><input type=image id=j></form>");
    let i = find_element_by_tag(&f, "input").unwrap();
    assert_eq!(q30e_set(&f, Some(&i)), "im.x=0&im.y=0");
}

fn q30e_plan(html: &str, submitter_id: Option<&str>, page: &str) -> SubmitPlan {
    let (doc, f) = q30e_form(html);
    let s = submitter_id.map(|id| q30c_by(&doc, id));
    plan_submission(&f, s.as_ref(), Some(&Url::parse(page).unwrap())).expect("plan")
}

#[test]
fn q30e_get_replaces_query_keeps_fragment_and_resolves_action() {
    let p = q30e_plan("<form id=f action='/w/index.php?old=1#frag'><input name=search value='Rust programming language'><input type=hidden name=go value=Go></form>", None, "https://en.example.org/wiki/Main");
    assert_eq!(p.method, SubmitMethod::Get);
    assert_eq!(p.url.as_str(), "https://en.example.org/w/index.php?search=Rust+programming+language&go=Go#frag");
    assert!(p.post.is_none());
    let p = q30e_plan("<form id=f><input name=q value=1></form>", None, "https://e.org/p?a=b");
    assert_eq!(p.url.as_str(), "https://e.org/p?q=1");
    let p = q30e_plan("<form id=f action=s><input name=q value=1></form>", None, "https://e.org/dir/p");
    assert_eq!(p.url.as_str(), "https://e.org/dir/s?q=1");
    assert!(plan_submission(&q30e_form("<form id=f action='mailto:a@b.c'></form>").1, None, None).is_err());
}

#[test]
fn q30e_post_urlencoded_multipart_and_text_plain_bodies() {
    let form = |enc: &str| format!("<form id=f method=post action=/p {enc}><input name=a value='x y'><input type=file name=up><textarea name=t>l1\nl2</textarea></form>");
    let p = q30e_plan(&form(""), None, "https://e.org/");
    assert_eq!(p.url.as_str(), "https://e.org/p");
    let post = p.post.unwrap();
    assert_eq!(post.content_type, "application/x-www-form-urlencoded");
    assert_eq!(String::from_utf8(post.body).unwrap(), "a=x+y&up=&t=l1%0D%0Al2");
    let post = q30e_plan(&form("enctype=text/plain"), None, "https://e.org/").post.unwrap();
    assert_eq!(post.content_type, "text/plain");
    assert_eq!(String::from_utf8(post.body).unwrap(), "a=x y\r\nup=\r\nt=l1\r\nl2\r\n");
    let post = q30e_plan(&form("enctype=multipart/form-data"), None, "https://e.org/").post.unwrap();
    let b = post.content_type.strip_prefix("multipart/form-data; boundary=").expect("boundary").to_string();
    let want = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nx y\r\n--{b}\r\nContent-Disposition: form-data; name=\"up\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--{b}\r\nContent-Disposition: form-data; name=\"t\"\r\n\r\nl1\r\nl2\r\n--{b}--\r\n"
    );
    assert_eq!(String::from_utf8(post.body).unwrap(), want);
    let again = q30e_plan(&form("enctype=multipart/form-data"), None, "https://e.org/").post.unwrap();
    assert_ne!(again.content_type, format!("multipart/form-data; boundary={b}"), "a fresh boundary per submission");
}

#[test]
fn q30e_formaction_formmethod_formenctype_overrides_and_unknown_values() {
    let html = "<form id=f action=/a method=get><input name=q value=1><button id=b formaction=/b formmethod=post formenctype=text/plain formtarget=_blank name=n value=v>X</button><button id=c>Y</button></form>";
    let p = q30e_plan(html, Some("b"), "https://e.org/");
    assert_eq!((p.method, p.url.as_str(), p.target.as_str()), (SubmitMethod::Post, "https://e.org/b", "_blank"));
    let post = p.post.unwrap();
    assert_eq!((post.content_type.as_str(), String::from_utf8(post.body).unwrap().as_str()), ("text/plain", "q=1\r\nn=v\r\n"));
    let p = q30e_plan(html, Some("c"), "https://e.org/");
    assert_eq!((p.method, p.url.as_str()), (SubmitMethod::Get, "https://e.org/a?q=1"));
    let p = q30e_plan("<form id=f method=PoSt enctype=bogus></form>", None, "https://e.org/");
    assert_eq!(p.post.unwrap().content_type, "application/x-www-form-urlencoded");
    let p = q30e_plan("<form id=f method=nonsense></form>", None, "https://e.org/");
    assert_eq!(p.method, SubmitMethod::Get);
    assert_eq!(submit_method_of(&q30e_form("<form id=f method=dialog></form>").1, None), SubmitMethod::Dialog);
}

#[test]
fn q30e_default_button_and_implicit_submission_rules() {
    let (_d, f) = q30e_form("<form id=f><input name=a><input name=b></form>");
    assert!(default_button(&f).is_none() && !implicit_submission_ok(&f), "two text fields and no button: no implicit submit");
    let (_d, f) = q30e_form("<form id=f><input name=a><input type=checkbox name=c></form>");
    assert!(implicit_submission_ok(&f), "one text field (a checkbox does not block)");
    let (_d, f) = q30e_form("<form id=f><input name=a><input name=b><button type=button>no</button><button id=s>go</button></form>");
    assert_eq!(attr_string(&default_button(&f).unwrap(), "id").as_deref(), Some("s"));
}

#[test]
fn q30e_post_request_text_headers_cookies_and_samesite_method() {
    let url = Url::parse("https://shop.test/login").unwrap();
    let page = Url::parse("https://shop.test/form").unwrap();
    let post = PostBody { content_type: "application/x-www-form-urlencoded".into(), body: b"a=1&b=2".to_vec() };
    let ctx = CookieContext::navigation(&url, Some(&page));
    let t = build_post_request_ctx(&url, &ctx, &post, Some(&page));
    assert!(t.starts_with("POST https://shop.test/login HTTP/1.1\r\nHost: shop.test\r\n"), "{t}");
    assert!(t.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
    assert!(t.contains("Origin: https://shop.test\r\n"));
    assert!(t.contains("Content-Length: 7\r\n"));
    assert!(t.ends_with("\r\n\r\na=1&b=2"));
    // Cookie partition headers as for any navigation (X-Wire-Nav + initiator) when the jar is usable.
    let support = CookieSupport::new(false);
    let hs = wire_cookie_headers_with(&support, &ctx);
    if !hs.is_empty() {
        let names: Vec<&str> = hs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"X-Wire-Nav") && names.contains(&"X-Wire-Initiator-Site"), "{names:?}");
    }
    let t = build_post_request_ctx(&url, &CookieContext::navigation(&url, None), &post, Some(&Url::parse("file:///x.html").unwrap()));
    assert!(t.contains("Origin: null\r\n"), "{t}");
}

#[test]
fn q30e_post_reaches_a_fake_wire_with_exact_bytes_and_final_url_is_read() {
    use std::os::unix::net::UnixListener;
    let path = unique_test_socket_path("q30e-post");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind");
    let url = Url::parse("http://127.0.0.1:9/p").unwrap();
    let post = PostBody { content_type: "text/plain".into(), body: b"k=v\r\n".to_vec() };
    let text = build_post_request_ctx(&url, &CookieContext::navigation(&url, None), &post, None);
    let want = text.clone().into_bytes();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while got.len() < want.len() {
            match Read::read(&mut s, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        assert_eq!(got, want);
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Wire-Final-Url: http://127.0.0.1:9/done\r\n\r\n<p>ok</p>").unwrap();
    });
    let r = fetch_raw_via_wire(&path, &text);
    server.join().unwrap();
    let _ = std::fs::remove_file(&path);
    match r {
        WireFetchRaw::Responded { body, final_url, .. } => {
            assert_eq!(body, b"<p>ok</p>");
            assert_eq!(final_url.as_deref(), Some("http://127.0.0.1:9/done"));
        }
        WireFetchRaw::Unreachable => panic!("unreachable"),
    }
}

fn q30e_app(html: &str, script: &str) -> Option<(App, Handle)> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    clear_check_state();
    clear_text_state();
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    Some((app, doc))
}

fn q30e_log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value("t", "window.L.join('|')").unwrap_or_default()
}

#[test]
fn q30e_click_submit_event_preventdefault_requestsubmit_vs_submit_and_history() {
    // A navigation drops the page's JS (as for any page): the script is reinstalled after each one.
    let script = "window.L=[]; var f=document.getElementById('f'); f.addEventListener('submit',function(e){L.push('submit:'+(e.submitter&&e.submitter.id)+':'+(e instanceof SubmitEvent)); if(window.cancel)e.preventDefault();});";
    let Some((mut app, doc)) = q30e_app(
        "<form id=f action=/s><input id=i name=q value=1><button id=b name=bn value=BV>Go</button></form>",
        script,
    ) else {
        return;
    };
    let b = q30c_by(&doc, "b");
    app.js.as_ref().unwrap().eval_value::<bool>("t", "window.cancel=true");
    app.click_activate(Some(&b), (0.0, 0.0));
    assert_eq!(q30e_log(&app), "submit:b:true");
    assert!(app.history.is_empty() && app.nav_generation == 0, "preventDefault stops the submission");
    app.js.as_ref().unwrap().eval_value::<bool>("t", "window.cancel=false,L.length=0,true");
    app.click_activate(Some(&b), (0.0, 0.0));
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/s?q=1&bn=BV"));
    assert_eq!(app.nav_generation, 1);
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    app.js.as_ref().unwrap().eval_value::<bool>("t", "document.getElementById('f').submit(),true");
    let log = q30e_log(&app);
    app.js_sync();
    assert_eq!(log, "", "submit() fires no submit event");
    assert_eq!(app.nav_generation, 2);
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/s?q=1"));
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    app.js.as_ref().unwrap().eval_value::<bool>("t", "document.getElementById('f').requestSubmit(document.getElementById('b')),true");
    let log = q30e_log(&app);
    app.js_sync();
    assert_eq!(log, "submit:b:true");
    assert_eq!(app.nav_generation, 3);
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/s?q=1&bn=BV"));
    clear_check_state();
    clear_text_state();
}

#[test]
fn q30e_enter_in_a_text_field_submits_implicitly_and_post_goes_into_history() {
    let Some((mut app, doc)) = q30e_app(
        "<form id=f method=post action=/login><input id=u name=user><input id=p type=password name=pw></form><form id=g action=/one><input id=o name=q><input type=checkbox name=c checked></form><form id=h><input id=h1 name=a><input id=h2 name=b></form>",
        "window.L=[]; document.addEventListener('submit',function(e){L.push(e.target.id)});",
    ) else {
        return;
    };
    let h1 = q30c_by(&doc, "h1");
    app.set_focus_element(Some(&h1), false);
    app.text_control_key(&h1, &Key::Named(NamedKey::Enter), false);
    assert_eq!(app.nav_generation, 0);
    let o = q30c_by(&doc, "o");
    app.set_focus_element(Some(&o), false);
    for c in ["h", "i"] {
        app.text_control_key(&o, &Key::Character(c.into()), false);
    }
    app.text_control_key(&o, &Key::Named(NamedKey::Enter), false);
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/one?q=hi&c=on"));
    let u = q30c_by(&doc, "u");
    app.set_focus_element(Some(&u), false);
    app.text_control_key(&u, &Key::Character("a".into()), false);
    app.text_control_key(&u, &Key::Named(NamedKey::Enter), false);
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/login"));
    assert!(app.pending_post.is_none(), "the POST body is consumed by that one navigation");
    app.navigate_history(-1);
    assert!(app.pending_post.is_none());
    clear_check_state();
    clear_text_state();
}

#[test]
fn q30e_mixed_content_form_action_is_blocked_and_dialog_closes() {
    let Some((mut app, doc)) = q30e_app(
        "<form id=f action='http://example.com/x'><input name=q></form><dialog id=d open><form id=m method=dialog><button id=ok value=yes>OK</button></form></dialog>",
        "window.L=[]; document.getElementById('d').addEventListener('close',function(){L.push('closed:'+this.returnValue)});",
    ) else {
        return;
    };
    app.base_url = Some(Url::parse("https://secure.example/").unwrap());
    app.form_navigate(&q30c_by(&doc, "f"), None);
    assert_eq!(app.nav_generation, 0, "https page -> http action is blocked");
    app.click_activate(Some(&q30c_by(&doc, "ok")), (0.0, 0.0));
    assert_eq!(q30e_log(&app), "closed:yes");
    assert_eq!(app.nav_generation, 0, "method=dialog never navigates");
}

#[test]
fn q30e_formdata_of_a_form_reads_the_data_set() {
    clear_check_state();
    clear_text_state();
    let doc = parse_html("<html><body><form id=f><input name=a value=1><input type=checkbox name=c checked><textarea name=t>x</textarea><button name=b value=bv>B</button></form></body></html>");
    let js = run_page_scripts(&doc, &[src("var f=document.getElementById('f'); var d=new FormData(f); var e=new FormData(f, f.querySelector('button')); window.out=[Array.from(d.keys()).join(','), Array.from(e.keys()).join(','), d.get('t')].join('|');")], None).expect("js");
    let got: String = js.eval_value("t", "window.out").expect("out");
    assert_eq!(got, "a,c,t|a,c,t,b|x");
>>>>>>> 06e5f40 (scratchy: Q30e: form submission (data set, GET/POST/multipart/text-plain, submit events, implicit submission, dialog, FormData(form), final-URL tracking))
}
