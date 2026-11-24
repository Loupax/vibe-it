//! C-BELL: a page's Web Notifications go to the shell, never to the desktop (web/notes/C-BELL-SPIKE.md).
//!
//! CEF shows the host only the permission prompt, never the notification (Chrome's runtime sends a
//! granted one straight to D-Bus). So:
//! - Renderer (`install_shim`, at every script context's creation): `SHIM_JS` replaces `Notification`,
//!   `ServiceWorkerRegistration.prototype.showNotification`/`getNotifications` and
//!   `permissions.query({name:'notifications'})`. Each notification goes to the browser process as the
//!   process message `NOTIFY_MSG` through a native function the shim holds in a closure (pages cannot
//!   reach it). Permission is "granted" on every site, or with `--notify-sites` only on the listed
//!   hostnames ("denied" elsewhere; the renderers get the list as the `SITES_SWITCH` switch).
//! - Browser process (`handle_message`): checks the site again, caps the lengths, rate-limits per page
//!   and emits the `notify` event (PROTOCOL.md).
//! - Native safety net (`HostPermissionHandler`, `block_native`): Chromium's own notification
//!   permission is BLOCK and every notification prompt is denied at once, so no Chrome bubble shows and
//!   Chrome's D-Bus bridge never pops a desktop notification, not even from a worker the shim does not
//!   reach (those are lost silently).

use cef::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::control::emit;
use crate::protocol::{Event, TabId};

pub const NOTIFY_MSG: &str = "web-host.notify";
/// The renderers' switch carrying `--notify-sites` (comma-separated hostnames); absent = every site.
pub const SITES_SWITCH: &str = "web-notify-sites";
/// CEF_PERMISSION_TYPE_NOTIFICATIONS.
const NOTIFICATIONS: u32 = 1 << 15;

/// Lengths the host passes on, in characters (longer ones end with `…`).
pub const MAX_TITLE: usize = 200;
pub const MAX_BODY: usize = 500;
pub const MAX_TAG: usize = 100;
pub const MAX_ORIGIN: usize = 200;
/// Rate limit per page: a burst of `BURST`, then one every `REFILL_MS`; the rest is dropped.
pub const BURST: f64 = 5.0;
pub const REFILL_MS: f64 = 2000.0;

/// Browser process: `--notify-sites` (None = every site). Set once in main.
pub static SITES: OnceLock<Option<Vec<String>>> = OnceLock::new();

/// `--notify-sites a.example,b.example` -> the hostnames, lowercased; "" = no site.
pub fn parse_sites(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The hostname of a serialized origin (`https://a.example:8443` -> `a.example`, `http://[::1]:80`
/// -> `[::1]`); None for an opaque origin (`null`) or anything without a host.
pub fn origin_host(origin: &str) -> Option<String> {
    let rest = origin.split_once("://")?.1;
    let hostport = rest.split(['/', '?', '#']).next()?;
    let host = if hostport.starts_with('[') {
        &hostport[..=hostport.find(']')?]
    } else {
        hostport
            .rsplit_once(':')
            .map(|(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { hostport })
            .unwrap_or(hostport)
    };
    (!host.is_empty()).then(|| host.trim_end_matches('.').to_ascii_lowercase())
}

/// Whether notifications from `origin` are allowed: every site with no list, else exactly the
/// listed hostnames (no subdomains).
pub fn site_allowed(origin: &str, sites: Option<&[String]>) -> bool {
    match sites {
        None => true,
        Some(list) => origin_host(origin).is_some_and(|h| list.contains(&h)),
    }
}

/// A notification prompt (and nothing else in it): the native handler denies it at once.
pub fn deny_prompt(requested: u32) -> bool {
    requested == NOTIFICATIONS
}

/// At most `max` characters, control characters (newlines too) as spaces, trimmed; cut ones end with `…`.
pub fn cap(s: &str, max: usize) -> String {
    let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = clean.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Token bucket per page (`BURST`, `REFILL_MS`).
#[derive(Default)]
pub struct RateLimit {
    /// Per page: tokens, last update, notifications dropped since the last one let through.
    pages: HashMap<TabId, (f64, Instant, u32)>,
}

impl RateLimit {
    /// Whether page `tab` may send one more notification at `now` (and counts it if so).
    pub fn allow(&mut self, tab: TabId, now: Instant) -> bool {
        self.check(tab, now).is_ok()
    }

    /// As `allow`; Err carries how many were dropped in a row so far (1 = the first, which the host
    /// logs; later ones are counted, not logged).
    pub fn check(&mut self, tab: TabId, now: Instant) -> Result<(), u32> {
        let (tokens, last, dropped) = self.pages.entry(tab).or_insert((BURST, now, 0));
        let refill = now.saturating_duration_since(*last).as_secs_f64() * 1000.0 / REFILL_MS;
        *tokens = (*tokens + refill).min(BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            *dropped = 0;
            Ok(())
        } else {
            *dropped += 1;
            Err(*dropped)
        }
    }

    pub fn forget(&mut self, tab: TabId) {
        self.pages.remove(&tab);
    }
}

static LIMIT: OnceLock<Mutex<RateLimit>> = OnceLock::new();

fn limit() -> std::sync::MutexGuard<'static, RateLimit> {
    LIMIT.get_or_init(Default::default).lock().unwrap()
}

/// A page closed: its rate-limit state goes.
pub fn forget_tab(tab: TabId) {
    limit().forget(tab);
}

wrap_permission_handler! {
    pub struct HostPermissionHandler {}

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            if !deny_prompt(requested_permissions) {
                // Other permissions keep Chrome's default handling (C-DL's item).
                return 0;
            }
            let origin = requesting_origin.map(|o| o.to_string()).unwrap_or_default();
            log!("notify: native notification prompt {prompt_id} from {origin} denied (the shim answers pages)");
            match callback {
                Some(cb) => {
                    cb.cont(PermissionRequestResult::DENY);
                    1
                }
                None => 0,
            }
        }
    }
}

/// Browser process, at context initialization: Chromium's own notification permission is BLOCK by
/// default and every per-site exception (an "Allow" clicked in an older run's bubble) is dropped.
pub fn block_native() {
    let Some(ctx) = request_context_get_global_context() else {
        log!("notify: no global request context; native notifications NOT blocked");
        return;
    };
    ctx.set_content_setting(None, None, ContentSettingTypes::NOTIFICATIONS, ContentSettingValues::BLOCK);
    let now = ctx.content_setting(None, None, ContentSettingTypes::NOTIFICATIONS).get_raw();
    let name = CefString::from("profile.content_settings.exceptions.notifications");
    let (mut v, d) = (value_create(), dictionary_value_create());
    let mut err = CefString::from("");
    let cleared = match (v.as_mut(), d) {
        (Some(v), Some(mut d)) => {
            v.set_dictionary(Some(&mut d));
            ctx.set_preference(Some(&name), Some(v), Some(&mut err))
        }
        _ => 0,
    };
    log!(
        "notify: native notifications default {now} (2 = block), site exceptions cleared={cleared} {}",
        err.to_string()
    );
}

/// Browser process: append the renderers' switch (`--notify-sites` given).
pub fn child_switch(cmd: &mut CommandLine) {
    if let Some(Some(list)) = SITES.get() {
        cmd.append_switch_with_value(
            Some(&CefString::from(SITES_SWITCH)),
            Some(&CefString::from(list.join(",").as_str())),
        );
    }
}

/// Runs in the page's context; called with the native `post(title, body, tag, kind, origin)`, `all`
/// (every site allowed) and `sites` (the comma-separated list otherwise). Nothing is left on the global
/// object but the replaced APIs.
const SHIM_JS: &str = r#"(function (post, all, sites) {
  'use strict';
  const S = v => (v === undefined || v === null) ? '' : String(v);
  const ORIGIN = S(self.origin);
  let host = '';
  try { host = new URL(ORIGIN).hostname.replace(/\.$/, '').toLowerCase(); } catch (e) {}
  const allowed = all || (host !== '' && sites.split(',').indexOf(host) >= 0);
  const PERM = allowed ? 'granted' : 'denied';
  const fire = (t, name) => setTimeout(() => {
    const e = new Event(name); t.dispatchEvent(e);
    const h = t['on' + name]; if (typeof h === 'function') h.call(t, e);
  }, 0);
  class Notification extends EventTarget {
    constructor(title, options) {
      super();
      if (arguments.length < 1) throw new TypeError("Failed to construct 'Notification': 1 argument required, but only 0 present.");
      options = options || {};
      this.title = S(title); this.body = S(options.body); this.tag = S(options.tag);
      this.data = options.data === undefined ? null : options.data;
      this.onclick = this.onshow = this.onerror = this.onclose = null;
      if (!allowed) { fire(this, 'error'); return; }
      post(this.title, this.body, this.tag, 'page', ORIGIN);
      fire(this, 'show');
    }
    close() { fire(this, 'close'); }
    static get permission() { return PERM; }
    static get maxActions() { return 2; }
    static requestPermission(cb) { if (typeof cb === 'function') setTimeout(() => cb(PERM), 0); return Promise.resolve(PERM); }
  }
  // Notification and ServiceWorkerRegistration are [SecureContext]: Blink installs them AFTER
  // OnContextCreated (LocalWindowProxy::Initialize: DidCreateScriptContext, then
  // InstallConditionalFeatures). NEVER define Notification here: Blink's install would meet our
  // property; a non-configurable one makes that install throw when strict page code is on the
  // stack (an iframe inserted from strict code commits about:blank synchronously), and Blink's
  // FromJust then aborts the renderer (C-BELL-CRASH, Gmail); a configurable one is just replaced.
  // So it is defined (configurable, as Blink's) once Blink's own is in place:
  // - page code on the stack (a frame created by a script): the microtask below runs when that
  //   script's turn ends, after the install;
  // - no script on the stack (a navigation commits): the microtask runs as soon as this script
  //   returns, BEFORE the install; it then waits for the parser's first node (a MutationObserver
  //   callback: the parser takes a microtask checkpoint before each parser-inserted script).
  // Insecure contexts never get Blink's Notification: defined at once there (still not now).
  // A new same-origin iframe's window taken in the turn that created it (its microtask has not run
  // yet) gets ours from the iframe's `contentWindow` getter (wrapped below, in define()). Other ways
  // to that window in that turn (`frames[i]`, `contentDocument.defaultView`, `window.open`) get
  // Chromium's own: permission "denied", natively blocked, so nothing shows, nothing reaches the bar.
  // ServiceWorkerRegistration: a page can only get a registration through navigator.serviceWorker,
  // so an own getter on this navigator (Blink installs nothing on the instance, only on
  // Navigator.prototype) patches the prototype on first use.
  const toStr = Function.prototype.toString;
  const native = v => { try { return typeof v === 'function' && /\[native code\]\s*\}$/.test(toStr.call(v)); } catch (e) { return false; } };
  const adoptFrames = () => {
    const I = globalThis.HTMLIFrameElement && HTMLIFrameElement.prototype;
    const d = I && Object.getOwnPropertyDescriptor(I, 'contentWindow');
    if (!d || !d.configurable || !native(d.get)) return;
    const get = d.get;
    Object.defineProperty(I, 'contentWindow', {configurable: true, enumerable: d.enumerable, get() {
      const w = get.call(this);
      try {
        // Blink's install in that frame is done: its window proxy is initialized by now.
        if (w && w.origin === ORIGIN && native(w.Notification))
          Object.defineProperty(w, 'Notification', {value: Notification, writable: true, configurable: true, enumerable: false});
      } catch (e) {}
      return w;
    }});
  };
  const define = () => {
    try {
      Object.defineProperty(globalThis, 'Notification', {value: Notification, writable: true, configurable: true, enumerable: false});
      adoptFrames();
    } catch (e) {}
  };
  const MO = globalThis.MutationObserver, doc = globalThis.document;
  queueMicrotask(() => {
    if (!globalThis.isSecureContext || Object.getOwnPropertyDescriptor(globalThis, 'Notification')) return define();
    if (!MO || !doc) return;
    const mo = new MO(() => { mo.disconnect(); define(); });
    mo.observe(doc, {childList: true, subtree: true});
  });
  let patched = false;
  const patchSW = () => {
    if (patched || !globalThis.ServiceWorkerRegistration) return;
    patched = true;
    const P = ServiceWorkerRegistration.prototype;
    P.showNotification = function (title, options) {
      if (!allowed) return Promise.reject(new TypeError('No notification permission has been granted for this origin.'));
      options = options || {};
      post(S(title), S(options.body), S(options.tag), 'sw', ORIGIN);
      return Promise.resolve();
    };
    P.getNotifications = function () { return Promise.resolve([]); };
  };
  if (globalThis.navigator && globalThis.Navigator) {
    Object.defineProperty(navigator, 'serviceWorker', {configurable: true, enumerable: true, get() {
      patchSW();
      const d = Object.getOwnPropertyDescriptor(Navigator.prototype, 'serviceWorker');
      return d && d.get ? d.get.call(this) : undefined;
    }});
  }
  if (globalThis.Permissions && Permissions.prototype.query) {
    const q = Permissions.prototype.query;
    Permissions.prototype.query = function (d) {
      if (d && d.name === 'notifications') return Promise.resolve({state: PERM, name: 'notifications', onchange: null});
      return q.apply(this, arguments);
    };
  }
})"#;

wrap_v8_handler! {
    struct NotifyPost {}

    impl V8Handler {
        // Called by SHIM_JS: (title, body, tag, kind, origin).
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
                for i in 0..5 {
                    list.set_string(i, Some(&CefString::from(s(i).as_str())));
                }
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
            1
        }
    }
}

/// Renderer: the renderer's `--web-notify-sites` (None: every site), read once.
fn renderer_sites() -> &'static Option<String> {
    static S: OnceLock<Option<String>> = OnceLock::new();
    S.get_or_init(|| {
        let cmd = command_line_get_global()?;
        let name = CefString::from(SITES_SWITCH);
        (cmd.has_switch(Some(&name)) != 0).then(|| CefString::from(&cmd.switch_value(Some(&name))).to_string())
    })
}

/// Renderer, OnContextCreated: replace the notification APIs in this context.
pub fn install_shim(context: &mut V8Context) {
    let sites = renderer_sites();
    let args = [
        v8_value_create_function(Some(&CefString::from("post")), Some(&mut NotifyPost::new())),
        v8_value_create_bool(sites.is_none() as i32),
        v8_value_create_string(Some(&CefString::from(sites.as_deref().unwrap_or("")))),
    ];
    crate::renderer::call_with(context, "notify", SHIM_JS, &args);
}

/// Browser process: a forwarded notification. Returns false for other messages.
pub fn handle_message(tab: Option<TabId>, msg: &ProcessMessage) -> bool {
    if CefString::from(&msg.name()).to_string() != NOTIFY_MSG {
        return false;
    }
    let (Some(tab), Some(a)) = (tab, msg.argument_list()) else {
        return true;
    };
    let s = |i| CefString::from(&a.string(i)).to_string();
    let (kind, origin) = (s(3), s(4));
    let sites = SITES.get().and_then(|s| s.as_deref());
    if !site_allowed(&origin, sites) {
        // The shim already says "denied" there; a renderer that sends one anyway is ignored.
        log!("notify: tab {tab}: {origin:?} is not in --notify-sites; dropped");
        return true;
    }
    if let Err(n) = limit().check(tab, Instant::now()) {
        if n == 1 {
            log!("notify: tab {tab}: rate limit, dropping ({kind} from {origin}; more are not logged)");
        }
        return true;
    }
    emit(Event::Notify {
        tab,
        title: cap(&s(0), MAX_TITLE),
        body: cap(&s(1), MAX_BODY),
        tag: cap(&s(2), MAX_TAG),
        origin: cap(&origin, MAX_ORIGIN),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn origin_hosts() {
        assert_eq!(origin_host("https://Mail.Example.com").as_deref(), Some("mail.example.com"));
        assert_eq!(origin_host("http://127.0.0.1:8080").as_deref(), Some("127.0.0.1"));
        assert_eq!(origin_host("http://[::1]:80").as_deref(), Some("[::1]"));
        assert_eq!(origin_host("https://a.example./").as_deref(), Some("a.example"));
        assert_eq!(origin_host("null"), None);
        assert_eq!(origin_host("file://"), None);
        assert_eq!(origin_host(""), None);
    }

    #[test]
    fn sites_all_or_exact_hostnames() {
        assert!(site_allowed("null", None));
        assert!(site_allowed("https://x.example", None));
        let list = parse_sites(" Chat.Example.com, ,mail.example.com.,");
        assert_eq!(list, vec!["chat.example.com", "mail.example.com"]);
        assert!(site_allowed("https://chat.example.com", Some(&list)));
        assert!(site_allowed("https://mail.example.com:8443", Some(&list)));
        assert!(!site_allowed("https://example.com", Some(&list)), "no parent domain");
        assert!(!site_allowed("https://a.chat.example.com", Some(&list)), "no subdomains");
        assert!(!site_allowed("null", Some(&list)), "opaque origins are denied with a list");
        assert!(!site_allowed("https://chat.example.com", Some(&[])), "an empty list allows nothing");
    }

    #[test]
    fn only_a_pure_notification_prompt_is_denied() {
        assert!(deny_prompt(NOTIFICATIONS));
        assert!(!deny_prompt(NOTIFICATIONS | 1 << 3), "mixed prompts keep Chrome's handling");
        assert!(!deny_prompt(1 << 3));
        assert!(!deny_prompt(0));
    }

    #[test]
    fn caps_lengths_and_control_characters() {
        assert_eq!(cap("  a\nb\tc ", 10), "a b c");
        assert_eq!(cap("abcdef", 4), "abc\u{2026}");
        assert_eq!(cap("abcd", 4), "abcd");
        assert_eq!(cap(&"é".repeat(300), MAX_TITLE).chars().count(), MAX_TITLE);
    }

    #[test]
    fn rate_limit_burst_then_refill_per_page() {
        let mut r = RateLimit::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(r.allow(1, t0));
        }
        assert_eq!(r.check(1, t0), Err(1), "the 6th at once is dropped (the first drop: logged)");
        assert!(r.allow(2, t0), "another page has its own bucket");
        assert_eq!(r.check(1, t0 + Duration::from_millis(1000)), Err(2), "counted, not logged");
        assert_eq!(r.check(1, t0 + Duration::from_millis(2100)), Ok(()), "one more after REFILL_MS");
        assert_eq!(r.check(1, t0 + Duration::from_millis(2100)), Err(1), "a new run of drops");
        assert!(!r.allow(1, t0 + Duration::from_millis(2200)));
        r.forget(1);
        assert!(r.allow(1, t0 + Duration::from_millis(2200)));
    }

    #[test]
    fn shim_keeps_the_defensive_tricks() {
        // C-BELL-CRASH: Notification is replaced from a microtask (after Blink's install), and
        // nothing the shim defines is non-configurable (Blink's install would abort the renderer).
        assert!(SHIM_JS.contains("const define = () => {"));
        assert_eq!(SHIM_JS.matches("Object.defineProperty(globalThis").count(), 1, "only in define()");
        assert_eq!(
            SHIM_JS.matches("define();").count(),
            2,
            "called from the microtask and the observer only"
        );
        let micro = SHIM_JS.find("queueMicrotask(() => {").expect("the microtask");
        assert!(SHIM_JS.find("return define();").is_some_and(|i| i > micro));
        assert!(SHIM_JS.find("mo.disconnect(); define();").is_some_and(|i| i > micro));
        // A fresh iframe's window from contentWindow gets ours (only over Blink's native one).
        assert!(SHIM_JS.contains("w.origin === ORIGIN && native(w.Notification)"));
        assert!(
            SHIM_JS.contains("adoptFrames();\n    } catch (e) {}"),
            "wrapped after the install, in define()"
        );
        assert!(!SHIM_JS.contains("configurable: false"));
        assert!(!SHIM_JS.contains("writable: false"));
        assert!(!SHIM_JS.contains("Object.freeze") && !SHIM_JS.contains("preventExtensions"));
        // ServiceWorkerRegistration via the serviceWorker getter on this navigator.
        assert!(SHIM_JS.contains("Object.defineProperty(navigator, 'serviceWorker'"));
        // The native function is an argument (nothing global to call or delete).
        assert!(SHIM_JS.starts_with("(function (post, all, sites)"));
        assert!(!SHIM_JS.contains("window.__"));
        // Denied sites: an error event, a rejected showNotification, nothing posted.
        assert!(SHIM_JS.contains("if (!allowed) { fire(this, 'error'); return; }"));
        assert!(SHIM_JS.contains("if (!allowed) return Promise.reject"));
        assert!(SHIM_JS.trim_end().ends_with("})"));
    }
}
