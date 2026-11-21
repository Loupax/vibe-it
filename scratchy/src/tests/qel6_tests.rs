// QEL6: a UA-dispatched (trusted) event runs a microtask checkpoint after each listener (empty JS
// stack); a script-made dispatch (el.click()) keeps the listeners back to back.
#[allow(unused_imports)]
use super::*;

#[test]
fn qel6_microtask_checkpoint_between_listeners_of_ua_events() {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let js = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(
            "document.body.innerHTML = \"<button id=b onclick=\\\"console.log(1+1)\\\">x</button>\";\
             var b = document.getElementById(\"b\");\
             b.addEventListener(\"click\", function(){ console.log(\"l1\"); Promise.resolve().then(function(){ console.log(\"then1\"); }); });\
             b.addEventListener(\"click\", function(){ console.log(\"l2\"); });",
        )],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap();
    out.borrow_mut().clear();
    js.eval("t", "__scratchy_dn(document.getElementById(\"b\").__id, \"click\", {bubbles:true,cancelable:true});");
    let ua: Vec<String> = out.borrow().clone();
    assert_eq!(ua, ["log:l1", "log:then1", "log:l2", "log:2"], "{ua:?}");
    out.borrow_mut().clear();
    js.eval("t", "document.getElementById(\"b\").click();");
    let sy: Vec<String> = out.borrow().clone();
    assert_eq!(sy, ["log:l1", "log:l2", "log:2", "log:then1"], "{sy:?}");
}
