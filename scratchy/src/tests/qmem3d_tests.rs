// QMEM3d: malloc_trim hook seams: once per navigation, once per idle period, never with video.
#[allow(unused_imports)]
use super::*;
use crate::mem_trim::{requests, IdleTrim, IDLE_AFTER};
use std::time::Duration;

#[test]
fn qmem3d_idle_trims_once_per_period() {
    let mut g = IdleTrim::default();
    let old = Some(IDLE_AFTER + Duration::from_secs(1));
    assert!(!g.should_trim(Some(Duration::from_secs(1)), None, false), "recent input");
    assert!(g.should_trim(old, None, false));
    assert!(!g.should_trim(old, None, false), "at most once per idle period");
    g.activity();
    assert!(g.should_trim(old, old, false), "new period after activity");
}

#[test]
fn qmem3d_no_idle_trim_while_video() {
    let mut g = IdleTrim::default();
    let old = Some(IDLE_AFTER + Duration::from_secs(1));
    assert!(!g.should_trim(old, old, true), "video box on screen");
    assert!(!g.should_trim(old, Some(Duration::from_millis(100)), false), "frame just presented");
    assert!(g.should_trim(old, old, false));
}

/// Measurement (run with --ignored --nocapture): trim duration and RSS drop on a fragmented heap.
#[test]
#[ignore]
fn qmem3d_measure_trim_on_big_heap() {
    fn rss() -> u64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        let l = s.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
        l.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024
    }
    let mut v: Vec<Vec<u8>> = (0..800_000).map(|i| vec![i as u8; 500]).collect();
    let keep: Vec<Vec<u8>> = v.drain(..).enumerate().filter(|(i, _)| i % 4 == 0).map(|(_, b)| b).collect();
    let before = rss();
    let took = crate::mem_trim::trim_now();
    eprintln!("QMEM3d: rss {before} MB -> {} MB, trim took {took:?}, kept {}", rss(), keep.len());
}

#[test]
fn qmem3d_trim_now_runs() {
    let _ = crate::mem_trim::trim_now();
}

#[test]
fn qmem3d_navigation_requests_trim() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("http://127.0.0.1:9/a").unwrap());
    let before = requests(); // the counter is global: other tests may add, never remove
    app.fetch_url(Url::parse("http://127.0.0.1:9/b").unwrap());
    assert!(requests() > before, "navigation requests a trim");
}
