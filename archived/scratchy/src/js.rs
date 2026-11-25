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
#[cfg(not(test))]
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

thread_local! {
    /// QDOM11: the XML MIME type of the top-level document being run (None: an HTML document).
    static PAGE_XML_MIME: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// QDOM11: marks the page about to run as an XML document of this MIME type (`run_page` turns the document into an
/// XMLDocument before its scripts run and clears the mark).
pub(crate) fn set_page_xml_mime(mime: Option<&str>) {
    PAGE_XML_MIME.with(|m| *m.borrow_mut() = mime.map(crate::js_dom::mime_essence));
}

pub(crate) fn page_url() -> Option<Url> {
    PAGE_URL.with(|u| u.borrow().clone())
}

/// Viewport reported as `window.innerWidth/innerHeight` when there is no window (headless).
const DEFAULT_VIEWPORT: (u32, u32) = (1024, 768);

thread_local! {
    /// Q33b2: the real window size (the App's layout width/height); (0, 0) = no window.
    static WINDOW_VIEWPORT: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
}

/// Q33b2: the App publishes the window size; new engines start with it.
pub(crate) fn set_window_viewport(w: u32, h: u32) {
    WINDOW_VIEWPORT.with(|v| v.set((w, h)));
}

fn env_viewport() -> (u32, u32) {
    let v = WINDOW_VIEWPORT.with(|v| v.get());
    if v.0 == 0 || v.1 == 0 { DEFAULT_VIEWPORT } else { v }
}

#[cfg(not(test))]
static JS_ENABLED: AtomicBool = AtomicBool::new(true);
// QFLK1: under `cargo test` every test runs on its own thread, so the flag is per thread; a test that
// flips it (q33a, qy19) cannot make a parallel test see "scripts run: 0" (load-sensitive flake).
#[cfg(test)]
thread_local! { static JS_ENABLED_T: std::cell::Cell<bool> = const { std::cell::Cell::new(true) }; }

/// Whether scripts run (and `<noscript>` is hidden). `--no-js` / `:set
/// nojs` turn it off; a `:set` change applies from the next page load.
pub(crate) fn js_enabled() -> bool {
    #[cfg(test)]
    return JS_ENABLED_T.with(|c| c.get());
    #[cfg(not(test))]
    JS_ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn set_js_enabled(on: bool) {
    #[cfg(test)]
    JS_ENABLED_T.with(|c| c.set(on));
    #[cfg(not(test))]
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
    /// QSCR404: a classic external script whose fetch failed (network error or a non-2xx
    /// status): it is not run, the element gets `error`. `code` is the message to log.
    ClassicError,
    /// QSCR404: a module that a static import could not fetch (`name` = its URL). Never run;
    /// an entry module whose static graph reaches it fails (`error` on its element).
    ModuleMissing,
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
    // QDOM11: an SVG script names its file in `href` / `xlink:href`.
    let src = if importmap { None } else if script_namespace(handle) == Some(SVG_NS) { element_attr(handle, "href") } else { element_attr(handle, "src") };
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

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// QDOM11: the namespace of a `<script>` element: only the XHTML and SVG ones are scripts (a `script` in no
/// namespace, in an XML document, is just data). The local name may carry a prefix (`svg:script`, QDOM14).
pub(crate) fn script_namespace(handle: &Handle) -> Option<&'static str> {
    let NodeData::Element { name, .. } = &handle.data else { return None };
    let local = name.local.as_ref();
    if local.rsplit(':').next() != Some("script") {
        return None;
    }
    match name.ns.as_ref() {
        "http://www.w3.org/1999/xhtml" => Some("http://www.w3.org/1999/xhtml"),
        SVG_NS => Some(SVG_NS),
        _ => None,
    }
}

/// QB8: every runnable classic `<script>` element in document order, the
/// same order and filter as `find_external_resources`.
fn collect_script_elements(handle: &Handle, out: &mut Vec<Handle>) {
    if script_namespace(handle).is_some() && script_ref(handle).is_some()
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
                                    Some(mk(url.to_string(), "failed to load script".to_string(), ScriptKind::ClassicError))
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
    // QDOM15: an event interface is not callable without `new` (a plain call has a `this` that is not an instance).
    var chk = init && /Event$/.test(name) ? "if (!(this instanceof " + name + ")) throw new TypeError(\"Failed to construct '" + name + "': Please use the 'new' operator, this DOM object constructor cannot be called as a function.\"); " : '';
    var f = new Function('init', 'return function ' + name + '() { ' + chk + 'if (init) init.apply(this, arguments); };')(init);
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
    NetworkError: 19, AbortError: 20, URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23, InvalidNodeTypeError: 24, DataCloneError: 25 };
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
  [['ELEMENT_NODE', 1], ['ATTRIBUTE_NODE', 2], ['TEXT_NODE', 3], ['CDATA_SECTION_NODE', 4], ['ENTITY_REFERENCE_NODE', 5],
    ['ENTITY_NODE', 6], ['PROCESSING_INSTRUCTION_NODE', 7], ['COMMENT_NODE', 8], ['DOCUMENT_NODE', 9], ['DOCUMENT_TYPE_NODE', 10],
    ['DOCUMENT_FRAGMENT_NODE', 11], ['NOTATION_NODE', 12]]
    .forEach(function (kv) {
      var dsc = { value: kv[1], enumerable: true };
      Object.defineProperty(Node, kv[0], dsc); Object.defineProperty(Node.prototype, kv[0], dsc);
    });
  // Q33d: events. One listener store (a WeakMap keyed by the target
  // object) serves window, document, nodes and bare `new EventTarget()`.
  var store = new WeakMap();
  var lists = function (t, type, create) {
    var m = store.get(t);
    if (!m) { if (!create) return null; m = {}; store.set(t, m); }
    return m[type] || (create ? (m[type] = []) : null);
  };
  // QDOM15: WebIDL conversions for the event init dictionaries.
  var toStr = function (v) { if (typeof v === 'symbol') throw new TypeError('Cannot convert a Symbol value to a string'); return String(v); };
  var dictOf = function (d) {
    if (d === undefined || d === null) return {};
    if (typeof d !== 'object' && typeof d !== 'function') throw new TypeError('The provided value is not of type of an event init dictionary.');
    return d;
  };
  var toDbl = function (v) { v = Number(v); if (!isFinite(v)) throw new TypeError('The provided double value is non-finite.'); return v; };
  var toLong = function (v) { return Number(v) | 0; };
  var toUShort = function (v) { return (Number(v) >>> 0) & 0xFFFF; };
  var toULong = function (v) { return Number(v) >>> 0; };
  var isWin = function (v) { return v === g || (v !== null && typeof v === 'object' && v.window === v && v.document !== undefined); };
  var isET = function (v) { return v !== null && (typeof v === 'object' || typeof v === 'function') && (v instanceof EventTarget || (v.addEventListener !== undefined && v.dispatchEvent !== undefined)); };
  var Event = ctor('Event', null, function (type, init) {
    if (arguments.length < 1) throw new TypeError("Failed to construct 'Event': 1 argument required, but only 0 present.");
    type = toStr(type);
    init = dictOf(init);
    this.type = type;
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.composed = !!init.composed;
    this.defaultPrevented = false;
    this.target = null;
    this.currentTarget = null;
    this.eventPhase = 0;
    this.timeStamp = __scratchy_now();
    // QDOM4: the DOM event flags (stop propagation, stop immediate, dispatch, initialized) are
    // hidden slots; `isTrusted` is one own, unforgeable (non-configurable) accessor.
    var self = this;
    ['_stop', '_stopNow', '_disp', '_passive'].forEach(function (k) { Object.defineProperty(self, k, { value: false, writable: true }); });
    Object.defineProperty(this, '_path', { value: [], writable: true });
    Object.defineProperty(this, '_init', { value: true, writable: true });
    Object.defineProperty(this, 'isTrusted', { get: isTrustedGet, enumerable: true, configurable: false });
  });
  var trusted = new WeakSet();
  var isTrustedGet = function () { return trusted.has(this); };
  Object.defineProperty(g, '__scratchy_trust', { value: function (ev) { trusted.add(ev); } });
  [['NONE', 0], ['CAPTURING_PHASE', 1], ['AT_TARGET', 2], ['BUBBLING_PHASE', 3]].forEach(function (kv) {
    var dsc = { value: kv[1], enumerable: true };
    Object.defineProperty(Event, kv[0], dsc); Object.defineProperty(Event.prototype, kv[0], dsc);
  });
  Event.prototype.preventDefault = function () { if (this.cancelable && !this._passive) this.defaultPrevented = true; };
  Event.prototype.stopPropagation = function () { this._stop = true; };
  Event.prototype.stopImmediatePropagation = function () { this._stop = true; this._stopNow = true; };
  Event.prototype.composedPath = function () { return this._disp && this._path ? this._path.slice() : []; };
  Object.defineProperty(Event.prototype, 'cancelBubble', { get: function () { return !!this._stop; },
    set: function (v) { if (v) this._stop = true; }, configurable: true });
  Object.defineProperty(Event.prototype, 'returnValue', { get: function () { return !this.defaultPrevented; },
    set: function (v) { if (!v) this.preventDefault(); }, configurable: true });
  Object.defineProperty(Event.prototype, 'srcElement', { get: function () { return this.target; }, configurable: true });
  var CustomEvent = ctor('CustomEvent', Event, function (type, init) {
    Event.call(this, type, init);
    this.detail = init && init.detail !== undefined ? init.detail : null;
  });
  var UIEvent = ctor('UIEvent', Event, function (type, init) {
    Event.call(this, type, init);
    init = dictOf(init);
    var view = init.view === undefined ? null : init.view;
    if (view !== null && !isWin(view)) throw new TypeError("Failed to construct 'UIEvent': member view is not of type Window.");
    var detail = init.detail === undefined ? 0 : toLong(init.detail);
    var which = init.which === undefined ? undefined : toULong(init.which);
    this.view = view; this.detail = detail;
    if (which !== undefined) this.which = which;
  });
  var relT = function (v) {
    if (v === undefined || v === null) return null;
    if (!isET(v)) throw new TypeError('member relatedTarget is not of type EventTarget.');
    return v;
  };
  var modKeys = function (self, init) {
    ['ctrlKey', 'shiftKey', 'altKey', 'metaKey'].forEach(function (k) { self[k] = !!init[k]; });
  };
  var MouseEvent = ctor('MouseEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = dictOf(init);
    var self = this;
    modKeys(this, init);
    ['clientX', 'clientY', 'screenX', 'screenY', 'movementX', 'movementY'].forEach(function (k) { self[k] = init[k] === undefined ? 0 : toDbl(init[k]); });
    this.button = init.button === undefined ? 0 : (Number(init.button) << 16 >> 16);
    this.buttons = init.buttons === undefined ? 0 : toUShort(init.buttons);
    this.pageX = init.pageX === undefined ? this.clientX : toDbl(init.pageX);
    this.pageY = init.pageY === undefined ? this.clientY : toDbl(init.pageY);
    this.relatedTarget = relT(init.relatedTarget);
  });
  // QWPT7c: offsetX/Y are relative to the padding edge of the event target (of the outermost <svg>
  // for an SVG element); client coordinates without an element target. Computed on read.
  ['X', 'Y'].forEach(function (ax, i) {
    Object.defineProperty(MouseEvent.prototype, 'offset' + ax, { enumerable: true, configurable: true, get: function () {
      var c = i ? this.clientY : this.clientX, t = this.target;
      if (!this.__pos || !t || t.nodeType !== 1 || !t.getBoundingClientRect) return c;
      var e = t;
      for (var p = t.parentNode; p && p.nodeType === 1; p = p.parentNode) if (p.namespaceURI === 'http://www.w3.org/2000/svg' && p.localName === 'svg') e = p;
      if (e.namespaceURI === 'http://www.w3.org/2000/svg' && e.localName !== 'svg') return c;
      var r = e.getBoundingClientRect();
      return c - (i ? r.top + (e.clientTop || 0) : r.left + (e.clientLeft || 0));
    } });
  });
  var KeyboardEvent = ctor('KeyboardEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = dictOf(init);
    modKeys(this, init);
    this.key = init.key === undefined ? '' : toStr(init.key);
    this.code = init.code === undefined ? '' : toStr(init.code);
    this.location = init.location === undefined ? 0 : toULong(init.location);
    this.repeat = !!init.repeat;
    this.isComposing = !!init.isComposing;
    this.charCode = init.charCode === undefined ? 0 : toULong(init.charCode);
    this.keyCode = init.keyCode === undefined ? 0 : toULong(init.keyCode);
    this.which = init.which === undefined ? (this.keyCode || this.charCode) : toULong(init.which);
  });
  // QY4/QA15: legacy init*Event methods and ErrorEvent.
  Event.prototype.initEvent = function (type, bubbles, cancelable) {
    if (this._disp) return;
    this._init = true; this._stop = false; this._stopNow = false; this.defaultPrevented = false; this.target = null;
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
    this.pageX = this.clientX; this.pageY = this.clientY; 
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
    this.relatedTarget = relT(dictOf(init).relatedTarget);
  });
  ctor('InputEvent', UIEvent, function (type, init) {
    UIEvent.call(this, type, init);
    init = dictOf(init);
    this.data = init.data === undefined || init.data === null ? null : toStr(init.data);
    this.inputType = init.inputType === undefined ? '' : toStr(init.inputType);
    this.isComposing = !!init.isComposing;
  });
  // QY12: the remaining Event subclasses: name, parent, init-dict defaults.
  var evt = function (name, parent, dflt, conv) {
    var C = ctor(name, parent, function (type, init) {
      parent.call(this, type, init);
      init = dictOf(init);
      for (var k in dflt) this[k] = init[k] === undefined ? (Array.isArray(dflt[k]) ? [] : dflt[k]) : (conv && conv[k] ? conv[k](init[k]) : init[k]);
    });
    return C;
  };
  var PE = evt('PointerEvent', MouseEvent, { pointerId: 0, width: 1, height: 1, pressure: 0, tangentialPressure: 0,
    tiltX: 0, tiltY: 0, twist: 0, pointerType: '', isPrimary: false });
  PE.prototype.getCoalescedEvents = PE.prototype.getPredictedEvents = function () { return []; };
  evt('WheelEvent', MouseEvent, { deltaX: 0, deltaY: 0, deltaZ: 0, deltaMode: 0 }, { deltaX: toDbl, deltaY: toDbl, deltaZ: toDbl, deltaMode: toULong });
  evt('DragEvent', MouseEvent, { dataTransfer: null }, { dataTransfer: function (v) {
    if (v === null) return null;
    if (typeof g.DataTransfer !== 'function' || !(v instanceof g.DataTransfer)) throw new TypeError("Failed to construct 'DragEvent': Failed to read the 'dataTransfer' property from 'DragEventInit': The provided value is not of type 'DataTransfer'.");
    return v;
  } });
  evt('TouchEvent', UIEvent, { touches: [], targetTouches: [], changedTouches: [], ctrlKey: false, shiftKey: false, altKey: false, metaKey: false });
  evt('CompositionEvent', UIEvent, { data: '' }, { data: toStr });
  // QDOM15: legacy `createEvent` interfaces.
  evt('TextEvent', UIEvent, { data: '' }, { data: toStr });
  evt('BeforeUnloadEvent', Event, {});
  evt('DeviceMotionEvent', Event, { acceleration: null, accelerationIncludingGravity: null, rotationRate: null, interval: 0 });
  evt('DeviceOrientationEvent', Event, { alpha: null, beta: null, gamma: null, absolute: false });
  evt('MediaQueryListEvent', Event, { media: '', matches: false }, { media: toStr, matches: function (v) { return !!v; } });
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
    return { capture: !!o.capture, once: !!o.once, passive: o.passive === undefined ? undefined : !!o.passive };
  };
  var oops = function (e) { __scratchy_err('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); };
  // QA12 + QDOM13: event handlers (HTML "event handlers"). One slot per (target, event type): `v` is
  // the value set through the IDL attribute (a function, or null; undefined = not set, so the content
  // attribute applies), `asrc` the content attribute text at that moment (a later change of the
  // attribute wins), `entry` the listener-list entry registered when a non-null value was first set
  // (so a handler runs in registration order among the listeners). A content attribute that was
  // never overridden is compiled lazily as `function (event) { code }` (recompiled when the text
  // changes) and runs after the listeners. `<body>`/`<frameset>` forward the Window-level names to
  // the Window (`fwdTypes`); the Window's attribute value comes from `slot.el` (the last body /
  // frameset whose attribute was set, `__scratchy_attr_changed`) or else `document.body`.
  var fwdTypes = {};
  'blur error focus load resize scroll afterprint beforeprint beforeunload hashchange languagechange message messageerror offline online pagehide pageshow popstate rejectionhandled storage unhandledrejection unload'
    .split(' ').forEach(function (k) { fwdTypes[k] = 1; });
  var compiled = new WeakMap(), hslots = new WeakMap();
  var isFwdEl = function (n, type) {
    return !!(fwdTypes[type] && n !== g && n && n.nodeType === 1 && n.__id !== undefined && /^(BODY|FRAMESET)$/.test(n.tagName));
  };
  var hslot = function (t, type, create) {
    var m = hslots.get(t);
    if (!m) { if (!create) return null; m = {}; hslots.set(t, m); }
    return m[type] || (create ? (m[type] = { v: undefined, asrc: null, el: null, entry: null }) : null);
  };
  var hsrc = function (t, s, type) {
    var el = t === g ? (s && s.el) || (g.document && g.document.body) : t;
    // QDOM4: scripting is disabled in a document without a browsing context (DOMParser, createHTMLDocument).
    if (!el || el.nodeType !== 1 || el.__id === undefined || el.ownerDocument !== g.document) return [null, null];
    return [el, __d_getns(el.__id, '', ('on' + type).toLowerCase())];
  };
  var getH = function (t, type) {
    var s = hslot(t, type, false), r = hsrc(t, s, type), el = r[0], src = r[1];
    if (s && s.v !== undefined) {
      if (s.asrc === src) return s.v;
      s.v = undefined;
    }
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
  // QCUR1: how many listeners exist per pointer-motion event type, so Rust only builds
  // and dispatches those events when a page can see them. Bit i of the mask = PTR_TYPES[i].
  var PTR_TYPES = ['mousemove', 'pointermove', 'mouseenter', 'mouseleave', 'pointerover', 'pointerout', 'pointerenter', 'pointerleave', 'wheel'];
  var ptrCount = {};
  var setH = function (t, type, fn) {
    var s = hslot(t, type, true), v = typeof fn === 'function' ? fn : null;
    s.v = v; s.asrc = hsrc(t, s, type)[1];
    var l;
    if (v) {
      if (!s.entry) {
        s.entry = { fn: s, h: true, capture: false, once: false, passive: false, removed: false };
        lists(t, type, true).push(s.entry);
        if (PTR_TYPES.indexOf(type) >= 0) ptrCount[type] = (ptrCount[type] | 0) + 1;
      }
    } else if (s.entry) {
      s.entry.removed = true;
      if ((l = lists(t, type, false))) { var i = l.indexOf(s.entry); if (i >= 0) l.splice(i, 1); }
      s.entry = null;
      if (ptrCount[type]) ptrCount[type]--;
    }
  };
  // The DOM prelude calls this after setAttribute/removeAttribute of an `on*` attribute: on a body/frameset
  // the Window's handler takes the element's attribute (HTML "event handler content attributes").
  Object.defineProperty(g, '__scratchy_attr_changed', { value: function (el, name) {
    var type = String(name).slice(2), s;
    // setting the attribute re-activates the handler even when the text is the same
    if (isFwdEl(el, type) && el.ownerDocument === g.document) { s = hslot(g, type, true); s.v = undefined; s.el = el; }
    if ((s = hslot(el, type, false))) s.v = undefined;
  } });
  // QDOM5d: HTML "document open steps" 11-13: the listeners and event handlers of the Document and the Window are erased.
  Object.defineProperty(g, '__scratchy_erase_listeners', { value: function (t) {
    var m = store.get(t), k;
    if (m) { for (k in m) m[k].forEach(function (e) { e.removed = true; }); store.delete(t); }
    var h = hslots.get(t);
    if (h) { for (k in h) if (h[k].entry) h[k].entry.removed = true; hslots.delete(t); }
  } });
  var callHandler = function (node, h, ev) {
    try {
      if (node === g && ev.type === 'error' && ev instanceof g.ErrorEvent) {
        // QERR1: window.onerror takes the 5-argument form; `true` cancels.
        if (h.call(node, ev.message, ev.filename, ev.lineno, ev.colno, ev.error) === true) ev.preventDefault();
      } else if (h.call(node, ev) === false) ev.preventDefault();
    } catch (x) { reportListenerError(x); }
  };
  // QDOM4: a listener exception is reported (window `error` event / onerror, the QERR1 path) and
  // dispatch continues; an unhandled one is logged as before.
  var reportListenerError = function (x) {
    var r = false;
    try { r = g.__scratchy_report_error(x, '', 0, 0); } catch (y) {}
    if (!r) oops(x);
  };
  // QDOM4/QDOM10: `window.event` is the event whose listener is running.
  var curEvent;
  Object.defineProperty(g, 'event', { get: function () { return curEvent; },
    set: function (v) { Object.defineProperty(g, 'event', { value: v, writable: true, configurable: true, enumerable: true }); },
    configurable: true, enumerable: true });
  // QEL6: an event born in Rust (__scratchy_dn) is dispatched with an empty JS stack, so HTML "clean up after
  // running script" runs a microtask checkpoint after each listener's callback. A script-made dispatch
  // (el.click(), dispatchEvent) has script below it on the stack: no checkpoint there.
  var uaEv = null;
  Object.defineProperty(g, '__scratchy_ua', { value: function (ev) { uaEv = ev; } });
  var dispatch = function (target, ev) {
    ev.target = target;
    var ua = uaEv === ev;
    uaEv = null;
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
    ev._path = path; ev._disp = true;
    var pl = g.__perf_lis; // QOBS4b: only present while --perf-report is recording
    // capFilter: true = capture listeners only, false = non-capture only.
    // QDOM13: the legacy prefixed names of UA-dispatched animation/transition events (DOM "inner invoke" step 8).
    var LEGACY = { animationstart: 'webkitAnimationStart', animationiteration: 'webkitAnimationIteration',
      animationend: 'webkitAnimationEnd', transitionend: 'webkitTransitionEnd' };
    // The handler a node runs for `type` (null: none). A body/frameset's Window-level handlers belong to
    // the Window; its own `focus`/`blur`/`error` (the window-focus event is dispatched at the body) use them.
    var handlerOf = function (node, type) {
      var fe = isFwdEl(node, type);
      if (fe && type !== 'focus' && type !== 'blur' && type !== 'error') return null;
      var h = getH(fe ? g : node, type);
      if (typeof h !== 'function' && !fe) h = node['on' + type];
      return typeof h === 'function' ? h : null;
    };
    // QDOM10: DOM "inner invoke" leaves window.event alone for a listener whose invocation target is in a shadow
    // tree. Path nodes up to the last shadow root are in one (lazy: computed at the first listener that runs).
    var shMax = -2;
    var inShadow = function (node) {
      if (shMax === -2) {
        shMax = -1;
        if (g.__scratchy_shadow_host) for (var k = path.length - 1; k >= 0; k--) {
          var pn = path[k];
          if (pn && pn.nodeType === 11 && g.__scratchy_shadow_host(pn)) { shMax = k; break; }
        }
      }
      if (shMax < 0) return false;
      for (var q = 0; q <= shMax; q++) if (path[q] === node) return true;
      return false;
    };
    var found = function (node, type, capFilter) {
      var l = lists(node, type, false);
      if (l) for (var j = 0; j < l.length; j++) if (!l[j].removed && l[j].capture === capFilter) return true;
      return !capFilter && !!handlerOf(node, type);
    };
    var run = function (node, type, capFilter) {
      var l = lists(node, type, false), hs = null;
      if (l) {
        var snap = l.slice();
        for (var i = 0; i < snap.length; i++) {
          var e = snap[i];
          if (e.removed || e.capture !== capFilter) continue;
          if (e.h) {
            // an IDL event handler registered in the list
            hs = e;
            var hf = handlerOf(node, type);
            if (hf) { ev._passive = false; var pe = curEvent; if (!inShadow(node)) curEvent = ev; callHandler(node, hf, ev); curEvent = pe; if (ua) g.__scratchy_microtasks(); }
            if (ev._stopNow) return;
            continue;
          }
          if (e.once) node.removeEventListener(type, e.fn, e.capture);
          ev._passive = e.passive;
          var t0 = pl ? g.__perf_cpu() : 0;
          var lf = e.fn, prevEv = curEvent;
          if (!inShadow(node)) curEvent = ev;
          try {
            // QDOM4: handleEvent is looked up at every invoke; a non-callable one is a reported TypeError.
            if (typeof lf === 'function') lf.call(node, ev);
            else { lf = lf.handleEvent; if (typeof lf !== 'function') throw new TypeError('handleEvent is not a function'); lf.call(e.fn, ev); }
          } catch (x) { reportListenerError(x); }
          curEvent = prevEv;
          if (ua) g.__scratchy_microtasks();
          if (pl && typeof lf === 'function') pl(type, lf, g.__perf_cpu() - t0);
          ev._passive = false;
          if (ev._stopNow) return;
        }
      }
      // a content attribute (or plain `node.onfoo` property) that was never set through the IDL attribute
      if (!capFilter && !ev._stopNow && !hs) {
        var h = handlerOf(node, type);
        if (h) { var pv = curEvent; if (!inShadow(node)) curEvent = ev; callHandler(node, h, ev); curEvent = pv; if (ua) g.__scratchy_microtasks(); }
      }
    };
    var invoke = function (node, phase, capFilter) {
      if (ev._stop) return;
      ev.currentTarget = node; ev.eventPhase = phase;
      var type = ev.type, lt = LEGACY[type];
      if (lt && ev.isTrusted && !found(node, type, capFilter) && found(node, lt, capFilter)) type = lt;
      var savedType = ev.type;
      ev.type = type;
      try { run(node, type, capFilter); } finally { ev.type = savedType; }
    };
    var i;
    try {
      for (i = path.length - 1; i > 0; i--) invoke(path[i], 1, true);
      invoke(path[0], 2, true); invoke(path[0], 2, false);
      if (ev.bubbles) for (i = 1; i < path.length; i++) invoke(path[i], 3, false);
    } finally {
      // spec "dispatch" end: the flags are cleared, the path is gone.
      ev.currentTarget = null; ev.eventPhase = 0; ev._disp = false; ev._stop = false; ev._stopNow = false; ev._path = [];
    }
    return !ev.defaultPrevented;
  };
  Object.defineProperty(g, '__scratchy_dispatch', { value: dispatch });
  // QERR1 ("report the exception", R64 E7): an uncaught exception of a top-level script fires an
  // ErrorEvent at window. Returns true when a handler cancelled it (no log line then).
  var reporting = false;
  Object.defineProperty(g, '__scratchy_report_error', { value: function (err, filename, line, col, msg) {
    if (reporting) return false;
    reporting = true;
    try {
      var m;
      if (msg !== undefined) m = null;
      else try { m = (err !== null && typeof err === 'object' && err.name !== undefined) ? err.name + ': ' + err.message : String(err); } catch (x) { m = 'exception'; }
      var ev = new g.ErrorEvent('error', { cancelable: true, message: msg !== undefined ? msg : 'Uncaught ' + m, filename: String(filename), lineno: line, colno: col, error: err });
      dispatch(g, ev);
      return ev.defaultPrevented;
    } catch (x) { return false; } finally { reporting = false; }
  } });
  var ETP = EventTarget.prototype;
  Object.defineProperty(g, '__scratchy_ptr_mask', { value: function () {
    var m = 0;
    for (var i = 0; i < PTR_TYPES.length; i++) if (ptrCount[PTR_TYPES[i]] > 0) m |= 1 << i;
    return m;
  } });
  ETP.addEventListener = function (type, fn, opt) {
    if (fn == null || (typeof fn !== 'function' && typeof fn !== 'object')) return;
    var o = normOpts(opt), l = lists(this, String(type), true);
    for (var i = 0; i < l.length; i++) if (l[i].fn === fn && l[i].capture === o.capture) return;
    if (o.passive === undefined) {
      // QDOM4: touchstart/touchmove/wheel/mousewheel listeners on window, document, <html>, <body> default to passive.
      var d = g.document;
      o.passive = (type === 'touchstart' || type === 'touchmove' || type === 'wheel' || type === 'mousewheel') &&
        (this === g || (d && (this === d || this === d.documentElement || this === d.body)));
    }
    l.push({ fn: fn, capture: o.capture, once: o.once, passive: o.passive, removed: false });
    type = String(type);
    if (PTR_TYPES.indexOf(type) >= 0) ptrCount[type] = (ptrCount[type] | 0) + 1;
  };
  ETP.removeEventListener = function (type, fn, opt) {
    var l = lists(this, String(type), false);
    if (!l) return;
    var c = normOpts(opt).capture;
    for (var i = 0; i < l.length; i++) {
      if (l[i].fn === fn && l[i].capture === c) {
        l[i].removed = true; l.splice(i, 1);
        if (ptrCount[String(type)]) ptrCount[String(type)]--;
        return;
      }
    }
  };
  ETP.dispatchEvent = function (ev) {
    if (!(ev instanceof Event)) throw new TypeError("Failed to execute 'dispatchEvent': parameter 1 is not of type 'Event'.");
    if (ev._disp || ev._init === false) throw new DOMException(ev._disp ? 'The event is already being dispatched.' : 'The event provided is uninitialized.', 'InvalidStateError');
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
  // QDOM13: event handler IDL attributes (HTML "event handler IDL attributes") as accessors on the
  // prototypes Chrome desktop puts them on. Value rules are `getH`/`setH` above. No `ontouch*` (sites
  // read `'ontouchstart' in window` as "touch device"). Also left out: onscrollsnapchange/onscrollsnapchanging
  // (never fired). QEL7: `onscrollend` is here since scratchy fires `scrollend`.
  var GLOBAL_H = ('abort animationcancel animationend animationiteration animationstart auxclick beforeinput beforematch beforetoggle ' +
    'blur cancel canplay canplaythrough change click close contentvisibilityautostatechange contextlost contextmenu contextrestored ' +
    'cuechange dblclick drag dragend dragenter dragleave dragover dragstart drop durationchange emptied ended error focus formdata ' +
    'gotpointercapture input invalid keydown keypress keyup load loadeddata loadedmetadata loadstart lostpointercapture mousedown ' +
    'mouseenter mouseleave mousemove mouseout mouseover mouseup mousewheel pause play playing pointercancel pointerdown pointerenter ' +
    'pointerleave pointermove pointerout pointerover pointerrawupdate pointerup progress ratechange reset resize scroll scrollend ' +
    'securitypolicyviolation seeked seeking select selectionchange selectstart slotchange stalled ' +
    'submit suspend timeupdate toggle transitioncancel transitionend transitionrun transitionstart volumechange waiting wheel').split(' ');
  // [IDL name, event type] pairs whose type is not the lowercased name
  var PREFIXED_H = [['onwebkitanimationstart', 'webkitAnimationStart'], ['onwebkitanimationiteration', 'webkitAnimationIteration'],
    ['onwebkitanimationend', 'webkitAnimationEnd'], ['onwebkittransitionend', 'webkitTransitionEnd']];
  var DOC_ELEMENT_H = ['copy', 'cut', 'paste'];
  var DOC_H = ['fullscreenchange', 'fullscreenerror', 'pointerlockchange', 'pointerlockerror', 'readystatechange', 'visibilitychange',
    'freeze', 'resume', 'prerenderingchange', 'beforexrselect'];
  var WIN_ONLY_H = ['afterprint', 'beforeprint', 'beforeunload', 'hashchange', 'languagechange', 'message', 'messageerror', 'offline',
    'online', 'pagehide', 'pageshow', 'popstate', 'rejectionhandled', 'storage', 'unhandledrejection', 'unload'];
  var WIN_EXTRA_H = ['devicemotion', 'deviceorientation', 'deviceorientationabsolute', 'appinstalled', 'beforeinstallprompt',
    'pagereveal', 'pageswap', 'orientationchange'];
  var FWD_BODY_H = ['blur', 'error', 'focus', 'load', 'resize', 'scroll'];
  // kind: 'el' (an element: forwarding on body/frameset), 'doc', 'win'.
  var installH = function (P, name, type, kind) {
    var tgt = function (self) {
      if (kind === 'win') return self === undefined || self === null ? g : self;
      if (self === null || typeof self !== 'object' || (kind === 'el' && self.__id === undefined && !(self instanceof EventTarget))) throw new TypeError('Illegal invocation');
      return kind === 'el' && isFwdEl(self, type) && BODY_FWD[name] ? g : self;
    };
    var gf = { get [name]() { return getH(tgt(this), type); } };
    var get = Object.getOwnPropertyDescriptor(gf, name).get;
    var sf = { set [name](v) { setH(tgt(this), type, v); } };
    var set = Object.getOwnPropertyDescriptor(sf, name).set;
    Object.defineProperty(P, name, { get: get, set: set, enumerable: true, configurable: true });
  };
  var BODY_FWD = {};
  FWD_BODY_H.concat(WIN_ONLY_H).forEach(function (n) { BODY_FWD['on' + n] = true; });
  var installAll = function (P, kind, extra) {
    GLOBAL_H.forEach(function (n) { installH(P, 'on' + n, n, kind); });
    PREFIXED_H.forEach(function (r) { installH(P, r[0], r[1], kind); });
    if (kind !== 'win') DOC_ELEMENT_H.forEach(function (n) { installH(P, 'on' + n, n, kind); });
    (extra || []).forEach(function (n) { installH(P, 'on' + n, n, kind); });
  };
  installAll(HTMLElement.prototype, 'el');
  installAll(Document.prototype, 'doc', DOC_H);
  installAll(Window.prototype, 'win', WIN_ONLY_H.concat(WIN_EXTRA_H));
  // `HTMLBodyElement` / `HTMLFrameSetElement` / `SVGElement` are created by the DOM prelude, which calls this.
  Object.defineProperty(g, '__scratchy_install_handlers', { value: function (P, which) {
    if (which === 'svg') installAll(P, 'el');
    else WIN_ONLY_H.forEach(function (n) { installH(P, 'on' + n, n, 'el'); }), FWD_BODY_H.forEach(function (n) { installH(P, 'on' + n, n, 'el'); });
  } });
  // QDOM1: WindowProperties, between Window.prototype and EventTarget.prototype: named access by id /
  // embed|form|img|object name (`__scratchy_set_named` is set by the DOM prelude). Only names that nothing
  // else in the chain (EventTarget.prototype, Object.prototype) defines are visible; own globals and `var`s
  // win because they are found first. Not enumerable: a Proxy without `ownKeys` of its own.
  var wpNamed = null, wpTarget = Object.create(EventTarget.prototype);
  var WindowProperties = new Proxy(wpTarget, {
    get: function (t, k, r) {
      if (wpNamed !== null && typeof k === 'string' && !(k in t)) { var v = wpNamed(k); if (v !== undefined) return v; }
      return Reflect.get(t, k, r);
    },
    has: function (t, k) {
      return Reflect.has(t, k) || (wpNamed !== null && typeof k === 'string' && wpNamed(k) !== undefined);
    },
  });
  try { Object.setPrototypeOf(Window.prototype, WindowProperties); } catch (e) {}
  Object.defineProperty(g, '__scratchy_set_named', { value: function (f) { wpNamed = f; }, configurable: true });
  g.top = g.parent = g.frames = g; g.opener = null;
  // QPT1d (R53 D8): `name` (settable), `closed` (a getter; a dropped iframe realm sets it, `__scratchy_closed`).
  var closedFlag = false;
  Object.defineProperty(g, 'name', { value: '', writable: true, configurable: true, enumerable: true });
  Object.defineProperty(g, 'closed', { get: function () { return closedFlag; }, configurable: true, enumerable: true });
  Object.defineProperty(g, '__scratchy_set_closed', { value: function () { closedFlag = true; } });
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
  // QAPI2: the Navigator members pages probe. Privacy (DECISIONS "Network and privacy"): every value is a fixed
  // constant, never read from the machine, so nothing fingerprints the user: webdriver false, deviceMemory 8 (the
  // top Chrome bucket), connection a static 4g/10 Mbit/50 ms, empty plugins/mimeTypes (no PDF viewer), and
  // userAgentData built from navigator.userAgent only (brand + major version, mobile false, platform 'Linux').
  (function () {
    var nav = g.navigator, NP = Navigator.prototype;
    var acc = function (name, fn) { Object.defineProperty(NP, name, { get: fn, set: undefined, configurable: true, enumerable: true }); };
    var NetworkInformation = ctor('NetworkInformation', EventTarget), NavigatorUAData = ctor('NavigatorUAData');
    var PluginArray = ctor('PluginArray'), MimeTypeArray = ctor('MimeTypeArray');
    [NetworkInformation, NavigatorUAData, PluginArray, MimeTypeArray].forEach(function (C) {
      Object.defineProperty(C.prototype, Symbol.toStringTag, { value: C.name, configurable: true });
    });
    var arrayLike = function (C) {
      var a = Object.create(C.prototype);
      Object.defineProperty(a, 'length', { value: 0 });
      return a;
    };
    [PluginArray, MimeTypeArray].forEach(function (C) {
      var P = C.prototype;
      Object.defineProperty(P, Symbol.iterator, { value: Array.prototype[Symbol.iterator], writable: true, configurable: true });
      Object.defineProperty(P, 'item', { value: function item(i) { if (arguments.length < 1) throw new TypeError("Failed to execute 'item' on '" + C.name + "': 1 argument required, but only 0 present."); return null; }, writable: true, configurable: true, enumerable: true });
      Object.defineProperty(P, 'namedItem', { value: function namedItem(n) { if (arguments.length < 1) throw new TypeError("Failed to execute 'namedItem' on '" + C.name + "': 1 argument required, but only 0 present."); return null; }, writable: true, configurable: true, enumerable: true });
    });
    Object.defineProperty(PluginArray.prototype, 'refresh', { value: function refresh() {}, writable: true, configurable: true, enumerable: true });
    var plugins = arrayLike(PluginArray), mimes = arrayLike(MimeTypeArray);
    acc('plugins', function () { return plugins; });
    acc('mimeTypes', function () { return mimes; });
    acc('pdfViewerEnabled', function () { return false; });
    acc('webdriver', function () { return false; });
    acc('deviceMemory', function () { return 8; });
    var conn = Object.create(NetworkInformation.prototype);
    Object.defineProperty(NetworkInformation.prototype, 'onchange', { value: null, writable: true, configurable: true, enumerable: true });
    [['effectiveType', '4g'], ['type', 'unknown'], ['downlink', 10], ['rtt', 50], ['saveData', false]].forEach(function (kv) {
      Object.defineProperty(NetworkInformation.prototype, kv[0], { get: function () { return kv[1]; }, configurable: true, enumerable: true });
    });
    acc('connection', function () { return conn; });
    // userAgentData: the brand is the last `Name/major` product token of the UA that is not a compatibility
    // token (scratchy/0.1 by default, Chrome/120 under a Chrome --user-agent), plus a GREASE brand.
    var uad = null;
    var uaBrands = function (ua) {
      var skip = /^(Mozilla|AppleWebKit|Safari|Gecko|KHTML|Version|Mobile)$/, re = /([A-Za-z][A-Za-z0-9_.-]*)\/(\d+)/g, m, last = null;
      while ((m = re.exec(ua))) if (!skip.test(m[1])) last = m;
      var out = [{ brand: 'Not;A=Brand', version: '99' }];
      if (last) out.push({ brand: last[1], version: last[2] });
      return out;
    };
    var mkBrands = function (ua) { return uaBrands(ua).map(function (b) { return { brand: b.brand, version: b.version }; }); };
    var UD = NavigatorUAData.prototype;
    Object.defineProperty(UD, 'brands', { get: function () { return mkBrands(nav.userAgent); }, configurable: true, enumerable: true });
    Object.defineProperty(UD, 'mobile', { get: function () { return false; }, configurable: true, enumerable: true });
    Object.defineProperty(UD, 'platform', { get: function () { return 'Linux'; }, configurable: true, enumerable: true });
    Object.defineProperty(UD, 'getHighEntropyValues', { value: function getHighEntropyValues(hints) {
      if (arguments.length < 1) return Promise.reject(new TypeError("Failed to execute 'getHighEntropyValues' on 'NavigatorUAData': 1 argument required, but only 0 present."));
      // Only the low-entropy fields, whatever is asked: a hint list never unlocks more (no architecture, model, ...).
      return Promise.resolve({ brands: this.brands, mobile: false, platform: 'Linux' });
    }, writable: true, configurable: true, enumerable: true });
    Object.defineProperty(UD, 'toJSON', { value: function toJSON() { return { brands: this.brands, mobile: false, platform: 'Linux' }; }, writable: true, configurable: true, enumerable: true });
    acc('userAgentData', function () {
      if (!uad) uad = Object.create(UD);
      return uad;
    });
  })();
  // QAPI3: navigator.serviceWorker and window.caches, inert stubs (DECISIONS "Network and privacy"): no worker is
  // ever registered, nothing is stored or persisted, no network. Pages probe these and fall back when they fail.
  (function () {
    var ServiceWorkerContainer = ctor('ServiceWorkerContainer', EventTarget), CacheStorage = ctor('CacheStorage'), Cache = ctor('Cache');
    [ServiceWorkerContainer, CacheStorage, Cache].forEach(function (C) {
      Object.defineProperty(C.prototype, Symbol.toStringTag, { value: C.name, configurable: true });
    });
    var method = function (P, iface, name, nargs, fn) {
      var f = { [name]: function () {
        if (arguments.length < nargs) return Promise.reject(new TypeError("Failed to execute '" + name + "' on '" + iface + "': " + nargs + ' argument' + (nargs > 1 ? 's' : '') + ' required, but only ' + arguments.length + ' present.'));
        return fn.apply(this, arguments);
      } }[name];
      Object.defineProperty(P, name, { value: f, writable: true, configurable: true, enumerable: true });
    };
    var SWC = ServiceWorkerContainer.prototype, never = null;
    ['oncontrollerchange', 'onmessage', 'onmessageerror'].forEach(function (n) {
      Object.defineProperty(SWC, n, { value: null, writable: true, configurable: true, enumerable: true });
    });
    Object.defineProperty(SWC, 'controller', { get: function () { return null; }, configurable: true, enumerable: true });
    Object.defineProperty(SWC, 'ready', { get: function () { return never || (never = new Promise(function () {})); }, configurable: true, enumerable: true });
    method(SWC, 'ServiceWorkerContainer', 'register', 1, function () {
      return Promise.reject(new g.DOMException('Failed to register a ServiceWorker: service workers are not supported in this browser.', 'SecurityError'));
    });
    method(SWC, 'ServiceWorkerContainer', 'getRegistrations', 0, function () { return Promise.resolve([]); });
    method(SWC, 'ServiceWorkerContainer', 'getRegistration', 0, function () { return Promise.resolve(undefined); });
    Object.defineProperty(SWC, 'startMessages', { value: function startMessages() {}, writable: true, configurable: true, enumerable: true });
    var swc = Object.create(SWC);
    Object.defineProperty(Navigator.prototype, 'serviceWorker', { get: function () { return swc; }, set: undefined, configurable: true, enumerable: true });
    var CP = Cache.prototype;
    method(CP, 'Cache', 'match', 1, function () { return Promise.resolve(undefined); });
    method(CP, 'Cache', 'matchAll', 0, function () { return Promise.resolve([]); });
    method(CP, 'Cache', 'add', 1, function () { return Promise.resolve(); });
    method(CP, 'Cache', 'addAll', 1, function () { return Promise.resolve(); });
    method(CP, 'Cache', 'put', 2, function () { return Promise.resolve(); });
    method(CP, 'Cache', 'delete', 1, function () { return Promise.resolve(false); });
    method(CP, 'Cache', 'keys', 0, function () { return Promise.resolve([]); });
    var CSP = CacheStorage.prototype;
    method(CSP, 'CacheStorage', 'open', 1, function () { return Promise.resolve(Object.create(CP)); });
    method(CSP, 'CacheStorage', 'match', 1, function () { return Promise.resolve(undefined); });
    method(CSP, 'CacheStorage', 'has', 1, function () { return Promise.resolve(false); });
    method(CSP, 'CacheStorage', 'delete', 1, function () { return Promise.resolve(false); });
    method(CSP, 'CacheStorage', 'keys', 0, function () { return Promise.resolve([]); });
    var cs = Object.create(CSP);
    Object.defineProperty(g, 'caches', { get: function () { return cs; }, set: function () {}, configurable: true, enumerable: true });
  })();
  g.innerWidth = Number(env.innerWidth); g.innerHeight = Number(env.innerHeight);
  // QY10: MediaQueryList. `matches` is re-evaluated on every read.
  // QEL7: `change` fires from the rendering update (step 3, `__scratchy_mql_run`) for a list that has a
  // listener when the viewport size changed and its result flipped. Lists are registered (and kept alive)
  // when a `change` listener or `onchange` handler is added; an iframe realm has no rendering update here.
  var MediaQueryList = ctor('MediaQueryList', EventTarget);
  Object.defineProperty(MediaQueryList.prototype, 'matches', { get: function () { return !!__scratchy_media(this.__q)[0]; }, configurable: true, enumerable: true });
  Object.defineProperty(MediaQueryList.prototype, 'media', { get: function () { return __scratchy_media(this.__q)[1]; }, configurable: true, enumerable: true });
  var mqls = [], mqlSig = null;
  var mqlWatch = function (m) {
    if (typeof g.__scratchy_qel7 !== 'function' || mqls.indexOf(m) >= 0) return;
    Object.defineProperty(m, '__last', { value: !!__scratchy_media(m.__q)[0], writable: true });
    mqls.push(m);
    g.__scratchy_qel7(4, true);
  };
  Object.defineProperty(MediaQueryList.prototype, 'onchange', { get: function () { var v = this.__onchange; return v === undefined ? null : v; },
    set: function (v) {
      if (typeof v === 'function') mqlWatch(this);
      Object.defineProperty(this, '__onchange', { value: typeof v === 'function' ? v : null, writable: true, configurable: true });
    }, configurable: true, enumerable: true });
  MediaQueryList.prototype.addEventListener = function (type, fn, opt) {
    if (String(type) === 'change' && fn != null) mqlWatch(this);
    return EventTarget.prototype.addEventListener.apply(this, arguments);
  };
  MediaQueryList.prototype.addListener = function (fn) { this.addEventListener('change', fn); };
  MediaQueryList.prototype.removeListener = function (fn) { this.removeEventListener('change', fn); };
  Object.defineProperty(g, '__scratchy_mql_run', { value: function () {
    mqlRun();
    if (g.__scratchy_q7_each) g.__scratchy_q7_each('__scratchy_mql_run', 4);  // QEL7b: the iframe realms' lists
  } });
  var mqlRun = function () {
    var sig = g.innerWidth + 'x' + g.innerHeight;
    if (sig === mqlSig) return;
    mqlSig = sig;
    var list = mqls.slice();
    for (var i = 0; i < list.length; i++) {
      var m = list[i], r = __scratchy_media(m.__q), now = !!r[0];
      if (now === m.__last) continue;
      m.__last = now;
      var ev = new g.MediaQueryListEvent('change', { matches: now, media: r[1] });
      __scratchy_trust(ev);
      __scratchy_ua(ev);
      try { __scratchy_dispatch(m, ev); } catch (e) {}
    }
  };
  g.matchMedia = function (q) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'matchMedia' on 'Window': 1 argument required, but only 0 present.");
    var m = Object.create(MediaQueryList.prototype);
    Object.defineProperty(m, '__q', { value: String(q) });
    return m;
  };
  // QCSS2: the `CSS` namespace (no constructor): `supports` runs the `@supports` evaluator, `escape` is CSS.escape.
  (function () {
    var CSS = {};
    Object.defineProperty(CSS, Symbol.toStringTag, { value: 'CSS', configurable: true });
    function def(name, len, fn) {
      Object.defineProperty(fn, 'name', { value: name, configurable: true });
      Object.defineProperty(fn, 'length', { value: len, configurable: true });
      Object.defineProperty(CSS, name, { value: fn, writable: true, configurable: true, enumerable: true });
    }
    def('supports', 1, function (a, b) {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'supports' on 'CSS': 1 argument required, but only 0 present.");
      if (arguments.length === 1) return !!__scratchy_supports1(String(a));
      return !!__scratchy_supports2(String(a), String(b));
    });
    def('escape', 1, function (v) {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'escape' on 'CSS': 1 argument required, but only 0 present.");
      var s = String(v), out = '', n = s.length;
      for (var i = 0; i < n; i++) {
        var c = s.charCodeAt(i), ch = s.charAt(i);
        if (c >= 0xD800 && c <= 0xDBFF && i + 1 < n) {
          var d = s.charCodeAt(i + 1);
          if (d >= 0xDC00 && d <= 0xDFFF) { out += ch + s.charAt(i + 1); i++; continue; }
        }
        if (c === 0) out += '�';
        else if ((c >= 1 && c <= 0x1F) || c === 0x7F || (i === 0 && c >= 0x30 && c <= 0x39) ||
                 (i === 1 && c >= 0x30 && c <= 0x39 && s.charCodeAt(0) === 0x2D)) out += '\\' + c.toString(16) + ' ';
        else if (i === 0 && n === 1 && c === 0x2D) out += '\\' + ch;
        else if (c >= 0x80 || c === 0x2D || c === 0x5F || (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A)) out += ch;
        else out += '\\' + ch;
      }
      return out;
    });
    Object.defineProperty(g, 'CSS', { value: CSS, writable: true, configurable: true, enumerable: false });
  })();
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
    timing: timing,
    navigation: { type: 0, redirectCount: 0 },
    now: function () { return __scratchy_now(); },
    getEntries: function () { return []; },
    getEntriesByType: function () { return []; },
    getEntriesByName: function () { return []; },
    mark: function () {}, measure: function () {},
    clearMarks: function () {}, clearMeasures: function () {},
    toJSON: function () { return { timeOrigin: perf.timeOrigin, timing: timing }; },
  };
  // QWPTFLK6: `timeOrigin` is anchored on first read as `Date.now() - performance.now()`, so
  // `timeOrigin + now()` tracks `Date.now()` even when `now()` is a manual clock (the WPT runner's
  // virtual clock stands still while the real one runs on: page setup under machine load used to
  // push `Date.now()` past `timeOrigin + now() + 30`). Reset by `__scratchy_apply_env`.
  var originAnchor = null;
  Object.defineProperty(perf, 'timeOrigin', { get: function () {
    if (originAnchor === null) originAnchor = Date.now() - __scratchy_now();
    return originAnchor;
  }, configurable: true, enumerable: true });
  Object.defineProperty(g, 'performance', { value: perf, writable: true, configurable: true, enumerable: true });
  // QV8-4: everything above that came from the page (`env`) or the clock, as one function. The V8
  // startup snapshot bakes this prelude over an about:blank env; a page engine then calls
  // `__scratchy_apply_env(its env)` (the same values the code above computed from `env`).
  var navObj = g.navigator, envHooks = [];
  Object.defineProperty(g, 'clientInformation', { get: function () { return g.navigator; }, set: function (v) { Object.defineProperty(g, 'clientInformation', { value: v, writable: true, configurable: true, enumerable: true }); }, configurable: true, enumerable: true });
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
      var d = Object.getOwnPropertyDescriptor(g.document, p[0]);
      if (d && 'value' in d) d.value = p[1], Object.defineProperty(g.document, p[0], d); // data properties only (baseURI became an accessor)
    });
    t0 = Date.now();
    Object.keys(timing).forEach(function (k) {
      timing[k] = (k === 'unloadEventStart' || k === 'unloadEventEnd' || k === 'redirectStart' || k === 'redirectEnd') ? 0 : t0;
    });
    originAnchor = null;
    envHooks.forEach(function (h) { h(); });
  }, configurable: true });
  // QPT1a (R53 D2/D5): binds a fresh iframe realm (made by `__scratchy_realm_new` from the startup
  // snapshot, so this hook is baked and must read page state only through `o`) to its frame. The
  // parent calls `w.__scratchy_realm_init({ docId, href, base, parent, top, frameElement, name, width,
  // height })`: the env is about:blank (or the pre-fetched URL) with the PARENT's origin, the
  // document is the iframe's second document, and the realm's timers, network requests, WebSocket
  // and unhandled-rejection reports go through the parent's queues (Rust pumps only the page).
  Object.defineProperty(g, '__scratchy_realm_init', { value: function (o) {
    delete g.__scratchy_realm_init;
    var p = o.parent, u;
    try { u = new g.URL(o.href); } catch (e) { u = new g.URL('about:blank'); }
    g.__scratchy_apply_env({ userAgent: p.navigator.userAgent, appVersion: p.navigator.appVersion,
      innerWidth: o.width, innerHeight: o.height, title: '', href: u.href, protocol: u.protocol, host: u.host,
      hostname: u.hostname, port: u.port, pathname: u.pathname, search: u.search, hash: u.hash, origin: o.opaque ? 'null' : p.location.origin });
    Object.defineProperty(g, 'isSecureContext', { value: !!p.isSecureContext, writable: true, configurable: true, enumerable: true });
    g.__d_bind_doc(o.docId, o.base, o.dom);
    g.__scratchy_q7_attach(p);  // QEL7b: scroll / MediaQueryList work is run by the parent's rendering update
    // QSTASH2: a same-origin frame shares its parent's localStorage/sessionStorage; an opaque one keeps its own.
    if (!o.opaque) g.__scratchy_ls_share(p);
    g.document.readyState = 'complete';
    // QPT1-D1: an opaque-origin (data:) frame sees cross-origin views of its parent and top, has no
    // frameElement, and tells the cross-origin proxy what its real parent/top are.
    if (o.opaque) {
      Object.defineProperty(g, '__scratchy_opaque', { value: true });
      Object.defineProperty(g, '__scratchy_xo', { value: o.xo, configurable: true });
      Object.defineProperty(g, '__scratchy_xparent', { value: p });
      Object.defineProperty(g, '__scratchy_xtop', { value: o.top });
    }
    [['parent', o.opaque ? o.xo(p) : p], ['top', o.opaque ? o.xo(o.top) : o.top], ['frameElement', o.opaque ? null : o.frameElement], ['opener', null], ['frames', g]].forEach(function (kv) {
      Object.defineProperty(g, kv[0], { value: kv[1], writable: true, configurable: true, enumerable: true });
    });
    // QPT1d: the browsing context name; a script's `name = x` tells the parent (its `window[name]` props).
    var nm = String(o.name || '');
    Object.defineProperty(g, 'name', { get: function () { return nm; }, set: function (v) { nm = String(v); if (o.onName) o.onName(); }, configurable: true, enumerable: true });
    g.__scratchy_timers_fwd(p);
    g.__net_fwd(p);
    // QPT1b2: the realm's `WebSocket` is its own class over the page's, recording the sockets this
    // realm creates (`__scratchy_realm_release` closes them); the page's own sockets are untouched.
    var wsRoot = p.__ws_root || p.WebSocket, wsDetach = p.__ws_detach, wsMine = [];
    function WebSocket(url, protocols) {
      if (!new.target) return wsRoot.apply(this, arguments);  // throws the constructor's own TypeError
      var ws = Reflect.construct(wsRoot, arguments, new.target);
      wsMine.push(ws);
      return ws;
    }
    WebSocket.prototype = Object.create(wsRoot.prototype, { constructor: { value: WebSocket, writable: true, configurable: true } });
    Object.setPrototypeOf(WebSocket, wsRoot);
    ['CONNECTING', 'OPEN', 'CLOSING', 'CLOSED'].forEach(function (k) { Object.defineProperty(WebSocket, k, { value: wsRoot[k], enumerable: true }); });
    Object.defineProperty(g, '__ws_root', { value: wsRoot, configurable: true });
    Object.defineProperty(g, '__ws_detach', { value: wsDetach, configurable: true });
    Object.defineProperty(g, 'WebSocket', { value: WebSocket, writable: true, configurable: true });
    p.__scratchy_rej_sub(g.__scratchy_rej_flush);
    // QDOM5c: the initial about:blank window navigates in place to a same-origin URL (the Window object stays): the
    // location parts and the document URL change, the about:blank base fallback goes.
    Object.defineProperty(g, '__scratchy_realm_nav', { value: function (href, keepBase) {
      var n = new g.URL(href);
      var e = { href: n.href, protocol: n.protocol, host: n.host, hostname: n.hostname, port: n.port, pathname: n.pathname, search: n.search, hash: n.hash };
      Object.keys(e).forEach(function (k) { locSt[k] = e[k]; });
      ['URL', 'documentURI'].forEach(function (k) {
        var d = Object.getOwnPropertyDescriptor(g.document, k);
        if (d && 'value' in d) { d.value = n.href; Object.defineProperty(g.document, k, d); }
      });
      if (!keepBase) g.__scratchy_frame_base_clear();
    }, configurable: true });
    // QPT1b (R53 D6): the parent calls this when it drops the realm.
    Object.defineProperty(g, '__scratchy_realm_release', { value: function () {
      g.__scratchy_q7_detach();
      g.__d_frames_drop_all();
      g.__d_realm_purge();
      g.__scratchy_set_closed();
      g.__scratchy_timers_cancel(g);
      g.__net_cancel(g);
      wsMine.splice(0).forEach(function (w) { try { wsDetach(w); } catch (e) {} });
      try { g.__scratchy_rej_flush(); } catch (e) {}  // report what is pending before the realm goes
      p.__scratchy_rej_unsub(g.__scratchy_rej_flush);
    }, configurable: true });
    // Navigation, history and scrolling of an iframe are not implemented (they would act on the
    // PAGE through the shared natives): they log and do nothing.
    ['__d_navigate', '__d_nav_replace', '__d_reload', '__d_url_update', '__d_url_move', '__d_scroll'].forEach(function (k) {
      g[k] = function () { if (k !== '__d_scroll') __scratchy_err('scratchy: js: not implemented: navigation of an iframe window (' + k + ')'); };
    });
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
pub(crate) const PRELUDE_SCRIPTS: &[&str] = &["prelude", "timers", "url", "globals", "intl", "fetch", "websocket", "cssom", "fonts", "media", "mse", "env", "frames-boot", "img-sweep", "prereq", "media-wait", "eval"];

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
    crate::incr_log::layout_boundary();
    d
}

/// QREC2: `--deterministic[=EPOCH_MS]` (report modes only): the epoch `Date` starts at; -1 = off.
static DET_EPOCH: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
pub(crate) const DET_SEED: u64 = 0x5C2A_7C41_D00D_F00D;
/// Seeds `crypto.getRandomValues` (splitmix64 state).
static DET_RNG: std::sync::Mutex<(u64,)> = std::sync::Mutex::new((DET_SEED,));
/// Epoch used by a bare `--deterministic`: 2023-11-14T22:13:20Z.
const DET_DEFAULT_EPOCH_MS: i64 = 1_700_000_000_000;

pub(crate) fn det_epoch() -> Option<i64> {
    let e = DET_EPOCH.load(std::sync::atomic::Ordering::Relaxed);
    (e >= 0).then_some(e)
}

/// V8's `--random-seed` value (a positive i32) for deterministic runs.
pub(crate) fn det_v8_seed() -> Option<u32> {
    det_epoch().map(|_| ((DET_SEED >> 33) as u32) | 1)
}

/// QREC2: removes `--deterministic[=EPOCH_MS]` from `args` and arms the mode. Only the report modes
/// (the first argument) accept it; anything else is an error (window mode never runs deterministic).
pub(crate) fn take_deterministic_flag(args: &mut Vec<String>) -> Result<(), String> {
    let mut epoch: Option<Result<i64, String>> = None;
    args.retain(|a| {
        if a == "--deterministic" {
            epoch = Some(Ok(DET_DEFAULT_EPOCH_MS));
            false
        } else if let Some(v) = a.strip_prefix("--deterministic=") {
            epoch = Some(v.parse::<i64>().ok().filter(|e| *e >= 0).ok_or_else(|| format!("--deterministic={v}: expected a non-negative epoch in milliseconds")));
            false
        } else {
            true
        }
    });
    let Some(epoch) = epoch else { return Ok(()) };
    let epoch = epoch?;
    let report = matches!(args.first().map(String::as_str), Some("--perf-report" | "--js-report" | "--missing-api-report" | "--wpt-th-one"));
    if !report {
        return Err("--deterministic is only for the report modes (--perf-report, --js-report, --missing-api-report, --wpt-th-one)".into());
    }
    DET_EPOCH.store(epoch, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// QREC2: after the preludes, in `--deterministic` runs: `Date` starts at the epoch and advances by
/// 1 ms per read (`Date.now()`, `new Date()`), so it never depends on the wall clock or timing.
const DET_DATE_JS: &str = r#"(function (g, E) {
  var N = g.Date, calls = 0;
  function now() { return E + calls++; }
  function D() {
    if (!new.target) return new N(now()).toString();
    if (arguments.length === 0) return new N(now());
    return Reflect.construct(N, arguments, new.target === D ? N : new.target);
  }
  D.prototype = N.prototype;
  Object.defineProperty(N.prototype, 'constructor', { value: D, writable: true, configurable: true });
  D.now = function now_() { return now(); };
  D.UTC = N.UTC; D.parse = N.parse;
  Object.defineProperty(D, 'name', { value: 'Date', configurable: true });
  Object.defineProperty(D, 'length', { value: 7, configurable: true });
  Object.defineProperty(g, 'Date', { value: D, writable: true, configurable: true });
})(globalThis, %EPOCH%);"#;

/// QSD1b: `n` bytes from the kernel CSPRNG (`getrandom(2)`, via the `libc` crate that
/// is already a dependency; no new crate). Falls back to `/dev/urandom`; if neither
/// works the bytes stay zero and an error is logged to stderr rather than panicking.
fn os_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if det_epoch().is_some() {
        // QREC2: `--deterministic`: a seeded splitmix64 stream, the same bytes in every run.
        let mut st = DET_RNG.lock().unwrap_or_else(|e| e.into_inner());
        for b in buf.iter_mut() {
            st.0 = st.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = st.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *b = ((z ^ (z >> 31)) >> 24) as u8;
        }
        return buf;
    }
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
    env.set("innerWidth", env_viewport().0)?;
    env.set("innerHeight", env_viewport().1)?;
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
    /// QSTASH2: the page's persistent localStorage (ops handed to stash's writer at the end of each task).
    pub(crate) storage: crate::js_storage::LocalStorage,
    /// QANIMHL: the CSS animations of the headless rendering steps (event_loop.rs; the App keeps its own
    /// `AnimTracker`).
    pub(crate) anim: std::cell::RefCell<crate::event_loop::HeadlessAnim>,
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
        self.storage_end_task();
    }

    /// QSTASH2: the end of a task: its localStorage ops go to the stash writer as one commit. A site
    /// forgotten meanwhile empties the page's copy.
    pub(crate) fn storage_end_task(&self) {
        if self.storage.end_task() {
            self.eng.with(|ctx| {
                let _ = js_engine::eval_sloppy::<()>(&ctx, "storage", "__scratchy_ls_forgotten()");
            });
        }
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
            // QERR1: an uncaught exception is "reported": an ErrorEvent at window first; a
            // handler that cancels it (onerror returning true, preventDefault) hides the log line.
            let report = |ctx: &js_engine::Ctx<'_>| -> (String, bool) {
                let (line, col) = js_engine::expose_pending(ctx, "__scratchy_exc");
                let msg = js_engine::describe_exception(ctx);
                if self.tripped() {
                    return (msg, false);
                }
                let call = format!("(function(e){{delete globalThis.__scratchy_exc;return __scratchy_report_error(e,{},{line},{col})}})(globalThis.__scratchy_exc)", js_string_literal(name));
                let handled = js_engine::eval_sloppy::<bool>(ctx, "error-event", &call).unwrap_or_else(|e| {
                    let _ = js_engine::describe_error(ctx, e);
                    false
                });
                (msg, handled)
            };
            let Ok(compiled) = compiled else {
                let (msg, handled) = report(&ctx);
                let now = thread_cpu_nanos();
                if !handled {
                    failed(msg, now.saturating_sub(t0), 0);
                }
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
                let (msg, handled) = report(&ctx);
                if !handled {
                    failed(msg, t1.saturating_sub(t0), run);
                }
            }
        });
        self.eng.drain_jobs(&*self.log);
        self.storage_end_task();
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
            "(function(p){{var r=globalThis.__scratchy_modr={{s:0}};p.then(function(){{r.s=1;}},function(e){{r.s=2;r.e=e;r.m=(e&&e.name?e.name+': '+e.message:'Uncaught '+String(e))+(e&&e.stack?'\\n'+e.stack:'');}});}})(import({}))",
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
                // QERR1: an ErrorEvent at window first (no throw position for a module rejection).
                let call = format!("(function(e){{return __scratchy_report_error(e,{},0,0)}})(globalThis.__scratchy_modr.e)", js_string_literal(name));
                let handled = self.eval_value_as::<bool>(None, "error-event", &call).unwrap_or(false);
                if !handled {
                    (self.log)("error", &format!("{name}: {msg}"));
                }
            }
        }
    }

    pub(crate) fn eval_value<T: FromJsRet>(&self, label: &str, code: &str) -> Option<T> {
        self.eval_value_as(None, label, code)
    }

    /// QREG2: `String()` of `code`'s value, evaluated (indirect eval) in the main realm; `Err` is the
    /// thrown error's message. Nothing is logged: the caller shows the message.
    pub(crate) fn eval_to_string(&self, label: &str, code: &str) -> Result<String, String> {
        let mut lit = String::from("'");
        for c in code.chars() {
            match c {
                '\\' => lit.push_str("\\\\"),
                '\'' => lit.push_str("\\'"),
                '\n' => lit.push_str("\\n"),
                '\r' => lit.push_str("\\r"),
                '\u{2028}' => lit.push_str("\\u2028"),
                '\u{2029}' => lit.push_str("\\u2029"),
                c => lit.push(c),
            }
        }
        lit.push('\'');
        let wrapped = format!("String((0, eval)({lit}))");
        let budget = perf::callback_budget();
        self.arm_budget(budget);
        let r = self.eng.with(|ctx| {
            js_engine::eval_sloppy::<String>(&ctx, label, &wrapped).map_err(|e| js_engine::describe_error(&ctx, e))
        });
        self.drain_after_task();
        r
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
    pub(crate) fn set_scroll(&self, x: f32, y: f32, els: Option<Vec<(usize, f32)>>) {
        let y = (y as f64).round();
        let x = (x as f64).round();
        // QJ6: element offsets the wheel changed, as `{nodeId: y}`; QXSCR1: the x offsets (keys
        // with `layout::X_KEY` set) as a second map.
        let m = els.map_or(String::new(), |v| {
            let (xs, ys): (Vec<_>, Vec<_>) = v.iter().partition(|(k, _)| k & crate::layout::X_KEY != 0);
            let obj = |v: Vec<&(usize, f32)>| {
                let body: Vec<String> =
                    v.iter().map(|(k, y)| format!("\"{}\":{}", k & !crate::layout::X_KEY, y.round())).collect();
                format!("{{{}}}", body.join(","))
            };
            format!("globalThis.__scratchy_elscroll({},{});", obj(ys), obj(xs))
        });
        let code = format!(
            "{m}(function(){{var n=0;if(globalThis.scrollY!=={y}){{globalThis.scrollY=globalThis.pageYOffset={y};n=1;}}\
             if(globalThis.scrollX!=={x}){{globalThis.scrollX=globalThis.pageXOffset={x};n=1;}}\
             if(n&&globalThis.__scratchy_scroll_doc)globalThis.__scratchy_scroll_doc();return true}})()"
        );
        let _ = self.eval_value::<bool>("scroll-sync", &code);
    }

    /// Q33b2: the window is `w` x `h` now. Updates `innerWidth/innerHeight` (and everything derived:
    /// `visualViewport`, `screen`, `documentElement.clientWidth`, the forced layout's viewport) and
    /// fires one `resize` at the window when the size changed. The caller runs once per frame, so
    /// bursts of resizes coalesce.
    pub(crate) fn set_viewport(&self, w: u32, h: u32) {
        let _ = self.eval_value::<bool>("viewport-sync", &format!("globalThis.__scratchy_setvp({w},{h})"));
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

    /// QEDIT-C: the ancestor revealing algorithm for `h` (find-in-page match, fragment target): fires
    /// `beforematch`, removes `hidden=until-found`, opens `<details>`. Returns how many things changed.
    pub(crate) fn reveal_node(&self, h: &Handle) -> usize {
        let id = self.table.borrow_mut().intern(h);
        self.eval_value::<f64>("reveal", &format!("__scratchy_reveal_id({id})")).map_or(0, |v| v as usize)
    }

    /// QDET1: the activation behaviour of a `<summary>`.
    pub(crate) fn summary_activate(&self, h: &Handle) {
        let id = self.table.borrow_mut().intern(h);
        let _ = self.eval_value::<f64>("summary", &format!("__scratchy_summary_activate_id({id})"));
    }

    pub(crate) fn take_requests(&self) -> js_dom::JsRequests {
        std::mem::take(&mut *self.reqs.borrow_mut())
    }

    /// Q33d: fires a trusted event of `kind` (`click`, `keydown`, ...) at
    /// `target` (`None` = `<body>`, else the document) with the given init fields, through
    /// capture/target/bubble. Returns false when a page handler called
    /// `preventDefault`, so the caller must skip its default action.
    /// QCUR1: bit i set when a page listener exists for `PTR_TYPES[i]` (mousemove,
    /// pointermove, mouseenter, mouseleave, pointerover, pointerout, pointerenter,
    /// pointerleave). Cheap: reads a JS-side counter kept by add/removeEventListener.
    pub(crate) fn pointer_listener_mask(&self) -> u32 {
        self.eval_value::<f64>("ptrmask", "__scratchy_ptr_mask()").map_or(0, |v| v as u32)
    }

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

    #[cfg(test)]
    pub(crate) fn run_due(&self) {
        self.run_due_slice(None);
    }

    /// QM1k1n6b: like `run_due` but stops once `slice` of wall time is used
    /// (checked after each callback, so one long callback just ends the
    /// slice). Timers run strictly in `__scratchy_timers_run_one` order, so
    /// what a slice leaves is picked up by the next turn in the same order.
    /// Returns true when the slice ended with work possibly left over (the
    /// caller must wake again at once: `next_deadline` is then already due).
    pub(crate) fn run_due_slice(&self, slice: Option<Duration>) -> bool {
        let start = Instant::now();
        for _ in 0..MAX_TIMERS_PER_TURN {
            if self.eval_value_as::<bool>(Some((perf::Kind::Timer, |r| *r == Some(false))), "timer", "__scratchy_timers_run_one()") != Some(true) {
                return false;
            }
            if slice.is_some_and(|s| start.elapsed() >= s) {
                return true;
            }
        }
        true
    }

    /// QEL1: the current time on the page's `__scratchy_now` clock (ms; the manual clock when one is set).
    pub(crate) fn now_ms(&self) -> f64 {
        self.eval_value::<f64>("now", "__scratchy_now()").unwrap_or(0.0)
    }

    /// QEL1: the frame time (ms, `__scratchy_now` clock) the pending animation frame callbacks wait for;
    /// `None` when none is pending.
    pub(crate) fn raf_due_ms(&self) -> Option<f64> {
        self.eval_value::<f64>("raf", "__scratchy_raf_due()").filter(|m| *m >= 0.0)
    }

    /// QEL1: `raf_due_ms` as an Instant for the App's wake-up.
    pub(crate) fn raf_deadline(&self) -> Option<Instant> {
        self.raf_due_ms().map(|ms| self.base + Duration::from_secs_f64(ms / 1000.0))
    }

    /// QEL1 (R64 E1/E2): "run the animation frame callbacks" for one rendering update: every callback
    /// pending now, with the same timestamp `ts`, never split by a timer slice; callbacks requested
    /// meanwhile wait for the next frame. Each callback is its own eval (own CPU budget, microtask
    /// checkpoint after it). Returns how many ran.
    pub(crate) fn run_animation_frames(&self, ts: f64) -> usize {
        let pending = self.eval_value::<f64>("raf", &format!("__scratchy_raf_begin({ts})")).unwrap_or(0.0);
        if pending < 1.0 {
            return 0;
        }
        let mut n = 0;
        while self.eval_value_as::<bool>(Some((perf::Kind::Timer, |r| *r == Some(false))), "raf", "__scratchy_raf_run_one()") == Some(true) {
            n += 1;
        }
        n
    }

    /// QANIMHL: the headless style + layout step: brings the page's forced layout up to date (a no-op
    /// when nothing changed since the last geometry read).
    pub(crate) fn animation_style_pass(&self) {
        self.eval("anim-style", "__d_anim_pass(Number(innerWidth) || 0, Number(innerHeight) || 0)");
    }

    /// QANIMHL: does the page's CSS (its `<style>` elements plus the external and adopted sheets) define
    /// any `@keyframes`? Re-reads the rules only where they changed (`rules_cache`).
    pub(crate) fn page_has_keyframes(&self) -> bool {
        let Some(page) = self.table.borrow().get(0) else { return false };
        drop(crate::rules_cache::cached_rules_for(&page, crate::rules_cache::Extra::Ver(crate::js_style::css_ver())));
        crate::animation::any_keyframes()
    }

    /// QEL1-QEL3: the observer hook of the rendering steps (step 7): the ResizeObserver gather/broadcast
    /// loop, then the IntersectionObserver evaluation (its callbacks are queued as a task).
    /// QEL7: what the rendering update owes (`__d_qel7`): bit 0 = `scroll` events queued, bit 1 = `scrollend`
    /// waiting for a quiet frame, bit 2 = a MediaQueryList with a `change` listener exists.
    pub(crate) fn qel7_bits(&self) -> u8 {
        js_dom::QEL7_BITS.with(|b| b.get())
    }

    /// QEL7: rendering step 2, "run the scroll steps": the queued `scroll` events, then `scrollend` for the
    /// targets that were quiet this update (not while `busy`: the window is still easing or a thumb is held).
    pub(crate) fn run_scroll_steps(&self, busy: bool) {
        if self.qel7_bits() & 3 != 0 {
            self.eval("scroll-steps", &format!("globalThis.__scratchy_scroll_steps({busy})"));
        }
    }

    /// QEL7: rendering step 3, "evaluate media queries and report changes".
    pub(crate) fn run_media_queries(&self) {
        if self.qel7_bits() & 4 != 0 {
            self.eval("mql-run", "globalThis.__scratchy_mql_run()");
        }
    }

    /// QEL2b: headless rendering step 6: the frame's layout, when a ResizeObserver observation may
    /// have changed (step 7 then reads it; no forced layout of its own). Free when nothing is observed.
    pub(crate) fn observer_layout(&self) {
        if crate::ro_geom::active() {
            self.eval("ro-layout", "globalThis.__scratchy_ro_layout && globalThis.__scratchy_ro_layout()");
        }
    }

    /// QFF1a: `document.fonts` re-reads the web-font load states (a load started or finished).
    pub(crate) fn fonts_sync(&self) {
        self.eval("fonts-sync", "globalThis.__scratchy_fonts_sync && globalThis.__scratchy_fonts_sync()");
    }

    pub(crate) fn observer_hook(&self) {
        if crate::ro_geom::active() {
            self.eval("ro-run", "globalThis.__scratchy_roRun && globalThis.__scratchy_roRun()");
        }
        self.eval("io-run", "globalThis.__scratchy_ioRun && globalThis.__scratchy_ioRun()");
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
    start_scripts_hooked(document, scripts, log, clock, &|_| {})
}

/// QWPT4a: `start_scripts_clocked` with `hook` run on the started engine (natives and preludes in
/// place) before the page's first script; the testharness child installs `__scratchy_wpt_done` there.
pub(crate) fn start_scripts_hooked(
    document: &Handle,
    scripts: &[ScriptSource],
    log: LogFn,
    clock: Option<Rc<std::cell::Cell<f64>>>,
    hook: &dyn Fn(&JsEngine),
) -> Option<JsEngine> {
    perf::mark("engine_start");
    let store = Rc::new(js_modules::ModuleStore::from_scripts(scripts, &*log));
    let engine = startup_engine(document, store, log, clock)?;
    hook(&engine);
    perf::mark("engine_ready");
    run_page(engine, document, scripts)
}

impl JsEngine {
    /// QWPT4a: the testharness child's native `__scratchy_wpt_done(json)` (only in that mode): the
    /// first call's argument lands in `sink`, later calls are ignored.
    pub(crate) fn install_wpt_done(&self, sink: Rc<std::cell::RefCell<Option<String>>>) {
        self.eng.with(|ctx| {
            native!(&ctx, "__scratchy_wpt_done", move |json: String| {
                let mut s = sink.borrow_mut();
                if s.is_none() {
                    *s = Some(json);
                }
            });
        });
    }

    /// QWPT7: `--wpt-th-one` only: the testdriver vendor (`tools/wpt/testdriver-vendor.js`) calls
    /// `__scratchy_td_wake()` when it queued an action; the runner then asks `td_next` for ticks.
    pub(crate) fn install_testdriver(&self, wake: Rc<Cell<bool>>) {
        self.eng.with(|ctx| {
            native!(&ctx, "__scratchy_td_wake", move || wake.set(true));
        });
    }

    /// QWPT7: the vendor's next tick (`__scratchy_td_next`, ops one per line; see wpt_input.rs),
    /// `None` when its queue is empty (or no vendor is loaded).
    pub(crate) fn td_next(&self) -> Option<String> {
        self.eval_value::<String>("testdriver", "typeof __scratchy_td_next === 'function' ? __scratchy_td_next() : ''").filter(|s| !s.is_empty())
    }

    /// QWPT7: the element under viewport point (`x`, `y`) of the page's real layout (the
    /// forced-layout cache, `GeomCache::hit`), `None` outside the viewport or over nothing.
    pub(crate) fn element_at(&self, x: f32, y: f32) -> Option<Handle> {
        let id: i32 = self.eval_value("hit", &format!("__scratchy_hit({x},{y})"))?;
        if id < 0 {
            return None;
        }
        self.table.borrow().get(id)
    }

    /// QWPT7: the JS wrapper id of `h` (`__scratchy_node(id)` in script).
    pub(crate) fn intern_node(&self, h: &Handle) -> i32 {
        self.table.borrow_mut().intern(h)
    }

    /// QWPT7: the node behind a JS wrapper id (`el.__id`), if it is still known.
    pub(crate) fn node_by_js_id(&self, id: i32) -> Option<Handle> {
        if id < 0 {
            return None;
        }
        self.table.borrow().get(id)
    }
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
        // QPT1a: same-origin iframe realms (R53 D1; the DOM prelude's `frameOf` calls it).
        js_engine::install_realm_native(&ctx);
        native!(&ctx, "__url_parse", url_parse_native);
        native!(&ctx, "__url_set", url_set_native);
        // QY10: `matchMedia` evaluates through the CSS media-query code.
        native!(&ctx, "__scratchy_media", |q: String| -> Vec<String> {
            // QWPT4a: the same viewport as innerWidth/innerHeight (the window, or 800x600 in the WPT child).
            let (vw, vh) = env_viewport();
            let (m, media) = css::media_query_string_matches(&q, vw as f32, vh as f32);
            vec![if m { "1".into() } else { String::new() }, media]
        });
        // QCSS2: `CSS.supports` shares `@supports`' evaluator (css.rs).
        native!(&ctx, "__scratchy_supports1", |c: String| -> bool { css::supports_condition_text(&c) });
        native!(&ctx, "__scratchy_supports2", |p: String, v: String| -> bool { css::supports_property_value(&p, &v) });
        // QFF1a: `document.fonts` and the web-font registry (fonts_prelude.js).
        native!(&ctx, "__scratchy_font_state", |key: String| -> String { crate::webfonts::state_of(&key).word().to_string() });
        native!(&ctx, "__scratchy_font_add", |key: String, family: String, bytes: crate::js_engine::Bytes, weight: String, style: String| -> bool {
            crate::webfonts::add_script_face(&key, &family, bytes.0, &weight, &style)
        });
        native!(&ctx, "__scratchy_font_check", |bytes: crate::js_engine::Bytes| -> bool { crate::webfonts::decode_face(bytes.0).is_some() });
        native!(&ctx, "__scratchy_font_remove", |key: String| crate::webfonts::remove_script_face(&key));
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
    let storage = crate::js_storage::LocalStorage::default();
    eng.with(|c| storage.install(&c));
    eng.with(|c| crate::js_storage::install_session(&c));
    js_dom::QEL7_BITS.with(|b| b.set(0));
    let mut engine = JsEngine { eng, log, base, media: None, dirty: dirty.clone(), table: Default::default(), reqs: reqs.clone(), net, ws, storage, anim: Default::default() };
    {
        let (budget, lg) = (engine.eng.budget.clone(), engine.log.clone());
        engine.eng.with(|c| install_script_natives(&c, budget, lg, store));
    }
    let prebuilt = engine.eng.prebuilt();
    if !prebuilt {
        engine.eval("prelude", PRELUDE);
        engine.eval("timers", TIMERS_PRELUDE);
        engine.eval("url", URL_PRELUDE);
        engine.eval("globals", globals_prelude());
        engine.eval("intl", INTL_PRELUDE);
        engine.eval("fetch", FETCH_PRELUDE);
        engine.eval("websocket", WS_PRELUDE);
    }
    engine.table = engine.eng.with(|ctx| js_dom::install_dom(&ctx, document, dirty.clone(), reqs.clone()));
    if !prebuilt {
        engine.eval("cssom", CSSOM_PRELUDE);
        engine.eval("fonts", FONTS_PRELUDE);
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
    // QSTASH2: the origin's stored pairs, before the first page script (opaque origins stay in memory).
    engine.eval("env", "__scratchy_ls_fill()");
    if let Some(e) = det_epoch() {
        engine.eval("deterministic", &DET_DATE_JS.replace("%EPOCH%", &e.to_string()));
    }
    crate::missing_api::install(&engine); // QMAPI1: report mode only
    Some(engine)
}

/// QPT1e: the prelude sources in the order the page engine runs them at startup without a snapshot
/// (`startup_engine`), for an iframe realm made without the snapshot to replay. Page-only parts
/// (`__scratchy_apply_env`, `__media_sweep`) are left out: `__scratchy_realm_init` applies the env.
pub(crate) fn realm_preludes() -> [&'static str; 12] {
    [PRELUDE, TIMERS_PRELUDE, URL_PRELUDE, globals_prelude(), INTL_PRELUDE, FETCH_PRELUDE, WS_PRELUDE, js_dom::DOM_PRELUDE, CSSOM_PRELUDE, FONTS_PRELUDE, js_media::MEDIA_PRELUDE, js_media::MSE_PRELUDE]
}

/// The page's own scripts and lifecycle events, over a started engine.
fn run_page(engine: JsEngine, document: &Handle, scripts: &[ScriptSource]) -> Option<JsEngine> {
    if let Some(mime) = PAGE_XML_MIME.with(|m| m.borrow_mut().take()) {
        engine.eval_value::<()>("xml-doc", &format!("__scratchy_xml_doc({mime:?})"));
    }
    let mut script_els = Vec::new();
    collect_script_elements(document, &mut script_els);
    let el_id = |s: &ScriptSource| match s.index.and_then(|i| script_els.get(i)) {
        Some(h) => engine.table.borrow_mut().intern(h),
        None => -1,
    };
    // QSCR404: a script that could not be fetched is not run; the element gets `error` (and `onerror`).
    let fire_error = |s: &ScriptSource| {
        (engine.log)("error", &format!("{}: {}", s.name, s.code));
        engine.eval_value::<()>("script-error", &format!("__scratchy_script_error({})", el_id(s)));
    };
    for s in scripts.iter().filter(|s| matches!(s.kind, ScriptKind::Classic | ScriptKind::ClassicError)) {
        if s.kind == ScriptKind::ClassicError {
            fire_error(s);
            continue;
        }
        let id = el_id(s);
        engine.eval_value::<()>("script", &format!("__scratchy_cs({id})"));
        engine.eval_as(Some(perf::Kind::ClassicScript), &s.name, &s.code);
        engine.eval_value::<()>("script", "__scratchy_cs(-1)");
    }
    // QY15: module scripts are deferred: after parsing (and the classic
    // scripts), in document order, before DOMContentLoaded.
    let store = js_modules::ModuleStore::from_scripts(scripts, &|_, _| {});
    let missing: Vec<&str> = scripts.iter().filter(|s| s.kind == ScriptKind::ModuleMissing).map(|s| s.name.as_str()).collect();
    for s in scripts.iter().filter(|s| matches!(s.kind, ScriptKind::Module | ScriptKind::ModuleError)) {
        if s.kind == ScriptKind::ModuleError {
            fire_error(s);
        } else if let Some(m) = js_modules::static_graph_missing(&s.name, &store, &missing) {
            fire_error(&ScriptSource { code: format!("failed to load module {m}"), ..s.clone() });
        } else {
            engine.eval_module(&s.name, s.code.len());
        }
    }
    engine.eval_as(Some(perf::Kind::Event), "DOMContentLoaded", FIRE_DOM_CONTENT_LOADED);
    perf::mark("dom_content_loaded");
    // QPT1f (R53 D9): the page's srcdoc iframes load (their scripts, DOMContentLoaded, load and the
    // <iframe>'s load) before the page's own load event, as in Chrome.
    engine.eval("frames-boot", "__scratchy_frames_boot()");
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
        // QSCR404: a URL only reached by a literal `import()` gets a leading `?`: it is still
        // prefetched (so the import resolves) but its failure is not the script's failure.
        let all = js_modules::scan_imports(&code);
        let resolved: Vec<(String, bool)> = all.iter().filter_map(|(sp, dynamic)| js_modules::resolve_specifier(sp, &name, &st.map, &st.doc_base).ok().map(|u| (u, *dynamic))).collect();
        let mut out: Vec<String> = Vec::new();
        for (u, dynamic) in &resolved {
            if is_data_url(u) || st.sources.borrow().contains_key(u) {
                continue;
            }
            let lazy = *dynamic && !resolved.iter().any(|(v, d)| v == u && !*d);
            let tagged = if lazy { format!("?{u}") } else { u.clone() };
            if !out.contains(&tagged) {
                out.push(tagged);
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
/// `globals_prelude.js` with `__LS_QUOTA_UNITS__` replaced by `config::QUOTA_UNITS` (QCONF2), so the
/// page's synchronous quota check and the Rust side (`stash`) use the one value.
fn globals_prelude() -> &'static str {
    static SRC: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| include_str!("globals_prelude.js").replace("__LS_QUOTA_UNITS__", &crate::config::QUOTA_UNITS.to_string()));
    SRC.as_str()
}

/// QY16: a minimal en-US `Intl` (QuickJS has none) and the `toLocale*String`
/// methods routed through it. See the file header and TODO.txt QY16a.
const INTL_PRELUDE: &str = include_str!("intl_prelude.js");

/// QSD4: constructable stylesheets and `document.adoptedStyleSheets`.
const CSSOM_PRELUDE: &str = include_str!("cssom_prelude.js");

/// QAPI4: `document.fonts` (FontFaceSet), `FontFace`, `FontFaceSetLoadEvent` (plain JS over the @font-face rules).
const FONTS_PRELUDE: &str = include_str!("fonts_prelude.js");

/// QF2: Blob, File, FileReader, blob: URL store (`__blob_lookup`); the
/// home of the fetch/XHR data classes (QF memo in TODO.txt).
const FETCH_PRELUDE: &str = concat!(include_str!("fetch_prelude.js"), "\n", include_str!("data_transfer_prelude.js"));

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
  // QPT1a (R53 D5): an iframe realm queues its timers on its parent's table (`fwd`), so the one pump
  // Rust drives runs them; a callback still runs in its own realm, with that realm's window as `this`.
  var fwd = null, fwdClear = null, fwdCancel = null;
  var add = function (fn, delay, args, repeat, recv) {
    if (typeof fn !== 'function') {
      var code = String(fn);
      fn = function () { (0, eval)(code); };
      args = [];
    }
    // QPT1d3: a closed window's timers never fire (Chrome); the call still returns an id.
    if (fwd) return g.closed ? nextId++ : fwd(fn, delay, args, repeat, recv || g);
    var id = nextId++;
    delay = Number(delay);
    if (!(delay > 0)) delay = 0;
    // QEL5 (HTML timer initialisation steps): the new timer's nesting level is the running timer's + 1
    // (0 outside a timer callback); only a level above 5 is clamped to 4 ms. An interval keeps its
    // requested `rep` and re-evaluates the clamp each time it re-arms, with its level + 1.
    var lvl = level + 1;
    var due = __scratchy_now() + (lvl > 5 ? Math.max(delay, 4) : delay);
    timers.push({ id: id, fn: fn, args: args, due: due, rep: repeat ? delay : -1, lvl: lvl, recv: recv });
    return id;
  };
  // QEL1 (R64 E1/E2): animation frame callbacks are NOT timers. Each realm numbers its own ids from 1;
  // the callbacks wait in one list (an iframe realm forwards to its parent's, as timers do) that the
  // Rust rendering-steps driver (event_loop.rs) runs as ONE batch per frame, all with the same
  // timestamp, never split by the timer slice. `rafSince` = when the list became non-empty: the frame
  // is due at the next 60 Hz boundary of the script clock STRICTLY after it (QY30).
  var rafs = [], rafRun = [], nextRaf = 1, rafSince = 0, rafTs = 0;
  var fwdRaf = null, fwdRafCancel = null;
  var rafAdd = function (fn, id, recv) {
    if (fwdRaf) return fwdRaf(fn, id, recv);
    if (!rafs.length) rafSince = __scratchy_now();
    rafs.push({ fn: fn, id: id, recv: recv, dead: false });
  };
  var rafCancel = function (id, recv) {
    if (fwdRafCancel) return fwdRafCancel(id, recv);
    var i;
    for (i = 0; i < rafs.length; i++) if (rafs[i].id === id && rafs[i].recv === recv) { rafs.splice(i, 1); return; }
    // cancelled from inside the running batch: it must not run either
    for (i = 0; i < rafRun.length; i++) if (rafRun[i].id === id && rafRun[i].recv === recv) { rafRun[i].dead = true; return; }
  };
  var clear = function (id) {
    if (fwdClear) return fwdClear(id);
    for (var i = 0; i < timers.length; i++) if (timers[i].id === id) { timers.splice(i, 1); return; }
  };
  // QPT1b (R53 D6): drops every timer, idle callback and rAF a dropped realm (`recv`) queued.
  var cancelFor = function (recv) {
    if (fwdCancel) return fwdCancel(recv);
    for (var i = timers.length - 1; i >= 0; i--) if (timers[i].recv === recv) timers.splice(i, 1);
    for (i = rafs.length - 1; i >= 0; i--) if (rafs[i].recv === recv) rafs.splice(i, 1);
    for (i = 0; i < rafRun.length; i++) if (rafRun[i].recv === recv) rafRun[i].dead = true;
  };
  Object.defineProperty(g, '__scratchy_timers_cancel', { value: cancelFor });
  Object.defineProperty(g, '__scratchy_timers_add', { value: add });
  Object.defineProperty(g, '__scratchy_timers_clear', { value: clear });
  Object.defineProperty(g, '__scratchy_raf_add', { value: rafAdd });
  Object.defineProperty(g, '__scratchy_raf_cancel', { value: rafCancel });
  Object.defineProperty(g, '__scratchy_timers_fwd', { value: function (p) {
    fwd = p.__scratchy_timers_add; fwdClear = p.__scratchy_timers_clear; fwdCancel = p.__scratchy_timers_cancel;
    fwdRaf = p.__scratchy_raf_add; fwdRafCancel = p.__scratchy_raf_cancel;
    delete g.__scratchy_timers_fwd;
  }, configurable: true });
  var def = function (name, fn) {
    Object.defineProperty(g, name, { value: fn, writable: true, configurable: true, enumerable: true });
  };
  def('setTimeout', function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), false); });
  def('setInterval', function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), true); });
  def('clearTimeout', clear);
  def('clearInterval', clear);
  def('requestAnimationFrame', function (fn) {
    if (typeof fn !== 'function') throw new TypeError('requestAnimationFrame: callback is not a function');
    var id = nextRaf++;
    if (!g.closed) rafAdd(fn, id, g);
    return id;
  });
  def('cancelAnimationFrame', function (id) { rafCancel(Number(id), g); });
  // QEL1: the time of the frame the pending callbacks wait for, -1 when there are none.
  Object.defineProperty(g, '__scratchy_raf_due', { value: function () {
    // QEL2: a ResizeObserver that may have a new size also wants a rendering update.
    // QEL3: so does an IntersectionObserver that may have a new intersection.
    var ro = g.__scratchy_ro_want ? g.__scratchy_ro_want() : -1;
    var io = g.__scratchy_io_want ? g.__scratchy_io_want() : -1;
    if (!rafs.length && ro < 0 && io < 0) return -1;
    var since = rafs.length ? rafSince : -1;
    if (ro >= 0 && (since < 0 || ro < since)) since = ro;
    if (io >= 0 && (since < 0 || io < since)) since = io;
    var due = (Math.floor(since / FRAME) + 1) * FRAME;
    while (due <= since) due += FRAME;
    return due;
  } });
  // QEL1: "run the animation frame callbacks": takes the list as it is now (callbacks requested
  // during the batch wait for the next frame) and returns its length; Rust then calls
  // `__scratchy_raf_run_one` until it returns false, each call its own eval so the microtask
  // checkpoint runs after every callback.
  Object.defineProperty(g, '__scratchy_raf_begin', { value: function (ts) {
    rafRun = rafs; rafs = []; rafTs = ts;
    return rafRun.length;
  } });
  // QEL4: "report the exception" for a timer / rAF callback, in the callback's own global
  // (WPT callback-cross-realm-report-exception): ErrorEvent + onerror; the log line only when unhandled.
  var reportCb = function (e, fn, recv) {
    var G = recv;
    try { var F = fn.constructor; if (typeof F === 'function' && F.name === 'Function') G = F('return globalThis')(); } catch (x) {}
    var handled = false;
    try { handled = (G.__scratchy_report_error || g.__scratchy_report_error)(e, '', 0, 0); } catch (x) {}
    if (!handled) __scratchy_err('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : ''));
  };
  Object.defineProperty(g, '__scratchy_raf_run_one', { value: function () {
    while (rafRun.length && rafRun[0].dead) rafRun.shift();
    if (!rafRun.length) return false;
    var c = rafRun.shift(), saved = level;
    level = 0;
    if (g.__perf_src) g.__perf_src(c.fn);
    try { c.fn.call(c.recv, rafTs); }
    catch (e) {
      reportCb(e, c.fn, c.recv);
    }
    level = saved;
    return true;
  } });
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
    var saved = level, runLvl = t.lvl;
    if (t.rep >= 0) { t.lvl = runLvl + 1; t.due = now + (t.lvl > 5 ? Math.max(t.rep, 4) : t.rep); } else timers.splice(best, 1);
    level = runLvl;
    if (g.__perf_src) g.__perf_src(t.fn);
    try { t.fn.apply(t.recv || g, t.args); }
    catch (e) { reportCb(e, t.fn, t.recv || g); }
    level = saved;
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
                    // QEL1: the next timer or virtual rendering frame (rAF batch), whichever is first.
                    let Some(due) = crate::event_loop::headless_next_due(engine) else { break };
                    if due > limit as f64 {
                        break;
                    }
                    if due > clock.get() {
                        clock.set(due);
                    }
                    let n = crate::event_loop::headless_turn(engine, clock.get());
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
    let mut state_dir = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--socket" {
            socket = it.next().map(std::path::PathBuf::from);
        } else if a == "--user-agent" || a == "--budget-ms" {
            it.next(); // installed by `run` (QY39b, QV8-0)
        } else if a == "--run-timers" {
            run_timers = it.next().and_then(|v| v.parse::<u64>().ok());
        } else if a == "--state-dir" {
            // QSTASH2: only an explicit flag persists here (no env, no default): reports stay reproducible.
            state_dir = it.next().map(std::path::PathBuf::from);
        } else {
            file = Some(a.clone());
        }
    }
    js_engine::ensure_snapshot(); // QV8-4: one-shot mode: build the startup snapshot first (V8)
    let Some(file) = file else {
        diag!("usage: scratchy --js-report [--socket <path>] [--state-dir <dir>] [--user-agent <string>] [--run-timers <ms>] [--budget-ms <ms>] <file>");
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
    if let Some(dir) = &state_dir {
        crate::js_storage::set_backend(crate::js_storage::Backend::open(dir));
    }
    write_raw_stdout(format_args!("{}", js_report_for_html_timers(&decode_html_bytes(&bytes, None), socket.as_deref(), &file, run_timers)));
    crate::js_storage::shutdown();
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
