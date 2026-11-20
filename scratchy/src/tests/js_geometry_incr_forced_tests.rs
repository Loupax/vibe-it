// QINC1i: the js_geometry suite (rects, offsets, scroll, Range / text rects) a second time with script
// forced layouts through the incremental engine (`incr_engine::forced_on` is on for threads whose name
// contains `incr_forced`), each update checked against the from-scratch layout.
include!("js_geometry_tests.rs");
