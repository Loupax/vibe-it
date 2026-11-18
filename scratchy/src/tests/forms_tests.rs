#[allow(unused_imports)]
use super::*;

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
    // Q30g: the closed text, the arrow and the listbox rows are painted from the
    // live selection (forms_select.rs); the tree only sizes the box.
    fn shown(html: &str) -> (String, bool, Vec<String>) {
        clear_select_state();
        let doc = parse_html(&format!("<html><body style=\"margin:0\">{html}</body></html>"));
        let _ = build_layout_tree(&doc, 1000.0);
        let h = find_element_by_tag(&doc, "select").expect("select");
        let i = select_paint_info(node_id(&h)).expect("registered by the build");
        (i.shown_text(), i.snap.listbox, i.snap.rows.iter().map(|r| r.label.clone()).collect())
    }
    let a = shown("<select><option>Alpha<option selected>Beta<option>Gamma</select>");
    assert_eq!((a.0.as_str(), a.1), ("Beta", false));
    assert_eq!(shown("<select><option>One<option>Two</select>").0, "One");
    assert_eq!(shown("<select style=\"appearance:none\"><option>One</select>").0, "One");
    let lb = shown("<select multiple size=3><option>A<option>B<option>C</select>");
    assert_eq!((lb.1, lb.2), (true, vec!["A".to_string(), "B".to_string(), "C".to_string()]));
    clear_select_state();
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
    let src = include_str!("../layout.rs");
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
    // Compared with one line rather than a pixel count: line height differs per font (R33).
    let ink = |px: &[u32]| (0..80usize).filter(|y| (0..120usize).any(|x| px[y * 120 + x] != 0x00FFFFFF)).count();
    let rows = q30d_render(&t, TEXT_MULTI, false, 120, 80);
    set_text_value(&t, "line one");
    let one = q30d_render(&t, TEXT_MULTI, false, 120, 80);
    assert!(ink(&rows) > 2 * ink(&one), "three lines of text paint on more rows than one, got {} vs {}", ink(&rows), ink(&one));
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
}

// ---- Q30g: live <select> state, popup state machine, keyboard --------------

fn q30g_doc(html: &str) -> Handle {
    clear_select_state();
    clear_check_state();
    clear_text_state();
    parse_html(&format!("<html><body>{html}</body></html>"))
}

fn q30g_selects(h: &Handle, out: &mut Vec<Handle>) {
    if element_tag(h) == "select" {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        q30g_selects(c, out);
    }
}

#[test]
fn q30g_defaults_single_multiple_listbox_optgroup_disabled() {
    let doc = q30g_doc(
        r#"<select id=a><option>x<option selected>y<option selected>z</select>
        <select id=b><option disabled>d<option>e<option>f</select>
        <select id=c multiple><option selected>1<option>2<option selected>3</select>
        <select id=d size=3><option>1<option>2</select>
        <select id=e><optgroup label=G disabled><option>g1<option>g2</optgroup><optgroup label=H><option>h1</optgroup></select>
        <select id=f></select>"#,
    );
    let idx = |id: &str| selected_indices(&q30c_by(&doc, id));
    assert_eq!(idx("a"), vec![2], "last selected attribute wins");
    assert_eq!(idx("b"), vec![1], "first non-disabled");
    assert_eq!(idx("c"), vec![0, 2]);
    assert!(idx("d").is_empty(), "single listbox selects nothing by default");
    assert_eq!(idx("e"), vec![2], "options of a disabled optgroup are disabled");
    assert!(idx("f").is_empty());
    let rows = select_rows(&q30c_by(&doc, "e"));
    assert_eq!(rows.len(), 5);
    assert!(rows[0].opt.is_none() && rows[0].disabled && rows[0].label == "G");
    assert!(rows[1].grouped && rows[1].disabled && !rows[4].disabled);
    clear_select_state();
}

#[test]
fn q30g_live_selection_option_selected_and_reset() {
    let doc = q30g_doc(
        r#"<form id=f><select id=s name=s><option id=o0 value=a>A<option id=o1 value=b selected>B<option id=o2 value=c>C</select>
        <select id=m name=m multiple><option id=m0>1<option id=m1>2<option id=m2>3</select></form>"#,
    );
    let (s, f) = (q30c_by(&doc, "s"), q30c_by(&doc, "f"));
    let (o0, o1, o2) = (q30c_by(&doc, "o0"), q30c_by(&doc, "o1"), q30c_by(&doc, "o2"));
    assert!(option_is_selected(&o1) && !option_is_selected(&o0));
    assert!(set_selected_indices(&s, &[2]));
    assert!(!set_selected_indices(&s, &[2]), "unchanged selection reports false");
    assert!(option_is_selected(&o2) && !option_is_selected(&o1));
    assert_eq!(selected_index(&s), 2);
    assert_eq!(q30e_set(&f, None), "s=c");
    assert!(set_option_selected(&o0, true));
    assert_eq!(selected_indices(&s), vec![0]);
    set_option_selected(&o0, false);
    assert_eq!(selected_indices(&s), vec![0], "deselecting the only selection resets to the first enabled option");
    set_selected_indices(&s, &[]);
    assert_eq!(selected_index(&s), -1);
    assert_eq!(q30e_set(&f, None), "", "a select with no selection submits nothing");
    let m = q30c_by(&doc, "m");
    set_option_selected(&q30c_by(&doc, "m0"), true);
    set_option_selected(&q30c_by(&doc, "m2"), true);
    assert_eq!(selected_indices(&m), vec![0, 2]);
    assert_eq!(q30e_set(&f, None), "m=1&m=3");
    set_option_selected(&q30c_by(&doc, "m0"), false);
    assert_eq!(selected_indices(&m), vec![2]);
    assert!(reset_form_state(&f));
    assert_eq!(selected_indices(&s), vec![1]);
    assert!(selected_indices(&m).is_empty());
    assert_eq!(q30e_set(&f, None), "s=b");
    clear_select_state();
}

#[test]
fn q30g_removed_selected_option_falls_back_to_default() {
    let doc = q30g_doc("<select id=s><option id=a>A<option id=b>B</select>");
    let s = q30c_by(&doc, "s");
    set_selected_indices(&s, &[1]);
    let b = q30c_by(&doc, "b");
    s.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, &b));
    assert_eq!(selected_indices(&s), vec![0]);
    clear_select_state();
}

#[test]
fn q30g_data_set_uses_live_selection() {
    let doc = q30g_doc(
        r#"<form id=f><select name=s><option value=1>a<option value=2 selected>b</select>
        <select name=d><option value=x>x<option value=y disabled>y</select></form>"#,
    );
    let f = q30c_by(&doc, "f");
    assert_eq!(q30e_set(&f, None), "s=2&d=x");
    let mut sels = Vec::new();
    q30g_selects(&doc, &mut sels);
    set_selected_indices(&sels[0], &[0]);
    set_selected_indices(&sels[1], &[1]);
    assert_eq!(q30e_set(&f, None), "s=1", "a disabled option is never submitted");
    clear_select_state();
}

#[test]
fn q30g_javascript_round_trips() {
    let doc = q30g_doc(
        r#"<form id=f><select id=s name=s><option value=a>A<option value=b selected>B<option value=c>C</select>
        <select id=m multiple><optgroup label=G><option>1<option selected>2</optgroup><option>3</select></form>"#,
    );
    let js = run_page_scripts(
        &doc,
        &[src(
            "var s=document.getElementById('s'),m=document.getElementById('m'),o=[];\
             o.push(s.value,s.selectedIndex,s.length,s.options.length,s.selectedOptions.length,s.type,m.type);\
             s.value='c'; o.push(s.value,s.selectedIndex,s.options[2].selected,s.options[1].selected,s.options[1].defaultSelected);\
             s.selectedIndex=0; o.push(s.value);\
             s.value='nope'; o.push(s.selectedIndex,'['+s.value+']');\
             s.options[1].selected=true; o.push(s.value,s.selectedIndex);\
             var n=document.createElement('option'); n.value='z'; n.text='Z'; s.add(n); o.push(s.length); n.selected=true; o.push(s.value);\
             s.remove(3); o.push(s.length);\
             s.length=1; o.push(s.length);\
             o.push(m.selectedOptions.length, m.options[1].selected, m.options[0].selected);\
             m.options[0].selected=true; o.push(m.selectedOptions.length);\
             m.selectedIndex=2; o.push(m.selectedOptions.length, m.selectedIndex);\
             s.value='a'; document.getElementById('f').reset(); o.push(s.value, m.selectedIndex);\
             window.out=o.join('|');",
        )],
        None,
    )
    .expect("js");
    let got: String = js.eval_value("t", "window.out").expect("out");
    assert_eq!(got, "b|1|3|3|1|select-one|select-multiple|c|2|true|false|true|a|-1|[]|b|1|4|z|3|1|1|true|false|2|1|2|a|1");
    clear_select_state();
}

#[test]
fn q30g_step_typeahead_and_click_selection_are_pure() {
    let dis = [true, false, false, true, false];
    assert_eq!(step_index(&dis, None, Step::Next), Some(1));
    assert_eq!(step_index(&dis, None, Step::Prev), Some(4));
    assert_eq!(step_index(&dis, Some(1), Step::Next), Some(2));
    assert_eq!(step_index(&dis, Some(2), Step::Next), Some(4), "skips the disabled option");
    assert_eq!(step_index(&dis, Some(4), Step::Next), Some(4), "stays at the end");
    assert_eq!(step_index(&dis, Some(1), Step::Prev), Some(1));
    assert_eq!(step_index(&dis, Some(2), Step::First), Some(1));
    assert_eq!(step_index(&dis, Some(1), Step::Last), Some(4));
    assert_eq!(step_index(&dis, Some(1), Step::PageNext(10)), Some(4));
    assert_eq!(step_index(&dis, Some(4), Step::PagePrev(2)), Some(1));
    let opts: Vec<(String, bool)> =
        [("Apple", false), ("Avocado", false), ("Banana", false), ("Blue", true), ("Berry", false)].iter().map(|(a, b)| (a.to_string(), *b)).collect();
    assert_eq!(typeahead_find(&opts, None, "b"), Some(2));
    assert_eq!(typeahead_find(&opts, Some(2), "b"), Some(4), "repeating a letter cycles, skipping disabled");
    assert_eq!(typeahead_find(&opts, Some(4), "b"), Some(2), "and wraps");
    assert_eq!(typeahead_find(&opts, Some(0), "av"), Some(1));
    assert_eq!(typeahead_find(&opts, Some(0), "AP"), Some(0), "case-insensitive, search starts at the current option");
    assert_eq!(typeahead_find(&opts, None, "zz"), None);
    let mut ta = TypeAhead::default();
    assert_eq!(ta.push('a', 0.0), "a");
    assert_eq!(ta.push('v', 0.5), "av");
    assert_eq!(ta.push('x', 2.0), "x", "a pause starts a new word");
    let d = [false, false, true, false, false];
    assert_eq!(click_selection(false, &[0], None, 3, true, true, &d), vec![3]);
    assert_eq!(click_selection(true, &[1], None, 3, false, false, &d), vec![3]);
    assert_eq!(click_selection(true, &[1], None, 3, true, false, &d), vec![1, 3]);
    assert_eq!(click_selection(true, &[1, 3], None, 3, true, false, &d), vec![1]);
    assert_eq!(click_selection(true, &[1], Some(1), 4, false, true, &d), vec![1, 3, 4], "range skips disabled");
}

fn q30g_rows(n: usize) -> Vec<SelRow> {
    (0..n).map(|i| SelRow { opt: Some(i), label: format!("Option {i}"), disabled: i == 2, grouped: false }).collect()
}

#[test]
fn q30g_popup_open_close_state_machine() {
    assert!(popup_next_open(false, PopupEvent::Activate));
    for e in [PopupEvent::Escape, PopupEvent::ClickAway, PopupEvent::Blur, PopupEvent::Tab, PopupEvent::Choose, PopupEvent::Activate] {
        assert!(!popup_next_open(true, e), "{e:?} closes an open popup");
    }
    for e in [PopupEvent::Escape, PopupEvent::ClickAway, PopupEvent::Blur, PopupEvent::Tab, PopupEvent::Choose] {
        assert!(!popup_next_open(false, e), "{e:?} never opens a closed popup");
    }
}

#[test]
fn q30g_popup_keys_move_choose_and_close() {
    let rows = q30g_rows(30);
    let mut st = PopupState { node: 1, highlight: Some(0), scroll: 0, ahead: TypeAhead::default() };
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Down, 0.0), PopupAct::Stay);
    assert_eq!(st.highlight, Some(1));
    popup_key(&mut st, &rows, 10, PopupKey::Down, 0.0);
    assert_eq!(st.highlight, Some(3), "disabled option 2 is skipped");
    popup_key(&mut st, &rows, 10, PopupKey::End, 0.0);
    assert_eq!((st.highlight, st.scroll), (Some(29), 20), "End scrolls the last row into view");
    popup_key(&mut st, &rows, 10, PopupKey::Home, 0.0);
    assert_eq!((st.highlight, st.scroll), (Some(0), 0));
    popup_key(&mut st, &rows, 10, PopupKey::PageDown, 0.0);
    assert_eq!(st.highlight, Some(10), "a page = visible-1 enabled options (option 2 is disabled)");
    popup_key(&mut st, &rows, 10, PopupKey::PageUp, 0.0);
    assert_eq!(st.highlight, Some(0));
    for _ in 0..15 {
        popup_key(&mut st, &rows, 10, PopupKey::Down, 0.0);
    }
    let h = st.highlight.unwrap();
    assert!(h >= st.scroll && h < st.scroll + 10, "highlight {h} stays inside the window starting at {}", st.scroll);
    popup_key(&mut st, &rows, 10, PopupKey::Home, 6.0);
    for c in "option 1".chars() {
        popup_key(&mut st, &rows, 10, PopupKey::Char(c), 7.0);
    }
    assert_eq!(st.highlight, Some(1), "type-ahead of the typed word");
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Enter, 9.0), PopupAct::Choose(1));
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Space, 9.0), PopupAct::Choose(1));
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Escape, 9.0), PopupAct::Close);
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Tab, 9.0), PopupAct::CloseAndPass);
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Other, 9.0), PopupAct::Stay);
    st.highlight = Some(2);
    assert_eq!(popup_key(&mut st, &rows, 10, PopupKey::Enter, 9.0), PopupAct::Close, "a disabled option cannot be chosen");
}

#[test]
fn q30g_popup_geometry_below_flip_shrink_scroll_and_hit() {
    let win = (800.0, 600.0);
    let g = popup_geometry((100.0, 100.0, 120.0, 20.0), 5, 18.0, 60.0, win);
    assert!(!g.above && g.y == 120.0 && g.x == 100.0 && g.w == 120.0 && g.visible == 5 && g.h == 5.0 * 18.0 + 2.0);
    let g = popup_geometry((700.0, 100.0, 80.0, 20.0), 5, 18.0, 200.0, win);
    assert_eq!((g.w, g.x), (200.0, 600.0));
    let g = popup_geometry((100.0, 100.0, 120.0, 20.0), 30, 18.0, 60.0, win);
    assert_eq!(g.visible, POPUP_MAX_ROWS);
    let g = popup_geometry((100.0, 570.0, 120.0, 20.0), 8, 18.0, 60.0, win);
    assert!(g.above && g.visible == 8 && g.y + g.h == 570.0, "{g:?}");
    let g = popup_geometry((100.0, 90.0, 120.0, 20.0), 10, 18.0, 60.0, (800.0, 200.0));
    assert!(g.visible < 10 && g.y + g.h <= 200.0 && g.y >= 0.0, "{g:?}");
    let g = popup_geometry((100.0, 100.0, 120.0, 20.0), 30, 18.0, 60.0, win);
    assert_eq!(popup_hit(&g, 0, 30, (110.0, g.y + 1.0 + 5.0)), PopupHit::Row(0));
    assert_eq!(popup_hit(&g, 7, 30, (110.0, g.y + 1.0 + 18.0 * 2.0 + 1.0)), PopupHit::Row(9));
    assert_eq!(popup_hit(&g, 0, 30, (110.0, g.y + 0.5)), PopupHit::Border);
    assert_eq!(popup_hit(&g, 0, 30, (10.0, 10.0)), PopupHit::Outside);
    assert_eq!(popup_hit(&g, 0, 30, (110.0, g.y + g.h + 1.0)), PopupHit::Outside);
}

fn q30g_app(html: &str, script: &str) -> Option<(App, Handle)> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = q30g_doc(html);
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(&doc, &[src(script)], None);
    Some((app, doc))
}

#[test]
fn q30g_keyboard_on_a_closed_select_changes_it_and_fires_events() {
    let Some((mut app, doc)) = q30g_app(
        "<select id=s><option>Apple<option>Avocado<option disabled>Berry<option>Banana</select>",
        "window.L=[]; var s=document.getElementById('s'); ['input','change'].forEach(function(k){s.addEventListener(k,function(e){L.push(k+':'+s.value+':'+e.bubbles)})});",
    ) else {
        return;
    };
    let s = q30c_by(&doc, "s");
    let log = |app: &App| -> String { app.js.as_ref().unwrap().eval_value("t", "L.join('|')").expect("log") };
    app.set_focus_element(Some(&s), true);
    assert!(app.focused_select().is_some());
    assert!(app.select_control_key(&s, &Key::Named(NamedKey::ArrowDown)));
    assert_eq!(selected_indices(&s), vec![1]);
    assert_eq!(log(&app), "input:Avocado:true|change:Avocado:true");
    assert!(app.select_control_key(&s, &Key::Named(NamedKey::ArrowDown)));
    assert_eq!(selected_indices(&s), vec![3], "the disabled option is skipped");
    assert!(app.select_control_key(&s, &Key::Named(NamedKey::Home)));
    assert_eq!(selected_indices(&s), vec![0]);
    assert!(app.select_control_key(&s, &Key::Character("b".into())));
    assert_eq!(selected_indices(&s), vec![3], "type-ahead jumps to the first enabled match");
    let n = log(&app);
    assert!(app.select_control_key(&s, &Key::Named(NamedKey::End)));
    assert_eq!(log(&app), n, "no change, no event");
    assert!(!app.select_control_key(&s, &Key::Named(NamedKey::F5)));
    assert!(app.select_control_key(&s, &Key::Named(NamedKey::Escape)));
    assert!(app.interaction.focus.is_none());
    clear_select_state();
}

#[test]
fn q30g_listbox_arrows_and_space_toggle() {
    let Some((mut app, doc)) = q30g_app("<select id=m multiple size=4><option>1<option>2<option>3<option>4</select>", "") else { return };
    let m = q30c_by(&doc, "m");
    app.set_focus_element(Some(&m), true);
    assert!(app.select_control_key(&m, &Key::Named(NamedKey::ArrowDown)));
    assert_eq!(selected_indices(&m), vec![0]);
    assert!(app.select_control_key(&m, &Key::Named(NamedKey::ArrowDown)));
    assert_eq!(selected_indices(&m), vec![1], "arrows move a single selection");
    assert!(app.select_control_key(&m, &Key::Named(NamedKey::Space)));
    assert_eq!(selected_indices(&m), Vec::<usize>::new(), "space toggles the cursor option of a multiple");
    clear_select_state();
}
