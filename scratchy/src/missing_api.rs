//! QMAPI1 (R64 section 5): `scratchy --missing-api-report [--socket S] [--seconds N] URL...`.
//!
//! Report mode only (a thread-local flag set by `main`; normal mode installs nothing, pinned by a test):
//! * `missing_api_prelude.js` puts one forwarding Proxy at the end of the DOM prototype chains and records
//!   misses (`get` / `has`) whose name is a standard member per `missing_api_idl.json` (generated from WPT's
//!   `interfaces/*.idl` by `tools/missing-api/gen-idl.py`); answers the page sees are unchanged.
//! * the log hook groups V8 error messages that name a missing API (`x.y is not a function`, `Cannot read
//!   properties of undefined (reading 'y')`, `Foo is not defined`) and counts the preludes' existing
//!   `not implemented` stub lines.
//!
//! The page is loaded and pumped by `perf::perf_report` (same recorder, same JSON); this mode adds the
//! `missing_api` member, so JS time with and without the report is one `--perf-report` apart.

use crate::perf::json_str;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static OUT: RefCell<String> = const { RefCell::new(String::new()) };
    static ERRORS: RefCell<BTreeMap<(String, String), u64>> = const { RefCell::new(BTreeMap::new()) };
    static STUBS: RefCell<BTreeMap<String, u64>> = const { RefCell::new(BTreeMap::new()) };
}

const IDL_JSON: &str = include_str!("missing_api_idl.json");
const PRELUDE: &str = include_str!("missing_api_prelude.js");

pub(crate) fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

pub(crate) fn set_enabled(on: bool) {
    ENABLED.with(|e| e.set(on));
}

/// Forgets the previous page's error and stub groups (one process can report several pages).
pub(crate) fn reset() {
    ERRORS.with(|e| e.borrow_mut().clear());
    STUBS.with(|e| e.borrow_mut().clear());
}

/// Called by `startup_engine` after the preludes, before any page script; a no-op unless report mode.
pub(crate) fn install(engine: &crate::js::JsEngine) {
    if !enabled() {
        return;
    }
    engine.eng.with(|ctx| {
        crate::js_engine::native!(&ctx, "__mapi_out", |s: String| OUT.with(|o| *o.borrow_mut() = s));
    });
    engine.eval("missing_api", &PRELUDE.replacen("__MAPI_IDL__", IDL_JSON.trim(), 1));
}

/// The member part of the report, read from the page's engine (`{"members":[..],"ignored":n,..}`).
fn dump(engine: &crate::js::JsEngine) -> String {
    OUT.with(|o| o.borrow_mut().clear());
    engine.eval("missing_api", "__mapi_out(__mapi_dump())");
    OUT.with(|o| o.borrow().clone())
}

/// Groups one logged line (`level` is the page log level). Cheap; only called in report mode.
pub(crate) fn note_log(level: &str, text: &str) {
    if !enabled() {
        return;
    }
    if let Some(i) = text.find("not implemented") {
        let what: String = text[i..].chars().take(100).collect();
        STUBS.with(|s| *s.borrow_mut().entry(what).or_default() += 1);
    } else if level == "error" {
        if let Some((kind, name)) = classify_error(text) {
            ERRORS.with(|e| *e.borrow_mut().entry((kind.to_string(), name)).or_default() += 1);
        }
    }
}

/// `(kind, member)` for a V8 message that names a missing API, else None.
pub(crate) fn classify_error(text: &str) -> Option<(&'static str, String)> {
    let ident = |s: &str| s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') && !s.is_empty();
    if let Some(i) = text.find(" is not a function") {
        let head = text[..i].rsplit(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("");
        let last = head.rsplit('.').next().unwrap_or(head).trim_end_matches(')');
        if ident(last) {
            return Some(("not_a_function", last.to_string()));
        }
    }
    if let Some(i) = text.find("Cannot read properties of ") {
        let rest = &text[i..];
        if let (Some(a), Some(b)) = (rest.find("(reading '"), rest.rfind("')")) {
            let name = &rest[a + 10..b];
            if a + 10 <= b && !name.is_empty() {
                return Some(("read_of_nullish", name.to_string()));
            }
        }
    }
    if let Some(i) = text.find(" is not defined") {
        let head = text[..i].rsplit(|c: char| c.is_whitespace()).next().unwrap_or("");
        if ident(head) {
            return Some(("not_defined", head.to_string()));
        }
    }
    None
}

/// The `"missing_api"` member of the page's report: members (from the proxy), grouped errors, stubs.
pub(crate) fn finish(engine: &crate::js::JsEngine) -> String {
    let d = dump(engine);
    let errors: Vec<String> = ERRORS.with(|e| {
        let mut v: Vec<_> = e.borrow().iter().map(|((k, n), c)| (*c, k.clone(), n.clone())).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
        v.iter().map(|(c, k, n)| format!("{{\"kind\":{},\"name\":{},\"count\":{c}}}", json_str(k), json_str(n))).collect()
    });
    let stubs: Vec<String> = STUBS.with(|s| {
        let mut v: Vec<_> = s.borrow().iter().map(|(k, c)| (*c, k.clone())).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        v.iter().map(|(c, k)| format!("{{\"stub\":{},\"count\":{c}}}", json_str(k))).collect()
    });
    let d = if d.is_empty() { "{\"members\":[],\"ignored\":0,\"dropped\":0}".to_string() } else { d };
    // `d` is an object text ending in `}`: append the Rust-side groups to it.
    format!("{},\"errors\":[{}],\"stubs\":[{}]}}", d.trim_end().trim_end_matches('}'), errors.join(","), stubs.join(","))
}

/// `scratchy --missing-api-report [--socket S] [--seconds N] URL...` (flags as `--perf-report`).
pub(crate) fn main(args: &[String]) -> i32 {
    set_enabled(true);
    crate::perf::perf_report_main(args)
}
