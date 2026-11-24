//! C-BELL spike (web/notes/C-BELL-SPIKE.md): how can the host see a page's Web Notifications?
//! OFF unless `WEB_HOST_NOTIFY_SPIKE` is set (the renderer processes inherit it):
//!   log    the permission handler logs every permission prompt and keeps Chrome's default
//!   grant  ... and accepts notification prompts (the native route: Chrome then shows them itself)
//!   shim   the renderer replaces `Notification` and `ServiceWorkerRegistration.showNotification`
//!          in every frame at context creation (permission "granted") and forwards each
//!          notification's title/body/tag to the browser process, which logs it
//! Spike code only: not the design for step 2.

use cef::*;

pub const NOTIFY_MSG: &str = "web-host.notify";
const NOTIFICATIONS: u32 = 1 << 15; // CEF_PERMISSION_TYPE_NOTIFICATIONS

pub fn mode() -> Option<String> {
    std::env::var("WEB_HOST_NOTIFY_SPIKE").ok().filter(|m| !m.is_empty())
}

wrap_permission_handler! {
    pub struct SpikePermissionHandler {}

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let origin = requesting_origin.map(|o| o.to_string()).unwrap_or_default();
            let grant = mode().as_deref() == Some("grant") && requested_permissions == NOTIFICATIONS;
            log!("notify-spike: permission prompt {prompt_id} origin {origin} permissions {requested_permissions:#x} -> {}",
                if grant { "ACCEPT" } else { "default handling" });
            match (grant, callback) {
                (true, Some(cb)) => {
                    cb.cont(PermissionRequestResult::ACCEPT);
                    1
                }
                _ => 0,
            }
        }

        fn on_dismiss_permission_prompt(&self, _browser: Option<&mut Browser>, prompt_id: u64, result: PermissionRequestResult) {
            log!("notify-spike: permission prompt {prompt_id} dismissed, result {}", result.get_raw());
        }
    }
}

/// Runs in the page's context with the native `post(title, body, tag, kind)` as its argument;
/// nothing is left on the global object but the replaced APIs.
const SHIM_JS: &str = r#"(function (post) {
  'use strict';
  const S = v => (v === undefined || v === null) ? '' : String(v);
  class Notification extends EventTarget {
    constructor(title, options) {
      super();
      if (arguments.length < 1) throw new TypeError("Failed to construct 'Notification': 1 argument required, but only 0 present.");
      options = options || {};
      this.title = S(title); this.body = S(options.body); this.tag = S(options.tag);
      this.data = options.data === undefined ? null : options.data;
      this.onclick = this.onshow = this.onerror = this.onclose = null;
      post(this.title, this.body, this.tag, 'page');
      setTimeout(() => { const e = new Event('show'); this.dispatchEvent(e); if (this.onshow) this.onshow(e); }, 0);
    }
    close() { setTimeout(() => { const e = new Event('close'); this.dispatchEvent(e); if (this.onclose) this.onclose(e); }, 0); }
    static get permission() { return 'granted'; }
    static get maxActions() { return 2; }
    static requestPermission(cb) { if (typeof cb === 'function') setTimeout(() => cb('granted'), 0); return Promise.resolve('granted'); }
  }
  // Notification and ServiceWorkerRegistration are [SecureContext]: Blink installs them AFTER
  // OnContextCreated (seen: undefined here, and still no ServiceWorkerRegistration in a microtask).
  // Notification: a non-configurable property defined first, so Blink's later define of the real
  // interface object fails. ServiceWorkerRegistration: a page can only get a registration through
  // navigator.serviceWorker, so an own getter on this navigator patches the prototype on first use.
  Object.defineProperty(globalThis, 'Notification', {value: Notification, writable: false, configurable: false, enumerable: false});
  let patched = false;
  const patchSW = () => {
    if (patched || !globalThis.ServiceWorkerRegistration) return;
    patched = true;
    const P = ServiceWorkerRegistration.prototype;
    P.showNotification = function (title, options) { options = options || {}; post(S(title), S(options.body), S(options.tag), 'sw'); return Promise.resolve(); };
    P.getNotifications = function () { return Promise.resolve([]); };
  };
  if (globalThis.navigator) {
    Object.defineProperty(navigator, 'serviceWorker', {configurable: true, enumerable: true, get() {
      patchSW();
      const d = Object.getOwnPropertyDescriptor(Navigator.prototype, 'serviceWorker');
      return d && d.get ? d.get.call(this) : undefined;
    }});
  }
  if (globalThis.Permissions && Permissions.prototype.query) {
    const q = Permissions.prototype.query;
    Permissions.prototype.query = function (d) {
      if (d && d.name === 'notifications') return Promise.resolve({state: 'granted', name: 'notifications', onchange: null});
      return q.apply(this, arguments);
    };
  }
})"#;

wrap_v8_handler! {
    struct NotifyPost {}

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            _retval: Option<&mut Option<V8Value>>,
            _exception: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let args = arguments.unwrap_or(&[]);
            let s = |i: usize| -> String {
                args.get(i).and_then(|a| a.as_ref()).filter(|v| v.is_string() != 0)
                    .map(|v| CefString::from(&v.string_value()).to_string()).unwrap_or_default()
            };
            let Some(frame) = v8_context_get_current_context().and_then(|c| c.frame()) else { return 1 };
            let Some(mut msg) = process_message_create(Some(&CefString::from(NOTIFY_MSG))) else { return 1 };
            if let Some(list) = msg.argument_list() {
                for i in 0..4 {
                    list.set_string(i, Some(&CefString::from(s(i).as_str())));
                }
                list.set_string(4, Some(&CefString::from(&frame.url())));
                list.set_bool(5, frame.is_main());
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
            1
        }
    }
}

/// Renderer, OnContextCreated: replace the notification APIs in this context.
pub fn install_shim(context: &mut V8Context) {
    let t = std::time::Instant::now();
    let mut ret = None;
    let mut exc = None;
    context.eval(
        Some(&CefString::from(SHIM_JS)),
        Some(&CefString::from("web-host:notify-shim")),
        1,
        Some(&mut ret),
        Some(&mut exc),
    );
    let (Some(f), Some(post)) = (
        ret,
        v8_value_create_function(Some(&CefString::from("post")), Some(&mut NotifyPost::new())),
    ) else {
        return;
    };
    f.execute_function(None, Some(&[Some(post)]));
    log!(
        "notify-spike: shim installed in {} us (renderer pid {})",
        t.elapsed().as_micros(),
        std::process::id()
    );
}

/// Browser process: a forwarded notification. Returns false for other messages.
pub fn handle_message(tab: Option<i32>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != NOTIFY_MSG {
        return false;
    }
    let Some(a) = msg.argument_list() else { return true };
    let s = |i| CefString::from(&a.string(i)).to_string();
    log!(
        "notify-spike: NOTIFY tab {tab:?} kind {} title {:?} body {:?} tag {:?} frame {} (main {})",
        s(3),
        s(0),
        s(1),
        s(2),
        s(4),
        a.bool(5) != 0
    );
    true
}
