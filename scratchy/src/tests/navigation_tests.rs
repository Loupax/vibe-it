#[allow(unused_imports)]
use super::*;

// ---------- QY19: <meta http-equiv=refresh> ----------

#[test]
fn qy19_parse_meta_refresh() {
    let base = Url::parse("https://example.com/a/b").unwrap();
    let p = |c: &str| parse_meta_refresh(c, Some(&base)).map(|(d, u)| (d, u.to_string()));
    assert_eq!(p("0; url='x'"), Some((0, "https://example.com/a/x".into())));
    assert_eq!(p("5;URL=x"), Some((5, "https://example.com/a/x".into())));
    assert_eq!(p("  2 , url = \"/y?z=1\""), Some((2, "https://example.com/y?z=1".into())));
    assert_eq!(p("0;url=\"https://html.duckduckgo.com/html\""), Some((0, "https://html.duckduckgo.com/html".into())));
    assert_eq!(p("1; x"), Some((1, "https://example.com/a/x".into())), "url= prefix optional");
    assert_eq!(p("999;url=x").map(|r| r.0), Some(10), "delay clamped");
    assert_eq!(p("0; url=x#frag"), Some((0, "https://example.com/a/x#frag".into())));
    // Bare reload, junk, empty URL: never followed.
    assert_eq!(p("3"), None);
    assert_eq!(p("3;"), None);
    assert_eq!(p("0; url="), None);
    assert_eq!(p("url=x"), None);
    assert_eq!(p("-1; url=x"), None);
    assert_eq!(p(""), None);
}

#[test]
fn qy19_find_meta_refresh_and_chain_cap() {
    let base = Url::parse("https://example.com/").unwrap();
    let doc = parse_html(r#"<html><head><meta HTTP-EQUIV="Refresh" content="0; url=/next"></head><body></body></html>"#);
    let (d, u) = meta_refresh_action(&doc, Some(&base), 0).expect("found");
    assert_eq!((d, u.as_str()), (0, "https://example.com/next"));
    assert!(meta_refresh_action(&doc, Some(&base), META_REFRESH_MAX_CHAIN).is_none(), "chain capped");
    let other = parse_html(r#"<html><head><meta http-equiv="content-type" content="0; url=/x"></head></html>"#);
    assert!(meta_refresh_action(&other, Some(&base), 0).is_none());
}

#[test]
fn qy19_noscript_meta_only_with_js_off() {
    // JS_ENABLED is process-wide; tests that read or flip it hold this lock (main CI flake 4608cd4).
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let html = r#"<html><head><noscript><meta http-equiv="refresh" content="0; url=/h"></noscript></head><body></body></html>"#;
    let was = js_enabled();
    set_js_enabled(false);
    let off = new_parser().read_from(&mut html.as_bytes()).unwrap().document;
    set_js_enabled(true);
    let on = new_parser().read_from(&mut html.as_bytes()).unwrap().document;
    set_js_enabled(was);
    assert!(find_meta_refresh_content(&off).is_some(), "JS off: noscript meta is an element");
    assert!(find_meta_refresh_content(&on).is_none(), "JS on: noscript meta ignored");
}

#[test]
fn qy19_meta_redirect_replaces_history_and_is_chained() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.history = vec![Url::parse("http://127.0.0.1:9/start").unwrap(), Url::parse("http://127.0.0.1:9/redir").unwrap()];
    app.history_index = 1;
    app.base_url = Some(Url::parse("http://127.0.0.1:9/redir").unwrap());
    app.fetch_url(app.history[1].clone());
    let generation = app.nav_generation;
    let page = FetchedPage {
        html: br#"<html><head><meta http-equiv="refresh" content="0; url=/final#sec"></head><body>old</body></html>"#.to_vec(),
        extra_css: String::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/redir").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert_eq!(app.history.len(), 2, "no extra back step");
    assert_eq!(app.history[1].as_str(), "http://127.0.0.1:9/final#sec", "entry replaced");
    assert_eq!(app.history_index, 1);
    assert_eq!(app.meta_chain, 1);
    assert!(app.nav_generation > generation, "a new fetch was started");
    assert_eq!(app.nav_fragment.as_deref(), Some("sec"), "fragment of the target kept");
    // A stale delayed refresh (user navigated meanwhile) is ignored.
    let g = app.nav_generation;
    app.handle_meta_refresh(g - 1, Url::parse("http://127.0.0.1:9/zzz").unwrap());
    assert_eq!(app.nav_generation, g);
    // A current one pushes a new entry.
    app.handle_meta_refresh(g, Url::parse("http://127.0.0.1:9/zzz").unwrap());
    assert_eq!(app.history.len(), 3);
}

fn qy17_style(css: &str, html: &str) -> ComputedStyle {
    let doc = parse_html(html);
    let p = find_element_by_tag(&doc, "p").expect("no <p>");
    resolve_style(&p, &parse_stylesheet(css), ROOT_STYLE, 800.0)
}

#[test]
fn qy17_layered_rules_apply_unlayered_beat_them_and_later_layers_win() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = r#"<p id="a" class="c">x</p>"#;
    // A layered rule applies when nothing else does (it was dropped before).
    assert_eq!(qy17_style("@layer a{p{color:red}}", h).color, red);
    // Unlayered beats layered regardless of specificity and order.
    assert_eq!(qy17_style("@layer a{#a{color:red}} p{color:green}", h).color, green);
    assert_eq!(qy17_style("p{color:green} @layer a{#a.c{color:red}}", h).color, green);
    // Later layer beats earlier, regardless of specificity.
    assert_eq!(qy17_style("@layer a{p{color:green}} @layer b{p{color:red}}", h).color, red);
    assert_eq!(qy17_style("@layer a{#a{color:red}} @layer b{p{color:green}}", h).color, green);
    // Same layer: specificity then order as usual.
    assert_eq!(qy17_style("@layer a{#a{color:green} p{color:red}}", h).color, green);
    // Anonymous layers are separate, ordered layers.
    assert_eq!(qy17_style("@layer{p{color:red}} @layer{p{color:green}}", h).color, green);
}

#[test]
fn qy17_layer_statement_fixes_order_and_blocks_reopen() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    assert_eq!(qy17_style("@layer a, b; @layer b{p{color:green}} @layer a{p{color:red}}", h).color, green);
    // Without the statement the first block mentioned is the earlier layer.
    assert_eq!(qy17_style("@layer b{p{color:green}} @layer a{p{color:red}}", h).color, red);
    // A block reopens an earlier layer without moving it.
    assert_eq!(qy17_style("@layer a{p{color:green}} @layer b{p{color:red}} @layer a{p{color:green}}", h).color, red);
}

#[test]
fn qy17_important_reverses_layer_order() {
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    // Earlier layer's important beats a later layer's important.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} @layer b{p{color:red!important}}", h).color, green);
    // Layered important beats unlayered important.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} p{color:red!important}", h).color, green);
    // Any important beats a normal unlayered declaration.
    assert_eq!(qy17_style("@layer a{p{color:green!important}} p{color:red}", h).color, green);
    // Specificity within a layer still orders important declarations.
    assert_eq!(qy17_style("@layer a{#x{color:green!important} p{color:red!important}}", r#"<p id="x">x</p>"#).color, green);
}

#[test]
fn qy17_nested_layers_and_media_supports_nesting() {
    let red = TextColor::rgb(255, 0, 0);
    let green = TextColor::rgb(0, 128, 0);
    let h = "<p>x</p>";
    // `a.b` is a sub-layer of `a`: it loses to rules directly in `a`.
    assert_eq!(qy17_style("@layer a{@layer b{p{color:red}} p{color:green}}", h).color, green);
    assert_eq!(qy17_style("@layer a.b{p{color:red}} @layer a{p{color:green}}", h).color, green);
    // Sub-layers order within their parent; the parent's order decides across parents.
    assert_eq!(qy17_style("@layer x.q, y; @layer y{p{color:green}} @layer x.q{p{color:red}}", h).color, green);
    assert_eq!(qy17_style("@layer a.p, a.q; @layer a.q{p{color:green}} @layer a.p{p{color:red}}", h).color, green);
    // Layer inside @media / @supports, and @media inside a layer.
    assert_eq!(qy17_style("@media screen{@layer a{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@media print{@layer a{p{color:red}}}", h).color, ROOT_STYLE.color);
    assert_eq!(qy17_style("@supports (display:block){@layer a{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@layer a{@media screen{p{color:red}}}", h).color, red);
    assert_eq!(qy17_style("@layer a{@media print{p{color:red}}}", h).color, ROOT_STYLE.color);
    assert_eq!(qy17_style("@layer a{@media screen{p{color:red}}} p{color:green}", h).color, green);
}

#[test]
fn qy17_hidden_attribute_important_inside_a_layer_hides_the_element() {
    let css = "@layer primer{template,[hidden]{display:none!important}} .x{display:flex}";
    let s = qy17_style(css, r#"<p hidden class="x">x</p>"#);
    assert!(matches!(s.display, Display::None));
    let s = qy17_style(css, r#"<p class="x">x</p>"#);
    assert!(!matches!(s.display, Display::None));
}

#[test]
fn qy17_user_agent_rules_stay_below_author_rules() {
    // The UA sheet gives <p> margins; a layered, low-specificity author rule
    // still beats it (UA is its own, lower origin).
    let doc = parse_html("<p>x</p>");
    let p = find_element_by_tag(&doc, "p").unwrap();
    let mut rules = parse_ua_stylesheet("p{margin-top:20px}");
    rules.extend(parse_stylesheet("@layer a{p{margin-top:3px}}"));
    let s = resolve_style(&p, &rules, ROOT_STYLE, 800.0);
    assert!(matches!(s.margin.top, BoxLength::Px(v) if v == 3.0));
}

#[test]
fn qy17_import_layer_wraps_the_imported_sheet() {
    let base = Url::parse("https://e.com/css/main.css").unwrap();
    let mut fetch = |u: &Url| -> Option<String> {
        match u.path() {
            "/css/a.css" => Some("p{color:red}".to_string()),
            _ => None,
        }
    };
    let out = expand_css_imports("@import url(a.css) layer(x.y) screen;\n@import \"a.css\" layer;", &base, &mut vec![base.clone()], &mut fetch);
    assert!(out.contains("@layer x.y {\n@media screen {\np{color:red}"), "{out}");
    assert!(out.contains("@layer {\np{color:red}"), "{out}");
    // Layered import loses to an unlayered rule even when written after it.
    let out = expand_css_imports("@import url(a.css) layer(x);\np{color:green}", &base, &mut vec![base.clone()], &mut fetch);
    assert_eq!(qy17_style(&out, "<p>x</p>").color, TextColor::rgb(0, 128, 0));
}

// ---------- QF11: navigation from JS (location.href/assign/replace/reload) ----------

/// Runs `code` on a page at https://page.test/dir/index.html and returns the
/// requests it made of the App plus the log.
fn qf11_run(code: &str) -> (crate::js_dom::JsRequests, Vec<String>) {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let (engine, _doc, log) = events_engine("<html><body></body></html>", code);
    let reqs = engine.take_requests();
    let l = log.borrow().clone();
    (reqs, l)
}

#[test]
fn qf11_href_assign_replace_reload_request_navigation() {
    let (r, l) = qf11_run("location.href = 'next.html';");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/dir/next.html"), "{l:?}");
    assert!(!r.nav_replace && !r.reload);
    let (r, _) = qf11_run("location.assign('/abs?x=1');");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/abs?x=1"));
    assert!(!r.nav_replace);
    let (r, _) = qf11_run("location.replace('//other.test/p');");
    assert_eq!(r.nav.as_deref(), Some("https://other.test/p"));
    assert!(r.nav_replace, "replace must not add a history entry");
    let (r, _) = qf11_run("location.reload();");
    assert!(r.reload && r.nav.is_none());
    let (r, _) = qf11_run("window.location = '/w';");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/w"));
    let (r, _) = qf11_run("document.location = '/d';");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/d"));
    let (r, _) = qf11_run("location.pathname = '/np';");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/np"));
    let (r, _) = qf11_run("location.search = 'q=2';");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/dir/index.html?q=2"));
    let (r, _) = qf11_run("window.open('/self', '_self');");
    assert_eq!(r.nav.as_deref(), Some("https://page.test/self"));
}

#[test]
fn qf11_fragment_change_stays_in_document_and_fires_hashchange() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_with_timers(
        "window.addEventListener('hashchange', function(e){ console.log('hc', e.oldURL, e.newURL); });\
         var n = history.length; location.hash = 'sec'; console.log(location.href, history.length - n);\
         location.assign('#two'); console.log(location.hash);\
         location.replace('#three'); console.log(location.hash, history.length - n);",
    );
    assert_eq!(log[0], "log:https://page.test/dir/index.html#sec 1", "{log:?}");
    assert_eq!(log[1], "log:#two", "{log:?}");
    assert_eq!(log[2], "log:#three 2", "{log:?}");
    assert!(log.iter().any(|l| l == "log:hc https://page.test/dir/index.html https://page.test/dir/index.html#sec"), "{log:?}");
    assert_eq!(log.iter().filter(|l| l.starts_with("log:hc ")).count(), 3, "{log:?}");
}

#[test]
fn qf11_push_replace_state_do_not_navigate() {
    let (r, l) = qf11_run("history.pushState({}, '', '/x'); history.replaceState({}, '', '/y?z=1'); console.log(location.href);");
    assert!(r.nav.is_none() && !r.reload, "{r:?}");
    assert_eq!(l[0], "log:https://page.test/y?z=1");
}

#[test]
fn qf11_javascript_urls_invalid_urls_and_file_targets() {
    let (r, l) = qf11_run("location.href = 'javascript:alert(1)'; console.log('alive');");
    assert!(r.nav.is_none());
    assert!(l[0].contains("javascript: URL navigation"), "{l:?}");
    assert_eq!(l[1], "log:alive");
    let (r, l) = qf11_run("try { location.assign('http://'); } catch (e) { console.log(e.name); }");
    assert!(r.nav.is_none());
    assert_eq!(l[0], "log:SyntaxError");
    let (r, l) = qf11_run("location.href = 'file:///etc/passwd';");
    assert!(r.nav.is_none());
    assert!(l[0].contains("refused: file:"), "{l:?}");
}

// ---------- QY48: pushState/replaceState reach the App URL and history ----------

fn qy48_app(script: &str) -> Option<(App, Handle)> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html("<html><body><p id=p>x</p></body></html>");
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/start").unwrap());
    set_page_url(app.base_url.as_ref());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    Some((app, doc))
}

const QY48_POP: &str = "window.L=[]; addEventListener('popstate',function(e){L.push('pop:'+JSON.stringify(e.state)+':'+location.pathname);});";

fn qy48_eval(app: &mut App, code: &str) {
    app.js.as_ref().unwrap().eval_value::<bool>("t", &format!("{code};true"));
    app.js_sync();
}

fn qy48_run_timers(app: &mut App) {
    std::thread::sleep(std::time::Duration::from_millis(10));
    app.js.as_ref().unwrap().run_due();
    app.js_sync();
}

#[test]
fn qy48_pushstate_updates_app_url_and_adds_entry_without_navigating() {
    let Some((mut app, _doc)) = qy48_app(QY48_POP) else { return };
    app.js_sync();
    qy48_eval(&mut app, "history.pushState({n:1},'','/results?q=rust')");
    assert_eq!(app.history.iter().map(Url::as_str).collect::<Vec<_>>(), ["https://e.org/start", "https://e.org/results?q=rust"]);
    assert_eq!(app.history_index, 1);
    assert_eq!(app.base_url.as_ref().map(Url::as_str), Some("https://e.org/results?q=rust"));
    assert_eq!(app.nav_generation, 0, "no fetch");
    qy48_eval(&mut app, "history.replaceState({n:2},'','/results?q=rust&p=2')");
    assert_eq!(app.history.len(), 2, "replaceState adds no entry");
    assert_eq!(app.history[1].as_str(), "https://e.org/results?q=rust&p=2");
    assert_eq!(app.nav_generation, 0);
    // pushState to the very same URL still adds an entry.
    qy48_eval(&mut app, "history.pushState({n:3},'','/results?q=rust&p=2')");
    assert_eq!(app.history.len(), 3);
}

#[test]
fn qy48_back_forward_between_same_document_entries_fire_popstate_without_reload() {
    let Some((mut app, _doc)) = qy48_app(QY48_POP) else { return };
    qy48_eval(&mut app, "history.pushState({n:1},'','/a'); history.pushState({n:2},'','/b')");
    assert_eq!(app.history.len(), 3);
    app.navigate_history(-1);
    assert_eq!(app.nav_generation, 0, "no reload");
    assert_eq!(app.history_index, 1);
    assert_eq!(app.base_url.as_ref().map(Url::as_str), Some("https://e.org/a"));
    qy48_run_timers(&mut app);
    assert_eq!(q30e_log(&app), "pop:{\"n\":1}:/a");
    app.navigate_history(-1);
    qy48_run_timers(&mut app);
    assert_eq!(q30e_log(&app), "pop:{\"n\":1}:/a|pop:null:/start");
    assert_eq!(app.nav_generation, 0);
    app.navigate_history(1);
    app.navigate_history(1);
    qy48_run_timers(&mut app);
    assert!(q30e_log(&app).ends_with("pop:{\"n\":2}:/b"), "{}", q30e_log(&app));
    assert_eq!(app.history_index, 2);
    assert_eq!(app.nav_generation, 0);
}

#[test]
fn qy48_cross_document_entry_still_reloads() {
    let Some((mut app, _doc)) = qy48_app("window.L=[];") else { return };
    // Entry 0 belongs to an earlier document: back to it must fetch.
    app.history = vec![Url::parse("https://e.org/old").unwrap(), Url::parse("https://e.org/start").unwrap()];
    app.history_doc = vec![7, app.doc_gen];
    app.history_index = 1;
    qy48_eval(&mut app, "history.pushState({}, '', '/p')");
    assert_eq!(app.history.len(), 3);
    app.navigate_history(-1);
    assert_eq!(app.nav_generation, 0, "same-document step");
    app.navigate_history(-1);
    assert_eq!(app.nav_generation, 1, "crossing into another document fetches");
    assert_eq!(app.history_index, 0);
}

#[test]
fn qy48_fragment_after_pushstate_stays_in_document() {
    let Some((mut app, _doc)) = qy48_app("window.L=[];") else { return };
    qy48_eval(&mut app, "history.pushState({}, '', '/spa'); location.hash = 'sec'");
    assert_eq!(app.nav_generation, 0, "no fetch for a fragment change after pushState");
    assert_eq!(app.history.last().map(Url::as_str), Some("https://e.org/spa#sec"));
    assert_eq!(app.history.len(), 3);
    assert_eq!(app.base_url.as_ref().map(Url::as_str), Some("https://e.org/spa"));
}

#[test]
fn qy48_page_initiated_back_moves_app_index() {
    let Some((mut app, _doc)) = qy48_app("window.L=[];") else { return };
    qy48_eval(&mut app, "history.pushState({}, '', '/a'); history.pushState({}, '', '/b')");
    qy48_eval(&mut app, "history.back()");
    qy48_run_timers(&mut app);
    assert_eq!(app.history_index, 1);
    assert_eq!(app.base_url.as_ref().map(Url::as_str), Some("https://e.org/a"));
    assert_eq!(app.nav_generation, 0);
}
