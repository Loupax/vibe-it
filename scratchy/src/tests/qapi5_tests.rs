#[allow(unused_imports)]
use super::*;

#[test]
fn qapi5_aria_properties_reflect_attributes() {
    let (log, body) = mutate(
        "<html><body><div id=a role=button aria-label=L></div></body></html>",
        "var a=document.getElementById('a'), d=document.createElement('p');\
         console.log(a.ariaLabel, a.role, a.ariaHidden, d.ariaLabel, d.role);\
         a.ariaHidden='true'; a.ariaHasPopup='menu'; a.ariaValueNow=5; a.ariaKeyShortcuts='x'; a.ariaAutoComplete='list';\
         console.log(a.getAttribute('aria-hidden'), a.getAttribute('aria-haspopup'), a.getAttribute('aria-valuenow'), a.ariaValueNow);\
         a.ariaLabel=null; a.role=null; console.log(a.hasAttribute('aria-label'), a.hasAttribute('role'), a.ariaLabel);\
         a.setAttribute('aria-expanded','false'); console.log(a.ariaExpanded);\
         console.log('ariaColSpan' in Element.prototype, Object.getOwnPropertyDescriptor(Element.prototype,'ariaLabel').set !== undefined,\
           document.createElementNS('http://www.w3.org/2000/svg','svg').ariaLabel);",
    );
    assert_eq!(
        log,
        ["log:L button null null null", "log:true menu 5 5", "log:false false null", "log:false", "log:true true null"],
        "{log:?}"
    );
    assert!(body.contains("aria-hidden=\"true\"") && body.contains("aria-keyshortcuts=\"x\"") && body.contains("aria-autocomplete=\"list\""), "{body}");
    assert!(!body.contains("aria-label"), "{body}");
}

#[test]
fn qapi5_slot_name_and_assignment() {
    let (log, _) = mutate(
        "<html><body><div id=h><b slot=x id=bx>X</b><i id=ib>I</i>text</div></body></html>",
        "var h=document.getElementById('h'), sr=h.attachShadow({mode:'open'});\
         sr.innerHTML='<slot id=s1 name=x></slot><slot id=s2><u>fb</u></slot>';\
         var s1=sr.getElementById('s1'), s2=sr.getElementById('s2'), bx=document.getElementById('bx');\
         console.log(s1.name === 'x', JSON.stringify(s2.name), bx.slot); s2.name='q'; console.log(s2.getAttribute('name'));\
         s2.name='';\
         console.log(s1.assignedNodes().length, s1.assignedElements()[0] === bx, s2.assignedNodes().length, s2.assignedElements().length);\
         console.log(s2.assignedNodes({flatten:true}).length, bx.assignedSlot === s1, document.getElementById('ib').assignedSlot === s2);\
         var u=document.createElement('slot'); console.log(u.assignedNodes().length, u.assignedElements({flatten:true}).length);",
    );
    let log: Vec<_> = log.into_iter().filter(|l| !l.contains("shadow root rendering")).collect();
    assert_eq!(
        log,
        ["log:true \"\" x", "log:q", "log:1 true 2 1", "log:2 true true", "log:0 0"],
        "{log:?}"
    );
}
