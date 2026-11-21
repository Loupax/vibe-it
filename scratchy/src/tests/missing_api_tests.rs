//! QMAPI1: the missing-API report mode (src/missing_api.rs).

use crate::missing_api;
use std::cell::RefCell;
use std::rc::Rc;

/// Resets the per-thread report-mode state when a test ends (even on a panic).
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        missing_api::set_enabled(false);
        missing_api::reset();
    }
}

/// Runs the inline scripts of `html`; returns the lines logged (`level: text`) and the live engine.
fn run(html: &str) -> (Vec<String>, Option<crate::js::JsEngine>) {
    let lines: Rc<RefCell<Vec<String>>> = Default::default();
    let l = lines.clone();
    let log: crate::js::LogFn = Rc::new(move |level, text| {
        missing_api::note_log(level, text);
        l.borrow_mut().push(format!("{level}: {text}"));
    });
    let doc = crate::dom::parse_html(html);
    let scripts = crate::js::inline_scripts(&doc);
    let engine = crate::js::start_scripts(&doc, &scripts, log);
    let out = lines.borrow().clone();
    (out, engine)
}

#[test]
fn qmapi1_normal_mode_installs_nothing() {
    let _g = Guard;
    missing_api::set_enabled(false);
    let (lines, _e) = run(
        "<script>console.error('R ' + (Object.getPrototypeOf(EventTarget.prototype) === Object.prototype) + ' ' + typeof __mapi_dump + ' ' + typeof __mapi_out);\
         document.body.nonsense; 'animate' in document.body;</script>",
    );
    assert!(lines.iter().any(|l| l.contains("R true undefined undefined")), "{lines:?}");
}

#[test]
fn qmapi1_records_misses_and_keeps_answers() {
    let _g = Guard;
    missing_api::set_enabled(true);
    let (lines, e) = run(
        "<script>var el = document.body;\
         var a = el.__reactFiber$x;\
         var b = el.zzzStandardless;\
         var has1 = 'zzzStandardless' in el;\
         console.error('A ' + a + ' ' + b + ' ' + has1 + ' ' + ('hasOwnProperty' in el) + ' ' + ('IntersectionObserver' in window));\
         var c = el.getAnimations; var d = el.getAnimations; var k = typeof navigator.userActivation;\
         var w = typeof window.documentPictureInPicture; var undeclared = typeof QMapiNoSuchGlobal;\
         console.error('B ' + typeof c + ' ' + k + ' ' + w + ' ' + undeclared);</script>",
    );
    assert!(lines.iter().any(|l| l.contains("A undefined undefined false true")), "answers unchanged: {lines:?}");
    let json = missing_api::finish(&e.expect("engine"));
    // Non-standard names (expandos) are counted, not reported.
    assert!(!json.contains("__reactFiber") && !json.contains("zzzStandardless"), "{json}");
    assert!(json.contains("\"ignored\":"), "{json}");
    // Standard names that scratchy lacks are reported with their receiver interface and hit count.
    assert!(json.contains("\"iface\":\"HTMLBodyElement\",\"kind\":\"get\",\"name\":\"getAnimations\",\"hits\":2"), "{json}");
    assert!(json.contains("\"iface\":\"Navigator\",\"kind\":\"get\",\"name\":\"userActivation\""), "{json}");
}

#[test]
fn qmapi1_has_probe_of_a_standard_global_is_recorded() {
    let _g = Guard;
    missing_api::set_enabled(true);
    // `Highlight` is a standard interface scratchy does not define (checked below); the probe must be seen.
    let (lines, e) = run("<script>console.error('H ' + ('Highlight' in window) + ' ' + (typeof Highlight));</script>");
    let json = missing_api::finish(&e.expect("engine"));
    assert!(lines.iter().any(|l| l.contains("H false undefined")), "{lines:?}");
    assert!(json.contains("\"kind\":\"has\",\"name\":\"Highlight\""), "{json}");
}

#[test]
fn qmapi1_error_messages_are_grouped_by_member() {
    assert_eq!(missing_api::classify_error("error: scratchy: js: TypeError: el.animate is not a function"), Some(("not_a_function", "animate".into())));
    assert_eq!(
        missing_api::classify_error("TypeError: Cannot read properties of undefined (reading 'userActivation')"),
        Some(("read_of_nullish", "userActivation".into()))
    );
    assert_eq!(missing_api::classify_error("ReferenceError: ResizeObserver is not defined"), Some(("not_defined", "ResizeObserver".into())));
    assert_eq!(missing_api::classify_error("something else"), None);
}

#[test]
fn qmapi1_stubs_and_errors_reach_the_report() {
    let _g = Guard;
    missing_api::set_enabled(true);
    let (_l, e) = run("<script>try { crypto.subtle.digest('SHA-1', new Uint8Array(1)); } catch (x) {} nosuchfn1();</script>");
    let json = missing_api::finish(&e.expect("engine"));
    assert!(json.contains("not implemented: SubtleCrypto.digest"), "{json}");
    assert!(json.contains("\"name\":\"nosuchfn1\""), "{json}");
}

const QMAPI2_CSS: &str = "a { frobnicate: 1; color: red } b { frobnicate: 2 } li::zzz-thing { color: blue } p:nth-foo(2n) { color: red }\
    @font-face { font-family: x } @property --y { syntax: '*'; } @supports (frobnicate: 1) { a { color: red } }\
    @supports (display: grid) { a { color: red } } @media (hover: hover) and (min-width: 10px) { a { color: red } }\
    @media (min-width: 10px) { a:zork { color: red } }";

#[test]
fn qmapi2_normal_mode_records_nothing() {
    let _g = Guard;
    missing_api::set_enabled(false);
    let rules = crate::css::parse_stylesheet(QMAPI2_CSS);
    for r in &rules {
        let _ = r.decls();
    }
    assert_eq!(missing_api::css_json(), "[]");
}

#[test]
fn qmapi2_counts_css_gaps_and_keeps_the_rules() {
    let _g = Guard;
    let normal = crate::css::parse_stylesheet(QMAPI2_CSS).len();
    missing_api::set_enabled(true);
    let rules = crate::css::parse_stylesheet(QMAPI2_CSS);
    assert_eq!(rules.len(), normal, "report mode must not change the parsed rules");
    let json = missing_api::css_json();
    assert!(json.contains("\"kind\":\"declaration\",\"name\":\"frobnicate\",\"count\":2,\"example\":\"frobnicate: 1\""), "{json}");
    assert!(json.contains("\"kind\":\"pseudo_element\",\"name\":\"::zzz-thing\""), "{json}");
    assert!(json.contains("\"name\":\":zork\",\"count\":1,\"example\":\"a:zork\""), "{json}");
    assert!(json.contains("\"kind\":\"pseudo_class\",\"name\":\":nth-foo()\""), "{json}");
    assert!(json.contains("\"kind\":\"at_rule\",\"name\":\"@font-face\""), "{json}");
    assert!(json.contains("\"kind\":\"at_rule\",\"name\":\"@property\""), "{json}");
    assert!(json.contains("\"kind\":\"supports_false\",\"name\":\"(frobnicate: 1)\""), "{json}");
    assert!(!json.contains("display: grid"), "{json}");
    assert!(json.contains("\"kind\":\"media_feature\",\"name\":\"hover\""), "{json}");
    assert!(json.starts_with("[{\"kind\":\"declaration\""), "sorted by hits: {json}");
}
