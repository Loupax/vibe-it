// QEDIT-A: HTML reflection of contentEditable, isContentEditable, designMode, spellcheck, autocapitalize, autocorrect and
// writingSuggestions (reflection only; no editing behaviour, no spell checking).
use super::*;

fn run(js: &str) -> Vec<String> {
    run_in_html("<html><body></body></html>", js)
}

#[test]
fn qedit_a_contenteditable_enumerated_and_throws() {
    let log = run(
        "var d = document.createElement('div'), out = [];\
         ['true','TRUE','false','plaintext-only','PlainText-Only','INHERIT'].forEach(function(v){ d.contentEditable = v; out.push(d.contentEditable + '/' + d.getAttribute('contenteditable')); });\
         ['foobar','', 'fal\\u017fe', 'pla\\u0131ntext-only', 'pla\\u0130ntext-only'].forEach(function(v){ try { d.contentEditable = v; out.push('no throw ' + v); } catch (e) { out.push(e.name); } });\
         var e = document.createElement('div'); e.setAttribute('contenteditable', 'fal\\u017fe'); out.push(e.contentEditable);\
         e.setAttribute('contenteditable', ''); out.push(e.contentEditable);\
         e.setAttribute('contenteditable', 'TRUE'); out.push(e.contentEditable);\
         console.log(out.join(' '));",
    );
    assert_eq!(
        log,
        ["log:true/true true/true false/false plaintext-only/plaintext-only plaintext-only/plaintext-only inherit/null SyntaxError SyntaxError SyntaxError SyntaxError SyntaxError inherit true true"],
        "{log:?}"
    );
}

#[test]
fn qedit_a_is_content_editable_inherits_and_design_mode() {
    let log = run(
        "document.body.innerHTML = '<div id=a contenteditable><p id=b><span id=c></span></p><i id=n contenteditable=false><b id=m></b></i></div><div id=z></div>';\
         var g = function(i){ return document.getElementById(i).isContentEditable; }, out = [];\
         out.push(g('a'), g('b'), g('c'), g('n'), g('m'), g('z'));\
         var det = document.createElement('p'); out.push(det.isContentEditable);\
         out.push(document.designMode);\
         document.designMode = 'ON'; out.push(document.designMode, g('z'), g('n'), g('b'), det.isContentEditable);\
         document.designMode = 'bogus'; out.push(document.designMode);\
         document.designMode = 'Off'; out.push(document.designMode, g('z'));\
         console.log(out.join(' '));",
    );
    assert_eq!(log, ["log:true true true false false false false off on true false true false on off false"], "{log:?}");
}

#[test]
fn qedit_a_spellcheck_boolean_with_inherit_default() {
    let log = run(
        "document.body.innerHTML = '<div spellcheck=false><span id=s></span><span id=t spellcheck=true></span><span id=u spellcheck=bogus></span></div><div spellcheck=true><span id=k spellcheck=FaLsE></span><span id=w spellcheck=\"fal\\u017fe\"></span></div><p id=p></p>';\
         var g = function(i){ return document.getElementById(i).spellcheck; };\
         var d = document.createElement('div'), out = [g('s'), g('t'), g('u'), g('k'), g('w'), g('p'), d.spellcheck];\
         d.spellcheck = false; out.push(d.getAttribute('spellcheck'), d.spellcheck);\
         d.spellcheck = 1; out.push(d.getAttribute('spellcheck'), d.spellcheck);\
         d.setAttribute('spellcheck', ''); out.push(d.spellcheck);\
         console.log(out.join(' '));",
    );
    assert_eq!(log, ["log:false true false false true true true false false true true true"], "{log:?}");
}

#[test]
fn qedit_a_autocapitalize_enumerated_and_form_inheritance() {
    let log = run(
        "var out = [], d = document.createElement('div');\
         out.push(d.autocapitalize);\
         ['on','off','none','WORDS','Characters','sentences','foo'].forEach(function(v){ d.autocapitalize = v; out.push(d.autocapitalize + '/' + d.getAttribute('autocapitalize')); });\
         d.autocapitalize = ''; out.push('[' + d.autocapitalize + ']');\
         var f = document.createElement('form'); f.setAttribute('autocapitalize', 'words'); document.body.appendChild(f);\
         var i = document.createElement('input'), img = document.createElement('img'), p = document.createElement('input');\
         f.appendChild(i); f.appendChild(img); p.type = 'password'; f.appendChild(p);\
         out.push(i.autocapitalize, '[' + img.autocapitalize + ']', p.autocapitalize);\
         i.setAttribute('autocapitalize', ''); out.push(i.autocapitalize);\
         i.setAttribute('autocapitalize', 'off'); out.push(i.autocapitalize);\
         var o = document.createElement('textarea'); o.setAttribute('form', 'ff'); f.id = 'ff'; document.body.appendChild(o); out.push(o.autocapitalize);\
         console.log(out.join(' '));",
    );
    assert_eq!(
        log,
        ["log: sentences/on none/off none/none words/WORDS characters/Characters sentences/sentences sentences/foo [] words [] words words none words"],
        "{log:?}"
    );
}

#[test]
fn qedit_a_autocorrect_boolean_and_form_inheritance() {
    let log = run(
        "var out = [], d = document.createElement('div');\
         out.push(d.autocorrect); d.autocorrect = false; out.push(d.autocorrect, d.getAttribute('autocorrect'));\
         d.autocorrect = 'hello'; out.push(d.autocorrect, d.getAttribute('autocorrect'));\
         ['OFF','on','','bogus'].forEach(function(v){ d.setAttribute('autocorrect', v); out.push(d.autocorrect); });\
         var f = document.createElement('form'); f.setAttribute('autocorrect', 'off'); document.body.appendChild(f);\
         var i = document.createElement('select'), img = document.createElement('img'), e = document.createElement('input'), t = document.createElement('input');\
         e.type = 'email'; f.appendChild(i); f.appendChild(img); f.appendChild(e); f.appendChild(t);\
         out.push(i.autocorrect, img.autocorrect, e.autocorrect, t.autocorrect);\
         t.setAttribute('autocorrect', ''); out.push(t.autocorrect);\
         f.setAttribute('autocorrect', 'on'); t.removeAttribute('autocorrect'); out.push(t.autocorrect, e.autocorrect);\
         console.log(out.join(' '));",
    );
    assert_eq!(
        log,
        ["log:true false off true on false true true true false true false false true true false"],
        "{log:?}"
    );
}

#[test]
fn qedit_a_writing_suggestions_enumerated_inherited() {
    let log = run(
        "document.body.innerHTML = '<div writingsuggestions=false><span id=a></span><span id=b writingsuggestions=true></span><span id=c writingsuggestions=foo></span></div><p id=d writingsuggestions=\"\"></p>';\
         var g = function(i){ return document.getElementById(i).writingSuggestions; }, out = [g('a'), g('b'), g('c'), g('d')];\
         var x = document.createElement('input'); out.push(x.writingSuggestions, x.getAttribute('writingsuggestions'));\
         x.writingSuggestions = false; out.push(x.writingSuggestions, x.getAttribute('writingsuggestions'));\
         x.writingSuggestions = 'FaLsE'; out.push(x.writingSuggestions, x.getAttribute('writingsuggestions'));\
         x.writingSuggestions = ''; out.push(x.writingSuggestions, '[' + x.getAttribute('writingsuggestions') + ']');\
         console.log(out.join(' '));",
    );
    assert_eq!(
        log,
        ["log:false true false true true  false false false FaLsE true []"],
        "{log:?}"
    );
}
