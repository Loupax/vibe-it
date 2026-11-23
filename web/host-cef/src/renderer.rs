//! Renderer-process side: CEF tells the renderer when the focused DOM node changes;
//! whether it is editable goes to the browser process as a process message, which
//! page.rs turns into `editable-focused` events.

use cef::*;

pub const EDITABLE_MSG: &str = "web-host.editable";

wrap_render_process_handler! {
    pub struct HostRenderProcessHandler {}

    impl RenderProcessHandler {
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
