//! C-AUTOFOCUS (KEYS.md T29, T32, `gi`; PROTOCOL.md `blur-field`, `focus-field`): the shell parks a
//! field the page focused itself (the host blurs it), focuses it again on `i`, and `gi` focuses the
//! page's last text field or its first visible one.
//!
//! Renderer: every script context of every frame gets field.js before any page script
//! (`install`, renderer.rs step 4). It keeps the document's last focused text field and returns a
//! function, which the renderer keeps per frame (never on `window`: pages cannot call it). The browser
//! process sends `FIELD_MSG` with the operation to a frame; the renderer runs the frame's function and
//! answers with `FIELD_RESULT_MSG` (operation, ok), which becomes a `field-result` event.
//!
//! Browser: which frame. `blur` and `last` go to the frame of the last `editable-focused true` (the
//! frame the renderer's focus message came from), else the main frame; `first` to the main frame.

use std::cell::RefCell;

use cef::*;

use crate::protocol::{Event, FieldOp, TabId};

/// Browser -> renderer: argument 0 is the operation (`blur`, `last`, `first`).
pub const FIELD_MSG: &str = "web-host.field";
/// Renderer -> browser: argument 0 the operation, argument 1 whether it worked.
pub const FIELD_RESULT_MSG: &str = "web-host.field-result";

const FIELD_JS: &str = include_str!("field.js");

thread_local! {
    /// Renderer: per frame (CEF's frame identifier), its current context and field.js's function.
    static FIELDS: RefCell<Vec<(String, V8Context, V8Value)>> = const { RefCell::new(Vec::new()) };
}

fn frame_id(frame: &Frame) -> String {
    CefString::from(&frame.identifier()).to_string()
}

/// Renderer, OnContextCreated: field.js in this context; its function replaces the frame's old one.
pub fn install(frame: &Frame, context: &mut V8Context) {
    let Some(f) = crate::renderer::call_with(context, "field", FIELD_JS, &[]).filter(|f| f.is_function() != 0) else {
        return;
    };
    let id = frame_id(frame);
    FIELDS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(i, _, _)| *i != id);
        v.push((id, context.clone(), f));
    });
}

/// Renderer, OnContextReleased: forget that context's function.
pub fn released(context: &mut V8Context) {
    FIELDS.with(|v| v.borrow_mut().retain(|(_, c, _)| c.is_same(Some(&mut context.clone())) == 0));
}

/// Renderer: a `FIELD_MSG` for `frame`. Returns false for other messages.
pub fn renderer_message(frame: Option<&mut Frame>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != FIELD_MSG {
        return false;
    }
    let Some(frame) = frame else { return true };
    let op = msg
        .argument_list()
        .map(|a| CefString::from(&a.string(0)).to_string())
        .unwrap_or_default();
    let id = frame_id(frame);
    let found = FIELDS.with(|v| v.borrow().iter().find(|(i, _, _)| *i == id).map(|(_, c, f)| (c.clone(), f.clone())));
    let ok = match found {
        Some((mut context, f)) => {
            let arg = [v8_value_create_string(Some(&CefString::from(op.as_str())))];
            f.execute_function_with_context(Some(&mut context), None, Some(&arg))
                .is_some_and(|r| r.is_bool() != 0 && r.bool_value() != 0)
        }
        None => false,
    };
    if let Some(mut reply) = process_message_create(Some(&CefString::from(FIELD_RESULT_MSG))) {
        if let Some(a) = reply.argument_list() {
            a.set_string(0, Some(&CefString::from(op.as_str())));
            a.set_bool(1, ok as i32);
        }
        frame.send_process_message(ProcessId::BROWSER, Some(&mut reply));
    }
    true
}

/// Browser: asks `browser`'s frame `frame` (CEF's identifier; None or gone: the main frame) to run
/// `op`. No frame at all: a failed `field-result` at once.
pub fn request(tab: TabId, browser: &Browser, frame: Option<&str>, op: FieldOp) {
    let frame = frame
        .and_then(|f| browser.frame_by_identifier(Some(&CefString::from(f))))
        .or_else(|| browser.main_frame());
    let msg = process_message_create(Some(&CefString::from(FIELD_MSG)));
    let (Some(frame), Some(mut msg)) = (frame, msg) else {
        return crate::control::emit(Event::FieldResult { tab, op, ok: false });
    };
    if let Some(a) = msg.argument_list() {
        a.set_string(0, Some(&CefString::from(op.name())));
    }
    frame.send_process_message(ProcessId::RENDERER, Some(&mut msg));
}

/// Browser: a `FIELD_RESULT_MSG` from page `tab`. Returns false for other messages.
pub fn result_message(tab: Option<TabId>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != FIELD_RESULT_MSG {
        return false;
    }
    let (Some(tab), Some(a)) = (tab, msg.argument_list()) else {
        return true;
    };
    let op = CefString::from(&a.string(0)).to_string();
    match FieldOp::from_name(&op) {
        Some(op) => crate::control::emit(Event::FieldResult {
            tab,
            op,
            ok: a.bool(1) != 0,
        }),
        None => log!("field result for an unknown operation {op:?}: dropped"),
    }
    true
}
