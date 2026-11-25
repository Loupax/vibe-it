#[allow(unused_imports)]
use super::*;

fn logs_of(log: &[String]) -> Vec<&str> {
    log.iter().filter_map(|l| l.strip_prefix("log:")).collect()
}

#[test]
fn qapi7_broadcast_channel_same_realm_and_close() {
    crate::js::set_page_url(Url::parse("https://qapi7.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var a=new BroadcastChannel('c'), b=new BroadcastChannel('c'), o=new BroadcastChannel('other'), got=[];\
         console.log(a.name, typeof a.close, a instanceof EventTarget, a.onmessage, a.onmessageerror);\
         b.onmessage=function(e){ got.push('b:'+JSON.stringify(e.data)+':'+(e instanceof MessageEvent)); };\
         o.onmessage=function(){ got.push('o'); };\
         a.onmessage=function(){ got.push('self'); };\
         var m={x:[1,2]}; a.postMessage(m); m.x.push(3);\
         try { a.postMessage(function(){}); } catch(e) { console.log('clone', e.name); }\
         try { new BroadcastChannel(); } catch(e) { console.log('ctor', e.name); }\
         setTimeout(function(){ b.close(); a.postMessage('late');\
           try { b.postMessage(1); } catch(e) { console.log('closed', e.name, e instanceof DOMException); } }, 20);\
         setTimeout(function(){ console.log(got.join('|')); }, 60);",
    );
    let l = logs_of(&log);
    assert_eq!(l[0], "c function true null null", "{log:?}");
    assert!(l.contains(&"clone DataCloneError"), "{log:?}");
    assert!(l.contains(&"ctor TypeError"), "{log:?}");
    assert!(l.contains(&"closed InvalidStateError true"), "{log:?}");
    assert_eq!(*l.last().unwrap(), "b:{\"x\":[1,2]}:true", "{log:?}");
}

#[test]
fn qapi7_broadcast_channel_reaches_same_origin_iframe() {
    crate::js::set_page_url(Url::parse("https://qapi7.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         var inner=new w.BroadcastChannel('x'), outer=new BroadcastChannel('x'), got=[];\
         inner.addEventListener('message', function(e){ got.push('in:'+e.data.v+':'+(e instanceof w.MessageEvent)); inner.postMessage({v:'back'}); });\
         outer.onmessage=function(e){ got.push('out:'+e.data.v); };\
         outer.postMessage({v:'go'});\
         setTimeout(function(){ console.log(got.join(',')); }, 60);",
    );
    assert_eq!(logs_of(&log)[0], "in:go:true,out:back", "{log:?}");
}

#[test]
fn qapi7_content_editable_reflection() {
    let log = run_html_with_timers(
        "<html><body><div id=a contenteditable><p id=p><b id=b>x</b></p></div><div id=n contenteditable=false><i id=i></i></div><div id=z></div></body></html>",
        "var $=function(i){return document.getElementById(i);};\
         console.log($('a').contentEditable, $('n').contentEditable, $('z').contentEditable, $('b').isContentEditable, $('i').isContentEditable, $('z').isContentEditable);\
         $('z').contentEditable='TRUE'; console.log($('z').getAttribute('contenteditable'), $('z').isContentEditable);\
         $('z').contentEditable='plaintext-only'; console.log($('z').contentEditable);\
         $('z').contentEditable='inherit'; console.log($('z').hasAttribute('contenteditable'), $('z').contentEditable);\
         try { $('z').contentEditable='bogus'; } catch(e) { console.log(e.name); }\
         console.log('ontouchstart' in window, 'ontouchstart' in document, document.body.ontouchstart);",
    );
    let l = logs_of(&log);
    assert_eq!(l[0], "true false inherit true false false", "{log:?}");
    assert_eq!(l[1], "true true", "{log:?}");
    assert_eq!(l[2], "plaintext-only", "{log:?}");
    assert_eq!(l[3], "false inherit", "{log:?}");
    assert_eq!(l[4], "SyntaxError", "{log:?}");
    assert_eq!(l[5], "false false undefined", "{log:?}");
}
