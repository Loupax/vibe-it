//! C-NEWBUF (KEYS.md "Buffers", T103-T104). Each test names its row.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::completion::{compute_ghost, CompleteCache};
use crate::keys::*;
use crate::protocol::Command;

const A: &str = "https://a.test/one";

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

/// Runs `:line` as typed (Delete hides a ghost) and returns the commands it sent.
fn run(k: &mut Keys, line: &str) -> Vec<Command> {
    k.effects.clear();
    press(k, KeyInput::ch(':'));
    for c in line.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn one_buffer() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = A.into();
    k.title = "Page one".into();
    k.effects.clear();
    k
}

#[test]
fn c_newbuf_t103_new_url_opens_a_buffer_like_go_and_shows_it() {
    for name in ["new", "bufferNew"] {
        let mut k = one_buffer();
        let sent = run(&mut k, &format!("{name} example.com/x"));
        assert_eq!(
            sent,
            vec![Command::NewTab {
                req: 1,
                url: Some("https://example.com/x".into())
            }]
        );
        assert_eq!(k.buffers.current, 2, "{name}");
        assert_eq!(k.buffers.alternate, Some(1), "the old buffer is Ctrl-^'s");
        assert_eq!(k.url, "https://example.com/x");
        assert_eq!(k.buffers.get(1).unwrap().url, A, "buffer 1 keeps its page");
        assert_eq!(k.vim_mode, VimMode::Normal);
        assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    }
    // Relative words resolve against the page on screen, as :go does.
    let mut k = one_buffer();
    let sent = run(&mut k, "new /two");
    assert_eq!(
        sent,
        vec![Command::NewTab {
            req: 1,
            url: Some("https://a.test/two".into())
        }]
    );
    // Ctrl-^ goes back.
    k.effects.clear();
    k.buffer_alternate();
    assert_eq!(k.buffers.current, 1);
}

#[test]
fn c_newbuf_t103_a_bad_word_or_two_words_change_nothing() {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    assert!(run(&mut k, "new /x").is_empty(), "relative with no page, as :go");
    assert_eq!(k.buffers.list.len(), 1);
    assert!(!k.message.is_empty());
    assert!(run(&mut k, "new a b").is_empty());
    assert_eq!(k.message, "web: usage: :new [url]");
    assert_eq!(k.buffers.list.len(), 1);
}

#[test]
fn c_newbuf_t104_new_without_an_argument_opens_blank_and_the_go_prompt() {
    let mut k = one_buffer();
    let sent = run(&mut k, "new");
    assert_eq!(sent, vec![Command::NewTab { req: 1, url: None }]);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.buffers.alternate, Some(1));
    assert_eq!(k.vim_mode, VimMode::Command, "the : line is open");
    assert_eq!(k.line.text, "go ");
    assert_eq!(k.line.cursor, 3);
    // Typing the address and Enter runs :go in the new buffer.
    for c in "b.test".chars() {
        press(&mut k, KeyInput::ch(c));
    }
    press(&mut k, KeyInput::sym(Keysym::Delete));
    k.effects.clear();
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(k.effects.contains(&Effect::Send(Command::Open {
        url: "https://b.test".into()
    })));
    assert_eq!(k.buffers.list.len(), 2, ":go navigates, it opens no third buffer");
}

#[test]
fn c_newbuf_t90_the_names_are_offered_and_complete_like_go() {
    assert!(COMMAND_NAMES.contains(&"new"));
    assert!(COMMAND_NAMES.contains(&"bufferNew"));
    let cache = CompleteCache::new(None);
    let g = compute_ghost("bufferNe", 8, &cache).unwrap();
    assert!(g.candidates.iter().any(|c| c == "bufferNew"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort();
    assert_eq!(sorted, COMMAND_NAMES, "kept sorted");
    for n in ["new", "bufferNew"] {
        assert!(crate::config::COMPLETERS.iter().any(|(c, _)| *c == n), "{n}");
    }
}
