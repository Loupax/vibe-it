//! QEDIT-D: the focus fixup rule. A focused element that is removed stops being the active element for
//! good; re-inserting it (or moving its detached subtree in) does not make it active again.
#[allow(unused_imports)]
use super::*;

fn run(js: &str) -> String {
    let doc = parse_html("<html><body><div id=p tabindex=0><div id=c tabindex=0></div></div></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let scripts = vec![src(js)];
    start_scripts(&doc, &scripts, Rc::new(move |_l, t| o.borrow_mut().push(t.to_string()))).unwrap();
    let lines = out.borrow();
    lines.join("\n")
}

#[test]
fn readded_formerly_active_element_is_not_active_again() {
    let r = run("var p = document.getElementById('p'); p.focus();\
        var a = document.activeElement === p;\
        document.body.removeChild(p); document.body.appendChild(p);\
        console.log(a, document.activeElement === document.body);");
    assert_eq!(r, "true true");
}

#[test]
fn descendant_of_a_removed_subtree_is_not_reactivated_by_reinsertion() {
    let r = run("var p = document.getElementById('p'), c = document.getElementById('c'); c.focus();\
        document.body.removeChild(p); var mid = document.activeElement === document.body;\
        document.body.appendChild(p);\
        console.log(mid, document.activeElement === document.body);");
    assert_eq!(r, "true true");
}

#[test]
fn wpt_active_element_sequence() {
    let r = run("var c = document.querySelector('#c'); c.focus();\
        var s1 = document.activeElement === c; document.querySelector('#p').innerHTML = '';\
        var s2 = document.activeElement === document.body;\
        var el = document.querySelector('#p'); el.tabIndex = 0; el.focus();\
        var s3 = document.activeElement === el;\
        document.body.removeChild(el); document.body.appendChild(el);\
        console.log(s1, s2, s3, document.activeElement === document.body);");
    assert_eq!(r, "true true true true");
}

#[test]
fn tab_index_reflects_the_attribute_and_defaults_by_element() {
    let r = run("var d = document.createElement('div'), b = document.createElement('button'), a = document.createElement('a');\
        var o = [d.tabIndex, b.tabIndex, a.tabIndex]; d.tabIndex = 3; o.push(d.getAttribute('tabindex'), d.tabIndex);\
        d.setAttribute('tabindex', ' -1x'); o.push(d.tabIndex); d.setAttribute('tabindex', 'x'); o.push(d.tabIndex);\
        d.tabIndex = 2.9; o.push(d.getAttribute('tabindex'));\
        var p = document.createElement('div'); document.body.appendChild(p); p.tabIndex = 0; p.focus(); o.push(document.activeElement === p);\
        console.log(o.join());");
    assert_eq!(r, "-1,0,-1,3,3,-1,-1,2,true");
}

#[test]
fn delegates_focus_host_is_not_a_focus_target_itself() {
    let r = run("var h = document.createElement('div'); h.tabIndex = 0; document.body.appendChild(h);\
        var sr = h.attachShadow({mode: 'open', delegatesFocus: true});\
        var o = []; h.focus(); o.push(document.activeElement === document.body);\
        var i = document.createElement('input'); sr.appendChild(i); h.focus();\
        o.push(i.matches(':focus'));\
        console.log(o.join());");
    assert_eq!(r.lines().last().unwrap(), "true,true");
}

#[test]
fn focused_element_that_becomes_display_none_is_blurred_after_the_script() {
    let doc = parse_html("<html><head><style>#c:focus { display: none }</style></head><body><div id=p><div id=c tabindex=0></div></div></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let js = "var c = document.getElementById('c'); c.addEventListener('blur', function () { console.log('blur', document.activeElement === document.body); });\
        c.focus(); console.log('sync', document.activeElement === c);";
    let scripts = vec![src(js)];
    let eng = start_scripts(&doc, &scripts, Rc::new(move |_l, t| o.borrow_mut().push(t.to_string()))).unwrap();
    drop(eng);
    assert_eq!(out.borrow().join("\n"), "sync true\nblur true");
}

#[test]
fn focus_inside_a_connected_tree_survives_unrelated_inserts() {
    let r = run("var c = document.getElementById('c'); c.focus();\
        document.body.appendChild(document.createElement('span'));\
        console.log(document.activeElement === c);");
    assert_eq!(r, "true");
}
