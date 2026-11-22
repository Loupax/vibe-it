//! QAPI6: `:has()` (relative selector lists, descendant / `>` / `+` / `~` anchors) in stylesheets and in
//! `querySelector` / `matches` / `closest`, and its invalidation: a change inside the subject (or its later
//! siblings for `+` / `~`) re-matches the anchors, in the incremental engine (log-only seam: the engine must get
//! it right from the write log alone, a mismatch with the from-scratch styles panics) and from scratch.
use super::*;
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam, Stats};
use crate::invalidation::{ChildChange, InvalidationMap, Mark};


// ---------- matching: querySelector / matches / closest ----------

#[test]
fn qapi6_query_selector_with_each_combinator() {
    let html = "<html><body><div id=a class=card><p class=x>x</p></div><div id=b class=card><span><p class=x>y</p></span></div>\
        <div id=c class=card><em>z</em></div><h2 id=h1>t</h2><p id=n1>after</p><h2 id=h2>u</h2><span id=s>v</span><p id=n2>w</p></div>\
        </body></html>";
    let code = "var ids = function (s) { return Array.prototype.map.call(document.querySelectorAll(s), function (e) { return e.id; }).join(','); };\
        console.log(ids('.card:has(.x)'));\
        console.log(ids('.card:has(> .x)'));\
        console.log(ids('h2:has(+ p)'));\
        console.log(ids('h2:has(~ p)'));\
        console.log(ids('div:has(> span .x)'));\
        console.log(ids('.card:not(:has(.x))'));\
        console.log(ids(':is(.card:has(em), h2:has(+ span))'));\
        console.log(ids('.card:has(.x, em)'));\
        console.log(document.getElementById('a').matches(':has(p)'), document.getElementById('c').matches(':has(p)'));\
        var p = document.querySelector('#b p'); console.log(p.closest(':has(> span)').id);\
        try { document.querySelector(':has(:has(p))'); console.log('no throw'); } catch (e) { console.log(e.name); }\
        try { document.querySelector(':has()'); console.log('no throw'); } catch (e) { console.log(e.name); }";
    let log = run_in_html(html, code);
    assert_eq!(
        log,
        vec![
            "log:a,b".to_string(),
            "log:a".into(),
            "log:h1".into(),
            "log:h1,h2".into(),
            "log:b".into(),
            "log:c".into(),
            "log:c,h2".into(),
            "log:a,b,c".into(),
            "log:true false".into(),
            "log:b".into(),
            "log:SyntaxError".into(),
            "log:SyntaxError".into(),
        ],
        "{log:?}"
    );
}

// ---------- stylesheets, from scratch ----------

#[test]
fn qapi6_stylesheet_rules_match_each_combinator() {
    let css = ".card:has(.x){color:red} h2:has(+ p){color:red} h3:has(~ .y){color:red} ul:has(> li.z){color:red} \
        section:not(:has(img)){color:red} :is(nav:has(b)){color:red}";
    let body = "<div id=c1 class=card><i class=x>1</i></div><div id=c2 class=card><i>2</i></div>\
        <h2 id=h1>a</h2><p>p</p><h2 id=h2>b</h2><div>d</div>\
        <h3 id=t1>c</h3><div>e</div><div class=y>f</div><h3 id=t2>g</h3>\
        <ul id=u1><li class=z>1</li></ul><ul id=u2><li><b class=z>2</b></li></ul>\
        <section id=s1>no img</section><section id=s2><img></section><nav id=n1><b>b</b></nav><nav id=n2>x</nav>";
    let html = format!("<html><head><style>{css}</style></head><body>{body}</body></html>");
    let code = "console.log(['c1','c2','h1','h2','t1','t2','u1','u2','s1','s2','n1','n2'].map(function (id) {\
        return id + '=' + (getComputedStyle(document.getElementById(id)).color === 'rgb(255, 0, 0)' ? 'R' : '-'); }).join(' '));";
    let log = run_in_html(&html, code);
    assert_eq!(log, vec!["log:c1=R c2=- h1=R h2=- t1=R t2=- u1=R u2=- s1=R s2=- n1=R n2=-".to_string()], "{log:?}");
}

// ---------- the invalidation map ----------

fn map(css: &str) -> InvalidationMap {
    InvalidationMap::build(&parse_stylesheet(css), 1)
}

fn ids(v: &[(Handle, Mark)]) -> Vec<String> {
    let mut o: Vec<String> = v.iter().map(|(h, _)| element_attr(h, "id").map(|s| s.to_string()).unwrap_or_else(|| format!("<{}>", element_tag(h)))).collect();
    o.sort();
    o.dedup();
    o
}

const TREE: &str = "<html><body><div id=top class=card><div id=mid class=card><span id=leaf></span></div><p id=sib></p></div></body></html>";

#[test]
fn qapi6_map_reaches_the_anchors_through_ancestors_filtered_by_the_anchor_compound() {
    let m = map(".card:has(.x){color:red}");
    assert!(!m.is_global() && m.has_has());
    let d = parse_html(TREE);
    // a class on the leaf: both `.card` ancestors are anchors; body/html are not `.card`
    let r = ids(&m.for_class(&by_id(&d, "leaf"), None, Some("x")));
    assert_eq!(r, vec!["mid", "top"]);
    // a class the relative selector does not name: nothing
    assert!(m.for_class(&by_id(&d, "leaf"), None, Some("q")).is_empty());
    // `>`: the same one-hop reach (the parent is among the ancestors)
    let m = map(".card:has(> .x){color:red}");
    assert_eq!(ids(&m.for_class(&by_id(&d, "leaf"), None, Some("x"))), vec!["mid", "top"]);
}

#[test]
fn qapi6_map_sibling_anchors_are_earlier_siblings() {
    let m = map("div:has(+ .x){color:red} div:has(~ p.y){color:blue}");
    let d = parse_html(TREE);
    // `#sib` gets .x: its earlier sibling #mid is the `+` anchor (and earlier siblings of its ancestors: none are divs
    // before #top)
    assert_eq!(ids(&m.for_class(&by_id(&d, "sib"), None, Some("x"))), vec!["mid"]);
    assert_eq!(ids(&m.for_class(&by_id(&d, "sib"), None, Some("y"))), vec!["mid"]);
}

#[test]
fn qapi6_map_insert_and_remove_mark_the_anchors() {
    let m = map(".card:has(img){color:red}");
    let d = parse_html(TREE);
    let leaf = by_id(&d, "leaf");
    let img = parse_html("<html><body><img id=im></body></html>");
    let img = by_id(&img, "im");
    img.parent.set(Some(Rc::downgrade(&leaf)));
    leaf.children.borrow_mut().push(img.clone());
    let r = ids(&m.for_children(&leaf, &ChildChange::Inserted(img.clone())));
    assert!(r.contains(&"mid".to_string()) && r.contains(&"top".to_string()), "{r:?}");
    leaf.children.borrow_mut().clear();
    img.parent.set(None);
    let r = ids(&m.for_children(&leaf, &ChildChange::Removed { node: img, index: 0 }));
    assert_eq!(r, vec!["mid", "top"]);
}

#[test]
fn qapi6_map_has_in_a_non_subject_compound_walks_on_to_the_subject() {
    let m = map(".card:has(.x) > p{color:red}");
    let d = parse_html(TREE);
    // anchors #mid/#top, then their `p` children: #sib only
    assert_eq!(ids(&m.for_class(&by_id(&d, "leaf"), None, Some("x"))), vec!["sib"]);
}

#[test]
fn qapi6_map_nested_combinator_inside_has_is_global() {
    // `:has(:is(.a .b))` can match through an `.a` above the anchor: not bounded
    assert!(map(".card:has(:is(.a .b)){color:red}").is_global());
    assert!(!map(".card:has(:is(.a, .b) .c){color:red}").is_global());
}

#[test]
fn qapi6_map_soundness_fuzz_with_has_shapes() {
    const HAS: [&str; 14] = [
        ".K:has(.K)", "div:has(> .K)", ".K:has(+ .K)", "span:has(~ .K)", ".K:has(.K > .K)", ":not(:has(.K)) .K", ".K:has(.K) + .K",
        ":is(.K:has(span)) > .K", "li:has(:hover)", ".K:has(+ .K .K)", "div:has(:empty)", ":has(> input:checked) .K",
        "p:has(~ .K:nth-child(2))", ".K:has([x])",
    ];
    let n: u64 = std::env::var("SCRATCHY_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let mut t = super::invalidation_tests::Tally { steps: 0, changed: 0, predicted: 0, predicted_on_changed_steps: 0, global_steps: 0 };
    for seed in 1..=n {
        super::invalidation_tests::fuzz_case_with(seed, 12, &mut t, &HAS);
    }
    diag!("qapi6 :has fuzz: {} steps, {} global, changed {}, predicted {}", t.steps, t.global_steps, t.changed, t.predicted);
    assert!(t.changed > 0, "the corpus never changed a match");
    assert_eq!(t.global_steps, 0, "no `:has()` shape here is global");
}

// ---------- the incremental engine (log only) ----------

struct LogOnly(Seam);

impl LogOnly {
    fn new() -> LogOnly {
        incr_style::reset();
        let _ = incr_log::drain();
        set_interaction(&InteractionState::default());
        LogOnly(incr_style::set_seam(Seam { shadow: true, reconcile: false, panic: true }))
    }
}

impl Drop for LogOnly {
    fn drop(&mut self) {
        incr_style::set_seam(self.0);
        set_interaction(&InteractionState::default());
        incr_style::reset();
    }
}

fn build(doc: &Handle) -> Stats {
    let _ = build_layout_tree_sized(doc, 800.0, 600.0);
    incr_style::with_engine(|e| e.expect("shadow engine").stats.clone())
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"))
}

fn engine_color(h: &Handle) -> TextColor {
    incr_style::with_engine(|e| e.unwrap().entry(h).expect("styled").style().unwrap().color)
}

fn set_attr(h: &Handle, name: &str, v: Option<&str>) {
    let old = element_attr(h, name).map(|s| s.to_string());
    match v {
        Some(v) => crate::js_dom::set_attr(h, name, v),
        None => {
            if let NodeData::Element { attrs, .. } = &h.data {
                attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
            }
        }
    }
    incr_log::note_node(h, Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(v) });
}

fn insert_at(parent: &Handle, child: &Handle, at: usize) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().insert(at, child.clone());
    let (prev, next) = incr_log::siblings(child);
    incr_log::note_node(parent, Change::ChildInsert { child: node_id(child), prev, next });
}

fn remove(child: &Handle) {
    let p = parent_handle(child).expect("attached");
    let (prev, next) = incr_log::siblings(child);
    p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, child));
    child.parent.set(None);
    incr_log::note_node(&p, Change::ChildRemove { child: node_id(child), prev, next });
}

fn element(html: &str) -> Handle {
    let d = parse_html(&format!("<html><body>{html}</body></html>"));
    let body = find_by_tag(&d, "body").unwrap();
    let c = body.children.borrow()[0].clone();
    body.children.borrow_mut().clear();
    c.parent.set(None);
    c
}

#[test]
fn qapi6_engine_class_and_attribute_writes_rematch_the_anchor() {
    let _g = LogOnly::new();
    let doc = page(".card:has(.on){color:red} .card:has([data-k]){font-weight:bold}", "<div id=c class=card><p><span id=s>x</span></p></div><div id=o>y</div>");
    build(&doc);
    let c = by_id(&doc, "c");
    assert_eq!(engine_color(&c), TextColor::rgb(0, 0, 0));
    set_attr(&by_id(&doc, "s"), "class", Some("on"));
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(engine_color(&c), TextColor::rgb(255, 0, 0));
    assert!(s.rematched <= 3, "the anchor and the changed element, not the page: {s:?}");
    set_attr(&by_id(&doc, "s"), "class", None);
    build(&doc);
    assert_eq!(engine_color(&c), TextColor::rgb(0, 0, 0));
    set_attr(&by_id(&doc, "s"), "data-k", Some("1"));
    let s = build(&doc);
    assert!(!s.full);
    let w = incr_style::with_engine(|e| e.unwrap().entry(&c).unwrap().style().unwrap().font_weight);
    assert_eq!(w, FontWeight::BOLD);
}

#[test]
fn qapi6_engine_insert_and_remove_rematch_the_anchor() {
    let _g = LogOnly::new();
    let doc = page("ul:has(> li.z){color:red} div:has(img){color:red}", "<ul id=u><li>a</li></ul><div id=d><p id=dp></p></div>");
    build(&doc);
    let (u, d) = (by_id(&doc, "u"), by_id(&doc, "d"));
    let li = element("<li class=z>z</li>");
    insert_at(&u, &li, 1);
    let s = build(&doc);
    assert!(!s.full);
    assert_eq!(engine_color(&u), TextColor::rgb(255, 0, 0));
    remove(&li);
    build(&doc);
    assert_eq!(engine_color(&u), TextColor::rgb(0, 0, 0));
    let img = element("<span><img></span>");
    insert_at(&by_id(&doc, "dp"), &img, 0);
    build(&doc);
    assert_eq!(engine_color(&d), TextColor::rgb(255, 0, 0));
    remove(&img);
    build(&doc);
    assert_eq!(engine_color(&d), TextColor::rgb(0, 0, 0));
}

#[test]
fn qapi6_engine_sibling_anchors() {
    let _g = LogOnly::new();
    let doc = page("h2:has(+ p.x){color:red} h3:has(~ .y){color:red}", "<h2 id=h>t</h2><p id=p>p</p><h3 id=t>u</h3><div>a</div><div id=z>b</div>");
    build(&doc);
    let (h, t) = (by_id(&doc, "h"), by_id(&doc, "t"));
    set_attr(&by_id(&doc, "p"), "class", Some("x"));
    build(&doc);
    assert_eq!(engine_color(&h), TextColor::rgb(255, 0, 0));
    // an element inserted between h2 and p.x breaks the `+`
    let body = find_by_tag(&doc, "body").unwrap();
    let pos = body.children.borrow().iter().position(|c| Rc::ptr_eq(c, &h)).unwrap();
    let e = element("<em>e</em>");
    insert_at(&body, &e, pos + 1);
    build(&doc);
    assert_eq!(engine_color(&h), TextColor::rgb(0, 0, 0));
    remove(&e);
    build(&doc);
    assert_eq!(engine_color(&h), TextColor::rgb(255, 0, 0));
    set_attr(&by_id(&doc, "z"), "class", Some("y"));
    build(&doc);
    assert_eq!(engine_color(&t), TextColor::rgb(255, 0, 0));
}

#[test]
fn qapi6_engine_change_under_display_none_reaches_a_rendered_anchor() {
    let _g = LogOnly::new();
    let doc = page(".card:has(.on){color:red} .hide{display:none}", "<div id=c class=card><div class=hide><p><span id=s>x</span></p></div></div>");
    build(&doc);
    let c = by_id(&doc, "c");
    // #s has no engine entry (under display: none): the log's kept handle finds the anchor
    set_attr(&by_id(&doc, "s"), "class", Some("on"));
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(engine_color(&c), TextColor::rgb(255, 0, 0));
    // an insert / removal under the hidden subtree too
    set_attr(&by_id(&doc, "s"), "class", None);
    build(&doc);
    assert_eq!(engine_color(&c), TextColor::rgb(0, 0, 0));
    let n = element("<b class=on>n</b>");
    insert_at(&by_id(&doc, "s"), &n, 0);
    build(&doc);
    assert_eq!(engine_color(&c), TextColor::rgb(255, 0, 0));
    remove(&n);
    build(&doc);
    assert_eq!(engine_color(&c), TextColor::rgb(0, 0, 0));
}

#[test]
fn qapi6_display_toggled_by_has_with_the_engine() {
    // a dropdown shown only while it has items: the anchor is `display: none` itself
    let _g = LogOnly::new();
    let doc = page(".menu{display:none} .menu:has(li){display:block;color:red}", "<ul id=m class=menu></ul>");
    build(&doc);
    let m = by_id(&doc, "m");
    let li = element("<li>a</li>");
    insert_at(&m, &li, 0);
    build(&doc);
    assert_eq!(engine_color(&m), TextColor::rgb(255, 0, 0));
    remove(&li);
    build(&doc);
}

#[test]
fn qapi6_no_has_page_keeps_no_nodes() {
    // a page without `:has()` pays nothing: the map says so and the log keeps no handles
    let _g = LogOnly::new();
    let doc = page(".a .b{color:red}", "<div id=a><span id=b>x</span></div>");
    build(&doc);
    let m = crate::invalidation::current().expect("map");
    assert!(!m.has_has());
    set_attr(&by_id(&doc, "b"), "class", Some("b"));
    let _ = incr_log::drain();
    assert!(incr_log::taken_node(node_id(&by_id(&doc, "b"))).is_none());
}

#[test]
fn qapi6_engine_unfiltered_anchor_with_outer_hops_marks_only_attribute_subjects() {
    // YouTube's `:has(> [role=dialog]:not([aria-hidden=true]) .x) [dialog]`: the anchors carry no tag/id/class, so
    // every ancestor is one and the descendant hop reaches the whole page; the subject's attribute keeps the marks
    // to `[dialog]` elements, and many writes in one update walk the page once (the batch memo).
    let _g = LogOnly::new();
    let mut body = String::from("<div id=top><div role=dialog id=d aria-hidden=true><span class=x>x</span></div><p dialog id=s>s</p>");
    for i in 0..200 {
        body += &format!("<div><span id=f{i}>f</span></div>");
    }
    body += "</div>";
    let doc = page(":has(> [role=dialog]:not([aria-hidden=true]) .x) [dialog]{color:red}", &body);
    build(&doc);
    let s_el = by_id(&doc, "s");
    assert_eq!(engine_color(&s_el), TextColor::rgb(0, 0, 0));
    set_attr(&by_id(&doc, "d"), "aria-hidden", None);
    for i in 0..50 {
        set_attr(&by_id(&doc, &format!("f{i}")), "aria-hidden", Some("true"));
    }
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(engine_color(&s_el), TextColor::rgb(255, 0, 0));
    assert!(s.rematched <= 60, "the [dialog] subject and the written elements, not the 600-element page: {s:?}");
}

// ---------- script forced layouts: engine on and off agree ----------

#[test]
fn qapi6_forced_layouts_with_has_engine_on_and_off() {
    let html = "<html><head><style>.card{height:10px} .card:has(.big){height:50px} h2:has(+ .x){height:100px} .hide{display:none}</style></head>\
        <body><div id=c class=card><p id=p><span id=s>x</span></p><div class=hide><i id=hid>h</i></div></div><h2 id=h>t</h2><div id=n>n</div>\
        <div id=after>a</div></body></html>";
    let code = "var r = function () { var b = document.getElementById('after').getBoundingClientRect(); return b.y; };\
        var out = [r()];\
        document.getElementById('s').className = 'big'; out.push(r());\
        document.getElementById('s').className = ''; out.push(r());\
        document.getElementById('hid').className = 'big'; out.push(r());\
        document.getElementById('hid').remove(); out.push(r());\
        var b = document.createElement('b'); b.className = 'big'; document.getElementById('p').appendChild(b); out.push(r());\
        b.remove(); out.push(r());\
        document.getElementById('n').className = 'x'; out.push(r());\
        document.getElementById('h').after(document.createElement('hr')); out.push(r());\
        console.log(out.join(' '));";
    let run = |on: bool| {
        let was = crate::incr_engine::set_forced_seam(Some(on));
        let log = run_in_html(html, code);
        crate::incr_engine::set_forced_seam(was);
        log
    };
    let off = run(false);
    let on = run(true);
    assert_eq!(on, off, "the engine agrees with from scratch");
    let ys: Vec<f32> = off[0].trim_start_matches("log:").split(' ').map(|v| v.parse().unwrap()).collect();
    assert_eq!(ys.len(), 9);
    assert!(ys[1] > ys[0] + 30.0, "the card grew: {ys:?}");
    assert_eq!(ys[2], ys[0]);
    assert!(ys[3] > ys[0] + 30.0, "a hidden descendant counts for :has: {ys:?}");
    assert_eq!(ys[4], ys[0]);
    assert!(ys[5] > ys[0] + 30.0);
    assert_eq!(ys[6], ys[0]);
    assert!(ys[7] > ys[6] + 50.0, "h2:has(+ .x) grew: {ys:?}");
    assert!(ys[8] < ys[7], "the hr broke the + : {ys:?}");
}
