// QEDIT-C: hidden=until-found, beforematch, <details> open + toggle, fragment-navigation reveal.
#[allow(unused_imports)]
use super::*;

#[test]
fn qedit_c_hidden_idl() {
    let l = run_in_html(
        "<html><body><div id=d></div></body></html>",
        "var d = document.getElementById('d');\
         d.hidden = 'UnTiL-FoUnD'; console.log(d.hidden, d.getAttribute('hidden'));\
         d.hidden = 'foo'; console.log(d.hidden, JSON.stringify(d.getAttribute('hidden')));\
         d.hidden = ''; console.log(d.hidden, d.getAttribute('hidden'));\
         d.hidden = NaN; console.log(d.hidden); d.hidden = 1; console.log(d.hidden);\
         d.hidden = false; console.log(getComputedStyle(d).contentVisibility); d.hidden = 'until-found';\
         console.log(getComputedStyle(d).contentVisibility, getComputedStyle(d).display);",
    );
    assert_eq!(l[0], "log:until-found until-found", "{l:?}");
    assert_eq!(l[1], "log:true \"\"");
    assert_eq!(l[2], "log:false null");
    assert_eq!(l[3], "log:false");
    assert_eq!(l[4], "log:true");
    assert_eq!(l[5], "log:visible");
    assert_eq!(l[6], "log:hidden block");
}

#[test]
fn qedit_c_reveal_order_and_toggle() {
    let l = run_html_with_timers(
        "<html><body><details id=a3><div id=a2 hidden=until-found><details id=a1 hidden=until-found><div id=c>x</div></details></div></details></body></html>",
        "var $ = function (x) { return document.getElementById(x); };\
         var ev = [];\
         $('a1').addEventListener('beforematch', function (e) { ev.push('a1 open=' + $('a1').open + ' hid=' + $('a1').hidden); });\
         $('a2').addEventListener('beforematch', function (e) { ev.push('a2 target=' + e.target.id + ' bubbles=' + e.bubbles); });\
         $('a3').addEventListener('toggle', function () {\
           ev.push('toggle a3 open=' + $('a3').open + ' a2hid=' + $('a2').hidden); console.log(ev.join('|')); });\
         location.hash = '#c';\
         console.log('sync', $('a1').hidden, $('a2').hidden, $('a1').open, $('a3').open);",
    );
    assert_eq!(l[0], "log:sync false false true true", "{l:?}");
    assert_eq!(
        l[1],
        "log:a1 open=true hid=until-found|a2 target=a1 bubbles=true|a2 target=a2 bubbles=true|toggle a3 open=true a2hid=false"
    );
}

#[test]
fn qedit_c_reveal_aborts_when_made_hidden() {
    let l = run_in_html(
        "<html><body><div id=o hidden=until-found><div id=i hidden=until-found><p id=c>x</p></div></div></body></html>",
        "var $ = function (x) { return document.getElementById(x); };\
         $('i').addEventListener('beforematch', function () { $('i').hidden = true; });\
         $('o').addEventListener('beforematch', function (e) { console.log('outer', e.target.id); });\
         location.hash = '#c';\
         console.log($('i').hidden, $('o').hidden);",
    );
    // `i` became plain `hidden`: the algorithm stops, `o` keeps its state and gets no event of its own.
    assert_eq!(l[0], "log:outer i", "{l:?}");
    assert_eq!(l[1], "log:true until-found");
}

#[test]
fn qedit_c_no_beforematch_without_until_found() {
    let l = run_in_html(
        "<html><body><div id=t>x</div></body></html>",
        "var n = 0; document.getElementById('t').addEventListener('beforematch', function () { n++; });\
         location.hash = '#t'; console.log(n);",
    );
    assert_eq!(l[0], "log:0", "{l:?}");
}

#[test]
fn qedit_c_summary_target_does_not_open_details() {
    let l = run_in_html(
        "<html><body><details id=d><summary id=s>sum</summary><p>body</p></details></body></html>",
        "var r = __scratchy_reveal_node(document.getElementById('s')); console.log(r, document.getElementById('d').open);\
         r = __scratchy_reveal_node(document.querySelector('p')); console.log(r, document.getElementById('d').open);",
    );
    assert_eq!(l[0], "log:0 false", "{l:?}");
    assert_eq!(l[1], "log:1 true");
}

fn qedit_c_laid(html: &str) -> (Laid, ShapeCache) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut fs = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut fs, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    (laid, cache)
}

#[test]
fn qedit_c_until_found_text_is_laid_out_but_has_no_height() {
    let (laid, cache) = qedit_c_laid("<html><body style=\"margin:0\"><div hidden=until-found>needle in hay</div><p>tail</p></body></html>");
    assert_eq!(crate::search::find_in_laid(&laid, &cache, "needle").len(), 1, "find-in-page still sees text inside until-found");
    let (plain, _) = qedit_c_laid("<html><body style=\"margin:0\"><div>needle in hay</div><p>tail</p></body></html>");
    assert!(laid.h < plain.h, "until-found block has no content height ({} vs {})", laid.h, plain.h);
}
