// QM1k1n6 (REGRESSIONS.md R46): the two main-thread stalls fixed with the frame-budget
// design. The live effect (video painted per second, idle turns per second) is measured
// in a nested window with SCRATCHY_FRAME_STATS; these tests pin the code facts it rests on.
#[allow(unused_imports)]
use super::*;

/// `App::pump_net` runs on every event-loop turn and may only touch the event-loop proxy
/// (and so drop a clone, which wakes the loop) when a job actually starts. That rests on
/// `NetScheduler::pump` never calling its starter for an empty queue.
#[test]
fn qm1k1n6_pump_with_nothing_queued_starts_nothing() {
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let mut calls = 0;
    for _ in 0..1000 {
        sched.pump(&mut |_| calls += 1);
    }
    assert_eq!(calls, 0);
}

/// mpv's software render must not sleep until the frame's target time on the main thread.
#[cfg(feature = "media")]
#[test]
fn qm1k1n6_sw_render_does_not_block_for_target_time() {
    let block = crate::media_mpv::sw_render_param_ints();
    assert_eq!(block, vec![(crate::media_mpv::RP_BLOCK_FOR_TARGET_TIME, 0)]);
    assert_eq!(crate::media_mpv::RP_BLOCK_FOR_TARGET_TIME, 12, "MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME in mpv/render.h");
}
