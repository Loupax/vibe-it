// QDOM18: "ensure pre-insert validity" and the "replace" variant for insertBefore / appendChild / replaceChild
// and the ParentNode / ChildNode mixins (js_dom.rs `cnCheck`). Errors are thrown before any mutation.
#[allow(unused_imports)]
use super::*;

fn run(js: &str) -> Vec<String> {
    run_in_html("<!doctype html><div id=p></div>", js)
}

const HELPERS: &str = "var err=function(f){try{f();return 'ok';}catch(e){return e.name;}};
    var newDoc=function(){return document.implementation.createDocument(null,null,null);};
    var dt=function(){return document.implementation.createDocumentType('html','','');};";

fn t(js: &str) -> Vec<String> {
    run(&format!("{HELPERS}{js}"))
}

#[test]
fn qdom18_document_parent_rules() {
    let log = t("var d=newDoc(), e=d.createElement('a');
         console.log(err(function(){d.appendChild(d.createTextNode('x'));}));
         d.appendChild(e);
         console.log(err(function(){d.appendChild(d.createElement('b'));}));
         console.log(err(function(){d.insertBefore(d.createElement('b'), e);}));
         console.log(err(function(){d.insertBefore(dt(), null);}));
         console.log(err(function(){d.insertBefore(dt(), e);}));
         console.log(err(function(){d.appendChild(dt());}));
         console.log(d.childNodes.length);
         console.log(err(function(){d.appendChild(d);}));");
    assert_eq!(
        log,
        vec![
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:ok",
            "log:HierarchyRequestError",
            "log:2",
            "log:HierarchyRequestError",
        ]
    );
}

#[test]
fn qdom18_fragment_into_document_keeps_children() {
    let log = t("var d=newDoc(), f=d.createDocumentFragment();
         f.appendChild(d.createElement('a')); f.appendChild(d.createElement('b'));
         console.log(err(function(){d.appendChild(f);})+' '+f.childNodes.length+' '+d.childNodes.length);
         var g=d.createDocumentFragment(); g.appendChild(d.createTextNode('x'));
         console.log(err(function(){d.appendChild(g);})+' '+g.childNodes.length);
         var h=d.createDocumentFragment(); h.appendChild(d.createElement('c'));
         console.log(err(function(){d.appendChild(h);})+' '+h.childNodes.length+' '+d.childNodes.length);");
    assert_eq!(log, vec!["log:HierarchyRequestError 2 0", "log:HierarchyRequestError 1", "log:ok 0 1"]);
}

#[test]
fn qdom18_replace_child_document() {
    let log = t("var d=newDoc(), a=d.createElement('a'), t=dt();
         d.appendChild(t); d.appendChild(a);
         console.log(err(function(){d.replaceChild(d.createElement('b'), t);}));
         console.log(err(function(){d.replaceChild(dt(), a);}));
         console.log(err(function(){d.replaceChild(d.createElement('b'), a);})+' '+d.documentElement.localName);
         var d2=newDoc(); d2.appendChild(d2.createElement('x')); var tt=d2.appendChild(d2.createComment('c'));
         console.log(err(function(){d2.replaceChild(dt(), tt);}));
         console.log(err(function(){d.replaceChild(d.createTextNode('x'), d.documentElement);}));");
    assert_eq!(
        log,
        vec!["log:HierarchyRequestError", "log:HierarchyRequestError", "log:ok b", "log:HierarchyRequestError", "log:HierarchyRequestError"]
    );
}

#[test]
fn qdom18_check_order_and_not_found() {
    let log = t("var p=document.getElementById('p'), a=document.createElement('a'), other=document.createElement('o');
         console.log(err(function(){p.insertBefore(a, other);}));
         console.log(err(function(){p.replaceChild(a, other);}));
         var c=p.appendChild(document.createElement('c'));
         console.log(err(function(){c.insertBefore(p, other);}));
         console.log(err(function(){p.appendChild(p);}));
         console.log(err(function(){p.appendChild(document);}));
         console.log(err(function(){document.createTextNode('t').appendChild(a);}));
         console.log(err(function(){p.insertBefore(a);}));
         console.log(err(function(){p.insertBefore(a, {});}));
         console.log(p.childNodes.length);");
    assert_eq!(
        log,
        vec![
            "log:NotFoundError",
            "log:NotFoundError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:TypeError",
            "log:TypeError",
            "log:1",
        ]
    );
}

#[test]
fn qdom18_no_mutation_record_on_failure() {
    let log = t("var d=newDoc(); d.appendChild(d.createElement('a')); var n=0;
         new MutationObserver(function(r){n+=r.length;}).observe(d,{childList:true});
         err(function(){d.appendChild(d.createElement('b'));});
         err(function(){d.replaceChild(d.createTextNode('x'), d.firstChild);});
         err(function(){d.replaceChildren(d.createTextNode('x'));});
         Promise.resolve().then(function(){console.log(n+' '+d.childNodes.length);});");
    assert_eq!(log, vec!["log:0 1"]);
}

#[test]
fn qdom18_mixins_on_document() {
    let log = t("var d=newDoc(), a=d.createElement('a');
         console.log(err(function(){d.append('text');}));
         console.log(err(function(){d.append(a, d.createElement('b'));}));
         console.log(d.childNodes.length);
         d.append(a); console.log(err(function(){d.prepend(dt());})+' '+d.firstChild.nodeType);
         console.log(err(function(){d.prepend(dt());}));
         console.log(err(function(){a.after(dt());}));
         console.log(err(function(){a.before(dt());}));
         console.log(err(function(){d.replaceChildren(d.createElement('z'), d.createElement('y'));})+' '+d.childNodes.length);
         console.log(err(function(){a.replaceWith('s');}));");
    assert_eq!(
        log,
        vec![
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:0",
            "log:ok 10",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError 2",
            "log:HierarchyRequestError",
        ]
    );
}

#[test]
fn qdom18_element_fast_path_semantics() {
    let log = t("var p=document.getElementById('p'), a=document.createElement('a'), b=document.createElement('b');
         p.appendChild(a); p.insertBefore(b, a); console.log(p.innerHTML);
         p.insertBefore(a, a); console.log(p.innerHTML);
         p.insertBefore(b, undefined); console.log(p.innerHTML);
         p.replaceChild(a, a); console.log(p.innerHTML);
         p.replaceChild(b, a); console.log(p.innerHTML);
         var f=document.createDocumentFragment(); f.append('x', document.createElement('i'));
         p.replaceChild(f, b); console.log(p.innerHTML+' '+f.childNodes.length);
         ");
    assert_eq!(
        log,
        vec![
            "log:<b></b><a></a>",
            "log:<b></b><a></a>",
            "log:<a></a><b></b>",
            "log:<a></a><b></b>",
            "log:<b></b>",
            "log:x<i></i> 0",
        ]
    );
}
