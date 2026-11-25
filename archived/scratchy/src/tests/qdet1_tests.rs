// QDET1: <details>/<summary>: closed details render only the first summary (or a default "Details"),
// a disclosure marker, toggle events for every `open` write, name groups, summary activation and focus.
#[allow(unused_imports)]
use super::*;
use crate::perf::{incr_get, Incr};

#[test]
fn qdet1_closed_details_hides_everything_but_the_first_summary() {
    let html = "<html><body style=\"margin:0\"><details id=d><summary id=s1 style=\"height:20px\">one</summary>\
        <summary id=s2 style=\"height:20px\">two</summary>loose text<p id=p style=\"height:100px\">body</p></details><div id=after></div></body></html>";
    let m0 = incr_get(Incr::CheckMismatch);
    let l = run_in_html(
        html,
        "var $ = function (x) { return document.getElementById(x); };\
         var y = function () { return $('after').getBoundingClientRect().y; };\
         var c0 = y(); console.log(c0, getComputedStyle($('p')).display, getComputedStyle($('s2')).display, getComputedStyle($('s1')).display);\
         $('d').open = true; var o0 = y(); console.log(o0 > c0 + 100);\
         $('d').removeAttribute('open'); console.log(y() === c0);\
         $('d').setAttribute('open', ''); console.log(y() === o0);",
    );
    assert_eq!(l[0], "log:20 none none list-item", "{l:?}");
    assert_eq!(l[1], "log:true");
    assert_eq!(l[2], "log:true");
    assert_eq!(l[3], "log:true");
    assert_eq!(incr_get(Incr::CheckMismatch), m0, "the engine agrees with the from-scratch oracle");
}

#[test]
fn qdet1_summary_less_details_has_a_default_summary() {
    let l = run_in_html(
        "<html><body style=\"margin:0\"><details id=d><p id=p style=\"height:100px\">body</p></details><div id=after></div></body></html>",
        "var $ = function (x) { return document.getElementById(x); };\
         var y = function () { return $('after').getBoundingClientRect().y; };\
         var c = y(); console.log(c > 5 && c < 60); $('d').open = true; console.log(y() > c + 90);\
         var s = document.createElement('summary'); s.style.height = '30px'; $('d').insertBefore(s, $('p'));\
         console.log(y() > c + 100);",
    );
    assert_eq!(l[0], "log:true", "{l:?}");
    assert_eq!(l[1], "log:true");
    assert_eq!(l[2], "log:true");
}

#[test]
fn qdet1_toggle_fires_for_attribute_writes_and_coalesces() {
    let l = run_html_with_timers(
        "<html><body><details id=d><summary>s</summary>x</details></body></html>",
        "var d = document.getElementById('d'), ev = [];\
         d.addEventListener('toggle', function (e) { ev.push(e.oldState + '>' + e.newState + ':' + d.open); });\
         d.addEventListener('beforetoggle', function () { ev.push('BEFORE'); });\
         d.setAttribute('open', ''); d.setAttribute('open', 'x'); d.open = true;\
         setTimeout(function () { d.removeAttribute('open'); d.toggleAttribute('open'); d.removeAttribute('open');\
           setTimeout(function () { d.removeAttribute('open'); console.log(ev.join(' ')); }, 20); }, 20);",
    );
    // One event for the first batch (closed > open); the second batch ends where it began: oldState open, newState closed.
    assert_eq!(l[0], "log:closed>open:true open>closed:false", "{l:?}");
}

#[test]
fn qdet1_name_group_is_exclusive() {
    let l = run_html_with_timers(
        "<html><body><details id=a name=g open><summary>a</summary>x</details><details id=b name=g><summary>b</summary>y</details>\
         <details id=c name=h open><summary>c</summary>z</details><div><details id=e name=g><summary>e</summary></details></div></body></html>",
        "var $ = function (x) { return document.getElementById(x); }, ev = [];\
         ['a', 'b', 'c', 'e'].forEach(function (i) { $(i).addEventListener('toggle', function () { ev.push(i + '=' + $(i).open); }); });\
         $('b').open = true;\
         console.log($('a').open, $('b').open, $('c').open);\
         $('e').setAttribute('open', ''); console.log($('a').open, $('b').open, $('e').open);\
         setTimeout(function () { console.log(ev.join(',')); }, 20);",
    );
    assert_eq!(l[0], "log:false true true", "{l:?}");
    assert_eq!(l[1], "log:false false true");
    assert_eq!(l[2], "log:b=false,a=false,e=true");
}

#[test]
fn qdet1_summary_click_toggles_and_only_the_first_summary() {
    let l = run_html_with_timers(
        "<html><body><details id=d><summary id=s1>one</summary><summary id=s2>two</summary></details></body></html>",
        "var $ = function (x) { return document.getElementById(x); }, n = 0;\
         $('d').addEventListener('toggle', function () { n++; });\
         $('s1').click(); console.log($('d').open); $('s2').click(); console.log($('d').open);\
         $('s1').addEventListener('click', function (e) { e.preventDefault(); }); $('s1').click(); console.log($('d').open);\
         setTimeout(function () { console.log(n); }, 20);",
    );
    assert_eq!(l[0], "log:true", "{l:?}");
    assert_eq!(l[1], "log:true");
    assert_eq!(l[2], "log:true");
    assert_eq!(l[3], "log:1");
}

#[test]
fn qdet1_summary_is_focusable() {
    let l = run_in_html(
        "<html><body><details><summary id=s1>one</summary><summary id=s2>two</summary></details><summary id=s3>lone</summary></body></html>",
        "var $ = function (x) { return document.getElementById(x); };\
         $('s1').focus(); console.log(document.activeElement.id);\
         $('s2').focus(); console.log(document.activeElement.id);\
         $('s3').focus(); console.log(document.activeElement.id);",
    );
    assert_eq!(l[0], "log:s1", "{l:?}");
    assert_eq!(l[1], "log:s1");
    assert_eq!(l[2], "log:s1");
}

#[test]
fn qdet1_text_only_details_toggles_without_engine_mismatch() {
    let m0 = incr_get(Incr::CheckMismatch);
    let l = run_in_html(
        "<html><body style=\"margin:0\"><details id=d>just some loose text that is hidden while closed</details><div id=after></div></body></html>",
        "var d = document.getElementById('d'), a = document.getElementById('after');\
         var c = a.getBoundingClientRect().y; d.open = true; var o = a.getBoundingClientRect().y;\
         d.open = false; console.log(o > c, a.getBoundingClientRect().y === c);",
    );
    assert_eq!(l[0], "log:true true", "{l:?}");
    assert_eq!(incr_get(Incr::CheckMismatch), m0);
}
