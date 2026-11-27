//! Renderer-process side: CEF tells the renderer when the focused DOM node changes;
//! whether it is editable goes to the browser process as a process message, which
//! page.rs turns into `editable-focused` events. A middle-click on a link (C-MIDCLICK) goes the
//! same way (`MIDDLE_MSG`, page.rs turns it into `open-link`), and a plain left-click on a
//! `target=_blank` link (C-FOCUSEXISTING, `BLANK_MSG`, `blank-link`); so does a page's notification
//! (C-BELL, notify.rs). `on_context_created` is where every injected script starts. C-AUTOFOCUS
//! (field.rs) also answers the browser's `web-host.field` messages here, and C-YANK (selection.rs)
//! its `web-host.selection` messages.

use cef::*;

pub const EDITABLE_MSG: &str = "web-host.editable";
/// C-MIDCLICK: argument 0 is the absolute URL of the link that was middle-clicked.
pub const MIDDLE_MSG: &str = "web-host.middle-click";
/// C-FOCUSEXISTING: argument 0 is the absolute URL of the `target=_blank` link that was left-clicked.
pub const BLANK_MSG: &str = "web-host.blank-click";

wrap_v8_handler! {
    struct LinkClickHandler {
        msg: &'static str,
    }

    impl V8Handler {
        // Called by newtab::MIDDLE_JS / BLANK_JS with the link's URL; sent to the browser as `msg`.
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            _retval: Option<&mut Option<V8Value>>,
            _exception: Option<&mut CefString>,
        ) -> i32 {
            let Some(url) = arguments.and_then(|a| a.first()).and_then(|v| v.as_ref()).filter(|v| v.is_string() != 0) else {
                return 1;
            };
            let url = CefString::from(&url.string_value());
            let frame = v8_context_get_current_context().and_then(|c| c.frame());
            let (Some(frame), Some(mut msg)) = (frame, process_message_create(Some(&CefString::from(self.msg)))) else { return 1 };
            if let Some(args) = msg.argument_list() {
                args.set_string(0, Some(&url));
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
            1
        }
    }
}

/// Evaluates `js`, a function expression, in `context` (now, before any page script) and calls it
/// with `args`: native functions passed this way live only in the function's closure, so pages can
/// neither call nor replace them. `label` names the script in error lines. Returns what the call
/// returned (field.rs keeps the function its script returns).
pub fn call_with(context: &mut V8Context, label: &str, js: &str, args: &[Option<V8Value>]) -> Option<V8Value> {
    let (mut ret, mut exc) = (None, None);
    let url = CefString::from(format!("web-host:{label}").as_str());
    context.eval(Some(&CefString::from(js)), Some(&url), 1, Some(&mut ret), Some(&mut exc));
    if let Some(e) = exc {
        eprintln!("web-host-cef: {label} script: {}", CefString::from(&e.message()));
        return None;
    }
    let f = ret.filter(|f| f.is_function() != 0)?;
    let r = f.execute_function(None, Some(args));
    if r.is_none() {
        eprintln!("web-host-cef: {label} script threw");
    }
    r
}

/// C-MIDCLICK, C-FOCUSEXISTING: a link-click script (newtab::MIDDLE_JS, BLANK_JS) which takes its
/// function (a `LinkClickHandler` sending `msg`) from the global `fn_name` and deletes it at once.
fn link_click(context: &mut V8Context, label: &str, fn_name: &str, js: &str, msg: &'static str) {
    let Some(window) = context.global() else { return };
    let mut handler = LinkClickHandler::new(msg);
    let name = CefString::from(fn_name);
    let mut func = v8_value_create_function(Some(&name), Some(&mut handler));
    window.set_value_bykey(Some(&name), func.as_mut(), V8Propertyattribute::default());
    // eval, not execute_java_script: it runs now, in this context, before any page script.
    let (mut ret, mut exc) = (None, None);
    context.eval(Some(&CefString::from(js)), None, 0, Some(&mut ret), Some(&mut exc));
    if let Some(e) = exc {
        eprintln!("web-host-cef: {label} script: {}", CefString::from(&e.message()));
    }
}

wrap_render_process_handler! {
    pub struct HostRenderProcessHandler {}

    impl RenderProcessHandler {
        // The scripts every context of every frame gets when it is created, before any page script,
        // one step each. A new one (C-AUTOFOCUS) goes at the end, preferably through `call_with`.
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            let Some(context) = context else { return };
            // 1. C-BELL: the notification shim (notify.rs). It replaces `Notification` from a
            //    microtask, after Blink's own install (C-BELL-CRASH). Nothing here may define a
            //    global Blink installs later (Window's [SecureContext] / conditional interfaces):
            //    a non-configurable one aborts the renderer.
            crate::notify::install_shim(context);
            // 2. C-NOTAB: newtab::CTRL_SHIFT_JS (queued; it only adds a listener).
            if let Some(f) = &frame {
                f.execute_java_script(Some(&CefString::from(crate::newtab::CTRL_SHIFT_JS)), None, 0);
            }
            // 3. C-MIDCLICK: newtab::MIDDLE_JS.
            use crate::newtab::{BLANK_FN, BLANK_JS, MIDDLE_FN, MIDDLE_JS};
            link_click(context, "middle-click", MIDDLE_FN, MIDDLE_JS, MIDDLE_MSG);
            // 4. C-AUTOFOCUS: field.js (a focusin listener keeping the last text field; field.rs keeps
            //    the function the host's `blur-field` / `focus-field` run).
            if let Some(f) = &frame {
                crate::field::install(f, context);
            }
            // 5. C-FOCUSEXISTING: newtab::BLANK_JS (always: the shell decides what the click opens).
            link_click(context, "blank-click", BLANK_FN, BLANK_JS, BLANK_MSG);
            // 6. C-YANK: selection.js (no listener; selection.rs keeps the function `selection` runs).
            if let Some(f) = &frame {
                crate::selection::install(f, context);
            }
        }

        fn on_context_released(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            if let Some(context) = context {
                crate::field::released(context);
                crate::selection::released(context);
            }
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let (Some(m), mut frame) = (message, frame) else { return 0 };
            (crate::field::renderer_message(frame.as_deref_mut(), m) || crate::selection::renderer_message(frame, m)) as i32
        }

        fn on_focused_node_changed(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, node: Option<&mut Domnode>) {
            // node is None when focus left the page's elements (document or nothing focused).
            let editable = node.map(|n| n.is_editable() != 0).unwrap_or(false);
            let frame = match frame {
                Some(f) => Some(f.clone()),
                None => browser.and_then(|b| b.main_frame()),
            };
            let (Some(frame), Some(mut msg)) = (frame, process_message_create(Some(&CefString::from(EDITABLE_MSG)))) else { return };
            if let Some(args) = msg.argument_list() {
                args.set_bool(0, editable as i32);
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
        }
    }
}
