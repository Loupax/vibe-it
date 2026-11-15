//! Q33a: running classic `<script>`s with QuickJS (`rquickjs`).
//!
//! Scope of this first slice: no DOM API at all. A page's scripts run in
//! document order inside a fresh QuickJS context that has only a tiny
//! prelude (`console`, `window`/`document` stubs that can register
//! `DOMContentLoaded`/`load` listeners), then those two events fire. A
//! script error is logged to stderr and never stops the next script or
//! the page. The engine is created per page load and dropped afterwards
//! (Q33d will need to keep it alive for events/timers).
use super::*;
use rquickjs::context::EvalOptions;
use rquickjs::{Context, FromJs, Ctx, Error as JsError, Exception, Function, Runtime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

/// Inline scripts only, in document order — for pages with no way to
/// fetch external ones (no `--socket`, no base URL).
pub(crate) fn inline_scripts(document: &Handle) -> Vec<ScriptSource> {
    let mut res = ExternalResources::default();
    find_external_resources(document, &mut res);
    res.scripts
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            ScriptRef::Inline(code) => Some(ScriptSource { name: format!("inline script #{}", i + 1), code }),
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
                    ScriptRef::Inline(code) => Some(ScriptSource { name: format!("inline script #{}", i + 1), code }),
                    ScriptRef::External(src) => {
                        let url = resolve_href(Some(base), &src).ok()?;
                        match fetch_script_via_wire(path, &url) {
                            Some(code) => Some(ScriptSource { name: url.to_string(), code }),
                            None => {
                                eprintln!("scratchy: js: could not fetch script {url}");
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
  var listeners = {};
  var target = function (o, key) {
    o.addEventListener = function (type, fn) {
      if (typeof fn !== 'function') return;
      (listeners[key + type] = listeners[key + type] || []).push(fn);
    };
    o.removeEventListener = function (type, fn) {
      var l = listeners[key + type] || [];
      var i = l.indexOf(fn);
      if (i >= 0) l.splice(i, 1);
    };
    return o;
  };
  g.window = g; g.self = g;
  target(g, 'w:');
  g.document = target({ readyState: 'loading' }, 'd:');
  Object.defineProperty(g, '__scratchy_fire', { value: function (key, type, handler) {
    var list = (listeners[key + type] || []).slice();
    if (typeof handler === 'function') list.push(handler);
    list.forEach(function (fn) {
      try { fn.call(key === 'w:' ? g : g.document, { type: type }); }
      catch (e) { console.error('Uncaught ' + String(e) + (e && e.stack ? '\\n' + e.stack : '')); }
    });
  } });
})(globalThis);
"#;

const FIRE_DOM_CONTENT_LOADED: &str = "document.readyState='interactive';__scratchy_fire('d:','DOMContentLoaded');";
const FIRE_LOAD: &str = "document.readyState='complete';__scratchy_fire('w:','load',window.onload);";

/// Total wall-clock budget for one page's scripts; QuickJS is
/// interrupted past it so a runaway loop cannot freeze the window.
/// Sink for console output and script errors: `(level, text)`.
pub(crate) type LogFn = Rc<dyn Fn(&str, &str)>;

const SCRIPT_BUDGET: Duration = Duration::from_secs(5);
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

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

/// Runs `scripts` in order, then `DOMContentLoaded` and `load`. `log`
/// receives `(level, text)` for every `console` call and every script
/// error (level `error`); the real caller prints those to stderr.
#[cfg(test)]
pub(crate) fn run_scripts_with(scripts: &[ScriptSource], log: LogFn) {
    run_scripts_in(&parse_html(""), scripts, log);
}

/// `run_scripts_with` against a real document (Q33b DOM read API).
pub(crate) fn run_scripts_in(document: &Handle, scripts: &[ScriptSource], log: LogFn) {
    let Ok(rt) = Runtime::new() else {
        log("error", "could not create the JS runtime");
        return;
    };
    rt.set_memory_limit(MEMORY_LIMIT);
    let deadline = Instant::now() + SCRIPT_BUDGET;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let Ok(ctx) = Context::full(&rt) else {
        log("error", "could not create the JS context");
        return;
    };
    ctx.with(|ctx| {
        let l = log.clone();
        let native = Function::new(ctx.clone(), move |level: String, text: String| l(&level, &text));
        if let Ok(native) = native {
            let _ = ctx.globals().set("__scratchy_log", native);
        }
    });
    let eval = |name: &str, code: &str| {
        ctx.with(|ctx| {
            // HTML classic scripts are sloppy unless they opt in; rquickjs
            // defaults to strict, so build the options explicitly (R5).
            let mut opts = EvalOptions::default();
            opts.strict = false;
            opts.filename = Some(name.to_string());
            if let Err(e) = ctx.eval_with_options::<(), _>(code, opts) {
                log("error", &format!("{name}: {}", describe_error(&ctx, e)));
            }
        });
        drain_jobs(&rt, &*log);
    };
    eval("prelude", PRELUDE);
    ctx.with(|ctx| js_dom::install_dom(&ctx, document));
    for s in scripts {
        eval(&s.name, &s.code);
    }
    eval("DOMContentLoaded", FIRE_DOM_CONTENT_LOADED);
    eval("load", FIRE_LOAD);
}

/// The real entry point: runs a page's scripts (no-op with JS off or
/// nothing to run) with console output going to stderr.
pub(crate) fn run_page_scripts(document: &Handle, scripts: &[ScriptSource]) {
    if !js_enabled() || scripts.is_empty() {
        return;
    }
    run_scripts_in(
        document,
        scripts,
        Rc::new(|level, text| {
            if level == "error" {
                eprintln!("scratchy: js error: {text}");
            } else {
                eprintln!("scratchy: console.{level}: {text}");
            }
        }),
    );
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
        eprintln!("usage: scratchy --js-report [--socket <path>] <file>");
        return 2;
    };
    let bytes = match fs::read(&file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("scratchy: {file}: {e}");
            return 2;
        }
    };
    set_js_enabled(true);
    print!("{}", js_report_for_html(&decode_html_bytes(&bytes, None), socket.as_deref(), &file));
    0
}

pub(crate) fn js_report_for_html(html: &str, socket: Option<&std::path::Path>, file: &str) -> String {
    let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return "could not parse page\n".into() };
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
