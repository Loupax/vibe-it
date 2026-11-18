//! QV8-1 SPIKE (branch `scratchy/qv8-1-v8-spike`, never merged as is): does V8 (the `v8`
//! crate, rusty_v8) fit scratchy? Behind the cargo feature `engine-v8` (off by default).
//! QuickJS stays the page engine; this module only answers the QV8-1 questions:
//!
//!   scratchy --v8-spike seccomp            platform threads, JIT tier-up, sockets refused
//!   scratchy --v8-spike corpus <dir>       the QV8-0 corpus + CPU loops on V8 (bare)
//!   scratchy --v8-spike corpus-qjs <dir>   the same, QuickJS, same binary (control)
//!   scratchy --v8-spike natives            five `__d_*`-shaped natives + call cost (both engines)
//!   scratchy --v8-spike budget             thread-CPU watchdog, terminate/cancel, pause
//!   scratchy --v8-spike stack              stack limit from the real thread stack
//!   scratchy --v8-spike jobs               explicit microtasks, per-job budget, drain cap
//!   scratchy --v8-spike modules            static + dynamic import through host callbacks
//!   scratchy --v8-spike startup            empty isolate vs trivial snapshot
//!
//! `SCRATCHY_V8_FLAGS` is passed to `v8::V8::set_flags_from_string` before V8 starts
//! (e.g. `--jitless`, `--allow-natives-syntax`). Timing code here is spike-only.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use crate::js::{pause_budget_since, script_clock_nanos, thread_cpu_nanos, thread_stack_size};

fn ms(ns: u64) -> f64 {
    (ns as f64 / 1e4).round() / 100.0
}

/// The flags in force (for the report).
fn flags() -> String {
    std::env::var("SCRATCHY_V8_FLAGS").unwrap_or_default()
}

static INIT: Once = Once::new();

/// Starts V8 once per process: flags, the default platform (worker threads for
/// concurrent compile/GC), initialize.
pub(crate) fn init() {
    INIT.call_once(|| {
        let f = flags();
        if !f.is_empty() {
            v8::V8::set_flags_from_string(&f);
        }
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

/// QY47a for V8: the JS stack may use min(4 MB, half of this thread's real stack) below the
/// current stack position; the rest stays for native frames. Returns the limit address.
fn stack_limit_here() -> usize {
    let probe = 0u8;
    let sp = &probe as *const u8 as usize;
    let size = thread_stack_size().unwrap_or(0);
    let js = if size == 0 { 984 * 1024 } else { (size / 2).min(4 << 20) };
    sp.saturating_sub(js)
}

/// A new isolate whose stack limit comes from the real stack of the calling thread.
fn new_isolate(params: v8::CreateParams) -> v8::OwnedIsolate {
    init();
    let limit = stack_limit_here();
    // SAFETY: the address is inside this thread's stack and the isolate is only used here.
    let params = unsafe { params.set_stack_limit(limit as *mut u32) };
    v8::Isolate::new(params)
}

fn proc_status(key: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(key)).map(|l| l[key.len()..].trim().to_string()))
        .unwrap_or_default()
}

fn rss_kb() -> String {
    format!("VmRSS {} VmHWM {}", proc_status("VmRSS:"), proc_status("VmHWM:"))
}

/// Compile + run `code` in the current context. `(compile_ns, run_ns, error)`; run includes
/// a microtask checkpoint (promise jobs of the script, as the QuickJS corpus does).
fn run_code(scope: &mut v8::PinScope, name: &str, code: &str) -> (u64, u64, Option<String>) {
    v8::tc_scope!(let tc, scope);
    let Some(src) = v8::String::new(tc, code) else { return (0, 0, Some("string too long".into())) };
    let rn = v8::String::new(tc, name).unwrap();
    let origin = v8::ScriptOrigin::new(tc, rn.into(), 0, 0, false, 0, None, false, false, false, None);
    let t0 = thread_cpu_nanos();
    let script = v8::Script::compile(tc, src, Some(&origin));
    let t1 = thread_cpu_nanos();
    let Some(script) = script else {
        let e = exc(tc);
        return (t1 - t0, 0, Some(e));
    };
    let r = script.run(tc);
    let err = if r.is_none() { Some(exc(tc)) } else { None };
    tc.perform_microtask_checkpoint();
    let t2 = thread_cpu_nanos();
    (t1 - t0, t2 - t1, err)
}

fn exc(tc: &mut v8::PinnedRef<'_, v8::TryCatch<v8::HandleScope>>) -> String {
    if tc.has_terminated() {
        return "terminated".into();
    }
    match tc.exception() {
        Some(e) => e.to_rust_string_lossy(tc),
        None => "unknown error".into(),
    }
}

/// Eval and return the result as a string (spike helper).
fn eval_str(scope: &mut v8::PinScope, code: &str) -> String {
    v8::tc_scope!(let tc, scope);
    let src = v8::String::new(tc, code).unwrap();
    match v8::Script::compile(tc, src, None).and_then(|s| s.run(tc)) {
        Some(v) => v.to_rust_string_lossy(tc),
        None => format!("<{}>", exc(tc)),
    }
}

const PREAMBLE: &str = "var self=globalThis;var console={log:function(){},warn:function(){},error:function(){}};";

/// The QV8-0 CPU loops, same text as perf.rs.
const CPU_LOOPS: [(&str, &str); 3] = [
    ("fib25", "function fib(n){return n<2?n:fib(n-1)+fib(n-2)} if(fib(25)!==75025) throw new Error('fib');"),
    (
        "sort_200k_objects",
        "var a=[],s=12345;for(var i=0;i<200000;i++){s=(s*1103515245+12345)&0x7fffffff;a.push({k:s,i:i});} a.sort(function(x,y){return x.k-y.k}); \
         for(var i=1;i<a.length;i++) if(a[i-1].k>a[i].k) throw new Error('sort');",
    ),
    (
        "json_round_trip",
        "var o=[];for(var i=0;i<20000;i++)o.push({id:i,name:'item'+i,tags:['a','b','c'],nested:{x:i/3,y:[1,2,3],s:'str\\u00e9'+i}}); \
         var t=JSON.stringify(o);var p=JSON.parse(t);if(p.length!==20000||JSON.stringify(p)!==t) throw new Error('json');",
    ),
];

fn read_corpus(dir: &str) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(dir).expect("corpus dir").filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "js")).collect();
    files.sort();
    files
        .iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned()))
        .collect()
}

fn react_of(texts: &[(String, String)]) -> Option<String> {
    texts.iter().find(|(n, _)| n.starts_with("react-1")).map(|(_, t)| t.clone())
}

/// V8 corpus: each script in a fresh isolate+context (no compilation cache across scripts),
/// bare (PREAMBLE only; react-dom after react, unmeasured), then the CPU loops: run 1 = first
/// run, runs 2..=10 of the same compiled script in the same context = steady state.
fn corpus_v8(dir: &str) {
    let texts = read_corpus(dir);
    println!("engine V8 {} flags '{}'", v8::V8::get_version(), flags());
    println!("name | bytes | compile ms | first run ms | compile+run ms | error");
    for (name, text) in &texts {
        let isolate = &mut new_isolate(Default::default());
        v8::scope!(let hs, isolate);
        let context = v8::Context::new(hs, Default::default());
        let scope = &mut v8::ContextScope::new(hs, context);
        run_code(scope, "pre", PREAMBLE);
        if name.starts_with("react-dom") {
            run_code(scope, "react", &react_of(&texts).unwrap_or_default());
        }
        let (c, r, e) = run_code(scope, name, text);
        println!("{name} | {} | {} | {} | {} | {:?}", text.len(), ms(c), ms(r), ms(c + r), e);
    }
    for (name, code) in CPU_LOOPS {
        let isolate = &mut new_isolate(Default::default());
        v8::scope!(let hs, isolate);
        let context = v8::Context::new(hs, Default::default());
        let scope = &mut v8::ContextScope::new(hs, context);
        v8::tc_scope!(let tc, scope);
        let src = v8::String::new(tc, code).unwrap();
        let t0 = thread_cpu_nanos();
        let script = v8::Script::compile(tc, src, None).unwrap();
        let comp = thread_cpu_nanos() - t0;
        let mut runs = Vec::new();
        for _ in 0..10 {
            let t = thread_cpu_nanos();
            let ok = script.run(tc).is_some();
            runs.push(thread_cpu_nanos() - t);
            assert!(ok, "{name} failed: {}", exc(tc));
        }
        let mut steady = runs[5..].to_vec();
        steady.sort();
        println!("loop {name} | compile {} | first {} | steady median(6..10) {} | all {:?}", ms(comp), ms(runs[0]), ms(steady[2]), runs.iter().map(|r| ms(*r)).collect::<Vec<_>>());
    }
    let isolate = &mut new_isolate(Default::default());
    let hs = isolate.get_heap_statistics();
    println!("end {} | last isolate heap used {} KB", rss_kb(), hs.used_heap_size() / 1024);
}

/// QuickJS control in the same binary: bare for all scripts (react-dom after react), and the
/// CPU loops with steady state (the same source re-evaluated in the same context).
fn corpus_qjs(dir: &str) {
    use rquickjs::{Context, Runtime};
    let texts = read_corpus(dir);
    let (n, v) = crate::js::engine_info();
    println!("engine {n} {v}");
    println!("name | bytes | compile ms | first run ms | compile+run ms | error");
    for (name, text) in &texts {
        let react = react_of(&texts).unwrap_or_default();
        let pre: Vec<&str> = if name.starts_with("react-dom") { vec![PREAMBLE, react.as_str()] } else { vec![PREAMBLE] };
        let (c, r, e) = crate::perf::bare_eval(text, &pre);
        println!("{name} | {} | {} | {} | {} | {:?}", text.len(), ms(c), ms(r), ms(c + r), e);
    }
    for (name, code) in CPU_LOOPS {
        let rt = Runtime::new().unwrap();
        rt.set_memory_limit(1 << 30);
        rt.set_max_stack_size(crate::js::js_stack_limit());
        let ctx = Context::full(&rt).unwrap();
        let runs: Vec<u64> = ctx.with(|ctx| {
            (0..10)
                .map(|_| {
                    let t = thread_cpu_nanos();
                    let r: rquickjs::Result<()> = ctx.eval(code);
                    assert!(r.is_ok(), "{name}");
                    thread_cpu_nanos() - t
                })
                .collect()
        });
        let mut steady = runs[5..].to_vec();
        steady.sort();
        println!("loop {name} | first(compile+run) {} | steady median(6..10) {} | all {:?}", ms(runs[0]), ms(steady[2]), runs.iter().map(|r| ms(*r)).collect::<Vec<_>>());
    }
    println!("end {}", rss_kb());
}

// ---------------------------------------------------------------- natives

/// Argument conversion, the types scratchy natives take (QV8-2's seam will own this).
trait FromArg: Sized {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self>;
}
impl FromArg for i32 {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        v.int32_value(scope)
    }
}
impl FromArg for f64 {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        v.number_value(scope)
    }
}
impl FromArg for bool {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        Some(v.boolean_value(scope))
    }
}
impl FromArg for String {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        Some(v.to_rust_string_lossy(scope))
    }
}
impl<T: FromArg> FromArg for Vec<T> {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        let a = v8::Local::<v8::Array>::try_from(v).ok()?;
        let mut out = Vec::with_capacity(a.length() as usize);
        for i in 0..a.length() {
            let e = a.get_index(scope, i)?;
            out.push(T::from_arg(scope, e)?);
        }
        Some(out)
    }
}
impl<T: FromArg> FromArg for Option<T> {
    fn from_arg(scope: &mut v8::PinScope, v: v8::Local<v8::Value>) -> Option<Self> {
        if v.is_null_or_undefined() { Some(None) } else { T::from_arg(scope, v).map(Some) }
    }
}

trait IntoRet {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value>;
}
impl IntoRet for () {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::undefined(scope).into()
    }
}
impl IntoRet for i32 {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Integer::new(scope, self).into()
    }
}
impl IntoRet for f64 {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, self).into()
    }
}
impl IntoRet for bool {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Boolean::new(scope, self).into()
    }
}
impl IntoRet for String {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::String::new(scope, &self).map(Into::into).unwrap_or_else(|| v8::undefined(scope).into())
    }
}
impl<T: IntoRet> IntoRet for Vec<T> {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let els: Vec<v8::Local<v8::Value>> = self.into_iter().map(|e| e.into_ret(scope)).collect();
        v8::Array::new_with_elements(scope, &els).into()
    }
}
impl<T: IntoRet> IntoRet for Option<T> {
    fn into_ret<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Some(v) => v.into_ret(scope),
            None => v8::null(scope).into(),
        }
    }
}

/// A native: called with the scope and the arguments, returns the JS value.
type NativeFn = Box<dyn for<'s, 'i> Fn(&mut v8::PinScope<'s, 'i>, &v8::FunctionCallbackArguments<'s>) -> Option<v8::Local<'s, v8::Value>>>;

fn trampoline<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, mut rv: v8::ReturnValue<'s, v8::Value>) {
    let Ok(ext) = v8::Local::<v8::External>::try_from(args.data()) else { return };
    // SAFETY: `data` is the External made by `register` from a leaked `Box<NativeFn>`.
    let f = unsafe { &*(ext.value() as *const NativeFn) };
    match f(scope, &args) {
        Some(v) => rv.set(v),
        None => {
            // wrong argument types: a TypeError, as rquickjs gives
            let m = v8::String::new(scope, "native: bad arguments").unwrap();
            let e = v8::Exception::type_error(scope, m);
            scope.throw_exception(e);
        }
    }
}

/// Registers `f` as global `name`. The box is leaked (spike); QV8-3 keeps them in a per-isolate
/// arena dropped with the isolate.
fn register(scope: &mut v8::PinScope, name: &str, f: NativeFn) {
    let ptr = Box::into_raw(Box::new(f)) as *mut c_void;
    let ext = v8::External::new(scope, ptr);
    let func = v8::Function::builder(trampoline).data(ext.into()).build(scope).unwrap();
    let key = v8::String::new(scope, name).unwrap();
    let global = scope.get_current_context().global(scope);
    global.set(scope, key.into(), func.into());
}

macro_rules! native {
    ($scope:expr, $name:expr, |$($a:ident : $t:ty),*| -> $r:ty $body:block) => {{
        #[allow(unused_mut, unused_variables, unused_assignments)]
        let f: NativeFn = Box::new(move |scope, args| {
            let mut i = 0;
            $( let $a: $t = <$t as FromArg>::from_arg(scope, args.get(i))?; i += 1; )*
            let r: $r = (|| $body)();
            Some(IntoRet::into_ret(r, scope))
        });
        register($scope, $name, f);
    }};
}

/// A tiny stand-in for the DomTable: (tag, attrs, classes).
struct FakeNode {
    tag: String,
    attrs: Vec<(String, String)>,
    classes: Vec<String>,
}

thread_local! {
    static NODES: RefCell<Vec<FakeNode>> = RefCell::new((0..100).map(|i| FakeNode { tag: if i % 2 == 0 { "DIV".into() } else { "SPAN".into() }, attrs: vec![("id".into(), format!("n{i}"))], classes: vec![] }).collect());
    /// CPU ms each `__d_rect` burns as a "forced layout" (excluded from the budget clock).
    static LAYOUT_MS: Cell<u64> = const { Cell::new(0) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn burn_cpu(ms: u64) {
    let t = thread_cpu_nanos();
    let mut x = 0u64;
    while thread_cpu_nanos() - t < ms * 1_000_000 {
        x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
    }
}

/// `console.log` + the five natives (+ `__d_run_script`, which calls back into JS).
fn install_natives(scope: &mut v8::PinScope) {
    native!(scope, "__scratchy_log", |level: String, text: String| -> () {
        LOG.with(|l| l.borrow_mut().push(format!("{level}: {text}")));
        if std::env::var_os("SPIKE_ECHO").is_some() {
            eprintln!("js[{level}]: {text}");
        }
    });
    // 1. string getter (js_dom `__d_tag`)
    native!(scope, "__d_tag", |id: i32| -> String { NODES.with(|n| n.borrow().get(id as usize).map(|n| n.tag.clone()).unwrap_or_default()) });
    // 2. numeric (js_dom `__d_nid`)
    native!(scope, "__d_nid", |id: i32| -> f64 { id as f64 * 2.0 + 1.0 });
    // 3. Vec<f64> out (`__d_rect`, forced layout, budget paused) and Vec<String> in
    native!(scope, "__d_rect", |id: i32, vw: f64, vh: f64| -> Vec<f64> {
        let t = thread_cpu_nanos();
        burn_cpu(LAYOUT_MS.with(Cell::get));
        pause_budget_since(t);
        vec![id as f64, 0.0, vw / 2.0, vh / 2.0]
    });
    native!(scope, "__d_set_classes", |id: i32, cls: Vec<String>| -> i32 {
        NODES.with(|n| {
            if let Some(n) = n.borrow_mut().get_mut(id as usize) {
                n.classes = cls;
                n.classes.len() as i32
            } else {
                -1
            }
        })
    });
    // 4. Option (js_dom `__d_attr`)
    native!(scope, "__d_attr", |id: i32, name: String| -> Option<String> {
        NODES.with(|n| n.borrow().get(id as usize).and_then(|n| n.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())))
    });
    // 5. calls back into JS (js.rs `install_script_natives` run): compile+run code now
    let f: NativeFn = Box::new(|scope, args| {
        let code = args.get(0).to_rust_string_lossy(scope);
        let (_, _, e) = run_code(scope, "inline", &code);
        Some(v8::Boolean::new(scope, e.is_none()).into())
    });
    register(scope, "__d_run_script", f);
    eval_str(
        scope,
        "globalThis.console={log:function(){__scratchy_log('log',Array.prototype.join.call(arguments,' '))},error:function(){__scratchy_log('error',Array.prototype.join.call(arguments,' '))}};",
    );
}

fn natives_demo() {
    let isolate = &mut new_isolate(Default::default());
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    install_natives(scope);
    let checks = [
        ("console.log('hello', 1+1)", "undefined"),
        ("__d_tag(3)", "SPAN"),
        ("__d_nid(20)", "41"),
        ("JSON.stringify(__d_rect(7, 800, 600))", "[7,0,400,300]"),
        ("__d_set_classes(2, ['a','b','c'])", "3"),
        ("__d_attr(5, 'id')", "n5"),
        ("String(__d_attr(5, 'nope'))", "null"),
        ("__d_run_script('globalThis.fromInline = __d_tag(0) + __d_nid(1)')", "true"),
        ("fromInline", "DIV3"),
        ("__d_run_script('throw new Error(\"x\")')", "false"),
        ("try { __d_set_classes(1, 5); 'no' } catch (e) { e.name }", "TypeError"),
    ];
    let mut ok = 0;
    for (code, want) in checks {
        let got = eval_str(scope, code);
        let pass = got == want;
        ok += pass as usize;
        println!("{} {code} => {got}{}", if pass { "ok  " } else { "FAIL" }, if pass { String::new() } else { format!(" (want {want})") });
    }
    println!("log lines: {:?}", LOG.with(|l| l.borrow().clone()));
    println!("natives: {ok}/{} checks", checks.len());
    // call cost: 1M calls of the numeric and the string native
    for (label, code) in [
        ("1M __d_nid (f64)", "var s=0;for(var i=0;i<1e6;i++)s+=__d_nid(i&63);"),
        ("1M __d_tag (String)", "var s=0;for(var i=0;i<1e6;i++)s+=__d_tag(i&63).length;"),
        ("100k __d_attr (Option<String>)", "var s=0;for(var i=0;i<1e5;i++){var a=__d_attr(i&63,'id');s+=a?a.length:0}"),
    ] {
        let (c, r, e) = run_code(scope, label, code);
        println!("v8 {label}: {} ms (compile {}) err {e:?}", ms(r), ms(c));
    }
    // same natives through rquickjs, same loops
    use rquickjs::{Context, Function, Runtime};
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        let g = ctx.globals();
        g.set("__d_nid", Function::new(ctx.clone(), |id: i32| -> f64 { id as f64 * 2.0 + 1.0 }).unwrap()).unwrap();
        g.set("__d_tag", Function::new(ctx.clone(), |id: i32| -> String { NODES.with(|n| n.borrow().get(id as usize).map(|n| n.tag.clone()).unwrap_or_default()) }).unwrap()).unwrap();
        g.set(
            "__d_attr",
            Function::new(ctx.clone(), |id: i32, name: String| -> Option<String> {
                NODES.with(|n| n.borrow().get(id as usize).and_then(|n| n.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())))
            })
            .unwrap(),
        )
        .unwrap();
        for (label, code) in [
            ("1M __d_nid (f64)", "var s=0;for(var i=0;i<1e6;i++)s+=__d_nid(i&63);"),
            ("1M __d_tag (String)", "var s=0;for(var i=0;i<1e6;i++)s+=__d_tag(i&63).length;"),
            ("100k __d_attr (Option<String>)", "var s=0;for(var i=0;i<1e5;i++){var a=__d_attr(i&63,'id');s+=a?a.length:0}"),
        ] {
            let t = thread_cpu_nanos();
            let r: rquickjs::Result<()> = ctx.eval(code);
            println!("qjs {label}: {} ms ok {}", ms(thread_cpu_nanos() - t), r.is_ok());
        }
    });
}

// ---------------------------------------------------------------- budget

/// The watchdog: while a callback is armed (`deadline` != 0, budget-clock nanoseconds), it
/// asks the isolate for an interrupt every `TICK`; the interrupt runs ON the JS thread, reads
/// the budget clock there (thread CPU minus `pause_budget_since` time) and terminates when it
/// is past the deadline. So the budget is thread CPU, not wall, and natives that pause the
/// budget (forced layout) are excluded exactly as in QuickJS.
struct Watch {
    deadline: AtomicU64,
    tripped: AtomicBool,
    stop: AtomicBool,
    interrupts: AtomicU64,
    handle: v8::IsolateHandle,
}

const TICK: Duration = Duration::from_millis(2);

unsafe extern "C" fn on_interrupt(_iso: v8::UnsafeRawIsolatePtr, data: *mut c_void) {
    // SAFETY: data is the `Arc<Watch>` kept alive by the watchdog thread and the caller.
    let w = unsafe { &*(data as *const Watch) };
    w.interrupts.fetch_add(1, Ordering::Relaxed);
    if HOOK_MODE.with(Cell::get) == 2 && w.deadline.load(Ordering::Relaxed) != 0 {
        let seq = JOB_SEQ.with(Cell::get);
        if seq != SEQ_SEEN.with(Cell::get) {
            // a new promise job started since the last tick: give it a fresh budget
            SEQ_SEEN.with(|c| c.set(seq));
            let now = script_clock_nanos();
            if now.saturating_sub(DRAIN_START.with(Cell::get)) > DRAIN_CAP.with(Cell::get) {
                w.tripped.store(true, Ordering::Relaxed);
                w.handle.terminate_execution();
                return;
            }
            w.deadline.store(now + JOB_BUDGET.with(Cell::get), Ordering::Relaxed);
            return;
        }
    }
    let d = w.deadline.load(Ordering::Relaxed);
    if d != 0 && script_clock_nanos() > d {
        w.tripped.store(true, Ordering::Relaxed);
        w.handle.terminate_execution();
    }
}

fn start_watchdog(handle: v8::IsolateHandle) -> Arc<Watch> {
    let w = Arc::new(Watch { deadline: AtomicU64::new(0), tripped: AtomicBool::new(false), stop: AtomicBool::new(false), interrupts: AtomicU64::new(0), handle });
    let w2 = w.clone();
    std::thread::Builder::new()
        .name("js-watchdog".into())
        .spawn(move || {
            while !w2.stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                if w2.deadline.load(Ordering::Relaxed) != 0 {
                    w2.handle.request_interrupt(on_interrupt, Arc::as_ptr(&w2) as *mut c_void);
                }
            }
        })
        .unwrap();
    w
}

impl Watch {
    fn arm(&self, budget: Duration) {
        self.tripped.store(false, Ordering::Relaxed);
        self.deadline.store(script_clock_nanos() + budget.as_nanos() as u64, Ordering::Relaxed);
    }
    fn disarm(&self) {
        self.deadline.store(0, Ordering::Relaxed);
    }
}

/// One "callback": arm, run, disarm, cancel a termination. `(budget-clock ms, raw ms, result)`.
fn callback(scope: &mut v8::PinScope, w: &Watch, budget: Duration, code: &str) -> (f64, f64, String) {
    let (r0, b0) = (thread_cpu_nanos(), script_clock_nanos());
    w.arm(budget);
    let (_, _, e) = run_code(scope, "cb", code);
    w.disarm();
    let killed = w.tripped.load(Ordering::Relaxed);
    if scope.is_execution_terminating() || killed {
        scope.thread_safe_handle().cancel_terminate_execution();
    }
    let res = if killed { format!("KILLED ({})", e.unwrap_or_default()) } else { e.map(|e| format!("error {e}")).unwrap_or_else(|| "ok".into()) };
    (ms(script_clock_nanos() - b0), ms(thread_cpu_nanos() - r0), res)
}

fn budget_demo() {
    let isolate = &mut new_isolate(Default::default());
    let w = start_watchdog(isolate.thread_safe_handle());
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    install_natives(scope);
    let b = Duration::from_millis(500);
    let cases: [(&str, &str); 9] = [
        ("warm-up callback", "globalThis.n = 0; for (var i = 0; i < 1e6; i++) n += i;"),
        ("infinite loop", "for (;;) {}"),
        ("next callback runs", "globalThis.after1 = 42; 'fine'"),
        ("infinite loop with try/catch/finally (termination is uncatchable)", "for (;;) { try { for(;;){} } catch (e) { globalThis.caught = 1 } finally { globalThis.fin = 1 } }"),
        ("runaway recursion that catches RangeError and recurses again", "function r(n){ try { r(n+1) } catch (e) { r(n+1) } } r(0)"),
        ("next callback runs (state intact)", "globalThis.after2 = after1 + 1; String(typeof caught) + '/' + String(typeof fin)"),
        ("20 x 40 ms forced layout + 100 ms of script (budget paused in native)", "__layout_ms(40); var r; for (var i = 0; i < 20; i++) r = __d_rect(i, 800, 600); var t = Date.now(); while (Date.now() - t < 100) {}"),
        ("same 900 ms of CPU, all script (no pause): killed", "var t = Date.now(); while (Date.now() - t < 900) {}"),
        ("after kills: promise + microtask still work", "globalThis.p = 0; Promise.resolve(5).then(function(v){ globalThis.p = v });"),
    ];
    let f: NativeFn = Box::new(|scope, args| {
        let v = args.get(0).int32_value(scope).unwrap_or(0) as u64;
        LAYOUT_MS.with(|c| c.set(v));
        Some(v8::undefined(scope).into())
    });
    register(scope, "__layout_ms", f);
    for (label, code) in cases {
        let (bc, raw, res) = callback(scope, &w, b, code);
        println!("{label}: {res} | budget clock {bc} ms | raw CPU {raw} ms");
    }
    println!("state: after1={} after2={} p={} typeof caught={} typeof fin={}", eval_str(scope, "after1"), eval_str(scope, "after2"), eval_str(scope, "p"), eval_str(scope, "typeof caught"), eval_str(scope, "typeof fin"));
    // kill latency: budgets 5/20/50/100 ms on a busy loop
    for bms in [5u64, 20, 50, 100] {
        let (bc, _, res) = callback(scope, &w, Duration::from_millis(bms), "for(;;){}");
        println!("kill latency budget {bms} ms -> stopped at {bc} ms budget clock ({res})");
    }
    // watchdog overhead: fib(30) with the watchdog armed (interrupt every 2 ms) vs disarmed
    run_code(scope, "def", "function fib(n){return n<2?n:fib(n-1)+fib(n-2)}");
    for _ in 0..3 {
        run_code(scope, "w", "fib(27)");
    }
    let t = thread_cpu_nanos();
    run_code(scope, "fib", "fib(30)");
    let off = thread_cpu_nanos() - t;
    let i0 = w.interrupts.load(Ordering::Relaxed);
    let (_, on, res) = callback(scope, &w, Duration::from_secs(10), "fib(30)");
    println!("fib(30): watchdog off {} ms, armed {} ms ({res}, {} interrupts)", ms(off), on, w.interrupts.load(Ordering::Relaxed) - i0);
    w.stop.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------- stack

fn stack_demo_here(label: &str) {
    let isolate = &mut new_isolate(Default::default());
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    let size = thread_stack_size().unwrap_or(0);
    println!("[{label}] thread stack {} KB, JS limit {} KB below entry", size / 1024, (size / 2).min(4 << 20) / 1024);
    let cases = [
        ("f(1500)", "function f(n){ return n === 0 ? 0 : 1 + f(n-1) } f(1500)"),
        ("f(10000)", "function g(n){ return n === 0 ? 0 : 1 + g(n-1) } g(10000)"),
        (
            "YouTube-like nested constructors (300 deep, 3 super calls each)",
            "class A{constructor(n){this.c = n ? new C(n-1) : null}} class B extends A{constructor(n){super(n)}} class C extends B{constructor(n){super(n); this.d=n}} var x = new C(300); var d=0; while (x) { d++; x = x.c } d",
        ),
        ("max depth (runaway, caught)", "var depth=0; function h(){ depth++; h() } try { h() } catch (e) { (e instanceof RangeError) + ' ' + e.message + ' depth ' + depth }"),
        ("max depth, 8 locals per frame", "var depth2=0; function k(a,b,c){ var x1=a+1,x2=b+2,x3=c+3,x4=x1*x2,x5=x3*x4,x6=x5+1,x7=x6+2,x8=x7+3; depth2++; return k(x1,x2,x8)+x7 } try { k(1,2,3) } catch (e) { e.name + ' depth ' + depth2 }"),
        ("uncaught runaway -> catchable error, isolate still fine", "function u(){u()} u()"),
        ("after", "'still alive ' + f(100)"),
    ];
    for (l, code) in cases {
        let v = eval_str(scope, code);
        println!("[{label}] {l}: {v}");
    }
}

fn stack_qjs_depth(label: &str) {
    use rquickjs::{Context, Runtime};
    let rt = Runtime::new().unwrap();
    rt.set_max_stack_size(crate::js::js_stack_limit());
    let ctx = Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        let r: String = ctx.eval("var depth=0; function h(){ depth++; h() } try { h() } catch (e) { e.name + ' depth ' + depth }").unwrap_or_default();
        println!("[{label}] QuickJS max depth (js_stack_limit {} KB): {r}", crate::js::js_stack_limit() / 1024);
    });
}

fn stack_demo() {
    stack_demo_here("main thread");
    stack_qjs_depth("main thread");
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            stack_demo_here("2 MB thread");
            stack_qjs_depth("2 MB thread");
        })
        .unwrap()
        .join()
        .unwrap();
}

// ---------------------------------------------------------------- promise jobs

thread_local! {
    static HOOK_WATCH: RefCell<Option<Arc<Watch>>> = const { RefCell::new(None) };
    static JOB_BUDGET: Cell<u64> = const { Cell::new(0) };
    static DRAIN_START: Cell<u64> = const { Cell::new(0) };
    static DRAIN_CAP: Cell<u64> = const { Cell::new(u64::MAX) };
    static JOBS_SEEN: Cell<u64> = const { Cell::new(0) };
    /// 0 = hook does nothing, 1 = hook reads the clock and re-arms per job (exact),
    /// 2 = hook only bumps JOB_SEQ; the watchdog interrupt re-arms when it sees a new job.
    static HOOK_MODE: Cell<u8> = const { Cell::new(2) };
    static JOB_SEQ: Cell<u64> = const { Cell::new(0) };
    static SEQ_SEEN: Cell<u64> = const { Cell::new(0) };
}

/// PromiseHook `Before` = a promise reaction job is about to run: give it a fresh budget
/// (QY46). If the whole drain is past its cap, stop here (V8 then drops the rest of the queue).
unsafe extern "C" fn promise_hook(t: v8::PromiseHookType, _p: v8::Local<v8::Promise>, _parent: v8::Local<v8::Value>) {
    if t != v8::PromiseHookType::Before {
        return;
    }
    JOBS_SEEN.with(|c| c.set(c.get() + 1));
    match HOOK_MODE.with(Cell::get) {
        0 => return,
        2 => {
            JOB_SEQ.with(|c| c.set(c.get() + 1));
            return;
        }
        _ => {}
    }
    HOOK_WATCH.with(|w| {
        if let Some(w) = w.borrow().as_ref() {
            let now = script_clock_nanos();
            if now.saturating_sub(DRAIN_START.with(Cell::get)) > DRAIN_CAP.with(Cell::get) {
                w.tripped.store(true, Ordering::Relaxed);
                w.handle.terminate_execution();
                return;
            }
            w.deadline.store(now + JOB_BUDGET.with(Cell::get), Ordering::Relaxed);
        }
    });
}

fn drain(scope: &mut v8::PinScope, w: &Arc<Watch>, job_budget: Duration, cap: Duration) -> (f64, bool) {
    JOB_BUDGET.with(|c| c.set(job_budget.as_nanos() as u64));
    DRAIN_CAP.with(|c| c.set(cap.as_nanos() as u64));
    DRAIN_START.with(|c| c.set(script_clock_nanos()));
    w.arm(job_budget);
    let t = script_clock_nanos();
    scope.perform_microtask_checkpoint();
    w.disarm();
    let killed = w.tripped.load(Ordering::Relaxed);
    if killed || scope.is_execution_terminating() {
        scope.thread_safe_handle().cancel_terminate_execution();
    }
    (ms(script_clock_nanos() - t), killed)
}

fn jobs_demo() {
    if let Ok(m) = std::env::var("SPIKE_HOOK_MODE") {
        HOOK_MODE.with(|c| c.set(m.parse().unwrap_or(2)));
    }
    println!("hook mode {}", HOOK_MODE.with(Cell::get));
    let isolate = &mut new_isolate(Default::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let w = start_watchdog(isolate.thread_safe_handle());
    HOOK_WATCH.with(|h| *h.borrow_mut() = Some(w.clone()));
    isolate.set_promise_hook(promise_hook);
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    let b = Duration::from_millis(100);
    let cap = Duration::from_millis(400);
    let busy = "function busy(ms){var t=Date.now();while(Date.now()-t<ms){}}";
    run_code(scope, "def", busy);
    // 1. explicit policy: nothing runs until the checkpoint
    eval_str(scope, "globalThis.order=[]; Promise.resolve().then(function(){order.push('job')}); order.push('sync');");
    println!("before checkpoint: {}", eval_str(scope, "order.join(',')"));
    drain(scope, &w, b, cap);
    println!("after checkpoint: {}", eval_str(scope, "order.join(',')"));
    // 2. per-job budget: 8 jobs of 60 ms each = 480 ms total, every job under its 100 ms budget
    eval_str(scope, "globalThis.done=0; for (var i=0;i<6;i++) Promise.resolve().then(function(){busy(60); done++});");
    let (t, k) = drain(scope, &w, b, Duration::from_secs(10));
    println!("6 x 60 ms jobs, 100 ms per-job budget: done={} killed={k} drain {t} ms", eval_str(scope, "done"));
    // 3. one runaway job between two good ones
    eval_str(scope, "globalThis.r=[]; Promise.resolve().then(function(){r.push('A')}); Promise.resolve().then(function(){for(;;){}}); Promise.resolve().then(function(){r.push('C')});");
    let (t, k) = drain(scope, &w, b, cap);
    println!("A, runaway, C: killed={k} after kill r={} drain {t} ms", eval_str(scope, "r.join(',')"));
    let (_, k2) = drain(scope, &w, b, cap);
    println!("  next checkpoint (does C run later?): r={} killed={k2}", eval_str(scope, "r.join(',')"));
    // 4. drain cap: 20 x 50 ms jobs (1 s) with a 400 ms cap
    eval_str(scope, "globalThis.capped=0; for (var i=0;i<20;i++) Promise.resolve().then(function(){busy(50); capped++});");
    JOBS_SEEN.with(|c| c.set(0));
    let (t, k) = drain(scope, &w, b, cap);
    println!("20 x 50 ms jobs, 400 ms drain cap: ran {} killed={k} drain {t} ms", eval_str(scope, "capped"));
    let (_, _) = drain(scope, &w, b, cap);
    println!("  next checkpoint: capped={} (jobs left in the queue after a cap trip are dropped by V8)", eval_str(scope, "capped"));
    // 5. queueMicrotask (V8 has none: an embedder native over Isolate::EnqueueMicrotask, and
    //    a JS version over a promise reaction) and async functions: does the hook see them?
    println!("typeof queueMicrotask in a bare V8 context: {}", eval_str(scope, "typeof queueMicrotask"));
    let f: NativeFn = Box::new(|scope, args| {
        let f = v8::Local::<v8::Function>::try_from(args.get(0)).ok()?;
        scope.enqueue_microtask(f);
        Some(v8::undefined(scope).into())
    });
    register(scope, "__native_qm", f);
    eval_str(scope, "globalThis.__js_qm = function (cb) { Promise.resolve().then(function () { cb() }) };");
    JOBS_SEEN.with(|c| c.set(0));
    eval_str(scope, "globalThis.q=0; __native_qm(function(){q++}); __native_qm(function(){q++});");
    drain(scope, &w, b, cap);
    println!("native EnqueueMicrotask x2: ran {} hook Before seen {}", eval_str(scope, "q"), JOBS_SEEN.with(Cell::get));
    JOBS_SEEN.with(|c| c.set(0));
    eval_str(scope, "globalThis.q2=0; __js_qm(function(){q2++}); __js_qm(function(){q2++});");
    drain(scope, &w, b, cap);
    println!("JS queueMicrotask via promise x2: ran {} hook Before seen {}", eval_str(scope, "q2"), JOBS_SEEN.with(Cell::get));
    eval_str(scope, "globalThis.q3=[]; __native_qm(function(){busy(80); q3.push(1)}); __native_qm(function(){busy(80); q3.push(2)}); __native_qm(function(){for(;;){}}); ");
    let (t, k) = drain(scope, &w, b, cap);
    println!("native microtasks 80 ms, 80 ms, runaway (no hook => ONE budget for all three): killed={k} q3={} drain {t} ms", eval_str(scope, "q3.join(',')"));
    JOBS_SEEN.with(|c| c.set(0));
    eval_str(scope, "globalThis.aw=0; (async function(){ await null; aw++; await null; aw++ })();");
    drain(scope, &w, b, cap);
    println!("async fn with 2 awaits: aw={} hook Before seen {}", eval_str(scope, "aw"), JOBS_SEEN.with(Cell::get));
    // 7. cost of the promise hook: 200k chained thens per hook mode
    for (mode, label) in [(1u8, "clock read per job (mode 1)"), (2, "counter per job, re-arm in the 2 ms interrupt (mode 2)"), (0, "installed, returns at once (mode 0)")] {
        HOOK_MODE.with(|c| c.set(mode));
        eval_str(scope, "globalThis.cnt=0; var p=Promise.resolve(); for (var i=0;i<200000;i++) p=p.then(function(){cnt++});");
        let t = thread_cpu_nanos();
        drain(scope, &w, Duration::from_secs(10), Duration::from_secs(100));
        println!("200k chained thens, hook {label}: {} ms (cnt {})", ms(thread_cpu_nanos() - t), eval_str(scope, "cnt"));
    }
    HOOK_MODE.with(|c| c.set(2));
    w.stop.store(true, Ordering::Relaxed);
    {
        // no hook at all: a second isolate
        let iso2 = &mut new_isolate(Default::default());
        iso2.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let hs2, iso2);
        let c2 = v8::Context::new(hs2, Default::default());
        let s2 = &mut v8::ContextScope::new(hs2, c2);
        eval_str(s2, "globalThis.cnt=0; var p=Promise.resolve(); for (var i=0;i<200000;i++) p=p.then(function(){cnt++});");
        let t = thread_cpu_nanos();
        s2.perform_microtask_checkpoint();
        println!("200k chained thens, NO hook: {} ms (cnt {})", ms(thread_cpu_nanos() - t), eval_str(s2, "cnt"));
    }
    // QuickJS, same 200k chain (execute_pending_job one at a time, as drain_jobs does)
    let rt = rquickjs::Runtime::new().unwrap();
    let ctx = rquickjs::Context::full(&rt).unwrap();
    ctx.with(|c| {
        let _: rquickjs::Result<()> = c.eval("globalThis.cnt=0; var p=Promise.resolve(); for (var i=0;i<200000;i++) p=p.then(function(){cnt++});");
    });
    let t = thread_cpu_nanos();
    while rt.is_job_pending() {
        let _ = rt.execute_pending_job();
    }
    let n: i32 = ctx.with(|c| c.eval("cnt").unwrap_or(-1));
    println!("QuickJS 200k chained thens: {} ms (cnt {n})", ms(thread_cpu_nanos() - t));
}

/// Heap/RSS after loading the five bare corpus libraries into ONE context, per engine.
fn memory_demo(dir: &str, engine: &str) {
    let texts: Vec<(String, String)> = read_corpus(dir).into_iter().filter(|(n, _)| !n.starts_with("jquery") && !n.starts_with("react-dom")).collect();
    let rss0 = proc_status("VmRSS:");
    if engine == "v8" {
        let isolate = &mut new_isolate(Default::default());
        {
            v8::scope!(let hs, isolate);
            let context = v8::Context::new(hs, Default::default());
            let scope = &mut v8::ContextScope::new(hs, context);
            run_code(scope, "pre", PREAMBLE);
            for (n, t) in &texts {
                let (_, _, e) = run_code(scope, n, t);
                assert!(e.is_none(), "{n}: {e:?}");
            }
            scope.low_memory_notification();
            let hs = scope.get_heap_statistics();
            println!("V8 one context, {} libs: heap used {} KB, total {} KB, external {} KB | RSS before {rss0}, {}", texts.len(), hs.used_heap_size() / 1024, hs.total_heap_size() / 1024, hs.external_memory() / 1024, rss_kb());
        }
    } else {
        let rt = rquickjs::Runtime::new().unwrap();
        rt.set_max_stack_size(crate::js::js_stack_limit());
        let ctx = rquickjs::Context::full(&rt).unwrap();
        ctx.with(|c| {
            let _: rquickjs::Result<()> = c.eval(PREAMBLE);
            for (n, t) in &texts {
                let r: rquickjs::Result<()> = c.eval(t.as_str());
                assert!(r.is_ok(), "{n}");
            }
        });
        rt.run_gc();
        println!("QuickJS one context, {} libs: heap used {} KB | RSS before {rss0}, {}", texts.len(), rt.memory_usage().memory_used_size / 1024, rss_kb());
    }
}

// ---------------------------------------------------------------- modules

thread_local! {
    static MOD_SRC: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static MOD_CACHE: RefCell<HashMap<String, v8::Global<v8::Module>>> = RefCell::new(HashMap::new());
    static MOD_NAMES: RefCell<HashMap<i32, String>> = RefCell::new(HashMap::new());
}

/// The resolver scratchy already has (js_modules::PageResolver): base + specifier -> URL,
/// with the import map applied by scratchy, not by V8 (V8 has no import-map support).
fn resolve_name(base: &str, spec: &str) -> String {
    let map = [("lib", "https://x.test/lib/index.js")];
    if let Some((_, u)) = map.iter().find(|(k, _)| *k == spec) {
        return u.to_string();
    }
    url::Url::parse(base).and_then(|b| b.join(spec)).map(|u| u.to_string()).unwrap_or_else(|_| spec.to_string())
}

fn compile_module<'s>(scope: &mut v8::PinScope<'s, '_>, url: &str) -> Option<v8::Local<'s, v8::Module>> {
    if let Some(g) = MOD_CACHE.with(|c| c.borrow().get(url).cloned()) {
        return Some(v8::Local::new(scope, g));
    }
    let src = MOD_SRC.with(|m| m.borrow().get(url).cloned())?;
    let s = v8::String::new(scope, &src)?;
    let n = v8::String::new(scope, url)?;
    let origin = v8::ScriptOrigin::new(scope, n.into(), 0, 0, false, 0, None, false, false, true, None);
    let mut source = v8::script_compiler::Source::new(s, Some(&origin));
    let m = v8::script_compiler::compile_module(scope, &mut source)?;
    MOD_CACHE.with(|c| c.borrow_mut().insert(url.to_string(), v8::Global::new(scope, m)));
    MOD_NAMES.with(|c| c.borrow_mut().insert(m.get_identity_hash().get(), url.to_string()));
    Some(m)
}

fn resolve_cb<'s>(context: v8::Local<'s, v8::Context>, spec: v8::Local<'s, v8::String>, _attrs: v8::Local<'s, v8::FixedArray>, referrer: v8::Local<'s, v8::Module>) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let base = MOD_NAMES.with(|c| c.borrow().get(&referrer.get_identity_hash().get()).cloned()).unwrap_or_default();
    let url = resolve_name(&base, &spec.to_rust_string_lossy(scope));
    compile_module(scope, &url)
}

fn dyn_import_cb<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _opts: v8::Local<'s, v8::Data>,
    resource: v8::Local<'s, v8::Value>,
    spec: v8::Local<'s, v8::String>,
    _attrs: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let base = resource.to_rust_string_lossy(scope);
    let url = resolve_name(&base, &spec.to_rust_string_lossy(scope));
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    match compile_module(scope, &url) {
        Some(m) => {
            if m.get_status() == v8::ModuleStatus::Uninstantiated && m.instantiate_module(scope, resolve_cb).is_none() {
                let e = v8::String::new(scope, "instantiate failed").unwrap();
                resolver.reject(scope, e.into());
                return Some(promise);
            }
            if m.get_status() == v8::ModuleStatus::Instantiated {
                let _ = m.evaluate(scope);
            }
            let ns = m.get_module_namespace();
            resolver.resolve(scope, ns);
        }
        None => {
            let e = v8::String::new(scope, &format!("cannot load {url}")).unwrap();
            let e = v8::Exception::type_error(scope, e);
            resolver.reject(scope, e);
        }
    }
    Some(promise)
}

extern "C" fn import_meta_cb(context: v8::Local<v8::Context>, module: v8::Local<v8::Module>, meta: v8::Local<v8::Object>) {
    v8::callback_scope!(unsafe scope, context);
    let url = MOD_NAMES.with(|c| c.borrow().get(&module.get_identity_hash().get()).cloned()).unwrap_or_default();
    let k = v8::String::new(scope, "url").unwrap();
    let v = v8::String::new(scope, &url).unwrap();
    meta.create_data_property(scope, k.into(), v.into());
}

fn modules_demo() {
    MOD_SRC.with(|m| {
        let mut m = m.borrow_mut();
        m.insert("https://x.test/main.js".into(), "import { add } from './util.js'; import def from 'lib'; globalThis.out = add(2, 3) + ':' + def + ':' + import.meta.url; export const fromMain = 1;".into());
        m.insert("https://x.test/util.js".into(), "export function add(a, b) { return a + b }".into());
        m.insert("https://x.test/lib/index.js".into(), "export default 'libdefault';".into());
        m.insert("https://x.test/lazy.js".into(), "export const lazy = 'lazy:' + import.meta.url;".into());
        m.insert("https://x.test/tla.js".into(), "await Promise.resolve(); export const tla = 'tla';".into());
    });
    let isolate = &mut new_isolate(Default::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    isolate.set_host_import_module_dynamically_callback(dyn_import_cb);
    isolate.set_host_initialize_import_meta_object_callback(import_meta_cb);
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    let m = compile_module(scope, "https://x.test/main.js").unwrap();
    let inst = m.instantiate_module(scope, resolve_cb);
    let r = m.evaluate(scope);
    scope.perform_microtask_checkpoint();
    println!("static import graph: instantiate {inst:?}, evaluate promise {}, out = {}", r.map(|r| r.is_promise()).unwrap_or(false), eval_str(scope, "out"));
    // dynamic import from a CLASSIC script (resource name = the page URL)
    let code = "globalThis.dyn='pending'; import('./lazy.js').then(function(ns){ dyn = ns.lazy }, function(e){ dyn = 'ERR ' + e }); import('./tla.js').then(function(ns){ globalThis.tla = ns.tla }); import('./missing.js').catch(function(e){ globalThis.miss = e.name + ': ' + e.message });";
    {
        v8::tc_scope!(let tc, scope);
        let s = v8::String::new(tc, code).unwrap();
        let n = v8::String::new(tc, "https://x.test/page.html").unwrap();
        let origin = v8::ScriptOrigin::new(tc, n.into(), 0, 0, false, 0, None, false, false, false, None);
        v8::Script::compile(tc, s, Some(&origin)).and_then(|s| s.run(tc));
        tc.perform_microtask_checkpoint();
    }
    println!("dynamic import(): dyn = {}, tla = {}, missing = {}", eval_str(scope, "dyn"), eval_str(scope, "globalThis.tla"), eval_str(scope, "globalThis.miss"));
}

// ---------------------------------------------------------------- startup

fn startup_demo() {
    init();
    let trivial = "globalThis.__pre = { a: 1, f: function (x) { return x + 1 } }; for (var i = 0; i < 200; i++) globalThis['g' + i] = function (y) { return y * 2 };";
    let blob = {
        let mut sc = v8::Isolate::snapshot_creator(None, None);
        {
            v8::scope!(let scope, &mut sc);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            run_code(scope, "pre", trivial);
            scope.set_default_context(context);
        }
        sc.create_blob(v8::FunctionCodeHandling::Keep).unwrap()
    };
    let blob: Vec<u8> = blob.to_vec();
    println!("snapshot blob {} KB", blob.len() / 1024);
    for round in 0..3 {
        let (c0, w0) = (thread_cpu_nanos(), Instant::now());
        {
            let isolate = &mut new_isolate(Default::default());
            v8::scope!(let hs, isolate);
            let context = v8::Context::new(hs, Default::default());
            let scope = &mut v8::ContextScope::new(hs, context);
            eval_str(scope, "1");
        }
        let empty = (ms(thread_cpu_nanos() - c0), w0.elapsed().as_secs_f64() * 1000.0);
        let (c0, w0) = (thread_cpu_nanos(), Instant::now());
        {
            let isolate = &mut new_isolate(v8::CreateParams::default().snapshot_blob(blob.clone().into()));
            v8::scope!(let hs, isolate);
            let context = v8::Context::new(hs, Default::default());
            let scope = &mut v8::ContextScope::new(hs, context);
            assert_eq!(eval_str(scope, "__pre.f(1) + g199(2)"), "6");
        }
        let snap = (ms(thread_cpu_nanos() - c0), w0.elapsed().as_secs_f64() * 1000.0);
        let (c0, w0) = (thread_cpu_nanos(), Instant::now());
        {
            let isolate = &mut new_isolate(Default::default());
            v8::scope!(let hs, isolate);
            let context = v8::Context::new(hs, Default::default());
            let scope = &mut v8::ContextScope::new(hs, context);
            run_code(scope, "pre", trivial);
        }
        let nosnap = (ms(thread_cpu_nanos() - c0), w0.elapsed().as_secs_f64() * 1000.0);
        let (c0, w0) = (thread_cpu_nanos(), Instant::now());
        {
            let rt = rquickjs::Runtime::new().unwrap();
            let ctx = rquickjs::Context::full(&rt).unwrap();
            ctx.with(|c| {
                let _: rquickjs::Result<()> = c.eval(trivial);
            });
        }
        let qjs = (ms(thread_cpu_nanos() - c0), w0.elapsed().as_secs_f64() * 1000.0);
        println!("round {round}: isolate+context+drop (CPU ms, wall ms) empty {empty:?} | from snapshot {snap:?} | same script without snapshot {nosnap:?} | QuickJS runtime+context+script {qjs:?}");
    }
}

// ---------------------------------------------------------------- seccomp / JIT

fn tasks() -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc/self/task") {
        for e in rd.flatten() {
            let p = e.path();
            let comm = std::fs::read_to_string(p.join("comm")).unwrap_or_default().trim().to_string();
            let sc = std::fs::read_to_string(p.join("status")).ok().and_then(|s| s.lines().find(|l| l.starts_with("Seccomp:")).map(|l| l[8..].trim().to_string())).unwrap_or_default();
            v.push((comm, sc));
        }
    }
    v
}

fn try_sockets(label: &str) {
    // SAFETY: plain libc calls with valid arguments; fds closed if opened.
    unsafe {
        for (name, af, ty) in [("AF_INET/TCP", libc::AF_INET, libc::SOCK_STREAM), ("AF_INET6/UDP", libc::AF_INET6, libc::SOCK_DGRAM), ("AF_NETLINK", libc::AF_NETLINK, libc::SOCK_RAW), ("AF_UNIX", libc::AF_UNIX, libc::SOCK_STREAM)] {
            let fd = libc::socket(af, ty, 0);
            let err = *libc::__errno_location();
            println!("[{label}] socket({name}) = {fd}{}", if fd < 0 { format!(" errno {err} ({})", std::io::Error::from_raw_os_error(err)) } else { String::new() });
            if fd >= 0 {
                libc::close(fd);
            }
        }
    }
    let dns = std::net::ToSocketAddrs::to_socket_addrs(&("example.com", 80)).map(|a| a.count());
    println!("[{label}] DNS example.com: {dns:?}");
}

fn seccomp_demo() {
    println!("Seccomp mode of main thread before V8: {}", proc_status("Seccomp:"));
    let before = tasks().len();
    init();
    let isolate = &mut new_isolate(Default::default());
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    let natives = flags().contains("--allow-natives-syntax");
    let hot = "function hot(n){var s=0;for(var i=0;i<n;i++){s=(s+i*i)%1000003}return s}";
    run_code(scope, "def", hot);
    let mut times = Vec::new();
    for _ in 0..200 {
        let t = thread_cpu_nanos();
        run_code(scope, "call", "hot(100000)");
        times.push(thread_cpu_nanos() - t);
    }
    println!(
        "V8 {} flags '{}': hot(1e5) call 1 {} ms, call 10 {} ms, call 200 {} ms, sum {} ms",
        v8::V8::get_version(),
        flags(),
        ms(times[0]),
        ms(times[9]),
        ms(times[199]),
        ms(times.iter().sum())
    );
    if natives {
        let st: i64 = eval_str(scope, "%GetOptimizationStatus(hot)").parse().unwrap_or(-1);
        let names = ["IsFunction", "NeverOptimize", "MaybeDeopted", "Optimized", "Maglevved", "TurboFanned", "Interpreted", "MarkedForOptimization", "MarkedForConcurrentOptimization", "OptimizingConcurrently", "IsExecuting", "TopmostFrameIsTurboFanned", "LiteMode", "MarkedForDeoptimization", "Baseline"];
        let set: Vec<&str> = names.iter().enumerate().filter(|(i, _)| st >= 0 && st & (1 << i) != 0).map(|(_, n)| *n).collect();
        println!("%GetOptimizationStatus(hot) = {st} {set:?}");
        // a fresh function walked through the tiers: interpreted -> Sparkplug -> Maglev -> Turbofan
        run_code(scope, "def2", "function t2(n){var s=0;for(var i=0;i<n;i++)s+=i;return s}");
        for (label, code) in [
            ("t2 cold", "t2(1)"),
            ("after %CompileBaseline", "%CompileBaseline(t2); t2(2)"),
            ("after %OptimizeMaglevOnNextCall", "%PrepareFunctionForOptimization(t2); t2(3); %OptimizeMaglevOnNextCall(t2); t2(4)"),
            ("after %OptimizeFunctionOnNextCall", "%OptimizeFunctionOnNextCall(t2); t2(5)"),
        ] {
            run_code(scope, label, code);
            let st: i64 = eval_str(scope, "%GetOptimizationStatus(t2)").parse().unwrap_or(-1);
            let set: Vec<&str> = names.iter().enumerate().filter(|(i, _)| st >= 0 && st & (1 << i) != 0).map(|(_, n)| *n).collect();
            println!("  {label}: {st} {set:?}");
        }
    }
    let ts = tasks();
    println!("threads before V8 init {before}, after {}: {:?}", ts.len(), ts);
    println!("all threads seccomp-filtered: {}", ts.iter().all(|(_, s)| s == "2"));
    try_sockets("main thread, after V8 ran");
    std::thread::spawn(|| try_sockets("new thread")).join().unwrap();
    println!("{}", rss_kb());
}

/// For the libmpv-next-to-V8 test: run a hot loop on a fresh isolate on this thread and
/// return (result, CPU ms, seccomp modes of all threads).
#[cfg(test)]
pub(crate) fn jit_smoke() -> (String, f64, Vec<(String, String)>) {
    let isolate = &mut new_isolate(Default::default());
    v8::scope!(let hs, isolate);
    let context = v8::Context::new(hs, Default::default());
    let scope = &mut v8::ContextScope::new(hs, context);
    eval_str(scope, "function fib(n){return n<2?n:fib(n-1)+fib(n-2)} fib(25)");
    let t = thread_cpu_nanos();
    let r = eval_str(scope, "fib(30)");
    (r, ms(thread_cpu_nanos() - t), tasks())
}

/// `scratchy --v8-spike <mode> [args]`.
pub(crate) fn main(args: &[String]) -> i32 {
    crate::js::set_js_enabled(true);
    let mode = args.first().map(String::as_str).unwrap_or("");
    let arg = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "seccomp" => seccomp_demo(),
        "corpus" => corpus_v8(arg),
        "corpus-qjs" => corpus_qjs(arg),
        "natives" => natives_demo(),
        "budget" => budget_demo(),
        "stack" => stack_demo(),
        "jobs" => jobs_demo(),
        "modules" => modules_demo(),
        "startup" => startup_demo(),
        "memory" => memory_demo(arg, args.get(2).map(String::as_str).unwrap_or("v8")),
        _ => {
            eprintln!("usage: scratchy --v8-spike seccomp|corpus <dir>|corpus-qjs <dir>|natives|budget|stack|jobs|modules|startup");
            return 2;
        }
    }
    0
}
