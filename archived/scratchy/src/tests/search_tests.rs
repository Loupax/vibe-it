// QSEARCH1: find + highlight (KEYS.md S1-S3, S5, T7, T8, T22-T25, T31).
#[allow(unused_imports)]
use super::*;
use crate::search::*;
use std::time::Instant;

fn laid_with_cache(html: &str) -> (Laid, ShapeCache) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    (laid, cache)
}

fn find(html: &str, q: &str) -> Vec<SearchMatch> {
    let (laid, cache) = laid_with_cache(html);
    find_in_laid(&laid, &cache, q)
}

#[test]
fn qsearch1_smartcase_both_ways() {
    let lines = [SearchLine::from_text("Foo foo FOO", 0.0, 0.0, 10.0, 20.0)];
    assert_eq!(find_matches(&lines, "foo").len(), 3, "lower-case query ignores case");
    assert_eq!(find_matches(&lines, "Foo").len(), 1, "a capital makes it case-sensitive");
    assert_eq!(find_matches(&lines, "FOO").len(), 1);
    assert_eq!(find_matches(&lines, "fOo").len(), 0);
    assert!(find_matches(&lines, "").is_empty());
}

#[test]
fn qsearch1_pure_finder_orders_and_never_crosses_lines() {
    let lines = [SearchLine::from_text("ab ab", 0.0, 0.0, 10.0, 20.0), SearchLine::from_text("ba", 0.0, 30.0, 10.0, 20.0)];
    let m = find_matches(&lines, "ab");
    assert_eq!(m.iter().map(|m| (m.line, m.start)).collect::<Vec<_>>(), vec![(0, 0), (0, 3)]);
    assert_eq!(m[1].rects, vec![(30.0, 0.0, 20.0, 20.0)]);
    assert!(find_matches(&lines, "abba").is_empty(), "no match across lines");
}

#[test]
fn qsearch1_matches_across_inline_elements() {
    let m = find("<html><body><p>foo<b>bar</b> baz</p></body></html>", "foobar");
    assert_eq!(m.len(), 1);
    assert!(!m[0].rects.is_empty());
}

#[test]
fn qsearch1_no_match_across_blocks() {
    let html = "<html><body><p>foo</p><p>bar</p></body></html>";
    assert!(find(html, "foobar").is_empty());
    assert!(find(html, "foo bar").is_empty());
    assert_eq!(find(html, "bar").len(), 1);
}

#[test]
fn qsearch1_display_none_text_never_matches() {
    let html = "<html><body><p>shown</p><p style='display:none'>secret</p><p>a<span style='display:none'>secret</span>b</p></body></html>";
    assert!(find(html, "secret").is_empty());
    assert_eq!(find(html, "shown").len(), 1);
    assert_eq!(find(html, "ab").len(), 1, "the hidden span takes no text");
}

#[test]
fn qsearch1_visibility_hidden_text_is_masked() {
    let html = "<html><body><p>a<span style='visibility:hidden'>secret</span>b</p></body></html>";
    assert!(find(html, "secret").is_empty());
    assert!(find(html, "ab").is_empty());
}

#[test]
fn qsearch1_wrapped_text_still_matches_with_real_rects() {
    let html = "<html><body style='margin:0'><p style='width:60px;margin:0'>aaaa bbbb cccc dddd</p></body></html>";
    let one = find(html, "bbbb");
    assert_eq!(one.len(), 1);
    assert!(one[0].rects.iter().all(|r| r.2 > 0.0 && r.3 > 0.0));
}

fn first_below(ms: &[SearchMatch], top: f32) -> Option<usize> {
    first_at_or_below_in(ms, top, &std::collections::HashMap::new())
}

fn m_at(y: f32) -> SearchMatch {
    SearchMatch { line: 0, start: 0, end: 1, rects: vec![(0.0, y, 10.0, 20.0)], regions: Vec::new(), nodes: Vec::new() }
}

#[test]
fn qsearch1_nearest_and_first_below() {
    let ms = [m_at(10.0), m_at(100.0), m_at(300.0)];
    assert_eq!(nearest_match(&ms, (0.0, 120.0)), Some(1));
    assert_eq!(nearest_match(&ms, (0.0, 5000.0)), Some(2));
    assert_eq!(nearest_match(&[], (0.0, 0.0)), None);
    assert_eq!(first_below(&ms, 100.0), Some(1), "at the top counts");
    assert_eq!(first_below(&ms, 101.0), Some(2));
    assert_eq!(first_below(&ms, 301.0), None, "none below: the caller wraps");
}

#[test]
fn qsearch1_relayout_keeps_the_nearest_current_match() {
    let (a, ca) = laid_with_cache("<html><body style='margin:0'><p>x</p><p>x</p><p>x</p></body></html>");
    let mut st = SearchState { query: "x".into(), highlight: true, ..Default::default() };
    st.relayout(&a, &ca);
    assert_eq!(st.matches.len(), 3);
    assert_eq!(st.current, Some(0));
    st.current = Some(2);
    let third_y = st.matches[2].origin().1;
    // A spacer above shifts every match down; the current one follows.
    let (b, cb) = laid_with_cache("<html><body style='margin:0'><div style='height:5px'></div><p>x</p><p>x</p><p>x</p></body></html>");
    st.relayout(&b, &cb);
    assert_eq!(st.current, Some(2));
    assert!(st.matches[2].origin().1 > third_y);
    // Cleared highlights: relayout does nothing; the query stays.
    st.clear_highlights();
    st.relayout(&b, &cb);
    assert!(st.matches.is_empty() && st.query == "x");
}

// ---- App-level: keys ----

fn search_app() -> Option<App> {
    let event_loop = test_event_loop().ok()?;
    let doc = parse_html("<html><body><p>one two</p></body></html>");
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    let (laid, cache) = laid_with_cache("<html><body style='margin:0'><p>Alpha beta</p><p>alpha</p></body></html>");
    app.cached = Some(Rc::new(laid));
    app.shape_cache = cache;
    app.key_mode = KeyMode::Normal;
    Some(app)
}

fn press(app: &mut App, key: Key) {
    app.route_key(&key, None, true, false, Instant::now());
    app.route_key(&key, None, false, false, Instant::now());
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        app.search_line_key(&Key::Character(c.to_string().as_str().into()));
    }
}

#[test]
fn qsearch1_t8_t22_t23_slash_types_and_enter_commits() {
    let Some(mut app) = search_app() else { return };
    press(&mut app, Key::Character("/".into()));
    assert_eq!(app.vim_mode, VimMode::Search);
    type_str(&mut app, "alpha");
    assert_eq!(app.command_line.text, "alpha");
    app.search_line_key(&Key::Named(NamedKey::Enter));
    assert_eq!(app.vim_mode, VimMode::Normal);
    assert_eq!(app.search.query, "alpha");
    assert!(app.search.highlight);
    assert_eq!(app.search.matches.len(), 2, "smartcase: both Alpha and alpha");
    assert_eq!(app.search.current, Some(0));
    assert_eq!(app.status_message, "[1/2]");
}

#[test]
fn qsearch1_t23_first_match_at_or_below_the_top_wraps() {
    let Some(mut app) = search_app() else { return };
    app.scroll_offset = 1000.0;
    app.commit_search("alpha");
    assert_eq!(app.search.current, Some(0));
    assert!(app.status_message.starts_with("search hit BOTTOM"), "{}", app.status_message);
    app.scroll_offset = app.search.matches[1].origin().1;
    app.commit_search("alpha");
    assert_eq!(app.search.current, Some(1), "the match at the top of the screen counts");
}

#[test]
fn qsearch1_t16_no_match_message() {
    let Some(mut app) = search_app() else { return };
    app.commit_search("zzz");
    assert_eq!(app.status_message, "Pattern not found: zzz");
    assert!(app.search.matches.is_empty());
}

#[test]
fn qsearch1_t24_t25_empty_enter_and_esc_cancel_and_change_nothing() {
    let Some(mut app) = search_app() else { return };
    app.commit_search("alpha");
    let before = app.search.matches.len();
    press(&mut app, Key::Character("/".into()));
    type_str(&mut app, "beta");
    app.search_line_key(&Key::Named(NamedKey::Escape));
    assert_eq!(app.vim_mode, VimMode::Normal);
    assert_eq!(app.search.query, "alpha");
    assert_eq!(app.search.matches.len(), before);
    press(&mut app, Key::Character("/".into()));
    app.search_line_key(&Key::Named(NamedKey::Enter));
    assert_eq!(app.vim_mode, VimMode::Normal);
    assert_eq!(app.search.query, "alpha", "empty Enter is a cancel");
    assert!(app.search.highlight);
}

#[test]
fn qsearch1_t7_esc_in_normal_clears_highlights_keeps_query_and_mode() {
    let Some(mut app) = search_app() else { return };
    app.commit_search("alpha");
    press(&mut app, Key::Named(NamedKey::Escape));
    assert!(!app.search.highlight && app.search.matches.is_empty());
    assert_eq!(app.search.query, "alpha");
    assert_eq!(app.key_mode, KeyMode::Normal);
}

#[test]
fn qsearch1_t31_navigation_clears_highlights_keeps_query() {
    let Some(mut app) = search_app() else { return };
    app.commit_search("alpha");
    app.replace_document(parse_html("<html><body>new</body></html>"), String::new());
    assert!(!app.search.highlight && app.search.matches.is_empty());
    assert_eq!(app.search.query, "alpha");
}

#[test]
fn qsearch1_search_line_reuses_the_paste_chord() {
    let Some(mut app) = search_app() else { return };
    press(&mut app, Key::Character("/".into()));
    // Ctrl-v is the shared paste chord, never a literal `v` (the clipboard is not asserted).
    app.modifiers = ModifiersState::CONTROL;
    app.search_line_key(&Key::Character("v".into()));
    assert!(!app.command_line.text.starts_with('v'), "{:?}", app.command_line.text);
    app.modifiers = ModifiersState::empty();
    app.command_line.clear();
    app.search_line_key(&Key::Character("v".into()));
    assert_eq!(app.command_line.text, "v");
    assert!(is_paste_chord(&Key::Character("v".into()), &ModifiersState::CONTROL));
}

// ---- QSEARCH2: n / N, bringing the screen to the match ----

fn nav_app(html: &str) -> Option<App> {
    let event_loop = test_event_loop().ok()?;
    let doc = parse_html("<html><body><p>x</p></body></html>");
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    let (laid, cache) = laid_with_cache(html);
    app.cached = Some(Rc::new(laid));
    app.shape_cache = cache;
    app.key_mode = KeyMode::Normal;
    app.surface_size = (800, 600 + crate::config::STATUS_BAR_H);
    Some(app)
}

const TALL: &str = "<html><body style='margin:0'><p style='margin:0'>needle</p><div style='height:1000px'></div>\
<p style='margin:0'>needle</p><div style='height:2000px'></div><p style='margin:0'>needle</p><div style='height:2000px'></div></body></html>";

#[test]
fn qsearch2_n_and_big_n_wrap_both_ways_with_messages() {
    let Some(mut app) = nav_app(TALL) else { return };
    app.commit_search("needle");
    assert_eq!(app.status_message, "[1/3]");
    press(&mut app, Key::Character("n".into()));
    assert_eq!((app.search.current, app.status_message.as_str()), (Some(1), "[2/3]"));
    press(&mut app, Key::Character("n".into()));
    assert_eq!(app.status_message, "[3/3]");
    press(&mut app, Key::Character("n".into()));
    assert_eq!(app.search.current, Some(0));
    assert_eq!(app.status_message, "search hit BOTTOM, continuing at TOP [1/3]");
    press(&mut app, Key::Character("N".into()));
    assert_eq!(app.search.current, Some(2));
    assert_eq!(app.status_message, "search hit TOP, continuing at BOTTOM [3/3]");
    press(&mut app, Key::Character("N".into()));
    assert_eq!((app.search.current, app.status_message.as_str()), (Some(1), "[2/3]"));
}

#[test]
fn qsearch2_t16_t17_no_query_and_no_match() {
    let Some(mut app) = nav_app(TALL) else { return };
    press(&mut app, Key::Character("n".into()));
    assert_eq!(app.status_message, "No previous search");
    app.search.query = "zzz".into();
    press(&mut app, Key::Character("N".into()));
    assert_eq!(app.status_message, "Pattern not found: zzz");
}

#[test]
fn qsearch2_offscreen_match_is_centred_visible_one_does_not_scroll() {
    let Some(mut app) = nav_app(TALL) else { return };
    app.commit_search("needle");
    assert_eq!(app.scroll_offset, 0.0, "the first match is on screen");
    app.search_step(true);
    let m = &app.search.matches[1];
    let (_, y, _, h) = m.bbox();
    assert!((app.scroll_offset - (y + h / 2.0 - 300.0)).abs() < 1.0, "{} vs {y}", app.scroll_offset);
    // The next match is far below: centred too. Going back (it is off screen again) centres it.
    app.search_step(true);
    let (_, y2, _, h2) = app.search.matches[2].bbox();
    assert!((app.scroll_offset - (y2 + h2 / 2.0 - 300.0)).abs() < 1.0);
    // The first match is at the very top of the page: clamped at 0, not negative.
    app.search_step(true);
    assert_eq!(app.scroll_offset, 0.0);
    // Two matches both on screen: n does not scroll.
    let Some(mut app) = nav_app("<html><body style='margin:0'><p>needle</p><p>needle</p><div style='height:3000px'></div></body></html>") else { return };
    app.scroll_offset = 0.0;
    app.commit_search("needle");
    app.search_step(true);
    assert_eq!(app.search.current, Some(1));
    assert_eq!(app.scroll_offset, 0.0);
}

#[test]
fn qsearch2_enter_commit_brings_the_screen_to_the_match() {
    let Some(mut app) = nav_app("<html><body style='margin:0'><div style='height:2000px'></div><p style='margin:0'>needle</p><div style='height:2000px'></div></body></html>") else { return };
    app.commit_search("needle");
    assert!(app.scroll_offset > 1000.0, "{}", app.scroll_offset);
}

#[test]
fn qsearch2_match_in_a_scroll_box_scrolls_the_box_and_highlight_follows() {
    let html = "<html><body style='margin:0'><div style='height:100px;overflow:auto;margin:0'>\
<div style='height:500px'></div><p style='margin:0'>needle</p><div style='height:500px'></div></div></body></html>";
    let Some(mut app) = nav_app(html) else { return };
    app.commit_search("needle");
    let m = app.search.matches[0].clone();
    assert_eq!(m.regions.len(), 1);
    let off = app.scroll_offsets.get(&m.regions[0].id).copied().unwrap_or(0.0);
    assert!(off > 300.0, "the box scrolled: {off}");
    assert_eq!(app.scroll_offset, 0.0, "the page did not move: the box is on screen");
    // The highlight is drawn inside the box (0..100), at the scrolled position.
    let rects = m.visible_rects(&app.scroll_offsets);
    assert_eq!(rects.len(), 1);
    assert!(rects[0].1 >= 0.0 && rects[0].1 + rects[0].3 <= 100.5, "{rects:?}");
    // Scrolled away again: the highlight is clipped out.
    let mut away = app.scroll_offsets.clone();
    away.insert(m.regions[0].id, 0.0);
    assert!(m.visible_rects(&away).is_empty());
}

#[test]
fn qsearch2_nested_boxes_scroll_innermost_first_then_the_page() {
    let html = "<html><body style='margin:0'><div style='height:3000px'></div>\
<div style='height:200px;overflow:auto'><div style='height:300px'></div>\
<div style='height:100px;overflow:auto'><div style='height:400px'></div><p style='margin:0'>needle</p><div style='height:400px'></div></div>\
<div style='height:600px'></div></div></body></html>";
    let Some(mut app) = nav_app(html) else { return };
    app.commit_search("needle");
    let m = app.search.matches[0].clone();
    assert_eq!(m.regions.len(), 2);
    let inner = app.scroll_offsets.get(&m.regions[1].id).copied().unwrap_or(0.0);
    let outer = app.scroll_offsets.get(&m.regions[0].id).copied().unwrap_or(0.0);
    assert!(inner > 0.0 && outer > 0.0, "inner {inner} outer {outer}");
    assert!(app.scroll_offset > 2000.0, "the page followed: {}", app.scroll_offset);
    let top = m.doc_top(&app.scroll_offsets);
    assert!(top >= app.scroll_offset && top <= app.scroll_offset + 600.0, "{top} in view at {}", app.scroll_offset);
    assert!(!m.visible_rects(&app.scroll_offsets).is_empty());
}

#[test]
fn qsearch2_kept_query_runs_on_a_new_page() {
    let Some(mut app) = nav_app(TALL) else { return };
    app.commit_search("needle");
    app.replace_document(parse_html("<html><body>new</body></html>"), String::new());
    assert!(!app.search.highlight && app.search.query == "needle");
    let (laid, cache) = laid_with_cache("<html><body style='margin:0'><p>a needle here</p></body></html>");
    app.cached = Some(Rc::new(laid));
    app.shape_cache = cache;
    press(&mut app, Key::Character("n".into()));
    assert!(app.search.highlight);
    assert_eq!(app.search.matches.len(), 1);
    assert_eq!(app.status_message, "[1/1]");
}

// ---- QSEARCH3: Enter clicks the current match (S7, T18-T20) ----

/// An app whose DOM, layout and JS all come from `body` (so the matches' node ids are the DOM's).
fn click_app(body: &str, script: &str) -> Option<App> {
    let event_loop = test_event_loop().ok()?;
    let doc = parse_html(&format!("<html><body style='margin:0'>{body}</body></html>"));
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    let tree = build_layout_tree(&doc, 1920.0);
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut app.font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    app.cached = Some(Rc::new(laid));
    app.shape_cache = cache;
    app.key_mode = KeyMode::Normal;
    app.surface_size = (800, 600 + crate::config::STATUS_BAR_H);
    Some(app)
}

fn click_log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value::<String>("t", "window.L.join('|')").unwrap_or_default()
}

#[test]
fn qsearch3_enter_in_a_plain_link_navigates() {
    let Some(mut app) = click_app("<p><a href='/next'>go needle here</a></p>", "window.L=[];") else { return };
    app.commit_search("needle");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/next"));
    assert_eq!(app.nav_generation, 1);
}

#[test]
fn qsearch3_prevent_default_handler_runs_and_nothing_navigates() {
    let script = "window.L=[]; document.getElementById('a').addEventListener('click',function(e){L.push('click:'+e.target.id); e.preventDefault();});";
    let Some(mut app) = click_app("<p><a id=a href='/next'>go <span id=s>needle</span> here</a></p>", script) else { return };
    app.commit_search("needle");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(click_log(&app), "click:s", "the click hit the inner element and bubbled to the link");
    assert_eq!(app.nav_generation, 0);
    assert!(app.history.is_empty());
}

#[test]
fn qsearch3_match_in_a_button_fires_its_click() {
    let script = "window.L=[]; document.getElementById('b').addEventListener('click',function(){L.push('b')});";
    let Some(mut app) = click_app("<p><button id=b>Press needle</button></p>", script) else { return };
    app.commit_search("needle");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(click_log(&app), "b");
}

#[test]
fn qsearch3_match_in_plain_text_clicks_that_element() {
    let script = "window.L=[]; document.getElementById('p').addEventListener('click',function(e){L.push('p:'+e.target.id)});";
    let Some(mut app) = click_app("<p id=p>a needle b</p>", script) else { return };
    app.commit_search("needle");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(click_log(&app), "p:p");
    assert_eq!(app.nav_generation, 0);
}

#[test]
fn qsearch3_match_spanning_two_links_warns_and_clicks_nothing() {
    let script = "window.L=[]; document.addEventListener('click',function(){L.push('click')});";
    let Some(mut app) = click_app("<p><a href='/one'>foo</a><a href='/two'>bar</a></p>", script) else { return };
    app.commit_search("foobar");
    assert_eq!(app.search.current, Some(0));
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(app.status_message, "Match spans 2 links");
    assert_eq!(click_log(&app), "");
    assert_eq!(app.nav_generation, 0);
}

#[test]
fn qsearch3_match_across_one_link_and_plain_text_clicks_inside_the_link() {
    let script = "window.L=[]; document.getElementById('a').addEventListener('click',function(e){L.push('a:'+e.target.id); e.preventDefault()});";
    let Some(mut app) = click_app("<p>foo<a id=a href='/one'><b id=b>bar</b></a></p>", script) else { return };
    app.commit_search("foobar");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!(click_log(&app), "a:b");
}

#[test]
fn qsearch3_enter_without_a_current_match_does_nothing() {
    let script = "window.L=[]; document.addEventListener('click',function(){L.push('click')});";
    let Some(mut app) = click_app("<p><a href='/next'>needle</a></p>", script) else { return };
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!((click_log(&app).as_str(), app.nav_generation), ("", 0));
    app.commit_search("zzz");
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!((click_log(&app).as_str(), app.nav_generation), ("", 0));
    app.commit_search("needle");
    app.search.clear_highlights();
    press(&mut app, Key::Named(NamedKey::Enter));
    assert_eq!((click_log(&app).as_str(), app.nav_generation), ("", 0));
}
