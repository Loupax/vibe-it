//! QHOV1: a hover that changes only what is painted costs no relayout and repaints only the damaged
//! rect. The engine reports the rects its splices changed (`incr_layout::take_damage`); the App patches
//! the kept page with them (`repaint_rect`). These tests pin that the damage is small and that the
//! patched page is pixel for pixel the page a full paint of the new tree makes.
use super::*;

const W: u32 = 600;
const H: u32 = 900;

struct Live;

impl Drop for Live {
    fn drop(&mut self) {
        crate::anim_eval::set_app_clock_override(None);
        set_interaction(&InteractionState::default());
    }
}

fn find_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| &*v == id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_id(c, id))
}

/// An App on `html` after its first frame, its engine damage reset. `None` without a display.
fn live_app(html: &str) -> Option<(App, Live)> {
    let proxy = test_event_loop().ok().map(|l| l.create_proxy())?;
    let live = Live;
    crate::anim_eval::set_app_clock_override(Some(0.0));
    set_interaction(&InteractionState::default());
    let mut app = App::new_static(parse_html(html), proxy, None);
    app.rebuild_layout(W, H, false);
    assert!(app.cached.as_ref().is_some_and(crate::incr_layout::owns), "the App shows the engine's tree");
    let _ = crate::incr_layout::take_damage();
    Some((app, live))
}

fn hover(app: &mut App, id: &str) {
    let n = node_id(&find_id(&app.document, id).expect("id"));
    let mut st = app.interaction.clone();
    st.hover = app.chain_for(n);
    app.set_interaction_state(st);
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

type Rects = Vec<(u32, u32, u32, u32)>;

/// One hover step: `kept` is the page before it; returns the damage rects (None = full paint) and
/// asserts the patched page equals the full paint of the new tree.
fn step(app: &mut App, kept: &mut Vec<u32>, id: &str) -> Option<Rects> {
    hover(app, id);
    app.rebuild_layout(W, H, false);
    let laid0 = app.cached.clone().expect("laid");
    let rects = crate::incr_layout::take_damage().and_then(|d| crate::app::damage_screen_rects(&d, &laid0, (0.0, 0.0), (W, H)));
    let want = full_paint(app);
    match &rects {
        Some((rects, margin)) => {
            let laid = app.cached.clone().expect("laid");
            let mut scratch = vec![0u32; kept.len()];
            for &(l, t, r, b) in rects {
                crate::app::repaint_rect(
                    &laid, &mut app.font_system, &mut app.swash_cache, &mut app.shape_cache, &app.scroll_offsets,
                    &mut scratch, (W, H), (0.0, 0.0), None, None, (l, t, r, b), *margin,
                );
                for y in t..b {
                    let (a, z) = ((y * W + l) as usize, (y * W + r) as usize);
                    kept[a..z].copy_from_slice(&scratch[a..z]);
                }
            }
        }
        None => *kept = want.clone(),
    }
    let bad = kept.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(bad, 0, "hover #{id}: the patched page differs from a full paint in {bad} pixels (damage {rects:?})");
    rects.map(|r| r.0)
}

const PAGE: &str = "<html><head><style>
body{margin:0;font:14px sans-serif}
a{color:#0000ee;text-decoration:none}
#l:hover{color:#c00;text-decoration:underline}
#bg{display:block;height:30px;margin:10px;background:#eee}
#bg:hover{background:#9cf}
#bd{height:30px;margin:10px;border:3px solid #ccc}
#bd:hover{border-color:#f80}
#sh{height:30px;margin:20px 10px;background:#fff;border:1px solid #ccc}
#sh:hover{box-shadow:0 4px 18px rgba(0,0,0,.5)}
#op{height:30px;margin:10px;background:#c33}
#op:hover{opacity:.3}
#tall{height:30px;margin:10px;background:#ddd}
#tall:hover{height:80px}
#far{height:30px;background:#cfc;margin:10px}
</style></head><body>
<p><a id=l href=\"#\">a link to hover</a> and text after it in the same paragraph.</p>
<a id=bg href=\"#\">bg</a><div id=bd>bd</div><div id=sh>sh</div><div id=op>op</div><div id=tall>tall</div>
<div id=far>far</div><p>The end of the page, far below every hover target.</p></body></html>";

/// A colour hover is a main-thread frame whose damage is a small rect.
#[test]
fn qhov1_colour_hover_has_a_small_damage() {
    let Some((mut app, _l)) = live_app(PAGE) else { return };
    hover(&mut app, "l");
    app.rebuild_layout(W, H, false);
    let d = crate::incr_layout::take_damage().expect("a paint-only hover has damage").rects;
    assert!(!d.is_empty() && d.len() <= 2, "{} rects", d.len());
    let area: f32 = d.iter().map(|r| (r.r - r.l) * (r.b - r.t)).sum();
    assert!(area < 0.2 * (W * H) as f32, "damage {d:?} is a small part of the page");
    // Taking it reset it: nothing changed since.
    assert_eq!(crate::incr_layout::take_damage().map(|d| d.rects.len()), Some(0));
}

/// Hovers over paint-only and layout targets: every patched page equals the full paint.
#[test]
fn qhov1_patched_page_equals_a_full_paint_for_every_hover() {
    let Some((mut app, _l)) = live_app(PAGE) else { return };
    let mut kept = full_paint(&mut app);
    let mut small = 0;
    for id in ["l", "bg", "bd", "sh", "op", "tall", "far", "l", "sh", "bg", "op", "bd"] {
        let r = step(&mut app, &mut kept, id);
        small += usize::from(r.is_some());
    }
    assert!(small >= 8, "most hovers repaint only a damaged rect (got {small} of 12)");
}

/// A box-shadow that appears on hover reaches past the border box: its pixels are in the damage.
#[test]
fn qhov1_box_shadow_is_inside_the_damage() {
    let Some((mut app, _l)) = live_app(PAGE) else { return };
    let mut kept = full_paint(&mut app);
    let r = step(&mut app, &mut kept, "sh").expect("a shadow hover repaints a rect");
    let laid = app.cached.clone().unwrap();
    let el = node_id(&find_id(&app.document, "sh").unwrap());
    fn walk(l: &Laid, n: usize, out: &mut Option<(f32, f32, f32, f32)>) {
        if l.style.node == n && l.style.pseudo == 0 {
            *out = Some((l.x, l.y, l.w, l.h));
        }
        if let LaidContent::Blocks(v) = &l.content {
            v.iter().for_each(|c| walk(c, n, out));
        }
    }
    let mut found = None;
    walk(&laid, el, &mut found);
    let (x, y, w, h) = found.expect("box");
    assert!(
        r.iter().any(|&(l, t, rr, b)| (l as f32) <= x - 10.0 && (rr as f32) >= x + w + 10.0 && (t as f32) <= y - 5.0 && (b as f32) >= y + h + 10.0),
        "{r:?} covers the shadow around {x},{y} {w}x{h}"
    );
}

/// A hover inside a scroll region or a fixed box cannot be patched from a document rect: unknown damage.
#[test]
fn qhov1_scroll_region_and_fixed_hovers_are_full_repaints() {
    let html = "<html><head><style>body{margin:0}#box{height:60px;overflow:auto;width:200px}\
                #in{height:30px;background:#eee}#in:hover{background:#f00}\
                #fx{position:fixed;top:300px;left:300px;width:50px;height:20px;background:#eee}#fx:hover{background:#0f0}\
                </style></head><body><div id=box><div id=in>in</div><div style=\"height:200px\"></div></div><div id=fx>f</div></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    hover(&mut app, "in");
    app.rebuild_layout(W, H, false);
    assert!(crate::incr_layout::take_damage().is_none(), "inside a scroll region");
    hover(&mut app, "fx");
    app.rebuild_layout(W, H, false);
    assert!(crate::incr_layout::take_damage().is_none(), "a fixed box");
}

/// A layout change (the box grows and moves what follows) is patched too, or repainted in full.
#[test]
fn qhov1_layout_hover_is_still_correct() {
    let Some((mut app, _l)) = live_app(PAGE) else { return };
    let mut kept = full_paint(&mut app);
    step(&mut app, &mut kept, "tall");
    step(&mut app, &mut kept, "far");
    step(&mut app, &mut kept, "tall");
}

const FX_PAGE: &str = "<html><head><style>
body{margin:0;font:14px sans-serif;background:#fff}
.b{position:absolute;left:20px;width:200px;height:40px}
#under{top:20px;background:#9c9}
#under:hover{background:#c33}
#blur{top:70px;background:#36c;filter:blur(6px)}
#near{top:112px;background:#ddd}
#near:hover{background:#f80}
#bd{top:200px;background:#cc6}
#bd:hover{background:#6cc}
#glass{top:210px;left:120px;width:200px;height:40px;position:absolute;backdrop-filter:blur(4px);background:rgba(255,255,255,.3)}
#mul{top:300px;left:20px;width:200px;height:40px;position:absolute;background:#fc9;mix-blend-mode:multiply}
#low{top:290px;background:#69c}
#low:hover{background:#c69}
#far{top:500px;background:#ccc}
#far:hover{background:#999}
#sh{top:560px;background:#eee;text-shadow:0 0 6px #000;color:#222}
</style></head><body>
<div class=b id=under>under</div><div class=b id=blur>blur</div><div class=b id=near>near</div>
<div class=b id=bd>bd</div><div id=glass></div><div class=b id=low>low</div><div id=mul></div>
<div class=b id=far>far</div><div class=b id=sh>shadowed text</div></body></html>";

/// Effects: every hover (next to a blurred box, under a backdrop-filter box, under a multiply box, far
/// from all of them) leaves the patched page equal to a full paint.
#[test]
fn qhov1_effects_patched_page_equals_a_full_paint() {
    let Some((mut app, _l)) = live_app(FX_PAGE) else { return };
    let mut kept = full_paint(&mut app);
    let mut res = Vec::new();
    for id in ["far", "under", "near", "bd", "low", "sh", "near", "bd", "far", "under"] {
        res.push((id, step(&mut app, &mut kept, id).is_some()));
    }
    let got = |id: &str, n: usize| res.iter().filter(|(i, _)| *i == id).nth(n).map(|(_, r)| *r).unwrap();
    assert!(!got("near", 0), "a rect reached by the blur of #blur is a full repaint");
    assert!(!got("bd", 0), "a rect under the backdrop-filter box is a full repaint");
    assert!(!got("low", 0), "a rect under the mix-blend-mode box is a full repaint");
    assert!(got("far", 0), "a hover away from every layer box is still a damage repaint");
}

/// A changed box with a blur of its own: its rect is repainted whole (it is a layer).
#[test]
fn qhov1_hover_on_a_filtered_box_is_correct() {
    let html = "<html><head><style>body{margin:0}#f{position:absolute;left:30px;top:30px;width:100px;height:40px;background:#36c;filter:blur(3px)}\
                #f:hover{background:#c33}#g{position:absolute;left:300px;top:300px;width:60px;height:30px;background:#eee}#g:hover{background:#999}</style></head>\
                <body><div id=f>f</div><div id=g>g</div></body></html>";
    let Some((mut app, _l)) = live_app(html) else { return };
    let mut kept = full_paint(&mut app);
    assert!(step(&mut app, &mut kept, "g").is_some(), "a plain box elsewhere");
    assert!(step(&mut app, &mut kept, "f").is_none(), "the filtered box itself (and #g leaving)");
}
