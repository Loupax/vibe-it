//! QCSS2: `CSS.supports()` / `CSS.escape()` and the `CSS` namespace.
use super::*;

fn js(code: &str) -> Vec<String> {
    run_in_html("<html><body></body></html>", code)
}

const CONDS: &[&str] = &[
    "(display: flex)",
    "(display: nonsense)",
    "(nonsense: 1)",
    "(--custom: anything)",
    "(-webkit-box-sizing: border-box)",
    "(-moz-box-sizing: border-box)",
    "(-webkit-nonsense: 1)",
    "(color: red !important)",
    "not (display: flex)",
    "not (display: nonsense)",
    "(display: flex) and (color: red)",
    "(display: flex) and (color: nonsense)",
    "(display: nonsense) or (color: red)",
    "(display: flex) and (color: red) or (width: 1px)",
    "((display: flex) or (nonsense: 1)) and (color: red)",
    "selector(a > b)",
    "selector(:is(a, b))",
    "selector(a >)",
    "not selector(a >)",
    "(selector(a b)) and (display: block)",
    "font-format(woff2)",
    "font-format(nonsense)",
    "font-tech(variations)",
    "unknown(x)",
    "(display: flex) garbage",
];

fn sheet_says(cond: &str) -> bool {
    !parse_stylesheet(&format!("@supports {cond} {{ a {{ color: red }} }}")).is_empty()
}

#[test]
fn qcss2_agrees_with_at_supports() {
    for c in CONDS {
        assert_eq!(css::supports_condition_text(c), sheet_says(c), "CSS.supports vs @supports on `{c}`");
    }
}

#[test]
fn qcss2_js_matches_rust() {
    let list = CONDS.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(",");
    let log = js(&format!("console.log([{list}].map(c => CSS.supports(c) ? 1 : 0).join(''));"));
    let want: String = CONDS.iter().map(|c| if css::supports_condition_text(c) { '1' } else { '0' }).collect();
    assert_eq!(log, [format!("log:{want}")], "{log:?}");
}

#[test]
fn qcss2_condition_results() {
    assert!(css::supports_condition_text("(display: flex)"));
    assert!(css::supports_condition_text("display: flex"));
    assert!(!css::supports_condition_text("(display: nonsense)"));
    assert!(css::supports_condition_text("(--x: 1)"));
    assert!(css::supports_condition_text("(-webkit-box-sizing: border-box)"));
    assert!(css::supports_condition_text("selector(a > b)"));
    assert!(!css::supports_condition_text("selector(a >)"));
    assert!(css::supports_condition_text("font-format(woff2)"));
    assert!(!css::supports_condition_text("font-format(nonsense)"));
    assert!(!css::supports_condition_text(""));
    assert!(!css::supports_condition_text("(display: flex) garbage"));
}

#[test]
fn qcss2_property_value() {
    assert!(css::supports_property_value("display", "flex"));
    assert!(css::supports_property_value("DISPLAY", "flex"));
    assert!(!css::supports_property_value("display", "nonsense"));
    assert!(!css::supports_property_value("nonsense", "1"));
    assert!(css::supports_property_value("--foo", ""));
    assert!(css::supports_property_value("--foo", "anything at all"));
    assert!(css::supports_property_value("-webkit-box-sizing", "border-box"));
    assert!(!css::supports_property_value("color", "red !important"));
    assert!(!css::supports_property_value("color", "red) or (display: flex"));
    assert!(!css::supports_property_value("display: flex", "x"));
    assert!(!css::supports_property_value("", "x"));
}

#[test]
fn qcss2_namespace_shape() {
    let log = js("console.log(typeof CSS, Object.prototype.toString.call(CSS), typeof CSS.supports, typeof CSS.escape, \
         CSS.supports.length, CSS.escape.length, CSS.supports.name, Object.getOwnPropertyDescriptor(globalThis, 'CSS').enumerable, \
         Object.getPrototypeOf(CSS) === Object.prototype, String(CSS.supports('(display: flex)')), String(CSS.supports('display', 'flex')), \
         String(CSS.supports('display', 'nonsense')), String(CSS.supports('--a', 'b')));\
         try { CSS.supports(); } catch (e) { console.log(e.constructor.name); } try { new CSS(); } catch (e) { console.log(e.constructor.name); }");
    assert_eq!(log, ["log:object [object CSS] function function 1 1 supports false true true true false true", "log:TypeError", "log:TypeError"], "{log:?}");
}

#[test]
fn qcss2_escape() {
    let cases: &[(&str, &str)] = &[
        ("abc", "abc"),
        ("a b", "a\\\\ b"),
        ("1a", "\\\\31 a"),
        ("-1a", "-\\\\31 a"),
        ("-", "\\\\-"),
        ("--a", "--a"),
        ("a.b#c", "a\\\\.b\\\\#c"),
        ("\\u0000", "\\uFFFD"),
        ("\\u0001", "\\\\1 "),
        ("é_x", "é_x"),
    ];
    for (i, o) in cases {
        let log = js(&format!("console.log(CSS.escape('{i}') === '{o}' ? 'ok' : 'bad:' + CSS.escape('{i}'));"));
        assert_eq!(log, ["log:ok"], "CSS.escape({i:?}) want {o:?}: {log:?}");
    }
}
