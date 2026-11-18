#[allow(unused_imports)]
use super::*;

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
