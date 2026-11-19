#[allow(unused_imports)]
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

// ---------- QY53i: custom-property memo (parent table address + own pairs) ----------

#[test]
fn qy53i_siblings_with_same_own_custom_prop_share_one_map() {
    let css = ":root{--a:1px} .s{--x:2px}";
    let doc = parse_html(r#"<html><body><p class="s" id="a">a</p><p class="s" id="b">b</p></body></html>"#);
    let rules = parse_stylesheet(css);
    let html = find_html_element(&doc).unwrap();
    let hs = resolve_style(&html, &rules, ROOT_STYLE, 800.0);
    let inherited = InheritedStyle { custom_props: hs.custom_props, ..ROOT_STYLE };
    let a = resolve_style(&by_id(&doc, "a"), &rules, inherited, 800.0);
    let hits = crate::css::CUSTOM_PROPS_MEMO_HITS.with(|c| c.get());
    let b = resolve_style(&by_id(&doc, "b"), &rules, inherited, 800.0);
    let (pa, pb) = (a.custom_props.expect("a table"), b.custom_props.expect("b table"));
    assert!(pa == pb, "siblings with the same own --x share one interned table");
    assert_eq!(crate::css::CUSTOM_PROPS_MEMO_HITS.with(|c| c.get()), hits + 1, "the second sibling is a memo hit");
    assert_eq!(pa.get("--x").as_deref(), Some("2px"));
    assert_eq!(pa.get("--a").as_deref(), Some("1px"));
}

#[test]
fn qy53i_same_own_raw_value_under_different_parents_resolves_per_parent() {
    // Same own declaration (`--y: var(--x)`) under two parents with
    // different `--x`: the memo key includes the parent table, so each
    // child resolves against its own parent.
    let html = r#"<html><head><style>.k{--y:var(--x)} .k p{color:var(--y)}</style></head><body>
        <div style="--x:#010101"><div class="k"><p>one</p></div></div>
        <div style="--x:#020202"><div class="k"><p>two</p></div></div>
        <div style="--x:#010101"><div class="k"><p>three</p></div></div></body></html>"#;
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert_eq!(
        colors,
        vec![TextColor::rgb(1, 1, 1), TextColor::rgb(2, 2, 2), TextColor::rgb(1, 1, 1)],
        "each var(--y) resolves against its own parent's --x"
    );
}

// ---------- QY53i2: the custom-property arena is freed on navigation ----------

#[test]
fn qy53i2_reset_frees_the_arena_and_old_handles_read_empty() {
    use crate::css::{CUSTOM_PROPS_FREED, custom_props_mem, intern_custom_props, reset_custom_props_arena};
    reset_custom_props_arena();
    let mut t = crate::css::CustomProps::new();
    t.insert("--qy53i2".into(), "1px".into());
    let old = intern_custom_props(t.clone());
    let mut u = t.clone();
    u.insert("--other".into(), "2px".into());
    intern_custom_props(u);
    assert_eq!(custom_props_mem().0.0, 2);
    assert_eq!(old.get("--qy53i2").as_deref(), Some("1px"));
    let freed = CUSTOM_PROPS_FREED.with(|c| c.get());
    reset_custom_props_arena();
    assert_eq!(CUSTOM_PROPS_FREED.with(|c| c.get()), freed + 2, "both tables freed");
    assert_eq!(custom_props_mem(), ((0, 0), (0, 0)), "arena and memo are empty");
    // A style that outlived its document reads no custom properties, never a new table's.
    let new = intern_custom_props(t);
    assert_ne!(old, new, "a new generation");
    assert_eq!(old.get("--qy53i2"), None);
    assert!(old.table().is_none());
    assert_eq!(new.get("--qy53i2").as_deref(), Some("1px"));
}

#[test]
fn qy53i2_var_inheritance_override_and_cycle_survive_a_reset() {
    let html = r#"<html><head><style>:root{--c:#030303} .o{--c:#040404} .i,.o p{color:var(--c)}
        .cy{--a:var(--b);--b:var(--a);color:var(--a,#050505)}</style></head><body>
        <p class="i">inherit</p><div class="o"><p>override</p></div><div class="cy"><p>cycle</p></div></body></html>"#;
    let doc = parse_html(html);
    let colors = |doc: &Handle| {
        let tree = build_layout_tree(doc, 1920.0);
        let mut c = Vec::new();
        find_all_span_colors(&tree, &mut c);
        c
    };
    let before = colors(&doc);
    crate::css::reset_custom_props_arena();
    let after = colors(&doc);
    assert_eq!(before, after);
    assert_eq!(after.len(), 3, "{after:?}");
    assert_eq!(after[0], TextColor::rgb(3, 3, 3), "inherited from :root");
    assert_eq!(after[1], TextColor::rgb(4, 4, 4), "override");
    assert_eq!(after[2], TextColor::rgb(5, 5, 5), "a cycle falls back");
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
    assert!(rules[0].decls().color.is_some(), "the supported declaration in the same rule should still apply");
    assert!(
        rules[0].decls().grid_area.is_none(),
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
    assert!(r[0].decls().color.is_some());
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

// ---------- QY53a: first-line / first-letter rule presence is computed once per rule list ----------

fn qy53_any_first_line(b: &LayoutBox) -> bool {
    b.style.first_line.is_some()
        || match &b.content {
            BoxContent::Blocks(c) => c.iter().any(qy53_any_first_line),
            _ => false,
        }
}

fn qy53_page(extra_css: &str, paragraphs: usize) -> String {
    let mut body = String::new();
    for i in 0..paragraphs {
        body.push_str(&format!("<p class=\"k{i}\">alpha beta gamma delta {i}</p>"));
    }
    format!("<html><head><style>p {{ margin: 0 }} {extra_css}</style></head><body>{body}</body></html>")
}

#[test]
fn qy53a_pseudo_rule_scan_runs_once_per_rule_list_not_per_element() {
    let doc = parse_html(&qy53_page("", 60));
    crate::style::PSEUDO_SCANS.with(|c| c.set(0));
    let tree = build_layout_tree(&doc, 800.0);
    let first = crate::style::PSEUDO_SCANS.with(|c| c.get());
    assert_eq!(first, 2, "one first-line and one first-letter scan for the whole build");
    assert!(!qy53_any_first_line(&tree));
    // Same CSS again: the parsed rules (and their flags) are reused, no scan at all.
    let _ = build_layout_tree(&doc, 800.0);
    assert_eq!(crate::style::PSEUDO_SCANS.with(|c| c.get()), first);
}

#[test]
fn qy53a_a_first_line_rule_added_later_is_seen() {
    let doc = parse_html(&qy53_page("", 3));
    assert!(!qy53_any_first_line(&build_layout_tree(&doc, 800.0)));
    // A script adds a stylesheet with a ::first-line rule: new CSS text, new rule list, flags recomputed.
    let head = find_by_tag(&doc, "head").unwrap();
    let style = markup5ever_rcdom::Node::new(NodeData::Element {
        name: html5ever::QualName::new(None, html5ever::ns!(html), html5ever::LocalName::from("style")),
        attrs: std::cell::RefCell::new(Vec::new()),
        template_contents: std::cell::RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    let text = markup5ever_rcdom::Node::new(NodeData::Text { contents: std::cell::RefCell::new("p::first-line { color: #f00 }".into()) });
    text.parent.set(Some(std::rc::Rc::downgrade(&style)));
    style.children.borrow_mut().push(text);
    style.parent.set(Some(std::rc::Rc::downgrade(&head)));
    head.children.borrow_mut().push(style);
    assert!(qy53_any_first_line(&build_layout_tree(&doc, 800.0)), "::first-line applied after the rule appeared");
}

#[test]
fn qy53a_style_text_change_drops_the_flag() {
    let with = parse_html(&qy53_page("p::first-line { color: #f00 }", 2));
    assert!(qy53_any_first_line(&build_layout_tree(&with, 800.0)));
    let without = parse_html(&qy53_page("", 2));
    assert!(!qy53_any_first_line(&build_layout_tree(&without, 800.0)));
    assert!(qy53_any_first_line(&build_layout_tree(&with, 800.0)), "flipping back works too");
}

// ---------- QY53b: RuleIndex / HoverKeys are kept while the rule list is unchanged ----------

fn qy53_derived_builds() -> usize {
    crate::style::DERIVED_BUILDS.with(|c| c.get())
}

fn qy53_colors(doc: &Handle) -> Vec<TextColor> {
    let tree = build_layout_tree(doc, 800.0);
    let mut out = Vec::new();
    find_all_span_colors(&tree, &mut out);
    out
}

#[test]
fn qy53b_index_is_built_once_for_unchanged_rules_and_rebuilt_on_css_change() {
    let html = qy53_page(".k0 { color: #ff0000 } .k1 { color: #0000ff } p:hover { color: #00ff00 }", 4);
    let doc = parse_html(&html);
    let before = qy53_derived_builds();
    let c1 = qy53_colors(&doc);
    assert_eq!(qy53_derived_builds(), before + 1);
    let c2 = qy53_colors(&doc);
    let c3 = qy53_colors(&doc);
    assert_eq!(qy53_derived_builds(), before + 1, "same CSS: no rebuild");
    assert_eq!(c1, c2);
    assert_eq!(c1, c3);
    assert!(c1.contains(&TextColor::rgb(0xff, 0, 0)) && c1.contains(&TextColor::rgb(0, 0, 0xff)), "{c1:?}");
    // The hover keys survive with the cached rules too.
    let hk = crate::style::HOVER_KEYS.with(|k| k.borrow().clone());
    let doc2 = parse_html(&qy53_page(".k0 { color: #ff0000 } .k1 { color: #0000ff } p:hover { color: #00ff00 }", 4));
    let _ = build_layout_tree(&doc2, 800.0);
    assert_eq!(crate::style::HOVER_KEYS.with(|k| k.borrow().clone()).tags, hk.tags);
    // Changed CSS: rebuilt, and the new rules apply.
    let doc3 = parse_html(&qy53_page(".k0 { color: #00ffff } .k1 { color: #0000ff }", 4));
    let c4 = qy53_colors(&doc3);
    assert_eq!(qy53_derived_builds(), before + 2);
    assert!(c4.contains(&TextColor::rgb(0, 0xff, 0xff)) && !c4.contains(&TextColor::rgb(0xff, 0, 0)), "{c4:?}");
}

#[test]
fn qy53b_class_change_with_cached_index_matches_the_new_class() {
    let doc = parse_html(&qy53_page(".k0 { color: #ff0000 } .k1 { color: #0000ff } .z { color: #00aa00 }", 2));
    let first = qy53_colors(&doc);
    assert!(first.contains(&TextColor::rgb(0xff, 0, 0)), "{first:?}");
    let builds = qy53_derived_builds();
    // A script changes the class of the first <p>: same rules, cached index.
    let p = find_by_tag(&doc, "p").unwrap();
    crate::js_dom::set_attr(&p, "class", "z");
    let second = qy53_colors(&doc);
    assert_eq!(qy53_derived_builds(), builds, "rules unchanged: index reused");
    assert!(second.contains(&TextColor::rgb(0, 0xaa, 0)) && !second.contains(&TextColor::rgb(0xff, 0, 0)), "{second:?}");
}

// ---------- QY53d: one selector match per (element, pseudo) per build ----------

fn qy53d_count_elements(h: &Handle) -> usize {
    let own = usize::from(matches!(h.data, NodeData::Element { .. }));
    own + h.children.borrow().iter().map(qy53d_count_elements).sum::<usize>()
}

#[test]
fn qy53d_each_element_and_pseudo_is_matched_at_most_once_per_build() {
    let css = "body { counter-reset: n } p { counter-increment: n } p::before { content: counter(n) \". \" } \
               p::after { content: \" <\" attr(class) \">\" } .k1 { background-image: none }";
    let doc = parse_html(&qy53_page(css, 30));
    let elements = qy53d_count_elements(&doc);
    crate::style::MATCH_RUNS.with(|c| c.set(0));
    let tree = build_layout_tree(&doc, 800.0);
    let runs = crate::style::MATCH_RUNS.with(|c| c.get());
    // element, ::before, ::after, ::first-line, ::first-letter, ::marker at most.
    assert!(runs <= elements * 6, "{runs} matching passes for {elements} elements");
    // The counter/content pass still produced its text.
    let mut text = String::new();
    collect_all_text(&tree, &mut text);
    assert!(text.contains("1. alpha") && text.contains("30. alpha"), "{text}");
    assert!(text.contains("<k3>"), "{text}");
    // A second build starts with a fresh memo and does the same work.
    crate::style::MATCH_RUNS.with(|c| c.set(0));
    let _ = build_layout_tree(&doc, 800.0);
    assert_eq!(crate::style::MATCH_RUNS.with(|c| c.get()), runs, "memo is per build");
}

#[test]
fn qy53d_memo_is_dropped_after_the_build_so_a_changed_dom_is_matched_again() {
    let doc = parse_html(&qy53_page(".k0 { color: #ff0000 } .z { color: #0000ff }", 2));
    let before = qy53_colors(&doc);
    let p = find_by_tag(&doc, "p").unwrap();
    if let NodeData::Element { attrs, .. } = &p.data {
        for a in attrs.borrow_mut().iter_mut() {
            if &*a.name.local == "class" {
                a.value = "z".into();
            }
        }
    }
    let after = qy53_colors(&doc);
    assert_ne!(before, after, "attribute change must restyle");
    assert!(crate::style::MATCH_MEMO.with(|m| m.borrow().is_none()));
}

#[test]
fn qy53g_class_list_contains_splits_on_ascii_whitespace_only() {
    use crate::dom::class_list_contains;
    use selectors::attr::CaseSensitivity::{AsciiCaseInsensitive, CaseSensitive};
    let cs = CaseSensitive;
    // HTML's ASCII whitespace: space, tab, LF, FF, CR.
    for sep in [" ", "\t", "\n", "\x0C", "\r", " \t\n\x0C\r "] {
        let v = format!("{sep}aa{sep}b{sep}ccc{sep}");
        for name in ["aa", "b", "ccc"] {
            assert!(class_list_contains(&v, name, cs), "{name:?} in {v:?}");
        }
        assert!(!class_list_contains(&v, "a", cs), "prefix of a class is not a class: {v:?}");
        assert!(!class_list_contains(&v, "cc", cs));
        assert!(!class_list_contains(&v, "aab", cs));
    }
    // Not separators: U+000B (vertical tab) and Unicode spaces.
    assert!(!class_list_contains("a\x0Bb", "a", cs));
    assert!(class_list_contains("a\x0Bb", "a\x0Bb", cs));
    assert!(!class_list_contains("a\u{00A0}b", "a", cs));
    assert!(!class_list_contains("a\u{3000}b", "b", cs));
    assert!(class_list_contains("x a\u{00A0}b", "a\u{00A0}b", cs));
    // Empty and degenerate inputs.
    assert!(!class_list_contains("", "a", cs));
    assert!(!class_list_contains("a b", "", cs));
    assert!(!class_list_contains("   ", "a", cs));
    // Case sensitivity is the caller's (quirks mode); default is sensitive.
    assert!(!class_list_contains("Foo", "foo", cs));
    assert!(class_list_contains("x\tFoo", "foo", AsciiCaseInsensitive));
    // Non-ASCII class names compare byte-exactly.
    assert!(class_list_contains("é\tü", "ü", cs));
}

#[test]
fn qy53g_class_selectors_match_with_tab_newline_and_form_feed_separators() {
    // Tab, LF and FF between classes, also on the ancestor a descendant
    // selector walks; U+00A0 must not split.
    let html = "<html><head><style>\
        .a .b { color: #ff0000; } \
        .nb { color: #00ff00; } \
        </style></head><body>\
        <div class=\"x\ta\ny\"><p class=\"q\x0Cb\">one</p></div>\
        <div class=\"zz\"><p class=\"b\">two</p></div>\
        <p class=\"m\u{00A0}nb\">three</p>\
        </body></html>";
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut colors = Vec::new();
    find_all_span_colors(&tree, &mut colors);
    assert_eq!(colors.len(), 3, "three paragraphs of spans");
    assert_eq!(colors[0], TextColor::rgb(0xff, 0, 0), "tab/LF on the ancestor and FF on the element split classes");
    assert_ne!(colors[1], TextColor::rgb(0xff, 0, 0), "no .a ancestor");
    assert_ne!(colors[2], TextColor::rgb(0, 0xff, 0), "U+00A0 is not ASCII whitespace, so there is no class nb");
}

#[test]
fn qy53g_pseudo_element_rules_are_indexed_by_their_originating_compound() {
    let rules = parse_stylesheet(
        ".a::before{content:'x'} p::after{content:'y'} #x::marker{color:red} .q .b.c::first-line{color:red} \
         ::before{content:'z'} :hover::after{content:'w'}",
    );
    let ix = RuleIndex::build(&rules);
    assert_eq!(ix.pseudo.universal, vec![4, 5], "only the pseudo rules with no id/class/tag stay universal");
    assert_eq!(ix.pseudo.by_class.get("a"), Some(&vec![0]));
    assert_eq!(ix.pseudo.by_tag.get("p"), Some(&vec![1]));
    assert_eq!(ix.pseudo.by_id.get("x"), Some(&vec![2]));
    assert_eq!(ix.pseudo.by_class.get("b"), Some(&vec![3]), "rightmost compound of the originating element");
    assert!(ix.normal.universal.is_empty() && ix.normal.by_class.is_empty());
}

fn qy53g_elements(h: &Handle, out: &mut Vec<Handle>) {
    if matches!(h.data, NodeData::Element { .. }) {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        qy53g_elements(c, out);
    }
}

#[test]
fn qy53g_index_and_ancestor_bloom_give_the_same_matches_as_a_full_scan() {
    let css = ".a .b{color:red} .a > .b{margin:0} #root .c{padding:0} div div p{color:blue} \
               .x .y .z{color:green} .nope .b{color:pink} ul > li + li{color:red} .a ~ .b{margin:1px} \
               :is(.a) .c{color:red} :where(.k, .a) p{color:red} :not(.zz) .b{margin:2px} section .b{color:red} \
               .a .b::before{content:'x'} .nope p::after{content:'y'} div::after{content:'z'} \
               .x li::marker{color:red} .a p::first-line{color:red} *{padding:1px} [title] .b{color:red}";
    let doc = parse_html(
        "<html><body id=root><div class='a'><div class=\"x\tb\"><p class=\"b\nc y\">t<span class=z>s</span></p>\
         <ul><li>1<li class=b>2<li title=t><span class='b c'>3</span></ul></div><p class=b>q</p></div>\
         <section><div class=k><p class=c>r</p></div></section><div title=t><b class=b>w</b></div></body></html>",
    );
    let rules = parse_stylesheet(css);
    let mut elements = Vec::new();
    qy53g_elements(&doc, &mut elements);
    let pseudos = [None, Some("before"), Some("after"), Some("marker"), Some("first-line")];
    let summarize = |h: &Handle| -> Vec<Vec<u64>> {
        pseudos
            .iter()
            .map(|p| matching_declarations_for(h, &rules, 800.0, *p).iter().map(|(sp, _)| *sp).collect())
            .collect()
    };
    let full: Vec<_> = elements.iter().map(summarize).collect();
    RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), RuleIndex::build(&rules))));
    ANCESTOR_BLOOM.with(|b| *b.borrow_mut() = Some(Default::default()));
    // Document order, then reverse order (the filter must pop and re-push).
    let forward: Vec<_> = elements.iter().map(summarize).collect();
    let mut backward: Vec<_> = elements.iter().rev().map(summarize).collect();
    backward.reverse();
    ANCESTOR_BLOOM.with(|b| *b.borrow_mut() = None);
    RULE_INDEX.with(|ri| *ri.borrow_mut() = None);
    assert_eq!(full, forward);
    assert_eq!(full, backward);
    assert!(full.iter().filter(|m| !m[0].is_empty()).count() > 5);
    assert!(full.iter().any(|m| !m[1].is_empty()), "some ::before rule matches");
}

#[test]
fn qy53g_ancestor_bloom_holds_exactly_the_ancestor_chain() {
    use crate::dom::atom_hash;
    let doc = parse_html(
        "<html><body><div id=d class=\"a\tb\"><p class=p1><span class=s1>x</span></p></div>\
         <section class=other><em class=e>y</em></section></body></html>",
    );
    let span = find_by_tag(&doc, "span").unwrap();
    let em = find_by_tag(&doc, "em").unwrap();
    let mut bloom = AncestorBloom::default();
    let f = bloom.filter_for(&span);
    for k in ["html", "body", "div", "p", "d", "a", "b", "p1"] {
        assert!(f.might_contain_hash(atom_hash(k)), "{k} is on span's ancestor chain");
    }
    for k in ["s1", "section", "other", "e"] {
        assert!(!f.might_contain_hash(atom_hash(k)), "{k} is not an ancestor of span");
    }
    let f = bloom.filter_for(&em);
    for k in ["section", "other", "body"] {
        assert!(f.might_contain_hash(atom_hash(k)), "{k} is on em's ancestor chain");
    }
    for k in ["div", "a", "b", "p1", "d", "e"] {
        assert!(!f.might_contain_hash(atom_hash(k)), "{k} was popped when moving to em");
    }
}

// ---------- QY53f1: per-piece rule cache (incremental `<style>` parse) ----------

fn qy53f1_page(styles: &[&str]) -> Handle {
    let mut s = String::from("<html><head>");
    for st in styles {
        s.push_str(&format!("<style>{st}</style>"));
    }
    s.push_str("</head><body><p id=a class=k0>x</p><p id=b class=k1>y</p><p id=c class=k0 title=t>z</p></body></html>");
    parse_html(&s)
}

fn qy53f1_style_nodes(doc: &Handle) -> Vec<Handle> {
    let mut v = Vec::new();
    crate::images::collect_style_nodes(doc, &mut v);
    v
}

fn qy53f1_set_text(style: &Handle, css: &str) {
    for c in style.children.borrow().iter() {
        if let NodeData::Text { contents } = &c.data {
            *contents.borrow_mut() = css.into();
            return;
        }
    }
    panic!("style without text node");
}

fn qy53f1_parsed() -> usize {
    crate::rules_cache::PIECES_PARSED.with(|c| c.get())
}

fn qy53f1_reset() {
    crate::rules_cache::PIECES_PARSED.with(|c| c.set(0));
    crate::rules_cache::FULL_PARSES.with(|c| c.set(0));
}

/// Colours from the warm (incremental) cache, then from a forced full parse; they must agree.
fn qy53f1_check(doc: &Handle) -> Vec<TextColor> {
    let warm = qy53_colors(doc);
    let warm_len = crate::rules_cache::rule_count();
    RULES_CACHE.with(|c| *c.borrow_mut() = None);
    let full = qy53_colors(doc);
    assert_eq!(warm, full, "incremental cascade equals a full parse");
    assert_eq!(warm_len, crate::rules_cache::rule_count(), "same rule list length");
    warm
}

#[test]
fn qy53f1_insert_change_remove_in_the_middle_equals_full_parse() {
    let doc = qy53f1_page(&[".k0 { color: #ff0000 }", ".k1 { color: #0000ff }", "#c { color: #00ff00 }"]);
    qy53f1_check(&doc);
    let head = find_by_tag(&doc, "head").unwrap();
    // Insert a piece in the middle.
    let frag = parse_html("<style>.k1 { color: #123456 } p[title] { color: #654321 }</style>");
    let mut nodes = qy53f1_style_nodes(&doc);
    let new = qy53f1_style_nodes(&frag).remove(0);
    let pos = head.children.borrow().iter().position(|c| Rc::ptr_eq(c, &nodes[1])).unwrap();
    head.children.borrow_mut().insert(pos, new);
    qy53f1_reset();
    let after_insert = qy53_colors(&doc);
    assert_eq!(qy53f1_parsed(), 1, "only the inserted piece parsed");
    assert_eq!(after_insert, qy53f1_check(&doc));
    // Change the first piece.
    nodes = qy53f1_style_nodes(&doc);
    qy53f1_set_text(&nodes[0], ".k0 { color: #abcdef }");
    qy53f1_reset();
    let changed = qy53_colors(&doc);
    assert_eq!(qy53f1_parsed(), 1, "only the edited piece parsed");
    assert_ne!(changed, after_insert);
    assert_eq!(changed, qy53f1_check(&doc));
    // Remove the middle piece.
    let mid = nodes[1].clone();
    head.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, &mid));
    qy53f1_reset();
    let removed = qy53_colors(&doc);
    assert_eq!(qy53f1_parsed(), 0, "removal parses nothing");
    assert_eq!(removed, qy53f1_check(&doc));
}

#[test]
fn qy53f1_unchanged_document_parses_nothing() {
    let doc = qy53f1_page(&[".k0 { color: #ff0000 }", ".k1 { color: #0000ff }"]);
    qy53_colors(&doc);
    qy53f1_reset();
    qy53_colors(&doc);
    assert_eq!(qy53f1_parsed(), 0);
    assert_eq!(crate::rules_cache::FULL_PARSES.with(|c| c.get()), 0);
}

#[test]
fn qy53f1_layer_falls_back_to_full_parse() {
    let doc = qy53f1_page(&["@layer a, b; @layer b { .k0 { color: #ff0000 } } @layer a { .k0 { color: #0000ff } }", ".k1 { color: #00ff00 }"]);
    let first = qy53f1_check(&doc);
    // b is later than a, so b's red wins over a's blue.
    assert_eq!(format!("{:?}", first[0]), format!("{:?}", qy53_colors(&qy53f1_page(&[".k0 { color: #ff0000 }"]))[0]));
    let nodes = qy53f1_style_nodes(&doc);
    qy53f1_set_text(&nodes[1], ".k1 { color: #111111 }");
    qy53f1_reset();
    let c = qy53_colors(&doc);
    assert_eq!(crate::rules_cache::FULL_PARSES.with(|c| c.get()), 1, "layered sheet: full parse");
    assert_eq!(c, qy53f1_check(&doc));
}

#[test]
fn qy53f1_order_and_specificity_ties_across_pieces() {
    // Equal specificity: the later piece wins; higher specificity wins regardless of order.
    let doc = qy53f1_page(&[".k0 { color: #ff0000 } p#a { color: #0000ff }", ".k0 { color: #00ff00 }"]);
    let c = qy53f1_check(&doc);
    assert_ne!(c[0], c[2], "id selector beats the later class on #a only");
    let green = qy53_colors(&qy53f1_page(&[".k0 { color: #00ff00 }"]))[0];
    assert_eq!(c[2], green, "later piece wins an equal-specificity tie");
    // Insert a piece before: it now loses the tie to both of the others.
    let head = find_by_tag(&doc, "head").unwrap();
    let frag = parse_html("<style>.k0 { color: #010101 }</style>");
    let new = qy53f1_style_nodes(&frag).remove(0);
    head.children.borrow_mut().insert(0, new);
    let c = qy53f1_check(&doc);
    assert_eq!(c[2], green);
}

#[test]
fn qy53f1_external_css_piece_changes_reparse_only_it() {
    let doc = qy53f1_page(&[".k0 { color: #ff0000 }"]);
    let t = |extra: &str| {
        let tree = build_layout_tree_with_resources(&doc, 800.0, None, extra, &HashMap::new());
        let mut out = Vec::new();
        find_all_span_colors(&tree, &mut out);
        out
    };
    let a = t(".k0 { color: #0000ff }");
    qy53f1_reset();
    let a2 = t(".k0 { color: #0000ff }");
    assert_eq!(qy53f1_parsed(), 0);
    assert_eq!(a, a2);
    let b = t(".k0 { color: #00ff00 }");
    assert_eq!(qy53f1_parsed(), 1, "only the external piece");
    assert_ne!(a, b);
}

#[test]
fn qy53f1_repeated_insert_and_change_keeps_the_cache_bounded() {
    let doc = qy53f1_page(&[".k0 { color: #ff0000 }", ".k1 { color: #0000ff }"]);
    qy53_colors(&doc);
    let head = find_by_tag(&doc, "head").unwrap();
    let (entries0, _) = crate::style::rules_cache_mem().0;
    let pieces = |_: ()| RULES_CACHE.with(|c| c.borrow().as_ref().map_or(0, |r| r.piece_count()));
    for i in 0..20 {
        // Insert a new <style>, build, then change its text and build, then remove it.
        let frag = parse_html(&format!("<style>.s{i} {{ color: #00000{} }}</style>", i % 10));
        let new = qy53f1_style_nodes(&frag).remove(0);
        head.children.borrow_mut().push(new.clone());
        qy53_colors(&doc);
        qy53f1_set_text(&new, &format!(".t{i} {{ color: #0a0b0c }}"));
        qy53_colors(&doc);
        assert_eq!(pieces(()), 4, "2 originals + 1 inserted + the external piece");
        head.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, &new));
        qy53_colors(&doc);
        assert_eq!(pieces(()), 3);
    }
    assert_eq!(crate::style::rules_cache_mem().0.0, entries0, "no rules left behind by removed pieces");
}

/// QY53f1b: parsing into an existing list appends (the earlier rules are untouched, layer ranks
/// are fixed on the appended tail only) and equals `parse_stylesheet_with_keyframes`.
#[test]
fn qy53f1b_parse_stylesheet_into_appends_and_matches_the_returning_form() {
    let a = "p { color: red } @keyframes k { from { opacity: 0 } to { opacity: 1 } }";
    let b = "@layer base, over; @layer over { p { color: blue } } @layer base { p { color: green } } div { margin: 0 }";
    let (ra, ka) = crate::css::parse_stylesheet_with_keyframes(a);
    let (rb, kb) = crate::css::parse_stylesheet_with_keyframes(b);
    let mut into = Vec::new();
    let ka2 = crate::css::parse_stylesheet_into(a, &mut into);
    assert_eq!(into.len(), ra.len());
    let n = into.len();
    let kb2 = crate::css::parse_stylesheet_into(b, &mut into);
    assert_eq!(into.len(), ra.len() + rb.len());
    assert_eq!(ka.len(), ka2.len());
    assert_eq!(kb.len(), kb2.len());
    for (x, y) in ra.iter().chain(rb.iter()).zip(into.iter()) {
        assert_eq!(x.layer, y.layer, "layer ranks preserved");
    }
    assert!(into[..n].iter().all(|r| r.layer == 0), "the head keeps its unlayered rank");
    assert!(into[n..].iter().any(|r| r.layer >= 2), "the tail got layer ranks");
}

/// QY53f1c: `StyleRule` was 4040 bytes (two inline `Declarations` of ~2 KB each); the block is
/// now kept as text and parsed on first use, so the rule list of a big sheet stays small.
#[test]
fn qy53f1c_style_rule_is_small() {
    let n = std::mem::size_of::<StyleRule>();
    assert!(n <= 512, "size_of::<StyleRule>() = {n}");
}

/// QY53f1c: parsing a sheet parses no declaration block; the cascade parses only the blocks
/// of rules that match, and the lazily parsed values equal the eagerly parsed ones.
#[test]
fn qy53f1c_declaration_blocks_parse_lazily_and_match_the_eager_parse() {
    let css = "p { color: red; margin: 1px 2px !important } .never { color: blue } div { width: 5px }";
    let doc = parse_html("<html><body><p id=p>x</p><div>y</div></body></html>");
    let start = crate::css::BLOCKS_EXPANDED.with(|c| c.get());
    let rules = parse_stylesheet(css);
    assert_eq!(rules.len(), 3);
    assert!(rules.iter().all(|r| !r.is_parsed()), "no block parsed by the sheet parse");
    let p = find_element_by_tag(&doc, "p").expect("p");
    let st = resolve_style(&p, &rules, ROOT_STYLE, 800.0);
    assert!(rules[0].is_parsed(), "the matching rule was parsed");
    assert!(!rules[1].is_parsed() && !rules[2].is_parsed(), "non-matching rules stay unparsed");
    assert_eq!(crate::css::BLOCKS_EXPANDED.with(|c| c.get()) - start, 1);
    assert!(st.color != ROOT_STYLE.color, "the matched block applied");
    assert!(rules[0].important().margin_top.is_some() && rules[0].decls().margin_top.is_none());
    // The eager parse (the `--css-debug` path) gives the same declarations.
    crate::css::CSS_DEBUG_CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let eager = parse_stylesheet(css);
    crate::css::CSS_DEBUG_CAPTURE.with(|c| *c.borrow_mut() = None);
    assert!(eager.iter().all(|r| r.is_parsed()));
    for (a, b) in rules.iter().zip(eager.iter()) {
        assert!(a.decls() == b.decls() && a.important() == b.important());
    }
    assert!(rules[1].important() == &Declarations::default(), "no !important: the empty set");
}

/// QY53f1c: the generated-content pre-pass only parses blocks that may declare
/// `content`/counters/`quotes`, and still finds them.
#[test]
fn qy53f1c_precompute_generated_parses_only_candidate_blocks() {
    let css = "p { color: red } li::before { content: counter(c) } ol { counter-reset: c } li { counter-increment: c }";
    let doc = parse_html("<html><body><ol><li>a</li><li>b</li></ol><p>x</p></body></html>");
    // Without any candidate block the pass parses nothing (it returns before the walk).
    let plain = parse_stylesheet("p { color: red }");
    precompute_generated(&doc, &plain, 800.0);
    assert!(!plain[0].is_parsed(), "a block without generated-content properties stays unparsed");
    // With candidates, the walk parses the matched blocks only (`p` is not in the document).
    let doc = parse_html("<html><body><ol><li>a</li><li>b</li></ol></body></html>");
    let rules = parse_stylesheet(css);
    precompute_generated(&doc, &rules, 800.0);
    assert!(!rules[0].is_parsed(), "an unmatched non-candidate block stays unparsed");
    assert!(rules[1].is_parsed() && rules[2].is_parsed() && rules[3].is_parsed());
    assert!(!rules[0].may_declare_generated() && rules[1].may_declare_generated());
}
