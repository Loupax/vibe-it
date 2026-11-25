// QEL7b: scroll leftovers: y clamped before reporting, `behavior` (smooth through the App's easing, instant in
// headless), the `scroll-behavior` property, scroll + MediaQueryList events inside iframe realms.
#[allow(unused_imports)]
use super::*;

const TALL: &str = "<html><body style='margin:0'><div style='height:3000px'></div></body></html>";
const SHORT: &str = "<html><body style='margin:0'><div style='height:100px'></div></body></html>";

fn run(html: &str, code: &str) -> Vec<String> {
    run_html_with_timers(html, code)
}

fn logs(log: &[String]) -> Vec<&str> {
    log.iter().filter_map(|l| l.strip_prefix("log:")).collect()
}

#[test]
fn scroll_to_past_the_end_is_clamped_before_it_is_reported() {
    let log = run(
        TALL,
        "var max = document.documentElement.scrollHeight - innerHeight;\
         addEventListener('scroll', () => console.log('scroll', scrollY === max));\
         scrollTo(0, 1e6);\
         console.log('now', scrollY === max, pageYOffset === max, max > 0);\
         scrollTo(0, 1e7); scrollBy(0, 500); document.documentElement.scrollTop = 1e9;\
         setTimeout(() => console.log('late', scrollY === max), 50);",
    );
    // One event (the first move); the later calls past the end change nothing and report nothing.
    assert_eq!(logs(&log), ["now true true true", "scroll true", "late true"], "{log:?}");
}

#[test]
fn scroll_on_a_page_shorter_than_the_viewport_stays_at_zero_without_events() {
    let log = run(
        SHORT,
        "addEventListener('scroll', () => console.log('scroll'));\
         scrollTo(0, 50); scrollBy(0, 20); scrollTo({ top: 10 });\
         console.log('y', scrollY);",
    );
    assert_eq!(logs(&log), ["y 0"], "{log:?}");
}

#[test]
fn behavior_is_validated_and_instant_without_an_app() {
    let log = run(
        TALL,
        "scrollTo({ top: 100, behavior: 'smooth' });\
         console.log('smooth', scrollY);\
         scrollBy({ top: 50, behavior: 'smooth' });\
         console.log('by', scrollY);\
         scrollTo({ top: 10, behavior: 'instant' }); console.log('instant', scrollY);\
         scrollTo({ top: 20, behavior: 'auto' }); console.log('auto', scrollY);\
         for (const f of [() => scrollTo({ behavior: 'nope' }), () => scrollBy({ behavior: 'nope' }),\
                          () => document.body.scrollIntoView({ behavior: 'nope' }),\
                          () => document.body.scrollTo({ behavior: 'nope' })]) {\
           try { f(); console.log('no throw'); } catch (e) { console.log(e.name); }\
         }",
    );
    assert_eq!(
        logs(&log),
        ["smooth 100", "by 150", "instant 10", "auto 20", "TypeError", "TypeError", "TypeError", "TypeError"],
        "{log:?}"
    );
}

#[test]
fn scroll_behavior_property_is_computed_and_not_inherited() {
    let log = run(
        "<html style='scroll-behavior:smooth'><body><div id=d style='scroll-behavior: smooth'></div><p id=p></p></body></html>",
        "const v = id => getComputedStyle(document.getElementById(id)).getPropertyValue('scroll-behavior');\
         console.log(getComputedStyle(document.documentElement).getPropertyValue('scroll-behavior'), v('d'), v('p'),\
                     getComputedStyle(document.body).getPropertyValue('scroll-behavior'));",
    );
    assert_eq!(logs(&log), ["smooth smooth auto auto"], "{log:?}");
}

#[test]
fn scroll_into_view_instant_in_headless_even_when_smooth_is_asked() {
    let log = run(
        "<html><body style='margin:0'><div style='height:2000px'></div><p id=p style='margin:0'>x</p><div style='height:2000px'></div></body></html>",
        "addEventListener('scroll', () => console.log('scroll', scrollY));\
         document.getElementById('p').scrollIntoView({ behavior: 'smooth', block: 'start' });\
         console.log('sync', scrollY);",
    );
    assert_eq!(logs(&log), ["sync 2000", "scroll 2000"], "{log:?}");
}

// ---- iframe realms ----

const FRAME: &str = "var f = document.createElement('iframe'); f.style.width = '300px'; f.style.height = '100px';\
    document.body.appendChild(f);\
    var w = f.contentWindow, d = f.contentDocument;\
    d.body.innerHTML = '<div id=in style=\"overflow:auto;width:50px;height:50px\"><div style=\"height:900px\"></div></div><div style=\"height:3000px\"></div>';";

#[test]
fn iframe_window_scroll_event_fires_in_the_parents_rendering_step_with_scrollend() {
    let log = run(
        TALL,
        &format!(
            "{FRAME}\
             w.addEventListener('scroll', e => console.log('scroll', e.target === d, e.isTrusted, w.scrollY));\
             w.addEventListener('scrollend', () => console.log('scrollend'));\
             requestAnimationFrame(() => console.log('raf'));\
             w.scrollTo(0, 40); w.scrollTo(0, 30);\
             console.log('sync', w.scrollY);"
        ),
    );
    assert_eq!(logs(&log), ["sync 30", "scroll true true 30", "raf", "scrollend"], "{log:?}");
}

#[test]
fn removed_iframe_reports_no_more_scroll_events() {
    let log = run(
        TALL,
        &format!(
            "{FRAME}\
             w.addEventListener('scroll', () => console.log('scroll'));\
             w.scrollTo(0, 40); f.remove();\
             setTimeout(() => console.log('end'), 100);"
        ),
    );
    assert!(!logs(&log).contains(&"scroll"), "{log:?}");
}

#[test]
fn iframe_media_query_list_registers_with_the_parents_step_bits() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let o = log.clone();
    let js = crate::js::start_scripts_clocked(
        &parse_html(TALL),
        &[src(&format!("{FRAME} var m = w.matchMedia('(max-width: 10px)'); m.addEventListener('change', () => console.log('change'));"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap();
    assert!(js.qel7_bits() & 4 != 0, "a watched list in an iframe asks for the media-query step");
}

// ---- the App ----

fn app_with_script(html: &str, script: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(html);
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.smooth_scroll_tau_ms = crate::config::SMOOTH_SCROLL_TAU_MS;
    crate::js_dom::SMOOTH_OK.with(|b| b.set(true));
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    Some(app)
}

#[test]
fn smooth_scroll_to_sets_the_apps_easing_target_and_reads_back_the_old_offset() {
    let Some(mut app) = app_with_script(TALL, "scrollTo({ top: 400, behavior: 'smooth' }); window.after = scrollY;") else { return };
    app.js_sync();
    assert_eq!(app.scroll_target, Some(400.0));
    assert_eq!(app.scroll_offset, 0.0, "painted offset unchanged until a step");
    let after = app.js.as_ref().unwrap().eval_value::<f64>("t", "window.after").unwrap();
    assert_eq!(after, 0.0, "scrollY is still the old offset right after the call");
    // An instant scroll cancels the animation.
    app.js.as_ref().unwrap().eval_value::<bool>("t", "scrollTo({ top: 50, behavior: 'instant' }); true").unwrap();
    app.js_sync();
    assert_eq!((app.scroll_offset, app.scroll_target), (50.0, None));
}

#[test]
fn css_scroll_behavior_on_the_root_makes_an_auto_scroll_smooth() {
    let Some(mut app) = app_with_script(
        "<html style='scroll-behavior:smooth'><body style='margin:0'><div style='height:3000px'></div></body></html>",
        "scrollBy(0, 100);",
    ) else { return };
    app.js_sync();
    assert_eq!(app.scroll_target, Some(100.0));
    app.js.as_ref().unwrap().eval_value::<bool>("t", "scrollBy({ top: 20, behavior: 'instant' }); true").unwrap();
    app.js_sync();
    assert_eq!(app.scroll_target, None, "instant overrides the CSS");
}

#[test]
fn smooth_is_off_when_the_app_does_not_ease() {
    let Some(mut app) = app_with_script(TALL, "") else { return };
    app.smooth_scroll_tau_ms = 0.0;
    crate::js_dom::SMOOTH_OK.with(|b| b.set(false));
    app.js.as_ref().unwrap().eval_value::<bool>("t", "scrollTo({ top: 300, behavior: 'smooth' }); true").unwrap();
    app.js_sync();
    assert_eq!((app.scroll_offset, app.scroll_target), (300.0, None));
}
