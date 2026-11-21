// QEL5 (HTML timer initialisation steps): timer nesting level and the 4 ms clamp (js.rs TIMERS_PRELUDE).
#[allow(unused_imports)]
use super::*;

/// A timer nested more than 5 deep is clamped to 4 ms; levels 1-5 are not. Timestamps are the manual
/// clock's (each run at its due time).
#[test]
fn qel5_timeout_clamp_only_above_nesting_5() {
    let log = run_with_timers(
        "var ts = []; (function step(n) { ts.push(__scratchy_now()); if (n < 8) setTimeout(step, 0, n + 1); else console.log(ts.join(',')); })(0);",
    );
    // the script's own call is level 0; timeouts 1..5 run at 0, level 6 waits 4 ms, then 4 each
    assert_eq!(log, vec!["log:0,0,0,0,0,0,4,8,12"], "{log:?}");
}

/// A 2 ms delay is kept at levels 1-5 and raised to 4 (not increased by 4) at level 6+.
#[test]
fn qel5_clamp_is_a_floor_not_an_addition() {
    let log = run_with_timers(
        "var ts = []; (function step(n) { ts.push(__scratchy_now()); if (n < 8) setTimeout(step, 2, n + 1); else console.log(ts.join(',')); })(0);",
    );
    assert_eq!(log, vec!["log:0,2,4,6,8,10,14,18,22"], "{log:?}");
}

/// An interval is not clamped for its first five runs, then keeps an interval >= 4 ms.
#[test]
fn qel5_interval_clamps_after_nesting() {
    let log = run_with_timers(
        "var ts = [], id = setInterval(function () { ts.push(__scratchy_now()); if (ts.length == 8) { clearInterval(id); console.log(ts.join(',')); } }, 0);",
    );
    assert_eq!(log, vec!["log:0,0,0,0,0,4,8,12"], "{log:?}");
}

/// A timer set outside a callback starts at level 1 whatever chain ran before.
#[test]
fn qel5_top_level_timer_starts_at_level_one() {
    let log = run_with_timers(
        "(function step(n) { if (n < 7) setTimeout(step, 0, n + 1); else setTimeout(function () { console.log('chain', __scratchy_now()); }, 0); })(0);\
         setTimeout(function () { console.log('flat', __scratchy_now()); }, 0);",
    );
    assert_eq!(log[0], "log:flat 0", "{log:?}");
    assert_eq!(log[1], "log:chain 12", "{log:?}");
}
