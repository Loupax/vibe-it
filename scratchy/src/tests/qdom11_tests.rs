// QDOM11: a response with an XML MIME type is parsed with roxmltree (not html5ever) for the top-level document and for
// iframes; the document is not an HTML document; XHTML and SVG `script` elements run in tree order (CDATA, SVG `href`);
// a well-formedness error gives the `parsererror` document.
use super::*;

fn logs(log: &[String]) -> Vec<String> {
    log.iter().filter(|l| l.starts_with("log:") || l.starts_with("error:")).map(|l| l.strip_prefix("log:").unwrap_or(l).to_string()).collect()
}

/// Runs the scripts of the XML document `xml` (served as `mime`) as a top-level page, then `extra` in a script of its own.
#[allow(clippy::let_and_return)]
fn run_xml(mime: &str, xml: &str, extra: &str) -> Vec<String> {
    crate::js::set_page_url(Url::parse("http://qdom11.test/a.xhtml").ok().as_ref());
    crate::js::set_page_xml_mime(Some(mime));
    let doc = crate::js_dom::parse_xml_document(xml, "http://qdom11.test/a.xhtml");
    let mut scripts = crate::js::inline_scripts(&doc);
    scripts.push(src(extra));
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(&doc, &scripts, Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))), Some(clock)).unwrap();
    drop(engine);
    let v = out.borrow().clone();
    v
}

#[allow(clippy::let_and_return)]
fn run_frames(js: &str) -> Vec<String> {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", js);
    for _ in 0..20 {
        qf7_drive(&engine, &fake);
        std::thread::sleep(std::time::Duration::from_millis(5));
        engine.run_due();
    }
    let out = log.borrow().clone();
    out
}

#[test]
fn qdom11_xml_mime_types() {
    use crate::js_dom::is_xml_mime;
    for m in ["application/xhtml+xml", "image/svg+xml", "text/xml", "application/xml", "application/rss+xml; charset=utf-8", "TEXT/XML"] {
        assert!(is_xml_mime(m), "{m}");
    }
    for m in ["text/html", "text/plain", "application/json", "", "+xml", "image/png", "text/xmlx"] {
        assert!(!is_xml_mime(m), "{m}");
    }
}

#[test]
fn qdom11_xhtml_page_is_not_html_and_scripts_run_in_tree_order() {
    let xml = "<html xmlns='http://www.w3.org/1999/xhtml'><head><title>t</title></head><body>\
        <script>console.log('1 ' + document.contentType + ' ' + (document instanceof XMLDocument) + ' ' + document.createElement('DIV').localName);</script>\
        <svg xmlns='http://www.w3.org/2000/svg'><script><![CDATA[ if (1 < 2) console.log('2 svg-cdata'); ]]></script></svg>\
        <foo xmlns=''><script>console.log('never: no namespace');</script></foo>\
        <script type='text/plain'>console.log('never: not javascript');</script>\
        <script>console.log('3 ' + document.getElementsByTagName('script').length + ' ' + document.documentElement.tagName);</script>\
        </body></html>";
    let l = logs(&run_xml("application/xhtml+xml", xml, "console.log('4 last ' + document.currentScript);"));
    assert_eq!(l, ["1 application/xhtml+xml true DIV", "2 svg-cdata", "3 5 html", "4 last null"], "{l:?}");
}

#[test]
fn qdom11_svg_document_and_plain_xml() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg'><script>console.log('svg ' + document.contentType + ' ' + document.documentElement.localName + ' ' + document.createElement('A').localName);</script></svg>";
    assert_eq!(logs(&run_xml("image/svg+xml", svg, "")), ["svg image/svg+xml svg A"]);
    let xml = "<?xml version='1.0'?><!--c--><r xmlns:h='http://www.w3.org/1999/xhtml'><h:script>console.log('prefixed ' + document.contentType);</h:script></r>";
    assert_eq!(logs(&run_xml("text/xml", xml, "console.log(document.firstChild.nodeType + ' ' + document.childNodes.length);")), ["prefixed text/xml", "8 2"]);
}

#[test]
fn qdom11_doctype_node_and_entities() {
    let xml = "<?xml version='1.0'?><!--a--><!DOCTYPE html PUBLIC '-//X//Y' 'x.dtd' [ <!ENTITY e 'ent'> <!-- in subset --> ]><html xmlns='http://www.w3.org/1999/xhtml'><body>&e;</body></html>";
    let l = logs(&run_xml(
        "application/xhtml+xml",
        xml,
        "var d = document.childNodes; console.log(d.length, d[0].nodeType, d[1].nodeType, d[1].name, d[1].publicId, d[1].systemId, d[2].nodeType, document.body.textContent);",
    ));
    assert_eq!(l, ["3 8 10 html -//X//Y x.dtd 1 ent"], "{l:?}");
}

#[test]
fn qdom11_well_formedness_error_gives_the_parsererror_document() {
    let l = logs(&run_xml(
        "application/xhtml+xml",
        "<html xmlns='http://www.w3.org/1999/xhtml'><body><script>console.log('never')</script><p></body></html>",
        "var r = document.documentElement; console.log(r.localName, r.namespaceURI, document.childNodes.length, r.textContent.indexOf('XML Parsing Error') === 0, r.firstElementChild.localName);",
    ));
    assert_eq!(l, ["parsererror http://www.mozilla.org/newlayout/xml/parsererror.xml 1 true sourcetext"], "{l:?}");
}

#[test]
fn qdom11_html_page_is_unchanged() {
    // The HTML path keeps its flags: contentType text/html, upper-case tag names, scripts run.
    let l = logs(&run_html_with_timers("<html><body><p id=a></p></body></html>", "console.log(document.contentType, document.createElement('p').tagName, document.getElementById('a').tagName);"));
    assert_eq!(l, ["text/html P P"], "{l:?}");
}

#[test]
fn qdom11_iframe_xhtml_blob_is_parsed_as_xml_and_its_scripts_run() {
    let l = run_frames(
        "var x = '<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p id=\"x\"/><script>window.ran = [document.contentType, document.createElement(\"B\").localName].join();</script>\
          <svg xmlns=\"http://www.w3.org/2000/svg\"><script><![CDATA[ window.svg = 1 < 2; ]]></script></svg><foo xmlns=\"\"><script>window.bad = 1;</script></foo></body></html>';\
         var u = URL.createObjectURL(new Blob([x], {type: 'application/xhtml+xml'}));\
         var f = document.createElement('iframe'); f.src = u; f.onload = function () { var w = f.contentWindow, d = f.contentDocument;\
           console.log('loaded', d.contentType, d instanceof w.XMLDocument, d.getElementById('x').localName, w.ran, w.svg, w.bad, d.getElementsByTagName('script').length); };\
         document.body.appendChild(f);",
    );
    assert!(l.contains(&"log:loaded application/xhtml+xml true p application/xhtml+xml,B true undefined 3".to_string()), "{l:?}");
}

#[test]
fn qdom11_iframe_svg_and_malformed_xml_blobs() {
    let l = run_frames(
        "var mk = function (type, text, tag) { var u = URL.createObjectURL(new Blob([text], {type: type})); var f = document.createElement('iframe'); f.src = u;\
           f.onload = function () { var d = f.contentDocument; console.log(tag, d.contentType, d.documentElement.localName, d.documentElement.namespaceURI); }; document.body.appendChild(f); };\
         mk('image/svg+xml', '<svg xmlns=\"http://www.w3.org/2000/svg\"/>', 'svg');\
         mk('text/xml', '<a><b></a>', 'bad');\
         mk('application/xml', '<r/>', 'xml');",
    );
    assert!(l.contains(&"log:svg image/svg+xml svg http://www.w3.org/2000/svg".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad text/xml parsererror http://www.mozilla.org/newlayout/xml/parsererror.xml".to_string()), "{l:?}");
    assert!(l.contains(&"log:xml application/xml r null".to_string()), "{l:?}");
}

#[test]
fn qdom11_prefetched_xhtml_iframe() {
    let mut pages = HashMap::new();
    let mut mimes = HashMap::new();
    let u = "http://qdom11b.test/dir/child.xhtml".to_string();
    pages.insert(u.clone(), "<html xmlns='http://www.w3.org/1999/xhtml'><body><script>parent.postMessage(document.contentType + ' ' + document.body.localName, '*');</script></body></html>".to_string());
    mimes.insert(u, "application/xhtml+xml".to_string());
    crate::net::set_iframe_pages_typed(pages, mimes);
    crate::js::set_page_url(Url::parse("http://qdom11b.test/dir/page.html").ok().as_ref());
    let log = run_html_with_timers(
        "<html><body><iframe src='child.xhtml'></iframe></body></html>",
        "onmessage = function (e) { console.log('msg', e.data); }; document.querySelector('iframe').addEventListener('load', function () { console.log('type', window[0].document.contentType); });\
         setTimeout(function () { console.log('end'); }, 100);",
    );
    let l = logs(&log);
    assert!(l.contains(&"msg application/xhtml+xml body".to_string()) && l.contains(&"type application/xhtml+xml".to_string()), "{log:?}");
    crate::net::set_iframe_pages(HashMap::new());
}
