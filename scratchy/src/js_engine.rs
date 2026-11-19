//! QV8-2: the JS engine seam. Everything that names a QuickJS (`rquickjs`)
//! type or call lives here; the rest of the crate talks to this module only.
//! QuickJS is the only backend today. A V8 backend (QV8-3) replaces the
//! bodies, not the shape:
//!
//! - value conversion: natives take and return plain Rust types (`i32`, `u32`,
//!   `f64`, `bool`, `String`, `Vec<_>`, `Option<_>`, [`Bytes`], [`JsObject`]);
//! - [`register`] / [`native!`]: the one way to install a global native (the
//!   per-file `native!` macros are gone). A native closure's lifetime is owned
//!   by its function object in QuickJS; a V8 backend keeps closures in a
//!   per-engine arena reached through the [`Ctx`] handle (the native gets a
//!   `&Ctx`, never an engine pointer, so the arena stays an implementation
//!   detail);
//! - [`JsObject`]: reader/builder for the object-shaped arguments and results
//!   (js_net's request descriptor and completions);
//! - [`Bytes`] / [`BytesRef`]: `Uint8Array` in and out (js_net, js_ws, the UTF-8
//!   decoder);
//! - [`Source`] + [`Compiled`]: compile and run global code in two steps, so
//!   the caller can arm the budget between them; usable from inside a native
//!   (the `__scratchy_run_global` nested run takes a `Ctx`);
//! - [`Engine`]: the runtime and context, the CPU [`Budget`] (deadline +
//!   tripped flag), promise-job draining, module loading, memory and stack
//!   limits, the rejection hook;
//! - error and stack formatting ([`describe_error`]).
//!
//! The 14 raw `rquickjs::qjs` calls (compile, run, version, HTMLDDA) are all
//! in this file.
use super::*;
use rquickjs::context::EvalOptions;
use rquickjs::function::{IntoArgs, IntoJsFunc};
use rquickjs::loader::{Loader, Resolver};
use rquickjs::{qjs, Coerced, Context, Exception, FromJs, Function, IntoJs, Module, Object, Runtime, TypedArray, Value};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The call context handed to every native and to `Engine::with`.
pub(crate) type Ctx<'js> = rquickjs::Ctx<'js>;
pub(crate) type JsError = rquickjs::Error;
pub(crate) type JsResult<T> = rquickjs::Result<T>;

/// A type a native or an eval can return into Rust.
pub(crate) trait FromJsRet: for<'js> FromJs<'js> {}
impl<T: for<'js> FromJs<'js>> FromJsRet for T {}

// ---------------------------------------------------------------- natives

/// Installs `f` as the global function `name`.
pub(crate) fn register<'js, P, F>(ctx: &Ctx<'js>, name: &str, f: F)
where
    F: IntoJsFunc<'js, P> + 'js,
{
    if let Ok(func) = Function::new(ctx.clone(), f) {
        let _ = ctx.globals().set(name, func);
    }
}

/// QV8-4: the QuickJS backend has no startup snapshot: the preludes always run.
pub(crate) fn prebuilt(_ctx: &Ctx<'_>) -> bool {
    false
}

/// QV8-4: no-ops here (V8 builds its startup snapshot in the background).
pub(crate) fn warm_up() {}

/// QOBS3: the QuickJS backend gives no script URLs through rquickjs, so `by_caller` stays empty.
pub(crate) fn caller_frame(_ctx: &Ctx<'_>, _skip: &[&str]) -> Option<String> {
    None
}
pub(crate) fn baking() -> bool {
    false
}
pub(crate) fn ensure_snapshot() {}

/// `native!(ctx, "__name", |a: i32, b: String| -> String { .. })`.
macro_rules! native {
    ($ctx:expr, $name:expr, $f:expr) => {
        $crate::js_engine::register($ctx, $name, $f)
    };
}
pub(crate) use native;

/// Calls the global function `name` with `args`; `None` when it is not defined.
pub(crate) fn call_global<'js, A, R>(ctx: &Ctx<'js>, name: &str, args: A) -> Option<JsResult<R>>
where
    A: IntoArgs<'js>,
    R: FromJs<'js>,
{
    call_global_with(ctx, name, || Ok(args))
}

/// `call_global` that builds the arguments only when the function exists.
pub(crate) fn call_global_with<'js, A, R>(ctx: &Ctx<'js>, name: &str, make: impl FnOnce() -> JsResult<A>) -> Option<JsResult<R>>
where
    A: IntoArgs<'js>,
    R: FromJs<'js>,
{
    let f: Function = ctx.globals().get(name).ok()?;
    Some(make().and_then(|a| f.call(a)))
}

/// The error for "that global is not a function" (a strict `call_global`).
pub(crate) fn not_a_function() -> JsError {
    JsError::new_from_js("undefined", "function")
}

/// The error that means "the JS exception is pending" (rethrown to the caller).
pub(crate) fn pending_exception() -> JsError {
    JsError::Exception
}

pub(crate) fn set_global<'js, V: IntoJs<'js>>(ctx: &Ctx<'js>, name: &str, v: V) -> JsResult<()> {
    ctx.globals().set(name, v)
}

pub(crate) fn remove_global(ctx: &Ctx<'_>, name: &str) {
    let _ = ctx.globals().remove(name);
}

/// Global `name` as an object reader, if it is one.
pub(crate) fn global_object<'js>(ctx: &Ctx<'js>, name: &str) -> Option<JsObject<'js>> {
    ctx.globals().get::<_, Object>(name).ok().map(JsObject)
}

/// Runs `code` as a script for its side effects (no result, errors ignored).
pub(crate) fn eval_quiet(ctx: &Ctx<'_>, code: &str) {
    let _ = ctx.eval::<(), _>(code);
}

/// QV8-3b test seam: run the budget check now (the V8 watchdog's tick; QuickJS checks by itself).
#[cfg(test)]
pub(crate) fn poll_budget_now() {}

/// QY40: gives the global `name` the [[IsHTMLDDA]] slot (falsy, `typeof` 'undefined').
pub(crate) fn mark_htmldda(ctx: &Ctx<'_>, name: &str) {
    if let Ok(v) = ctx.globals().get::<_, Value>(name) {
        // SAFETY: `ctx` is live and `v` is a value of it.
        unsafe { qjs::JS_SetIsHTMLDDA(ctx.as_raw().as_ptr(), v.as_raw()) };
    }
}

// ---------------------------------------------------------------- values

/// A `Uint8Array` argument, copied out.
pub(crate) struct Bytes(pub(crate) Vec<u8>);

impl<'js> FromJs<'js> for Bytes {
    fn from_js(ctx: &Ctx<'js>, v: Value<'js>) -> JsResult<Self> {
        let ta = TypedArray::<u8>::from_js(ctx, v)?;
        // SAFETY: copied at once on the JS thread; no JS runs while the slice is alive.
        Ok(Bytes(unsafe { ta.as_bytes() }.map(<[u8]>::to_vec).unwrap_or_default()))
    }
}

/// A `Uint8Array` result built from a borrowed slice (one copy, into the engine).
pub(crate) struct BytesRef<'a>(pub(crate) &'a [u8]);

impl<'js> IntoJs<'js> for BytesRef<'_> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        TypedArray::<u8>::new_copy(ctx.clone(), self.0)?.into_js(ctx)
    }
}

/// A JS object: reader (`get`) and builder (`new` + `set`).
pub(crate) struct JsObject<'js>(Object<'js>);

impl<'js> JsObject<'js> {
    pub(crate) fn new(ctx: &Ctx<'js>) -> JsResult<Self> {
        Object::new(ctx.clone()).map(JsObject)
    }

    pub(crate) fn set<V: IntoJs<'js>>(&self, key: &str, v: V) -> JsResult<()> {
        self.0.set(key, v)
    }

    /// QOBS4: the function's name (QuickJS gives no script URL through rquickjs); empty when not a function.
    pub(crate) fn function_source(&self) -> String {
        if !self.0.is_function() {
            return String::new();
        }
        let name: String = self.0.get("name").unwrap_or_default();
        if name.is_empty() { "(anonymous)".to_string() } else { name }
    }

    /// The property converted to `T`; `None` when it is missing or not a `T`.
    pub(crate) fn get<T: FromJs<'js>>(&self, key: &str) -> Option<T> {
        self.0.get::<_, T>(key).ok()
    }
}

impl<'js> FromJs<'js> for JsObject<'js> {
    fn from_js(ctx: &Ctx<'js>, v: Value<'js>) -> JsResult<Self> {
        Object::from_js(ctx, v).map(JsObject)
    }
}

impl<'js> IntoJs<'js> for JsObject<'js> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        self.0.into_js(ctx)
    }
}

// ---------------------------------------------------------------- errors

/// A pending exception as the one log text: `Name: message` plus the stack, or
/// `Uncaught <value>` for a thrown non-error. Clears the exception.
pub(crate) fn describe_exception(ctx: &Ctx<'_>) -> String {
    describe_error(ctx, JsError::Exception)
}

pub(crate) fn describe_error(ctx: &Ctx<'_>, err: JsError) -> String {
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
        let text = Coerced::<String>::from_js(ctx, value.clone()).map(|c| c.0).unwrap_or_else(|_| "<unprintable value>".into());
        return format!("Uncaught {text}");
    }
    err.to_string()
}

/// Drops the pending exception without reading it.
pub(crate) fn discard_exception(ctx: &Ctx<'_>) {
    let _ = ctx.catch();
}

/// The engine's name and version for the perf report (a V8 backend fills the same pair).
pub(crate) fn engine_info() -> (&'static str, String) {
    // SAFETY: JS_GetVersion returns a pointer to a static NUL-terminated string.
    let v = unsafe { std::ffi::CStr::from_ptr(qjs::JS_GetVersion()) }.to_string_lossy().into_owned();
    ("QuickJS", format!("{v} (rquickjs 0.14.0)"))
}

// ---------------------------------------------------------------- eval

/// Global-script source ready for the engine (NUL-free), compiled in a separate
/// step from running so the caller can arm the budget in between.
pub(crate) struct Source {
    src: std::ffi::CString,
    file: std::ffi::CString,
    len: usize,
}

/// A compiled global script, consumed by `run`.
pub(crate) struct Compiled(qjs::JSValue);

impl Source {
    /// `None` when `code` or `name` contains a NUL byte.
    pub(crate) fn new(code: &str, name: &str) -> Option<Source> {
        Some(Source { src: std::ffi::CString::new(code).ok()?, file: std::ffi::CString::new(name).ok()?, len: code.len() })
    }

    /// Compiles as sloppy global code (HTML classic scripts are sloppy, R5).
    /// `Err(())` leaves the exception pending (`describe_exception`).
    pub(crate) fn compile(&self, ctx: &Ctx<'_>) -> Result<Compiled, ()> {
        let raw = ctx.as_raw().as_ptr();
        // SAFETY: `raw` is this live context; the source is NUL-terminated and
        // `len` excludes the terminator; the compiled function is consumed by
        // `Compiled::run`, whose result we own and free.
        let func = unsafe { qjs::JS_Eval(raw, self.src.as_ptr(), self.len as _, self.file.as_ptr(), (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32) };
        // SAFETY: `func` was returned by JS_Eval above.
        if unsafe { qjs::JS_IsException(func) } { Err(()) } else { Ok(Compiled(func)) }
    }
}

impl Compiled {
    /// Runs the script. `Err(())` leaves the exception pending.
    pub(crate) fn run(self, ctx: &Ctx<'_>) -> Result<(), ()> {
        let raw = ctx.as_raw().as_ptr();
        // SAFETY: consumes `self.0`; the result is wrapped so it is freed.
        let res = unsafe { qjs::JS_EvalFunction(raw, self.0) };
        // SAFETY: `res` is the value just returned.
        if unsafe { qjs::JS_IsException(res) } {
            return Err(());
        }
        // SAFETY: `res` is an owned value.
        drop(unsafe { Value::from_raw(ctx.clone(), res) });
        Ok(())
    }
}

/// Evaluates `code` as sloppy global code under the file name `label` and converts the result.
pub(crate) fn eval_sloppy<T: FromJsRet>(ctx: &Ctx<'_>, label: &str, code: &str) -> JsResult<T> {
    let mut opts = EvalOptions::default();
    opts.strict = false;
    opts.filename = Some(label.to_string());
    ctx.eval_with_options::<T, _>(code, opts)
}

// ---------------------------------------------------------------- budget

/// The CPU-time interrupt budget (QY5): a deadline on the script clock and the
/// flag the interrupt sets when it stops a script. Cheap to clone; natives that
/// arm it (the nested run) hold a clone.
#[derive(Clone)]
pub(crate) struct Budget {
    deadline: Arc<AtomicU64>,
    tripped: Arc<AtomicBool>,
}

impl Budget {
    fn new(initial: Duration) -> Budget {
        Budget { deadline: Arc::new(AtomicU64::new(script_clock_nanos() + initial.as_nanos() as u64)), tripped: Arc::new(AtomicBool::new(false)) }
    }

    /// Clears the tripped flag and starts a fresh `budget`.
    pub(crate) fn arm(&self, budget: Duration) {
        self.tripped.store(false, Ordering::Relaxed);
        self.deadline.store(script_clock_nanos() + budget.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Starts a fresh `budget` but keeps the tripped flag (a drain's per-job deadline).
    pub(crate) fn rearm_keep_tripped(&self, budget: Duration) {
        self.deadline.store(script_clock_nanos() + budget.as_nanos() as u64, Ordering::Relaxed);
    }

    /// True when the interrupt stopped a script since the last `arm`.
    pub(crate) fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------- engine

/// What `Engine::new` wires up.
pub(crate) struct EngineOpts {
    pub(crate) memory_limit: usize,
    pub(crate) stack_limit: usize,
    /// The page configuration: rejection hook, CPU interrupt (budget starts at
    /// `SCRIPT_BUDGET`) and the module loader. `None` is a bare engine.
    pub(crate) page: Option<Rc<js_modules::ModuleStore>>,
}

/// A live JS runtime and its one context. Field order matters: the context
/// drops before the runtime.
pub(crate) struct Engine {
    ctx: Context,
    rt: Runtime,
    pub(crate) budget: Budget,
}

impl Engine {
    /// `Err` is the log line for the failure.
    pub(crate) fn new(opts: EngineOpts) -> Result<Engine, &'static str> {
        let rt = Runtime::new().map_err(|_| "could not create the JS runtime")?;
        rt.set_memory_limit(opts.memory_limit);
        rt.set_max_stack_size(opts.stack_limit); // QY47a
        let budget = Budget::new(SCRIPT_BUDGET);
        if let Some(store) = &opts.page {
            // QY14: forward every rejection event to the prelude's queue.
            rt.set_host_promise_rejection_tracker(Some(Box::new(|ctx, promise, reason, handled| {
                if call_global::<_, ()>(&ctx, "__scratchy_rej_push", (promise, reason, handled)).is_some_and(|r| r.is_err()) {
                    discard_exception(&ctx);
                }
            })));
            let b = budget.clone();
            rt.set_interrupt_handler(Some(Box::new(move || {
                let over = script_clock_nanos() > b.deadline.load(Ordering::Relaxed);
                if over {
                    b.tripped.store(true, Ordering::Relaxed);
                }
                over
            })));
            rt.set_loader(PageResolver(store.clone()), PageLoader(store.clone()));
        }
        let ctx = Context::full(&rt).map_err(|_| "could not create the JS context")?;
        Ok(Engine { ctx, rt, budget })
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(Ctx<'_>) -> R) -> R {
        self.ctx.with(f)
    }

    pub(crate) fn is_job_pending(&self) -> bool {
        self.rt.is_job_pending()
    }

    /// Bytes the engine reports as in use (the perf report's JS heap).
    pub(crate) fn memory_used(&self) -> i64 {
        self.rt.memory_usage().memory_used_size
    }

    /// QOBS7: `(used, total, external)` heap bytes; QuickJS has no external figure.
    pub(crate) fn heap_stats(&self) -> (i64, i64, Option<i64>) {
        let m = self.rt.memory_usage();
        (m.memory_used_size, m.malloc_size, None)
    }

    /// Runs every queued promise job, ignoring failures and the budget (the bare corpus eval).
    pub(crate) fn run_all_jobs(&self) {
        while self.rt.is_job_pending() {
            let _ = self.rt.execute_pending_job();
        }
    }

    /// Runs pending promise jobs, logging the message of any job that throws
    /// (QY14), then reports rejections that stayed unhandled via the prelude's
    /// `__scratchy_rej_flush` (which may queue more jobs: loop until quiet).
    pub(crate) fn drain_jobs(&self, log: &dyn Fn(&str, &str)) {
        // QY46: every job gets its own `CALLBACK_BUDGET` (a runaway job is still
        // killed at 500 ms) and the whole drain stops starting jobs at `DRAIN_CAP`;
        // the rest stay queued and `next_deadline` reports them as due.
        let drain_start = script_clock_nanos();
        // `tripped` stays sticky across jobs on purpose: a module entry's trip is
        // caught into a rejection and reported by the caller after the drain.
        let arm = || self.budget.rearm_keep_tripped(perf::callback_budget());
        // QY22/QY44a: a job killed by the budget is uncatchable (skips `finally`),
        // so once one trips we STOP: later jobs must not start on an expired
        // deadline (they would be killed part-way and leave library state stale).
        // They stay queued in the engine and run in the next turn's fresh budget.
        let mut killed = 0usize;
        'drain: loop {
            while self.rt.is_job_pending() {
                if script_clock_nanos().saturating_sub(drain_start) >= perf::drain_cap().as_nanos() as u64 {
                    break 'drain;
                }
                arm();
                let span = perf::begin(Some(perf::job_kind()));
                let res = self.rt.execute_pending_job();
                perf::end(span, res.is_err() && self.budget.tripped());
                if let Err(job) = res {
                    job.0.with(|c| {
                        let msg = describe_exception(&c);
                        if self.budget.tripped() {
                            killed += 1;
                        } else {
                            log("error", &format!("Uncaught (in promise job) {msg}"));
                        }
                    });
                    if killed > 0 {
                        break 'drain;
                    }
                }
            }
            arm();
            let more = self.ctx.with(|c| match call_global::<_, bool>(&c, "__scratchy_rej_flush", ()) {
                None => false,
                Some(Ok(b)) => b,
                Some(Err(e)) => {
                    log("error", &describe_error(&c, e));
                    false
                }
            });
            if !more && !self.rt.is_job_pending() {
                break;
            }
        }
        if killed > 0 {
            let deferred = if self.rt.is_job_pending() { ", remaining jobs deferred to the next turn" } else { "" };
            log("error", &format!("js: promise jobs stopped: exceeded the {} ms CPU-time callback budget ({killed} job killed{deferred})", perf::callback_budget().as_millis()));
        }
    }
}

// ---------------------------------------------------------------- modules

/// Resolves module specifiers against the page's import map (QY15).
struct PageResolver(Rc<js_modules::ModuleStore>);
/// Serves module sources from the pre-fetched map (QY15).
struct PageLoader(Rc<js_modules::ModuleStore>);

impl Resolver for PageResolver {
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str, _attrs: Option<rquickjs::loader::ImportAttributes<'js>>) -> JsResult<String> {
        js_modules::resolve_specifier(name, base, &self.0.map, &self.0.doc_base).map_err(|m| JsError::new_resolving_message(base, name, m))
    }
}

impl Loader for PageLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _attrs: Option<rquickjs::loader::ImportAttributes<'js>>) -> JsResult<Module<'js, rquickjs::module::Declared>> {
        let known = self.0.sources.borrow().get(name).cloned();
        let src = match known {
            Some(s) => s,
            None if crate::images::is_data_url(name) => crate::images::decode_data_uri_text(name).ok_or_else(|| JsError::new_loading_message(name, "malformed data: URL"))?,
            None => {
                return Err(JsError::new_loading_message(name, "module was not pre-fetched (the fetch failed, or it is a run-time import of an unlisted URL: QY15a)"));
            }
        };
        let m = Module::declare(ctx.clone(), name, src)?;
        // `import.meta.url`: an inline module's name carries a `#inline-module-N` tag.
        let url = match name.rsplit_once("#inline-module-") {
            Some((u, _)) => u,
            None => name,
        };
        m.meta()?.set("url", url)?;
        Ok(m)
    }
}

impl Engine {
    /// QV8-4: QuickJS has no startup snapshot.
    pub(crate) fn prebuilt(&self) -> bool {
        false
    }
}
