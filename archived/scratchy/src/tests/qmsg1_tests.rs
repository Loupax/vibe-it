#[allow(unused_imports)]
use super::*;

fn logs_of(log: &[String]) -> Vec<&str> {
    log.iter().filter_map(|l| l.strip_prefix("log:")).collect()
}

#[test]
fn qmsg1_target_origin_forms_and_options() {
    crate::js::set_page_url(Url::parse("https://qmsg1.test:8443/p.html").ok().as_ref());
    let log = run_with_timers(
        "var got=[]; addEventListener('message', function(e){ got.push(e.data+'@'+e.origin); });\
         postMessage('star','*'); postMessage('slash','/'); postMessage('upper','HTTPS://QMSG1.TEST:8443/');\
         postMessage('dslash','https://qmsg1.test:8443//'); postMessage('opt',{targetOrigin:'https://qmsg1.test:8443'});\
         postMessage('one'); postMessage('other','https://qmsg1.test'); postMessage('other2',{targetOrigin:'https://evil.test'});\
         try { postMessage('x','bogus'); } catch(e) { console.log('throw', e.name, e instanceof DOMException); }\
         try { postMessage('x','*',[{}]); } catch(e) { console.log('throw', e.name); }\
         setTimeout(function(){ console.log(got.join(',')); }, 20);",
    );
    let l = logs_of(&log);
    let o = "https://qmsg1.test:8443";
    assert_eq!(l[0], "throw SyntaxError true", "{log:?}");
    assert_eq!(l[1], "throw DataCloneError", "{log:?}");
    assert_eq!(
        l[2],
        format!("star@{o},slash@{o},upper@{o},dslash@{o},opt@{o},one@{o}"),
        "{log:?}"
    );
}

#[test]
fn qmsg1_transferred_ports_reach_the_event_and_order_is_kept() {
    crate::js::set_page_url(Url::parse("https://qmsg1.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow, seq=[];\
         var ch=new MessageChannel();\
         w.addEventListener('message', function(e){ seq.push(e.data+':'+e.ports.length); if (e.ports[0]) e.ports[0].postMessage('via-port'); });\
         ch.port2.onmessage=function(e){ seq.push(e.data); };\
         w.postMessage(1,'*',[ch.port1]); w.postMessage(2,'*'); w.postMessage(3,{targetOrigin:'*',transfer:[]});\
         setTimeout(function(){ console.log(seq.join(' ')); }, 30);",
    );
    assert_eq!(logs_of(&log)[0], "1:1 2:0 3:0 via-port", "{log:?}");
}

#[test]
fn qmsg1_prefetched_iframe_scripts_reply_through_postmessage() {
    let mut pages = HashMap::new();
    pages.insert(
        "http://qmsg1b.test/dir/child.html".to_string(),
        "<!doctype html><script>onmessage=function(e){ e.source.postMessage('echo '+e.data+' '+e.origin,'*'); };</script>".to_string(),
    );
    crate::net::set_iframe_pages(pages);
    crate::js::set_page_url(Url::parse("http://qmsg1b.test/dir/page.html").ok().as_ref());
    let log = run_html_with_timers(
        "<html><body><iframe src='child.html'></iframe></body></html>",
        "var f=document.querySelector('iframe');\
         onmessage=function(e){ console.log(e.data, e.source===f.contentWindow, e.origin); };\
         f.addEventListener('load', function(){ window[0].postMessage('hi','HTTP://QMSG1B.TEST/'); window[0].postMessage('no','http://evil.test'); });\
         setTimeout(function(){ console.log('end'); }, 100);",
    );
    let l = logs_of(&log);
    assert_eq!(l, ["echo hi http://qmsg1b.test true http://qmsg1b.test", "end"], "{log:?}");
}
