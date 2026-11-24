//! C-SCRIPT-MAP (SCRIPTING.md "web.map", KEYS.md T118-T124): user mappings in NORMAL, and proof
//! that keys behave exactly as before when no mapping (or no related mapping) exists.

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::config::{evdev, MAP_TIMEOUT_MS, SCRIPT_BUDGET_MS};
use crate::keys::*;
use crate::protocol::Command;

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);
const TIMEOUT: Duration = Duration::from_millis(MAP_TIMEOUT_MS);

fn keys() -> Keys {
    Keys::new(None, String::new(), None)
}

/// A shell with `src` loaded as init.js; the load's log lines are dropped.
fn shell(src: &str) -> Keys {
    let mut k = keys();
    k.load_script_source("init.js", src, BUDGET);
    k.url = "https://a.test/".into();
    k.effects.clear();
    k
}

fn ms(t: Instant, n: u64) -> Instant {
    t + Duration::from_millis(n)
}

fn press(k: &mut Keys, key: KeyInput, t: Instant) -> bool {
    k.key(key.sym.raw(), true, &key, t)
}

fn release(k: &mut Keys, key: KeyInput, t: Instant) -> bool {
    k.key(key.sym.raw(), false, &key, t)
}

fn tap(k: &mut Keys, key: KeyInput, t: Instant) -> (bool, bool) {
    (press(k, key, t), release(k, key, t))
}

fn tap_str(k: &mut Keys, s: &str, t: Instant) {
    for c in s.chars() {
        tap(k, KeyInput::ch(c), t);
    }
}

/// The effects so far, the log lines left out.
fn effects(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter(|e| !matches!(e, Effect::Log(_)))
        .collect()
}

fn open(url: &str) -> Effect {
    Effect::Send(Command::Open { url: url.into() })
}

const RELOAD: Effect = Effect::Send(Command::Reload { hard: false });

// ---------- notation (pure) ----------

#[test]
fn map_keys_notation() {
    let k = |c, ctrl| MapKey { ch: c, ctrl };
    assert_eq!(parse_map_keys("gh"), Ok(vec![k('g', false), k('h', false)]));
    assert_eq!(parse_map_keys("G"), Ok(vec![k('G', false)]));
    assert_eq!(parse_map_keys("<C-x>"), Ok(vec![k('x', true)]));
    assert_eq!(
        parse_map_keys("<c-X>"),
        Ok(vec![k('x', true)]),
        "a letter under Ctrl is one key, any case"
    );
    assert_eq!(parse_map_keys("<C-^>a"), Ok(vec![k('^', true), k('a', false)]));
    assert_eq!(parse_map_keys("<lt>a<LT>"), Ok(vec![k('<', false), k('a', false), k('<', false)]));
    assert_eq!(parse_map_keys(">"), Ok(vec![k('>', false)]));
    for bad in ["", "<", "a<b", "<A-x>", "<Esc>", "<CR>", "<C-xy>", "<C->", "\n"] {
        assert!(parse_map_keys(bad).is_err(), "{bad:?} is refused");
    }
    // A press as a mapping key: Alt/Super and keys with no character never are.
    assert_eq!(MapKey::from_input(&KeyInput::ch('x').ctrl()), Some(k('x', true)));
    assert_eq!(MapKey::from_input(&KeyInput::ch('X').ctrl()), Some(k('x', true)));
    assert_eq!(MapKey::from_input(&KeyInput::ch('G')), Some(k('G', false)));
    assert_eq!(MapKey::from_input(&KeyInput::ch('x').alt()), None);
    assert_eq!(MapKey::from_input(&KeyInput::sym(Keysym::Escape)), None);
    assert_eq!(MapKey::from_input(&KeyInput::sym(Keysym::Return)), None);
}

#[test]
fn map_rhs_cr_rule() {
    let line = |t: &str, run| Ok(MapLine { text: t.into(), run });
    assert_eq!(parse_map_rhs(":go https://example.com<CR>"), line("go https://example.com", true));
    assert_eq!(parse_map_rhs(":refresh<cr>"), line("refresh", true));
    assert_eq!(parse_map_rhs(":refresh<Enter>"), line("refresh", true));
    assert_eq!(parse_map_rhs(":refresh<RETURN>"), line("refresh", true));
    assert_eq!(parse_map_rhs(":go "), line("go ", false));
    assert_eq!(parse_map_rhs(":"), line("", false));
    assert_eq!(parse_map_rhs(":let @+ = <lt>x><CR>"), line("let @+ = <x>", true));
    for bad in [
        "go x<CR>",     // no ':'
        ":a<CR>:b<CR>", // <CR> before the end
        ":a<CR> ",      // <CR> not at the very end
        ":a<Esc>",      // no other keys
        ":a<C-u>",
        ":a<b", // lone '<'
        ":<Tab>",
    ] {
        assert!(parse_map_rhs(bad).is_err(), "{bad:?} is refused");
    }
}

#[test]
fn map_matcher_steps() {
    let ks = |s: &str| parse_map_keys(s).unwrap();
    let maps = [ks("a"), ks("abc"), ks("xy")];
    let it = || maps.iter().map(|m| m.as_slice());
    let typed = |s: &str| ks(s).into_iter().map(Some).collect::<Vec<_>>();
    assert_eq!(resolve_mapping(&typed("x"), it(), true), MapStep::Wait);
    assert_eq!(resolve_mapping(&typed("xy"), it(), true), MapStep::Fire { index: 2, len: 2 });
    assert_eq!(resolve_mapping(&typed("xz"), it(), true), MapStep::Replay);
    assert_eq!(resolve_mapping(&typed("x"), it(), false), MapStep::Replay, "timeout: no full match");
    assert_eq!(
        resolve_mapping(&typed("a"), it(), true),
        MapStep::Wait,
        "a is also the start of abc"
    );
    assert_eq!(resolve_mapping(&typed("a"), it(), false), MapStep::Fire { index: 0, len: 1 });
    assert_eq!(resolve_mapping(&typed("abx"), it(), true), MapStep::Fire { index: 0, len: 1 });
    assert_eq!(resolve_mapping(&typed("abc"), it(), true), MapStep::Fire { index: 1, len: 3 });
    assert_eq!(resolve_mapping(&[Some(ks("x")[0]), None], it(), true), MapStep::Replay);
    assert_eq!(resolve_mapping(&typed("q"), it(), true), MapStep::Replay);
}

// ---------- the TODO's list ----------

#[test]
fn map_a_prefix_waits_then_the_mapping_runs() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "gh", ":go http://127.0.0.1:1/h<CR>")"#);
    assert_eq!(tap(&mut k, KeyInput::ch('g'), t), (false, false));
    assert!(effects(&mut k).is_empty(), "g waits");
    assert_eq!(k.map_wait(t), Some(TIMEOUT));
    assert_eq!(tap(&mut k, KeyInput::ch('h'), ms(t, 200)), (false, false));
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/h")]);
    assert_eq!(k.map_wait(t), None);
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn map_timeout_replays_the_pending_keys_as_without_the_mapping() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "jk", ":refresh<CR>")"#);
    // j tapped and released while it waits: on the timeout it scrolls (down and up at once).
    tap(&mut k, KeyInput::ch('j'), t);
    assert!(effects(&mut k).is_empty());
    assert_eq!(k.map_tick(ms(t, 999)), Some(Duration::from_millis(1)), "not yet");
    assert!(effects(&mut k).is_empty());
    assert_eq!(k.map_tick(t + TIMEOUT), None);
    assert_eq!(
        effects(&mut k),
        vec![Effect::KeyDown(evdev::KEY_DOWN), Effect::KeyUp(evdev::KEY_DOWN)]
    );
    // j still held at the timeout: the scroll key is held until j is released.
    let j = KeyInput::ch('j');
    press(&mut k, j, t);
    k.map_tick(t + TIMEOUT);
    assert_eq!(effects(&mut k), vec![Effect::KeyDown(evdev::KEY_DOWN)]);
    release(&mut k, j, ms(t, 1500));
    assert_eq!(effects(&mut k), vec![Effect::KeyUp(evdev::KEY_DOWN)]);
    // A key that matches nothing replays j at once, then is handled itself.
    tap(&mut k, KeyInput::ch('j'), t);
    tap(&mut k, KeyInput::ch('h'), ms(t, 10));
    assert_eq!(
        effects(&mut k),
        vec![
            Effect::KeyDown(evdev::KEY_DOWN),
            Effect::KeyUp(evdev::KEY_DOWN),
            Effect::Send(Command::Back)
        ]
    );
    // A key pressed after the deadline but before the timer fired: the timeout comes first.
    tap(&mut k, KeyInput::ch('j'), t);
    tap(&mut k, KeyInput::ch('k'), t + TIMEOUT);
    assert_eq!(
        effects(&mut k),
        vec![
            Effect::KeyDown(evdev::KEY_DOWN),
            Effect::KeyUp(evdev::KEY_DOWN),
            Effect::KeyDown(evdev::KEY_UP),
            Effect::KeyUp(evdev::KEY_UP)
        ]
    );
    // jk in time runs the mapping, no scrolling.
    tap(&mut k, KeyInput::ch('j'), t);
    tap(&mut k, KeyInput::ch('k'), ms(t, 999));
    assert_eq!(effects(&mut k), vec![RELOAD]);
}

#[test]
fn map_overrides_j() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "j", ":refresh<CR>")"#);
    assert_eq!(tap(&mut k, KeyInput::ch('j'), t), (false, false));
    assert_eq!(effects(&mut k), vec![RELOAD], "no scroll, no wait");
    assert_eq!(k.map_wait(t), None);
    // k is still k.
    tap(&mut k, KeyInput::ch('k'), t);
    assert_eq!(effects(&mut k), vec![Effect::KeyDown(evdev::KEY_UP), Effect::KeyUp(evdev::KEY_UP)]);
}

#[test]
fn map_overrides_gg_and_leaves_gi_and_a_lone_g_alone() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "gg", ":refresh<CR>")"#);
    tap_str(&mut k, "gg", t);
    assert_eq!(effects(&mut k), vec![RELOAD], "no scroll to the top");
    // `gi` still focuses a field (the built-in sequence gets its second key).
    tap_str(&mut k, "gi", t);
    assert_eq!(
        effects(&mut k),
        vec![Effect::Send(Command::FocusField {
            which: crate::protocol::FieldWhich::Last
        })]
    );
    // g, then the timeout, then g: the first g arms the built-in sequence, the second completes it
    // unmapped (as vim reads a command's second key with mappings off).
    tap(&mut k, KeyInput::ch('g'), t);
    k.map_tick(t + TIMEOUT);
    tap(&mut k, KeyInput::ch('g'), t + TIMEOUT);
    assert_eq!(
        effects(&mut k),
        vec![Effect::KeyDown(evdev::KEY_HOME), Effect::KeyUp(evdev::KEY_HOME)]
    );
}

#[test]
fn map_cr_runs_the_line() {
    let t = Instant::now();
    let mut k = shell(
        r#"
        web.command("Hi", (a) => web.cmd("go http://127.0.0.1:1/" + a.join("-")));
        web.map("n", "Q", ":Hi a b<CR>");
        web.map("n", "<C-x>", ":refresh<Enter>");
        web.map("n", "<lt>", ":refresh<return>");
        "#,
    );
    assert_eq!(k.message, "");
    tap(&mut k, KeyInput::ch('Q'), t);
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    tap(&mut k, KeyInput::ch('x').ctrl(), t);
    assert_eq!(effects(&mut k), vec![RELOAD]);
    tap(&mut k, KeyInput::ch('<'), t);
    assert_eq!(effects(&mut k), vec![RELOAD]);
    // x without Ctrl is not <C-x>.
    tap(&mut k, KeyInput::ch('x'), t);
    assert!(effects(&mut k).is_empty());
}

#[test]
fn map_without_cr_opens_the_line_with_the_text() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "ge", ":go ")"#);
    tap_str(&mut k, "ge", t);
    assert!(effects(&mut k).is_empty(), "nothing runs");
    assert_eq!(k.vim_mode, VimMode::Command);
    assert_eq!(k.line.text, "go https://a.test/");
    assert_eq!(k.line.sel, Some((3, 18)), "C-EDITURL: the page URL, selected");
    assert_eq!(k.line.cursor, 18, "cursor at the end");
    // The user finishes the line.
    for c in "http://127.0.0.1:1/x".chars() {
        tap(&mut k, KeyInput::ch(c), t);
    }
    tap(&mut k, KeyInput::sym(Keysym::Delete), t);
    tap(&mut k, KeyInput::sym(Keysym::Return), t);
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/x")]);
}

#[test]
fn map_keys_after_an_opening_mapping_go_into_the_line() {
    let t = Instant::now();
    let mut k = shell(
        r#"
        web.map("n", "a", ":go ");
        web.map("n", "abc", ":refresh<CR>");
        "#,
    );
    tap(&mut k, KeyInput::ch('a'), t);
    assert_eq!(k.vim_mode, VimMode::Normal, "a waits: it starts abc");
    tap(&mut k, KeyInput::ch('b'), t);
    tap(&mut k, KeyInput::ch('q'), t);
    // `abq`: `a` runs (the longest mapping it matches), then b and q are typed into its line.
    assert_eq!(k.vim_mode, VimMode::Command);
    assert_eq!(k.line.text, "go bq");
}

#[test]
fn map_bad_rhs_or_keys_is_refused_at_map_time() {
    let t = Instant::now();
    for (src, msg) in [
        (r#"web.map("n", "j", ":a<CR>:b<CR>")"#, "<CR> only at the end"),
        (r#"web.map("n", "j", ":a<Esc>")"#, "<Esc>"),
        (r#"web.map("n", "j", "refresh<CR>")"#, "does not start with ':'"),
        (r#"web.map("n", "j", 5)"#, "rhs must be"),
        (r#"web.map("n", "<A-j>", ":refresh<CR>")"#, "unknown key <A-j>"),
        (r#"web.map("n", "", ":refresh<CR>")"#, "no keys"),
        (r#"web.map("i", "j", ":refresh<CR>")"#, "unknown mode"),
    ] {
        let mut k = shell(&format!("\n{src}\n"));
        assert!(k.message.starts_with("init.js:2: web.map: "), "{src}: {}", k.message);
        assert!(k.message.contains(msg), "{src}: {}", k.message);
        // No mapping was made: j scrolls.
        tap(&mut k, KeyInput::ch('j'), t);
        assert_eq!(
            effects(&mut k),
            vec![Effect::KeyDown(evdev::KEY_DOWN), Effect::KeyUp(evdev::KEY_DOWN)],
            "{src}"
        );
    }
}

// ---------- more behaviour ----------

#[test]
fn map_same_keys_again_replaces() {
    let t = Instant::now();
    let mut k = shell(
        r#"
        web.map("n", "Q", ":go http://127.0.0.1:1/one<CR>");
        web.map("n", "Q", ":go http://127.0.0.1:1/two<CR>");
        "#,
    );
    tap(&mut k, KeyInput::ch('Q'), t);
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/two")]);
}

#[test]
fn map_function_rhs_is_a_call_under_the_budget() {
    let t = Instant::now();
    let mut k = shell(
        r#"
        let n = 0;
        web.map("n", "Q", () => { n++; web.cmd("go http://127.0.0.1:1/" + n); web.cmd("refresh"); });
        web.map("n", "W", () => { throw new Error("boom"); });
        web.map("n", "L", () => { for (;;) {} });
        "#,
    );
    tap(&mut k, KeyInput::ch('Q'), t);
    tap(&mut k, KeyInput::ch('Q'), t);
    assert_eq!(
        effects(&mut k),
        vec![open("http://127.0.0.1:1/1"), RELOAD, open("http://127.0.0.1:1/2"), RELOAD]
    );
    tap(&mut k, KeyInput::ch('W'), t);
    assert_eq!(k.message, "init.js:4: boom");
    let start = Instant::now();
    tap(&mut k, KeyInput::ch('L'), t);
    assert!(k.message.contains("interrupted: ran longer than SCRIPT_BUDGET_MS"), "{}", k.message);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn map_shift_between_the_keys_does_not_break_a_sequence() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "gT", ":refresh<CR>")"#);
    tap(&mut k, KeyInput::ch('g'), t);
    let shift = KeyInput::sym(Keysym::Shift_L);
    press(&mut k, shift, t);
    tap(&mut k, KeyInput::ch('T'), t);
    release(&mut k, shift, t);
    assert_eq!(effects(&mut k), vec![RELOAD]);
}

#[test]
fn map_colon_can_hide_the_command_line_as_in_vim() {
    // No guard (vim allows it; KEYS.md T118's note). Esc cannot be mapped.
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", ":", ":refresh<CR>")"#);
    tap(&mut k, KeyInput::ch(':'), t);
    assert_eq!(effects(&mut k), vec![RELOAD]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    // A mapping to ':' with no text gives the line back.
    let mut k = shell(r#"web.map("n", ";", ":")"#);
    tap(&mut k, KeyInput::ch(';'), t);
    assert_eq!(k.vim_mode, VimMode::Command);
    assert_eq!(k.line.text, "");
}

#[test]
fn map_pending_keys_are_dropped_by_a_mode_change_and_focus_loss() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "jk", ":refresh<CR>")"#);
    tap(&mut k, KeyInput::ch('j'), t);
    k.set_key_mode(KeyMode::Input);
    assert_eq!(k.map_wait(t), None);
    assert_eq!(k.map_tick(t + TIMEOUT), None);
    assert!(effects(&mut k).is_empty());
    k.set_key_mode(KeyMode::Normal);
    tap(&mut k, KeyInput::ch('j'), t);
    k.release_all();
    assert_eq!(k.map_wait(t), None);
    assert!(effects(&mut k).is_empty());
}

#[test]
fn map_only_in_normal_not_in_lines_or_input() {
    let t = Instant::now();
    let mut k = shell(r#"web.map("n", "Q", ":refresh<CR>")"#);
    tap(&mut k, KeyInput::ch(':'), t);
    tap(&mut k, KeyInput::ch('Q'), t);
    assert_eq!(k.line.text, "Q");
    assert!(effects(&mut k).is_empty());
    let mut k = shell(r#"web.map("n", "Q", ":refresh<CR>")"#);
    k.set_key_mode(KeyMode::Input);
    assert_eq!(tap(&mut k, KeyInput::ch('Q'), t), (true, true), "INPUT: the page's");
    assert!(effects(&mut k).is_empty());
}

// ---------- no regression: the built-in keys with and without mappings ----------

/// Mappings that the test streams below never complete. `gZ` and `yQ` share their first key with a
/// built-in sequence, so a `g` or `y` waits for the mapping and is replayed (a key that does
/// something on its own, `j` say, would be delayed by such a mapping by design: covered above).
const UNRELATED: &str = r#"
    web.map("n", "gZ", ":refresh<CR>");
    web.map("n", "yQ", ":refresh<CR>");
    web.map("n", "Q", ":refresh<CR>");
    web.map("n", "<C-x>", ":refresh<CR>");
    web.map("n", "zz", ":go ");
"#;

/// What one key event did: the page got it, the effects, and the state after.
#[derive(Debug, PartialEq)]
struct Outcome {
    forward: bool,
    effects: Vec<Effect>,
    key_mode: KeyMode,
    vim_mode: VimMode,
    message: String,
    line: String,
}

/// Feeds `events` (key, pressed, ms) to `k`, firing the mapping timer when the compositor would
/// (its effects are added to the next event's, as they come before it).
fn run(k: &mut Keys, events: &[(KeyInput, bool, u64)], t0: Instant) -> (Vec<Outcome>, usize) {
    let mut out = Vec::new();
    let mut waits = 0;
    let mut carried = Vec::new();
    for (key, pressed, at) in events {
        let now = ms(t0, *at);
        if let Some(w) = k.map_wait(now) {
            if w.is_zero() {
                k.map_tick(now);
                carried.extend(effects(k));
            }
        }
        let forward = k.key(key.sym.raw(), *pressed, key, now);
        let mut e = std::mem::take(&mut carried);
        e.extend(effects(k));
        out.push(Outcome {
            forward,
            effects: e,
            key_mode: k.key_mode,
            vim_mode: k.vim_mode,
            message: k.message.clone(),
            line: k.line.text.clone(),
        });
        waits += usize::from(k.map_wait(now).is_some());
    }
    (out, waits)
}

/// Returns how often the mapped shell was waiting for a mapping after an event.
fn same_with_and_without_mappings(events: &[(KeyInput, bool, u64)]) -> usize {
    let t0 = Instant::now();
    let mut k = keys();
    k.url = "https://a.test/".into();
    let (plain, _) = run(&mut k, events, t0);
    let (no_maps, _) = run(&mut shell(r#"web.command("Hi", () => {});"#), events, t0);
    let (mapped, waits) = run(&mut shell(UNRELATED), events, t0);
    for (i, ((p, n), m)) in plain.iter().zip(&no_maps).zip(&mapped).enumerate() {
        assert_eq!(p, n, "event {i} {:?}: a script without mappings changed it", events[i]);
        assert_eq!(p, m, "event {i} {:?}: unrelated mappings changed it", events[i]);
    }
    waits
}

fn taps(s: &str, start: u64, gap: u64) -> Vec<(KeyInput, bool, u64)> {
    let mut v = Vec::new();
    for (i, c) in s.chars().enumerate() {
        let k = KeyInput::ch(c);
        let at = start + i as u64 * gap;
        v.push((k, true, at));
        v.push((k, false, at + 1));
    }
    v
}

#[test]
fn map_no_regression_gg_gi_yy_g_other() {
    for gap in [5, 400, MAP_TIMEOUT_MS + 1, 3000] {
        for s in ["gg", "gi", "yy", "gj", "gxg", "gyy", "ygg", "g:", "g/", "jjk", "hl", "G", "nN"] {
            same_with_and_without_mappings(&taps(s, 0, gap));
        }
    }
}

#[test]
fn map_no_regression_held_scroll_key_and_ctrl_caret() {
    let j = KeyInput::ch('j');
    // j held (OS repeats), then released; then Ctrl-^ and Ctrl-6, Ctrl-d / Ctrl-u.
    let mut ev = vec![(j, true, 0), (j, true, 600), (j, true, 650), (j, false, 2000)];
    for (i, k) in [
        KeyInput::ch('^').ctrl(),
        KeyInput::ch('6').ctrl(),
        KeyInput::ch('d').ctrl(),
        KeyInput::ch('u').ctrl(),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 3000 + 10 * i as u64;
        ev.push((k, true, at));
        ev.push((k, false, at + 1));
    }
    same_with_and_without_mappings(&ev);
    // j pressed, held past the timeout with no repeat, then released.
    same_with_and_without_mappings(&[(j, true, 0), (j, false, 2500)]);
}

#[test]
fn map_no_regression_esc_esc_and_input_mode() {
    let esc = KeyInput::sym(Keysym::Escape);
    let mut ev = taps("i", 0, 10);
    // In INPUT: Esc Esc within ESC_ESC_MS, a slow pair, mapped keys go to the page.
    ev.extend([(esc, true, 100), (esc, false, 101)]);
    ev.extend(taps("Qg", 200, 10));
    ev.extend([(esc, true, 300), (esc, false, 301), (esc, true, 500), (esc, false, 501)]);
    ev.extend(taps("i", 600, 10));
    ev.extend([(esc, true, 700), (esc, false, 701), (esc, true, 1100), (esc, false, 1101)]);
    ev.extend([(esc, true, 1150), (esc, false, 1151)]);
    // Back in NORMAL: g then Esc (clears the search, not gg), Enter goes to the page, `:` and `/`.
    ev.extend(taps("g", 1300, 10));
    ev.extend([(esc, true, 1400), (esc, false, 1401)]);
    let ret = KeyInput::sym(Keysym::Return);
    ev.extend(taps("g", 1500, 10));
    ev.extend([(ret, true, 1600), (ret, false, 1601)]);
    ev.extend(taps(":hl", 1700, 10));
    ev.extend([(esc, true, 1800), (esc, false, 1801)]);
    ev.extend(taps("/gg", 1900, 10));
    ev.extend([(esc, true, 2000), (esc, false, 2001)]);
    same_with_and_without_mappings(&ev);
}

#[test]
fn map_no_regression_a_pseudo_random_stream() {
    // Keys the mappings above share a start with or that the built-ins use; never `Z`, `Q`, `z`,
    // Ctrl-x, so no mapping completes. `:` and `/` open a line, Esc and Enter leave it; now and
    // then `i`, a key for the page and Esc Esc (so the stream stays mostly in NORMAL).
    let alphabet: Vec<KeyInput> = vec![
        KeyInput::ch('g'),
        KeyInput::ch('g'),
        KeyInput::ch('y'),
        KeyInput::ch('j'),
        KeyInput::ch('j'),
        KeyInput::ch('k'),
        KeyInput::ch('h'),
        KeyInput::ch('G'),
        KeyInput::ch(':'),
        KeyInput::ch('/'),
        KeyInput::ch('x'),
        KeyInput::ch('^').ctrl(),
        KeyInput::sym(Keysym::Escape),
        KeyInput::sym(Keysym::Escape),
        KeyInput::sym(Keysym::Return),
        KeyInput::sym(Keysym::Shift_L),
        KeyInput::sym(Keysym::Down),
    ];
    let mut waited = 0;
    for seed in 1..=40u64 {
        let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut rnd = |n: u64| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 33) % n
        };
        let mut ev = Vec::new();
        let mut at = 0;
        let mut down: Vec<KeyInput> = Vec::new();
        for _ in 0..200 {
            at += [0, 5, 50, 290, 310, 700, 1001, 2000][rnd(8) as usize];
            if !down.is_empty() && rnd(3) == 0 {
                let k = down.remove(rnd(down.len() as u64) as usize);
                ev.push((k, false, at));
                continue;
            }
            if rnd(25) == 0 {
                let esc = KeyInput::sym(Keysym::Escape);
                for (i, k) in [KeyInput::ch('i'), KeyInput::ch('j'), esc, esc].into_iter().enumerate() {
                    ev.push((k, true, at + 10 * i as u64));
                    ev.push((k, false, at + 10 * i as u64 + 1));
                }
                at += 50;
                continue;
            }
            let k = alphabet[rnd(alphabet.len() as u64) as usize];
            ev.push((k, true, at)); // a press of a held key is an OS repeat
            if down.contains(&k) {
                continue;
            }
            // Mostly typed (released at once), now and then held across the next events.
            if rnd(4) == 0 {
                down.push(k);
            } else {
                at += 1;
                ev.push((k, false, at));
            }
        }
        for k in down {
            ev.push((k, false, at + 1));
        }
        waited += same_with_and_without_mappings(&ev);
    }
    assert!(waited > 200, "the streams made the mapped shell wait {waited} times");
}
