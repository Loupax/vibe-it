#[allow(unused_imports)]
use super::*;
use crate::incr_log::{self, Change, FormKind, Val};
use std::collections::HashMap;

// QINC1e: the write log (R60 section 5). One table row per native: the entries a script's write
// leaves, a no-op write (QY53m) leaves none, a write to a detached node leaves none.

fn walk(n: &Handle, out: &mut Vec<Handle>) {
    out.push(n.clone());
    for c in n.children.borrow().iter() {
        walk(c, out);
    }
}

/// Runs `code` against `html` and returns the log as readable lines (nodes by `id`, else tag).
fn log_of(html: &str, code: &str) -> Vec<String> {
    let doc = parse_html(html);
    let mut before = Vec::new();
    walk(&doc, &mut before);
    let _ = incr_log::drain();
    let _engine = start_scripts(&doc, &[src(code)], Rc::new(|_, _| {}));
    let drained = incr_log::drain();
    let mut all = before;
    walk(&doc, &mut all);
    let names: HashMap<usize, String> = all
        .iter()
        .map(|h| {
            let n = match &h.data {
                NodeData::Element { .. } => element_attr(h, "id").map(|s| s.to_string()).unwrap_or_else(|| element_tag(h).to_string()),
                NodeData::Text { .. } => "#text".to_string(),
                _ => "#other".to_string(),
            };
            (incr_log::key(h), n)
        })
        .collect();
    let nm = |k: usize| names.get(&k).cloned().unwrap_or_else(|| "?".to_string());
    let opt = |k: Option<usize>| k.map_or("-".to_string(), nm);
    let val = |v: &Val| match v {
        Val::Absent => "absent".to_string(),
        Val::Set(s) => format!("[{s}]"),
        Val::Big => "big".to_string(),
    };
    assert!(!drained.global, "{drained:?}");
    drained
        .entries
        .iter()
        .map(|e| {
            let who = if e.node == 0 { "-".to_string() } else { nm(e.node) };
            let what = match &e.change {
                Change::Attr { name, old, new } => format!("Attr {name} {}->{}", val(old), val(new)),
                Change::ChildInsert { child, prev, next } => format!("Insert {} prev={} next={}", nm(*child), opt(*prev), opt(*next)),
                Change::ChildRemove { child, prev, next } => format!("Remove {} prev={} next={}", nm(*child), opt(*prev), opt(*next)),
                other => format!("{other:?}"),
            };
            format!("{who}: {what}")
        })
        .collect()
}

const PAGE: &str = "<html><body><div id=a class=x title=t>hi</div><ul id=l><li id=one>1</li><li id=two>2</li></ul>\
    <input id=cb type=checkbox><input id=tx value=v><textarea id=ta>q</textarea>\
    <select id=sel><option id=o1 value=1>1</option><option id=o2 value=2>2</option></select>\
    <form id=f><input id=fi value=d></form><style id=st>.x{color:red}</style></body></html>";

#[test]
fn qinc1e_each_native_logs_its_entries() {
    let cases: &[(&str, &str, &[&str])] = &[
        ("setAttribute", "document.getElementById('a').setAttribute('class','y')", &["a: Attr class [x]->[y]"]),
        ("setAttribute new", "document.getElementById('a').setAttribute('data-k','v')", &["a: Attr data-k absent->[v]"]),
        ("removeAttribute", "document.getElementById('a').removeAttribute('title')", &["a: Attr title [t]->absent"]),
        ("className property", "document.getElementById('a').className='z'", &["a: Attr class [x]->[z]"]),
        ("id property", "document.getElementById('a').id='b'", &["b: Attr id [a]->[b]"]),
        ("style write", "document.getElementById('a').style.color='red'", &["a: Attr style absent->[color: red;]"]),
        ("setAttributeNS", "document.getElementById('a').setAttributeNS('http://www.w3.org/1999/xlink','xlink:href','u')", &["a: Attr href absent->[u]"]),
        (
            "removeAttributeNS",
            "var e=document.getElementById('a'); e.setAttributeNS('http://www.w3.org/1999/xlink','xlink:href','u'); e.removeAttributeNS('http://www.w3.org/1999/xlink','href')",
            &["a: Attr href absent->[u]", "a: Attr href [u]->absent"],
        ),
        ("text node data", "document.getElementById('a').firstChild.data='bye'", &["#text: Text"]),
        ("textContent", "document.getElementById('a').textContent='bye'", &["a: TextContent"]),
        ("innerHTML", "document.getElementById('a').innerHTML='<b>x</b>'", &["a: InnerHtml"]),
        (
            "appendChild",
            "var n=document.createElement('li'); n.id='n'; document.getElementById('l').appendChild(n)",
            &["l: Insert n prev=two next=-"],
        ),
        (
            "insertBefore",
            "var n=document.createElement('li'); n.id='n'; var l=document.getElementById('l'); l.insertBefore(n, l.firstChild)",
            &["l: Insert n prev=- next=one"],
        ),
        ("removeChild", "var l=document.getElementById('l'); l.removeChild(document.getElementById('one'))", &["l: Remove one prev=- next=two"]),
        (
            "move within parent",
            "var l=document.getElementById('l'); l.appendChild(document.getElementById('one'))",
            &["l: Remove one prev=- next=two", "l: Insert one prev=two next=-"],
        ),
        (
            "move between parents",
            "document.getElementById('f').appendChild(document.getElementById('two'))",
            &["l: Remove two prev=one next=-", "f: Insert two prev=fi next=-"],
        ),
        (
            "fragment",
            "var fr=document.createDocumentFragment(); var a=document.createElement('i'); a.id='i1'; var b=document.createElement('i'); b.id='i2'; fr.appendChild(a); fr.appendChild(b); document.getElementById('l').appendChild(fr)",
            &["l: Insert i1 prev=two next=i2", "l: Insert i2 prev=i1 next=-"],
        ),
        ("checked", "document.getElementById('cb').checked=true", &["cb: Form(Checked)"]),
        ("indeterminate", "document.getElementById('cb').indeterminate=true", &["cb: Form(Indeterminate)"]),
        ("value", "document.getElementById('tx').value='new'", &["tx: Form(Value)"]),
        ("selection", "var t=document.getElementById('tx'); t.setSelectionRange(1,1)", &["tx: Form(Selection)"]),
        ("selectedIndex", "document.getElementById('sel').selectedIndex=1", &["sel: Form(SelectedIndex)"]),
        ("option selected", "document.getElementById('o2').selected=true", &["o2: Form(OptionSelected)"]),
        ("form reset", "var t=document.getElementById('fi'); t.value='changed'; document.getElementById('f').reset()", &["fi: Form(Value)", "f: Form(Reset)"]),
        (
            "style sheet rule edit",
            "document.getElementById('st').sheet.insertRule('b{color:blue}',0)",
            &["st: SheetText"],
        ),
        (
            "adopted css",
            "var s=new CSSStyleSheet(); s.replaceSync('p{color:red}'); document.adoptedStyleSheets=[s]",
            &["-: Adopted", "-: CssVer"],
        ),
    ];
    let mut bad = Vec::new();
    for (name, code, want) in cases {
        let got = log_of(PAGE, code);
        let mut got_sorted = got.clone();
        let mut want_sorted: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        // Order inside one write is part of the contract for child lists only; compare as sets for the rest.
        if !name.contains("move") && !name.contains("fragment") {
            got_sorted.sort();
            want_sorted.sort();
        } else {
            got_sorted = got.clone();
        }
        if got_sorted != want_sorted {
            bad.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn qinc1e_no_op_writes_log_nothing() {
    let cases: &[(&str, &str)] = &[
        ("setAttribute same", "document.getElementById('a').setAttribute('class','x')"),
        ("removeAttribute absent", "document.getElementById('a').removeAttribute('nope')"),
        ("setAttributeNS same", "var e=document.getElementById('a'); e.setAttributeNS('http://www.w3.org/1999/xlink','xlink:href','u'); e.removeAttributeNS('http://www.w3.org/1999/xlink','zzz')"),
        ("text data same", "document.getElementById('a').firstChild.data='hi'"),
        ("textContent same", "document.getElementById('a').textContent='hi'"),
        ("checked same", "document.getElementById('cb').checked=false"),
    ];
    for (name, code) in cases {
        let got = log_of(PAGE, code);
        // setAttributeNS above legitimately adds one attribute; everything else must be empty.
        let allowed = usize::from(name.contains("NS"));
        assert!(got.len() <= allowed, "{name}: {got:?}");
    }
}

#[test]
fn qinc1e_detached_writes_log_nothing() {
    let code = "var d=document.createElement('div'); d.setAttribute('class','q'); d.removeAttribute('class');\
        d.setAttributeNS('http://www.w3.org/1999/xlink','xlink:href','u'); d.textContent='t'; d.innerHTML='<p>x</p>';\
        d.firstChild.firstChild.data='y'; var k=document.createElement('span'); d.appendChild(k); d.removeChild(k);\
        var i=document.createElement('input'); i.checked=true; i.value='v'; i.indeterminate=true; d.appendChild(i);\
        var sl=document.createElement('select'); sl.innerHTML='<option>a</option><option>b</option>'; sl.selectedIndex=1; d.appendChild(sl);";
    let got = log_of(PAGE, code);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn qinc1e_log_is_bounded_and_overflow_is_global() {
    let _ = incr_log::drain();
    for i in 0..incr_log::CAP + 10 {
        incr_log::note(i + 1, Change::Text);
    }
    let d = incr_log::drain();
    assert_eq!(d.entries.len(), incr_log::CAP);
    assert!(d.global);
    // Drained: the next window starts clean.
    incr_log::note(1, Change::Text);
    let d = incr_log::drain();
    assert_eq!(d.entries.len(), 1);
    assert!(!d.global);
    incr_log::note(0, Change::Global);
    assert!(incr_log::drain().global);
    assert_eq!(Val::of(Some(&"x".repeat(600))), Val::Big);
    assert_eq!(Val::of(None), Val::Absent);
    let _ = FormKind::Checked;
}

#[test]
fn qinc1e_perf_report_has_the_incr_log_counters() {
    incr_log::note(1, Change::Text);
    incr_log::layout_boundary();
    let j = incr_log::stats_json();
    assert!(j.starts_with("{\"layouts\":"), "{j}");
    assert!(j.contains("\"max_per_layout\":") && j.contains("\"global\":"), "{j}");
}
