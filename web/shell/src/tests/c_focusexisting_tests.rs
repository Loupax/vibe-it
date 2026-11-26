//! C-FOCUSEXISTING (KEYS.md T170, SCRIPTING.md `web.buffers.focusExisting`): a plain click on a
//! `target=_blank` link (`blank-link`) to a URL a buffer already shows switches to that buffer when the
//! setting is on; a middle-click (`open-link`) always opens a new buffer.

use std::path::PathBuf;
use std::time::Duration;

use crate::buffers::TabCreated;
use crate::config::SCRIPT_BUDGET_MS;
use crate::keys::*;
use crate::protocol::{Command, Event};
use crate::script::init_files;

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);

/// Buffer 1 (`https://a.test/one`, "Page one", tab 1) and buffer 2 (`https://blue.example/two`,
/// "Blue page", tab 2), 2 current; `init` run as the init file first ("" for none).
fn two_buffers(init: &str) -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    if !init.is_empty() {
        assert!(k.load_script_source("init.js", init, BUDGET), "{}", k.message);
    }
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/one".into();
    k.title = "Page one".into();
    k.open_buffer("https://blue.example/two");
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    k.url = "https://blue.example/two".into();
    k.title = "Blue page".into();
    k.effects.clear();
    k.message.clear();
    k
}

const ON: &str = "web.buffers.focusExisting = true;";

fn new_tab_urls(k: &Keys) -> Vec<Option<String>> {
    k.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(Command::NewTab { url, .. }) => Some(url.clone()),
            _ => None,
        })
        .collect()
}

fn shown_tabs(k: &Keys) -> Vec<u64> {
    k.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(Command::ShowTab { tab }) => Some(*tab),
            _ => None,
        })
        .collect()
}

/// A new buffer 3 for `url`, current, 2 the alternate.
fn assert_new_buffer(k: &Keys, url: &str) {
    assert_eq!(k.buffers.list.len(), 3);
    assert_eq!(k.buffers.current, 3);
    assert_eq!(k.buffers.alternate, Some(2));
    assert_eq!(new_tab_urls(k), vec![Some(url.to_string())]);
    assert!(!k.message.starts_with("already open"), "{}", k.message);
}

#[test]
fn t170_setting_off_a_blank_link_opens_a_new_buffer() {
    let mut k = two_buffers("");
    k.set_key_mode(KeyMode::Input);
    assert!(k.link_click(2, "https://a.test/one", true));
    assert_new_buffer(&k, "https://a.test/one");
    assert_eq!(k.key_mode, KeyMode::Normal);
    // A script that leaves the setting alone: off too.
    let mut k = two_buffers("web.buffers.deleteConfirmation = false;");
    assert!(k.link_click(2, "https://a.test/one", true));
    assert_new_buffer(&k, "https://a.test/one");
}

#[test]
fn t170_setting_on_no_buffer_has_the_url_a_new_buffer() {
    let mut k = two_buffers(ON);
    assert!(k.link_click(2, "https://c.test/", true));
    assert_new_buffer(&k, "https://c.test/");
}

#[test]
fn t170_setting_on_a_match_switches_to_it_sets_the_alternate_and_says_so() {
    let mut k = two_buffers(ON);
    k.set_key_mode(KeyMode::Input);
    assert!(k.link_click(2, "https://a.test/one", true));
    assert_eq!(k.buffers.list.len(), 2, "no new buffer");
    assert_eq!(k.buffers.current, 1);
    assert_eq!(k.buffers.alternate, Some(2), "Ctrl-^ returns");
    assert_eq!(k.message, "already open: [1] Page one");
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(new_tab_urls(&k).is_empty());
    assert_eq!(shown_tabs(&k), vec![1]);
    // The buffer left kept its state.
    assert_eq!(k.buffers.get(2).unwrap().url, "https://blue.example/two");
    assert_eq!(k.url, "https://a.test/one");
    // Ctrl-^ goes back.
    k.buffer_alternate();
    assert_eq!(k.buffers.current, 2);
}

#[test]
fn t170_a_match_with_no_title_is_named_by_its_url() {
    let mut k = two_buffers(ON);
    k.buffers.get_mut(1).unwrap().title.clear();
    assert!(k.link_click(2, "https://a.test/one", true));
    assert_eq!(k.message, "already open: [1] https://a.test/one");
}

#[test]
fn t170_two_matches_take_the_lowest_number() {
    let mut k = two_buffers(ON);
    // Buffer 3 shows the same page as buffer 2.
    k.open_buffer("https://blue.example/two");
    assert_eq!(k.buffers.tab_created(3, Some(2)), TabCreated::Buffer(3));
    k.url = "https://blue.example/two".into();
    k.title = "Blue again".into();
    // Buffer 4 is on screen and links to it.
    k.open_buffer("https://d.test/");
    assert_eq!(k.buffers.tab_created(4, Some(3)), TabCreated::Buffer(4));
    k.url = "https://d.test/".into();
    k.effects.clear();
    assert!(k.link_click(4, "https://blue.example/two", true));
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.buffers.alternate, Some(4));
    assert_eq!(k.message, "already open: [2] Blue page");
}

#[test]
fn t170_the_match_is_the_buffer_on_screen_only_the_message() {
    let mut k = two_buffers(ON);
    assert!(k.link_click(2, "https://blue.example/two", true));
    assert_eq!(k.buffers.list.len(), 2);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.buffers.alternate, Some(1), "the alternate is untouched");
    assert!(k.effects.is_empty(), "{:?}", k.effects);
    assert_eq!(k.message, "already open: [2] Blue page");
}

#[test]
fn t170_only_an_exact_url_matches() {
    for url in [
        "https://a.test/one#top",
        "https://a.test/one/",
        "https://a.test/one?",
        "http://a.test/one",
        "https://A.test/one",
    ] {
        let mut k = two_buffers(ON);
        assert!(k.link_click(2, url, true));
        assert_new_buffer(&k, url);
    }
    // The other way round: the buffer has the fragment, the link not.
    let mut k = two_buffers(ON);
    k.buffers.get_mut(1).unwrap().url = "https://a.test/one#top".into();
    assert!(k.link_click(2, "https://a.test/one", true));
    assert_new_buffer(&k, "https://a.test/one");
}

#[test]
fn t170_the_match_uses_the_live_url_of_the_buffer_on_screen() {
    // Buffer 2 navigated since it became current: its URL lives in `Keys` until it is left.
    let mut k = two_buffers(ON);
    k.url = "https://blue.example/three".into();
    assert!(k.link_click(2, "https://blue.example/three", true));
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.message, "already open: [2] Blue page");
    let mut k = two_buffers(ON);
    k.url = "https://blue.example/three".into();
    assert!(k.link_click(2, "https://blue.example/two", true));
    assert_new_buffer(&k, "https://blue.example/two");
}

#[test]
fn t105_a_middle_click_ignores_the_setting() {
    let mut k = two_buffers(ON);
    assert!(k.link_click(2, "https://a.test/one", false));
    assert_new_buffer(&k, "https://a.test/one");
    let mut k = two_buffers(ON);
    assert!(k.link_click(2, "https://blue.example/two", false));
    assert_new_buffer(&k, "https://blue.example/two");
}

#[test]
fn t170_a_click_from_a_page_not_on_screen_is_ignored() {
    for blank in [true, false] {
        let mut k = two_buffers(ON);
        k.set_key_mode(KeyMode::Input);
        assert!(!k.link_click(1, "https://blue.example/two", blank));
        assert!(!k.link_click(9, "https://c.test/", blank));
        assert_eq!(k.buffers.list.len(), 2);
        assert_eq!(k.buffers.current, 2);
        assert!(k.effects.is_empty());
        assert!(k.message.is_empty());
        assert_eq!(k.key_mode, KeyMode::Input, "the mode is untouched");
    }
}

#[test]
fn t170_the_setting_is_false_by_default_and_reads_back() {
    let mut k = Keys::new(None, String::new(), None);
    assert!(
        k.load_script_source(
            "init.js",
            "if (web.buffers.focusExisting !== false) throw new Error('default');\n\
             web.buffers.focusExisting = true;\n\
             if (web.buffers.focusExisting !== true) throw new Error('set');",
            BUDGET,
        ),
        "{}",
        k.message
    );
    assert!(k.scripts.as_ref().unwrap().focus_existing());
}

#[test]
fn t170_a_non_boolean_is_a_script_error_and_keeps_the_value() {
    for v in ["1", "\"yes\"", "null", "undefined", "{}"] {
        let mut k = Keys::new(None, String::new(), None);
        let src = format!("web.buffers.focusExisting = true;\nweb.buffers.focusExisting = {v};");
        assert!(!k.load_script_source("init.js", &src, BUDGET), "{v}");
        assert_eq!(k.message, "init.js:2: web.buffers.focusExisting must be true or false", "{v}");
        assert!(k.scripts.as_ref().unwrap().focus_existing(), "{v}: keeps its value");
    }
}

/// A config directory of its own (`XDG_CONFIG_HOME` for `init_files`), removed by name at the end.
struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("web-c-focusexisting-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web/profiles/default")).unwrap();
        Config(dir)
    }

    fn init(&self, src: &str) {
        std::fs::write(self.0.join("web/profiles/default/init.js"), src).unwrap();
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn t170_source_puts_the_setting_back_to_false() {
    let c = Config::new("source");
    c.init(ON);
    let home = c.0.display().to_string();
    let files = init_files(&move |k| (k == "XDG_CONFIG_HOME").then(|| home.clone()), "default");
    let mut k = Keys::new(None, String::new(), None);
    k.load_scripts_with(&files, BUDGET);
    assert!(k.scripts.as_ref().unwrap().focus_existing());
    // The file no longer sets it: after :source it is false again (a new runtime, the default).
    c.init("// nothing\n");
    k.source();
    assert!(!k.scripts.as_ref().unwrap().focus_existing());
    // The file sets it again: true again.
    c.init(ON);
    k.source();
    assert!(k.scripts.as_ref().unwrap().focus_existing());
}

// C-FOCUSEXISTING-REDIRECT: a buffer also matches the URL its page was requested as before redirects.

fn url_event(tab: u64, url: &str, original: Option<&str>) -> Event {
    Event::Url {
        tab,
        url: url.into(),
        original: original.map(str::to_string),
    }
}

#[test]
fn t170_redirect_a_background_buffer_matches_its_original_url() {
    let mut k = two_buffers(ON);
    // Buffer 1 is in the background; its `url` event arrives with the original.
    assert!(k.page_event(1, &url_event(1, "https://a.test/pull/7", Some("https://a.test/issues/7"))));
    assert!(k.link_click(2, "https://a.test/issues/7", true));
    assert_eq!(k.buffers.list.len(), 2, "no new buffer");
    assert_eq!(k.buffers.current, 1);
    assert_eq!(k.buffers.alternate, Some(2));
    assert_eq!(k.message, "already open: [1] Page one");
    assert!(new_tab_urls(&k).is_empty());
    // The current URL still matches too.
    let mut k = two_buffers(ON);
    assert!(k.page_event(1, &url_event(1, "https://a.test/pull/7", Some("https://a.test/issues/7"))));
    assert!(k.link_click(2, "https://a.test/pull/7", true));
    assert_eq!(k.buffers.current, 1);
}

#[test]
fn t170_redirect_the_current_buffer_matches_only_the_message() {
    let mut k = two_buffers(ON);
    assert!(!k.page_event(2, &url_event(2, "https://blue.example/b", Some("https://blue.example/a"))));
    k.original_url = Some("https://blue.example/a".into()); // what state.rs does on the event
    assert!(k.link_click(2, "https://blue.example/a", true));
    assert_eq!(k.buffers.list.len(), 2);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.buffers.alternate, Some(1));
    assert!(k.effects.is_empty(), "{:?}", k.effects);
    assert_eq!(k.message, "already open: [2] Blue page");
}

#[test]
fn t170_redirect_a_later_url_event_without_original_clears_it() {
    let mut k = two_buffers(ON);
    assert!(k.page_event(1, &url_event(1, "https://a.test/pull/7", Some("https://a.test/issues/7"))));
    assert!(k.page_event(1, &url_event(1, "https://a.test/next", None)));
    assert!(k.link_click(2, "https://a.test/issues/7", true));
    assert_new_buffer(&k, "https://a.test/issues/7");
    // The current buffer: the live value is replaced the same way.
    let mut k = two_buffers(ON);
    k.original_url = Some("https://blue.example/a".into());
    k.original_url = None;
    assert!(k.link_click(2, "https://blue.example/a", true));
    assert_new_buffer(&k, "https://blue.example/a");
}

#[test]
fn t170_redirect_switching_away_and_back_keeps_the_original() {
    let mut k = two_buffers(ON);
    k.original_url = Some("https://blue.example/a".into());
    k.switch_to(1);
    assert_eq!(k.original_url, None);
    assert_eq!(k.buffers.get(2).unwrap().original_url.as_deref(), Some("https://blue.example/a"));
    k.switch_to(2);
    assert_eq!(k.original_url.as_deref(), Some("https://blue.example/a"));
    k.effects.clear();
    assert!(k.link_click(2, "https://blue.example/a", true));
    assert_eq!(k.message, "already open: [2] Blue page");
    assert_eq!(k.buffers.list.len(), 2);
}

#[test]
fn t170_redirect_the_first_in_ls_order_wins() {
    let mut k = two_buffers(ON);
    // Buffer 1 matches by its original URL, buffer 3 by its current URL; 1 comes first.
    assert!(k.page_event(1, &url_event(1, "https://a.test/x", Some("https://a.test/target"))));
    k.open_buffer("https://c.test/");
    assert_eq!(k.buffers.tab_created(3, Some(2)), TabCreated::Buffer(3));
    k.url = "https://a.test/target".into();
    k.effects.clear();
    assert!(k.link_click(3, "https://a.test/target", true));
    assert_eq!(k.buffers.current, 1);
    assert_eq!(k.message, "already open: [1] Page one");
}

#[test]
fn t105_redirect_a_middle_click_still_opens_a_new_buffer() {
    let mut k = two_buffers(ON);
    assert!(k.page_event(1, &url_event(1, "https://a.test/pull/7", Some("https://a.test/issues/7"))));
    assert!(k.link_click(2, "https://a.test/issues/7", false));
    assert_new_buffer(&k, "https://a.test/issues/7");
}
