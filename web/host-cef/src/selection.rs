//! C-YANK (KEYS.md T54, `:yank`; PROTOCOL.md `selection`): the shell asks for the page's selected
//! text and writes it to the clipboard register `+`.
//!
//! Renderer: every script context of every frame gets selection.js before any page script
//! (`install`, renderer.rs step 6). It returns a function, which the renderer keeps per frame (never
//! on `window`: pages cannot call it or fake its answer). The browser process sends `SELECTION_MSG`
//! with the request number to a frame; the renderer runs the frame's function and answers with
//! `SELECTION_RESULT_MSG` (request, text), which becomes a `selection` event.
//!
//! Browser: which frame. The page's focused frame (the iframe clicked into), else the main frame.

use std::cell::RefCell;

use cef::*;

use crate::protocol::{Event, TabId};

/// Browser -> renderer: argument 0 is the request number (an int).
pub const SELECTION_MSG: &str = "web-host.selection";
/// Renderer -> browser: argument 0 the request number, argument 1 the selected text.
pub const SELECTION_RESULT_MSG: &str = "web-host.selection-result";

const SELECTION_JS: &str = include_str!("selection.js");

thread_local! {
    /// Renderer: per frame (CEF's frame identifier), its current context and selection.js's function.
    static SELECTIONS: RefCell<Vec<(String, V8Context, V8Value)>> = const { RefCell::new(Vec::new()) };
}

fn frame_id(frame: &Frame) -> String {
    CefString::from(&frame.identifier()).to_string()
}

/// The request number travels as a CEF int (32 bits); the shell's numbers count up from 1, so
/// wrapping through u32 keeps every number below 2^32 intact.
fn req_to_int(req: u64) -> i32 {
    req as u32 as i32
}

fn req_from_int(i: i32) -> u64 {
    i as u32 as u64
}

/// Renderer, OnContextCreated: selection.js in this context; its function replaces the frame's old one.
pub fn install(frame: &Frame, context: &mut V8Context) {
    let Some(f) = crate::renderer::call_with(context, "selection", SELECTION_JS, &[]).filter(|f| f.is_function() != 0) else {
        return;
    };
    let id = frame_id(frame);
    SELECTIONS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(i, _, _)| *i != id);
        v.push((id, context.clone(), f));
    });
}

/// Renderer, OnContextReleased: forget that context's function.
pub fn released(context: &mut V8Context) {
    SELECTIONS.with(|v| v.borrow_mut().retain(|(_, c, _)| c.is_same(Some(&mut context.clone())) == 0));
}

/// Renderer: a `SELECTION_MSG` for `frame`. Returns false for other messages.
pub fn renderer_message(frame: Option<&mut Frame>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != SELECTION_MSG {
        return false;
    }
    let Some(frame) = frame else { return true };
    let req = msg.argument_list().map(|a| a.int(0)).unwrap_or(0);
    let id = frame_id(frame);
    let found = SELECTIONS.with(|v| v.borrow().iter().find(|(i, _, _)| *i == id).map(|(_, c, f)| (c.clone(), f.clone())));
    let text = match found {
        Some((mut context, f)) => f
            .execute_function_with_context(Some(&mut context), None, Some(&[]))
            .filter(|r| r.is_string() != 0)
            .map(|r| CefString::from(&r.string_value()).to_string())
            .unwrap_or_default(),
        None => String::new(),
    };
    if let Some(mut reply) = process_message_create(Some(&CefString::from(SELECTION_RESULT_MSG))) {
        if let Some(a) = reply.argument_list() {
            a.set_int(0, req);
            a.set_string(1, Some(&CefString::from(text.as_str())));
        }
        frame.send_process_message(ProcessId::BROWSER, Some(&mut reply));
    }
    true
}

/// Browser: asks `browser`'s focused frame (else its main frame) for its selection. No frame at
/// all: an empty `selection` at once.
pub fn request(tab: TabId, browser: &Browser, req: u64) {
    let frame = browser.focused_frame().or_else(|| browser.main_frame());
    let msg = process_message_create(Some(&CefString::from(SELECTION_MSG)));
    let (Some(frame), Some(mut msg)) = (frame, msg) else {
        return crate::control::emit(Event::Selection {
            tab,
            req,
            text: String::new(),
        });
    };
    if let Some(a) = msg.argument_list() {
        a.set_int(0, req_to_int(req));
    }
    frame.send_process_message(ProcessId::RENDERER, Some(&mut msg));
}

/// Browser: a `SELECTION_RESULT_MSG` from page `tab`. Returns false for other messages.
pub fn result_message(tab: Option<TabId>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != SELECTION_RESULT_MSG {
        return false;
    }
    let (Some(tab), Some(a)) = (tab, msg.argument_list()) else {
        return true;
    };
    crate::control::emit(Event::Selection {
        tab,
        req: req_from_int(a.int(0)),
        text: CefString::from(&a.string(1)).to_string(),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_survives_the_int() {
        for req in [0u64, 1, 7, i32::MAX as u64, i32::MAX as u64 + 1, u32::MAX as u64] {
            assert_eq!(req_from_int(req_to_int(req)), req);
        }
    }
}
