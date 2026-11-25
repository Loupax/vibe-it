#[allow(unused_imports)]
use super::*;

// ---------- QDOM12: Text.splitText, Text.wholeText, Node.normalize ----------

#[test]
fn qdom12_split_text_and_whole_text() {
    let (log, body) = mutate(
        "<html><body><p id=p>hello world</p></body></html>",
        "var p = document.getElementById('p'), t = p.firstChild, n = t.splitText(5);\
         console.log(t.data, '|', n.data, p.childNodes.length, t.nextSibling === n, n instanceof Text, n.ownerDocument === document, n.parentNode === p);\
         console.log(t.wholeText, '|', n.wholeText, '|', t.wholeText === n.wholeText);\
         var e = document.createElement('b'); p.insertBefore(e, n); console.log(t.wholeText, '|', n.wholeText);\
         try { t.splitText(6); } catch (x) { console.log(x.name, x instanceof DOMException); }\
         try { t.splitText(); } catch (x) { console.log(x.name); }\
         var e0 = n.splitText(5); console.log(JSON.stringify(e0.data), n.data, n.nextSibling === e0);\
         var d = document.createTextNode('abc'), r = d.splitText(1); console.log(d.data, r.data, r.parentNode, d.nextSibling);\
         console.log(t.splitText(-4294967296 + 2).data, t.data, t.splitText('1').data);",
    );
    assert_eq!(
        log,
        [
            "log:hello |  world 2 true true true true",
            "log:hello world | hello world | true",
            "log:hello |  world",
            "log:IndexSizeError true",
            "log:TypeError",
            "log:\"d\"  worl true",
            "log:a bc null null",
            "log:llo he e",
        ],
        "{log:?}"
    );
    assert!(body.starts_with("<p id=\"p\">he"), "{body}");
}

#[test]
fn qdom12_normalize_merges_and_drops_empty() {
    let (log, body) = mutate(
        "<html><body><div id=d></div></body></html>",
        "var d = document.getElementById('d');\
         function T(s) { return document.createTextNode(s); }\
         d.appendChild(T('')); d.appendChild(T('a')); d.appendChild(T('')); d.appendChild(T('b'));\
         var s = document.createElement('span'); s.appendChild(T('x')); s.appendChild(T('')); s.appendChild(T('y')); d.appendChild(s);\
         d.appendChild(T('c')); d.appendChild(T('d')); d.appendChild(document.createComment('k')); d.appendChild(T('')); d.appendChild(T('e'));\
         var first = d.childNodes[1], sx = s.firstChild;\
         console.log(d.childNodes.length, document.normalize(), d.normalize());\
         console.log(d.childNodes.length, d.firstChild === first, first.data, s.childNodes.length, s.firstChild === sx, sx.data);\
         console.log(Array.prototype.map.call(d.childNodes, function (n) { return n.nodeType + ':' + n.data; }).join(','));\
         var f = document.createDocumentFragment(); f.appendChild(T('1')); f.appendChild(T('2')); f.normalize(); console.log(f.childNodes.length, f.firstChild.data);\
         var m = new MutationObserver(function () {}); m.observe(d, { childList: true, characterData: true, subtree: true });\
         d.appendChild(T('z')); d.appendChild(T('w')); d.normalize(); console.log(d.lastChild.data, m.takeRecords().map(function (r) { return r.type; }).join());",
    );
    assert_eq!(
        log,
        [
            "log:10 undefined undefined",
            "log:5 true ab 1 true xy",
            "log:3:ab,1:undefined,3:cd,8:k,3:e",
            "log:1 12",
            "log:ezw childList,childList,characterData,childList,childList",
        ],
        "{log:?}"
    );
    assert_eq!(body, "<div id=\"d\">ab<span>xy</span>cd<!--k-->ezw</div>");
}

// QDOM12: a split / merged Text node relayouts (with the incremental engine forced on too).
#[test]
fn qdom12_split_and_normalize_relayout() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} #d{display:inline-block;font:16px monospace}</style></head><body><span id=d>ab</span></body></html>",
        "var d = document.getElementById('d'), w0 = d.offsetWidth, t = d.firstChild, n = t.splitText(1);\
         var w1 = d.offsetWidth; n.data = 'bbbbbb'; var w2 = d.offsetWidth;\
         d.normalize(); var w3 = d.offsetWidth;\
         d.firstChild.splitText(2).data = ''; var w4 = d.offsetWidth; d.normalize();\
         console.log(w0 > 0, w1 === w0, w2 > w1, w3 === w2, w4 < w3, d.childNodes.length, d.firstChild.data, d.offsetWidth === w4);",
    );
    assert_eq!(log, ["log:true true true true true 1 ab true"], "{log:?}");
}
