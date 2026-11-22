// QINC1m: the old pipeline is gone. The incremental engine is the only App frame and script forced
// layout path; `SCRATCHY_INCR` (the old escape hatch) and the test-matrix knobs that selected the old
// path no longer exist, so setting them changes nothing. The from-scratch layout stays as the engine's
// first-layout / fallback path and as the oracle of check mode (`SCRATCHY_INCR_CHECK=1`, always on
// in tests).
#[allow(unused_imports)]
use super::*;
use crate::perf::{incr_get, Incr};

/// Every `.rs` file under `src/` outside `src/tests/`, as (path, text).
fn sources() -> Vec<(std::path::PathBuf, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("src dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                if p != root.join("tests") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push((p.clone(), std::fs::read_to_string(&p).expect("source")));
            }
        }
    }
    out
}

#[test]
fn qinc1m_no_code_reads_the_removed_flags() {
    let src = sources();
    assert!(src.len() > 50, "found the sources: {}", src.len());
    // `SCRATCHY_INCR=0` cannot select anything: nothing reads the variable (nor the test knobs).
    for name in ["\"SCRATCHY_INCR\"", "\"SCRATCHY_TEST_INCR_FORCED\"", "\"SCRATCHY_SHADOW\""] {
        let hits: Vec<_> = src.iter().filter(|(_, t)| t.contains(name)).map(|(p, _)| p.display().to_string()).collect();
        assert!(hits.is_empty(), "{name} is still read in {hits:?}");
    }
    // The debug option stays.
    assert!(src.iter().any(|(_, t)| t.contains("\"SCRATCHY_INCR_CHECK\"")), "SCRATCHY_INCR_CHECK is kept");
    // The old App frame path and the hover shortcut that only it used are gone.
    assert!(!src.iter().any(|(p, _)| p.ends_with("hover_patch.rs")));
}

#[test]
fn qinc1m_check_mode_is_on_in_tests() {
    assert!(crate::incr_style::checking(), "tests run every engine update against the from-scratch oracle");
}

#[test]
fn qinc1m_forced_layouts_always_use_the_engine() {
    let html = "<html><head><style>.big{height:50px}</style></head><body><div id=a></div><div id=b>after</div></body></html>";
    let code = "var a = document.getElementById('a'), out = [];\
         for (var i = 0; i < 3; i++) { a.className = i % 2 ? 'big' : ''; out.push(document.getElementById('b').getBoundingClientRect().y); }\
         console.log(out.join(','));";
    let (e0, m0) = (incr_get(Incr::ForcedEngine), incr_get(Incr::CheckMismatch));
    let log = run_in_html(html, code);
    assert_eq!(log.len(), 1, "{log:?}");
    let ys: Vec<f32> = log[0].trim_start_matches("log:").split(',').map(|v| v.parse().unwrap()).collect();
    assert_eq!(ys.len(), 3);
    assert!(ys[1] > ys[0] + 40.0 && ys[2] == ys[0], "the class writes move #b: {ys:?}");
    assert!(incr_get(Incr::ForcedEngine) - e0 >= 3, "every read went through the engine");
    assert_eq!(incr_get(Incr::CheckMismatch), m0, "the engine agrees with the from-scratch oracle");
}

#[test]
fn qinc1m_check_mode_still_reports_a_difference() {
    // The oracle comparison has teeth: a text change the write log never describes (reconcile off)
    // leaves the kept trees stale, and check mode reports it instead of keeping it silently.
    crate::incr_style::reset();
    crate::incr_box::reset();
    let _ = crate::incr_log::drain();
    set_interaction(&InteractionState::default());
    let saved = crate::incr_style::set_seam(crate::incr_style::Seam { reconcile: false, panic: false });
    let m0 = incr_get(Incr::CheckMismatch);
    let doc = parse_html("<html><body><p id=p>x</p><p>y</p></body></html>");
    let _ = build_layout_tree_sized(&doc, 800.0, 600.0);
    let t = by_id(&doc, "p").children.borrow()[0].clone();
    if let NodeData::Text { contents } = &t.data {
        *contents.borrow_mut() = "unlogged".into();
    }
    let _ = build_layout_tree_sized(&doc, 800.0, 600.0);
    let found = crate::incr_style::take_mismatches();
    crate::incr_style::set_seam(saved);
    crate::incr_style::reset();
    crate::incr_box::reset();
    assert!(found.iter().any(|x| x.contains("unlogged")), "{found:?}");
    assert!(incr_get(Incr::CheckMismatch) > m0);
}

#[test]
fn qinc1m_app_frames_always_use_the_engine() {
    let Ok(event_loop) = test_event_loop() else { return }; // no display available
    crate::anim_eval::set_app_clock_override(Some(0.0));
    set_interaction(&InteractionState::default());
    let mut app = App::new_static(parse_html("<html><body><p id=p>x</p></body></html>"), event_loop.create_proxy(), None);
    app.rebuild_layout(600, 400, false);
    crate::anim_eval::set_app_clock_override(None);
    assert!(app.live_ver != 0, "the engine adopted the first layout");
    assert!(app.cached.as_ref().is_some_and(crate::incr_layout::owns), "the App shows the engine's tree");
}
