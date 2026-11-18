#[allow(unused_imports)]
use super::*;
use crate::js_engine::{self, native, Bytes, BytesRef, Ctx, Engine, EngineOpts, JsObject, JsResult, Source};

// ---- QV8-2: the engine seam (js_engine.rs)

fn bare() -> Engine {
    Engine::new(EngineOpts { memory_limit: 1 << 28, stack_limit: crate::js::js_stack_limit(), page: None }).ok().unwrap()
}

fn run(eng: &Engine, code: &str) -> Result<(), String> {
    eng.with(|ctx| {
        let src = Source::new(code, "t").unwrap();
        let compiled = src.compile(&ctx).map_err(|_| js_engine::describe_exception(&ctx))?;
        compiled.run(&ctx).map_err(|_| js_engine::describe_exception(&ctx))
    })
}

fn install_mkbytes<'js>(ctx: &Ctx<'js>) {
    native!(ctx, "mkbytes", move |ctx: Ctx<'js>| -> JsResult<JsObject<'js>> {
        let o = JsObject::new(&ctx)?;
        o.set("body", BytesRef(b"hi"))?;
        o.set("n", 7u32)?;
        Ok(o)
    });
}

#[test]
fn qv8_2_native_registration_and_value_conversion() {
    let eng = bare();
    let seen = Rc::new(RefCell::new(String::new()));
    let s = seen.clone();
    eng.with(|ctx| {
        native!(&ctx, "add", |a: i32, b: i32| -> i32 { a + b });
        native!(&ctx, "join", |v: Vec<String>, sep: String| -> String { v.join(&sep) });
        native!(&ctx, "maybe", |on: bool| -> Option<f64> { on.then_some(1.5) });
        native!(&ctx, "note", move |b: Bytes| {
            *s.borrow_mut() = String::from_utf8_lossy(&b.0).into_owned();
        });
        install_mkbytes(&ctx);
    });
    let code = "if (add(2, 3) !== 5) throw new Error('add');\
                if (join(['a','b'], '-') !== 'a-b') throw new Error('join');\
                if (maybe(true) !== 1.5 || maybe(false) !== undefined) throw new Error('opt');\
                note(new Uint8Array([104, 105]));\
                var o = mkbytes(); if (o.n !== 7 || o.body.length !== 2 || o.body[0] !== 104) throw new Error('obj');";
    assert_eq!(run(&eng, code), Ok(()));
    assert_eq!(*seen.borrow(), "hi");
}

#[test]
fn qv8_2_object_reader_and_error_formatting() {
    let eng = bare();
    run(&eng, "var d = {url: 'u', keepalive: true, headers: [['a','b']]};").unwrap();
    eng.with(|ctx| {
        let d = js_engine::global_object(&ctx, "d").unwrap();
        assert_eq!(d.get::<String>("url").as_deref(), Some("u"));
        assert_eq!(d.get::<bool>("keepalive"), Some(true));
        assert_eq!(d.get::<Vec<Vec<String>>>("headers"), Some(vec![vec!["a".to_string(), "b".to_string()]]));
        assert_eq!(d.get::<String>("missing"), None);
    });
    let err = run(&eng, "throw new TypeError('boom')").unwrap_err();
    assert!(err.starts_with("TypeError: boom"), "{err}");
    assert_eq!(run(&eng, "throw 5").unwrap_err(), "Uncaught 5");
    // A syntax error fails at compile time and leaves the engine usable.
    assert!(run(&eng, "var = ;").is_err());
    assert_eq!(run(&eng, "1+1"), Ok(()));
}

#[test]
fn qv8_2_source_rejects_nul_and_jobs_drain() {
    assert!(Source::new("a\0b", "t").is_none());
    let eng = bare();
    run(&eng, "var hit = 0; Promise.resolve().then(function () { hit = 1; });").unwrap();
    assert!(eng.is_job_pending());
    eng.run_all_jobs();
    assert!(!eng.is_job_pending());
    eng.with(|ctx| assert_eq!(js_engine::eval_sloppy::<i32>(&ctx, "t", "hit").ok(), Some(1)));
}
