//! QAPI6c: check mode's "adopted element style reads vh: build true vs engine false" (YouTube results while
//! the window settled). A `display: none` element is resolved ONCE in a from-scratch build, so its match runs
//! inside that resolve, and a candidate rule under a height media query (`(min-height: ..)`) set the
//! resolve's viewport-height read flag; the engine takes the match from the build's memo before its own
//! resolve and saw no read. The media read now stays out of the per-element flag (the engine re-matches in
//! full on a height change when some rule's media reads the height), but still reaches the build's global
//! flag. Each test runs the engine in check mode with the mismatch panic off and asserts an empty report.
use super::*;
use crate::incr_log;
use crate::incr_style::{self, Seam, Stats};

struct Checked(Seam, bool);

impl Checked {
    fn new() -> Checked {
        incr_style::reset();
        let _ = incr_log::drain();
        set_interaction(&InteractionState::default());
        let seam = incr_style::set_seam(Seam { shadow: true, reconcile: true, panic: false });
        Checked(seam, incr_style::set_adopt_verify(true))
    }
}

impl Drop for Checked {
    fn drop(&mut self) {
        incr_style::set_seam(self.0);
        incr_style::set_adopt_verify(self.1);
        let _ = incr_style::take_mismatches();
        set_interaction(&InteractionState::default());
        incr_style::reset();
    }
}

fn build_vh(doc: &Handle, vh: f32) -> (Stats, Vec<String>) {
    let _ = build_layout_tree_sized(doc, 800.0, vh);
    (incr_style::with_engine(|e| e.expect("shadow engine").stats.clone()), incr_style::take_mismatches())
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"))
}

fn color_of(doc: &Handle, id: &str) -> TextColor {
    let h = by_id(doc, id);
    incr_style::with_engine(|e| e.unwrap().entry(&h).expect("styled").style().unwrap().color)
}

fn height_of(doc: &Handle, id: &str) -> Option<BoxLength> {
    let h = by_id(doc, id);
    incr_style::with_engine(|e| e.unwrap().entry(&h).expect("styled").style().unwrap().height)
}

const CSS: &str = "#n{display:none} #c{display:block;height:10vh} \
                   @media (min-height: 650px) { #n, #s{color:red} }";
const BODY: &str = "<div id=n>hidden</div><div id=s>shown</div><div id=c>vh</div>";

/// The YouTube report: the restart (first build) adopts the hidden element's style without a flag
/// mismatch, and a height change across the media threshold (the window settling) keeps the engine
/// equal to the build: the media rule flips for both elements, the `10vh` element follows.
#[test]
fn qapi6c_hidden_element_under_height_media_adopts_without_vh_mismatch() {
    let _g = Checked::new();
    let doc = page(CSS, BODY);
    let (s, m) = build_vh(&doc, 600.0);
    assert!(s.full && s.adopted >= 5, "{s:?}");
    assert!(m.is_empty(), "restart: {m:?}");
    assert!(viewport_height_read(), "the media read still reaches the build's global flag");
    assert_eq!(color_of(&doc, "s"), TextColor::rgb(0, 0, 0));
    for vh in [700.0, 640.0, 312.0, 720.0] {
        let (s, m) = build_vh(&doc, vh);
        assert!(m.is_empty(), "height {vh}: {m:?} {s:?}");
        assert!(s.full, "a height media rule re-matches in full on a height change: {s:?}");
        let want = if vh >= 650.0 { TextColor::rgb(255, 0, 0) } else { TextColor::rgb(0, 0, 0) };
        assert_eq!(color_of(&doc, "s"), want, "height {vh}");
        assert_eq!(color_of(&doc, "n"), want, "hidden element, height {vh}");
        assert_eq!(height_of(&doc, "c"), Some(BoxLength::Px(vh / 10.0)), "height {vh}");
    }
}

/// Without a height media rule the hidden element's style reads nothing of the height; a `vh` length on
/// it still sets its own flag (a height change re-cascades it, nothing else).
#[test]
fn qapi6c_hidden_element_own_vh_keeps_its_flag() {
    let _g = Checked::new();
    let doc = page("#n{display:none;height:10vh} #s{color:blue}", "<div id=n>hidden</div><div id=s>shown</div>");
    let (s, m) = build_vh(&doc, 600.0);
    assert!(s.full && m.is_empty(), "{m:?} {s:?}");
    let (s, m) = build_vh(&doc, 700.0);
    assert!(m.is_empty(), "{m:?}");
    assert!(!s.full && s.recascaded == 1, "only #n re-cascades: {s:?}");
    assert_eq!(height_of(&doc, "n"), Some(BoxLength::Px(70.0)));
}

/// As `SCRATCHY_INCR=1` runs it (adopted styles not verified): the adopted flags and the full re-match
/// still give the from-scratch styles after height changes across the threshold.
#[test]
fn qapi6c_unverified_adoption_follows_height_changes() {
    let _g = Checked::new();
    let v = incr_style::set_adopt_verify(false);
    let doc = page(CSS, BODY);
    let (s, m) = build_vh(&doc, 700.0);
    assert!(s.full && s.resolved == 0 && m.is_empty(), "{m:?} {s:?}");
    for vh in [600.0, 680.0, 400.0] {
        let (s, m) = build_vh(&doc, vh);
        assert!(m.is_empty(), "height {vh}: {m:?} {s:?}");
        let want = if vh >= 650.0 { TextColor::rgb(255, 0, 0) } else { TextColor::rgb(0, 0, 0) };
        assert_eq!(color_of(&doc, "n"), want, "height {vh}");
        assert_eq!(height_of(&doc, "c"), Some(BoxLength::Px(vh / 10.0)), "height {vh}");
    }
    incr_style::set_adopt_verify(v);
}
