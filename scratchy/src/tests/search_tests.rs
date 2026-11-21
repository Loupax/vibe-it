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

fn m_at(y: f32) -> SearchMatch {
    SearchMatch { line: 0, start: 0, end: 1, rects: vec![(0.0, y, 10.0, 20.0)] }
}

#[test]
fn qsearch1_nearest_and_first_below() {
    let ms = [m_at(10.0), m_at(100.0), m_at(300.0)];
    assert_eq!(nearest_match(&ms, (0.0, 120.0)), Some(1));
    assert_eq!(nearest_match(&ms, (0.0, 5000.0)), Some(2));
    assert_eq!(nearest_match(&[], (0.0, 0.0)), None);
    assert_eq!(first_at_or_below(&ms, 100.0), Some(1), "at the top counts");
    assert_eq!(first_at_or_below(&ms, 101.0), Some(2));
    assert_eq!(first_at_or_below(&ms, 301.0), None, "none below: the caller wraps");
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
