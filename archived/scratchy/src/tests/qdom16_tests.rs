// QDOM16: ChildNode / ParentNode mixins on every node kind (js_dom.rs `cnMixin`).
#[allow(unused_imports)]
use super::*;

fn run(js: &str) -> Vec<String> {
    run_in_html("<!doctype html><div id=p></div>", js)
}

/// before/after/replaceWith/remove exist on CharacterData and DocumentType, with string conversion.
#[test]
fn qdom16_childnode_on_text_comment_doctype() {
    let log = run(
        "var p=document.getElementById('p'); var t=document.createTextNode('T'); p.appendChild(t);
         t.after('a', document.createElement('x')); t.before(null, undefined);
         console.log(p.innerHTML);
         var c=document.createComment('c'); p.appendChild(c); c.replaceWith('r', 'q'); console.log(p.innerHTML);
         t.remove(); t.remove(); console.log(p.innerHTML+' '+t.parentNode);
         var dt=document.implementation.createDocumentType('html','',''); var d=document.implementation.createDocument(null,'',null);
         d.appendChild(dt); dt.remove(); console.log(d.childNodes.length+' '+typeof dt.before+typeof dt.after+typeof dt.replaceWith);",
    );
    assert_eq!(
        log,
        vec![
            "log:nullundefinedTa<x></x>",
            "log:nullundefinedTa<x></x>rq",
            "log:nullundefineda<x></x>rq null",
            "log:0 functionfunctionfunction"
        ]
    );
}

/// Viable siblings: arguments that are siblings of the node are skipped when picking the anchor.
#[test]
fn qdom16_viable_sibling() {
    let log = run(
        "var p=document.getElementById('p'); p.innerHTML='<a></a><b></b><c></c><d></d>';
         var [a,b,c,d]=p.children; c.before(a, b); console.log(p.innerHTML);
         p.innerHTML='<a></a><b></b><c></c><d></d>'; [a,b,c,d]=p.children; b.after(d, a); console.log(p.innerHTML);
         p.innerHTML='<a></a><b></b><c></c>'; [a,b,c]=p.children; b.replaceWith(c, 'x', b); console.log(p.innerHTML);",
    );
    assert_eq!(
        log,
        vec!["log:<a></a><b></b><c></c><d></d>", "log:<b></b><d></d><a></a><c></c>", "log:<a></a><c></c>x<b></b>"]
    );
}

/// ParentNode on Document: append/prepend/replaceChildren validate like pre-insert.
#[test]
fn qdom16_document_parentnode_and_exceptions() {
    let log = run(
        "var d=document.implementation.createHTMLDocument('t'); var e=function(f){try{f();return 'ok'}catch(x){return x.name}};
         console.log(e(function(){d.append('text')}));
         console.log(e(function(){d.append(d.createElement('a'))}));
         console.log(e(function(){d.append(d.createElement('a'),d.createElement('b'))}));
         d.replaceChildren(d.implementation.createDocumentType('html','',''), d.createElement('a'));
         console.log(d.childNodes.length+' '+d.doctype.name+' '+d.documentElement.tagName);
         d.prepend(d.createComment('c')); console.log(d.firstChild.nodeType);
         d.replaceChildren(); console.log(d.childNodes.length+' '+d.doctype);
         var pre=document.createElement('p'); console.log(e(function(){pre.append(document)}));
         console.log(e(function(){pre.append(pre)}));
         console.log(e(function(){pre.append(document.implementation.createDocumentType('html','',''))}));",
    );
    assert_eq!(
        log,
        vec![
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:2 html A",
            "log:8",
            "log:0 null",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError",
        ]
    );
}

/// document.doctype on the page document; unscopables list the mixin members.
#[test]
fn qdom16_page_doctype_and_unscopables() {
    let log = run(
        "console.log((document.doctype instanceof DocumentType)+' '+document.doctype.name);
         var u=Element.prototype[Symbol.unscopables];
         console.log(['before','after','replaceWith','remove','prepend','append','replaceChildren'].every(function(k){return u[k]===true}));
         console.log(Object.getPrototypeOf(u)+' '+Text.prototype.hasOwnProperty('before')+' '+CharacterData.prototype.before.length);",
    );
    assert_eq!(log, vec!["log:true html", "log:true", "log:null false 0"]);
}
