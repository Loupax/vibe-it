#[allow(unused_imports)]
use super::*;

// ---------- QWPTBUD: the testharness runner's own callback budget ----------

const BUDGET_MS: u64 = 60_000;

#[test]
fn budgets_file_parses_path_ms_and_skips_comments_and_bad_lines() {
    let m = crate::wpt_th_run::parse_budgets("# c\n\na/b.html 60000 reason words\nbad.html x\nc.html\nd.html 5000\n");
    assert_eq!(m.len(), 2);
    assert_eq!(m["a/b.html"], 60_000);
    assert_eq!(m["d.html"], 5_000);
}

#[test]
fn unlisted_files_keep_the_default_budget_and_timeout() {
    let (args, t) = crate::wpt_th_run::budget_for("not/listed.html", std::time::Duration::from_secs(30));
    assert!(args.is_empty());
    assert_eq!(t.as_secs(), 30);
    assert_eq!(crate::perf::DEFAULT_BUDGET_MS, 500, "the App's interactive budget is unchanged");
}

fn run_busy(ms: u64, budget: u64) -> Option<String> {
    crate::perf::set_budget_ms(budget);
    let doc = parse_html("<html><body></body></html>");
    let code = format!("setTimeout(function(){{ var t=Date.now(); while(Date.now()-t<{ms}){{}} globalThis.out='done'; }}, 0);");
    let e = start_scripts(&doc, &[src(&code)], Rc::new(|_, _| {})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    e.run_due();
    let r = e.eval_value::<String>("t", "typeof out === 'undefined' ? 'cut' : out");
    crate::perf::set_budget_ms(crate::perf::DEFAULT_BUDGET_MS);
    r
}

#[test]
fn a_long_single_callback_finishes_under_the_runner_budget_and_is_a_cut_under_a_tiny_one() {
    assert_eq!(run_busy(700, BUDGET_MS).as_deref(), Some("done"));
    assert_ne!(run_busy(700, 100).as_deref(), Some("done"));
}
