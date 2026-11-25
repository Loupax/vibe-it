#[allow(unused_imports)]
use super::*;

// ---- QAN1a: @keyframes + animation longhands (parse only) ----

fn qan1a_anims(css: &str, body: &str) -> Vec<Animation> {
    let doc = parse_html(&format!("<html><body>{body}</body></html>"));
    let rules = parse_stylesheet(css);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    st.animations.map(|a| a.resolve()).unwrap_or_default()
}

#[test]
fn qan1a_shorthand_token_order() {
    let a = qan1a_anims("div{animation: 2s ease-in 500ms 3 alternate both paused foo}", "<div></div>");
    assert_eq!(a.len(), 1);
    let a = &a[0];
    assert_eq!(a.name.as_deref(), Some("foo"));
    assert_eq!(a.duration, 2000.0);
    assert_eq!(a.delay, 500.0);
    assert_eq!(a.iterations, 3.0);
    assert_eq!(a.direction, AnimDirection::Alternate);
    assert_eq!(a.fill, AnimFill::Both);
    assert_eq!(a.play_state, AnimPlayState::Paused);
    assert_eq!(a.timing, TimingFunction::CubicBezier(0.42, 0.0, 1.0, 1.0));
    // Name first, negative delay, infinite, cubic-bezier.
    let a = qan1a_anims("div{animation: bar 1s cubic-bezier(0,1,1,0) -500s infinite}", "<div></div>");
    assert_eq!(a[0].name.as_deref(), Some("bar"));
    assert_eq!(a[0].delay, -500000.0);
    assert!(a[0].iterations.is_infinite());
    assert_eq!(a[0].timing, TimingFunction::CubicBezier(0.0, 1.0, 1.0, 0.0));
    // Defaults.
    let a = qan1a_anims("div{animation: foo}", "<div></div>");
    assert_eq!(a[0].duration, 0.0);
    assert_eq!(a[0].timing, TimingFunction::EASE);
    assert_eq!(a[0].iterations, 1.0);
    assert_eq!(a[0].fill, AnimFill::None);
    // `none` and a keyword-like name.
    assert_eq!(qan1a_anims("div{animation: none}", "<div></div>")[0].name, None);
    assert_eq!(qan1a_anims("div{animation: foo none}", "<div></div>")[0].fill, AnimFill::None);
}

#[test]
fn qan1a_comma_lists_and_longhand_cycling() {
    let a = qan1a_anims("div{animation: a 1s, b 2s linear 1s}", "<div></div>");
    assert_eq!(a.len(), 2);
    assert_eq!((a[0].name.as_deref(), a[0].duration), (Some("a"), 1000.0));
    assert_eq!((a[1].name.as_deref(), a[1].duration, a[1].delay), (Some("b"), 2000.0, 1000.0));
    assert_eq!(a[1].timing, TimingFunction::Linear);
    let a = qan1a_anims(
        "div{animation-name: a, b, c; animation-duration: 1s, 2s; animation-direction: reverse}",
        "<div></div>",
    );
    assert_eq!(a.len(), 3);
    assert_eq!(a.iter().map(|x| x.duration).collect::<Vec<_>>(), [1000.0, 2000.0, 1000.0]);
    assert!(a.iter().all(|x| x.direction == AnimDirection::Reverse));
}

#[test]
fn qan1a_webkit_aliases_and_invalid_dropped() {
    let a = qan1a_anims("div{-webkit-animation: foo 1s; -webkit-animation-delay: 2s}", "<div></div>");
    assert_eq!((a[0].duration, a[0].delay), (1000.0, 2000.0));
    // Invalid declarations are dropped, the rest of the sheet survives.
    let a = qan1a_anims(
        "div{animation: foo 1s; animation-duration: -1s; animation-timing-function: cubic-bezier(2,0,1,1); animation: 1s 2s 3s x}",
        "<div></div>",
    );
    assert_eq!((a[0].name.as_deref(), a[0].duration), (Some("foo"), 1000.0));
    assert_eq!(a[0].timing, TimingFunction::EASE);
}

#[test]
fn qan1a_timing_functions() {
    let t = |v: &str| parse_animation_value("animation-timing-function", v).map(|d| d.timing.unwrap()[0]);
    assert_eq!(t("linear"), Ok(TimingFunction::Linear));
    assert_eq!(t("ease-out"), Ok(TimingFunction::CubicBezier(0.0, 0.0, 0.58, 1.0)));
    assert_eq!(t("ease-in-out"), Ok(TimingFunction::CubicBezier(0.42, 0.0, 0.58, 1.0)));
    assert_eq!(t("steps(4)"), Ok(TimingFunction::Steps(4, StepPos::JumpEnd)));
    assert_eq!(t("steps(2, start)"), Ok(TimingFunction::Steps(2, StepPos::JumpStart)));
    assert_eq!(t("steps(3, jump-both)"), Ok(TimingFunction::Steps(3, StepPos::JumpBoth)));
    assert_eq!(t("steps(1, jump-none)"), Err(()));
    assert_eq!(t("steps(0)"), Err(()));
    assert_eq!(t("step-start"), Ok(TimingFunction::Steps(1, StepPos::JumpStart)));
    assert_eq!(t("step-end"), Ok(TimingFunction::Steps(1, StepPos::JumpEnd)));
    assert_eq!(t("cubic-bezier(0.1, 0.7, 1, 0.1)"), Ok(TimingFunction::CubicBezier(0.1, 0.7, 1.0, 0.1)));
}

#[test]
fn qan1a_inherit_initial_per_longhand() {
    let doc = parse_html("<html><body><section><div></div></section></body></html>");
    let rules = parse_stylesheet(
        "section{animation: a 3s linear 2s} div{animation-name: inherit; animation-duration: inherit; animation-delay: initial; animation-timing-function: inherit}",
    );
    let s = find_element_by_tag(&doc, "section").unwrap();
    let ss = resolve_style(&s, &rules, ROOT_STYLE, 800.0);
    let d = find_element_by_tag(&doc, "div").unwrap();
    let inh = InheritedStyle { parent: Some(ss), ..ROOT_STYLE };
    let ds = resolve_style(&d, &rules, inh, 800.0);
    let a = ds.animations.unwrap().resolve();
    assert_eq!(a[0].name.as_deref(), Some("a"));
    assert_eq!(a[0].duration, 3000.0);
    assert_eq!(a[0].delay, 0.0);
    assert_eq!(a[0].timing, TimingFunction::Linear);
    assert_eq!(a[0].iterations, 1.0);
    // Not inherited by default.
    let rules = parse_stylesheet("section{animation: a 3s}");
    let ds = resolve_style(&d, &rules, InheritedStyle { parent: Some(ss), ..ROOT_STYLE }, 800.0);
    assert!(ds.animations.is_none());
}

#[test]
fn qan1a_keyframes_registry() {
    let (_, kf) = parse_stylesheet_with_keyframes(
        "@keyframes a { from { opacity: 0 } 0%, 50% { opacity: .5 } to { opacity: 1 !important; color: red } }
         @-webkit-keyframes w { 100% { opacity: 1 } }
         @keyframes \"s\" { to { opacity: 1 } }
         @keyframes none { to { opacity: 1 } }
         @keyframes bad { 150% { opacity: 1 } 10% { opacity: 1 } }
         @keyframes dup { to { opacity: 0.1 } } @keyframes dup { to { opacity: 0.9 } }
         p { color: red }",
    );
    let a = registry_get(&kf, "a", 800.0).expect("a");
    assert_eq!(a.iter().map(|k| k.offset).collect::<Vec<_>>(), [0.0, 0.0, 0.5, 1.0]);
    assert!(a[3].decls.opacity.is_none(), "!important inside a keyframe is dropped");
    assert!(a[3].decls.color.is_some());
    assert_eq!(a[2].decls.opacity, Some(0.5));
    assert!(registry_get(&kf, "w", 800.0).is_some() && registry_get(&kf, "s", 800.0).is_some());
    assert!(registry_get(&kf, "none", 800.0).is_none());
    assert_eq!(registry_get(&kf, "dup", 800.0).unwrap()[0].decls.opacity, Some(0.9), "last definition wins");
}

#[test]
fn qan1a_keyframes_inside_media_and_supports() {
    let (rules, kf) = parse_stylesheet_with_keyframes(
        "@media all { @keyframes yes { to { opacity: 1 } } }
         @media not all { @keyframes no { to { opacity: 1 } } }
         @supports (display: grid) { @keyframes sy { to { opacity: 1 } } }
         @supports (foo: bar) { @keyframes sn { to { opacity: 1 } } }
         div { color: blue }",
    );
    assert!(registry_get(&kf, "yes", 800.0).is_some() && registry_get(&kf, "sy", 800.0).is_some());
    assert!(registry_get(&kf, "no", 800.0).is_none() && registry_get(&kf, "sn", 800.0).is_none());
    assert_eq!(rules.len(), 1, "the sheet after the keyframes still parses");
}

#[test]
fn qan1a_registry_reaches_the_cascade_cache() {
    let doc = parse_html("<html><head><style>@keyframes k { to { opacity: 1 } } div { animation: k 1s }</style></head><body><div></div></body></html>");
    let _ = build_layout_tree(&doc, 800.0);
    assert!(lookup_keyframes("k", 800.0).is_some());
    let _ = build_layout_tree(&doc, 800.0);
    assert!(lookup_keyframes("k", 800.0).is_some(), "cache hit keeps the registry");
}

#[test]
fn qan1a_computed_style_stays_small() {
    fn is_copy<T: Copy>() {}
    is_copy::<ComputedStyle>();
    eprintln!("size_of ComputedStyle = {}", std::mem::size_of::<ComputedStyle>());
}

// ---- QAN1b: evaluating animations ----

/// Style of the first `div` at document time `now` ms (a fresh start-time
/// map each call, so the animation starts at `now`'s own zero).
fn qan1b_style(css: &str, now: f64, body: &str) -> ComputedStyle {
    let doc = parse_html(&format!("<html><body>{body}</body></html>"));
    let (rules, kf) = parse_stylesheet_with_keyframes(css);
    set_keyframes(Rc::new(kf));
    set_document_clock(0.0);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let _ = resolve_style(&d, &rules, ROOT_STYLE, 800.0); // records the start at 0
    set_document_clock(now);
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    set_document_clock(0.0);
    finish_animation_pass();
    finish_animation_pass();
    st
}

#[test]
fn qan1b_timing_functions() {
    let f = TimingFunction::CubicBezier(0.0, 1.0, 1.0, 0.0);
    assert!((f.ease(0.5) - 0.5).abs() < 1e-3);
    assert!((TimingFunction::Linear.ease(0.3) - 0.3).abs() < 1e-6);
    assert!((TimingFunction::EASE.ease(0.5) - 0.8024).abs() < 1e-2);
    let end = TimingFunction::Steps(4, StepPos::JumpEnd);
    assert_eq!(end.ease(0.0), 0.0);
    assert_eq!(end.ease(0.3), 0.25);
    assert_eq!(end.ease(0.99), 0.75);
    assert_eq!(end.ease(1.0), 1.0);
    let start = TimingFunction::Steps(2, StepPos::JumpStart);
    assert_eq!(start.ease(0.0), 0.5);
    assert_eq!(start.ease(0.6), 1.0);
}

#[test]
fn qan1b_color_interpolates_in_premultiplied_srgb() {
    let st = qan1b_style(
        "@keyframes k{from{background-color:rgb(0,200,0)}to{background-color:rgb(200,0,0)}} div{animation:k 1000s linear 0s}",
        500_000.0,
        "<div></div>",
    );
    assert_eq!(st.background_color, Some(TextColor::rgba(100, 100, 0, 255)));
    // transparent -> red: premultiplied keeps the colour red, alpha halves
    let st = qan1b_style(
        "@keyframes k{from{background-color:transparent}to{background-color:rgb(200,0,0)}} div{animation:k 1000s linear}",
        500_000.0,
        "<div></div>",
    );
    assert_eq!(st.background_color, Some(TextColor::rgba(200, 0, 0, 128)));
}

#[test]
fn qan1b_negative_delay_and_cubic_bezier_body_case() {
    let st = qan1b_style(
        "@keyframes k{from{background-color:rgb(0,200,0)}to{background-color:rgb(200,0,0)}} div{animation:k 1000000s cubic-bezier(0,1,1,0) -500000s}",
        0.0,
        "<div></div>",
    );
    let c = st.background_color.unwrap();
    assert!((c.r() as i32 - 100).abs() <= 1 && (c.g() as i32 - 100).abs() <= 1);
}

#[test]
fn qan1b_opacity_length_and_discrete_fields() {
    let st = qan1b_style(
        "@keyframes k{from{opacity:0;width:100px;margin-left:10px;display:block;z-index:0}to{opacity:1;width:200px;margin-left:20px;display:none;z-index:10}} div{animation:k 1s linear}",
        250.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.25).abs() < 1e-3);
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if (w - 125.0).abs() < 1e-3));
    assert!(matches!(st.margin.left, BoxLength::Px(m) if (m - 12.5).abs() < 1e-3));
    assert!(st.display != Display::None, "display is discrete: still A at 25 %");
    let st = qan1b_style(
        "@keyframes k{from{display:block}to{display:none}} div{animation:k 1s linear}",
        600.0,
        "<div></div>",
    );
    assert_eq!(st.display, Display::None);
}

#[test]
fn qan1b_mixed_units_are_discrete() {
    let st = qan1b_style(
        "@keyframes k{from{width:100px}to{width:50%}} div{animation:k 1s linear}",
        250.0,
        "<div></div>",
    );
    assert!(matches!(st.width, Some(BoxLength::Px(_))));
}

#[test]
fn qan1b_fill_modes() {
    let css = |fill: &str| {
        format!("@keyframes k{{from{{opacity:0.2}}to{{opacity:0.8}}}} div{{animation:k 1s linear 1s {fill}}}")
    };
    // before the delay
    assert_eq!(qan1b_style(&css("none"), 500.0, "<div></div>").opacity, 1.0);
    assert!((qan1b_style(&css("backwards"), 500.0, "<div></div>").opacity - 0.2).abs() < 1e-3);
    assert!((qan1b_style(&css("both"), 500.0, "<div></div>").opacity - 0.2).abs() < 1e-3);
    // after the end
    assert_eq!(qan1b_style(&css("none"), 3000.0, "<div></div>").opacity, 1.0);
    assert_eq!(qan1b_style(&css("backwards"), 3000.0, "<div></div>").opacity, 1.0);
    assert!((qan1b_style(&css("forwards"), 3000.0, "<div></div>").opacity - 0.8).abs() < 1e-3);
}

#[test]
fn qan1b_iterations_direction_and_paused() {
    let css = |extra: &str| format!("@keyframes k{{from{{opacity:0}}to{{opacity:1}}}} div{{animation:k 1s linear {extra}}}");
    // second iteration of an alternate animation runs backwards
    let st = qan1b_style(&css("2 alternate"), 1250.0, "<div></div>");
    assert!((st.opacity - 0.75).abs() < 1e-3);
    let st = qan1b_style(&css("infinite"), 10_250.0, "<div></div>");
    assert!((st.opacity - 0.25).abs() < 1e-3);
    let st = qan1b_style(&css("reverse"), 250.0, "<div></div>");
    assert!((st.opacity - 0.75).abs() < 1e-3);
    // paused: time is frozen at the start
    let st = qan1b_style(&css("paused"), 5000.0, "<div></div>");
    assert_eq!(st.opacity, 0.0);
}

#[test]
fn qan1b_per_keyframe_timing_and_steps() {
    let st = qan1b_style(
        "@keyframes k{0%{opacity:0;animation-timing-function:steps(2,end)}100%{opacity:1}} div{animation:k 1s linear}",
        600.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.5).abs() < 1e-3);
}

#[test]
fn qan1b_missing_endpoint_uses_the_base_style() {
    let st = qan1b_style(
        "@keyframes k{to{opacity:0}} div{opacity:0.5;animation:k 1s linear}",
        500.0,
        "<div></div>",
    );
    assert!((st.opacity - 0.25).abs() < 1e-3);
}

#[test]
fn qan1b_animation_beats_inline_but_not_important() {
    let css = "@keyframes k{from{width:100px}to{width:100px}} div{animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, r#"<div style="width:10px"></div>"#);
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 100.0));
    let css = "@keyframes k{from{width:100px}to{width:100px}} div{width:10px !important;animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, "<div></div>");
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 10.0));
}

#[test]
fn qan1b_important_in_keyframe_is_ignored() {
    let css = "@keyframes k{from{width:100px !important}to{width:100px !important}} div{width:10px;animation:k 1s linear}";
    let st = qan1b_style(css, 500.0, "<div></div>");
    assert!(matches!(st.width, Some(BoxLength::Px(w)) if w == 10.0));
}

#[test]
fn qan1b_pseudo_elements_animate() {
    let doc = parse_html("<html><body><div></div></body></html>");
    let (rules, kf) = parse_stylesheet_with_keyframes(
        "@keyframes k{from{opacity:0}to{opacity:1}} div::after{content:'x';animation:k 1s linear -500ms both}",
    );
    set_keyframes(Rc::new(kf));
    set_document_clock(0.0);
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style_for(&d, &rules, ROOT_STYLE, 800.0, Some("after"));
    assert!((st.opacity - 0.5).abs() < 1e-3);
    finish_animation_pass();
}

#[test]
fn qan1b_no_animation_means_no_start_bookkeeping() {
    let doc = parse_html("<html><body><div></div></body></html>");
    let rules = parse_stylesheet("div{opacity:0.5}");
    let d = find_element_by_tag(&doc, "div").expect("div");
    let st = resolve_style(&d, &rules, ROOT_STYLE, 800.0);
    assert!(st.animations.is_none());
    assert_eq!(st.opacity, 0.5);
}

#[test]
fn qan1b_computed_style_stays_copy_report_size() {
    fn is_copy<T: Copy>() {}
    is_copy::<ComputedStyle>();
    eprintln!("size_of::<ComputedStyle>() = {}", std::mem::size_of::<ComputedStyle>());
}

// ---- QAN1c: animation start/end events and the settle-first restyles ----

struct Qan1c {
    rules: Vec<StyleRule>,
    div: Handle,
    tracker: AnimTracker,
    engine: JsEngine,
    log: Rc<RefCell<Vec<String>>>,
    builds: usize,
    last: ComputedStyle,
}

impl Qan1c {
    fn new(css: &str, body: &str, script: &str) -> Qan1c {
        let doc = parse_html(&format!("<html><body>{body}</body></html>"));
        let (rules, kf) = parse_stylesheet_with_keyframes(css);
        set_keyframes(Rc::new(kf));
        let div = find_element_by_tag(&doc, "div").expect("div");
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let engine = start_scripts(&doc, &[src(script)], Rc::new(move |level, text| l.borrow_mut().push(format!("{level}:{text}")))).expect("engine");
        set_app_clock_override(Some(0.0));
        set_app_clock(7);
        let last = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        let mut q = Qan1c { rules, div, tracker: AnimTracker::default(), engine, log, builds: 1, last };
        q.build(0.0);
        q
    }
    /// One restyle at app clock `ms` (what the App does in `redraw`).
    fn build(&mut self, ms: f64) {
        set_app_clock_override(Some(ms));
        set_app_clock(7);
        self.last = resolve_style(&self.div, &self.rules, ROOT_STYLE, 800.0);
        finish_animation_pass();
        self.tracker.after_build(7, take_animation_report(), document_clock_ms());
    }
    /// What `App::anim_tick` does at `ms`: dispatch, restyle if asked.
    fn tick(&mut self, ms: f64) -> Option<f64> {
        let t = self.tracker.tick(ms);
        for ev in t.events {
            let init = [
                ("bubbles", EventField::Bool(true)),
                ("animationName", EventField::Str(ev.name)),
                ("elapsedTime", EventField::Num(ev.elapsed_s)),
                ("pseudoElement", EventField::Str(ev.pseudo)),
            ];
            self.engine.dispatch_event(ev.node.as_ref(), ev.kind, &init);
        }
        if t.restyle {
            self.builds += 1;
            self.build(ms);
        }
        t.next
    }
    fn events(&self) -> Vec<String> {
        self.log.borrow().iter().filter(|l| l.contains("ev:")).cloned().collect()
    }
}

const QAN1C_LISTEN: &str = "['start','iteration','end','cancel'].forEach(function (k) { document.addEventListener('animation' + k, function (e) { console.log('ev:' + e.type + ':' + e.animationName + ':' + e.elapsedTime + ':' + e.pseudoElement + ':' + (e.target.tagName)); }); });";

#[test]
fn qan1c_fade_in_forwards_restyles_twice_and_ends_once() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 200ms forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.last.opacity, 0.0);
    // Starts at once (no delay): the event, no extra restyle, deadline at 200 ms.
    assert_eq!(q.tick(0.0), Some(200.0));
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    // Nothing happens before the end.
    assert_eq!(q.tick(100.0), Some(200.0));
    assert_eq!(q.builds, 1);
    assert_eq!(q.tick(200.0), None);
    assert_eq!(q.builds, 2, "initial build + one restyle at the end");
    assert_eq!(q.last.opacity, 1.0, "final state painted");
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV", "log:ev:animationend:a:0.2::DIV"]);
    // Later ticks are silent and cause no restyle.
    assert_eq!(q.tick(5000.0), None);
    assert_eq!(q.builds, 2);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_delay_restyles_at_start_and_at_end() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 200ms 300ms forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), Some(300.0));
    assert!(q.events().is_empty());
    assert_eq!(q.tick(300.0), Some(500.0));
    assert_eq!(q.builds, 2, "restyle when the delay elapses");
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    assert_eq!(q.tick(500.0), None);
    assert_eq!(q.builds, 3);
    assert_eq!(q.last.opacity, 1.0);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_name_change_cancels_the_animation() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 1s}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    q.tick(0.0);
    // The page changed the animation name: the next build no longer styles it.
    let (rules, _) = parse_stylesheet_with_keyframes("div{animation:none}");
    q.rules = rules;
    q.build(300.0);
    assert_eq!(q.tick(300.0), None);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV", "log:ev:animationcancel:a:0.3::DIV"]);
    // A cancelled animation never ends.
    q.tick(5000.0);
    assert_eq!(q.events().len(), 2);
    set_app_clock_override(None);
}

#[test]
fn qan1c_infinite_gives_start_only_and_paused_never_ends() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms infinite}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), None, "no deadline for an infinite animation");
    q.tick(10_000.0);
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms paused}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    assert_eq!(q.tick(0.0), None);
    q.tick(10_000.0);
    assert_eq!(q.builds, 1);
    assert_eq!(q.events(), vec!["log:ev:animationstart:a:0::DIV"]);
    set_app_clock_override(None);
}

#[test]
fn qan1c_iteration_events_are_capped_to_one_per_second() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 100ms 40 forwards}";
    let mut q = Qan1c::new(css, "<div>x</div>", QAN1C_LISTEN);
    let mut t = q.tick(0.0);
    let mut n = 0;
    while let Some(at) = t {
        t = q.tick(at);
        n += 1;
        assert!(n < 100, "deadline loop must terminate");
    }
    let it = q.events().iter().filter(|e| e.contains("animationiteration")).count();
    // 4 s of animation: about one per second, never one per 100 ms iteration.
    assert!((3..=5).contains(&it), "{:?}", q.events());
    assert_eq!(q.events().iter().filter(|e| e.contains("animationend")).count(), 1);
    set_app_clock_override(None);
}

#[test]
fn qan1c_onanimation_attribute_and_property_handlers_run() {
    let css = "@keyframes a{from{opacity:0}to{opacity:1}} div{animation:a 50ms forwards}";
    let mut q = Qan1c::new(css, "<div onanimationstart=\"console.log('ev:attr-start')\">x</div>", "document.querySelector('div').onanimationend = function (e) { console.log('ev:prop-end:' + e.animationName); };");
    q.tick(0.0);
    q.tick(50.0);
    assert_eq!(q.events(), vec!["log:ev:attr-start", "log:ev:prop-end:a"]);
    set_app_clock_override(None);
}

fn qan1d0_red_p(html_css: &str) -> bool {
    let doc = parse_html(&format!("<html><body><style>{html_css}</style><p>x</p></body></html>"));
    let tree = build_layout_tree(&doc, 800.0);
    let mut v = Vec::new();
    find_all_span_colors(&tree, &mut v);
    v.contains(&TextColor::rgb(255, 0, 0))
}

fn qan1d0_match(q: &str) -> String {
    let out = run_collecting(&[src(&format!("console.log(matchMedia('{q}').matches);"))]);
    out[0].clone()
}

#[test]
fn qan1d0_reduced_motion_default_and_set_motion_flip() {
    let _m = MOTION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(SET_ARGS.contains(&"motion") && SET_ARGS.contains(&"nomotion"));
    set_reduced_motion(true);
    assert!(reduced_motion());
    let reduce_css = "@media (prefers-reduced-motion: reduce){p{color:#f00}}";
    let nopref_css = "@media (prefers-reduced-motion: no-preference){p{color:#f00}}";
    assert!(qan1d0_red_p(reduce_css), "reduce block applies by default");
    assert!(!qan1d0_red_p(nopref_css), "no-preference block does not apply by default");
    assert_eq!(qan1d0_match("(prefers-reduced-motion: reduce)"), "log:true");
    assert_eq!(qan1d0_match("(prefers-reduced-motion: no-preference)"), "log:false");
    set_reduced_motion(false);
    assert!(!qan1d0_red_p(reduce_css), "reduce block off after :set motion");
    assert!(qan1d0_red_p(nopref_css), "no-preference block applies after :set motion");
    assert_eq!(qan1d0_match("(prefers-reduced-motion: reduce)"), "log:false");
    assert_eq!(qan1d0_match("(prefers-reduced-motion: no-preference)"), "log:true");
    set_reduced_motion(true);
    assert_eq!(qan1d0_match("(prefers-reduced-motion: reduce)"), "log:true");
}

// ---- QY53i3: animation list sets in a per-thread arena freed on navigation ----

#[test]
fn qy53i3_intern_same_lists_same_handle_and_reset_frees() {
    reset_anim_arena();
    let mk = |n: &str| AnimLonghands { name: vec![Some(n.to_string())], ..Default::default() };
    let a = intern_anim(mk("a")).expect("handle");
    let a2 = intern_anim(mk("a")).expect("handle");
    let b = intern_anim(mk("b")).expect("handle");
    assert_eq!(a, a2, "equal lists, equal handle");
    assert_ne!(a, b);
    assert_eq!(anim_arena_len(), 2);
    assert_eq!(a.resolve()[0].name.as_deref(), Some("a"));
    assert!(intern_anim(AnimLonghands::default()).is_none());
    reset_anim_arena();
    assert_eq!(anim_arena_len(), 0, "navigation clears the arena");
    assert!(a.table().is_none() && a.resolve().is_empty(), "stale handle reads as nothing");
    let c = intern_anim(mk("a")).expect("handle");
    assert_eq!(c.resolve()[0].name.as_deref(), Some("a"), "new handles work");
    assert_ne!(c, a, "a stale handle never aliases a new one");
    assert_eq!(anim_arena_len(), 1);
}
