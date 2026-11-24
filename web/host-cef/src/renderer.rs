//! Renderer-process side: CEF tells the renderer when the focused DOM node changes;
//! whether it is editable goes to the browser process as a process message, which
//! page.rs turns into `editable-focused` events.

use cef::*;

pub const EDITABLE_MSG: &str = "web-host.editable";

wrap_render_process_handler! {
    pub struct HostRenderProcessHandler {}

    impl RenderProcessHandler {
        // C-NOTAB: see newtab::CTRL_SHIFT_JS.
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            // C-BELL spike, off unless WEB_HOST_NOTIFY_SPIKE=shim.
            if let (Some("shim"), Some(c)) = (crate::notify_spike::mode().as_deref(), context) {
                crate::notify_spike::install_shim(c);
            }
            if let Some(f) = frame {
                f.execute_java_script(Some(&CefString::from(crate::newtab::CTRL_SHIFT_JS)), None, 0);
            }
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
