//! QY54b1: `<link rel=stylesheet>` inserted, removed or changed by script after load.
use super::*;
use crate::link_css::{apply_sync, LinkSheets};

const SHEET: &str = "#d{position:absolute;height:10px;width:30px}";

/// The page's external CSS as the App holds it, plus the link state over it.
struct Page {
    engine: JsEngine,
    doc: Handle,
    log: Rc<RefCell<Vec<String>>>,
    css: String,
    sheets: LinkSheets,
}

impl Page {
    fn new(html: &str, code: &str, css: &str, parts: crate::link_css::SheetParts) -> Page {
        crate::js_style::set_js_extra_css(css);
        let doc = parse_html(html);
        let sheets = LinkSheets::adopt(&doc, None, css.len(), parts);
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let engine = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
        Page { engine, doc, log: out, css: css.to_string(), sheets }
    }

    /// What `App::js_sync` does after a script turn: re-sync when a link changed. Returns the fetches.
    fn turn(&mut self) -> Vec<String> {
        if !self.engine.take_requests().links_changed {
            return Vec::new();
        }
        self.sync()
    }

    fn sync(&mut self) -> Vec<String> {
        let mut out = self.sheets.sync(&self.doc, None, &self.css);
        let fetch = std::mem::take(&mut out.fetch);
        apply_sync(&mut out, &mut self.css, Some(&self.engine));
        fetch
    }

    fn arrive(&mut self, href: &str, css: Option<&str>) {
        self.sheets.settle(href, css.map(str::to_string));
        assert!(self.sync().is_empty());
    }

    fn eval(&self, code: &str) {
        self.engine.eval("t", code);
    }

    fn logged(&self, needle: &str) -> usize {
        self.log.borrow().iter().filter(|l| l.contains(needle)).count()
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        crate::js_style::set_js_extra_css("");
    }
}

#[test]
fn qy54b1_script_link_applies_after_load_and_removal_restores() {
    let mut p = Page::new(
        "<html><head></head><body><div id=d>text</div></body></html>",
        "var d=document.getElementById('d');
         console.log('before '+d.getBoundingClientRect().height+' '+getComputedStyle(d).position);
         var l=document.createElement('link'); l.rel='stylesheet'; l.href='t.css';
         l.onload=function(){console.log('load '+d.getBoundingClientRect().height+' '+getComputedStyle(d).position)};
         l.onerror=function(){console.log('error')};
         document.head.appendChild(l); window.L=l;",
        "",
        Vec::new(),
    );
    assert_eq!(p.logged("before "), 1);
    assert_eq!(p.logged("before 10 "), 0, "{:?}", p.log.borrow());
    assert_eq!(p.turn(), ["t.css"], "a connected stylesheet link is fetched");
    assert!(p.sheets.is_pending("t.css"));
    assert_eq!(p.logged("load"), 0, "nothing fires before the sheet arrives");
    p.arrive("t.css", Some(SHEET));
    assert_eq!(p.css, format!("{SHEET}\n"));
    assert_eq!(p.logged("load 10 absolute"), 1, "{:?}", p.log.borrow());
    // Removing the link removes its CSS; putting it back reuses the text (no fetch, no second load).
    p.eval("L.remove(); console.log('removed '+document.getElementById('d').getBoundingClientRect().height);");
    assert!(p.turn().is_empty());
    assert_eq!(p.css, "");
    p.eval("console.log('after '+getComputedStyle(document.getElementById('d')).position);");
    assert_eq!(p.logged("after static"), 1, "{:?}", p.log.borrow());
    p.eval("document.head.appendChild(L); console.log('back '+getComputedStyle(document.getElementById('d')).position);");
    assert!(p.turn().is_empty());
    assert_eq!(p.css, format!("{SHEET}\n"));
    p.eval("console.log('again '+getComputedStyle(document.getElementById('d')).position);");
    assert_eq!(p.logged("again absolute"), 1);
    assert_eq!(p.logged("load"), 1, "one load per element and href");
    assert_eq!(p.logged("error"), 0);
}

#[test]
fn qy54b1_failed_sheet_fires_error_and_changes_nothing() {
    let mut p = Page::new(
        "<html><head></head><body><div id=d>text</div></body></html>",
        "var l=document.createElement('link'); l.setAttribute('rel','StyleSheet'); l.setAttribute('href','missing.css');
         l.addEventListener('load',function(){console.log('load')});
         l.addEventListener('error',function(e){console.log('error '+e.bubbles)});
         document.head.appendChild(l);",
        "",
        Vec::new(),
    );
    assert_eq!(p.turn(), ["missing.css"]);
    p.arrive("missing.css", None);
    assert_eq!(p.css, "");
    assert_eq!(p.logged("error false"), 1, "{:?}", p.log.borrow());
    assert_eq!(p.logged("load"), 0);
    // A second link to the same failed href is not refetched; it fires nothing while the first is
    // there, and errors at once when it becomes the first.
    p.eval("var m=document.createElement('link'); m.rel='stylesheet'; m.href='missing.css'; m.onerror=function(){console.log('m-error')}; document.body.appendChild(m);");
    assert!(p.turn().is_empty());
    assert_eq!(p.logged("m-error"), 0);
    p.eval("document.querySelector('link').remove();");
    assert!(p.turn().is_empty());
    assert_eq!(p.logged("m-error"), 1);
}

#[test]
fn qy54b1_link_inserted_twice_applies_once() {
    let mut p = Page::new(
        "<html><head></head><body><div id=d>text</div></body></html>",
        "var l=document.createElement('link'); l.rel='stylesheet'; l.href='t.css';
         l.onload=function(){console.log('load')};
         document.head.appendChild(l); document.head.appendChild(l); document.body.appendChild(l);",
        "",
        Vec::new(),
    );
    assert_eq!(p.turn(), ["t.css"], "one fetch");
    p.eval("document.head.appendChild(L=document.querySelector('link'));");
    assert!(p.turn().is_empty(), "already in flight");
    p.arrive("t.css", Some(SHEET));
    assert_eq!(p.css.matches("#d{").count(), 1, "{}", p.css);
    assert_eq!(p.logged("load"), 1);
    p.eval("document.head.appendChild(document.querySelector('link'));");
    p.turn();
    assert_eq!(p.css.matches("#d{").count(), 1);
    assert_eq!(p.logged("load"), 1);
}

#[test]
fn qy54b1_parsed_links_keep_their_text_order_and_disabled_drops_a_sheet() {
    // The parsed page: a.css and b.css (failed) fetched before load, as `fetch_stylesheet_parts` gives.
    let a = "#d{height:20px}";
    let css = format!("{a}\n");
    let parts = vec![("a.css".to_string(), Some(0..a.len())), ("b.css".to_string(), None)];
    let mut p = Page::new(
        "<html><head><link rel=stylesheet href=a.css><link rel=stylesheet href=b.css></head><body><div id=d>x</div></body></html>",
        "var a=document.querySelector('link'); a.onload=function(){console.log('parsed-load')};
         var n=document.createElement('link'); n.rel='stylesheet'; n.href='n.css';
         document.head.insertBefore(n, a);",
        &css,
        parts,
    );
    assert_eq!(p.turn(), ["n.css"], "parsed links are not fetched again");
    assert_eq!(p.css, css, "unchanged until the new sheet arrives");
    p.arrive("n.css", Some("#d{height:5px}"));
    assert_eq!(p.css, format!("#d{{height:5px}}\n{a}\n"), "document position: before the parsed link");
    assert_eq!(p.logged("parsed-load"), 0, "parsed links fire nothing (as before)");
    // `disabled` takes a's sheet out; turning it off puts it back without a fetch.
    p.eval("document.querySelectorAll('link')[1].disabled=true;");
    assert!(p.turn().is_empty());
    assert_eq!(p.css, "#d{height:5px}\n");
    p.eval("console.log('h '+document.getElementById('d').getBoundingClientRect().height);");
    assert_eq!(p.logged("h 5"), 1, "{:?}", p.log.borrow());
    p.eval("document.querySelectorAll('link')[1].disabled=false;");
    assert!(p.turn().is_empty());
    assert_eq!(p.css, format!("#d{{height:5px}}\n{a}\n"));
    // href change: the new href is fetched, the old text goes.
    p.eval("document.querySelectorAll('link')[0].href='m.css';");
    assert_eq!(p.turn(), ["m.css"]);
    assert_eq!(p.css, format!("{a}\n"));
    // rel change away from stylesheet drops it.
    p.eval("document.querySelectorAll('link')[1].rel='preload';");
    p.turn();
    assert_eq!(p.css, "");
}

#[test]
fn qy54b1_unchanged_links_and_other_writes_do_not_resync() {
    let a = "#d{height:20px}";
    let css = format!("{a}\n");
    let mut p = Page::new(
        "<html><head><link rel=stylesheet href=a.css></head><body><div id=d>x</div></body></html>",
        "var d=document.getElementById('d'); d.className='x'; d.appendChild(document.createElement('span'));
         var l=document.createElement('link'); l.rel='stylesheet'; l.href='q.css';
         document.createElement('div').appendChild(l);",
        &css,
        vec![("a.css".to_string(), Some(0..a.len()))],
    );
    assert!(!p.engine.take_requests().links_changed, "no connected link changed");
    let out = p.sheets.sync(&p.doc, None, &p.css);
    assert!(out.css.is_none() && out.fetch.is_empty() && out.fire.is_empty());
    // innerHTML with a link counts.
    p.eval("document.body.insertAdjacentHTML('beforeend', '<LINK rel=stylesheet href=h.css>');");
    assert_eq!(p.turn(), ["h.css"]);
}

#[test]
fn qy54b1_opaque_css_keeps_its_parsed_links_and_appends_new_ones() {
    // A page installed without parts (streamed page): its text stays first, its links are not refetched.
    let css = "#d{height:20px}\n";
    let mut p = Page::new(
        "<html><head><link rel=stylesheet href=a.css></head><body><div id=d>x</div></body></html>",
        "var n=document.createElement('link'); n.rel='stylesheet'; n.href='n.css'; document.head.prepend(n);",
        css,
        Vec::new(),
    );
    assert_eq!(p.turn(), ["n.css"]);
    p.arrive("n.css", Some("p{}"));
    assert_eq!(p.css, format!("{css}p{{}}\n"));
}

#[test]
fn qy54b1_parsed_disabled_link_is_not_fetched() {
    let doc = parse_html("<html><head><link rel=stylesheet href=a.css disabled><link rel='alternate STYLESHEET' href=b.css><link rel=icon href=c.css></head></html>");
    let mut res = ExternalResources::default();
    find_external_resources(&doc, &mut res);
    assert_eq!(res.stylesheets, ["b.css"]);
}

#[test]
fn qy54b1_a_second_link_to_an_applied_href_is_not_applied_again() {
    // YouTube watch: www-player.css is parsed, then a script inserts another link to it after the
    // main CSS. The sheet stays at its first place and the second link fires nothing (link_css docs).
    let a = "#d{height:20px}";
    let m = "#d{height:7px}";
    let css = format!("{a}\n{m}\n");
    let parts = vec![("a.css".to_string(), Some(0..a.len())), ("m.css".to_string(), Some(a.len() + 1..a.len() + 1 + m.len()))];
    let mut p = Page::new(
        "<html><head><link rel=stylesheet href=a.css><link rel=stylesheet href=m.css></head><body><div id=d>x</div></body></html>",
        "var l=document.createElement('link'); l.rel='stylesheet'; l.href='a.css';
         l.onload=function(){console.log('dup-load')}; document.head.appendChild(l);",
        &css,
        parts,
    );
    assert!(p.turn().is_empty(), "no fetch for an href already applied");
    assert_eq!(p.css, css, "unchanged: no second copy after m.css");
    assert_eq!(p.logged("dup-load"), 0, "a duplicate link fires nothing");
    // Removing the first link moves the sheet to the duplicate's place; now it is the one that loads.
    p.eval("document.querySelector('link').remove();");
    p.turn();
    assert_eq!(p.css, format!("{m}\n{a}\n"));
    assert_eq!(p.logged("dup-load"), 1);
}

#[test]
fn qy54b1_link_keys_are_resolved_urls() {
    let base = Url::parse("https://www.example.test/watch?v=1").unwrap();
    let k = crate::link_css::link_key(Some(&base), "/s/player/www-player.css");
    assert_eq!(k, "https://www.example.test/s/player/www-player.css");
    assert_eq!(crate::link_css::link_key(Some(&base), " https://www.example.test/s/player/www-player.css "), k);
    assert_eq!(crate::link_css::link_key(Some(&base), "data:text/css,p{}"), "data:text/css,p{}");
    assert_eq!(crate::link_css::link_key(None, "rel.css"), "rel.css");
    // A parsed absolute link and a script link to the same URL written relatively share one sheet.
    let doc = parse_html("<html><head><link rel=stylesheet href='https://www.example.test/a.css'><link rel=stylesheet href='/a.css'></head></html>");
    let css = "p{}\n";
    let mut s = LinkSheets::adopt(&doc, Some(&base), css.len(), vec![("https://www.example.test/a.css".to_string(), Some(0..3))]);
    let out = s.sync(&doc, Some(&base), css);
    assert!(out.css.is_none() && out.fetch.is_empty() && out.fire.is_empty());
}
