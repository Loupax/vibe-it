//! C4 buffers (KEYS.md "Buffers", T77-T92). Each test names its row.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::completion::{compute_ghost, CompleteCache};
use crate::keys::*;
use crate::picker::PickAction;
use crate::protocol::{Command, Event};

const A: &str = "https://a.test/one";
const B: &str = "https://blue.example/two";

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_text(k: &mut Keys, text: &str) {
    for c in text.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

/// Runs `:line` as typed (Delete hides a ghost, T52) and returns the effects it queued.
fn run(k: &mut Keys, line: &str) -> Vec<Effect> {
    k.effects.clear();
    press(k, KeyInput::ch(':'));
    type_text(k, line);
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
    std::mem::take(&mut k.effects)
}

fn sends(effects: &[Effect]) -> Vec<Command> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

/// Buffer 1 on tab 1 showing A, buffer 2 on tab 2 showing B (current), as after a second `web B`.
fn two_buffers() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = A.into();
    k.title = "Page one".into();
    k.open_buffer(B);
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    k.title = "Blue page".into();
    k.effects.clear();
    k.message.clear();
    k
}

#[test]
fn c4_t86_a_second_web_url_opens_a_new_buffer_and_shows_it() {
    let mut k = Keys::new(None, String::new(), None);
    k.buffers.tab_created(1, None);
    k.url = A.into();
    let n = k.open_buffer(B);
    assert_eq!(n, 2);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.url, B, "the new buffer shows its URL before the page reports it");
    // new-tab now; show-tab only when the host named the page (State sends it on tab-created).
    assert_eq!(
        sends(&k.effects),
        vec![Command::NewTab {
            req: 1,
            url: Some(B.into())
        }]
    );
    assert_eq!(k.buffers.current_tab(), None);
    assert_eq!(k.buffers.tab_created(5, Some(1)), TabCreated::Buffer(2));
    assert_eq!(k.buffers.current_tab(), Some(5));
    // Buffer 1 kept its URL.
    assert_eq!(k.buffers.get(1).unwrap().url, A);
}

#[test]
fn c4_t85_the_status_bar_shows_the_buffer_number() {
    let mut k = two_buffers();
    assert_eq!(k.bar_view().url, format!("[2] {B}"));
    run(&mut k, "buffer 1");
    assert_eq!(k.bar_view().url, format!("[1] {A}"));
}

#[test]
fn c4_t78_buffer_n_switches_and_shows_its_page() {
    let mut k = two_buffers();
    let e = run(&mut k, "buffer 1");
    assert_eq!(sends(&e), vec![Command::ShowTab { tab: 1 }]);
    assert_eq!(k.buffers.current, 1);
    assert_eq!((k.url.as_str(), k.title.as_str()), (A, "Page one"));
    let e = run(&mut k, "b 2");
    assert_eq!(sends(&e), vec![Command::ShowTab { tab: 2 }]);
    assert_eq!((k.url.as_str(), k.title.as_str()), (B, "Blue page"));
    let e = run(&mut k, "b 9");
    assert!(sends(&e).is_empty());
    assert_eq!(k.message, "web: no buffer 9");
    // The current one: nothing to switch, it is named.
    let e = run(&mut k, "b 2");
    assert!(sends(&e).is_empty());
    assert_eq!(k.message, "buffer 2: Blue page");
}

#[test]
fn c4_t79_buffer_partial_matches_title_hostname_description() {
    let mut k = two_buffers();
    run(&mut k, "b page one");
    assert_eq!(k.buffers.current, 1, "by title");
    run(&mut k, "b BLUE.EX");
    assert_eq!(k.buffers.current, 2, "by hostname, case-insensitive");
    run(&mut k, "bufferDescribe mail");
    run(&mut k, "b 1");
    run(&mut k, "b mai");
    assert_eq!(k.buffers.current, 2, "by description");
    run(&mut k, "b nothing");
    assert_eq!(k.message, "web: no buffer matches nothing");
    run(&mut k, "buffer page");
    assert_eq!(k.message, "web: more than one buffer matches page (:bufferList)");
    assert_eq!(k.buffers.current, 2);
}

#[test]
fn c4_t80_buffer_next_and_prev_wrap() {
    let mut k = two_buffers();
    k.open_buffer("https://c.test/");
    k.buffers.tab_created(3, Some(2));
    assert_eq!(k.buffers.current, 3);
    run(&mut k, "bufferNext");
    assert_eq!(k.buffers.current, 1, "wraps after the last");
    run(&mut k, "bn");
    assert_eq!(k.buffers.current, 2);
    run(&mut k, "bufferPrev");
    run(&mut k, "bp");
    assert_eq!(k.buffers.current, 3, "wraps before the first");
    // One buffer: next stays.
    let mut one = Keys::new(None, String::new(), None);
    one.buffers.tab_created(1, None);
    let e = run(&mut one, "bn");
    assert!(sends(&e).is_empty());
    assert_eq!(one.buffers.current, 1);
}

#[test]
fn c4_t81_buffer_kill_goes_to_the_alternate_and_closes_the_page() {
    let mut k = two_buffers();
    k.open_buffer("https://c.test/");
    k.buffers.tab_created(3, Some(2));
    run(&mut k, "b 1"); // current 1, alternate 3
    let e = run(&mut k, "bufferKill");
    assert_eq!(sends(&e), vec![Command::ShowTab { tab: 3 }, Command::CloseTab { tab: 1 }]);
    assert_eq!(k.buffers.current, 3);
    assert_eq!(k.message, "buffer 1 killed");
    assert!(k.buffers.get(1).is_none());
    // `:bd N` kills another buffer; the current stays.
    let e = run(&mut k, "bd 2");
    assert_eq!(sends(&e), vec![Command::CloseTab { tab: 2 }]);
    assert_eq!(k.buffers.current, 3);
    run(&mut k, "bd 7");
    assert_eq!(k.message, "web: no buffer 7");
    run(&mut k, "bd x");
    assert_eq!(k.message, crate::buffers::KILL_USAGE);
}

#[test]
fn c4_t81_a_buffer_killed_before_its_page_came_closes_the_page_when_it_comes() {
    let mut k = two_buffers();
    k.open_buffer("https://slow.test/");
    let e = run(&mut k, "bd");
    assert!(
        sends(&e).iter().all(|c| !matches!(c, Command::CloseTab { .. })),
        "no page to close yet"
    );
    assert_eq!(k.buffers.tab_created(9, Some(2)), TabCreated::Doomed);
    assert!(k.buffers.list.iter().all(|b| b.tab != Some(9)));
}

#[test]
fn c4_t82_killing_the_last_buffer_leaves_about_blank() {
    let mut k = Keys::new(None, String::new(), None);
    k.buffers.tab_created(1, None);
    k.url = A.into();
    let e = run(&mut k, "bd");
    assert_eq!(sends(&e), vec![Command::NewTab { req: 1, url: None }, Command::CloseTab { tab: 1 }]);
    assert_eq!(k.buffers.list.len(), 1);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.url, "about:blank");
    assert_eq!(k.buffers.alternate, None, "the killed buffer is no alternate");
}

#[test]
fn c4_t83_buffer_describe_sets_keeps_and_clears() {
    let mut k = two_buffers();
    run(&mut k, "bufferDescribe the blue one");
    assert_eq!(k.message, "buffer 2: the blue one");
    // Kept across navigation (a load in the buffer) and across switching.
    assert!(!k.page_event(
        2,
        &Event::Url {
            tab: 2,
            url: "https://blue.example/next".into(),
            original: None
        }
    ));
    k.navigated();
    run(&mut k, "b 1");
    run(&mut k, "b 2");
    assert_eq!(k.buffers.current().description, "the blue one");
    run(&mut k, "bufferDescribe");
    assert_eq!(k.message, "buffer 2: description cleared");
    assert_eq!(k.buffers.current().description, "");
}

#[test]
fn c4_t77_buffer_list_is_a_picker_over_the_buffers() {
    let mut k = two_buffers();
    run(&mut k, "bufferDescribe mail");
    run(&mut k, "ls");
    assert_eq!(k.vim_mode, VimMode::Pick);
    let p = k.picker.as_ref().unwrap();
    assert_eq!(p.name, "buffers");
    let rows: Vec<_> = p.matches().map(|r| (r.label.clone(), r.detail.clone(), r.action.clone())).collect();
    assert_eq!(
        rows,
        vec![
            ("1# Page one".into(), "a.test".into(), PickAction::Buffer(1)),
            ("2% Blue page".into(), "blue.example  mail".into(), PickAction::Buffer(2)),
        ]
    );
    // Filtered on title, hostname and description; Enter shows the buffer.
    type_text(&mut k, "a.te");
    assert!(k.pick_log().contains("[1/1 of 2] buffer 1"), "{}", k.pick_log());
    k.effects.clear();
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(sends(&k.effects), vec![Command::ShowTab { tab: 1 }]);
    // `:bufferList <filter>` types the filter.
    run(&mut k, "bufferList mail");
    assert!(k.pick_log().contains("[1/1 of 2] buffer 2"), "{}", k.pick_log());
}

#[test]
fn c4_t84_ctrl_caret_shows_the_previous_buffer() {
    let mut k = two_buffers();
    press(&mut k, KeyInput::ch('^').ctrl());
    assert_eq!(k.buffers.current, 1);
    assert_eq!(sends(&k.effects), vec![Command::ShowTab { tab: 1 }]);
    k.effects.clear();
    // Ctrl-6 (vim's other spelling) too, back to 2.
    press(&mut k, KeyInput::ch('6').ctrl());
    assert_eq!(k.buffers.current, 2);
    let mut one = Keys::new(None, String::new(), None);
    one.buffers.tab_created(1, None);
    press(&mut one, KeyInput::ch('^').ctrl());
    assert_eq!(one.message, "web: no alternate buffer");
    // Not in INPUT: the page gets it.
    let mut k = two_buffers();
    k.set_key_mode(KeyMode::Input);
    let key = KeyInput::ch('^').ctrl();
    assert!(k.key(key.sym.raw(), true, &key, Instant::now()));
    assert_eq!(k.buffers.current, 2);
}

#[test]
fn c4_t87_search_url_title_and_popup_return_are_per_buffer() {
    let mut k = two_buffers();
    press(&mut k, KeyInput::ch('/'));
    type_text(&mut k, "needle");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(k.search.session);
    k.popup_return = Some(KeyMode::Normal);
    run(&mut k, "b 1");
    assert_eq!(k.search.query, None);
    assert!(!k.search.session);
    assert_eq!(k.popup_return, None);
    k.effects.clear();
    press(&mut k, KeyInput::ch('n'));
    assert_eq!(k.message, NO_PREVIOUS_SEARCH);
    run(&mut k, "b 2");
    assert_eq!(k.search.query.as_deref(), Some("needle"));
    assert!(k.search.session, "the page kept its find session");
    assert_eq!(k.popup_return, Some(KeyMode::Normal));
}

#[test]
fn c4_t87_events_of_a_hidden_buffer_stay_in_that_buffer() {
    let mut k = two_buffers();
    k.message = "mine".into();
    // Buffer 1 (tab 1) is hidden: its events update it and nothing else.
    assert!(k.page_event(
        1,
        &Event::Url {
            tab: 1,
            url: "https://a.test/next".into(),
            original: None
        }
    ));
    assert!(k.page_event(
        1,
        &Event::Title {
            tab: 1,
            title: "Next".into()
        }
    ));
    assert!(k.page_event(1, &Event::EditableFocused { tab: 1, focused: true }));
    assert_eq!((k.url.as_str(), k.message.as_str()), (B, "mine"));
    let b1 = k.buffers.get(1).unwrap();
    assert_eq!((b1.url.as_str(), b1.title.as_str()), ("https://a.test/next", "Next"));
    // A cross-document load ends that buffer's find session (T31), loading until it ends.
    k.buffers.get_mut(1).unwrap().search.session = true;
    let load = |phase: &str| Event::Load {
        tab: 1,
        phase: phase.into(),
        status: 200,
        kind: "new".into(),
        error: None,
    };
    k.page_event(1, &load("start"));
    let b1 = k.buffers.get(1).unwrap();
    assert!(b1.loading && !b1.search.session && b1.title.is_empty());
    k.page_event(1, &load("end"));
    assert!(!k.buffers.get(1).unwrap().loading);
    // The current buffer's events go on to the rest of the shell.
    assert!(!k.page_event(2, &Event::Title { tab: 2, title: "x".into() }));
    // A page with no buffer (killed, its tab-closed still coming) is dropped.
    assert!(k.page_event(
        77,
        &Event::Title {
            tab: 77,
            title: "x".into()
        }
    ));
}

#[test]
fn c4_t88_forget_site_reloads_every_buffer_on_the_site() {
    let mut k = two_buffers();
    k.open_buffer("https://mail.a.test/inbox");
    k.buffers.tab_created(3, Some(2));
    k.open_buffer("https://a.test.evil/");
    k.buffers.tab_created(4, Some(3));
    let hit: Vec<(u32, u64)> = k.buffers_on_site("a.test").into_iter().map(|(n, t, _)| (n, t)).collect();
    assert_eq!(hit, vec![(1, 1), (3, 3)]);
    assert!(k.buffers_on_site("").is_empty());
    // The page command goes to each page by its tab.
    assert_eq!(Command::Reload { hard: false }.line_for(3), "{\"cmd\":\"reload\",\"tab\":3}\n");
}

#[test]
fn c4_t89_a_crash_concerns_its_buffer_only() {
    let mut k = two_buffers();
    assert!(k.page_event(
        1,
        &Event::Crashed {
            tab: 1,
            reason: "x".into()
        }
    ));
    assert_eq!(k.message, "web: the page of buffer 1 crashed (the other buffers are fine)");
    assert!(!k.buffers.get(2).unwrap().crashed);
    assert_eq!(k.buffers.list.len(), 2, "the buffer stays");
    run(&mut k, "b 1");
    assert_eq!(k.message, "web: the page crashed; :refresh loads it again");
    let e = run(&mut k, "refresh");
    assert_eq!(sends(&e), vec![Command::Reload { hard: false }]);
    k.page_event(
        1,
        &Event::Load {
            tab: 1,
            phase: "start".into(),
            status: 0,
            kind: "reload".into(),
            error: None,
        },
    );
    assert!(!k.buffers.get(1).unwrap().crashed);
}

#[test]
fn c4_t90_the_ghost_offers_long_names_an_alias_runs_as_typed() {
    for a in COMMAND_ALIASES {
        assert!(!COMMAND_NAMES.contains(a), "{a} is offered");
    }
    for n in ["bufferList", "buffer", "bufferNext", "bufferPrev", "bufferKill", "bufferDescribe"] {
        assert!(COMMAND_NAMES.contains(&n), "{n}");
    }
    let cache = CompleteCache::new(None);
    let g = compute_ghost("bufferNe", 8, &cache).unwrap();
    assert_eq!(g.rest("bufferNe"), "w");
    // `b` typed in full: no ghost text, Enter runs `:b` (not `:bookmarkAdd`); Tab still cycles.
    let g = compute_ghost("b", 1, &cache).unwrap();
    assert_eq!(g.rest("b"), "");
    assert!(
        compute_ghost("l", 1, &cache).is_none_or(|g| g.rest("l") != "s"),
        "ls is never offered"
    );
    let mut k = two_buffers();
    press(&mut k, KeyInput::ch(':'));
    press(&mut k, KeyInput::ch('b'));
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.message, "buffer 2: Blue page");
}

#[test]
fn c4_t91_q_quits_every_buffer() {
    let mut k = two_buffers();
    let e = run(&mut k, "q");
    assert_eq!(e, vec![Effect::Quit]);
}

#[test]
fn c4_t92_a_page_nobody_asked_for_is_not_shown() {
    // Ctrl/middle-click open nothing (the host drops them, host-cef newtab.rs); should a newer host
    // announce a page anyway, it gets a buffer of its own and the current one stays.
    let mut k = two_buffers();
    assert_eq!(k.buffers.tab_created(11, None), TabCreated::Adopted(3));
    assert_eq!(k.buffers.current, 2);
}

#[test]
fn c4_the_page_closed_by_itself_leaves_its_buffer() {
    let mut k = two_buffers();
    k.tab_gone(2);
    assert_eq!(k.buffers.current, 1);
    assert!(sends(&k.effects).iter().all(|c| !matches!(c, Command::CloseTab { .. })));
    assert_eq!(k.message, "buffer 2 closed by the page");
}
