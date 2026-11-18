//! QV8-3a: the V8 backend of the JS engine seam (the `v8` crate, rusty_v8). Compiled
//! instead of `js_engine.rs` when the cargo feature `engine-v8` is on; it has the same
//! public surface (see the module docs of `js_engine.rs` for what each item is for), so
//! no other file knows which engine it talks to.
//!
//! How the V8 types are hidden behind the seam's QuickJS-shaped API:
//!
//! - [`Ctx`] is a copyable handle to the live `v8::PinScope` (a raw pointer; the scope is
//!   owned by `Engine::with` or by the V8 callback that is running). Every V8 value is a
//!   `Local<'static, _>`: the `'js` lifetime of the seam is a marker only, and a value must
//!   not outlive the `with`/native call that made it (the same rule as rquickjs).
//! - natives: [`register`] wraps a Rust closure in a `v8::Function` whose data is an
//!   `External` pointing at the closure. The closures live in a per-engine arena (a slot of
//!   the isolate), dropped with the isolate.
//! - a "pending exception" (what rquickjs keeps in the context) is a `Global` in the same
//!   slot: `Source::compile`/`Compiled::run`/`call_global` store the exception they caught
//!   there, `describe_exception` reads and clears it, a native that returns
//!   `Err(pending_exception())` rethrows it.
//! - the CPU budget (R39 C1) is a watchdog thread that interrupts the isolate every 2 ms
//!   while a callback is armed; the interrupt runs on the JS thread, reads the budget clock
//!   and terminates the script.
//!
//! QV8-3b: the budget, promise jobs, stack and heap limits (R39 C1-C4):
//! - promise jobs run in ONE `perform_microtask_checkpoint` (V8 cannot run a single job). A
//!   COUNTER-ONLY `PromiseHook` bumps `JOBS` at the start of every reaction job; the watchdog
//!   interrupt sees the counter move and re-arms the per-job deadline, so every job gets its own
//!   budget while a runaway job (no counter movement) is killed. The hook never reads the clock.
//!   A killed job or a tripped drain cap drops the rest of the queue (V8 clears it when the
//!   script is terminated): the log says "remaining jobs dropped". `PENDING` (any promise created
//!   or resolved since the last drain) stands in for `is_job_pending`, which V8 cannot answer.
//! - the stack limit is `sp - min(4 MB, half the thread stack)` (`js::js_stack_limit`).
//! - a near-heap-limit callback terminates the script (and gives V8 a little room to unwind)
//!   instead of letting V8 abort the process; the error reads `InternalError: out of memory`.
//! - `document.all` (QY40) is an undetectable object made from an `ObjectTemplate`.
//!
//! QV8-3c: modules. V8 has no import-map or loader of its own: the resolve callback and
//! `HostImportModuleDynamically` call scratchy's `js_modules::resolve_specifier` and read the
//! pre-fetched `ModuleStore` sources (the same seam the QuickJS loader uses). Compiled modules
//! are cached by URL in the engine state; `import.meta.url` is the URL minus any
//! `#inline-module-N` tag. A module entry runs through `import()` (js.rs `eval_module`), so the
//! dynamic-import callback is the one place that instantiates and evaluates.
use super::*;
use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::time::Duration;

thread_local! {
    /// Promise reaction jobs started on this thread (fed by the promise hook).
    static JOBS: Cell<u64> = const { Cell::new(0) };
    /// A promise was created or resolved since the last drain (a job may be queued).
    static PENDING: Cell<bool> = const { Cell::new(false) };
    /// The near-heap-limit callback fired and terminated the script.
    static OOM: Cell<bool> = const { Cell::new(false) };
    /// The page engine's watchdog state, for the synchronous budget polls (tests).
    static CUR_WATCH: std::cell::RefCell<Option<Arc<Watch>>> = const { std::cell::RefCell::new(None) };
}

type Sc = v8::PinScope<'static, 'static>;
type L<T> = v8::Local<'static, T>;

/// Erases the lifetime of a local (see the module docs).
fn st<'a, T>(l: v8::Local<'a, T>) -> L<T> {
    // SAFETY: lifetimes only; the value is used while its scope is alive.
    unsafe { std::mem::transmute::<v8::Local<'a, T>, L<T>>(l) }
}

/// The call context handed to every native and to `Engine::with`.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'js> {
    sc: NonNull<Sc>,
    _p: PhantomData<&'js ()>,
}

impl<'js> Ctx<'js> {
    fn new(scope: &mut v8::PinScope<'_, '_>) -> Ctx<'js> {
        let p = scope as *mut v8::PinScope<'_, '_> as *mut Sc;
        // SAFETY: a reference is never null.
        Ctx { sc: unsafe { NonNull::new_unchecked(p) }, _p: PhantomData }
    }

    #[allow(clippy::mut_from_ref)]
    fn sc(&self) -> &mut Sc {
        // SAFETY: the scope outlives every Ctx made from it (module docs); V8 code is
        // single-threaded here, and a nested use only runs while the outer one is waiting.
        unsafe { &mut *self.sc.as_ptr() }
    }

    fn global(&self) -> v8::Local<'static, v8::Object> {
        st(self.sc().get_current_context().global(self.sc()))
    }
}

#[derive(Debug)]
pub(crate) enum JsError {
    /// The JS exception is pending (see `describe_exception`).
    Exception,
    Msg(String),
}

impl std::fmt::Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsError::Exception => f.write_str("Exception generated by JS"),
            JsError::Msg(m) => f.write_str(m),
        }
    }
}

pub(crate) type JsResult<T> = Result<T, JsError>;

fn conv_err(from: &str, to: &str) -> JsError {
    JsError::Msg(format!("Error converting from js '{from}' into type '{to}'"))
}

fn js_type_name(v: L<v8::Value>) -> &'static str {
    if v.is_undefined() {
        "undefined"
    } else if v.is_null() {
        "null"
    } else if v.is_boolean() {
        "bool"
    } else if v.is_number() {
        "float"
    } else if v.is_string() {
        "string"
    } else if v.is_array() {
        "array"
    } else if v.is_function() {
        "function"
    } else {
        "object"
    }
}

// ---------------------------------------------------------------- conversions

/// A JS value read into a Rust type.
pub(crate) trait FromJs<'js>: Sized {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self>;
}

/// A Rust value turned into a JS value.
pub(crate) trait IntoJs<'js> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>>;
}

/// A type a native or an eval can return into Rust.
pub(crate) trait FromJsRet: for<'js> FromJs<'js> {}
impl<T: for<'js> FromJs<'js>> FromJsRet for T {}

fn num<'js>(v: L<v8::Value>, ctx: &Ctx<'js>, to: &str) -> JsResult<f64> {
    if !v.is_number() {
        return Err(conv_err(js_type_name(v), to));
    }
    v.number_value(ctx.sc()).ok_or(JsError::Exception)
}

macro_rules! int_conv {
    ($($t:ty),*) => {$(
        impl<'js> FromJs<'js> for $t {
            fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
                Ok(num(v, ctx, stringify!($t))? as $t)
            }
        }
        impl<'js> IntoJs<'js> for $t {
            fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
                Ok(st(v8::Number::new(ctx.sc(), self as f64).into()))
            }
        }
    )*};
}
int_conv!(i32, u32, u64, u8);

impl<'js> FromJs<'js> for f64 {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        num(v, ctx, "f64")
    }
}
impl<'js> IntoJs<'js> for f64 {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        Ok(st(v8::Number::new(ctx.sc(), self).into()))
    }
}
impl<'js> FromJs<'js> for bool {
    fn from_js(_ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        if v.is_boolean() { Ok(v.is_true()) } else { Err(conv_err(js_type_name(v), "bool")) }
    }
}
impl<'js> IntoJs<'js> for bool {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        Ok(st(v8::Boolean::new(ctx.sc(), self).into()))
    }
}
impl<'js> FromJs<'js> for String {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        if v.is_string() { Ok(v.to_rust_string_lossy(ctx.sc())) } else { Err(conv_err(js_type_name(v), "String")) }
    }
}
impl<'js> IntoJs<'js> for String {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        self.as_str().into_js(ctx)
    }
}
impl<'js> IntoJs<'js> for &str {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        v8::String::new(ctx.sc(), self).map(|s| st(s.into())).ok_or_else(|| JsError::Msg("string too long".into()))
    }
}
impl<'js> FromJs<'js> for () {
    fn from_js(_ctx: &Ctx<'js>, _v: L<v8::Value>) -> JsResult<Self> {
        Ok(())
    }
}
impl<'js> IntoJs<'js> for () {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        Ok(st(v8::undefined(ctx.sc()).into()))
    }
}
impl<'js, T: FromJs<'js>> FromJs<'js> for Option<T> {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        if v.is_null_or_undefined() { Ok(None) } else { T::from_js(ctx, v).map(Some) }
    }
}
impl<'js, T: IntoJs<'js>> IntoJs<'js> for Option<T> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        match self {
            Some(v) => v.into_js(ctx),
            None => ().into_js(ctx),
        }
    }
}
impl<'js, T: FromJs<'js>> FromJs<'js> for Vec<T> {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        let a = v8::Local::<v8::Array>::try_from(v).map_err(|_| conv_err(js_type_name(v), "Vec"))?;
        let mut out = Vec::with_capacity(a.length() as usize);
        for i in 0..a.length() {
            let e = a.get_index(ctx.sc(), i).ok_or(JsError::Exception)?;
            out.push(T::from_js(ctx, st(e))?);
        }
        Ok(out)
    }
}
impl<'js, T: IntoJs<'js>> IntoJs<'js> for Vec<T> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        let mut els = Vec::with_capacity(self.len());
        for e in self {
            els.push(e.into_js(ctx)?);
        }
        Ok(st(v8::Array::new_with_elements(ctx.sc(), &els).into()))
    }
}
impl<'js, T: IntoJs<'js>> IntoJs<'js> for JsResult<T> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        self?.into_js(ctx)
    }
}

/// A `Uint8Array` argument, copied out.
pub(crate) struct Bytes(pub(crate) Vec<u8>);

impl<'js> FromJs<'js> for Bytes {
    fn from_js(_ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        let ta = v8::Local::<v8::Uint8Array>::try_from(v).map_err(|_| conv_err(js_type_name(v), "TypedArray<u8>"))?;
        let mut buf = vec![0u8; ta.byte_length()];
        ta.copy_contents(&mut buf);
        Ok(Bytes(buf))
    }
}

/// A `Uint8Array` result built from a borrowed slice (one copy, into the engine).
pub(crate) struct BytesRef<'a>(pub(crate) &'a [u8]);

impl<'js> IntoJs<'js> for BytesRef<'_> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        let sc = ctx.sc();
        let ab = v8::ArrayBuffer::new(sc, self.0.len());
        if let Some(p) = ab.data() {
            // SAFETY: `p` points at `len` freshly allocated bytes.
            unsafe { std::ptr::copy_nonoverlapping(self.0.as_ptr(), p.as_ptr() as *mut u8, self.0.len()) };
        }
        let ta = v8::Uint8Array::new(sc, ab, 0, self.0.len()).ok_or(JsError::Exception)?;
        Ok(st(ta.into()))
    }
}

/// A JS object: reader (`get`) and builder (`new` + `set`).
pub(crate) struct JsObject<'js> {
    ctx: Ctx<'js>,
    o: L<v8::Object>,
}

impl<'js> JsObject<'js> {
    pub(crate) fn new(ctx: &Ctx<'js>) -> JsResult<Self> {
        Ok(JsObject { ctx: *ctx, o: st(v8::Object::new(ctx.sc())) })
    }

    pub(crate) fn set<V: IntoJs<'js>>(&self, key: &str, v: V) -> JsResult<()> {
        let k = key.into_js(&self.ctx)?;
        let v = v.into_js(&self.ctx)?;
        self.o.set(self.ctx.sc(), k, v).ok_or(JsError::Exception)?;
        Ok(())
    }

    /// The property converted to `T`; `None` when it is missing or not a `T`.
    pub(crate) fn get<T: FromJs<'js>>(&self, key: &str) -> Option<T> {
        let k = key.into_js(&self.ctx).ok()?;
        let v = self.o.get(self.ctx.sc(), k)?;
        T::from_js(&self.ctx, st(v)).ok()
    }
}

impl<'js> FromJs<'js> for JsObject<'js> {
    fn from_js(ctx: &Ctx<'js>, v: L<v8::Value>) -> JsResult<Self> {
        let o = v8::Local::<v8::Object>::try_from(v).map_err(|_| conv_err(js_type_name(v), "Object"))?;
        Ok(JsObject { ctx: *ctx, o: st(o) })
    }
}

impl<'js> IntoJs<'js> for JsObject<'js> {
    fn into_js(self, _ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        Ok(st(self.o.into()))
    }
}

// ---------------------------------------------------------------- per-engine state

/// What the isolate slot holds: the closure arena and the pending exception.
struct State {
    arena: Vec<Box<NativeFn>>,
    pending: Option<v8::Global<v8::Value>>,
    /// The script was stopped by `terminate_execution` (no JS exception exists then).
    terminated: bool,
    /// ... because the heap limit was hit (read by `describe_error`).
    oom: bool,
    /// QV8-3c: the page's module sources and import map (`None` on a bare engine).
    modules: Option<Rc<js_modules::ModuleStore>>,
    /// QV8-3c: compiled modules by URL, and URL by module identity hash.
    mod_cache: std::collections::HashMap<String, v8::Global<v8::Module>>,
    mod_names: std::collections::HashMap<i32, String>,
}

fn state<'a>(ctx: &Ctx<'_>) -> &'a mut State {
    let p = ctx.sc().get_slot_mut::<State>().expect("engine state") as *mut State;
    // SAFETY: the slot lives as long as the isolate; callers use it briefly.
    unsafe { &mut *p }
}

/// Stores the exception caught by `tc` as the pending one.
fn stash(ctx: &Ctx<'_>, tc: &mut v8::PinnedRef<'_, v8::TryCatch<v8::HandleScope>>) {
    let st_ = state(ctx);
    if tc.has_terminated() {
        st_.terminated = true;
        st_.oom = OOM.with(Cell::take);
        st_.pending = None;
        return;
    }
    st_.terminated = false;
    st_.pending = tc.exception().map(|e| v8::Global::new(ctx.sc(), e));
}

// ---------------------------------------------------------------- natives

/// A registered native: the call context, the JS arguments; the JS result.
type NativeFn = dyn Fn(&Ctx<'static>, &v8::FunctionCallbackArguments<'static>) -> JsResult<L<v8::Value>>;

/// One native parameter: a JS argument, or the `Ctx` itself (consumes no argument).
pub(crate) trait FromParam<'js>: Sized {
    fn from_param(ctx: &Ctx<'js>, args: &v8::FunctionCallbackArguments<'static>, i: &mut usize) -> JsResult<Self>;
}

impl<'js> FromParam<'js> for Ctx<'js> {
    fn from_param(ctx: &Ctx<'js>, _args: &v8::FunctionCallbackArguments<'static>, _i: &mut usize) -> JsResult<Self> {
        Ok(*ctx)
    }
}

impl<'js, T: FromJs<'js>> FromParam<'js> for T {
    fn from_param(ctx: &Ctx<'js>, args: &v8::FunctionCallbackArguments<'static>, i: &mut usize) -> JsResult<Self> {
        let v = st(args.get(*i as i32));
        *i += 1;
        T::from_js(ctx, v)
    }
}

/// A Rust closure or fn usable as a native.
pub(crate) trait IntoNative<'js, P> {
    fn into_native(self) -> Box<NativeFn>;
}

macro_rules! into_native {
    ($($t:ident),*) => {
        impl<'js, F, R, $($t),*> IntoNative<'js, ($($t,)*)> for F
        where
            F: Fn($($t),*) -> R + 'js,
            R: IntoJs<'js>,
            $($t: FromParam<'js>,)*
        {
            #[allow(non_snake_case, unused_variables, unused_mut, unused_assignments)]
            fn into_native(self) -> Box<NativeFn> {
                let f: Box<dyn Fn(&Ctx<'js>, &v8::FunctionCallbackArguments<'static>) -> JsResult<L<v8::Value>> + 'js> = Box::new(move |ctx, args| {
                    let mut i = 0usize;
                    $( let $t = <$t as FromParam<'js>>::from_param(ctx, args, &mut i)?; )*
                    self($($t),*).into_js(ctx)
                });
                // SAFETY: `'js` is a marker (module docs); the closure is kept in the engine's
                // arena, which is dropped with the isolate, before anything it captured.
                unsafe { std::mem::transmute::<Box<dyn Fn(&Ctx<'js>, &v8::FunctionCallbackArguments<'static>) -> JsResult<L<v8::Value>> + 'js>, Box<NativeFn>>(f) }
            }
        }
    };
}
into_native!();
into_native!(A);
into_native!(A, B);
into_native!(A, B, C);
into_native!(A, B, C, D);
into_native!(A, B, C, D, E);
into_native!(A, B, C, D, E, G);

fn throw_msg(scope: &mut v8::PinScope<'_, '_>, m: &str, type_error: bool) {
    let Some(s) = v8::String::new(scope, m) else { return };
    let e = if type_error { v8::Exception::type_error(scope, s) } else { v8::Exception::error(scope, s) };
    scope.throw_exception(e);
}

fn trampoline<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, mut rv: v8::ReturnValue<'s, v8::Value>) {
    let Ok(ext) = v8::Local::<v8::External>::try_from(args.data()) else { return };
    // SAFETY: `data` is the External made by `register`, pointing into the arena's heap box.
    let f = unsafe { &*(ext.value() as *const Box<NativeFn>) };
    let ctx = Ctx::<'static>::new(scope);
    // SAFETY: lifetimes only.
    let args_s = unsafe { std::mem::transmute::<&v8::FunctionCallbackArguments<'s>, &v8::FunctionCallbackArguments<'static>>(&args) };
    match f(&ctx, args_s) {
        Ok(v) => rv.set(v),
        Err(JsError::Exception) => {
            let st_ = state(&ctx);
            if let Some(g) = st_.pending.take() {
                let e = v8::Local::new(scope, &g);
                scope.throw_exception(e);
            }
            // terminated: no exception to throw, the termination keeps unwinding.
        }
        Err(JsError::Msg(m)) => throw_msg(scope, &m, true),
    }
}

/// Installs `f` as the global function `name`.
pub(crate) fn register<'js, P, F>(ctx: &Ctx<'js>, name: &str, f: F)
where
    F: IntoNative<'js, P> + 'js,
{
    let boxed: Box<Box<NativeFn>> = Box::new(f.into_native());
    let ptr = &*boxed as *const Box<NativeFn> as *mut c_void;
    state(ctx).arena.push(boxed);
    // The arena `Vec` may grow, but each closure sits in its own heap box: `ptr` stays valid.
    let sc = ctx.sc();
    let ext = v8::External::new(sc, ptr);
    let Some(func) = v8::Function::builder(trampoline).data(ext.into()).build(sc) else { return };
    if let Some(k) = name.into_js(ctx).ok() {
        ctx.global().set(sc, k, func.into());
    }
}

/// `native!(ctx, "__name", |a: i32, b: String| -> String { .. })`.
macro_rules! native {
    ($ctx:expr, $name:expr, $f:expr) => {
        $crate::js_engine::register($ctx, $name, $f)
    };
}
pub(crate) use native;

// ---------------------------------------------------------------- calling JS

/// Arguments of a call: a tuple of `IntoJs` values.
pub(crate) trait IntoArgs<'js> {
    fn into_args(self, ctx: &Ctx<'js>) -> JsResult<Vec<L<v8::Value>>>;
}

macro_rules! into_args {
    ($($t:ident),*) => {
        impl<'js, $($t: IntoJs<'js>),*> IntoArgs<'js> for ($($t,)*) {
            #[allow(non_snake_case, unused_variables)]
            fn into_args(self, ctx: &Ctx<'js>) -> JsResult<Vec<L<v8::Value>>> {
                let ($($t,)*) = self;
                Ok(vec![$($t.into_js(ctx)?),*])
            }
        }
    };
}
into_args!();
into_args!(A);
into_args!(A, B);
into_args!(A, B, C);
into_args!(A, B, C, D);
into_args!(A, B, C, D, E);
into_args!(A, B, C, D, E, G);

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
    let key = name.into_js(ctx).ok()?;
    let f = ctx.global().get(ctx.sc(), key)?;
    let f = v8::Local::<v8::Function>::try_from(f).ok()?;
    let args = match make().and_then(|a| a.into_args(ctx)) {
        Ok(a) => a,
        Err(e) => return Some(Err(e)),
    };
    let sc = ctx.sc();
    v8::tc_scope!(let tc, sc);
    let recv: v8::Local<v8::Value> = ctx.global().into();
    match f.call(tc, recv, &args) {
        Some(v) => Some(R::from_js(ctx, st(v))),
        None => {
            stash(ctx, tc);
            Some(Err(JsError::Exception))
        }
    }
}

/// The error for "that global is not a function" (a strict `call_global`).
pub(crate) fn not_a_function() -> JsError {
    conv_err("undefined", "function")
}

/// The error that means "the JS exception is pending" (rethrown to the caller).
pub(crate) fn pending_exception() -> JsError {
    JsError::Exception
}

pub(crate) fn set_global<'js, V: IntoJs<'js>>(ctx: &Ctx<'js>, name: &str, v: V) -> JsResult<()> {
    let k = name.into_js(ctx)?;
    let v = v.into_js(ctx)?;
    ctx.global().set(ctx.sc(), k, v).ok_or(JsError::Exception)?;
    Ok(())
}

pub(crate) fn remove_global(ctx: &Ctx<'_>, name: &str) {
    if let Ok(k) = name.into_js(ctx) {
        let k: v8::Local<v8::Name> = k.try_into().unwrap();
        ctx.global().delete(ctx.sc(), k.into());
    }
}

/// Global `name` as an object reader, if it is one.
pub(crate) fn global_object<'js>(ctx: &Ctx<'js>, name: &str) -> Option<JsObject<'js>> {
    let k = name.into_js(ctx).ok()?;
    let v = ctx.global().get(ctx.sc(), k)?;
    JsObject::from_js(ctx, st(v)).ok()
}

/// Runs `code` as a script for its side effects (no result, errors ignored).
pub(crate) fn eval_quiet(ctx: &Ctx<'_>, code: &str) {
    if let Some(src) = Source::new(code, "eval") {
        if let Ok(c) = src.compile(ctx) {
            let _ = c.run(ctx);
        }
        discard_exception(ctx);
    }
}

unsafe extern "C" {
    /// `v8::ObjectTemplate::MarkAsUndetectable()`: in V8's public API (and so in the prebuilt
    /// library) but not bound by the `v8` crate. It takes only `this`, so the Itanium-mangled
    /// name carries no libc++ types.
    #[link_name = "_ZN2v814ObjectTemplate18MarkAsUndetectableEv"]
    fn v8_object_template_mark_as_undetectable(this: *const v8::ObjectTemplate);
}

/// The stand-in's handlers forward to the JS Proxy kept as the handler data.
fn all_named_get<'s>(scope: &mut v8::PinScope<'s, '_>, key: v8::Local<'s, v8::Name>, args: v8::PropertyCallbackArguments<'s>, mut rv: v8::ReturnValue<v8::Value>) -> v8::Intercepted {
    let Ok(proxy) = v8::Local::<v8::Object>::try_from(args.data()) else { return v8::Intercepted::kNo };
    match proxy.get(scope, key.into()) {
        Some(v) => {
            rv.set(v);
            v8::Intercepted::kYes
        }
        None => v8::Intercepted::kNo,
    }
}

fn all_indexed_get<'s>(scope: &mut v8::PinScope<'s, '_>, index: u32, args: v8::PropertyCallbackArguments<'s>, mut rv: v8::ReturnValue<v8::Value>) -> v8::Intercepted {
    let Ok(proxy) = v8::Local::<v8::Object>::try_from(args.data()) else { return v8::Intercepted::kNo };
    match proxy.get_index(scope, index) {
        Some(v) => {
            rv.set(v);
            v8::Intercepted::kYes
        }
        None => v8::Intercepted::kNo,
    }
}

fn all_call<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, mut rv: v8::ReturnValue<'s, v8::Value>) {
    let Ok(f) = v8::Local::<v8::Function>::try_from(args.data()) else { return };
    let argv: Vec<v8::Local<v8::Value>> = (0..args.length()).map(|i| args.get(i)).collect();
    if let Some(v) = f.call(scope, args.this().into(), &argv) {
        rv.set(v);
    }
}

/// QY40: gives `document.all` the [[IsHTMLDDA]] slot (falsy, `typeof` 'undefined', `== null`).
/// V8 can only mark an object undetectable through an `ObjectTemplate`, so the page's
/// `document.all` (a callable JS Proxy, kept in the global `name` until now) is wrapped in an
/// undetectable instance whose named/indexed/call handlers forward to it, and `document.all`
/// is redefined to return the wrapper.
pub(crate) fn mark_htmldda(ctx: &Ctx<'_>, name: &str) {
    let sc = ctx.sc();
    let Ok(k) = name.into_js(ctx) else { return };
    let Some(proxy) = ctx.global().get(sc, k) else { return };
    if !proxy.is_function() {
        return;
    }
    let tmpl = v8::ObjectTemplate::new(sc);
    // SAFETY: `tmpl` is a live template; the call only sets a flag on it (before any instance).
    unsafe { v8_object_template_mark_as_undetectable(&*tmpl) };
    tmpl.set_named_property_handler(v8::NamedPropertyHandlerConfiguration::new().getter(all_named_get).data(proxy));
    tmpl.set_indexed_property_handler(v8::IndexedPropertyHandlerConfiguration::new().getter(all_indexed_get).data(proxy));
    tmpl.set_call_as_function_handler(all_call, Some(proxy));
    let Some(inst) = tmpl.new_instance(sc) else { return };
    if set_global(ctx, "__d_all_w", RawValue(st(inst.into()))).is_ok() {
        eval_quiet(
            ctx,
            "(function () { var w = globalThis.__d_all_w; delete globalThis.__d_all_w; \
             Object.defineProperty(document, 'all', { get: function () { return w; }, configurable: true, enumerable: true }); })();",
        );
    }
}

// ---------------------------------------------------------------- errors

/// A pending exception as the one log text: `Name: message` plus the stack, or
/// `Uncaught <value>` for a thrown non-error. Clears the exception.
pub(crate) fn describe_exception(ctx: &Ctx<'_>) -> String {
    describe_error(ctx, JsError::Exception)
}

/// The frames of a V8 `stack` (`Name: message\n    at ...`) without its header, so the
/// text matches QuickJS (`    at f (file:1:2)` lines only).
fn stack_frames(stack: &str) -> &str {
    match stack.find("\n    at ") {
        Some(i) => &stack[i + 1..],
        None => "",
    }
}

pub(crate) fn describe_error(ctx: &Ctx<'_>, err: JsError) -> String {
    if !matches!(err, JsError::Exception) {
        return err.to_string();
    }
    let st_ = state(ctx);
    let was_terminated = std::mem::take(&mut st_.terminated);
    let was_oom = std::mem::take(&mut st_.oom);
    let Some(g) = st_.pending.take() else {
        return if was_oom {
            "InternalError: out of memory".into()
        } else if was_terminated {
            "InternalError: interrupted".into()
        } else {
            "Uncaught undefined".into()
        };
    };
    let sc = ctx.sc();
    let value = v8::Local::new(sc, &g);
    if value.is_native_error() {
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(value) {
            let get = |k: &str| -> String {
                let Ok(key) = k.into_js(ctx) else { return String::new() };
                match obj.get(ctx.sc(), key.into()) {
                    Some(v) if v.is_string() => v.to_rust_string_lossy(ctx.sc()),
                    _ => String::new(),
                }
            };
            let (name, message, stack) = (get("name"), get("message"), get("stack"));
            let msg = if name.is_empty() { message } else { format!("{name}: {message}") };
            let frames = stack_frames(&stack);
            return if frames.is_empty() { msg } else { format!("{msg}\n{frames}") };
        }
    }
    let text = value.to_rust_string_lossy(sc);
    format!("Uncaught {text}")
}

/// Drops the pending exception without reading it.
pub(crate) fn discard_exception(ctx: &Ctx<'_>) {
    let s = state(ctx);
    s.pending = None;
    // A swallowed termination must end here: V8 keeps terminating every script on the stack
    // (and the next ones in this `with`) until `cancel_terminate_execution`.
    if std::mem::take(&mut s.terminated) {
        ctx.sc().cancel_terminate_execution();
    }
    s.oom = false;
    OOM.with(|o| o.set(false));
}

/// The engine's name and version for the perf report (a V8 backend fills the same pair).
pub(crate) fn engine_info() -> (&'static str, String) {
    ("V8", format!("{} (v8 crate 152.2.0)", v8::V8::get_version()))
}

// ---------------------------------------------------------------- eval

/// Global-script source (NUL-free), compiled in a separate step from running so the
/// caller can arm the budget in between.
pub(crate) struct Source {
    code: String,
    file: String,
}

/// A compiled global script, consumed by `run`.
pub(crate) struct Compiled(L<v8::Script>);

fn compile_script(ctx: &Ctx<'_>, code: &str, file: &str) -> Result<L<v8::Script>, ()> {
    let sc = ctx.sc();
    v8::tc_scope!(let tc, sc);
    let (Some(src), Some(name)) = (v8::String::new(tc, code), v8::String::new(tc, file)) else {
        let s = state(ctx);
        s.pending = None;
        s.terminated = false;
        return Err(());
    };
    let origin = v8::ScriptOrigin::new(tc, name.into(), 0, 0, false, 0, None, false, false, false, None);
    match v8::Script::compile(tc, src, Some(&origin)) {
        Some(s) => Ok(st(s)),
        None => {
            stash(ctx, tc);
            Err(())
        }
    }
}

fn run_script(ctx: &Ctx<'_>, script: L<v8::Script>) -> Result<L<v8::Value>, ()> {
    let sc = ctx.sc();
    v8::tc_scope!(let tc, sc);
    match script.run(tc) {
        Some(v) => Ok(st(v)),
        None => {
            stash(ctx, tc);
            Err(())
        }
    }
}

impl Source {
    /// `None` when `code` or `name` contains a NUL byte.
    pub(crate) fn new(code: &str, name: &str) -> Option<Source> {
        if code.contains('\0') || name.contains('\0') {
            return None;
        }
        Some(Source { code: code.to_string(), file: name.to_string() })
    }

    /// Compiles as sloppy global code (HTML classic scripts are sloppy, R5).
    /// `Err(())` leaves the exception pending (`describe_exception`).
    pub(crate) fn compile(&self, ctx: &Ctx<'_>) -> Result<Compiled, ()> {
        compile_script(ctx, &self.code, &self.file).map(Compiled)
    }
}

impl Compiled {
    /// Runs the script. `Err(())` leaves the exception pending.
    pub(crate) fn run(self, ctx: &Ctx<'_>) -> Result<(), ()> {
        run_script(ctx, self.0).map(|_| ())
    }
}

/// Evaluates `code` as sloppy global code under the file name `label` and converts the result.
pub(crate) fn eval_sloppy<'js, T: FromJs<'js>>(ctx: &Ctx<'js>, label: &str, code: &str) -> JsResult<T> {
    let src = Source::new(code, label).ok_or_else(|| JsError::Msg("source contains a NUL byte".into()))?;
    let c = src.compile(ctx).map_err(|_| JsError::Exception)?;
    let v = run_script(ctx, c.0).map_err(|_| JsError::Exception)?;
    T::from_js(ctx, v)
}

// ---------------------------------------------------------------- budget

/// The CPU-time interrupt budget (QY5): a deadline on the script clock and the
/// flag the interrupt sets when it stops a script. Cheap to clone; natives that
/// arm it (the nested run) hold a clone.
#[derive(Clone)]
pub(crate) struct Budget {
    deadline: Arc<AtomicU64>,
    tripped: Arc<AtomicBool>,
    /// A callback is armed and JS may be running: the watchdog interrupts only then.
    active: Arc<AtomicBool>,
    /// A promise drain is running: a moving `JOBS` counter re-arms `deadline` (one budget per job).
    per_job: Arc<AtomicBool>,
    /// The drain cap on the script clock (0 = no drain running).
    drain_deadline: Arc<AtomicU64>,
    /// The drain cap (not a job's own budget) stopped the script.
    cap_hit: Arc<AtomicBool>,
    /// `JOBS` as the last check saw it.
    last_jobs: Arc<AtomicU64>,
}

impl Budget {
    fn new(initial: Duration) -> Budget {
        Budget {
            deadline: Arc::new(AtomicU64::new(script_clock_nanos() + initial.as_nanos() as u64)),
            tripped: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicBool::new(false)),
            per_job: Arc::new(AtomicBool::new(false)),
            drain_deadline: Arc::new(AtomicU64::new(0)),
            cap_hit: Arc::new(AtomicBool::new(false)),
            last_jobs: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Clears the tripped flag and starts a fresh `budget`.
    pub(crate) fn arm(&self, budget: Duration) {
        self.tripped.store(false, Ordering::Relaxed);
        self.set_deadline(budget);
        self.active.store(true, Ordering::Release);
    }

    /// An explicit arm wins over the watchdog's per-job re-arm: the job that moved the counter
    /// before this call is already covered, and a drain's cap must not cut a budget the caller
    /// asked for (a page-load script delivered from inside a job keeps its own, longer budget,
    /// as on QuickJS).
    fn set_deadline(&self, budget: Duration) {
        let d = script_clock_nanos() + budget.as_nanos() as u64;
        self.deadline.store(d, Ordering::Relaxed);
        self.last_jobs.store(JOBS.with(Cell::get), Ordering::Relaxed);
        if self.drain_deadline.load(Ordering::Relaxed) != 0 {
            self.drain_deadline.fetch_max(d, Ordering::Relaxed);
        }
    }

    /// Starts a fresh `budget` but keeps the tripped flag (a drain's per-job deadline).
    pub(crate) fn rearm_keep_tripped(&self, budget: Duration) {
        self.set_deadline(budget);
        self.active.store(true, Ordering::Release);
    }

    /// True when the interrupt stopped a script since the last `arm`.
    pub(crate) fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }
}

/// What the watchdog thread and the interrupt callback share.
struct Watch {
    budget: Budget,
    stop: AtomicBool,
    handle: v8::IsolateHandle,
}

const TICK: Duration = Duration::from_millis(2);

/// The budget check (JS thread only: the clock and the job counter are thread-local). Runs from
/// the watchdog's interrupt every tick, and synchronously from the test seams.
fn poll_budget(w: &Watch) {
    let b = &w.budget;
    if !b.active.load(Ordering::Acquire) {
        return;
    }
    let now = script_clock_nanos();
    if b.per_job.load(Ordering::Relaxed) {
        // A job started since the last check: it gets a fresh budget (QY46). Checked BEFORE the
        // deadline so a job that starts just as the old one expires is not blamed for it.
        let jobs = JOBS.with(Cell::get);
        if jobs != b.last_jobs.load(Ordering::Relaxed) {
            b.last_jobs.store(jobs, Ordering::Relaxed);
            b.deadline.store(now + perf::callback_budget().as_nanos() as u64, Ordering::Relaxed);
        }
    }
    let cap = b.drain_deadline.load(Ordering::Relaxed);
    // The job's own budget is checked first: a job that overruns it is "killed", and only a
    // drain of many well-behaved jobs is "capped".
    if now > b.deadline.load(Ordering::Relaxed) {
        b.tripped.store(true, Ordering::Relaxed);
        w.handle.terminate_execution();
    } else if cap != 0 && now > cap {
        b.cap_hit.store(true, Ordering::Relaxed);
        w.handle.terminate_execution();
    }
}

unsafe extern "C" fn on_interrupt(_iso: v8::UnsafeRawIsolatePtr, data: *mut c_void) {
    // SAFETY: `data` is the `Watch` owned by the Engine, which joins the watchdog thread
    // (and so outlives every interrupt it posted) before the isolate goes away.
    let w = unsafe { &*(data as *const Watch) };
    poll_budget(w);
}

/// QV8-3b test seam: the budget check, right now, on this thread's page engine (so a fake-clock
/// test is exact instead of waiting for the watchdog's 2 ms tick).
#[cfg(test)]
pub(crate) fn poll_budget_now() {
    CUR_WATCH.with(|w| {
        if let Some(w) = &*w.borrow() {
            poll_budget(w);
        }
    });
}

/// The counter-only promise hook (R39 C2): never reads the clock.
unsafe extern "C" fn promise_hook(ty: v8::PromiseHookType, _promise: v8::Local<v8::Promise>, _parent: v8::Local<v8::Value>) {
    match ty {
        v8::PromiseHookType::Before => {
            JOBS.with(|j| j.set(j.get().wrapping_add(1)));
            // Fake-clock tests: the job's budget must be re-armed before its body spends fake CPU.
            #[cfg(test)]
            if crate::js::FAKE_CLOCK.with(|f| f.get().is_some()) {
                poll_budget_now();
            }
        }
        v8::PromiseHookType::Init | v8::PromiseHookType::Resolve => PENDING.with(|p| p.set(true)),
        v8::PromiseHookType::After => {}
    }
}

/// C4: the heap is nearly full. Stop the script (an uncatchable termination, like the CPU budget)
/// and give V8 a little more room to unwind, instead of letting it abort the whole process.
unsafe extern "C" fn near_heap_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    // SAFETY: `data` is the Engine's boxed handle, alive as long as the isolate.
    let h = unsafe { &*(data as *const v8::IsolateHandle) };
    OOM.with(|o| o.set(true));
    h.terminate_execution();
    current + (32 << 20)
}

fn start_watchdog(w: Arc<Watch>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("js-watchdog".into())
        .spawn(move || {
            while !w.stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                if w.budget.active.load(Ordering::Acquire) {
                    w.handle.request_interrupt(on_interrupt, Arc::as_ptr(&w) as *mut c_void);
                }
            }
        })
        .expect("watchdog thread")
}

// ---------------------------------------------------------------- modules

/// QuickJS reports a module resolve/load failure as a `ReferenceError`; so does this.
fn throw_module_error(scope: &mut v8::PinScope<'_, '_>, m: &str) {
    let Some(s) = v8::String::new(scope, m) else { return };
    let e = v8::Exception::reference_error(scope, s);
    scope.throw_exception(e);
}

/// The compiled module for `url` (cached), loading its source like the QuickJS `PageLoader`.
/// A failure leaves an exception thrown in `scope` and returns `None`.
fn load_module<'s>(scope: &mut v8::PinScope<'s, '_>, url: &str) -> Option<v8::Local<'s, v8::Module>> {
    let ctx = Ctx::<'static>::new(scope);
    let stt = state(&ctx);
    if let Some(g) = stt.mod_cache.get(url) {
        return Some(v8::Local::new(scope, g));
    }
    let store = stt.modules.clone()?;
    let known = store.sources.borrow().get(url).cloned();
    let src = match known {
        Some(s) => s,
        None if crate::images::is_data_url(url) => match crate::images::decode_data_uri_text(url) {
            Some(s) => s,
            None => {
                throw_module_error(scope, &format!("Error loading module '{url}': malformed data: URL"));
                return None;
            }
        },
        None => {
            throw_module_error(scope, &format!("Error loading module '{url}': module was not pre-fetched (the fetch failed, or it is a run-time import of an unlisted URL: QY15a)"));
            return None;
        }
    };
    let code = v8::String::new(scope, &src)?;
    let name = v8::String::new(scope, url)?;
    let origin = v8::ScriptOrigin::new(scope, name.into(), 0, 0, false, 0, None, false, false, true, None);
    let mut source = v8::script_compiler::Source::new(code, Some(&origin));
    let m = v8::script_compiler::compile_module(scope, &mut source)?;
    let g = v8::Global::new(scope, m);
    let stt = state(&ctx);
    stt.mod_cache.insert(url.to_string(), g);
    stt.mod_names.insert(m.get_identity_hash().get(), url.to_string());
    Some(m)
}

/// Resolves `spec` against `base` with scratchy's resolver; a failure throws and returns `None`.
fn resolve_to_url(scope: &mut v8::PinScope<'_, '_>, base: &str, spec: &str) -> Option<String> {
    let ctx = Ctx::<'static>::new(scope);
    let store = state(&ctx).modules.clone()?;
    match js_modules::resolve_specifier(spec, base, &store.map, &store.doc_base) {
        Ok(u) => Some(u),
        Err(m) => {
            throw_module_error(scope, &format!("Error resolving module '{spec}' from '{base}': {m}"));
            None
        }
    }
}

fn resolve_cb<'s>(context: v8::Local<'s, v8::Context>, spec: v8::Local<'s, v8::String>, _attrs: v8::Local<'s, v8::FixedArray>, referrer: v8::Local<'s, v8::Module>) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let ctx = Ctx::<'static>::new(scope);
    let base = state(&ctx).mod_names.get(&referrer.get_identity_hash().get()).cloned().unwrap_or_default();
    let spec = spec.to_rust_string_lossy(scope);
    let url = resolve_to_url(scope, &base, &spec)?;
    load_module(scope, &url)
}

/// The promise reaction that turns the evaluation promise into the namespace (its data).
fn ns_callback<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, mut rv: v8::ReturnValue<'s, v8::Value>) {
    rv.set(args.data());
    let _ = scope;
}

/// `import(spec)`: load, instantiate and evaluate the module, resolve with its namespace (after
/// top-level await). Every failure is a rejection, except termination, which propagates.
fn dyn_import_cb<'s>(scope: &mut v8::PinScope<'s, '_>, _opts: v8::Local<'s, v8::Data>, resource: v8::Local<'s, v8::Value>, spec: v8::Local<'s, v8::String>, _attrs: v8::Local<'s, v8::FixedArray>) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let ctx = Ctx::<'static>::new(scope);
    let store = state(&ctx).modules.clone();
    let base = if resource.is_string() { resource.to_rust_string_lossy(scope) } else { store.as_ref().map(|s| s.doc_base.to_string()).unwrap_or_default() };
    let spec = spec.to_rust_string_lossy(scope);
    let mut terminated = false;
    let mut failure: Option<v8::Local<'s, v8::Value>> = None;
    let mut ns_promise: Option<v8::Local<'s, v8::Promise>> = None;
    {
        v8::tc_scope!(let tc, scope);
        let step = (|| {
            let url = resolve_to_url(tc, &base, &spec)?;
            let m = load_module(tc, &url)?;
            if m.get_status() == v8::ModuleStatus::Uninstantiated {
                m.instantiate_module(tc, resolve_cb)?;
            }
            let ev = m.evaluate(tc)?;
            let ev = v8::Local::<v8::Promise>::try_from(ev).ok()?;
            let ns = m.get_module_namespace();
            let f = v8::Function::builder(ns_callback).data(ns).build(tc)?;
            ev.then(tc, f)
        })();
        match step {
            Some(p) => ns_promise = Some(p),
            None if tc.has_terminated() => terminated = true,
            None => failure = tc.exception(),
        }
    }
    if terminated {
        return None;
    }
    if let Some(p) = ns_promise {
        // Forward the chained promise's outcome (a fulfilled namespace or the TLA/evaluation error).
        resolver.resolve(scope, p.into());
    } else {
        let e = failure.unwrap_or_else(|| v8::undefined(scope).into());
        resolver.reject(scope, e);
    }
    Some(promise)
}

extern "C" fn import_meta_cb(context: v8::Local<v8::Context>, module: v8::Local<v8::Module>, meta: v8::Local<v8::Object>) {
    v8::callback_scope!(unsafe scope, context);
    let ctx = Ctx::<'static>::new(scope);
    let name = state(&ctx).mod_names.get(&module.get_identity_hash().get()).cloned().unwrap_or_default();
    // An inline module's name carries a `#inline-module-N` tag.
    let url = name.rsplit_once("#inline-module-").map_or(name.as_str(), |(u, _)| u);
    if let (Some(k), Some(v)) = (v8::String::new(scope, "url"), v8::String::new(scope, url)) {
        meta.create_data_property(scope, k.into(), v.into());
    }
}

// ---------------------------------------------------------------- engine

/// What `Engine::new` wires up.
pub(crate) struct EngineOpts {
    pub(crate) memory_limit: usize,
    pub(crate) stack_limit: usize,
    /// The page configuration: rejection hook and CPU interrupt (budget starts at
    /// `SCRIPT_BUDGET`). `None` is a bare engine. (V8: the module store is unused until 3c.)
    pub(crate) page: Option<Rc<js_modules::ModuleStore>>,
}

/// `atob`/`btoa` (WHATWG forgiving-base64), which QuickJS has built in and V8 does not.
const BASE64_JS: &str = r#"(function (g) {
  var A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  function bad(m) {
    var e;
    if (typeof g.DOMException === 'function') { try { e = new g.DOMException(m, 'InvalidCharacterError'); } catch (x) {} }
    if (!e) { e = new Error(m); e.name = 'InvalidCharacterError'; }
    return e;
  }
  function def(n, f) { Object.defineProperty(g, n, { value: f, writable: true, configurable: true, enumerable: true }); }
  def('btoa', function btoa(s) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'btoa' on 'Window': 1 argument required, but only 0 present.");
    s = String(s);
    var o = '', i, a, b, c;
    for (i = 0; i < s.length; i++) if (s.charCodeAt(i) > 255) throw bad('Invalid character');
    for (i = 0; i < s.length; i += 3) {
      a = s.charCodeAt(i); b = s.charCodeAt(i + 1); c = s.charCodeAt(i + 2);
      o += A[a >> 2] + A[((a & 3) << 4) | ((b || 0) >> 4)] +
        (i + 1 < s.length ? A[((b & 15) << 2) | ((c || 0) >> 6)] : '=') + (i + 2 < s.length ? A[c & 63] : '=');
    }
    return o;
  });
  def('atob', function atob(s) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'atob' on 'Window': 1 argument required, but only 0 present.");
    s = String(s).replace(/[\t\n\f\r ]/g, '');
    if (s.length % 4 === 0) s = s.replace(/==?$/, '');
    if (s.length % 4 === 1 || /[^A-Za-z0-9+\/]/.test(s)) throw bad('The string to be decoded is not correctly encoded.');
    var o = '', buf = 0, bits = 0;
    for (var i = 0; i < s.length; i++) {
      buf = (buf << 6) | A.indexOf(s[i]); bits += 6;
      if (bits >= 8) { bits -= 8; o += String.fromCharCode((buf >> bits) & 255); }
    }
    return o;
  });
})(globalThis);"#;

static INIT: Once = Once::new();

/// Starts V8 once per process: the default platform (worker threads for concurrent
/// compile/GC), initialize. `SCRATCHY_V8_FLAGS` is passed to V8 first (e.g. `--jitless`).
fn init_v8() {
    INIT.call_once(|| {
        if let Ok(f) = std::env::var("SCRATCHY_V8_FLAGS") {
            if !f.is_empty() {
                v8::V8::set_flags_from_string(&f);
            }
        }
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

/// QY14: forwards every rejection event to the prelude's queue (`__scratchy_rej_push`).
unsafe extern "C" fn reject_callback(msg: v8::PromiseRejectMessage) {
    use v8::PromiseRejectEvent::*;
    let handled = match msg.get_event() {
        PromiseRejectWithNoHandler => false,
        PromiseHandlerAddedAfterReject => true,
        _ => return,
    };
    // SAFETY: called by V8 on the JS thread inside a script or microtask.
    v8::callback_scope!(unsafe scope, &msg);
    let ctx = Ctx::<'static>::new(scope);
    let promise: v8::Local<v8::Value> = msg.get_promise().into();
    let reason = msg.get_value().unwrap_or_else(|| v8::undefined(scope).into());
    let r = call_global::<_, ()>(&ctx, "__scratchy_rej_push", (RawValue(st(promise)), RawValue(st(reason)), handled));
    if r.is_some_and(|r| r.is_err()) {
        discard_exception(&ctx);
    }
}

/// An already-made JS value as a call argument.
struct RawValue(L<v8::Value>);
impl<'js> IntoJs<'js> for RawValue {
    fn into_js(self, _ctx: &Ctx<'js>) -> JsResult<L<v8::Value>> {
        Ok(self.0)
    }
}

/// A live V8 isolate and its one context. Field order matters: the context drops
/// before the isolate (which owns the closure arena), and the watchdog stops first.
pub(crate) struct Engine {
    watchdog: Option<(Arc<Watch>, std::thread::JoinHandle<()>)>,
    context: v8::Global<v8::Context>,
    isolate: UnsafeCell<v8::OwnedIsolate>,
    /// What `near_heap_limit` points at (dropped after the isolate).
    _oom_handle: Box<v8::IsolateHandle>,
    pub(crate) budget: Budget,
    /// Nesting depth of `with` (the outermost one ends a termination).
    depth: Cell<u32>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some((w, h)) = self.watchdog.take() {
            w.stop.store(true, Ordering::Relaxed);
            let _ = h.join();
            CUR_WATCH.with(|c| *c.borrow_mut() = None);
        }
    }
}

impl Engine {
    /// `Err` is the log line for the failure.
    pub(crate) fn new(opts: EngineOpts) -> Result<Engine, &'static str> {
        init_v8();
        // QY47a for V8: the JS stack may use `stack_limit` bytes below the current position.
        let probe = 0u8;
        let sp = &probe as *const u8 as usize;
        let limit = sp.saturating_sub(opts.stack_limit.max(64 * 1024));
        let params = v8::CreateParams::default().heap_limits(0, opts.memory_limit);
        // SAFETY: the address is inside this thread's stack and the isolate is only used here.
        let params = unsafe { params.set_stack_limit(limit as *mut u32) };
        let mut isolate = v8::Isolate::new(params);
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        isolate.set_host_import_module_dynamically_callback(dyn_import_cb);
        isolate.set_host_initialize_import_meta_object_callback(import_meta_cb);
        isolate.set_slot(State { arena: Vec::new(), pending: None, terminated: false, oom: false, modules: opts.page.clone(), mod_cache: Default::default(), mod_names: Default::default() });
        JOBS.with(|j| j.set(0));
        PENDING.with(|p| p.set(false));
        OOM.with(|o| o.set(false));
        isolate.set_promise_hook(promise_hook);
        let oom_handle = Box::new(isolate.thread_safe_handle());
        isolate.add_near_heap_limit_callback(near_heap_limit, &*oom_handle as *const v8::IsolateHandle as *mut c_void);
        let budget = Budget::new(SCRIPT_BUDGET);
        let mut watchdog = None;
        if opts.page.is_some() {
            isolate.set_promise_reject_callback(reject_callback);
            let w = Arc::new(Watch { budget: budget.clone(), stop: AtomicBool::new(false), handle: isolate.thread_safe_handle() });
            let h = start_watchdog(w.clone());
            CUR_WATCH.with(|c| *c.borrow_mut() = Some(w.clone()));
            watchdog = Some((w, h));
        }
        let context = {
            v8::scope!(let hs, &mut isolate);
            let c = v8::Context::new(hs, Default::default());
            v8::Global::new(hs, c)
        };
        let eng = Engine { watchdog, context, isolate: UnsafeCell::new(isolate), _oom_handle: oom_handle, budget, depth: Cell::new(0) };
        // QuickJS ships atob/btoa and no `Intl`; V8 is the other way round. Match QuickJS:
        // the page's own en-US `Intl` shim (intl_prelude.js) only installs when `Intl` is absent.
        eng.with(|ctx| {
            eval_quiet(&ctx, "delete globalThis.Intl; delete globalThis.queueMicrotask;");
            eval_quiet(&ctx, BASE64_JS);
        });
        Ok(eng)
    }

    #[allow(clippy::mut_from_ref)]
    fn iso(&self) -> &mut v8::OwnedIsolate {
        // SAFETY: single-threaded; `with` and the job drains never overlap a live borrow.
        unsafe { &mut *self.isolate.get() }
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(Ctx<'_>) -> R) -> R {
        self.depth.set(self.depth.get() + 1);
        let r = {
            let isolate = self.iso();
            v8::scope!(let hs, isolate);
            let context = v8::Local::new(hs, &self.context);
            let scope = &mut v8::ContextScope::new(hs, context);
            let ctx = Ctx::new(scope);
            f(ctx)
        };
        self.depth.set(self.depth.get() - 1);
        if self.depth.get() == 0 {
            self.budget.active.store(false, Ordering::Release);
            self.finish_termination();
        }
        r
    }

    /// Ends a termination (the budget tripped) once no JS is on the stack.
    fn finish_termination(&self) {
        let iso = self.iso();
        if iso.is_execution_terminating() {
            iso.cancel_terminate_execution();
            OOM.with(|o| o.set(false));
        }
    }

    /// V8 keeps no queue length to ask for. The promise hook notes every promise created or
    /// resolved since the last drain: if none, nothing can be queued; if some, a job may be
    /// (the next drain answers and clears it). Never true after a drain, even a cut-short one
    /// (the queue is dropped then).
    pub(crate) fn is_job_pending(&self) -> bool {
        PENDING.with(Cell::get)
    }

    /// Bytes the engine reports as in use (the perf report's JS heap).
    pub(crate) fn memory_used(&self) -> i64 {
        self.iso().get_heap_statistics().used_heap_size() as i64
    }

    /// Runs every queued promise job, ignoring failures and the budget (the bare corpus eval).
    pub(crate) fn run_all_jobs(&self) {
        self.iso().perform_microtask_checkpoint();
        PENDING.with(|p| p.set(false));
    }

    /// Runs the pending promise jobs, then reports rejections that stayed unhandled via the
    /// prelude's `__scratchy_rej_flush` (which may queue more jobs: loop until quiet).
    ///
    /// QY46 on V8: every job has its own callback budget (the watchdog re-arms it when the hook's
    /// counter moves) and the drain stops at `DRAIN_CAP`. A killed job or a tripped cap drops the
    /// rest of the queue (V8 clears it on termination): one log line says so.
    pub(crate) fn drain_jobs(&self, log: &dyn Fn(&str, &str)) {
        let b = &self.budget;
        let drain_start = script_clock_nanos();
        let cap = perf::drain_cap().as_nanos() as u64;
        b.cap_hit.store(false, Ordering::Relaxed);
        b.drain_deadline.store(drain_start + cap, Ordering::Relaxed);
        b.last_jobs.store(JOBS.with(Cell::get), Ordering::Relaxed);
        b.per_job.store(true, Ordering::Relaxed);
        let mut stop: Option<String> = None;
        let cap_line = || format!("js: promise jobs stopped: the drain used more than {} ms of CPU time (remaining jobs dropped)", perf::drain_cap().as_millis());
        loop {
            b.rearm_keep_tripped(perf::callback_budget());
            let span = perf::begin(Some(perf::job_kind()));
            self.iso().perform_microtask_checkpoint();
            b.active.store(false, Ordering::Release);
            // `tripped` is sticky across jobs (an earlier script's trip): the termination
            // flag says THIS checkpoint was stopped.
            let terminating = self.iso().is_execution_terminating();
            let oom = terminating && OOM.with(Cell::take);
            let capped = terminating && !oom && b.cap_hit.load(Ordering::Relaxed);
            perf::end(span, terminating && !capped);
            self.finish_termination();
            if terminating {
                stop = Some(if oom {
                    "js: promise jobs stopped: out of memory (1 job killed, remaining jobs dropped)".to_string()
                } else if capped {
                    cap_line()
                } else {
                    format!("js: promise jobs stopped: exceeded the {} ms CPU-time callback budget (1 job killed, remaining jobs dropped)", perf::callback_budget().as_millis())
                });
                break;
            }
            PENDING.with(|p| p.set(false));
            b.rearm_keep_tripped(perf::callback_budget());
            let more = self.with(|c| match call_global::<_, bool>(&c, "__scratchy_rej_flush", ()) {
                None => false,
                Some(Ok(b)) => b,
                Some(Err(e)) => {
                    log("error", &describe_error(&c, e));
                    false
                }
            });
            if !more && !PENDING.with(Cell::get) {
                break;
            }
            if script_clock_nanos().saturating_sub(drain_start) >= cap {
                stop = Some(cap_line());
                break;
            }
        }
        b.per_job.store(false, Ordering::Relaxed);
        b.drain_deadline.store(0, Ordering::Relaxed);
        if let Some(line) = stop {
            PENDING.with(|p| p.set(false));
            log("error", &line);
        }
    }
}
