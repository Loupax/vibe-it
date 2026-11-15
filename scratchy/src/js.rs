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
use rquickjs::{Context, Ctx, Error as JsError, Exception, Function, Runtime};
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
        .filter_map(|s| match s {
            ScriptRef::Inline(code) => Some(ScriptSource { name: "inline script".into(), code }),
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
            .map(|r| {
                scope.spawn(move || match r {
                    ScriptRef::Inline(code) => Some(ScriptSource { name: "inline script".into(), code }),
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
      catch (e) { console.error('Uncaught ' + (e && e.stack ? e.stack : e)); }
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
            return match ex.stack() {
                Some(stack) if !stack.is_empty() => format!("{msg}\n{stack}"),
                _ => msg,
            };
        }
        return format!("uncaught {value:?}");
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
pub(crate) fn run_scripts_with(scripts: &[ScriptSource], log: LogFn) {
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
            if let Err(e) = ctx.eval::<(), _>(code) {
                log("error", &format!("{name}: {}", describe_error(&ctx, e)));
            }
        });
        drain_jobs(&rt, &*log);
    };
    eval("prelude", PRELUDE);
    for s in scripts {
        eval(&s.name, &s.code);
    }
    eval("DOMContentLoaded", FIRE_DOM_CONTENT_LOADED);
    eval("load", FIRE_LOAD);
}

/// The real entry point: runs a page's scripts (no-op with JS off or
/// nothing to run) with console output going to stderr.
pub(crate) fn run_page_scripts(scripts: &[ScriptSource]) {
    if !js_enabled() || scripts.is_empty() {
        return;
    }
    run_scripts_with(
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
