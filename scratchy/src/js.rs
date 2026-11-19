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
use crate::js_engine::{self, native, Bytes, Ctx, Engine, EngineOpts, FromJsRet, JsObject, JsResult};
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

pub(crate) fn page_url() -> Option<Url> {
    PAGE_URL.with(|u| u.borrow().clone())
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
    /// QY15: `<script type=module>` with inline code.
    ModuleInline(String),
    /// QY15: `<script type=module src=...>`, `src` unresolved.
    ModuleExternal(String),
    /// QY15: `<script type=importmap>` JSON text.
    ImportMap(String),
}

/// QY15: what a `ScriptSource` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ScriptKind {
    #[default]
    Classic,
    /// An entry module script (run after the classic ones, in order).
    Module,
    /// A pre-fetched module that is only imported, never run by itself.
    ModuleDep,
    /// `code` is the import map JSON.
    ImportMap,
    /// A module script whose fetch failed: `code` is the message to log.
    ModuleError,
}

/// A script ready to run: `name` is only used in error messages.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ScriptSource {
    pub(crate) name: String,
    pub(crate) code: String,
    /// QB8: position among the document's runnable `<script>` elements
    /// (the `find_external_resources` order), so `document.currentScript`
    /// can find the element. `None` = no element (tests).
    pub(crate) index: Option<usize>,
    /// QY15: classic or one of the module kinds.
    pub(crate) kind: ScriptKind,
    /// QY15: the document base URL (resolves relative imports of inline
    /// modules and dynamic `import()` from classic scripts).
    pub(crate) base: Option<String>,
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

/// The `ScriptRef` for a `<script>` element, `None` if it isn't runnable
/// (QY15: classic, module or importmap). `nomodule` classic scripts are
/// skipped (this engine supports modules). `language=` is ignored.
pub(crate) fn script_ref(handle: &Handle) -> Option<ScriptRef> {
    let ty = element_attr(handle, "type");
    let tl = ty.as_deref().map(|t| t.trim().to_ascii_lowercase());
    let module = tl.as_deref() == Some("module");
    let importmap = tl.as_deref() == Some("importmap");
    if !module && !importmap && (!is_classic_script_type(ty.as_deref()) || element_attr(handle, "nomodule").is_some()) {
        return None;
    }
    let src = if importmap { None } else { element_attr(handle, "src") };
    if let Some(src) = src {
        let src = src.trim();
        return (!src.is_empty()).then(|| if module { ScriptRef::ModuleExternal(src.to_string()) } else { ScriptRef::External(src.to_string()) });
    }
    let mut code = String::new();
    for child in handle.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            code.push_str(&contents.borrow());
        }
    }
    if code.trim().is_empty() {
        return None;
    }
    Some(if importmap {
        ScriptRef::ImportMap(code)
    } else if module {
        ScriptRef::ModuleInline(code)
    } else {
        ScriptRef::Inline(code)
    })
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
/// fetch external ones (no `--socket`, no base URL). Inline modules run
/// too (their imports can only be `data:` URLs).
pub(crate) fn inline_scripts(document: &Handle) -> Vec<ScriptSource> {
    let mut res = ExternalResources::default();
    find_external_resources(document, &mut res);
    let base = Url::parse("about:blank").expect("static url");
    let nofetch = |_: &Url| None;
    // Externals become empty placeholders (keeps the indices aligned with
    // the document's script elements), dropped again below.
    let refs: Vec<ScriptRef> = res.scripts.into_iter().map(|r| if matches!(r, ScriptRef::External(_) | ScriptRef::ModuleExternal(_)) { ScriptRef::Inline(String::new()) } else { r }).collect();
    let mut out = fetch_scripts_with(&base, refs, &nofetch);
    out.retain(|s| !(s.kind == ScriptKind::Classic && s.code.is_empty()));
    out
}

/// Resolves every `ScriptRef` against `base` into runnable sources, in
/// document order. External scripts are fetched concurrently (one thread
/// each, joined in order); one that fails to fetch is logged and skipped.
/// QY15: module scripts and the import map are included, followed by the
/// pre-fetched static import graph (`ModuleDep`s).
pub(crate) fn fetch_scripts(path: &std::path::Path, base: &Url, top: &Url, refs: Vec<ScriptRef>) -> Vec<ScriptSource> {
    fetch_scripts_with(base, refs, &|u: &Url| fetch_script_via_wire(path, u, top))
}

pub(crate) fn fetch_scripts_with(base: &Url, refs: Vec<ScriptRef>, fetch: js_modules::Fetcher<'_>) -> Vec<ScriptSource> {
    if !js_enabled() {
        return Vec::new();
    }
    let doc_base = base.to_string();
    let mut scripts: Vec<ScriptSource> = thread::scope(|scope| {
        let handles: Vec<_> = refs
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let doc_base = doc_base.clone();
                scope.spawn(move || {
                    let mk = |name: String, code: String, kind: ScriptKind| ScriptSource { name, code, index: Some(i), kind, base: Some(doc_base.clone()) };
                    match r {
                        ScriptRef::Inline(code) => Some(mk(format!("inline script #{}", i + 1), code, ScriptKind::Classic)),
                        ScriptRef::ImportMap(code) => Some(mk(format!("importmap #{}", i + 1), code, ScriptKind::ImportMap)),
                        ScriptRef::ModuleInline(code) => Some(mk(format!("{doc_base}#inline-module-{}", i + 1), code, ScriptKind::Module)),
                        ScriptRef::External(src) if is_data_url(&src) => match decode_data_uri_text(&src) {
                            Some(code) => Some(mk("data: script".to_string(), code, ScriptKind::Classic)),
                            None => {
                                diag!("scratchy: js: malformed data: script URL");
                                None
                            }
                        },
                        ScriptRef::ModuleExternal(src) => {
                            let url = resolve_href(Some(base), &src).ok()?;
                            let name = url.to_string();
                            let code = if is_data_url(&name) { decode_data_uri_text(&name) } else { fetch(&url) };
                            Some(match code {
                                Some(code) => mk(name, code, ScriptKind::Module),
                                None => {
                                    diag!("scratchy: js: could not fetch module script {url}");
                                    mk(name, "could not fetch the module script".to_string(), ScriptKind::ModuleError)
                                }
                            })
                        }
                        ScriptRef::External(src) => {
                            let url = resolve_href(Some(base), &src).ok()?;
                            match fetch(&url) {
                                Some(code) => Some(mk(url.to_string(), code, ScriptKind::Classic)),
                                None => {
                                    diag!("scratchy: js: could not fetch script {url}");
                                    None
                                }
                            }
                        }
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
    });
    // Classic scripts only matter here for literal dynamic `import('x')`.
    let classic_dynamic = |s: &ScriptSource| s.kind == ScriptKind::Classic && s.code.contains("import(");
    if scripts.iter().any(|s| s.kind == ScriptKind::Module || classic_dynamic(s)) {
        let map = scripts
            .iter()
            .find(|s| s.kind == ScriptKind::ImportMap)
            .and_then(|s| js_modules::ImportMap::parse(&s.code, base))
            .unwrap_or_default();
        let entries: Vec<(String, String)> = scripts
            .iter()
            .filter(|s| s.kind == ScriptKind::Module || classic_dynamic(s))
            .map(|s| (if s.kind == ScriptKind::Module { s.name.clone() } else { doc_base.clone() }, s.code.clone()))
            .collect();
        let deps = js_modules::prefetch_graph(&entries, &map, base, fetch);
        scripts.extend(deps);
    }
    scripts
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
  ['log', 'info', 'debug', 'warn'].forEach(function (l) {
    c[l] = function () { __scratchy_log(l, fmt(Array.prototype.slice.call(arguments))); };
  });
  // QY14: a page's own `console.error` is level `console.error`, so
  // `--js-report` counts it apart from engine-reported errors (level
  // `error`: uncaught exceptions, `not implemented`, rejections).
  c.error = function () { __scratchy_log('console.error', fmt(Array.prototype.slice.call(arguments))); };
  Object.defineProperty(g, '__scratchy_err', { value: function (t) { __scratchy_log('error', String(t)); } });
  var depth = 0, counts = {};
  c.assert = function (cond) {
    if (cond) return;
    var a = Array.prototype.slice.call(arguments, 1);
    __scratchy_log('console.error', 'Assertion failed' + (a.length ? ': ' + fmt(a) : ''));
  };
  c.trace = function () { __scratchy_log('log', 'Trace: ' + fmt(Array.prototype.slice.call(arguments)) + '\n' + (new Error().stack || '')); };
  c.dir = c.dirxml = c.table = function () { __scratchy_log('log', fmt(Array.prototype.slice.call(arguments))); };
  c.group = c.groupCollapsed = function () { if (arguments.length) __scratchy_log('log', fmt(Array.prototype.slice.call(arguments))); depth++; };
  c.groupEnd = function () { if (depth > 0) depth--; };
  c.count = function (l) { l = l === undefined ? 'default' : String(l); counts[l] = (counts[l] || 0) + 1; __scratchy_log('log', l + ': ' + counts[l]); };
  c.clear = c.time = c.timeEnd = c.timeLog = function () {};
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
  // QY6e: callable only through `super()` of a defined custom element class (js_dom `ceNew`).
  var HTMLElement = function HTMLElement() {
    if (!new.target || !g.__ce_new) throw new TypeError('Illegal constructor');
    return g.__ce_new(new.target);
  };
  HTMLElement.prototype = Object.create(Element.prototype, { constructor: { value: HTMLElement, writable: true, configurable: true } });
  g.HTMLElement = HTMLElement;
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
  // QA14: DOMException with the legacy numeric codes. A real Error object
  // (so the JS error log prints `Name: message`) re-prototyped onto
  // DOMException.prototype.
  var DOM_CODES = { IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4, InvalidCharacterError: 5,
    NoModificationAllowedError: 7, NotFoundError: 8, NotSupportedError: 9, InvalidStateError: 11, SyntaxError: 12,
    InvalidModificationError: 13, NamespaceError: 14, InvalidAccessError: 15, TypeMismatchError: 17, SecurityError: 18,
    NetworkError: 19, AbortError: 20, URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23, DataCloneError: 25 };
  var DOM_CONSTS = { INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3, WRONG_DOCUMENT_ERR: 4,
    INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6, NO_MODIFICATION_ALLOWED_ERR: 7, NOT_FOUND_ERR: 8,
    NOT_SUPPORTED_ERR: 9, INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11, SYNTAX_ERR: 12, INVALID_MODIFICATION_ERR: 13,
    NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15, VALIDATION_ERR: 16, TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18,
    NETWORK_ERR: 19, ABORT_ERR: 20, URL_MISMATCH_ERR: 21, QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23,
    INVALID_NODE_TYPE_ERR: 24, DATA_CLONE_ERR: 25 };
  var DOMException = function DOMException(message, name) {
    var e = new Error(message === undefined ? '' : String(message));
    Object.setPrototypeOf(e, DOMException.prototype);
    var n = name === undefined ? 'Error' : String(name);
    Object.defineProperty(e, 'name', { value: n, writable: true, configurable: true });
    Object.defineProperty(e, 'code', { value: DOM_CODES[n] || 0, writable: true, configurable: true });
    return e;
  };
  DOMException.prototype = Object.create(Error.prototype, { constructor: { value: DOMException, writable: true, configurable: true } });
  Object.keys(DOM_CONSTS).forEach(function (k) {
    Object.defineProperty(DOMException, k, { value: DOM_CONSTS[k], enumerable: true });
    Object.defineProperty(DOMException.prototype, k, { value: DOM_CONSTS[k], enumerable: true });
  });
  g.DOMException = DOMException;
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
    this.keyCode = Number(init.keyCode) || 0; this.charCode = Number(init.charCode) || 0;
    this.which = init.which === undefined ? (this.keyCode || this.charCode) : (Number(init.which) || 0);
    this.location = Number(init.location) || 0; this.isComposing = !!init.isComposing;
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
  // QY12: the remaining Event subclasses: name, parent, init-dict defaults.
  var evt = function (name, parent, dflt) {
    var C = ctor(name, parent, function (type, init) {
      parent.call(this, type, init);
      init = init || {};
      for (var k in dflt) this[k] = init[k] === undefined ? (Array.isArray(dflt[k]) ? [] : dflt[k]) : init[k];
    });
    return C;
  };
  var PE = evt('PointerEvent', MouseEvent, { pointerId: 0, width: 1, height: 1, pressure: 0, tangentialPressure: 0,
    tiltX: 0, tiltY: 0, twist: 0, pointerType: '', isPrimary: false });
  PE.prototype.getCoalescedEvents = PE.prototype.getPredictedEvents = function () { return []; };
  evt('WheelEvent', MouseEvent, { deltaX: 0, deltaY: 0, deltaZ: 0, deltaMode: 0 });
  evt('DragEvent', MouseEvent, { dataTransfer: null });
  evt('TouchEvent', UIEvent, { touches: [], targetTouches: [], changedTouches: [], ctrlKey: false, shiftKey: false, altKey: false, metaKey: false });
  evt('CompositionEvent', UIEvent, { data: '' });
  evt('TransitionEvent', Event, { propertyName: '', elapsedTime: 0, pseudoElement: '' });
  evt('AnimationEvent', Event, { animationName: '', elapsedTime: 0, pseudoElement: '' });
  evt('PromiseRejectionEvent', Event, { promise: undefined, reason: undefined });
  evt('HashChangeEvent', Event, { oldURL: '', newURL: '' });
  evt('PopStateEvent', Event, { state: null });
  evt('StorageEvent', Event, { key: null, oldValue: null, newValue: null, url: '', storageArea: null });
  evt('MessageEvent', Event, { data: null, origin: '', lastEventId: '', source: null, ports: [] });
  evt('CloseEvent', Event, { wasClean: false, code: 0, reason: '' });
  evt('ProgressEvent', Event, { lengthComputable: false, loaded: 0, total: 0 });
  evt('ClipboardEvent', Event, { clipboardData: null });
  evt('PageTransitionEvent', Event, { persisted: false });
  evt('SubmitEvent', Event, { submitter: null });
  var normOpts = function (o) {
    if (typeof o === 'boolean') return { capture: o };
    o = o || {};
    return { capture: !!o.capture, once: !!o.once, passive: !!o.passive };
  };
  var oops = function (e) { __scratchy_err('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); };
  // QA12: inline `on<type>="..."` content attributes. Compiled lazily on
  // the first dispatch that reaches the element (recompiled when the
  // attribute text changes) as `function (event) { code }`, so `this` is
  // the element; the body's/frameset's window-forwarded handlers
  // (`onload`, `onresize`, ...) are looked up from <body> when the event
  // reaches the window. Compile errors are logged and remembered.
  var fwd = { load: 1, unload: 1, beforeunload: 1, resize: 1, scroll: 1, hashchange: 1, popstate: 1,
    pageshow: 1, pagehide: 1, online: 1, offline: 1, message: 1, storage: 1, error: 1, blur: 1, focus: 1 };
  var compiled = new WeakMap();
  var attrHandler = function (node, type) {
    var el = node;
    if (node === g) {
      if (!fwd[type] || !g.document) return null;
      el = g.document.body;
    } else if (node.nodeType !== 1 || (fwd[type] && node.__id !== undefined && /^(BODY|FRAMESET)$/.test(node.tagName) && type !== 'focus' && type !== 'blur' && type !== 'error')) {
      return null;
    }
    if (!el || el.__id === undefined) return null;
    var src = __d_attr(el.__id, 'on' + type);
    if (src == null) return null;
    var m = compiled.get(el);
    if (!m) { m = {}; compiled.set(el, m); }
    var c = m[type];
    if (!c || c.src !== src) {
      c = m[type] = { src: src, fn: null };
      try { c.fn = new Function('event', src); } catch (x) { oops(x); }
    }
    return c.fn;
  };
  var dispatch = function (target, ev) {
    ev.target = target;
    ev._stop = false; ev._stopNow = false;
    var path = [target];
    if (target && target.__id !== undefined) {
      // QSD3: a composed event continues from a shadow root to its host (no retargeting yet, QSD8).
      for (var n = target.parentNode; n; n = n.parentNode) path.push(n);
      for (var m = path[path.length - 1]; m && ev.composed && g.__scratchy_shadow_host && (m = g.__scratchy_shadow_host(m)); ) {
        path.push(m);
        for (n = m.parentNode; n; n = n.parentNode) path.push(n);
        m = path[path.length - 1];
      }
      if (path[path.length - 1] === g.document) path.push(g);
    }
    ev._path = path;
    var pl = g.__perf_lis; // QOBS4b: only present while --perf-report is recording
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
          var t0 = pl ? g.__perf_cpu() : 0;
          try {
            if (typeof e.fn === 'function') e.fn.call(node, ev);
            else if (e.fn && typeof e.fn.handleEvent === 'function') e.fn.handleEvent(ev);
          } catch (x) { oops(x); }
          if (pl) {
            var lf = typeof e.fn === 'function' ? e.fn : e.fn && e.fn.handleEvent;
            if (typeof lf === 'function') pl(ev.type, lf, g.__perf_cpu() - t0);
          }
          ev._passive = false;
          if (ev._stopNow) return;
        }
      }
      if (!capFilter && !ev._stopNow) {
        var h = node['on' + ev.type];
        if (typeof h !== 'function') h = attrHandler(node, ev.type);
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
  // QY12: `Window` interface. `Window.prototype` is its own object under
  // EventTarget.prototype (never Object.prototype: that pollutes every object
  // and breaks ShadyDOM); the global object inherits from it.
  var Window = ctor('Window', EventTarget, function () { throw new TypeError('Illegal constructor'); });
  try { Object.setPrototypeOf(g, Window.prototype); } catch (e) {}
  g.top = g.parent = g.frames = g; g.opener = null;
  var Image = ctor('Image', HTMLElement, function (w, h) {
    this.tagName = 'IMG'; this.src = ''; this.width = w || 0; this.height = h || 0;
  });
  var env = g.__scratchy_env || {};
  delete g.__scratchy_env;
  // QSD1b: `window.isSecureContext` (https and loopback hosts) and the Web Crypto
  // subset: `crypto.getRandomValues` / `randomUUID` over the OS RNG (`getrandom(2)`),
  // and a `crypto.subtle` placeholder whose methods log `not implemented` and reject.
  var rndBytes = g.__scratchy_random;
  delete g.__scratchy_random;
  var secure = env.protocol === 'https:' || env.protocol === 'wss:' ||
    /^(localhost|127(\.\d+){3}|\[?::1\]?)$/.test(env.hostname || '') || /\.localhost$/.test(env.hostname || '');
  Object.defineProperty(g, 'isSecureContext', { value: secure, writable: true, configurable: true, enumerable: true });
  var SubtleCrypto = ctor('SubtleCrypto', null, function () { throw new TypeError('Illegal constructor'); });
  var Crypto = ctor('Crypto', null, function () { throw new TypeError('Illegal constructor'); });
  var subtle = Object.create(SubtleCrypto.prototype);
  ['encrypt', 'decrypt', 'sign', 'verify', 'digest', 'generateKey', 'deriveKey', 'deriveBits', 'importKey',
    'exportKey', 'wrapKey', 'unwrapKey'].forEach(function (m) {
    Object.defineProperty(SubtleCrypto.prototype, m, { value: function () {
      g.__scratchy_err('scratchy: js: not implemented: SubtleCrypto.' + m);
      return Promise.reject(new DOMException('SubtleCrypto.' + m + ' is not implemented', 'NotSupportedError'));
    }, writable: true, configurable: true, enumerable: true });
  });
  var INT_ARRAYS = { Int8Array: 1, Uint8Array: 1, Uint8ClampedArray: 1, Int16Array: 1, Uint16Array: 1, Int32Array: 1,
    Uint32Array: 1, BigInt64Array: 1, BigUint64Array: 1 };
  var cryptoObj = Object.create(Crypto.prototype);
  var cproto = Crypto.prototype;
  Object.defineProperty(cproto, 'subtle', { get: function () { return subtle; }, configurable: true, enumerable: true });
  Object.defineProperty(cproto, 'getRandomValues', { value: function getRandomValues(a) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'getRandomValues' on 'Crypto': 1 argument required, but only 0 present.");
    var tag = ArrayBuffer.isView(a) ? Object.prototype.toString.call(a).slice(8, -1) : '';
    if (!INT_ARRAYS[tag]) throw new DOMException("Failed to execute 'getRandomValues' on 'Crypto': The provided ArrayBufferView is not an integer array type.", 'TypeMismatchError');
    if (a.byteLength > 65536) throw new DOMException("Failed to execute 'getRandomValues' on 'Crypto': The ArrayBufferView's byte length (" + a.byteLength + ") exceeds the number of bytes of entropy available via this API (65536).", 'QuotaExceededError');
    if (a.byteLength) new Uint8Array(a.buffer, a.byteOffset, a.byteLength).set(rndBytes(a.byteLength));
    return a;
  }, writable: true, configurable: true, enumerable: true });
  Object.defineProperty(cproto, 'randomUUID', { value: function randomUUID() {
    var b = rndBytes(16);
    b[6] = (b[6] & 0x0f) | 0x40; b[8] = (b[8] & 0x3f) | 0x80;
    var h = '';
    for (var i = 0; i < 16; i++) h += (b[i] < 16 ? '0' : '') + b[i].toString(16);
    return h.slice(0, 8) + '-' + h.slice(8, 12) + '-' + h.slice(12, 16) + '-' + h.slice(16, 20) + '-' + h.slice(20);
  }, writable: true, configurable: true, enumerable: true });
  Object.defineProperty(g, 'crypto', { get: function () { return cryptoObj; }, set: function () {}, configurable: true, enumerable: true });
  var Navigator = ctor('Navigator');
  g.navigator = Object.assign(Object.create(Navigator.prototype), {
    userAgent: env.userAgent, appName: 'Netscape', appVersion: env.appVersion, product: 'Gecko',
    platform: 'Linux x86_64', vendor: '', language: 'en-US', languages: ['en-US', 'en'],
    onLine: true, cookieEnabled: false, hardwareConcurrency: 1, maxTouchPoints: 0,
  });
  g.innerWidth = Number(env.innerWidth); g.innerHeight = Number(env.innerHeight);
  // QY10: MediaQueryList. `matches` is re-evaluated on every read;
  // `change` never fires (the viewport is fixed for the page's life
  // until Q33b2 plumbs the real size and a resize event).
  var MediaQueryList = ctor('MediaQueryList', EventTarget);
  Object.defineProperty(MediaQueryList.prototype, 'matches', { get: function () { return !!__scratchy_media(this.__q)[0]; }, configurable: true, enumerable: true });
  Object.defineProperty(MediaQueryList.prototype, 'media', { get: function () { return __scratchy_media(this.__q)[1]; }, configurable: true, enumerable: true });
  MediaQueryList.prototype.onchange = null;
  MediaQueryList.prototype.addListener = function (fn) { this.addEventListener('change', fn); };
  MediaQueryList.prototype.removeListener = function (fn) { this.removeEventListener('change', fn); };
  g.matchMedia = function (q) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'matchMedia' on 'Window': 1 argument required, but only 0 present.");
    var m = Object.create(MediaQueryList.prototype);
    Object.defineProperty(m, '__q', { value: String(q) });
    return m;
  };
  g.outerWidth = g.innerWidth; g.outerHeight = g.innerHeight; g.devicePixelRatio = 1;
  // QY44b: window.screen / visualViewport. The screen is the viewport
  // (headless: no separate monitor), read live so it stays >= innerWidth.
  var Screen = ctor('Screen'), ScreenOrientation = ctor('ScreenOrientation', EventTarget), VisualViewport = ctor('VisualViewport', EventTarget);
  var orient = Object.create(ScreenOrientation.prototype);
  Object.defineProperty(orient, 'type', { get: function () { return g.innerHeight > g.innerWidth ? 'portrait-primary' : 'landscape-primary'; }, configurable: true, enumerable: true });
  orient.angle = 0; orient.onchange = null;
  orient.lock = function () { return Promise.reject(new Error('screen.orientation.lock() is not supported')); };
  orient.unlock = function () {};
  var scr = Object.create(Screen.prototype);
  [['width', 'innerWidth'], ['availWidth', 'innerWidth'], ['height', 'innerHeight'], ['availHeight', 'innerHeight']].forEach(function (p) {
    Object.defineProperty(scr, p[0], { get: function () { return Number(g[p[1]]) || 0; }, configurable: true, enumerable: true });
  });
  scr.availLeft = 0; scr.availTop = 0; scr.colorDepth = 24; scr.pixelDepth = 24; scr.orientation = orient; scr.isExtended = false;
  Object.defineProperty(g, 'screen', { value: scr, writable: true, configurable: true, enumerable: true });
  g.screenX = g.screenLeft = 0; g.screenY = g.screenTop = 0;
  var vv = Object.create(VisualViewport.prototype);
  [['width', 'innerWidth'], ['height', 'innerHeight']].forEach(function (p) {
    Object.defineProperty(vv, p[0], { get: function () { return Number(g[p[1]]) || 0; }, configurable: true, enumerable: true });
  });
  vv.scale = 1; vv.offsetLeft = 0; vv.offsetTop = 0; vv.onresize = null; vv.onscroll = null;
  Object.defineProperty(vv, 'pageLeft', { get: function () { return Number(g.scrollX) || 0; }, configurable: true, enumerable: true });
  Object.defineProperty(vv, 'pageTop', { get: function () { return Number(g.scrollY) || 0; }, configurable: true, enumerable: true });
  Object.defineProperty(g, 'visualViewport', { value: vv, writable: true, configurable: true, enumerable: true });
  // QF11: `location` is a plain object whose fields are accessors over a
  // private store. The PUBLIC setters (page code) navigate; history.pushState
  // and friends mirror the URL through `__loc_set`, which only writes the store.
  var locSt = { href: env.href, protocol: env.protocol, host: env.host, hostname: env.hostname,
    port: env.port, pathname: env.pathname, search: env.search, hash: env.hash, origin: env.origin };
  var loc = {};
  var noFrag = function (h) { var i = String(h).indexOf('#'); return i < 0 ? String(h) : String(h).slice(0, i); };
  // Resolves `v` against the document base and navigates (replace = no new
  // history entry). A fragment-only change stays in the document.
  var locNav = function (v, replace) {
    var s = String(v), base = (g.document && g.document.baseURI) || locSt.href, href = s;
    if (/^\s*javascript:/i.test(s)) {
      __scratchy_err('scratchy: js: not implemented: javascript: URL navigation ' + JSON.stringify(s.slice(0, 120)));
      return;
    }
    if (base) {
      var u;
      try { u = new g.URL(s, base); } catch (e) { throw new g.DOMException("Failed to parse URL from '" + s + "'", 'SyntaxError'); }
      href = u.href;
      if (u.protocol === 'file:' && locSt.protocol !== 'file:' && locSt.protocol !== '') {
        __scratchy_err('scratchy: js: navigation to ' + JSON.stringify(href.slice(0, 120)) + ' refused: file: from ' + locSt.protocol);
        return;
      }
      if (u.href.indexOf('#') >= 0 && noFrag(u.href) === noFrag(locSt.href) && g.__hist_frag) {
        g.__hist_frag(u.href, !!replace);
        if (replace) g.__d_nav_replace(u.href); else g.__d_navigate(u.href);
        return;
      }
    }
    if (replace) g.__d_nav_replace(href); else g.__d_navigate(href);
  };
  Object.defineProperty(g, '__loc_set', { value: function (u) {
    ['href', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash', 'origin'].forEach(function (k) { locSt[k] = u[k]; });
  } });
  Object.defineProperty(g, '__loc_nav', { value: locNav });
  Object.keys(locSt).forEach(function (k) {
    Object.defineProperty(loc, k, {
      get: function () { return locSt[k]; },
      set: k === 'origin' ? undefined : function (v) {
        if (k === 'href') { locNav(v, false); return; }
        var u;
        try { u = new g.URL(locSt.href); } catch (e) { return; }
        u[k] = String(v);
        locNav(u.href, false);
      },
      configurable: true, enumerable: true });
  });
  loc.toString = function () { return this.href; };
  loc.assign = function assign(u) { locNav(u, false); };
  loc.replace = function replace(u) { locNav(u, true); };
  loc.reload = function reload() { g.__d_reload(); };
  Object.defineProperty(g, 'location', { get: function () { return loc; }, set: function (v) { locNav(v, false); },
    configurable: true, enumerable: true });
  var doc = ({ readyState: 'loading', title: env.title || '', URL: env.href, documentURI: env.href,
    baseURI: env.href, referrer: '', characterSet: 'UTF-8', contentType: 'text/html', compatMode: 'CSS1Compat',
    nodeType: 9, nodeName: '#document' });
  Object.setPrototypeOf(doc, Document.prototype);
  // QF14b: cookies live in wire's jar; the natives talk to its control
  // channel (synchronously). Second documents (they inherit from `doc`) and
  // pages without a cookie origin (file:) read '' and ignore writes. A
  // property of Document.prototype, like a browser's, so polyfills can
  // inspect or override it.
  Object.defineProperty(Document.prototype, 'cookie', {
    get: function () { return (this === g.document && g.__cookie_get) ? String(g.__cookie_get()) : ''; },
    set: function (v) { if (this === g.document && g.__cookie_set) g.__cookie_set(String(v)); },
    configurable: true, enumerable: true });
  Object.defineProperty(doc, 'location', { get: function () { return g.location; }, set: function (v) { g.__loc_nav(v, false); }, configurable: true });
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
  // QV8-4: everything above that came from the page (`env`) or the clock, as one function. The V8
  // startup snapshot bakes this prelude over an about:blank env; a page engine then calls
  // `__scratchy_apply_env(its env)` (the same values the code above computed from `env`).
  var navObj = g.navigator, envHooks = [];
  Object.defineProperty(g, '__scratchy_env_hooks', { value: envHooks, configurable: true });
  Object.defineProperty(g, '__scratchy_apply_env', { value: function (e) {
    var sec = e.protocol === 'https:' || e.protocol === 'wss:' ||
      /^(localhost|127(\.\d+){3}|\[?::1\]?)$/.test(e.hostname || '') || /\.localhost$/.test(e.hostname || '');
    Object.defineProperty(g, 'isSecureContext', { value: sec, writable: true, configurable: true, enumerable: true });
    navObj.userAgent = e.userAgent; navObj.appVersion = e.appVersion;
    g.innerWidth = Number(e.innerWidth); g.innerHeight = Number(e.innerHeight);
    g.outerWidth = g.innerWidth; g.outerHeight = g.innerHeight;
    Object.keys(locSt).forEach(function (k) { locSt[k] = e[k]; });
    [['title', e.title || ''], ['URL', e.href], ['documentURI', e.href], ['baseURI', e.href]].forEach(function (p) {
      var d = Object.getOwnPropertyDescriptor(doc, p[0]);
      if (d && 'value' in d) d.value = p[1], Object.defineProperty(doc, p[0], d); // data properties only (baseURI became an accessor)
    });
    t0 = Date.now();
    Object.keys(timing).forEach(function (k) {
      timing[k] = (k === 'unloadEventStart' || k === 'unloadEventEnd' || k === 'redirectStart' || k === 'redirectEnd') ? 0 : t0;
    });
    perf.timeOrigin = t0;
    envHooks.forEach(function (h) { h(); });
  }, configurable: true });
})(globalThis);
"#;

const FIRE_DOM_CONTENT_LOADED: &str =
    "document.readyState='interactive';__scratchy_dispatch(document,new Event('DOMContentLoaded',{bubbles:true}));";
const FIRE_LOAD: &str = "document.readyState='complete';__scratchy_dispatch(window,new Event('load'));";

/// Total wall-clock budget for one page's scripts; QuickJS is
/// interrupted past it so a runaway loop cannot freeze the window.
/// Sink for console output and script errors: `(level, text)`.
pub(crate) type LogFn = Rc<dyn Fn(&str, &str)>;

/// QOBS3: the names scratchy's own preludes are compiled under (`by_caller` skips their frames).
pub(crate) const PRELUDE_SCRIPTS: &[&str] = &["prelude", "timers", "url", "globals", "intl", "fetch", "websocket", "cssom", "media", "mse", "env", "img-sweep", "prereq", "media-wait", "eval"];

pub(crate) const SCRIPT_BUDGET: Duration = Duration::from_secs(5);
// QB3: budget for one timer or event callback. The page-load run keeps
// `SCRIPT_BUDGET`; a callback runs on the main thread while input is
// flowing, and the compositor drops a client that doesn't read its
// socket for long enough (R8), so it gets a tenth of that. The callback
// budget is `perf::callback_budget()` (500 ms, `--budget-ms` overrides it).

// QY22 test seam: `(now, step)` replaces the thread CPU clock; every read
// returns `now` and then advances it by `step` ns.
#[cfg(test)]
thread_local! {
    pub(crate) static FAKE_CLOCK: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
    /// Runs between the compile and the run of a page-load script.
    pub(crate) static AFTER_COMPILE: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}

/// QY22: a source larger than this is not compiled at all (compile time
/// grows with size, not with page behaviour; 1.4 s per 10 MB measured).
pub(crate) const MAX_SCRIPT_BYTES: usize = 32 * 1024 * 1024;

/// QY5: CPU time this thread has used, in nanoseconds. The interrupt
/// budgets are measured in this, not wall-clock time, so a loaded machine
/// (or a script that merely waits) does not eat a script's budget.
pub(crate) fn thread_cpu_nanos() -> u64 {
    #[cfg(test)]
    if let Some(t) = FAKE_CLOCK.with(|f| f.get().map(|(now, step)| {
        f.set(Some((now + step, step)));
        now
    })) {
        return t;
    }
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out-pointer for the duration of the call.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } != 0 {
        return 0;
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

thread_local! {
    /// QJ4: CPU time spent in host work that must not count against a
    /// script's budget (a forced layout for a geometry read).
    static PAUSED_NANOS: Cell<u64> = const { Cell::new(0) };
}

/// The clock the interrupt budgets run on: thread CPU time minus the time
/// `pause_budget` was told about.
pub(crate) fn script_clock_nanos() -> u64 {
    thread_cpu_nanos().saturating_sub(PAUSED_NANOS.with(Cell::get))
}

/// QJ4: excludes `start..now` (thread CPU nanoseconds from `thread_cpu_nanos`)
/// from the running script's budget.
pub(crate) fn pause_budget_since(start: u64) -> u64 {
    let d = thread_cpu_nanos().saturating_sub(start);
    PAUSED_NANOS.with(|p| p.set(p.get() + d));
    perf::forced_layout(d);
    d
}

/// QSD1b: `n` bytes from the kernel CSPRNG (`getrandom(2)`, via the `libc` crate that
/// is already a dependency; no new crate). Falls back to `/dev/urandom`; if neither
/// works the bytes stay zero and an error is logged to stderr rather than panicking.
fn os_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let mut off = 0;
    while off < n {
        // SAFETY: the pointer/length describe the unfilled tail of `buf`.
        let r = unsafe { libc::getrandom(buf[off..].as_mut_ptr().cast(), n - off, 0) };
        if r <= 0 {
            use std::io::Read;
            if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf[off..])).is_err() {
                eprintln!("scratchy: no OS random source available");
            }
            break;
        }
        off += r as usize;
    }
    buf
}

const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// Defines `__scratchy_env` (plain strings) for the prelude to build
/// `location`, `navigator`, `document.URL` from.
fn install_env(ctx: &Ctx<'_>) -> JsResult<()> {
    let env = JsObject::new(ctx)?;
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
    env.set("userAgent", navigator_user_agent())?;
    env.set("appVersion", navigator_app_version())?;
    env.set("innerWidth", DEFAULT_VIEWPORT.0)?;
    env.set("innerHeight", DEFAULT_VIEWPORT.1)?;
    js_engine::set_global(ctx, "__scratchy_env", env)
}

/// QY9: the eleven URL components the JS `URL` class caches, in order:
/// href, protocol, username, password, host, hostname, port, pathname,
/// search, hash, origin.
fn url_parts(url: &Url) -> Vec<String> {
    let origin = match url.scheme() {
        "http" | "https" | "ftp" | "ws" | "wss" => url.origin().ascii_serialization(),
        _ => "null".to_string(),
    };
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();
    let hostname = url.host_str().unwrap_or("").to_string();
    let host = if port.is_empty() { hostname.clone() } else { format!("{hostname}:{port}") };
    vec![
        url.as_str().to_string(),
        format!("{}:", url.scheme()),
        url.username().to_string(),
        url.password().unwrap_or("").to_string(),
        host,
        hostname,
        port,
        url.path().to_string(),
        url.query().filter(|q| !q.is_empty()).map(|q| format!("?{q}")).unwrap_or_default(),
        url.fragment().filter(|f| !f.is_empty()).map(|f| format!("#{f}")).unwrap_or_default(),
        origin,
    ]
}

/// `__url_parse(input, base)`: components, or `None` when invalid.
fn url_parse_native(input: String, base: Option<String>) -> Option<Vec<String>> {
    let parsed = match base.filter(|b| !b.is_empty()) {
        Some(b) => Url::parse(&b).ok()?.join(&input).ok()?,
        None => Url::parse(&input).ok()?,
    };
    Some(url_parts(&parsed))
}

/// `__url_set(href, component, value)`: apply a setter, return the new
/// components (unchanged ones when the setter rejects the value).
fn url_set_native(href: String, key: String, value: String) -> Option<Vec<String>> {
    let mut u = Url::parse(&href).ok()?;
    match key.as_str() {
        "href" => u = Url::parse(&value).ok()?,
        "protocol" => {
            let _ = u.set_scheme(value.trim_end_matches(':'));
        }
        "username" => {
            let _ = u.set_username(&value);
        }
        "password" => {
            let _ = u.set_password(if value.is_empty() { None } else { Some(&value) });
        }
        "host" => {
            let (h, p) = match value.rsplit_once(':') {
                Some((h, p)) if !value.ends_with(']') && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
                _ => (value.as_str(), None),
            };
            if let (true, Some(p)) = (u.set_host(Some(h)).is_ok(), p) {
                let _ = u.set_port(p.parse().ok());
            }
        }
        "hostname" => {
            let _ = u.set_host(Some(&value));
        }
        "port" => {
            if value.is_empty() {
                let _ = u.set_port(None);
            } else {
                let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(p) = digits.parse::<u16>() {
                    let _ = u.set_port(Some(p));
                }
            }
        }
        "pathname" => u.set_path(&value),
        "search" => {
            let v = value.strip_prefix('?').unwrap_or(&value);
            u.set_query(if value.is_empty() { None } else { Some(v) });
        }
        "hash" => {
            let v = value.strip_prefix('#').unwrap_or(&value);
            u.set_fragment(if value.is_empty() { None } else { Some(v) });
        }
        _ => {}
    }
    Some(url_parts(&u))
}

/// What `eval_value_as` records: the span kind and "this result means nothing ran".
pub(crate) type Recorded<T> = (perf::Kind, fn(&Option<T>) -> bool);

/// Q33t: a page's live QuickJS runtime. Kept in `App` for the page's
/// lifetime so timers can fire after load; dropping it (navigation)
/// discards every pending timer. Field order matters: the context must
/// drop before the runtime.
pub(crate) struct JsEngine {
    /// The engine seam (js_engine.rs). Its `budget` is the interrupt deadline
    /// in thread-CPU nanoseconds (QY5), reset before every top-level evaluation
    /// so each timer callback gets its own budget rather than sharing the
    /// page-load one, and the `tripped` flag the interrupt sets when it stops a
    /// script (so the caller can log a clear message).
    pub(crate) eng: Engine,
    pub(crate) log: LogFn,
    base: Instant,
    /// QM1f: the page's media elements (kept alive with the engine).
    #[allow(dead_code)]
    pub(crate) media: Option<Rc<std::cell::RefCell<media_el::MediaHost>>>,
    /// Q33c: set by every DOM write; the app drains it with `take_dirty`.
    dirty: Rc<std::cell::Cell<bool>>,
    /// Q33d: handle <-> JS id table, to address events at nodes.
    table: Rc<std::cell::RefCell<js_dom::DomTable>>,
    /// QB5: focus/blur/scroll requests for the App.
    pub(crate) reqs: Rc<std::cell::RefCell<js_dom::JsRequests>>,
    /// QF7: page networking state (policy, pending requests, report log).
    pub(crate) net: Rc<std::cell::RefCell<js_net::NetState>>,
    /// QF19w2: the page's WebSockets (closed with code 1001 when the engine drops).
    pub(crate) ws: Rc<std::cell::RefCell<js_ws::WsState>>,
}

/// One init field of a natively dispatched event (Q33d).
pub(crate) enum EventField {
    Bool(bool),
    Num(f64),
    Str(String),
    /// A node, passed as its JS table id (read back as an element).
    Node(Handle),
}

/// `s` as a JS string literal (everything outside printable ASCII is
/// `\u`-escaped, so it is safe in any source).
pub(crate) fn js_string_literal(s: &str) -> String {
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
    pub(crate) fn arm_budget(&self, budget: Duration) {
        self.eng.budget.arm(budget);
    }

    pub(crate) fn tripped(&self) -> bool {
        self.eng.budget.tripped()
    }

    /// QY5: the one log line for a script stopped by the interrupt budget.
    pub(crate) fn log_interrupted(&self, name: &str, what: &str, budget: Duration, size: usize) {
        let b = if budget.as_millis().is_multiple_of(1000) { format!("{} s", budget.as_secs()) } else { format!("{} ms", budget.as_millis()) };
        let kb = if size > 0 { format!(" ({} KB of source)", size.div_ceil(1024)) } else { String::new() };
        (self.log)("error", &format!("js: {name} stopped: exceeded the {b} CPU-time {what} budget{kb}"));
    }

    /// QY22: runs the pending promise jobs of a finished task under one
    /// fresh `CALLBACK_BUDGET` of their own (an expired task deadline must
    /// not kill every queued job at once).
    pub(crate) fn drain_after_task(&self) {
        self.arm_budget(perf::callback_budget());
        self.eng.drain_jobs(&*self.log);
    }

    pub(crate) fn eval(&self, name: &str, code: &str) {
        self.eval_as(None, name, code);
    }

    /// `eval` that records the run as `kind` (a page script or a lifecycle
    /// event) and returns `(compile_cpu_ns, run_cpu_ns, ok)`; the CPU numbers
    /// are only read from the clock when the perf recorder is on.
    pub(crate) fn eval_as(&self, kind: Option<perf::Kind>, name: &str, code: &str) -> (u64, u64, bool) {
        let timing = Cell::new((0u64, 0u64, true));
        if code.len() > MAX_SCRIPT_BYTES {
            (self.log)("error", &format!("js: {name} not compiled: {} MB of source exceeds the {} MB cap", code.len().div_ceil(1 << 20), MAX_SCRIPT_BYTES >> 20));
            return (0, 0, false);
        }
        // QY22: the page-load budget is armed AFTER compiling (compile time
        // follows source size, not page behaviour). The first arm only keeps
        // a runaway compile finite.
        self.arm_budget(perf::script_budget());
        let t0 = thread_cpu_nanos();
        self.eng.with(|ctx| {
            // HTML classic scripts are sloppy (R5): global code, not strict.
            let Some(source) = js_engine::Source::new(code, name) else {
                (self.log)("error", &format!("{name}: source contains a NUL byte"));
                timing.set((0, 0, false));
                return;
            };
            let compiled = source.compile(&ctx);
            let failed = |msg: String, compile: u64, run: u64| {
                if self.tripped() {
                    let ms = |n: u64| format!("{:.1} s", n as f64 / 1e9);
                    (self.log)(
                        "error",
                        &format!(
                            "js: {name} stopped: exceeded the {} s CPU-time page-load budget ({} KB of source; compiled in {}, ran {})",
                            perf::script_budget().as_secs(),
                            code.len().div_ceil(1024),
                            ms(compile),
                            ms(run)
                        ),
                    );
                } else {
                    (self.log)("error", &format!("{name}: {msg}"));
                }
            };
            let Ok(compiled) = compiled else {
                let msg = js_engine::describe_exception(&ctx);
                let now = thread_cpu_nanos();
                failed(msg, now.saturating_sub(t0), 0);
                timing.set((now.saturating_sub(t0), 0, false));
                return;
            };
            #[cfg(test)]
            if let Some(f) = AFTER_COMPILE.with(|c| c.get()) {
                f();
            }
            let t1 = thread_cpu_nanos();
            if kind.is_some() {
                perf::add_compile(t1.saturating_sub(t0));
            }
            self.arm_budget(perf::script_budget());
            let span = perf::begin(kind);
            if let Some(k) = kind {
                perf::set_source(k, || name.to_string());
            }
            let res = compiled.run(&ctx);
            let failed_run = res.is_err();
            perf::end(span, failed_run && self.tripped());
            let run = thread_cpu_nanos().saturating_sub(t1);
            timing.set((t1.saturating_sub(t0), run, !failed_run));
            if failed_run {
                let msg = js_engine::describe_exception(&ctx);
                failed(msg, t1.saturating_sub(t0), run);
            }
        });
        self.eng.drain_jobs(&*self.log);
        timing.get()
    }

    /// QY15: runs the entry module registered under `name` (its source is
    /// in the loader's map) through a dynamic `import()`, so an entry that
    /// other modules also import is evaluated once. A rejection (fetch
    /// failure, syntax error, throw) is logged once as `name: message`.
    fn eval_module(&self, name: &str, size: usize) {
        // The `.then` handler also marks the promise handled, so the QY14
        // unhandled-rejection report does not log the failure a second time.
        let code = format!(
            "(function(p){{var r=globalThis.__scratchy_modr={{s:0}};p.then(function(){{r.s=1;}},function(e){{r.s=2;r.m=(e&&e.name?e.name+': '+e.message:'Uncaught '+String(e))+(e&&e.stack?'\\n'+e.stack:'');}});}})(import({}))",
            js_string_literal(name)
        );
        perf::set_job_kind(Some(perf::Kind::ModuleScript));
        self.eval_as(Some(perf::Kind::ModuleScript), &format!("{name} (import)"), &code);
        perf::set_job_kind(None);
        let failed: Option<String> = self.eng.with(|ctx| {
            let r = js_engine::global_object(&ctx, "__scratchy_modr")?;
            (r.get::<i32>("s")? == 2).then(|| r.get::<String>("m").unwrap_or_default())
        });
        if let Some(msg) = failed {
            if self.tripped() {
                self.log_interrupted(name, "page-load", perf::script_budget(), size);
            } else {
                (self.log)("error", &format!("{name}: {msg}"));
            }
        }
    }

    pub(crate) fn eval_value<T: FromJsRet>(&self, label: &str, code: &str) -> Option<T> {
        self.eval_value_as(None, label, code)
    }

    /// `eval_value` that records the call as `rec.0`; `rec.1` says the result
    /// means "nothing ran" (an idle timer poll), which is then not counted.
    pub(crate) fn eval_value_as<T: FromJsRet>(&self, rec: Option<Recorded<T>>, label: &str, code: &str) -> Option<T> {
        let budget = perf::callback_budget();
        self.arm_budget(budget);
        let span = perf::begin(rec.map(|r| r.0));
        // QOBS4: a timer names its function itself (`__perf_src`); the others carry their label.
        if let Some((k, _)) = rec
            && k != perf::Kind::Timer
        {
            perf::set_source(k, || label.to_string());
        }
        let v = self.eng.with(|ctx| {
            match js_engine::eval_sloppy::<T>(&ctx, label, code) {
                Ok(v) => Some(v),
                Err(e) => {
                    let msg = js_engine::describe_error(&ctx, e);
                    if self.tripped() {
                        self.log_interrupted(label, "callback", budget, 0);
                    } else {
                        (self.log)("error", &format!("{label}: {msg}"));
                    }
                    None
                }
            }
        });
        match rec {
            Some((_, idle)) if idle(&v) => perf::discard(span),
            _ => perf::end(span, v.is_none() && self.tripped()),
        }
        self.drain_after_task();
        v
    }

    /// Whether JS wrote to the DOM since the last call (clears the flag).
    /// QM1g: which video boxes are on screen (node id -> size to render);
    /// every other media element stops rendering. Empty = page hidden.
    pub(crate) fn media_set_targets(&self, targets: &[(usize, crate::media_video::VideoTarget)]) {
        let Some(m) = &self.media else { return };
        let t = self.table.borrow();
        let map: std::collections::HashMap<i32, _> = targets.iter().filter_map(|(n, v)| t.id_of_ptr(*n).map(|i| (i, *v))).collect();
        m.borrow_mut().set_targets(&map);
    }

    /// QM1k1n1: the video nodes that have a laid-out box (visible or not).
    pub(crate) fn media_set_laid(&self, nodes: &[usize]) {
        let Some(m) = &self.media else { return };
        let t = self.table.borrow();
        let set: std::collections::HashSet<i32> = nodes.iter().filter_map(|n| t.id_of_ptr(*n)).collect();
        m.borrow_mut().set_laid(&set);
    }

    /// QM1g: renders waiting frames; the node ids that got a new one.
    pub(crate) fn media_render(&self) -> Vec<usize> {
        let Some(m) = &self.media else { return Vec::new() };
        let ids = m.borrow_mut().render_frames();
        let t = self.table.borrow();
        ids.into_iter().filter_map(|i| t.ptr_of(i)).collect()
    }

    /// QM1g: the newest frames and intrinsic sizes by node id (for `paint`/layout).
    pub(crate) fn media_video_state(&self) -> (crate::media_video::FrameMap, crate::media_video::SizeMap) {
        let mut frames = std::collections::HashMap::new();
        let mut sizes = std::collections::HashMap::new();
        let Some(m) = &self.media else { return (frames, sizes) };
        let t = self.table.borrow();
        for (id, f, sz) in m.borrow().video_state() {
            let Some(p) = t.ptr_of(id) else { continue };
            if let Some(f) = f {
                frames.insert(p, f);
            }
            if sz.0 > 0 {
                sizes.insert(p, sz);
            }
        }
        (frames, sizes)
    }

    /// QM1g2d: what the controls bar shows, read from the element cores (not from JS).
    pub(crate) fn media_controls_state(&self, nodes: &[usize]) -> std::collections::HashMap<usize, crate::media_controls::ControlsState> {
        let mut out = std::collections::HashMap::new();
        let Some(m) = &self.media else { return out };
        let t = self.table.borrow();
        let host = m.borrow();
        for &n in nodes {
            let Some(id) = t.id_of_ptr(n) else { continue };
            let st = match host.peek(id) {
                Some(c) => crate::media_controls::ControlsState { paused: c.paused, current: c.current_time, duration: c.duration, buffered: c.buffered(), volume: c.volume, muted: c.muted, ended: c.ended() },
                None => Default::default(),
            };
            out.insert(n, st);
        }
        out
    }

    /// QM1g2d: runs one controls action on a media element (user gesture).
    pub(crate) fn media_ui(&self, node: usize, op: crate::media_controls::UiOp) -> bool {
        let Some(id) = self.table.borrow().id_of_ptr(node) else { return false };
        let (name, arg) = op.js();
        self.eval_value::<bool>("media-ui", &format!("__scratchy_media_ui({id},'{name}',{arg})")).unwrap_or(false)
    }

    /// QM1g: true once when a video's size became known/changed (relayout).
    pub(crate) fn media_size_changed(&self) -> bool {
        self.media.as_ref().is_some_and(|m| m.borrow_mut().take_size_changed())
    }

    pub(crate) fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    /// QJ4: the App owns the real scroll offset; mirror it into
    /// `window.scrollY` so rect reads are viewport-relative to what the
    /// user sees.
    pub(crate) fn set_scroll(&self, y: f32, els: Option<Vec<(usize, f32)>>) {
        let y = (y as f64).round();
        // QJ6: element offsets the wheel changed, as `{nodeId: y}`.
        let m = els.map_or(String::new(), |v| {
            let body: Vec<String> = v.iter().map(|(k, y)| format!("\"{k}\":{}", y.round())).collect();
            format!("globalThis.__scratchy_elscroll({{{}}});", body.join(","))
        });
        let code = format!("{m}if(globalThis.scrollY!=={y}){{globalThis.scrollY=globalThis.pageYOffset={y};}}true");
        let _ = self.eval_value::<bool>("scroll-sync", &code);
    }

    /// QB5: drains pending focus/blur/scroll requests.
    /// QY48: the App stepped to a same-document history entry `delta` away
    /// whose URL is `href`: schedules `popstate` (with that entry's state).
    /// False when the page's own entry list does not line up (the App then
    /// falls back to loading the URL).
    pub(crate) fn history_traverse(&self, delta: isize, href: &str) -> bool {
        let code = format!("__hist_to({delta},{})", js_string_literal(href));
        self.eval_value::<bool>("history", &code).unwrap_or(false)
    }

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
                EventField::Node(h) => obj.push_str(&self.table.borrow_mut().intern(h).to_string()),
            }
            obj.push(',');
        }
        obj.push('}');
        let code = format!("__scratchy_dn({id},{},{obj})", js_string_literal(kind));
        self.eval_value_as::<bool>(Some((perf::Kind::Event, |_| false)), &format!("event:{kind}"), &code).unwrap_or(true)
    }

    /// When the earliest pending timer is due, `None` if there are none
    /// (the event loop then sleeps with no wake-up at all).
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        if self.eng.is_job_pending() {
            return Some(Instant::now());
        }
        let ms: f64 = self.eval_value("timer", "__scratchy_timers_next()")?;
        if self.eng.is_job_pending() {
            return Some(Instant::now());
        }
        (ms >= 0.0).then(|| self.base + Duration::from_secs_f64(ms / 1000.0))
    }

    /// Test seam: the earliest pending timer's due time in ms on the
    /// `__scratchy_now` clock.
    pub(crate) fn next_due_ms(&self) -> Option<f64> {
        let ms: Option<f64> = self.eval_value("timer", "__scratchy_timers_next()");
        // QY44a: jobs deferred by a budget trip are due right now.
        if self.eng.is_job_pending() {
            return Some(0.0);
        }
        ms.filter(|m| *m >= 0.0)
    }

    /// Runs every timer that is due now (bounded per turn), each under
    /// its own interrupt budget, draining promise jobs after each.
    ///
    /// QY31: a callback that tripped its budget (or whose eval failed) is
    /// counted as run, not as "nothing due": the timer was already consumed
    /// or rescheduled before it ran, so the caller keeps pumping.
    pub(crate) fn run_due_counted(&self) -> usize {
        let mut n = 0;
        let jobs_waiting = self.eng.is_job_pending();
        for _ in 0..MAX_TIMERS_PER_TURN {
            if self.eval_value_as::<bool>(Some((perf::Kind::Timer, |r| *r == Some(false))), "timer", "__scratchy_timers_run_one()") == Some(false) {
                break;
            }
            n += 1;
        }
        // QY44a: deferred promise jobs ran in that call's drain: progress.
        if n == 0 && jobs_waiting {
            n = 1;
        }
        n
    }

    pub(crate) fn run_due(&self) {
        for _ in 0..MAX_TIMERS_PER_TURN {
            if self.eval_value_as::<bool>(Some((perf::Kind::Timer, |r| *r == Some(false))), "timer", "__scratchy_timers_run_one()") != Some(true) {
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
#[cfg(test)]
pub(crate) fn run_scripts_in(document: &Handle, scripts: &[ScriptSource], log: LogFn) {
    drop(start_scripts(document, scripts, log));
}

/// Q33t: like `run_scripts_in` but hands back the live engine so the
/// caller can drive its timers.
pub(crate) fn start_scripts(document: &Handle, scripts: &[ScriptSource], log: LogFn) -> Option<JsEngine> {
    start_scripts_clocked(document, scripts, log, None)
}

/// QY47a: QuickJS's max JS stack: min(4 MB, half of this thread's real
/// stack); 1 MB (the engine default) if the size cannot be read. The clamp
/// keeps a runaway recursion a catchable RangeError instead of a SIGSEGV
/// on small (2 MB test) threads.
pub(crate) fn js_stack_limit() -> usize {
    const DEFAULT: usize = 1 << 20;
    let size = thread_stack_size().unwrap_or(0);
    if size == 0 {
        return DEFAULT;
    }
    (size / 2).min(4 << 20)
}

pub(crate) fn thread_stack_size() -> Option<usize> {
    // SAFETY: pthread_getattr_np fills an attr we init/destroy ourselves.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return None;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let rc = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        (rc == 0).then_some(size)
    }
}

/// `start_scripts` with an optional manual clock (milliseconds) that
/// replaces the real one behind `__scratchy_now`; tests use it to drive
/// timers deterministically. `None` is the real, monotonic clock.
pub(crate) fn start_scripts_clocked(
    document: &Handle,
    scripts: &[ScriptSource],
    log: LogFn,
    clock: Option<Rc<std::cell::Cell<f64>>>,
) -> Option<JsEngine> {
    perf::mark("engine_start");
    let store = Rc::new(js_modules::ModuleStore::from_scripts(scripts, &*log));
    let engine = startup_engine(document, store, log, clock)?;
    perf::mark("engine_ready");
    run_page(engine, document, scripts)
}

/// QV8-4: the V8 startup snapshot's source: scratchy's own startup over an empty document (the
/// engine is returned for `Engine::finish_bake`). `None` if anything logged an error.
#[cfg(feature = "engine-v8")]
pub(crate) fn bake_engine() -> Option<Engine> {
    let document = crate::dom::parse_html("<html><head></head><body></body></html>");
    let failed = Rc::new(Cell::new(false));
    let f = failed.clone();
    let log: LogFn = Rc::new(move |level, text| {
        if level == "error" {
            diag!("scratchy: startup snapshot: {text}");
            f.set(true);
        }
    });
    let store = Rc::new(js_modules::ModuleStore::from_scripts(&[], &*log));
    let engine = startup_engine(&document, store, log, None)?;
    if failed.get() {
        return None;
    }
    let JsEngine { eng, .. } = engine;
    Some(eng)
}

/// Engine, natives and preludes: everything before the page's own scripts. With the V8 startup
/// snapshot the preludes are already in the context (`prebuilt`) and only the page's state is applied.
fn startup_engine(document: &Handle, store: Rc<js_modules::ModuleStore>, log: LogFn, clock: Option<Rc<std::cell::Cell<f64>>>) -> Option<JsEngine> {
    let net_clock = clock.clone();
    let base = Instant::now();
    let eng = match Engine::new(EngineOpts { memory_limit: MEMORY_LIMIT, stack_limit: js_stack_limit(), page: Some(store.clone()) }) {
        Ok(e) => e,
        Err(msg) => {
            log("error", msg);
            return None;
        }
    };
    eng.with(|ctx| {
        let l = log.clone();
        native!(&ctx, "__scratchy_log", move |level: String, text: String| l(&level, &text));
        #[cfg(test)]
        native!(&ctx, "__test_burn", |ms: u64| {
            FAKE_CLOCK.with(|c| c.set(c.get().map(|(n, st)| (n + ms * 1_000_000, st))));
            // QV8-3b: a budget check right after the fake CPU is spent (a no-op on QuickJS, whose
            // interrupt handler runs by itself), so the fake-clock tests do not wait for the
            // V8 watchdog's 2 ms tick.
            js_engine::poll_budget_now();
        });
        let _ = install_env(&ctx);
        if perf::enabled() {
            // QV8-0: observer deliveries report their own spans (see perf.rs).
            native!(&ctx, "__perf_b", perf::observer_begin);
            native!(&ctx, "__perf_e", perf::observer_end);
            native!(&ctx, "__perf_cpu", perf::thread_cpu_ms);
            native!(&ctx, "__perf_lis", |ty: String, f: JsObject<'_>, ms: f64| perf::note_listener(&ty, ms, || f.function_source()));
            native!(&ctx, "__perf_src", |f: JsObject<'_>| perf::set_source(perf::Kind::Timer, || f.function_source()));
        }
        native!(&ctx, "__url_parse", url_parse_native);
        native!(&ctx, "__url_set", url_set_native);
        // QY10: `matchMedia` evaluates through the CSS media-query code.
        native!(&ctx, "__scratchy_media", |q: String| -> Vec<String> {
            let (m, media) = css::media_query_string_matches(&q, DEFAULT_VIEWPORT.0 as f32, DEFAULT_VIEWPORT.1 as f32);
            vec![if m { "1".into() } else { String::new() }, media]
        });
        // QSD1b: OS CSPRNG for `crypto.getRandomValues`/`randomUUID` (<= 65536 bytes per call).
        native!(&ctx, "__scratchy_random", |n: i32| -> Vec<u8> { os_random_bytes(n.clamp(0, 65536) as usize) });
        native!(&ctx, "__scratchy_now", move || match &clock {
            Some(c) => c.get(),
            None => base.elapsed().as_secs_f64() * 1000.0,
        });
    });
    let dirty = Rc::new(std::cell::Cell::new(false));
    let reqs: Rc<std::cell::RefCell<js_dom::JsRequests>> = Default::default();
    let net = Rc::new(std::cell::RefCell::new(js_net::NetState::new(log.clone()).with_clock(net_clock)));
    eng.with(|c| js_net::install_net(&c, net.clone(), reqs.clone()));
    let ws = Rc::new(std::cell::RefCell::new(js_ws::WsState::new(net.clone(), log.clone())));
    eng.with(|c| js_ws::install_ws(&c, ws.clone()));
    let mut engine = JsEngine { eng, log, base, media: None, dirty: dirty.clone(), table: Default::default(), reqs: reqs.clone(), net, ws };
    {
        let (budget, lg) = (engine.eng.budget.clone(), engine.log.clone());
        engine.eng.with(|c| install_script_natives(&c, budget, lg, store));
    }
    let prebuilt = engine.eng.prebuilt();
    if !prebuilt {
        engine.eval("prelude", PRELUDE);
        engine.eval("timers", TIMERS_PRELUDE);
        engine.eval("url", URL_PRELUDE);
        engine.eval("globals", GLOBALS_PRELUDE);
        engine.eval("intl", INTL_PRELUDE);
        engine.eval("fetch", FETCH_PRELUDE);
        engine.eval("websocket", WS_PRELUDE);
    }
    engine.table = engine.eng.with(|ctx| js_dom::install_dom(&ctx, document, dirty.clone(), reqs.clone()));
    if !prebuilt {
        engine.eval("cssom", CSSOM_PRELUDE);
    }
    {
        // QM1f: the media element layer (state machine in media_el.rs).
        let ctxb = media_el::BackendCtx { socket: js_net::net_executor().socket(), policy: engine.net.borrow().policy.clone() };
        let lg = engine.log.clone();
        let host = engine.eng.with(|c| js_media::install_media(&c, ctxb, Rc::new(move |l, t| lg(l, t))));
        engine.media = Some(host);
        if !prebuilt {
            engine.eval("media", js_media::MEDIA_PRELUDE);
            engine.eval("mse", js_media::MSE_PRELUDE);
        }
    }
    if prebuilt {
        // QV8-4: the page's state, over the baked preludes (see PRELUDE's `__scratchy_apply_env`).
        engine.eval("env", "__scratchy_apply_env(__scratchy_env); delete globalThis.__scratchy_env; __media_sweep();");
    }
    Some(engine)
}

/// The page's own scripts and lifecycle events, over a started engine.
fn run_page(engine: JsEngine, document: &Handle, scripts: &[ScriptSource]) -> Option<JsEngine> {
    let mut script_els = Vec::new();
    collect_script_elements(document, &mut script_els);
    for s in scripts.iter().filter(|s| s.kind == ScriptKind::Classic) {
        let id = match s.index.and_then(|i| script_els.get(i)) {
            Some(h) => engine.table.borrow_mut().intern(h),
            None => -1,
        };
        engine.eval_value::<()>("script", &format!("__scratchy_cs({id})"));
        engine.eval_as(Some(perf::Kind::ClassicScript), &s.name, &s.code);
        engine.eval_value::<()>("script", "__scratchy_cs(-1)");
    }
    // QY15: module scripts are deferred: after parsing (and the classic
    // scripts), in document order, before DOMContentLoaded.
    for s in scripts.iter().filter(|s| matches!(s.kind, ScriptKind::Module | ScriptKind::ModuleError)) {
        if s.kind == ScriptKind::ModuleError {
            (engine.log)("error", &format!("{}: {}", s.name, s.code));
        } else {
            engine.eval_module(&s.name, s.code.len());
        }
    }
    engine.eval_as(Some(perf::Kind::Event), "DOMContentLoaded", FIRE_DOM_CONTENT_LOADED);
    perf::mark("dom_content_loaded");
    engine.eval("img-sweep", "__scratchy_img_sweep()");
    engine.eval_as(Some(perf::Kind::Event), "load", FIRE_LOAD);
    perf::mark("load");
    Some(engine)
}

/// QY34: natives for script-inserted `<script>` elements (the JS side lives
/// in js_dom.rs). `__scratchy_run_global(code, name, top) -> bool` runs
/// `code` as sloppy global code (like the page-load path), logging a failure
/// itself; `top` (a script delivered by a network task, nothing else on the
/// stack) arms the `SCRIPT_BUDGET` after compiling exactly as page-load
/// scripts get it, a nested inline run stays under its caller's budget (a
/// trip there propagates, uncatchable, to the caller that logs it).
/// `__scratchy_utf8(bytes)` decodes a response body; `__mod_*` feed the
/// module loader's source map (the inserted module's graph is fetched by
/// the page-side code through the request queue before `import()`).
fn install_script_natives<'js>(ctx: &Ctx<'js>, budget: js_engine::Budget, log: LogFn, store: Rc<js_modules::ModuleStore>) {
    native!(ctx, "__scratchy_run_global", move |ctx: Ctx<'js>, code: String, name: String, top: bool| -> JsResult<bool> {
        if code.len() > MAX_SCRIPT_BYTES {
            log("error", &format!("js: {name} not compiled: {} MB of source exceeds the {} MB cap", code.len().div_ceil(1 << 20), MAX_SCRIPT_BYTES >> 20));
            return Ok(false);
        }
        let Some(source) = js_engine::Source::new(&code, &name) else {
            log("error", &format!("{name}: source contains a NUL byte"));
            return Ok(false);
        };
        let t0 = thread_cpu_nanos();
        if top {
            budget.arm(perf::script_budget());
        }
        let compiled = source.compile(&ctx);
        let fail = |ctx: &Ctx<'js>, t1: u64| -> JsResult<bool> {
            if budget.tripped() {
                if !top {
                    return Err(js_engine::pending_exception());
                }
                js_engine::discard_exception(ctx);
                let ms = |n: u64| format!("{:.1} s", n as f64 / 1e9);
                log(
                    "error",
                    &format!(
                        "js: {name} stopped: exceeded the {} s CPU-time page-load budget ({} KB of source; compiled in {}, ran {})",
                        perf::script_budget().as_secs(),
                        code.len().div_ceil(1024),
                        ms(t1.saturating_sub(t0)),
                        ms(thread_cpu_nanos().saturating_sub(t1))
                    ),
                );
                budget.arm(perf::callback_budget());
            } else {
                let msg = js_engine::describe_exception(ctx);
                log("error", &format!("{name}: {msg}"));
            }
            Ok(false)
        };
        let Ok(compiled) = compiled else {
            return fail(&ctx, thread_cpu_nanos());
        };
        let t1 = thread_cpu_nanos();
        if top {
            budget.arm(perf::script_budget());
            perf::add_compile(t1.saturating_sub(t0));
        }
        let span = perf::begin(top.then_some(perf::Kind::ClassicScript));
        let res = compiled.run(&ctx);
        perf::end(span, res.is_err() && budget.tripped());
        if res.is_err() {
            return fail(&ctx, t1);
        }
        Ok(true)
    });
    native!(ctx, "__scratchy_utf8", |bytes: Bytes| -> String {
        let b = bytes.0.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes.0);
        String::from_utf8_lossy(b).into_owned()
    });
    let st = store.clone();
    native!(ctx, "__mod_has", move |name: String| -> bool { st.sources.borrow().contains_key(&name) });
    let st = store.clone();
    native!(ctx, "__mod_register", move |name: String, code: String| {
        st.sources.borrow_mut().entry(name).or_insert(code);
    });
    // The module specifiers of `code` (as module `name`) that are not in the
    // store yet, resolved to URLs (the graph the page-side code still fetches).
    let st = store;
    native!(ctx, "__mod_deps", move |name: String, code: String| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (spec, _) in js_modules::scan_imports(&code) {
            if let Ok(u) = js_modules::resolve_specifier(&spec, &name, &st.map, &st.doc_base)
                && !is_data_url(&u)
                && !st.sources.borrow().contains_key(&u)
                && !out.contains(&u)
            {
                out.push(u);
            }
        }
        out
    });
}

/// QY9: `URL` and `URLSearchParams`; parsing and setters are Rust natives
/// over the `url` crate, the list/serialization logic is plain JS.
const URL_PRELUDE: &str = include_str!("url_prelude.js");

/// QY13: history, AbortController, MessageChannel, requestIdleCallback,
/// Storage, structuredClone, queueMicrotask (plain JS, see the file).
const GLOBALS_PRELUDE: &str = include_str!("globals_prelude.js");

/// QY16: a minimal en-US `Intl` (QuickJS has none) and the `toLocale*String`
/// methods routed through it. See the file header and TODO.txt QY16a.
const INTL_PRELUDE: &str = include_str!("intl_prelude.js");

/// QSD4: constructable stylesheets and `document.adoptedStyleSheets`.
const CSSOM_PRELUDE: &str = include_str!("cssom_prelude.js");

/// QF2: Blob, File, FileReader, blob: URL store (`__blob_lookup`); the
/// home of the fetch/XHR data classes (QF memo in TODO.txt).
const FETCH_PRELUDE: &str = include_str!("fetch_prelude.js");

/// QF19w2: `WebSocket` (the class; connection and framing are js_ws.rs).
const WS_PRELUDE: &str = include_str!("ws_prelude.js");

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
    var due = t + delay;
    if (raf) {
      // QY30: the next frame boundary STRICTLY after t (a modulo-based
      // delay can round back to t, e.g. at t = 100 ms).
      due = (Math.floor(t / FRAME) + 1) * FRAME;
      while (due <= t) due += FRAME;
    }
    timers.push({ id: id, fn: fn, args: args, due: due, rep: repeat ? delay : -1, lvl: level + 1, raf: raf });
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
    if (g.__scratchy_ioTick) g.__scratchy_ioTick();
    var now = __scratchy_now(), best = -1;
    for (var i = 0; i < timers.length; i++) {
      if (timers[i].due <= now && (best < 0 || timers[i].due < timers[best].due)) best = i;
    }
    if (best < 0) return false;
    var t = timers[best];
    if (t.rep >= 0) t.due = now + t.rep; else timers.splice(best, 1);
    var saved = level;
    level = t.lvl;
    if (g.__perf_src) g.__perf_src(t.fn);
    try { t.fn.apply(g, t.raf ? [now] : t.args); }
    catch (e) { __scratchy_err('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); }
    level = saved;
    if (g.__scratchy_ioTick) g.__scratchy_ioTick();
    return true;
  } });
})(globalThis);
"#;

/// The real entry point: runs a page's scripts (no-op with JS off or
/// nothing to run) with console output going to stderr.
/// Returns the live engine (Q33t) for the caller to keep for the page.
fn has_media_element(h: &Handle) -> bool {
    if let NodeData::Element { name, .. } = &h.data
        && matches!(name.local.as_ref(), "video" | "audio")
    {
        return true;
    }
    h.children.borrow().iter().any(has_media_element)
}

pub(crate) fn run_page_scripts(document: &Handle, scripts: &[ScriptSource], page_url: Option<&Url>) -> Option<JsEngine> {
    // QM1g: a page with media elements needs the engine (the element layer) even
    // without scripts: `<video autoplay muted>` and `onclick="v.play()"` need it.
    if !js_enabled() || (scripts.is_empty() && !(crate::media_video::boxes_enabled() && has_media_element(document))) {
        return None;
    }
    set_page_url(page_url);
    start_scripts(
        document,
        scripts,
        Rc::new(|level, text| {
            if level == "error" {
                diag!("scratchy: js error: {text}");
            } else if level == "net" {
                diag!("scratchy: js net: {text}");
            } else if level == "net-warning" || level == "media" {
                diag!("{text}");
            } else if level.starts_with("console.") {
                diag!("scratchy: {level}: {text}");
            } else {
                diag!("scratchy: console.{level}: {text}");
            }
        }),
    )
}

/// QY14: bounds for the `--run-timers` pump.
const REPORT_MAX_TIMER_CALLBACKS: usize = 5000;

/// QY14: `run_timers = Some(ms)` also pumps the page's timer queue after
/// the scripts ran, on a manual clock (simulated time, no sleeping): jump
/// to each next due time up to `ms`, run the due callbacks (each under the
/// per-callback CPU budget, promise jobs drained after each), at most
/// `REPORT_MAX_TIMER_CALLBACKS`. Errors from that phase are listed and
/// summarised apart from the load-phase ones. `None` is the QJ3 report.
pub(crate) fn js_report_for_scripts_timers(document: &Handle, scripts: &[ScriptSource], run_timers: Option<u64>) -> String {
    let errors = Rc::new(std::cell::RefCell::new(Vec::<(String, String)>::new()));
    let phase = Rc::new(std::cell::Cell::new(false));
    let (e, ph) = (errors.clone(), phase.clone());
    let log: LogFn = Rc::new(move |level, text| {
        if level == "error" || level == "console.error" {
            let kind = if level == "error" { "js error" } else { "console.error" };
            let kind = if ph.get() { format!("{kind} (timers)") } else { kind.to_string() };
            e.borrow_mut().push((kind, text.to_string()));
        }
    });
    let mut pumped = None;
    // QF7: the report has no App; it runs the page's network requests itself.
    let exec = js_net::net_executor();
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let mut net_entries = Vec::new();
    let mut cookie_smoke: Option<String> = None;
    match run_timers {
        None => {
            if let Some(engine) = start_scripts(document, scripts, log) {
                js_net::drive_report_net(&engine, &mut sched, &*exec);
                net_entries = engine.net_entries();
                cookie_smoke = engine.eval_value::<String>("report", "document.cookie");
            }
        }
        Some(limit) => {
            let clock = Rc::new(std::cell::Cell::new(0.0f64));
            let engine = start_scripts_clocked(document, scripts, log, Some(clock.clone()));
            phase.set(true);
            let mut callbacks = 0usize;
            if let Some(engine) = &engine {
                // QM1f: the clock is simulated, so media polls wait a little in real time.
                engine.eval("media-wait", "globalThis.__media_wait = 20");
                js_net::drive_report_net(engine, &mut sched, &*exec);
                while callbacks < REPORT_MAX_TIMER_CALLBACKS {
                    let Some(due) = engine.next_due_ms() else { break };
                    if due > limit as f64 {
                        break;
                    }
                    if due > clock.get() {
                        clock.set(due);
                    }
                    let n = engine.run_due_counted();
                    if n == 0 {
                        // The queue said something was due but nothing ran
                        // (stale due time): stop rather than spin.
                        break;
                    }
                    callbacks += n;
                    js_net::drive_report_net(engine, &mut sched, &*exec);
                }
                net_entries = engine.net_entries();
                cookie_smoke = engine.eval_value::<String>("report", "document.cookie");
            }
            pumped = Some((callbacks, clock.get()));
        }
    }
    let errors = errors.borrow();
    let is_err = |k: &str| k.starts_with("js error");
    let is_timer = |k: &str| k.ends_with("(timers)");
    let mut out = String::new();
    for (kind, err) in errors.iter() {
        out.push_str(kind);
        out.push_str(": ");
        out.push_str(err);
        out.push('\n');
    }
    let group = |sel: &dyn Fn(&str) -> bool| {
        let mut groups: Vec<(String, usize)> = Vec::new();
        for (kind, err) in errors.iter().filter(|(k, _)| sel(k)) {
            let first = err.lines().next().unwrap_or("").to_string();
            match groups.iter_mut().find(|(k, _)| *k == first) {
                Some((_, n)) => *n += 1,
                None => groups.push((first, 1)),
            }
            let _ = kind;
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.1));
        groups
    };
    let total = errors.iter().filter(|(k, _)| is_err(k) && !is_timer(k)).count();
    out.push_str("--- summary ---\n");
    let runnable = scripts.iter().filter(|s| matches!(s.kind, ScriptKind::Classic | ScriptKind::Module)).count();
    out.push_str(&format!("scripts run: {}\nerrors total: {}\n", runnable, total));
    for (first, n) in group(&|k| is_err(k) && !is_timer(k)) {
        out.push_str(&format!("{n:>4}  {first}\n"));
    }
    let ce = errors.iter().filter(|(k, _)| k == "console.error").count();
    if ce > 0 {
        out.push_str(&format!("console.error calls: {ce}\n"));
        for (first, n) in group(&|k| k == "console.error") {
            out.push_str(&format!("{n:>4}  {first}\n"));
        }
    }
    if let Some((callbacks, ms)) = pumped {
        let te = errors.iter().filter(|(k, _)| k == "js error (timers)").count();
        out.push_str(&format!("--- timers (simulated {ms:.0} ms, {callbacks} callbacks) ---\n"));
        out.push_str(&format!("errors in timer phase: {te}\n"));
        for (first, n) in group(&|k| k == "js error (timers)") {
            out.push_str(&format!("{n:>4}  {first}\n"));
        }
        let tce = errors.iter().filter(|(k, _)| k == "console.error (timers)").count();
        if tce > 0 {
            out.push_str(&format!("console.error calls in timer phase: {tce}\n"));
        }
    }
    out.push_str(&js_net::format_net_report(&net_entries));
    out.push_str(&format!("--- cookies ---\ndocument.cookie: {:?}\n", cookie_smoke.unwrap_or_default()));
    out
}

/// QJ3: `scratchy --js-report [--socket S] <file>`; headless, no window.
/// With a socket the external scripts are fetched through `wire` (against
/// the page's `<base href>`, else the file's own URL); without one only
/// inline scripts run.
pub(crate) fn js_report_main(args: &[String]) -> i32 {
    let mut socket = None;
    let mut file = None;
    let mut run_timers = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--socket" {
            socket = it.next().map(std::path::PathBuf::from);
        } else if a == "--user-agent" || a == "--budget-ms" {
            it.next(); // installed by `run` (QY39b, QV8-0)
        } else if a == "--run-timers" {
            run_timers = it.next().and_then(|v| v.parse::<u64>().ok());
        } else {
            file = Some(a.clone());
        }
    }
    js_engine::ensure_snapshot(); // QV8-4: one-shot mode: build the startup snapshot first (V8)
    let Some(file) = file else {
        diag!("usage: scratchy --js-report [--socket <path>] [--user-agent <string>] [--run-timers <ms>] [--budget-ms <ms>] <file>");
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
    write_raw_stdout(format_args!("{}", js_report_for_html_timers(&decode_html_bytes(&bytes, None), socket.as_deref(), &file, run_timers)));
    0
}

#[cfg(test)]
pub(crate) fn js_report_for_html(html: &str, socket: Option<&std::path::Path>, file: &str) -> String {
    js_report_for_html_timers(html, socket, file, None)
}

pub(crate) fn js_report_for_html_timers(html: &str, socket: Option<&std::path::Path>, file: &str, run_timers: Option<u64>) -> String {
    let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return "could not parse page\n".into() };
    // The page URL for origin checks is the copy's `<base href>`, not file://.
    set_page_url(find_base_url(&dom.document).or_else(|| file_page_url(file)).as_ref());
    if let Some(s) = socket {
        js_net::set_net_executor(Arc::new(WireExecutor(s.to_path_buf())));
    }
    let scripts = match (socket, file_page_url(file)) {
        (Some(socket), page_url) => {
            let base = find_base_url(&dom.document).or(page_url);
            match base {
                Some(base) => {
                    let mut res = ExternalResources::default();
                    find_external_resources(&dom.document, &mut res);
                    prefetch_iframes(socket, &base, &base, &dom.document);
                    fetch_scripts(socket, &base, &base, res.scripts)
                }
                None => inline_scripts(&dom.document),
            }
        }
        _ => inline_scripts(&dom.document),
    };
    js_report_for_scripts_timers(&dom.document, &scripts, run_timers)
}
