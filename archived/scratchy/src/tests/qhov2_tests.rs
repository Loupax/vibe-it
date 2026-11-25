//! QHOV2: the damage-rect repaint is not tied to `:hover` rules any more. Any engine update whose damage
//! is known (a JS class toggle, a text write, an animation frame) patches the buffer instead of painting the
//! page, and no kept copy of the page is needed: a buffer of age `a` misses the damage of the last `a - 1`
//! frames (`damage_plan`). `Sim` repeats the App's decision on 1..3 swap buffers and checks every presented
//! buffer against a full paint of the new tree.
use super::*;
use crate::incr_layout::Damage;

const W: u32 = 600;
const H: u32 = 900;

struct Live;

impl Drop for Live {
    fn drop(&mut self) {
        crate::anim_eval::set_app_clock_override(None);
        set_interaction(&InteractionState::default());
    }
}

/// An App on `html` with the page script `code` run, after its first frame, its engine damage reset.
/// `None` without a display.
fn live_app(html: &str, code: &str) -> Option<(App, Live)> {
    let proxy = test_event_loop().ok().map(|l| l.create_proxy())?;
    let live = Live;
    crate::anim_eval::set_app_clock_override(Some(0.0));
    set_interaction(&InteractionState::default());
    let mut app = App::new_static(parse_html(html), proxy, None);
    if !code.is_empty() {
        app.js = crate::js::start_scripts(&app.document, &[src(code)], Rc::new(|_, _| {}));
    }
    app.rebuild_layout(W, H, false);
    assert!(app.cached.as_ref().is_some_and(crate::incr_layout::owns), "the App shows the engine's tree");
    let _ = crate::incr_layout::take_damage();
    Some((app, live))
}

fn full_paint(app: &mut App) -> Vec<u32> {
    let mut px = vec![0x00FFFFFFu32; (W * H) as usize];
    let laid = app.cached.clone().expect("laid");
    if let Some(c) = laid.style.background_color {
        for p in px.iter_mut() {
            *p = crate::paint::blend(*p, c);
        }
    }
    paint(&laid, &mut app.font_system, &mut app.swash_cache, &mut app.shape_cache, &app.scroll_offsets, &mut px, W, H, 0.0, None);
    px
}

/// The swap buffers of a window and the damage history `App::redraw` keeps.
struct Sim {
    bufs: Vec<Vec<u32>>,
    /// The frame number each buffer was last presented in (`None` = never).
    last: Vec<Option<usize>>,
    frame: usize,
    hist: Vec<Damage>,
    fast: usize,
    full: usize,
}

impl Sim {
    fn new(n: usize) -> Sim {
        Sim { bufs: vec![vec![0; (W * H) as usize]; n], last: vec![None; n], frame: 0, hist: Vec::new(), fast: 0, full: 0 }
    }

    /// Presents the app's next frame (its tree is already updated): patches the buffer the way
    /// `redraw` does when the damage is known and the buffer's age allows it, else paints it whole;
    /// asserts the buffer equals a full paint. Returns whether it was a patch.
    fn present(&mut self, app: &mut App, what: &str) -> bool {
        let b = self.frame % self.bufs.len();
        let age = self.last[b].map_or(0, |l| self.frame - l);
        let laid = app.cached.clone().expect("laid");
        let damage = crate::incr_layout::take_damage();
        let want = full_paint(app);
        let plan = damage.as_ref().and_then(|d| crate::app::damage_plan(&self.hist, d, age));
        let rects = plan.and_then(|(m, _)| crate::app::damage_screen_rects(&m, &laid, (0.0, 0.0), (W, H)));
        let fast = rects.is_some();
        match rects {
            Some((rects, margin)) => {
                let mut scratch = vec![0u32; (W * H) as usize];
                for (l, t, r, bt) in rects {
                    crate::app::repaint_rect(
                        &laid, &mut app.font_system, &mut app.swash_cache, &mut app.shape_cache, &app.scroll_offsets,
                        &mut scratch, (W, H), (0.0, 0.0), None, None, (l, t, r, bt), margin,
                    );
                    for y in t..bt {
                        let (a, z) = ((y * W + l) as usize, (y * W + r) as usize);
                        self.bufs[b][a..z].copy_from_slice(&scratch[a..z]);
                    }
                }
            }
            None => self.bufs[b] = want.clone(),
        }
        // Known damage extends the chain (patched or painted whole); unknown damage restarts it.
        match damage {
            Some(d) => {
                self.hist.push(d);
                let extra = self.hist.len().saturating_sub(crate::app::DAMAGE_HISTORY);
                self.hist.drain(..extra);
            }
            None => self.hist.clear(),
        }
        let bad = self.bufs[b].iter().zip(&want).filter(|(a, b)| a != b).count();
        assert_eq!(bad, 0, "{what}: the patched buffer (age {age}, patch {fast}) differs from a full paint in {bad} pixels");
        self.last[b] = Some(self.frame);
        self.frame += 1;
        if fast {
            self.fast += 1;
        } else {
            self.full += 1;
        }
        fast
    }
}

fn js(app: &App, code: &str) {
    app.js.as_ref().expect("page script").eval("qhov2", code);
}

const PAGE: &str = "<html><head><style>
body{margin:0;font:14px sans-serif}
.row{height:30px;margin:6px 10px;background:#eee;border:1px solid #ccc}
.on{background:#9cf;border-color:#06c}
#fx{position:fixed;left:300px;top:20px;width:80px;height:40px;background:#ddd}
#fx.on{background:#fc9}
#t{margin:10px;width:300px}
</style></head><body>
<div class=row id=a>a</div><div class=row id=b>b</div><div class=row id=c>c</div><div class=row id=d>d</div>
<p id=t>some text here</p><div class=row id=e>e</div><div id=fx></div></body></html>";

const SCRIPT: &str = "function tg(id){document.getElementById(id).classList.toggle('on')}";

/// A page with no `:hover` rule at all: JS class toggles (what a mouseover handler does) are small
/// damage, and every buffer of a swap chain of 1 to 3 buffers equals a full paint.
#[test]
fn qhov2_js_class_toggle_hover_patches_the_buffer() {
    for nbuf in [1, 2, 3] {
        let Some((mut app, _l)) = live_app(PAGE, SCRIPT) else { return };
        assert_eq!(app.state_rules, 0, "the page has no state rules");
        let mut sim = Sim::new(nbuf);
        sim.present(&mut app, "first frame");
        let mut small = 0;
        for id in ["a", "b", "a", "c", "d", "c", "b", "e", "a", "d"] {
            js(&app, &format!("tg('{id}')"));
            app.rebuild_layout(W, H, false);
            small += usize::from(sim.present(&mut app, &format!("toggle #{id}, {nbuf} buffers")));
        }
        assert!(small >= 8, "{nbuf} buffers: most toggles patch (got {small} of 10; {} full)", sim.full);
    }
}

/// An idle redraw is a patch of nothing.
#[test]
fn qhov2_idle_redraw_paints_nothing() {
    let Some((mut app, _l)) = live_app(PAGE, SCRIPT) else { return };
    let mut sim = Sim::new(2);
    sim.present(&mut app, "first");
    sim.present(&mut app, "second");
    assert!(sim.present(&mut app, "idle"), "a frame with no change is a patch");
    assert!(sim.hist.last().is_some_and(|d| d.rects.is_empty()));
}

/// A small text write repaints only its line.
#[test]
fn qhov2_small_text_change() {
    let Some((mut app, _l)) = live_app(PAGE, SCRIPT) else { return };
    let mut sim = Sim::new(2);
    sim.present(&mut app, "first");
    sim.present(&mut app, "second");
    let mut small = 0;
    for t in ["other text", "some text here", "tiny", "a somewhat longer line of text"] {
        js(&app, &format!("document.getElementById('t').textContent = '{t}'"));
        app.rebuild_layout(W, H, false);
        small += usize::from(sim.present(&mut app, &format!("text {t}")));
    }
    assert!(small >= 3, "text writes patch ({small} of 4; {} full)", sim.full);
}

/// Animation frames: each tick changes the animated box's colour; the patched buffers equal full paints.
#[test]
fn qhov2_animation_frames() {
    let html = "<html><head><style>body{margin:0}@keyframes k{from{background:#f00}to{background:#00f}}\
                #an{height:40px;margin:10px;animation:k 1s linear}#p{margin:10px}</style></head>\
                <body><div id=an>a</div><p id=p>p</p></body></html>";
    let Some((mut app, _l)) = live_app(html, "") else { return };
    let mut sim = Sim::new(2);
    sim.present(&mut app, "first");
    let mut small = 0;
    for ms in [100.0, 200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0] {
        crate::anim_eval::set_app_clock_override(Some(ms));
        app.anim_tick(ms);
        app.rebuild_layout(W, H, false);
        small += usize::from(sim.present(&mut app, &format!("animation at {ms} ms")));
    }
    assert!(small >= 6, "animation frames patch ({small} of 8; {} full)", sim.full);
}

/// A fixed box in the path leaves the damage unknown: the frame is painted whole (and still right).
#[test]
fn qhov2_fixed_box_forces_a_full_paint() {
    let Some((mut app, _l)) = live_app(PAGE, SCRIPT) else { return };
    let mut sim = Sim::new(2);
    sim.present(&mut app, "first");
    sim.present(&mut app, "second");
    js(&app, "tg('fx')");
    app.rebuild_layout(W, H, false);
    assert!(!sim.present(&mut app, "fixed toggle"), "a fixed box is never patched");
    js(&app, "tg('a')");
    app.rebuild_layout(W, H, false);
    sim.present(&mut app, "after");
}

/// A mix of everything on three buffers, interleaving full paints (the damage history restarts).
#[test]
fn qhov2_mixed_frames_on_three_buffers() {
    let Some((mut app, _l)) = live_app(PAGE, SCRIPT) else { return };
    let mut sim = Sim::new(3);
    for i in 0..4 {
        sim.present(&mut app, &format!("warm {i}"));
    }
    for id in ["a", "fx", "b", "a", "c", "b", "fx", "d", "e", "c", "a"] {
        js(&app, &format!("tg('{id}')"));
        app.rebuild_layout(W, H, false);
        sim.present(&mut app, &format!("toggle #{id}"));
        if id == "b" {
            // a redraw with nothing to do
            sim.present(&mut app, "idle");
        }
    }
    assert!(sim.fast >= 8 && sim.full >= 2, "patches {} full {}", sim.fast, sim.full);
}

fn dmg(rects: &[(f32, f32, f32, f32)]) -> Damage {
    Damage { rects: rects.iter().map(|&(l, t, r, b)| crate::layout::Ink { l, t, r, b }).collect(), reach: 0.0, layer_fx: false }
}

/// `damage_plan`: the current rects, then the missing frames' (deduplicated); too old or unknown = none.
#[test]
fn qhov2_damage_plan_by_buffer_age() {
    let hist = [dmg(&[(0.0, 0.0, 10.0, 10.0)]), dmg(&[(0.0, 0.0, 10.0, 10.0), (20.0, 0.0, 30.0, 10.0)]), dmg(&[(40.0, 0.0, 50.0, 10.0)])];
    let cur = dmg(&[(0.0, 0.0, 10.0, 10.0)]);
    let n = |age| crate::app::damage_plan(&hist, &cur, age).map(|(m, c)| (m.rects.len(), c));
    assert_eq!(n(0), None, "age 0: the buffer's content is unknown");
    assert_eq!(n(1), Some((1, 1)), "age 1: the page presented last, only this frame's rects");
    assert_eq!(n(2), Some((2, 1)), "age 2: plus the last frame's");
    assert_eq!(n(3), Some((3, 1)), "age 3: the earlier duplicate rect is not repeated");
    assert_eq!(n(4), Some((3, 1)));
    assert_eq!(n(5), None, "older than the remembered frames: paint whole");
    assert_eq!(crate::app::damage_plan(&[], &cur, 2).map(|x| x.1), None, "no history after a full paint: an age-2 buffer is older than the full paint");
}
