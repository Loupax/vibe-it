//! C-PICK: the picker (KEYS.md "Picker", T58-T68). Each test names its row; the matcher, ranking,
//! scroll and scale tests name what they check.

use std::path::PathBuf;
use std::time::Instant;

use smithay::input::keyboard::Keysym;
use smithay::utils::Rectangle;

use crate::config::{PICKER_MAX_ROWS, PICKER_ROW_H, STATUS_BAR_H};
use crate::keys::*;
use crate::layout::Layout;
use crate::picker::*;
use crate::protocol::Command;

/// A history file in a fresh directory (removed by `Dir`'s drop).
struct Dir(PathBuf);

impl Dir {
    fn with_history(tag: &str, text: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("web-c-pick-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("history"), text).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Three sites: example.org (3 visits), github.com (2), news.test (1, no title).
const HISTORY: &str = "\
1000\thttps://www.example.org/\tExample Domain
1001\thttps://github.com/rust-lang/rust\tThe Rust repo
1002\thttps://www.example.org/docs/intro\tIntro to docs
1003\thttps://github.com/rust-lang/rust\tThe Rust repo
1004\thttp://news.test/today\t
1005\thttps://www.example.org/\tExample Domain (new)
";

fn keys_with(dir: &Dir) -> Keys {
    Keys::new(Some(dir.0.clone()), "file:///help.html".into(), None)
}

fn press(k: &mut Keys, key: KeyInput) -> bool {
    let page = k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
    page
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn run(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    type_str(k, line);
    // The `:` line's ghost could add to a typed prefix; Delete hides it (T52).
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

fn selected_url(k: &Keys) -> String {
    match &k.picker.as_ref().expect("a picker").selected_row().expect("a selection").action {
        PickAction::Open(u) => u.clone(),
        PickAction::Buffer(n) => format!("buffer {n}"),
        PickAction::Popup(n) => format!("popup {n}"),
        PickAction::Notice(n) => format!("notice {n}"),
    }
}

fn opens(k: &mut Keys) -> Vec<String> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(Command::Open { url }) => Some(url),
            _ => None,
        })
        .collect()
}

fn row(label: &str, fields: &[&str]) -> Row {
    Row {
        label: label.into(),
        detail: String::new(),
        fields: fields.iter().map(|f| f.to_string()).collect(),
        action: PickAction::Open(format!("https://{label}/")),
    }
}

// ---------- T58: opening ----------

#[test]
fn keys_t58_history_search_opens_the_picker_in_ranking_order() {
    let dir = Dir::with_history("t58", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert_eq!(k.key_mode, KeyMode::Normal);
    let p = k.picker.as_ref().unwrap();
    let urls: Vec<String> = p
        .matches()
        .map(|r| match &r.action {
            PickAction::Open(u) => u.clone(),
            PickAction::Buffer(n) => format!("buffer {n}"),
            PickAction::Popup(n) => format!("popup {n}"),
            PickAction::Notice(n) => format!("notice {n}"),
        })
        .collect();
    // C-HISTREAL: only URLs with a line in the file, by their own visits, ties newest first; no
    // made-up site roots (github.com/ and news.test/ were never visited).
    assert_eq!(
        urls,
        [
            "https://www.example.org/",
            "https://github.com/rust-lang/rust",
            "http://news.test/today",
            "https://www.example.org/docs/intro",
        ]
    );
    let labels: Vec<&str> = p.matches().map(|r| r.label.as_str()).collect();
    // The newest title of exactly that URL; no title: the URL.
    assert_eq!(labels[0], "Example Domain (new)");
    assert_eq!(labels[1], "The Rust repo");
    assert_eq!(labels[2], "http://news.test/today");
    assert_eq!(p.matches().nth(2).unwrap().detail, "", "the URL is the label, no detail");
    assert_eq!(p.matches().next().unwrap().detail, "https://www.example.org/");
    assert_eq!(selected_url(&k), "https://www.example.org/", "the first row is selected");
    let v = k.bar_view();
    assert_eq!(v.line, Some(('>', String::new(), 0, None)), "the bar shows the (empty) filter");
}

#[test]
fn c_histreal_a_visited_root_is_one_row_with_its_own_count_and_order_is_count_then_recency() {
    // The root was visited once; the site has three more page visits. Own count 1, not 4.
    let text = "\
1000\thttps://s.test/\tHome\n\
1001\thttps://s.test/a\tA\n\
1002\thttps://s.test/a\tA\n\
1003\thttps://s.test/b\tB\n\
1004\thttps://t.test/c\tC\n";
    let urls: Vec<String> = history_rows(text)
        .into_iter()
        .map(|r| match r.action {
            PickAction::Open(u) => u,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        urls,
        ["https://s.test/a", "https://t.test/c", "https://s.test/b", "https://s.test/"]
    );
    // No line for any root: no root row; every row's URL is a line of the file.
    let only_pages = "1\thttps://h.test/x\tX\n2\thttps://h.test/y\tY\n";
    let rows = history_rows(only_pages);
    assert_eq!(rows.len(), 2);
    for r in rows {
        let PickAction::Open(u) = r.action else { unreachable!() };
        assert!(only_pages.contains(&format!("\t{u}\t")), "{u}");
    }
}

#[test]
fn keys_t58_an_argument_is_the_filter() {
    let dir = Dir::with_history("t58arg", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch rust");
    assert_eq!(k.line.text, "rust");
    assert_eq!(selected_url(&k), "https://github.com/rust-lang/rust");
}

#[test]
fn keys_t58_empty_history_says_so_and_stays_normal() {
    let dir = Dir::with_history("t58empty", "");
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.picker.is_none());
    assert_eq!(k.message, "web: the history is empty");
}

#[test]
fn keys_t58_history_search_is_a_command_name_for_the_ghost() {
    assert!(COMMAND_NAMES.contains(&"historySearch"));
}

// ---------- T59: typing filters (fuzzy, best first) ----------

#[test]
fn keys_t59_typing_filters_and_selects_the_best() {
    let dir = Dir::with_history("t59", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    press(&mut k, KeyInput::sym(Keysym::Down));
    type_str(&mut k, "intro");
    let p = k.picker.as_ref().unwrap();
    assert_eq!(p.match_count(), 1);
    assert_eq!(p.selected, 0, "an edit selects the best match again");
    assert_eq!(selected_url(&k), "https://www.example.org/docs/intro");
    // Matched on the title too.
    press(&mut k, KeyInput::ch('u').ctrl());
    type_str(&mut k, "Rust repo");
    assert_eq!(selected_url(&k), "https://github.com/rust-lang/rust");
    // Backspace re-filters.
    for _ in 0.."Rust repo".len() {
        press(&mut k, KeyInput::sym(Keysym::BackSpace));
    }
    assert_eq!(k.picker.as_ref().unwrap().match_count(), 4);
    assert_eq!(k.bar_view().line, Some(('>', String::new(), 0, None)));
}

#[test]
fn keys_t59_the_page_gets_no_keys() {
    let dir = Dir::with_history("t59page", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    for key in [
        KeyInput::ch('j'),
        KeyInput::sym(Keysym::Down),
        KeyInput::ch('i'),
        KeyInput::sym(Keysym::space),
    ] {
        assert!(!press(&mut k, key), "{key:?} reached the page");
    }
    assert_eq!(k.key_mode, KeyMode::Normal, "`i` is typed, not INPUT");
    assert_eq!(k.line.text, "ji ");
}

#[test]
fn fuzzy_matches_letters_in_order_only() {
    assert!(fuzzy_score("gthb", "github.com").is_some());
    assert!(fuzzy_score("hubgit", "github.com").is_none());
    assert!(fuzzy_score("x", "github.com").is_none());
    assert_eq!(fuzzy_score("", "anything"), Some(0));
}

#[test]
fn fuzzy_is_smartcase() {
    assert!(fuzzy_score("rust", "The Rust repo").is_some());
    assert!(fuzzy_score("Rust", "The Rust repo").is_some());
    assert!(fuzzy_score("Rust", "the rust repo").is_none(), "a capital makes case matter");
}

#[test]
fn fuzzy_prefers_word_starts_and_runs() {
    // A run of the letters beats the same letters spread out.
    assert!(fuzzy_score("doc", "example.org/docs") > fuzzy_score("doc", "d-o-c"));
    // A word start beats the middle of a word.
    assert!(fuzzy_score("rep", "the rust repo") > fuzzy_score("rep", "carpet prep"));
    // The best placement is found, not the first: the early `a` in "xaxxx-ab" is skipped for the run
    // at the word start, so it scores as "-ab" does.
    assert_eq!(fuzzy_score("ab", "xaxxx-ab"), fuzzy_score("ab", "-ab"));
    assert!(fuzzy_score("ab", "xaxxx-ab") > fuzzy_score("ab", "xaxxxxxb"));
}

#[test]
fn ranking_is_best_score_first_ties_in_source_order() {
    let rows = vec![
        row("first", &["xdocs-a"]),
        row("second", &["docs"]),
        row("third", &["xdocs-b"]),
        row("none", &["nothing"]),
    ];
    let order = rank(&rows, "docs");
    // "docs" at the start of a word scores higher than inside one; the two others tie.
    assert_eq!(order, vec![1, 0, 2]);
    assert_eq!(rank(&rows, ""), vec![0, 1, 2, 3], "no filter: the source's order");
}

#[test]
fn ranking_needs_every_term_in_some_field() {
    let rows = vec![row("a", &["github.com/rust", "Rust repo"]), row("b", &["github.com/go", "Go repo"])];
    assert_eq!(rank(&rows, "git Rust"), vec![0], "terms may match different fields");
    assert_eq!(rank(&rows, "git repo"), vec![0, 1]);
    assert!(rank(&rows, "git python").is_empty());
}

// ---------- T60/T61: moving the selection ----------

#[test]
fn keys_t60_ctrl_n_and_down_move_down_and_stop_at_the_last() {
    let dir = Dir::with_history("t60", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    press(&mut k, KeyInput::ch('n').ctrl());
    assert_eq!(selected_url(&k), "https://github.com/rust-lang/rust");
    press(&mut k, KeyInput::sym(Keysym::Down));
    assert_eq!(selected_url(&k), "http://news.test/today");
    for _ in 0..10 {
        press(&mut k, KeyInput::sym(Keysym::Down));
    }
    assert_eq!(k.picker.as_ref().unwrap().selected, 3, "no wrap: stays on the last row");
    assert_eq!(k.line.text, "", "Ctrl-n types nothing");
}

#[test]
fn keys_t61_ctrl_p_and_up_move_up_and_stop_at_the_first() {
    let dir = Dir::with_history("t61", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    press(&mut k, KeyInput::sym(Keysym::Down));
    press(&mut k, KeyInput::sym(Keysym::Down));
    press(&mut k, KeyInput::ch('p').ctrl());
    assert_eq!(selected_url(&k), "https://github.com/rust-lang/rust");
    press(&mut k, KeyInput::sym(Keysym::Up));
    press(&mut k, KeyInput::sym(Keysym::Up));
    assert_eq!(k.picker.as_ref().unwrap().selected, 0);
}

#[test]
fn keys_t60_the_list_scrolls_to_keep_the_selection_shown() {
    let rows: Vec<Row> = (0..30).map(|i| row(&format!("r{i}"), &[&format!("r{i}")])).collect();
    let mut p = Picker::new("t", rows);
    let room = PICKER_MAX_ROWS;
    for _ in 0..room {
        p.move_by(1);
    }
    let v = p.view(room);
    assert_eq!(v.rows.len(), room);
    assert_eq!(v.selected, Some(room - 1), "the selection is the last row shown");
    assert_eq!(v.rows[room - 1].0, format!("r{room}"));
    assert_eq!(v.count, "30/30");
    // Moving back up inside the window does not scroll.
    p.move_by(-1);
    assert_eq!(p.view(room).rows[0].0, "r1");
    // A smaller room (a short page) still shows the selection.
    let v = p.view(3);
    assert_eq!(v.rows.len(), 3);
    assert_eq!(v.rows[v.selected.unwrap()].0, format!("r{}", room - 1));
    assert_eq!(scroll_top(5, 2, 4), 2);
    assert_eq!(scroll_top(0, 9, 4), 6);
    assert_eq!(scroll_top(3, 4, 4), 3);
}

#[test]
fn keys_t61a_page_down_and_page_up_move_a_screenful_and_clamp() {
    let hist: String = (0..40).map(|i| format!("{}\thttp://h{i}.test/\tT{i}\n", 1000 + i)).collect();
    let dir = Dir::with_history("t61a", &hist);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    press(&mut k, KeyInput::sym(Keysym::Page_Down));
    assert_eq!(k.picker.as_ref().unwrap().selected, PICKER_MAX_ROWS);
    press(&mut k, KeyInput::sym(Keysym::Page_Down));
    assert_eq!(k.picker.as_ref().unwrap().selected, 2 * PICKER_MAX_ROWS);
    for _ in 0..5 {
        press(&mut k, KeyInput::sym(Keysym::Page_Down));
    }
    assert_eq!(k.picker.as_ref().unwrap().selected, 39, "clamped at the last row");
    press(&mut k, KeyInput::sym(Keysym::Page_Up));
    assert_eq!(k.picker.as_ref().unwrap().selected, 39 - PICKER_MAX_ROWS);
    for _ in 0..5 {
        press(&mut k, KeyInput::sym(Keysym::Page_Up));
    }
    assert_eq!(k.picker.as_ref().unwrap().selected, 0, "clamped at the first row");
    assert_eq!(k.line.text, "", "the page keys type nothing");
}

// ---------- T62/T63: Enter ----------

#[test]
fn keys_t62_enter_opens_the_selected_url_and_closes() {
    let dir = Dir::with_history("t62", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    k.effects.clear();
    type_str(&mut k, "news");
    press(&mut k, KeyInput::ch('n').ctrl());
    let want = selected_url(&k);
    assert_eq!(want, "http://news.test/today");
    press(&mut k, KeyInput::sym(Keysym::KP_Enter));
    assert_eq!(opens(&mut k), vec![want]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.picker.is_none());
    assert_eq!(k.bar_view().line, None);
}

#[test]
fn keys_t63_enter_with_no_match_does_nothing() {
    let dir = Dir::with_history("t63", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    k.effects.clear();
    type_str(&mut k, "qqqq");
    assert_eq!(k.picker.as_ref().unwrap().match_count(), 0);
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(opens(&mut k).is_empty());
    assert_eq!(k.vim_mode, VimMode::Pick, "still open: edit the filter or Esc");
}

// ---------- T64/T65 ----------

#[test]
fn keys_t64_esc_closes_without_action() {
    let dir = Dir::with_history("t64", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    k.effects.clear();
    type_str(&mut k, "git");
    press(&mut k, KeyInput::sym(Keysym::Escape));
    assert!(k.effects.is_empty(), "{:?}", k.effects);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(k.picker.is_none());
    assert_eq!(k.line.text, "");
}

#[test]
fn keys_t65_tab_and_delete_do_nothing() {
    let dir = Dir::with_history("t65", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch ex");
    let before = (k.line.clone(), k.picker.as_ref().unwrap().selected);
    for key in [
        KeyInput::sym(Keysym::Tab),
        KeyInput::sym(Keysym::ISO_Left_Tab),
        KeyInput::sym(Keysym::Delete),
        KeyInput::ch('x').alt(),
    ] {
        press(&mut k, key);
        assert_eq!((k.line.clone(), k.picker.as_ref().unwrap().selected), before, "{key:?}");
    }
    assert_eq!(k.bar_view().ghost, "", "no ghost in PICK");
}

// ---------- T66-T68: the mouse and focus ----------

#[test]
fn keys_t66_a_press_over_the_list_closes_and_is_kept_from_the_page() {
    let dir = Dir::with_history("t66", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    assert!(k.pick_click(true));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.picker.is_none());
    assert!(!k.pick_click(true), "no picker: nothing is kept");
}

#[test]
fn keys_t67_a_press_elsewhere_closes_and_reaches_the_page() {
    let dir = Dir::with_history("t67", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    assert!(!k.pick_click(false));
    assert!(k.picker.is_none());
}

#[test]
fn keys_t68_an_editable_focus_closes_it_into_input() {
    let dir = Dir::with_history("t68", HISTORY);
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    // C-AUTOFOCUS: a focus the user caused (a click: `after_click`); the page's own leaves PICK open.
    k.buffers.tab_created(1, None);
    let t = std::time::Instant::now();
    assert_eq!(k.editable_focused(1, false, t), FocusOutcome::Parked);
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(k.picker.is_some());
    let later = t + std::time::Duration::from_millis(2 * crate::config::CLICK_FOCUS_MS);
    assert_eq!(k.editable_focused(1, true, later), FocusOutcome::Input);
    assert_eq!(k.key_mode, KeyMode::Input);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.picker.is_none());
    // A popup after a click (T29) too.
    run(&mut k, "x"); // in INPUT `:` is the page's: nothing opens
    let mut k = keys_with(&dir);
    run(&mut k, "historySearch");
    k.popup_opened(true);
    assert!(k.picker.is_none());
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Normal, "T57: back to NORMAL, not PICK");
}

// ---------- drawing: geometry and scale ----------

#[test]
fn picker_sits_on_the_bar_and_fits_the_page() {
    let l = Layout::new(1000, 800);
    let page_h = 800 - STATUS_BAR_H;
    assert_eq!(room(&l), PICKER_MAX_ROWS);
    let r = overlay_rect(&l, 3).unwrap();
    assert_eq!(
        r,
        Rectangle::new((0, page_h - 4 * PICKER_ROW_H).into(), (1000, 4 * PICKER_ROW_H).into())
    );
    assert_eq!(r.loc.y + r.size.h, l.bar().unwrap().loc.y, "its bottom is the bar's top");
    // A short page shows fewer rows; a page shorter than two rows shows none.
    let short = Layout::new(400, STATUS_BAR_H + 5 * PICKER_ROW_H + 3);
    assert_eq!(room(&short), 4);
    assert!(overlay_rect(&short, 4).is_some());
    assert!(overlay_rect(&short, 5).is_none());
    assert_eq!(room(&Layout::new(400, STATUS_BAR_H + PICKER_ROW_H)), 0);
}

#[test]
fn picker_scale_math_meets_the_bar_at_every_scale() {
    for s in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let phys = ((1000.0 * s) as i32, (801.0_f64 * s).round() as i32);
        let l = Layout::from_physical(phys.into(), s);
        let r = overlay_rect(&l, 5).unwrap();
        let (loc, size) = to_physical(r, s);
        let (bar_loc, _) = l.bar_physical().unwrap();
        assert_eq!(loc.y + size.h, bar_loc.y, "scale {s}: no gap or overlap with the bar");
        assert_eq!(size.w, (l.window.w as f64 * s).round() as i32, "scale {s}: full width");
        assert!(
            ((size.h as f64) - 6.0 * PICKER_ROW_H as f64 * s).abs() <= 1.0,
            "scale {s}: {size:?}"
        );
    }
    let (loc, size) = to_physical(Rectangle::new((0, 100).into(), (500, 44).into()), 2.0);
    assert_eq!((loc.x, loc.y, size.w, size.h), (0, 200, 1000, 88));
}

#[test]
fn picker_paints_at_the_scale_with_the_selection_lighter() {
    use crate::config::{PICKER_BG, PICKER_SEL_BG};
    let mut painter = crate::statusbar::Painter::new();
    let view = PickView {
        name: "history".into(),
        count: "2/2".into(),
        rows: vec![("One".into(), "https://one.test/".into()), ("Two".into(), String::new())],
        selected: Some(1),
    };
    for s in [1.0f32, 2.0] {
        let h = (3.0 * PICKER_ROW_H as f32 * s) as u32;
        let w = (300.0 * s) as u32;
        let pm = paint(&mut painter, &view, w, h, s).unwrap();
        assert_eq!((pm.width(), pm.height()), (w, h));
        let at = |y: f32| {
            let p = pm.pixel(w - 2, (y * s) as u32).unwrap();
            (p.red(), p.green(), p.blue())
        };
        let row = PICKER_ROW_H as f32;
        assert_eq!(at(row * 1.5), PICKER_BG, "scale {s}: row 1 unselected");
        assert_eq!(at(row * 2.5), PICKER_SEL_BG, "scale {s}: row 2 selected");
        assert_eq!(at(row * 2.0 + 0.5), PICKER_SEL_BG, "scale {s}: the selected row starts on its edge");
        if painter.has_fonts() {
            let ink = (0..w / 2)
                .flat_map(|x| (0..(row * s) as u32).map(move |y| (x, y)))
                .filter(|&(x, y)| {
                    let p = pm.pixel(x, y).unwrap();
                    (p.red(), p.green(), p.blue()) != PICKER_BG
                })
                .count();
            assert!(ink as f32 > 20.0 * s * s, "scale {s}: the header has text ({ink} px)");
        }
    }
}

#[test]
fn pick_log_names_the_selection_for_the_live_check() {
    let dir = Dir::with_history("log", HISTORY);
    let mut k = keys_with(&dir);
    assert_eq!(k.pick_log(), "");
    run(&mut k, "historySearch");
    assert_eq!(k.pick_log(), " pick history [1/4 of 4] https://www.example.org/");
}
