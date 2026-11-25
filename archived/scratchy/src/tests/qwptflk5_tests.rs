//! QWPTFLK5: a flaky subtest's failure message must reach the gate line, so the next CI flake is diagnosable.
use crate::wpt_th_run::*;

fn run(json: &str) -> FileResult {
    parse_child_json(json).expect("child json")
}

#[test]
fn a_regression_line_carries_the_failure_message() {
    let e = run(r#"{"harness":"OK","message":null,"tests":[["upload","PASS",null],["stream","FAIL","assert_unreached"]]}"#);
    let a = run(r#"{"harness":"OK","message":null,"tests":[["upload","FAIL","promise_test: Unhandled rejection with value: TypeError: network error"],["stream","FAIL","other"]]}"#);
    match compare(&e, &a) {
        Verdict::Regression(m) => {
            assert!(m.starts_with("subtest was passing, now FAIL: upload ("), "{m}");
            assert!(m.contains("TypeError: network error"), "{m}");
        }
        _ => panic!("expected a regression"),
    }
}

#[test]
fn messages_are_never_compared() {
    let a = run(r#"{"harness":"OK","message":null,"tests":[["s","FAIL","one"]]}"#);
    let b = run(r#"{"harness":"OK","message":null,"tests":[["s","FAIL","two"]]}"#);
    assert_eq!(a, b);
    assert!(matches!(compare(&a, &b), Verdict::Same));
}

#[test]
fn a_long_multiline_message_is_cut_to_one_line() {
    let long = "x\\n".repeat(400);
    let a = run(&format!(r#"{{"harness":"OK","message":null,"tests":[["p","PASS",null],["q","FAIL","{long}"]]}}"#));
    let e = run(r#"{"harness":"OK","message":null,"tests":[["p","PASS",null],["q","PASS",null]]}"#);
    match compare(&e, &a) {
        Verdict::Regression(m) => {
            assert!(!m.contains('\n'), "{m}");
            assert!(m.chars().count() < 400, "{}", m.len());
        }
        _ => panic!("expected a regression"),
    }
}
