//! Q33a: running classic `<script>`s with QuickJS (`rquickjs`).
//!
//! Scope of this first slice: no DOM API at all. A page's scripts run in
//! document order inside a fresh QuickJS context that has only a tiny
//! prelude (`console`, `window`/`document` stubs that can register
//! `DOMContentLoaded`/`load` listeners), then those two events fire. A
//! script error is logged to stderr and never stops the next script or
//! the page. The engine is created per page load and dropped afterwards
//! (Q33t keeps it alive in `App` for timers; see `JsEngine`.)
use super::*;
use rquickjs::context::EvalOptions;
use rquickjs::{Context, FromJs, Ctx, Error as JsError, Exception, Function, Runtime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

thread_local! {
    /// URL of the page whose scripts are about to run (`location`,
    /// `document.URL`); set by the caller, read by `run_scripts_with`.
    static PAGE_URL: std::cell::RefCell<Option<Url>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn set_page_url(url: Option<&Url>) {
    PAGE_URL.with(|u| *u.borrow_mut() = url.cloned());
}

/// Viewport reported as `window.innerWidth/innerHeight` until the real
/// size is plumbed through (new item Q33b2).
const DEFAULT_VIEWPORT: (u32, u32) = (1024, 768);

static JS_ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether scripts run (and `<noscript>` is hidden). `--no-js` / `:set
/// nojs` turn it off; a `:set` change applies from the next page load.
pub(crate) fn js_enabled() -> bool {
    JS_ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn set_js_enabled(on: bool) {
    JS_ENABLED.store(on, Ordering::Relaxed);
}

/// One `<script>` as found in the markup, before any fetching.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScriptRef {
    Inline(String),
    /// The raw `src` attribute, unresolved.
    External(String),
}

/// A script ready to run: `name` is only used in error messages.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScriptSource {
    pub(crate) name: String,
    pub(crate) code: String,
    /// QB8: position among the document's runnable `<script>` elements
    /// (the `find_external_resources` order), so `document.currentScript`
    /// can find the element. `None` = no element (tests).
    pub(crate) index: Option<usize>,
}

/// Classic script `type` values (absent/empty counts); `module`,
/// `importmap`, `application/json`, `text/template` etc. don't run.
pub(crate) fn is_classic_script_type(ty: Option<&str>) -> bool {
    let Some(ty) = ty else { return true };
    let ty = ty.trim().to_ascii_lowercase();
    ty.is_empty()
        || matches!(
            ty.as_str(),
            "text/javascript"
                | "application/javascript"
                | "application/x-javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "text/jscript"
                | "text/livescript"
                | "text/x-javascript"
                | "text/x-ecmascript"
        )
}

/// The `ScriptRef` for a `<script>` element, `None` if it isn't a
/// runnable classic script. (`language=` and `nomodule` are ignored.)
pub(crate) fn script_ref(handle: &Handle) -> Option<ScriptRef> {
    if !is_classic_script_type(element_attr(handle, "type").as_deref()) {
        return None;
    }
    if let Some(src) = element_attr(handle, "src") {
        let src = src.trim();
        return (!src.is_empty()).then(|| ScriptRef::External(src.to_string()));
    }
    let mut code = String::new();
    for child in handle.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            code.push_str(&contents.borrow());
        }
    }
    (!code.trim().is_empty()).then_some(ScriptRef::Inline(code))
}

/// QB8: every runnable classic `<script>` element in document order, the
/// same order and filter as `find_external_resources`.
fn collect_script_elements(handle: &Handle, out: &mut Vec<Handle>) {
    if let NodeData::Element { name, .. } = &handle.data
        && name.local.as_ref() == "script"
        && script_ref(handle).is_some()
    {
        out.push(handle.clone());
    }
    for child in handle.children.borrow().iter() {
        collect_script_elements(child, out);
    }
}

/// Inline scripts only, in document order — for pages with no way to
/// fetch external ones (no `--socket`, no base URL).
pub(crate) fn inline_scripts(document: &Handle) -> Vec<ScriptSource> {
    let mut res = ExternalResources::default();
    find_external_resources(document, &mut res);
    res.scripts
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            ScriptRef::Inline(code) => Some(ScriptSource { name: format!("inline script #{}", i + 1), code, index: Some(i) }),
            ScriptRef::External(_) => None,
        })
        .collect()
}

/// Resolves every `ScriptRef` against `base` into runnable sources, in
/// document order. External scripts are fetched concurrently (one thread
/// each, joined in order); one that fails to fetch is logged and skipped.
pub(crate) fn fetch_scripts(path: &std::path::Path, base: &Url, refs: Vec<ScriptRef>) -> Vec<ScriptSource> {
    if !js_enabled() {
        return Vec::new();
    }
    thread::scope(|scope| {
        let handles: Vec<_> = refs
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                scope.spawn(move || match r {
                    ScriptRef::Inline(code) => Some(ScriptSource { name: format!("inline script #{}", i + 1), code, index: Some(i) }),
                    ScriptRef::External(src) if is_data_url(&src) => match decode_data_uri_text(&src) {
                        Some(code) => Some(ScriptSource { name: "data: script".to_string(), code, index: Some(i) }),
                        None => {
                            diag!("scratchy: js: malformed data: script URL");
                            None
                        }
                    },
                    ScriptRef::External(src) => {
                        let url = resolve_href(Some(base), &src).ok()?;
                        match fetch_script_via_wire(path, &url) {
                            Some(code) => Some(ScriptSource { name: url.to_string(), code, index: Some(i) }),
                            None => {
                                diag!("scratchy: js: could not fetch script {url}");
                                None
                            }
                        }
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
    })
}

/// Everything a page sees before its own scripts run. `console.*` calls
/// go to the native `__scratchy_log(level, text)`; `window`/`document`
/// only know how to register and fire `DOMContentLoaded`/`load`.
const PRELUDE: &str = r#"
(function (g) {
  var fmt = function (a) {
    return a.map(function (x) {
      if (typeof x === 'string') return x;
      try { if (x && typeof x === 'object' && !(x instanceof Error)) return JSON.stringify(x); } catch (e) {}
      return String(x);
    }).join(' ');
  };
  var c = {};
  ['log', 'info', 'debug', 'warn', 'error'].forEach(function (l) {
    c[l] = function () { __scratchy_log(l, fmt(Array.prototype.slice.call(arguments))); };
  });
  g.console = c;
  g.window = g; g.self = g;
  // Q33b1: constructor globals pages feature-test or extend. Real DOM
  // wrappers (Q33b) hang off these prototypes.
  var ctor = function (name, parent, init) {
    var f = new Function('init', 'return function ' + name + '() { if (init) init.apply(this, arguments); };')(init);
    if (parent) f.prototype = Object.create(parent.prototype, { constructor: { value: f, writable: true, configurable: true } });
    g[name] = f;
    return f;
  };
  var EventTarget = ctor('EventTarget');
  var Node = ctor('Node', EventTarget);
  var Element = ctor('Element', Node);
  var HTMLElement = ctor('HTMLElement', Element);
  var Document = ctor('Document', Node);
  ctor('HTMLDocument', Document);
  // QY4: node-type constructor globals (wrappers get these prototypes in `wrap`).
  var CharacterData = ctor('CharacterData', Node);
  ctor('Text', CharacterData); ctor('Comment', CharacterData);
  ctor('DocumentFragment', Node);
  ctor('Attr', Node); ctor('NodeList'); ctor('HTMLCollection');
  g.NodeFilter = { FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3, SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 1,
    SHOW_ATTRIBUTE: 2, SHOW_TEXT: 4, SHOW_CDATA_SECTION: 8, SHOW_ENTITY_REFERENCE: 16, SHOW_ENTITY: 32,
    SHOW_PROCESSING_INSTRUCTION: 64, SHOW_COMMENT: 128, SHOW_DOCUMENT: 256, SHOW_DOCUMENT_TYPE: 512,
    SHOW_DOCUMENT_FRAGMENT: 1024, SHOW_NOTATION: 2048 };
  [['ELEMENT_NODE', 1], ['ATTRIBUTE_NODE', 2], ['TEXT_NODE', 3], ['COMMENT_NODE', 8], ['DOCUMENT_NODE', 9], ['DOCUMENT_FRAGMENT_NODE', 11]]
    .forEach(function (kv) { Node[kv[0]] = Node.prototype[kv[0]] = kv[1]; });
  // Q33d: events. One listener store (a WeakMap keyed by the target
  // object) serves window, document, nodes and bare `new EventTarget()`.
  var store = new WeakMap();
  var lists = function (t, type, create) {
    var m = store.get(t);
    if (!m) { if (!create) return null; m = {}; store.set(t, m); }
    return m[type] || (create ? (m[type] = []) : null);
  };
  var Event = ctor('Event', null, function (type, init) {
    init = init || {};
    this.type = String(type);
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.composed = !!init.composed;
    this.defaultPrevented = false;
    this.target = null;
    this.currentTarget = null;
    this.eventPhase = 0;
    this.isTrusted = false;
    this.timeStamp = __scratchy_now();
  });
  Event.NONE = 0; Event.CAPTURING_PHASE = 1; Event.AT_TARGET = 2; Event.BUBBLING_PHASE = 3;
  Event.prototype.preventDefault = function () { if (this.cancelable && !this._passive) this.defaultPrevented = true; };
  Event.prototype.stopPropagation = function () { this._stop = true; };
  Event.prototype.stopImmediatePropagation = function () { this._stop = true; this._stopNow = true; };
  Event.prototype.composedPath = function () { return this._path ? this._path.slice() : []; };
  Object.defineProperty(Event.prototype, 'cancelBubble', { get: function () { return !!this._stop; },
    set: function (v) { if (v) this._stop = true; }, configurable: true });
  var CustomEvent = ctor('CustomEvent', Event, function (type, init) {
    Event.call(this, type, init);
    this.detail = init && init.detail !== undefined ? init.detail : null;
  });
  var UIEvent = ctor('UIEvent', Event, function (type, init) { Event.call(this, type, init); this.detail = (init && init.detail) || 0; });
  var MouseEvent = ctor('MouseEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = init || {};
    var self = this;
    ['clientX', 'clientY', 'screenX', 'screenY', 'button', 'buttons'].forEach(function (k) { self[k] = Number(init[k]) || 0; });
    ['ctrlKey', 'shiftKey', 'altKey', 'metaKey'].forEach(function (k) { self[k] = !!init[k]; });
    this.pageX = this.clientX; this.pageY = this.clientY; this.offsetX = this.clientX; this.offsetY = this.clientY;
    this.relatedTarget = init.relatedTarget || null;
  });
  var KeyboardEvent = ctor('KeyboardEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = init || {};
    this.key = init.key === undefined ? '' : String(init.key);
    this.code = init.code === undefined ? '' : String(init.code);
    this.repeat = !!init.repeat;
    this.keyCode = Number(init.keyCode) || 0; this.which = this.keyCode;
    var self = this;
    ['ctrlKey', 'shiftKey', 'altKey', 'metaKey'].forEach(function (k) { self[k] = !!init[k]; });
  });
  // QY4/QA15: legacy init*Event methods and ErrorEvent.
  Event.prototype.initEvent = function (type, bubbles, cancelable) {
    this.type = String(type); this.bubbles = !!bubbles; this.cancelable = !!cancelable;
  };
  CustomEvent.prototype.initCustomEvent = function (type, bubbles, cancelable, detail) {
    this.initEvent(type, bubbles, cancelable); this.detail = detail === undefined ? null : detail;
  };
  UIEvent.prototype.initUIEvent = function (type, bubbles, cancelable, view, detail) {
    this.initEvent(type, bubbles, cancelable); this.view = view || null; this.detail = Number(detail) || 0;
  };
  MouseEvent.prototype.initMouseEvent = function (type, bubbles, cancelable, view, detail, sx, sy, cx, cy, ctrl, alt, shift, meta, button, related) {
    this.initUIEvent(type, bubbles, cancelable, view, detail);
    this.screenX = Number(sx) || 0; this.screenY = Number(sy) || 0; this.clientX = Number(cx) || 0; this.clientY = Number(cy) || 0;
    this.pageX = this.clientX; this.pageY = this.clientY; this.offsetX = this.clientX; this.offsetY = this.clientY;
    this.ctrlKey = !!ctrl; this.altKey = !!alt; this.shiftKey = !!shift; this.metaKey = !!meta;
    this.button = Number(button) || 0; this.relatedTarget = related || null;
  };
  KeyboardEvent.prototype.initKeyboardEvent = function (type, bubbles, cancelable, view, key, loc, ctrl, alt, shift, meta) {
    this.initUIEvent(type, bubbles, cancelable, view, 0);
    this.key = key === undefined ? '' : String(key); this.location = Number(loc) || 0;
    this.ctrlKey = !!ctrl; this.altKey = !!alt; this.shiftKey = !!shift; this.metaKey = !!meta;
  };
  ctor('ErrorEvent', Event, function (type, init) {
    Event.call(this, type, init);
    init = init || {};
    this.message = init.message === undefined ? '' : String(init.message);
    this.filename = init.filename === undefined ? '' : String(init.filename);
    this.lineno = Number(init.lineno) || 0; this.colno = Number(init.colno) || 0;
    this.error = init.error === undefined ? null : init.error;
  });
  var FocusEvent = ctor('FocusEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    this.relatedTarget = (init && init.relatedTarget) || null;
  });
  ctor('InputEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = init || {};
    this.data = init.data === undefined ? null : init.data;
    this.inputType = init.inputType || '';
    this.isComposing = false;
  });
  var normOpts = function (o) {
    if (typeof o === 'boolean') return { capture: o };
    o = o || {};
    return { capture: !!o.capture, once: !!o.once, passive: !!o.passive };
  };
  var oops = function (e) { console.error('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); };
  var dispatch = function (target, ev) {
    ev.target = target;
    ev._stop = false; ev._stopNow = false;
    var path = [target];
    if (target && target.__id !== undefined) {
      for (var n = target.parentNode; n; n = n.parentNode) path.push(n);
      if (path[path.length - 1] === g.document) path.push(g);
    }
    ev._path = path;
    // capFilter: true = capture listeners only, false = non-capture only.
    var invoke = function (node, phase, capFilter) {
      ev.currentTarget = node; ev.eventPhase = phase;
      var l = lists(node, ev.type, false);
      if (l) {
        var snap = l.slice();
        for (var i = 0; i < snap.length; i++) {
          var e = snap[i];
          if (e.removed || e.capture !== capFilter) continue;
          if (e.once) node.removeEventListener(ev.type, e.fn, e.capture);
          ev._passive = e.passive;
          try {
            if (typeof e.fn === 'function') e.fn.call(node, ev);
            else if (e.fn && typeof e.fn.handleEvent === 'function') e.fn.handleEvent(ev);
          } catch (x) { oops(x); }
          ev._passive = false;
          if (ev._stopNow) return;
        }
      }
      if (!capFilter && !ev._stopNow) {
        var h = node['on' + ev.type];
        if (typeof h === 'function') {
          try { if (h.call(node, ev) === false) ev.preventDefault(); } catch (x) { oops(x); }
        }
      }
    };
    var i;
    for (i = path.length - 1; i > 0 && !ev._stop; i--) invoke(path[i], 1, true);
    if (!ev._stop) { invoke(path[0], 2, true); if (!ev._stopNow) invoke(path[0], 2, false); }
    if (ev.bubbles) for (i = 1; i < path.length && !ev._stop; i++) invoke(path[i], 3, false);
    ev.currentTarget = null; ev.eventPhase = 0;
    return !ev.defaultPrevented;
  };
  Object.defineProperty(g, '__scratchy_dispatch', { value: dispatch });
  var ETP = EventTarget.prototype;
  ETP.addEventListener = function (type, fn, opt) {
    if (fn == null || (typeof fn !== 'function' && typeof fn !== 'object')) return;
    var o = normOpts(opt), l = lists(this, String(type), true);
    for (var i = 0; i < l.length; i++) if (l[i].fn === fn && l[i].capture === o.capture) return;
    l.push({ fn: fn, capture: o.capture, once: o.once, passive: o.passive, removed: false });
  };
  ETP.removeEventListener = function (type, fn, opt) {
    var l = lists(this, String(type), false);
    if (!l) return;
    var c = normOpts(opt).capture;
    for (var i = 0; i < l.length; i++) {
      if (l[i].fn === fn && l[i].capture === c) { l[i].removed = true; l.splice(i, 1); return; }
    }
  };
  ETP.dispatchEvent = function (ev) {
    if (!(ev instanceof Event)) throw new TypeError("Failed to execute 'dispatchEvent': parameter 1 is not of type 'Event'.");
    ev.isTrusted = false;
    return dispatch(this, ev);
  };
  ['addEventListener', 'removeEventListener', 'dispatchEvent'].forEach(function (m) {
    Object.defineProperty(g, m, { value: function () { return ETP[m].apply(g, arguments); }, writable: true, configurable: true });
  });
  var Image = ctor('Image', HTMLElement, function (w, h) {
    this.tagName = 'IMG'; this.src = ''; this.width = w || 0; this.height = h || 0;
  });
  var env = g.__scratchy_env || {};
  delete g.__scratchy_env;
  g.navigator = {
    userAgent: env.userAgent, appName: 'Netscape', appVersion: '5.0 (X11)', product: 'Gecko',
    platform: 'Linux x86_64', vendor: '', language: 'en-US', languages: ['en-US', 'en'],
    onLine: true, cookieEnabled: false, hardwareConcurrency: 1, maxTouchPoints: 0,
  };
  g.innerWidth = Number(env.innerWidth); g.innerHeight = Number(env.innerHeight);
  g.outerWidth = g.innerWidth; g.outerHeight = g.innerHeight; g.devicePixelRatio = 1;
  var loc = { href: env.href, protocol: env.protocol, host: env.host, hostname: env.hostname,
    port: env.port, pathname: env.pathname, search: env.search, hash: env.hash, origin: env.origin };
  loc.toString = function () { return this.href; };
  loc.assign = loc.replace = loc.reload = function () {};
  g.location = loc;
  var doc = ({ readyState: 'loading', title: env.title || '', URL: env.href, documentURI: env.href,
    baseURI: env.href, referrer: '', characterSet: 'UTF-8', contentType: 'text/html', compatMode: 'CSS1Compat',
    nodeType: 9, nodeName: '#document' });
  // cookie: reads are empty, writes are ignored (until Q33g)
  Object.defineProperty(doc, 'cookie', { get: function () { return ''; }, set: function () {}, configurable: true, enumerable: true });
  Object.setPrototypeOf(doc, Document.prototype);
  Object.defineProperty(doc, 'location', { get: function () { return g.location; }, configurable: true });
  // QY2: page visibility (the window is always shown).
  Object.defineProperty(doc, 'visibilityState', { get: function () { return 'visible'; }, configurable: true, enumerable: true });
  Object.defineProperty(doc, 'hidden', { get: function () { return false; }, configurable: true, enumerable: true });
  g.document = doc;
  // QY2: `performance`: Navigation Timing 1 numbers (epoch ms, all taken
  // at page start and monotonic), `timeOrigin`, empty entry lists.
  var t0 = Date.now();
  var timing = {};
  ['navigationStart', 'unloadEventStart', 'unloadEventEnd', 'redirectStart', 'redirectEnd', 'fetchStart',
    'domainLookupStart', 'domainLookupEnd', 'connectStart', 'connectEnd', 'secureConnectionStart',
    'requestStart', 'responseStart', 'responseEnd', 'domLoading', 'domInteractive',
    'domContentLoadedEventStart', 'domContentLoadedEventEnd', 'domComplete', 'loadEventStart', 'loadEventEnd']
    .forEach(function (k, i) {
      timing[k] = (k === 'unloadEventStart' || k === 'unloadEventEnd' || k === 'redirectStart' || k === 'redirectEnd') ? 0 : t0;
    });
  var perf = {
    timing: timing, timeOrigin: t0,
    navigation: { type: 0, redirectCount: 0 },
    now: function () { return __scratchy_now(); },
    getEntries: function () { return []; },
    getEntriesByType: function () { return []; },
    getEntriesByName: function () { return []; },
    mark: function () {}, measure: function () {},
    clearMarks: function () {}, clearMeasures: function () {},
    toJSON: function () { return { timeOrigin: t0, timing: timing }; },
  };
  Object.defineProperty(g, 'performance', { value: perf, writable: true, configurable: true, enumerable: true });
})(globalThis);
"#;

const FIRE_DOM_CONTENT_LOADED: &str =
    "document.readyState='interactive';__scratchy_dispatch(document,new Event('DOMContentLoaded',{bubbles:true}));";
const FIRE_LOAD: &str = "document.readyState='complete';__scratchy_dispatch(window,new Event('load'));";

/// Total wall-clock budget for one page's scripts; QuickJS is
/// interrupted past it so a runaway loop cannot freeze the window.
/// Sink for console output and script errors: `(level, text)`.
pub(crate) type LogFn = Rc<dyn Fn(&str, &str)>;

const SCRIPT_BUDGET: Duration = Duration::from_secs(5);
/// QB3: budget for one timer or event callback. The page-load run keeps
/// `SCRIPT_BUDGET`; a callback runs on the main thread while input is
/// flowing, and the compositor drops a client that doesn't read its
/// socket for long enough (R8), so it gets a tenth of that.
pub(crate) const CALLBACK_BUDGET: Duration = Duration::from_millis(500);
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// Defines `__scratchy_env` (plain strings) for the prelude to build
/// `location`, `navigator`, `document.URL` from.
fn install_env(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let env = rquickjs::Object::new(ctx.clone())?;
    let url = PAGE_URL.with(|u| u.borrow().clone()).unwrap_or_else(|| Url::parse("about:blank").unwrap());
    let origin = match url.scheme() {
        "http" | "https" | "ftp" | "ws" | "wss" => url.origin().ascii_serialization(),
        _ => "null".to_string(),
    };
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();
    let hostname = url.host_str().unwrap_or("").to_string();
    let host = if port.is_empty() { hostname.clone() } else { format!("{hostname}:{port}") };
    let pathname = url.path().to_string();
    env.set("href", url.as_str())?;
    env.set("protocol", format!("{}:", url.scheme()))?;
    env.set("host", host)?;
    env.set("hostname", hostname)?;
    env.set("port", port)?;
    env.set("pathname", pathname)?;
    env.set("search", url.query().filter(|q| !q.is_empty()).map(|q| format!("?{q}")).unwrap_or_default())?;
    env.set("hash", url.fragment().filter(|f| !f.is_empty()).map(|f| format!("#{f}")).unwrap_or_default())?;
    env.set("origin", origin)?;
    env.set("userAgent", format!("Mozilla/5.0 (X11; Linux x86_64) scratchy/{}", env!("CARGO_PKG_VERSION")))?;
    env.set("innerWidth", DEFAULT_VIEWPORT.0)?;
    env.set("innerHeight", DEFAULT_VIEWPORT.1)?;
    ctx.globals().set("__scratchy_env", env)
}

fn describe_error(ctx: &Ctx<'_>, err: JsError) -> String {
    if matches!(err, JsError::Exception) {
        let value = ctx.catch();
        if let Some(ex) = value.as_object().and_then(|o| Exception::from_object(o.clone())) {
            let msg = ex.message().unwrap_or_default();
            let name = value.as_object().and_then(|o| o.get::<_, String>("name").ok()).unwrap_or_default();
            let msg = if name.is_empty() { msg } else { format!("{name}: {msg}") };
            return match ex.stack() {
                Some(stack) if !stack.is_empty() => format!("{msg}\n{stack}"),
                _ => msg,
            };
        }
        let text = rquickjs::Coerced::<String>::from_js(ctx, value.clone())
            .map(|c| c.0)
            .unwrap_or_else(|_| "<unprintable value>".into());
        return format!("Uncaught {text}");
    }
    err.to_string()
}

fn drain_jobs(rt: &Runtime, log: &dyn Fn(&str, &str)) {
    while rt.is_job_pending() {
        if rt.execute_pending_job().is_err() {
            log("error", "Uncaught exception in a promise job");
        }
    }
}

/// Q33t: a page's live QuickJS runtime. Kept in `App` for the page's
/// lifetime so timers can fire after load; dropping it (navigation)
/// discards every pending timer. Field order matters: the context must
/// drop before the runtime.
pub(crate) struct JsEngine {
    ctx: Context,
    rt: Runtime,
    log: LogFn,
    base: Instant,
    /// Interrupt deadline in nanoseconds since `base`; reset before
    /// every top-level evaluation so each timer callback gets its own
    /// `SCRIPT_BUDGET` rather than sharing the page-load one.
    deadline: Arc<std::sync::atomic::AtomicU64>,
    /// Q33c: set by every DOM write; the app drains it with `take_dirty`.
    dirty: Rc<std::cell::Cell<bool>>,
    /// Q33d: handle <-> JS id table, to address events at nodes.
    table: Rc<std::cell::RefCell<js_dom::DomTable>>,
    /// QB5: focus/blur/scroll requests for the App.
    reqs: Rc<std::cell::RefCell<js_dom::JsRequests>>,
}

/// One init field of a natively dispatched event (Q33d).
pub(crate) enum EventField {
    Bool(bool),
    Num(f64),
    Str(String),
}

/// `s` as a JS string literal (everything outside printable ASCII is
/// `\u`-escaped, so it is safe in any source).
fn js_string_literal(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
        }
    }
    out.push('"');
    out
}

/// Max timer callbacks run in one event-loop turn, so a pile of due
/// zero-delay timers cannot starve painting and input.
const MAX_TIMERS_PER_TURN: usize = 100;

impl JsEngine {
    fn arm_budget(&self, budget: Duration) {
        let d = (self.base.elapsed() + budget).as_nanos() as u64;
        self.deadline.store(d, Ordering::Relaxed);
    }

    pub(crate) fn eval(&self, name: &str, code: &str) {
        self.arm_budget(SCRIPT_BUDGET);
        self.ctx.with(|ctx| {
            // HTML classic scripts are sloppy unless they opt in; rquickjs
            // defaults to strict, so build the options explicitly (R5).
            let mut opts = EvalOptions::default();
            opts.strict = false;
            opts.filename = Some(name.to_string());
            if let Err(e) = ctx.eval_with_options::<(), _>(code, opts) {
                (self.log)("error", &format!("{name}: {}", describe_error(&ctx, e)));
            }
        });
        drain_jobs(&self.rt, &*self.log);
    }

    pub(crate) fn eval_value<T: for<'js> FromJs<'js>>(&self, label: &str, code: &str) -> Option<T> {
        self.arm_budget(CALLBACK_BUDGET);
        let started = Instant::now();
        let v = self.ctx.with(|ctx| {
            let mut opts = EvalOptions::default();
            opts.strict = false;
            opts.filename = Some(label.to_string());
            match ctx.eval_with_options::<T, _>(code, opts) {
                Ok(v) => Some(v),
                Err(e) => {
                    (self.log)("error", &format!("{label}: {}", describe_error(&ctx, e)));
                    None
                }
            }
        });
        drain_jobs(&self.rt, &*self.log);
        if started.elapsed() >= CALLBACK_BUDGET {
            (self.log)("error", &format!("a callback ran past its {} ms budget and was interrupted", CALLBACK_BUDGET.as_millis()));
        }
        v
    }

    /// Whether JS wrote to the DOM since the last call (clears the flag).
    pub(crate) fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    /// QB5: drains pending focus/blur/scroll requests.
    pub(crate) fn take_requests(&self) -> js_dom::JsRequests {
        std::mem::take(&mut *self.reqs.borrow_mut())
    }

    /// Q33d: fires a trusted event of `kind` (`click`, `keydown`, ...) at
    /// `target` (`None` = `<body>`, else the document) with the given init fields, through
    /// capture/target/bubble. Returns false when a page handler called
    /// `preventDefault`, so the caller must skip its default action.
    pub(crate) fn dispatch_event(&self, target: Option<&Handle>, kind: &str, init: &[(&str, EventField)]) -> bool {
        let id = match target {
            Some(h) => self.table.borrow_mut().intern(h),
            None => -2,
        };
        let mut obj = String::from("{");
        for (k, v) in init {
            obj.push_str(&js_string_literal(k));
            obj.push(':');
            match v {
                EventField::Bool(b) => obj.push_str(&b.to_string()),
                EventField::Num(n) => obj.push_str(&n.to_string()),
                EventField::Str(s) => obj.push_str(&js_string_literal(s)),
            }
            obj.push(',');
        }
        obj.push('}');
        let code = format!("__scratchy_dn({id},{},{obj})", js_string_literal(kind));
        self.eval_value::<bool>(&format!("event:{kind}"), &code).unwrap_or(true)
    }

    /// When the earliest pending timer is due, `None` if there are none
    /// (the event loop then sleeps with no wake-up at all).
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let ms: f64 = self.eval_value("timer", "__scratchy_timers_next()")?;
        (ms >= 0.0).then(|| self.base + Duration::from_secs_f64(ms / 1000.0))
    }

    /// Runs every timer that is due now (bounded per turn), each under
    /// its own interrupt budget, draining promise jobs after each.
    pub(crate) fn run_due(&self) {
        for _ in 0..MAX_TIMERS_PER_TURN {
            if self.eval_value::<bool>("timer", "__scratchy_timers_run_one()") != Some(true) {
                break;
            }
        }
    }
}

/// Runs `scripts` in order, then `DOMContentLoaded` and `load`. `log`
/// receives `(level, text)` for every `console` call and every script
/// error (level `error`); the real caller prints those to stderr.
#[cfg(test)]
pub(crate) fn run_scripts_with(scripts: &[ScriptSource], log: LogFn) {
    run_scripts_in(&parse_html(""), scripts, log);
}

/// `run_scripts_with` against a real document (Q33b DOM read API).
/// The engine (and so every pending timer) is dropped on return.
pub(crate) fn run_scripts_in(document: &Handle, scripts: &[ScriptSource], log: LogFn) {
    drop(start_scripts(document, scripts, log));
}

/// Q33t: like `run_scripts_in` but hands back the live engine so the
/// caller can drive its timers.
pub(crate) fn start_scripts(document: &Handle, scripts: &[ScriptSource], log: LogFn) -> Option<JsEngine> {
    let Ok(rt) = Runtime::new() else {
        log("error", "could not create the JS runtime");
        return None;
    };
    rt.set_memory_limit(MEMORY_LIMIT);
    let base = Instant::now();
    let deadline = Arc::new(std::sync::atomic::AtomicU64::new((SCRIPT_BUDGET).as_nanos() as u64));
    let dl = deadline.clone();
    rt.set_interrupt_handler(Some(Box::new(move || base.elapsed().as_nanos() as u64 > dl.load(Ordering::Relaxed))));
    let Ok(ctx) = Context::full(&rt) else {
        log("error", "could not create the JS context");
        return None;
    };
    ctx.with(|ctx| {
        let l = log.clone();
        let native = Function::new(ctx.clone(), move |level: String, text: String| l(&level, &text));
        if let Ok(native) = native {
            let _ = ctx.globals().set("__scratchy_log", native);
        }
        let _ = install_env(&ctx);
        let now = Function::new(ctx.clone(), move || base.elapsed().as_secs_f64() * 1000.0);
        if let Ok(now) = now {
            let _ = ctx.globals().set("__scratchy_now", now);
        }
    });
    let dirty = Rc::new(std::cell::Cell::new(false));
    let reqs: Rc<std::cell::RefCell<js_dom::JsRequests>> = Default::default();
    let mut engine = JsEngine { ctx, rt, log, base, deadline, dirty: dirty.clone(), table: Default::default(), reqs: reqs.clone() };
    engine.eval("prelude", PRELUDE);
    engine.eval("timers", TIMERS_PRELUDE);
    engine.table = engine.ctx.with(|ctx| js_dom::install_dom(&ctx, document, dirty.clone(), reqs.clone()));
    let mut script_els = Vec::new();
    collect_script_elements(document, &mut script_els);
    for s in scripts {
        let id = match s.index.and_then(|i| script_els.get(i)) {
            Some(h) => engine.table.borrow_mut().intern(h),
            None => -1,
        };
        engine.eval_value::<()>("script", &format!("__scratchy_cs({id})"));
        engine.eval(&s.name, &s.code);
        engine.eval_value::<()>("script", "__scratchy_cs(-1)");
    }
    engine.eval("DOMContentLoaded", FIRE_DOM_CONTENT_LOADED);
    engine.eval("load", FIRE_LOAD);
    Some(engine)
}

/// Q33t: `setTimeout` & co, as plain JS over a timer table; Rust only
/// asks `__scratchy_timers_next()` (ms of the earliest due time on the
/// `__scratchy_now` clock, -1 if none) and calls `__scratchy_timers_
/// run_one()` (runs the earliest due timer, true if it ran one).
/// Intervals and nested (>5 deep) timeouts are clamped to >= 4ms.
const TIMERS_PRELUDE: &str = r#"
(function (g) {
  var timers = [], nextId = 1, level = 0, FRAME = 1000 / 60;
  var add = function (fn, delay, args, repeat, raf) {
    var id = nextId++;
    delay = Number(delay);
    if (!(delay > 0)) delay = 0;
    if (typeof fn !== 'function') {
      var code = String(fn);
      fn = function () { (0, eval)(code); };
      args = [];
    }
    if (repeat || level > 5) delay = Math.max(delay, 4);
    var t = __scratchy_now();
    if (raf) delay = FRAME - (t % FRAME);
    timers.push({ id: id, fn: fn, args: args, due: t + delay, rep: repeat ? delay : -1, lvl: level + 1, raf: raf });
    return id;
  };
  var clear = function (id) {
    for (var i = 0; i < timers.length; i++) if (timers[i].id === id) { timers.splice(i, 1); return; }
  };
  var def = function (name, fn) {
    Object.defineProperty(g, name, { value: fn, writable: true, configurable: true, enumerable: true });
  };
  def('setTimeout', function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), false); });
  def('setInterval', function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), true); });
  def('clearTimeout', clear);
  def('clearInterval', clear);
  def('requestAnimationFrame', function (fn) {
    if (typeof fn !== 'function') throw new TypeError('requestAnimationFrame: callback is not a function');
    return add(fn, 0, [], false, true);
  });
  def('cancelAnimationFrame', clear);
  Object.defineProperty(g, '__scratchy_timers_next', { value: function () {
    var m = -1;
    for (var i = 0; i < timers.length; i++) if (m < 0 || timers[i].due < m) m = timers[i].due;
    return m;
  } });
  Object.defineProperty(g, '__scratchy_timers_run_one', { value: function () {
    var now = __scratchy_now(), best = -1;
    for (var i = 0; i < timers.length; i++) {
      if (timers[i].due <= now && (best < 0 || timers[i].due < timers[best].due)) best = i;
    }
    if (best < 0) return false;
    var t = timers[best];
    if (t.rep >= 0) t.due = now + t.rep; else timers.splice(best, 1);
    var saved = level;
    level = t.lvl;
    try { t.fn.apply(g, t.raf ? [now] : t.args); }
    catch (e) { console.error('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); }
    level = saved;
    return true;
  } });
})(globalThis);
"#;

/// The real entry point: runs a page's scripts (no-op with JS off or
/// nothing to run) with console output going to stderr.
/// Returns the live engine (Q33t) for the caller to keep for the page.
pub(crate) fn run_page_scripts(document: &Handle, scripts: &[ScriptSource], page_url: Option<&Url>) -> Option<JsEngine> {
    if !js_enabled() || scripts.is_empty() {
        return None;
    }
    set_page_url(page_url);
    start_scripts(
        document,
        scripts,
        Rc::new(|level, text| {
            if level == "error" {
                diag!("scratchy: js error: {text}");
            } else {
                diag!("scratchy: console.{level}: {text}");
            }
        }),
    )
}

/// QJ3: runs `scripts` with a collecting log and returns the report
/// text: every error line, then a summary (scripts run, errors total,
/// errors grouped by first line with counts, most frequent first).
pub(crate) fn js_report_for_scripts(document: &Handle, scripts: &[ScriptSource]) -> String {
    let errors = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let e = errors.clone();
    run_scripts_in(
        document,
        scripts,
        Rc::new(move |level, text| {
            if level == "error" {
                e.borrow_mut().push(text.to_string());
            }
        }),
    );
    let errors = errors.borrow();
    let mut out = String::new();
    for err in errors.iter() {
        out.push_str("js error: ");
        out.push_str(err);
        out.push('\n');
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for err in errors.iter() {
        let first = err.lines().next().unwrap_or("").to_string();
        match groups.iter_mut().find(|(k, _)| *k == first) {
            Some((_, n)) => *n += 1,
            None => groups.push((first, 1)),
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    out.push_str("--- summary ---\n");
    out.push_str(&format!("scripts run: {}\nerrors total: {}\n", scripts.len(), errors.len()));
    for (first, n) in groups {
        out.push_str(&format!("{n:>4}  {first}\n"));
    }
    out
}

/// QJ3: `scratchy --js-report [--socket S] <file>`; headless, no window.
/// With a socket the external scripts are fetched through `wire` (against
/// the page's `<base href>`, else the file's own URL); without one only
/// inline scripts run.
pub(crate) fn js_report_main(args: &[String]) -> i32 {
    let mut socket = None;
    let mut file = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--socket" {
            socket = it.next().map(std::path::PathBuf::from);
        } else {
            file = Some(a.clone());
        }
    }
    let Some(file) = file else {
        diag!("usage: scratchy --js-report [--socket <path>] <file>");
        return 2;
    };
    let bytes = match fs::read(&file) {
        Ok(b) => b,
        Err(e) => {
            diag!("scratchy: {file}: {e}");
            return 2;
        }
    };
    set_js_enabled(true);
    write_raw_stdout(format_args!("{}", js_report_for_html(&decode_html_bytes(&bytes, None), socket.as_deref(), &file)));
    0
}

pub(crate) fn js_report_for_html(html: &str, socket: Option<&std::path::Path>, file: &str) -> String {
    let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return "could not parse page\n".into() };
    set_page_url(file_page_url(file).as_ref());
    let scripts = match (socket, file_page_url(file)) {
        (Some(socket), page_url) => {
            let base = find_base_url(&dom.document).or(page_url);
            match base {
                Some(base) => {
                    let mut res = ExternalResources::default();
                    find_external_resources(&dom.document, &mut res);
                    fetch_scripts(socket, &base, res.scripts)
                }
                None => inline_scripts(&dom.document),
            }
        }
        _ => inline_scripts(&dom.document),
    };
    js_report_for_scripts(&dom.document, &scripts)
}
