//! Q33b: the read-only DOM API for page scripts. Nodes live in Rust
//! (`Handle`s in a table); JS sees small integer ids through native
//! `__d_*` functions, and a JS-side wrapper per id (cached, so `===`
//! holds) turns them into `Element`-like objects. Document id is 0.
use super::*;
use markup5ever_rcdom::Node;
use rquickjs::{Ctx, Function};

#[derive(Default)]
pub(crate) struct DomTable {
    nodes: Vec<Handle>,
    ids: HashMap<usize, i32>,
    /// QA18a: ids of extra (detached) `Document` roots; id 0 is the page.
    docs: std::collections::HashSet<i32>,
}

impl DomTable {
    pub(crate) fn intern(&mut self, h: &Handle) -> i32 {
        let key = Rc::as_ptr(h) as usize;
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = self.nodes.len() as i32;
        self.nodes.push(h.clone());
        self.ids.insert(key, id);
        id
    }
    /// A `Document`-data node that is a fragment (not a real document).
    fn is_frag(&self, id: i32, h: &Handle) -> bool {
        is_fragment(id, h) && !self.docs.contains(&id)
    }
    fn get(&self, id: i32) -> Option<Handle> {
        usize::try_from(id).ok().and_then(|i| self.nodes.get(i)).cloned()
    }
}

/// QY8: index of `node` in `kids`. `hint` is the index it had last time; if
/// `kids[hint]` is still `node` the lookup is O(1), else scan (and the caller
/// stores the fresh index). Never stale: the hint is verified every call.
pub(crate) fn child_index(kids: &[Handle], node: &Handle, hint: usize, steps: &mut usize) -> Option<usize> {
    *steps += 1;
    if kids.get(hint).is_some_and(|k| Rc::ptr_eq(k, node)) {
        return Some(hint);
    }
    kids.iter().position(|k| {
        *steps += 1;
        Rc::ptr_eq(k, node)
    })
}

fn is_element(h: &Handle) -> bool {
    matches!(h.data, NodeData::Element { .. })
}

/// DOM `nodeType` of a node.
fn node_type(h: &Handle) -> i32 {
    match &h.data {
        NodeData::Element { .. } => 1,
        NodeData::Text { .. } => 3,
        NodeData::Comment { .. } => 8,
        NodeData::Document => 9,
        NodeData::Doctype { .. } => 10,
        _ => 0,
    }
}

fn descendants(root: &Handle, out: &mut Vec<Handle>) {
    for c in root.children.borrow().iter() {
        if is_element(c) {
            out.push(c.clone());
        }
        descendants(c, out);
    }
}

fn text_content(h: &Handle, out: &mut String) {
    match &h.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { .. } | NodeData::Document => {
            for c in h.children.borrow().iter() {
                text_content(c, out);
            }
        }
        _ => {}
    }
}

/// Parses a selector string; `None` when it is invalid.
fn parse_selector(sel: &str) -> Option<SelectorList<OurSelectorImpl>> {
    let mut input = CssParser::new(sel);
    let list = SelectorList::parse(&OurSelectorParser, &mut input, ParseRelative::No).ok()?;
    input.expect_exhausted().ok()?;
    Some(list)
}

fn matches_list(list: &SelectorList<OurSelectorImpl>, h: &Handle) -> bool {
    let mut caches = SelectorCaches::default();
    let mut ctx = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    matches_selector_list(list, &El(h.clone()), &mut ctx)
}

fn html_name(local: &str) -> html5ever::QualName {
    html5ever::QualName::new(None, html5ever::ns!(html), html5ever::LocalName::from(local))
}

/// QY6f: the content fragment of an HTML `<template>` (created on demand);
/// `None` for anything else. html5ever fills it while parsing.
fn template_content(h: &Handle) -> Option<Handle> {
    match &h.data {
        NodeData::Element { name, template_contents, .. }
            if name.local.as_ref() == "template" && name.ns == html5ever::ns!(html) =>
        {
            Some(template_contents.borrow_mut().get_or_insert_with(|| Node::new(NodeData::Document)).clone())
        }
        _ => None,
    }
}

/// Removes `h` from its parent's child list (no-op when detached).
fn detach(h: &Handle) {
    if let Some(p) = parent_handle(h) {
        p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, h));
    }
    h.parent.set(None);
}

fn is_ancestor_or_self(anc: &Handle, mut n: Handle) -> bool {
    loop {
        if Rc::ptr_eq(anc, &n) {
            return true;
        }
        match parent_handle(&n) {
            Some(p) => n = p,
            None => return false,
        }
    }
}

fn is_fragment(id: i32, h: &Handle) -> bool {
    id != 0 && matches!(h.data, NodeData::Document)
}

/// Inserts `child` into `parent` before `before` (end when `None`),
/// moving it from wherever it was; a fragment donates its children.
/// `false` when that would put a node inside itself.
fn insert_child(parent: &Handle, child: &Handle, before: Option<&Handle>, frag: bool) -> bool {
    if is_ancestor_or_self(child, parent.clone()) {
        return false;
    }
    let moved: Vec<Handle> = if frag { child.children.borrow_mut().drain(..).collect() } else { vec![child.clone()] };
    if !frag {
        detach(child);
    }
    let mut kids = parent.children.borrow_mut();
    let at = before.and_then(|b| kids.iter().position(|c| Rc::ptr_eq(c, b))).unwrap_or(kids.len());
    for m in &moved {
        m.parent.set(Some(Rc::downgrade(parent)));
    }
    kids.splice(at..at, moved);
    true
}

fn set_attr(h: &Handle, name: &str, value: &str) {
    if let NodeData::Element { attrs, .. } = &h.data {
        let mut attrs = attrs.borrow_mut();
        if let Some(a) = attrs.iter_mut().find(|a| a.name.local.as_ref() == name) {
            a.value = value.into();
        } else {
            let name = html5ever::QualName::new(None, html5ever::ns!(), html5ever::LocalName::from(name));
            attrs.push(html5ever::Attribute { name, value: value.into() });
        }
    }
}

fn remove_attr(h: &Handle, name: &str) {
    if let NodeData::Element { attrs, .. } = &h.data {
        attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
    }
}

fn clear_children(h: &Handle) {
    for c in h.children.borrow_mut().drain(..) {
        c.parent.set(None);
    }
}

fn set_children_text(h: &Handle, text: &str) {
    clear_children(h);
    if !text.is_empty() {
        let t = Node::new(NodeData::Text { contents: RefCell::new(text.into()) });
        t.parent.set(Some(Rc::downgrade(h)));
        h.children.borrow_mut().push(t);
    }
}

/// `innerHTML = html`: fragment-parses in the context of `h`.
fn set_inner_html(h: &Handle, html: &str) {
    // QY6f: a template's innerHTML is its content's.
    let target = template_content(h).unwrap_or_else(|| h.clone());
    clear_children(&target);
    for k in parse_fragment_in(h, html) {
        k.parent.set(Some(Rc::downgrade(&target)));
        target.children.borrow_mut().push(k);
    }
}

/// Fragment-parses `html` in the context of `h`; the returned nodes have
/// no parent yet.
fn parse_fragment_in(h: &Handle, html: &str) -> Vec<Handle> {
    let ctx = match &h.data {
        NodeData::Element { name, .. } => name.clone(),
        _ => html_name("div"),
    };
    let mut opts = html5ever::driver::ParseOpts::default();
    opts.tree_builder.scripting_enabled = js_enabled();
    let dom = html5ever::driver::parse_fragment(RcDom::default(), opts, ctx, Vec::new(), false).one(html);
    // The parser wraps the fragment in one synthetic root element.
    let root = dom.document.children.borrow().first().cloned();
    match root {
        Some(root) => root.children.borrow_mut().drain(..).collect(),
        None => Vec::new(),
    }
}

/// QA13: `document.write`. Parses `html` in the context of `after`'s
/// parent and inserts the nodes right after `after`. Returns the last
/// inserted node (or `after` when nothing was produced) so the next
/// write continues from there.
fn write_after(after: &Handle, html: &str) -> Handle {
    let Some(parent) = parent_handle(after) else { return after.clone() };
    let kids = parse_fragment_in(&parent, html);
    let last = kids.last().cloned().unwrap_or_else(|| after.clone());
    let mut ch = parent.children.borrow_mut();
    let at = ch.iter().position(|c| Rc::ptr_eq(c, after)).map_or(ch.len(), |i| i + 1);
    for k in &kids {
        k.parent.set(Some(Rc::downgrade(&parent)));
    }
    ch.splice(at..at, kids);
    last
}

fn serialize_node(h: &Handle, outer: bool) -> String {
    use html5ever::serialize::{serialize, Serialize, SerializeOpts, Serializer, TraversalScope};
    // QY6f: like rcdom's serializer, but a `<template>` prints its content.
    struct Ser(Handle);
    fn kids(h: &Handle) -> Vec<Handle> {
        match template_content(h) {
            Some(c) => c.children.borrow().clone(),
            None => h.children.borrow().clone(),
        }
    }
    fn node<S: Serializer>(s: &mut S, h: &Handle) -> std::io::Result<()> {
        match &h.data {
            NodeData::Element { name, attrs, .. } => {
                s.start_elem(name.clone(), attrs.borrow().iter().map(|a| (&a.name, &a.value[..])))?;
                for c in kids(h) {
                    node(s, &c)?;
                }
                s.end_elem(name.clone())
            }
            NodeData::Doctype { name, .. } => s.write_doctype(name),
            NodeData::Text { contents } => s.write_text(&contents.borrow()),
            NodeData::Comment { contents } => s.write_comment(contents),
            NodeData::ProcessingInstruction { target, contents } => s.write_processing_instruction(target, contents),
            NodeData::Document => {
                for c in h.children.borrow().clone() {
                    node(s, &c)?;
                }
                Ok(())
            }
        }
    }
    impl Serialize for Ser {
        fn serialize<S: Serializer>(&self, s: &mut S, scope: TraversalScope) -> std::io::Result<()> {
            match scope {
                TraversalScope::IncludeNode => node(s, &self.0),
                TraversalScope::ChildrenOnly(_) => {
                    for c in kids(&self.0) {
                        node(s, &c)?;
                    }
                    Ok(())
                }
            }
        }
    }
    let mut out = Vec::new();
    let opts = SerializeOpts {
        traversal_scope: if outer { TraversalScope::IncludeNode } else { TraversalScope::ChildrenOnly(None) },
        ..Default::default()
    };
    let _ = serialize(&mut out, &Ser(h.clone()), opts);
    String::from_utf8_lossy(&out).into_owned()
}

fn clone_node(h: &Handle, deep: bool) -> Handle {
    let data = match &h.data {
        NodeData::Element { name, attrs, mathml_annotation_xml_integration_point, .. } => NodeData::Element {
            name: name.clone(),
            attrs: RefCell::new(attrs.borrow().clone()),
            template_contents: RefCell::new(None),
            mathml_annotation_xml_integration_point: *mathml_annotation_xml_integration_point,
        },
        NodeData::Text { contents } => NodeData::Text { contents: RefCell::new(contents.borrow().clone()) },
        NodeData::Comment { contents } => NodeData::Comment { contents: contents.clone() },
        _ => NodeData::Document,
    };
    let n = Node::new(data);
    // QY6f: cloning a template copies its content (deep clones only).
    if deep {
        if let (Some(src), NodeData::Element { template_contents, .. }) = (template_content(h), &n.data) {
            *template_contents.borrow_mut() = Some(clone_node(&src, true));
        }
        for c in h.children.borrow().iter() {
            let cc = clone_node(c, true);
            cc.parent.set(Some(Rc::downgrade(&n)));
            n.children.borrow_mut().push(cc);
        }
    }
    n
}

/// Installs the `__d_*` natives for `document` and the JS wrappers on top.
/// QB5: requests page script made of the App (which owns focus and the
/// scroll offset); drained by `App::js_sync` through `take_requests`.
#[derive(Default, Debug, Clone)]
pub(crate) struct JsRequests {
    pub(crate) focus: Option<FocusReq>,
    /// `window.scroll*`: target axes (`None` = keep), `relative` = scrollBy.
    pub(crate) scroll: Option<ScrollReq>,
    /// Q33d1: `el.click()` on a link asks the App to follow it.
    pub(crate) nav: Option<String>,
    /// QF7: validated page-network requests for the App's scheduler.
    pub(crate) net: Vec<crate::js_net::NetJob>,
    /// QF7: results known at once (refused requests, suppressed beacons);
    /// delivered asynchronously like any other result.
    pub(crate) net_done: Vec<(u64, Result<WireResponse, NetError>)>,
    /// QF7: ids the page gave up on that may already be queued or running.
    pub(crate) net_abort: Vec<u64>,
}

#[derive(Debug, Clone)]
pub(crate) enum FocusReq {
    Focus(Handle),
    /// Only blurs if that element is the focused one.
    Blur(Handle),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrollReq {
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
    pub(crate) relative: bool,
}

/// QB5: what `focus()` accepts. Form controls count even though Tab
/// navigation skips them until Q30 makes them real controls.
fn js_focusable(h: &Handle) -> bool {
    if is_focusable(h) {
        return true;
    }
    let control = match element_tag(h) {
        "input" => !element_attr(h, "type").is_some_and(|t| t.eq_ignore_ascii_case("hidden")),
        "button" | "select" | "textarea" => true,
        _ => false,
    };
    control && element_attr(h, "disabled").is_none()
}

pub(crate) fn install_dom(ctx: &Ctx<'_>, document: &Handle, dirty: Rc<Cell<bool>>, reqs: Rc<RefCell<JsRequests>>) -> Rc<RefCell<DomTable>> {
    let table = Rc::new(RefCell::new(DomTable::default()));
    table.borrow_mut().intern(document);
    let globals = ctx.globals();
    // QA16: DOM write generation (invalidates the getComputedStyle cache).
    let wgen: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    macro_rules! native {
        ($name:expr, $f:expr) => {
            if let Ok(f) = Function::new(ctx.clone(), $f) {
                let _ = globals.set($name, f);
            }
        };
    }
    let t = table.clone();
    native!("__d_byid", move |id: String, doc: i32| -> i32 {
        let mut t = t.borrow_mut();
        let Some(root) = t.get(doc) else { return -1 };
        let mut all = Vec::new();
        descendants(&root, &mut all);
        match all.iter().find(|h| element_attr(h, "id").as_deref() == Some(id.as_str())) {
            Some(h) => t.intern(h),
            None => -1,
        }
    });
    // `None` (-> JS SyntaxError) for an invalid selector.
    let t = table.clone();
    native!("__d_query", move |root: i32, sel: String, first: bool| -> Option<Vec<i32>> {
        let list = parse_selector(&sel)?;
        let mut t = t.borrow_mut();
        let root = t.get(root)?;
        let mut all = Vec::new();
        descendants(&root, &mut all);
        let mut out = Vec::new();
        for h in all.iter().filter(|h| matches_list(&list, h)) {
            out.push(t.intern(h));
            if first {
                break;
            }
        }
        Some(out)
    });
    let t = table.clone();
    native!("__d_matches", move |id: i32, sel: String| -> Option<bool> {
        let list = parse_selector(&sel)?;
        let h = t.borrow().get(id)?;
        Some(is_element(&h) && matches_list(&list, &h))
    });
    let t = table.clone();
    native!("__d_kind", move |id: i32| -> i32 {
        let t = t.borrow();
        t.get(id).map_or(0, |h| if t.is_frag(id, &h) { 11 } else { node_type(&h) })
    });
    let t = table.clone();
    native!("__d_ns", move |id: i32| -> String {
        match t.borrow().get(id).as_deref().map(|h| &h.data) {
            Some(NodeData::Element { name, .. }) => name.ns.as_ref().to_string(),
            _ => String::new(),
        }
    });
    let t = table.clone();
    native!("__d_tag", move |id: i32| -> String {
        let Some(h) = t.borrow().get(id) else { return String::new() };
        match &h.data {
            NodeData::Element { name, .. } => {
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" {
                    name.local.as_ref().to_ascii_uppercase()
                } else {
                    name.local.as_ref().to_string()
                }
            }
            NodeData::Text { .. } => "#text".into(),
            NodeData::Comment { .. } => "#comment".into(),
            NodeData::Doctype { name, .. } => name.to_string(),
            NodeData::Document if id != 0 && !t.borrow().docs.contains(&id) => "#document-fragment".into(),
            NodeData::Document => "#document".into(),
            _ => String::new(),
        }
    });
    let t = table.clone();
    native!("__d_attr", move |id: i32, name: String| -> Option<String> {
        let h = t.borrow().get(id)?;
        element_attr(&h, &name.to_ascii_lowercase()).map(|v| v.to_string())
    });
    // QY11: attribute names in stored order (for `attributes`, `dataset`).
    let t = table.clone();
    native!("__d_attr_names", move |id: i32| -> Vec<String> {
        let Some(h) = t.borrow().get(id) else { return Vec::new() };
        match &h.data {
            NodeData::Element { attrs, .. } => attrs.borrow().iter().map(|a| a.name.local.as_ref().to_string()).collect(),
            _ => Vec::new(),
        }
    });
    let t = table.clone();
    native!("__d_text", move |id: i32| -> String {
        let mut s = String::new();
        if let Some(h) = t.borrow().get(id) {
            match &h.data {
                NodeData::Comment { contents } => s.push_str(contents),
                _ => text_content(&h, &mut s),
            }
        }
        s
    });
    let t = table.clone();
    native!("__d_children", move |id: i32, elements_only: bool| -> Vec<i32> {
        let mut t = t.borrow_mut();
        let Some(h) = t.get(id) else { return Vec::new() };
        let kids: Vec<Handle> = h.children.borrow().iter().cloned().collect();
        kids.iter().filter(|k| !elements_only || is_element(k)).map(|k| t.intern(k)).collect()
    });
    // QY8: sibling / first / last child without building `childNodes`.
    // `hints` remembers each node's last index in its parent's Vec; it is
    // verified on use (see `child_index`), so no write needs to invalidate it.
    let hints: Rc<RefCell<HashMap<i32, usize>>> = Rc::new(RefCell::new(HashMap::new()));
    let t = table.clone();
    let hs = hints.clone();
    native!("__d_sibling", move |id: i32, dir: i32, elements_only: bool| -> i32 {
        let mut t = t.borrow_mut();
        let Some(h) = t.get(id) else { return -1 };
        let Some(p) = parent_handle(&h) else { return -1 };
        let kids = p.children.borrow();
        let hint = hs.borrow().get(&id).copied().unwrap_or(0);
        let mut steps = 0;
        let Some(mut i) = child_index(&kids, &h, hint, &mut steps) else { return -1 };
        hs.borrow_mut().insert(id, i);
        loop {
            i = if dir > 0 { i + 1 } else if i == 0 { return -1 } else { i - 1 };
            let Some(k) = kids.get(i) else { return -1 };
            if !elements_only || is_element(k) {
                let kid = t.intern(k);
                hs.borrow_mut().insert(kid, i);
                return kid;
            }
        }
    });
    let t = table.clone();
    native!("__d_edge_child", move |id: i32, last: bool, elements_only: bool| -> i32 {
        let mut t = t.borrow_mut();
        let Some(h) = t.get(id) else { return -1 };
        let kids = h.children.borrow();
        let found = if last { kids.iter().rev().find(|k| !elements_only || is_element(k)) } else { kids.iter().find(|k| !elements_only || is_element(k)) };
        match found {
            Some(k) => t.intern(k),
            None => -1,
        }
    });
    let t = table.clone();
    native!("__d_parent", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id).and_then(|h| parent_handle(&h)) {
            Some(p) => t.intern(&p),
            None => -1,
        }
    });
    // 0 = documentElement, 1 = head, 2 = body, of document `doc`.
    let t = table.clone();
    native!("__d_special", move |which: i32, doc: i32| -> i32 {
        let mut t = t.borrow_mut();
        let Some(root) = t.get(doc) else { return -1 };
        let mut all = Vec::new();
        descendants(&root, &mut all);
        let want = ["html", "head", "body"][which.clamp(0, 2) as usize];
        // The document element is the first element child of the root.
        let first = root.children.borrow().iter().find(|c| is_element(c)).cloned();
        let found = if which == 0 { first } else { all.iter().find(|h| element_tag(h) == want).cloned() };
        match found.as_ref() {
            Some(h) => t.intern(h),
            None => -1,
        }
    });
    // QA16: serialised computed value of `prop` (dashed) for element `id`.
    let (t, gn) = (table.clone(), wgen.clone());
    let cache = Rc::new(RefCell::new(js_style::StyleCache::default()));
    native!("__d_cs", move |id: i32, pseudo: String, prop: String| -> String {
        let (page, h) = {
            let t = t.borrow();
            (t.get(0).unwrap(), t.get(id))
        };
        let Some(h) = h else { return String::new() };
        // QA18c: the cascade runs in the node's own document.
        let mut top = h.clone();
        while let Some(p) = parent_handle(&top) {
            top = p;
        }
        let is_page = Rc::ptr_eq(&top, &page);
        let is_doc = is_page || {
            let mut t = t.borrow_mut();
            let tid = t.intern(&top);
            t.docs.contains(&tid)
        };
        let root = if is_doc { top } else { page };
        let p = pseudo.trim_start_matches(':').to_ascii_lowercase();
        let p = match p.as_str() {
            "before" => Some("before"),
            "after" => Some("after"),
            _ => None,
        };
        match cache.borrow_mut().style_of(&root, gn.get(), &h, p, is_page) {
            Some(s) => js_style::serialize(&s, &prop),
            None => String::new(),
        }
    });
    native!("__d_csprops", move || -> Vec<String> { js_style::PROPS.iter().map(|s| s.to_string()).collect() });
    // ---- Q33c mutation. Every write marks the page dirty; the event
    // loop rebuilds layout once per turn, not once per call.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_setattr", move |id: i32, name: String, val: String| {
        if let Some(h) = t.borrow().get(id) {
            set_attr(&h, &name.to_ascii_lowercase(), &val);
            d.set(true);
            gn.set(gn.get() + 1);
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_rmattr", move |id: i32, name: String| {
        if let Some(h) = t.borrow().get(id) {
            remove_attr(&h, &name.to_ascii_lowercase());
            d.set(true);
            gn.set(gn.get() + 1);
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_settext", move |id: i32, text: String| {
        if let Some(h) = t.borrow().get(id) {
            match &h.data {
                NodeData::Text { contents } => *contents.borrow_mut() = text.as_str().into(),
                NodeData::Comment { .. } => {}
                _ => set_children_text(&h, &text),
            }
            d.set(true);
            gn.set(gn.get() + 1);
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_sethtml", move |id: i32, html: String| {
        if let Some(h) = t.borrow().get(id) {
            set_inner_html(&h, &html);
            d.set(true);
            gn.set(gn.get() + 1);
        }
    });
    // QA13: returns the id of the last inserted node, or -1.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_write", move |after: i32, html: String| -> i32 {
        let mut t = t.borrow_mut();
        let Some(a) = t.get(after) else { return -1 };
        let last = write_after(&a, &html);
        d.set(true);
        gn.set(gn.get() + 1);
        t.intern(&last)
    });
    let t = table.clone();
    native!("__d_gethtml", move |id: i32, outer: bool| -> String {
        t.borrow().get(id).map(|h| serialize_node(&h, outer)).unwrap_or_default()
    });
    // kind: 1 element (`s` = tag), 3 text, 8 comment, 11 fragment.
    let t = table.clone();
    native!("__d_create", move |kind: i32, s: String| -> i32 {
        let data = match kind {
            1 => NodeData::Element {
                name: html_name(&s.to_ascii_lowercase()),
                attrs: RefCell::new(Vec::new()),
                template_contents: RefCell::new(None),
                mathml_annotation_xml_integration_point: false,
            },
            3 => NodeData::Text { contents: RefCell::new(s.as_str().into()) },
            8 => NodeData::Comment { contents: s.as_str().into() },
            _ => NodeData::Document,
        };
        t.borrow_mut().intern(&Node::new(data))
    });
    // QY6f: id of a template's content fragment (-1 when not a template).
    let t = table.clone();
    native!("__d_template_content", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        let content = t.get(id).as_ref().and_then(template_content);
        match content {
            Some(c) => t.intern(&c),
            None => -1,
        }
    });
    // QA18a: a new detached Document root (its id is registered as a
    // document, not a fragment); a doctype node (name, public, system).
    let t = table.clone();
    native!("__d_newdoc", move || -> i32 {
        let mut t = t.borrow_mut();
        let id = t.intern(&Node::new(NodeData::Document));
        t.docs.insert(id);
        id
    });
    // QA18b1: [absolute url, html] of a pre-fetched iframe `src` (empty when
    // the iframe's src was not pre-fetched); `__d_load_html` parses a full
    // document into a second document's root.
    let t = table.clone();
    native!("__d_iframe_page", move |id: i32| -> Vec<String> {
        let t = t.borrow();
        let (Some(h), Some(root)) = (t.get(id), t.get(0)) else { return Vec::new() };
        let Some(src) = element_attr(&h, "src") else { return Vec::new() };
        let base = find_base_url(&root).or_else(crate::js::page_url);
        let Ok(url) = resolve_href(base.as_ref(), src.trim()) else { return Vec::new() };
        match crate::net::iframe_page(url.as_str()) {
            Some(html) => vec![url.to_string(), html],
            None => Vec::new(),
        }
    });
    let t = table.clone();
    native!("__d_load_html", move |id: i32, html: String| {
        let Some(doc) = t.borrow().get(id) else { return };
        let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return };
        clear_children(&doc);
        let kids: Vec<Handle> = dom.document.children.borrow_mut().drain(..).collect();
        for k in kids {
            k.parent.set(Some(Rc::downgrade(&doc)));
            doc.children.borrow_mut().push(k);
        }
    });
    let t = table.clone();
    native!("__d_create_doctype", move |name: String, public_id: String, system_id: String| -> i32 {
        let data = NodeData::Doctype { name: name.as_str().into(), public_id: public_id.as_str().into(), system_id: system_id.as_str().into() };
        t.borrow_mut().intern(&Node::new(data))
    });
    let t = table.clone();
    native!("__d_doctype", move |id: i32| -> Vec<String> {
        match t.borrow().get(id).as_deref().map(|h| &h.data) {
            Some(NodeData::Doctype { name, public_id, system_id }) => vec![name.to_string(), public_id.to_string(), system_id.to_string()],
            _ => Vec::new(),
        }
    });
    // QA14: createElementNS keeps the qualified name's case outside XHTML.
    let t = table.clone();
    native!("__d_create_ns", move |ns: String, name: String| -> i32 {
        let local = if ns.is_empty() || ns == "http://www.w3.org/1999/xhtml" { name.to_ascii_lowercase() } else { name };
        let ns = if ns.is_empty() { html5ever::ns!(html) } else { html5ever::Namespace::from(ns.as_str()) };
        let data = NodeData::Element {
            name: html5ever::QualName::new(None, ns, html5ever::LocalName::from(local.as_str())),
            attrs: RefCell::new(Vec::new()),
            template_contents: RefCell::new(None),
            mathml_annotation_xml_integration_point: false,
        };
        t.borrow_mut().intern(&Node::new(data))
    });
    let t = table.clone();
    native!("__d_clone", move |id: i32, deep: bool| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id) {
            Some(h) => t.intern(&clone_node(&h, deep)),
            None => -1,
        }
    });
    // `before` < 0 appends. `false` = HierarchyRequestError.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_insert", move |parent: i32, child: i32, before: i32| -> bool {
        let t = t.borrow();
        let (Some(p), Some(c)) = (t.get(parent), t.get(child)) else { return false };
        if !matches!(p.data, NodeData::Element { .. } | NodeData::Document) {
            return false;
        }
        let ok = insert_child(&p, &c, t.get(before).as_ref(), t.is_frag(child, &c));
        d.set(true);
        gn.set(gn.get() + 1);
        ok
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!("__d_remove", move |id: i32| {
        if let Some(h) = t.borrow().get(id) {
            detach(&h);
            d.set(true);
            gn.set(gn.get() + 1);
        }
    });
    // QB5: focus()/blur() only record a request; the App applies it.
    let (t, r) = (table.clone(), reqs.clone());
    native!("__d_focus", move |id: i32, focus: bool| {
        if let Some(h) = t.borrow().get(id) {
            if focus && js_focusable(&h) {
                r.borrow_mut().focus = Some(FocusReq::Focus(h));
            } else if !focus {
                r.borrow_mut().focus = Some(FocusReq::Blur(h));
            }
        }
    });
    let r = reqs.clone();
    native!("__d_navigate", move |href: String| {
        r.borrow_mut().nav = Some(href);
    });
    let r = reqs;
    native!("__d_scroll", move |x: f64, y: f64, relative: bool| {
        let f = |v: f64| if v.is_finite() { Some(v) } else { None };
        r.borrow_mut().scroll = Some(ScrollReq { x: f(x), y: f(y), relative });
    });
    let _ = ctx.eval::<(), _>(DOM_PRELUDE);
    table
}

/// Wrappers over the natives; runs after the base prelude (which defines
/// `document`). Element event listeners are stored but never fired yet.
const DOM_PRELUDE: &str = r#"
(function (g) {
  var doc = g.document, cache = {}, tagProto = Object.create(null);
  // QY6e: the mutation natives the custom-element hooks rebind (once, at the
  // first `customElements.define`); until then nothing is wrapped (zero cost).
  var __d_insert = g.__d_insert, __d_remove = g.__d_remove, __d_sethtml = g.__d_sethtml, __d_settext = g.__d_settext,
    __d_setattr = g.__d_setattr, __d_rmattr = g.__d_rmattr, __d_write = g.__d_write, ceNew;
  // QY6a: live MutationObserver registrations (0 until the first `observe()`),
  // and the childList batch used by multi-step APIs (replaceChild, append(...a, b)).
  var moN = 0, moBatch = null, moFlush;
  var moBegin = function () { if (moN > 0 && !moBatch) { moBatch = []; return true; } return false; };
  var moEnd = function (own) { if (own) moFlush(); };
  var queries, svgProto = Object.create(null);
  var wrap = function (id) {
    if (id === undefined || id < 0) return null;
    if (cache[id]) return cache[id];
    var k = __d_kind(id), raw = k === 1 ? __d_tag(id) : '', tg = raw.toLowerCase(), eP = null;
    if (k === 1) {
      // QY12: SVG-namespace elements get the SVG interfaces (by local name), unknown
      // non-custom HTML names HTMLUnknownElement, custom names plain HTMLElement.
      var ns = raw === raw.toUpperCase() ? '' : __d_ns(id);
      eP = ns === 'http://www.w3.org/2000/svg' ? (svgProto[raw] || SVGElement.prototype)
        : ns ? Element.prototype : tagProto[tg] || (tg.indexOf('-') < 0 ? HTMLUnknownElement : HTMLElement).prototype;
    }
    var o = Object.create(k === 1 ? eP
      : k === 3 ? Text.prototype : k === 8 ? Comment.prototype : k === 11 ? DocumentFragment.prototype
      : k === 10 ? DocumentType.prototype : Node.prototype);
    Object.defineProperty(o, '__id', { value: id });
    return (cache[id] = o);
  };
  var wrapAll = function (ids) { return ids.map(function (id) { return wrap(id); }); };
  var bad = function (sel) {
    return new DOMException("'" + sel + "' is not a valid selector", 'SyntaxError');
  };
  var q = function (id, sel, first) {
    var r = __d_query(id, String(sel), first);
    if (r === undefined || r === null) throw bad(sel);
    return r;
  };
  var def = function (o, name, get) { Object.defineProperty(o, name, { get: get, configurable: true, enumerable: true }); };
  // Q33b1: hang the accessors off the real constructors' prototypes so
  // `x instanceof Element` and `Element.prototype.foo = ...` work.
  var NodeProto = Node.prototype;
  def(NodeProto, 'nodeType', function () { return __d_kind(this.__id); });
  def(NodeProto, 'nodeName', function () { return __d_tag(this.__id); });
  def(NodeProto, 'textContent', function () { return __d_text(this.__id); });
  def(NodeProto, 'nodeValue', function () { return this.nodeType === 1 ? null : __d_text(this.__id); });
  // QA18a: a node's owner is the document it is attached under, else
  // the document that created/adopted its tree's root (`owners`), else the page.
  var owners = {};
  def(NodeProto, 'ownerDocument', function () {
    if (this.nodeType === 9) return null;
    var r = this, p;
    while ((p = __d_parent(r.__id)) >= 0) r = wrap(p);
    if (r.nodeType === 9 && r !== this) return r;
    return owners[r.__id] || owners[this.__id] || doc;
  });
  def(CharacterData.prototype, 'data', function () { return __d_text(this.__id); });
  def(CharacterData.prototype, 'length', function () { return __d_text(this.__id).length; });
  def(NodeProto, 'parentNode', function () { return wrap(__d_parent(this.__id)); });
  def(NodeProto, 'parentElement', function () {
    var p = this.parentNode; return p && p.nodeType === 1 ? p : null;
  });
  def(NodeProto, 'childNodes', function () { return wrapAll(__d_children(this.__id, false)); });
  def(NodeProto, 'firstChild', function () { return wrap(__d_edge_child(this.__id, false, false)); });
  def(NodeProto, 'lastChild', function () { return wrap(__d_edge_child(this.__id, true, false)); });
  def(NodeProto, 'nextSibling', function () { return wrap(__d_sibling(this.__id, 1, false)); });
  def(NodeProto, 'previousSibling', function () { return wrap(__d_sibling(this.__id, -1, false)); });
  NodeProto.hasChildNodes = function () { return __d_children(this.__id, false).length > 0; };
  NodeProto.contains = function (n) {
    for (; n; n = n.parentNode) if (n === this) return true;
    return false;
  };
  // QY16b: DOM spec "equals" over node ids. Attributes compare as sets (order
  // ignored); children compare pairwise in order.
  var eqNode = function (a, b) {
    var k = __d_kind(a);
    if (k !== __d_kind(b)) return false;
    if (k === 1) {
      if (__d_tag(a) !== __d_tag(b) || __d_ns(a) !== __d_ns(b)) return false;
      var na = __d_attr_names(a), nb = __d_attr_names(b);
      if (na.length !== nb.length) return false;
      for (var i = 0; i < na.length; i++) {
        var v = __d_attr(b, na[i]);
        if (v == null || v !== __d_attr(a, na[i])) return false;
      }
    } else if (k === 10) {
      var da = __d_doctype(a), db = __d_doctype(b);
      if (da[0] !== db[0] || da[1] !== db[1] || da[2] !== db[2]) return false;
    } else if (k === 3 || k === 8) {
      if (__d_text(a) !== __d_text(b)) return false;
    }
    var ca = __d_children(a, false), cb = __d_children(b, false);
    if (ca.length !== cb.length) return false;
    for (var j = 0; j < ca.length; j++) if (!eqNode(ca[j], cb[j])) return false;
    return true;
  };
  NodeProto.isEqualNode = function (other) {
    if (other == null) return false;
    if (other.__id === undefined) throw new TypeError("Failed to execute 'isEqualNode' on 'Node': parameter 1 is not of type 'Node'.");
    return eqNode(this.__id, other.__id);
  };
  NodeProto.isSameNode = function (other) { return this === other; };
  queries = function (P) {
    P.querySelector = function (s) { return wrap(q(this.__id, s, true)[0]); };
    P.querySelectorAll = function (s) { return wrapAll(q(this.__id, s, false)); };
    P.getElementsByTagName = function (t) { return wrapAll(q(this.__id, String(t).toLowerCase(), false)); };
    P.getElementsByClassName = function (c) {
      var sel = String(c).split(/\s+/).filter(Boolean).map(function (x) { return '.' + x; }).join('');
      return sel ? wrapAll(q(this.__id, sel, false)) : [];
    };
  };
  var ElementProto = Element.prototype;
  queries(ElementProto);
  // QSD0: ParentNode queries live on the prototype (zero per-wrapper cost), so a page
  // may freeze or replace them without breaking fragment wrappers.
  queries(DocumentFragment.prototype);
  def(ElementProto, 'tagName', function () { return __d_tag(this.__id); });
  def(ElementProto, 'localName', function () { return __d_tag(this.__id).toLowerCase(); });
  def(ElementProto, 'id', function () { var v = __d_attr(this.__id, 'id'); return v == null ? '' : v; });
  def(ElementProto, 'className', function () { var v = __d_attr(this.__id, 'class'); return v == null ? '' : v; });
  def(ElementProto, 'children', function () { return wrapAll(__d_children(this.__id, true)); });
  def(ElementProto, 'firstElementChild', function () { return wrap(__d_edge_child(this.__id, false, true)); });
  def(ElementProto, 'lastElementChild', function () { return wrap(__d_edge_child(this.__id, true, true)); });
  def(ElementProto, 'nextElementSibling', function () { return wrap(__d_sibling(this.__id, 1, true)); });
  def(ElementProto, 'previousElementSibling', function () { return wrap(__d_sibling(this.__id, -1, true)); });
  def(ElementProto, 'childElementCount', function () { return __d_children(this.__id, true).length; });
  def(ElementProto, 'classList', function () {
    var cls = this.className.split(/\s+/).filter(Boolean);
    return {
      length: cls.length,
      contains: function (c) { return cls.indexOf(String(c)) >= 0; },
      item: function (i) { return cls[i] === undefined ? null : cls[i]; },
      toString: function () { return cls.join(' '); }
    };
  });
  ElementProto.getAttribute = function (n) { var v = __d_attr(this.__id, String(n)); return v == null ? null : v; };
  ElementProto.hasAttribute = function (n) { return __d_attr(this.__id, String(n)) != null; };
  ElementProto.matches = function (s) {
    var r = __d_matches(this.__id, String(s));
    if (r === undefined || r === null) throw bad(s);
    return r;
  };
  ElementProto.closest = function (s) {
    for (var n = this; n && n.nodeType === 1; n = n.parentNode) if (n.matches(s)) return n;
    return null;
  };

  // ---- Q33c: DOM writes.
  var defs = function (o, name, get, set) {
    Object.defineProperty(o, name, { get: get, set: set, configurable: true, enumerable: true });
  };
  var herr = function (msg, name) { return new DOMException(msg, name); };
  // QA14: XML Name check (approximate: ASCII start, then ASCII name chars or any non-ASCII).
  var validName = function (n) { return /^[A-Za-z_:\u00C0-\uFFFF][-A-Za-z0-9_:.\u00B7-\uFFFF]*$/.test(n); };
  var checkName = function (fn, n) {
    if (!validName(n)) throw herr("Failed to execute '" + fn + "': The string contains invalid characters.", 'InvalidCharacterError');
  };
  var toNode = function (n) {
    if (n && typeof n === 'object' && n.__id !== undefined) return n;
    return wrap(__d_create(3, String(n)));
  };
  defs(NodeProto, 'textContent', function () { return __d_text(this.__id); },
    function (v) { __d_settext(this.__id, v == null ? '' : String(v)); });
  defs(NodeProto, 'nodeValue', function () { return this.nodeType === 1 ? null : __d_text(this.__id); },
    function (v) { if (this.nodeType === 3 || this.nodeType === 8) __d_settext(this.__id, String(v)); });
  defs(NodeProto, 'data', function () { return this.nodeType === 3 || this.nodeType === 8 ? __d_text(this.__id) : undefined; },
    function (v) { if (this.nodeType === 3 || this.nodeType === 8) __d_settext(this.__id, String(v)); });
  // The getter-only CharacterData.prototype.data above shadows the Node one
  // (Acid3's `span.firstChild.data = score` was silently a no-op).
  defs(CharacterData.prototype, 'data', function () { return __d_text(this.__id); },
    function (v) { __d_settext(this.__id, String(v)); });
  // QY6a: CharacterData editing (one __d_settext = one characterData record).
  var cdRange = function (n, fn, off, cnt) {
    var d = __d_text(n.__id);
    off = off >>> 0;
    if (off > d.length) throw herr("Failed to execute '" + fn + "' on 'CharacterData': The offset " + off + ' is greater than the node\'s length (' + d.length + ').', 'IndexSizeError');
    return [d, off, Math.min(cnt >>> 0, d.length - off)];
  };
  var cdReplace = function (n, fn, off, cnt, ins) {
    var r = cdRange(n, fn, off, cnt);
    __d_settext(n.__id, r[0].slice(0, r[1]) + ins + r[0].slice(r[1] + r[2]));
  };
  CharacterData.prototype.appendData = function (s) { cdReplace(this, 'appendData', __d_text(this.__id).length, 0, String(s)); };
  CharacterData.prototype.insertData = function (o, s) { cdReplace(this, 'insertData', o, 0, String(s)); };
  CharacterData.prototype.deleteData = function (o, c) { cdReplace(this, 'deleteData', o, c, ''); };
  CharacterData.prototype.replaceData = function (o, c, s) { cdReplace(this, 'replaceData', o, c, String(s)); };
  CharacterData.prototype.substringData = function (o, c) { var r = cdRange(this, 'substringData', o, c); return r[0].substr(r[1], r[2]); };
  defs(ElementProto, 'innerText', function () { return __d_text(this.__id); },
    function (v) { __d_settext(this.__id, v == null ? '' : String(v)); });
  defs(ElementProto, 'innerHTML', function () { return __d_gethtml(this.__id, false); },
    function (v) { __d_sethtml(this.__id, v == null ? '' : String(v)); });
  // QY6a: the outerHTML setter parses in a `div` (the spec uses the parent as
  // context) and swaps the nodes in as one replaceChild.
  defs(ElementProto, 'outerHTML', function () { return __d_gethtml(this.__id, true); }, function (v) {
    var p = this.parentNode;
    if (!p) return;
    if (p.nodeType === 9) throw herr("Failed to set the 'outerHTML' property on 'Element': Cannot set outerHTML on a child of a Document.", 'NoModificationAllowedError');
    var tmp = wrap(__d_create(1, p.nodeType === 1 ? p.localName || 'div' : 'div')), f = wrap(__d_create(11, ''));
    __d_sethtml(tmp.__id, v == null ? '' : String(v));
    while (tmp.firstChild) f.appendChild(tmp.firstChild);
    p.replaceChild(f, this);
  });
  var reflect = function (prop, attr) {
    defs(ElementProto, prop, function () { var v = __d_attr(this.__id, attr); return v == null ? '' : v; },
      function (v) { __d_setattr(this.__id, attr, String(v)); });
  };
  reflect('id', 'id'); reflect('className', 'class'); reflect('title', 'title');
  reflect('lang', 'lang'); reflect('dir', 'dir');
  defs(ElementProto, 'hidden', function () { return __d_attr(this.__id, 'hidden') != null; },
    function (v) { if (v) __d_setattr(this.__id, 'hidden', ''); else __d_rmattr(this.__id, 'hidden'); });
  var reflectBool = function (prop, attr) {
    defs(ElementProto, prop, function () { return __d_attr(this.__id, attr) != null; },
      function (v) { if (v) __d_setattr(this.__id, attr, ''); else __d_rmattr(this.__id, attr); });
  };
  reflectBool('autofocus', 'autofocus'); reflectBool('disabled', 'disabled');
  reflectBool('required', 'required'); reflectBool('checked', 'checked');
  // QB5: focus/blur record a request the App applies in `js_sync`.
  HTMLElement.prototype.focus = function () { __d_focus(this.__id, true); };
  HTMLElement.prototype.blur = function () { __d_focus(this.__id, false); };
  // QB5: window.scroll/scrollTo/scrollBy(x, y | {top, left}); NaN = keep axis.
  // scrollX/scrollY follow optimistically (the App owns the real offset).
  var scrollArgs = function (a, b) {
    if (a !== null && typeof a === 'object') return [a.left === undefined ? NaN : Number(a.left), a.top === undefined ? NaN : Number(a.top)];
    return [Number(a), Number(b)];
  };
  g.scrollX = g.pageXOffset = 0; g.scrollY = g.pageYOffset = 0;
  g.scroll = g.scrollTo = function () {
    var a = scrollArgs(arguments[0], arguments[1]);
    __d_scroll(a[0], a[1], false);
    if (a[0] === a[0]) g.scrollX = g.pageXOffset = Math.max(0, a[0]);
    if (a[1] === a[1]) g.scrollY = g.pageYOffset = Math.max(0, a[1]);
  };
  g.scrollBy = function () {
    var a = scrollArgs(arguments[0], arguments[1]);
    __d_scroll(a[0] === a[0] ? a[0] : 0, a[1] === a[1] ? a[1] : 0, true);
    if (a[0] === a[0]) g.scrollX = g.pageXOffset = Math.max(0, g.scrollX + a[0]);
    if (a[1] === a[1]) g.scrollY = g.pageYOffset = Math.max(0, g.scrollY + a[1]);
  };
  ElementProto.setAttribute = function (n, v) { checkName('setAttribute', String(n)); __d_setattr(this.__id, String(n), String(v)); };
  ElementProto.removeAttribute = function (n) { __d_rmattr(this.__id, String(n)); };
  ElementProto.toggleAttribute = function (n, force) {
    var has = this.hasAttribute(n), want = force === undefined ? !has : !!force;
    if (want && !has) this.setAttribute(n, ''); else if (!want && has) this.removeAttribute(n);
    return want;
  };
  // QB4: HTMLDialogElement. `showModal` also sets an internal attribute so
  // the UA sheet can centre it (`:modal` isn't supported). `::backdrop` is
  // not rendered. `close` fires synchronously (browsers queue a task).
  var domErr = herr;
  var HTMLDialogElement = function HTMLDialogElement() {};
  HTMLDialogElement.prototype = Object.create(HTMLElement.prototype, { constructor: { value: HTMLDialogElement, writable: true, configurable: true } });
  g.HTMLDialogElement = HTMLDialogElement;
  var DP = HTMLDialogElement.prototype;
  defs(DP, 'open', function () { return __d_attr(this.__id, 'open') != null; },
    function (v) { if (v) __d_setattr(this.__id, 'open', ''); else __d_rmattr(this.__id, 'open'); });
  defs(DP, 'returnValue', function () { return this.__rv === undefined ? '' : this.__rv; },
    function (v) { this.__rv = String(v); });
  DP.show = function () {
    if (this.open) { if (__d_attr(this.__id, 'data-scratchy-modal') != null) throw domErr('InvalidStateError', "Failed to execute 'show' on 'HTMLDialogElement': The element is already open, and therefore cannot be opened non-modally."); return; }
    __d_setattr(this.__id, 'open', '');
  };
  DP.showModal = function () {
    if (this.open) {
      if (__d_attr(this.__id, 'data-scratchy-modal') != null) return;
      throw domErr('InvalidStateError', "Failed to execute 'showModal' on 'HTMLDialogElement': The element already has an 'open' attribute, and therefore cannot be opened modally.");
    }
    __d_setattr(this.__id, 'data-scratchy-modal', '');
    __d_setattr(this.__id, 'open', '');
  };
  DP.close = function (rv) {
    if (!this.open) return;
    if (rv !== undefined) this.__rv = String(rv);
    __d_rmattr(this.__id, 'open');
    __d_rmattr(this.__id, 'data-scratchy-modal');
    this.dispatchEvent(new Event('close'));
  };
  // QY3: HTMLCanvasElement without drawing: no context type is supported,
  // so `getContext` is null (as in a browser lacking it), `toDataURL` is
  // the empty data URL, `toBlob` calls back null.
  var HTMLCanvasElement = function HTMLCanvasElement() {};
  HTMLCanvasElement.prototype = Object.create(HTMLElement.prototype, { constructor: { value: HTMLCanvasElement, writable: true, configurable: true } });
  g.HTMLCanvasElement = HTMLCanvasElement;
  var CP = HTMLCanvasElement.prototype;
  var canvasDim = function (name, dflt) {
    defs(CP, name, function () {
      var v = parseInt(__d_attr(this.__id, name), 10);
      return v >= 0 ? v : dflt;
    }, function (v) { __d_setattr(this.__id, name, String(Math.max(0, Math.floor(Number(v)) || 0))); });
  };
  canvasDim('width', 300);
  canvasDim('height', 150);
  CP.getContext = function () { return null; };
  CP.toDataURL = function () { return 'data:,'; };
  CP.toBlob = function (cb) { if (typeof cb === 'function') setTimeout(function () { cb(null); }, 0); };
  var classes = function (el) { return el.className.split(/\s+/).filter(Boolean); };
  def(ElementProto, 'classList', function () {
    var el = this;
    var put = function (c) { el.className = c.join(' '); };
    var list = {
      contains: function (c) { return classes(el).indexOf(String(c)) >= 0; },
      add: function () {
        var c = classes(el);
        for (var i = 0; i < arguments.length; i++) if (c.indexOf(String(arguments[i])) < 0) c.push(String(arguments[i]));
        put(c);
      },
      remove: function () {
        var drop = Array.prototype.map.call(arguments, String);
        put(classes(el).filter(function (x) { return drop.indexOf(x) < 0; }));
      },
      toggle: function (c, force) {
        c = String(c);
        var has = this.contains(c), want = force === undefined ? !has : !!force;
        if (want && !has) this.add(c); else if (!want && has) this.remove(c);
        return want;
      },
      replace: function (a, b) {
        var c = classes(el), i = c.indexOf(String(a));
        if (i < 0) return false;
        c[i] = String(b); put(c); return true;
      },
      item: function (i) { var c = classes(el); return c[i] === undefined ? null : c[i]; },
      toString: function () { return el.className; },
      forEach: function (f, t) { classes(el).forEach(f, t); }
    };
    def(list, 'length', function () { return classes(el).length; });
    return list;
  });
  // style.<prop> reads/writes the inline `style` attribute.
  var kebab = function (p) { return p.replace(/[A-Z]/g, function (m) { return '-' + m.toLowerCase(); }); };
  var readStyle = function (el) {
    var out = [], s = __d_attr(el.__id, 'style') || '';
    s.split(';').forEach(function (d) {
      var i = d.indexOf(':');
      if (i > 0) out.push([d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim()]);
    });
    return out;
  };
  var writeStyle = function (el, decls) {
    if (!decls.length) { __d_rmattr(el.__id, 'style'); return; }
    __d_setattr(el.__id, 'style', decls.map(function (d) { return d[0] + ': ' + d[1]; }).join('; ') + ';');
  };
  var styleGet = function (el, p) {
    var d = readStyle(el).filter(function (x) { return x[0] === p; });
    return d.length ? d[d.length - 1][1] : '';
  };
  var styleSet = function (el, p, v) {
    var d = readStyle(el).filter(function (x) { return x[0] !== p; });
    v = v == null ? '' : String(v).trim();
    if (v) d.push([p, v]);
    writeStyle(el, d);
  };
  var styles = {};
  def(ElementProto, 'style', function () {
    var el = this;
    if (styles[el.__id]) return styles[el.__id];
    var api = {
      getPropertyValue: function (p) { return styleGet(el, kebab(String(p))); },
      setProperty: function (p, v) { styleSet(el, kebab(String(p)), v); },
      removeProperty: function (p) { var o = styleGet(el, String(p)); styleSet(el, String(p), ''); return o; },
      item: function (i) { var d = readStyle(el)[i]; return d ? d[0] : ''; }
    };
    var proxy = new Proxy(api, {
      get: function (t, p) {
        if (p in t) return t[p];
        if (p === 'cssText') return __d_attr(el.__id, 'style') || '';
        if (p === 'length') return readStyle(el).length;
        if (typeof p !== 'string') return undefined;
        return styleGet(el, p === 'cssFloat' ? 'float' : kebab(p));
      },
      set: function (t, p, v) {
        if (p === 'cssText') { if (v == null || v === '') __d_rmattr(el.__id, 'style'); else __d_setattr(el.__id, 'style', String(v)); }
        else if (typeof p === 'string') styleSet(el, p === 'cssFloat' ? 'float' : kebab(p), v);
        return true;
      }
    });
    return (styles[el.__id] = proxy);
  });
  // QA16: getComputedStyle (cascade only, no layout; see js_style.rs).
  var csProps = __d_csprops();
  var camel = function (p) { return p.replace(/-([a-z])/g, function (m, c) { return c.toUpperCase(); }); };
  g.getComputedStyle = function (el, pseudo) {
    if (!el || el.__id === undefined || el.nodeType !== 1) throw new TypeError("Failed to execute 'getComputedStyle' on 'Window': parameter 1 is not of type 'Element'.");
    var ps = pseudo == null ? '' : String(pseudo);
    var get = function (p) { return __d_cs(el.__id, ps, p); };
    var api = {
      getPropertyValue: function (p) { return get(kebab(String(p)).toLowerCase()); },
      getPropertyPriority: function () { return ''; },
      item: function (i) { return csProps[i] === undefined ? '' : csProps[i]; },
      setProperty: function () { throw domErr('NoModificationAllowedError', 'These styles are computed, and therefore read-only.'); },
      removeProperty: function () { throw domErr('NoModificationAllowedError', 'These styles are computed, and therefore read-only.'); },
      toString: function () { return '[object CSSStyleDeclaration]'; }
    };
    return new Proxy(api, {
      get: function (t, p) {
        if (p in t) return t[p];
        if (p === 'length') return csProps.length;
        if (p === 'cssText') return '';
        if (p === 'parentRule') return null;
        if (typeof p !== 'string') return undefined;
        if (/^\d+$/.test(p)) return csProps[+p];
        return get(p === 'cssFloat' ? 'float' : kebab(p));
      },
      has: function (t, p) { return p in t || (typeof p === 'string' && csProps.indexOf(kebab(p)) >= 0); },
      set: function () { return true; }
    });
  };
  var afterIns = null; // QA18b: set below (iframe load events)
  var rmNode = function (n) { __d_remove(n.__id); };
  NodeProto.appendChild = function (c) {
    if (!c || c.__id === undefined) throw new TypeError("Failed to execute 'appendChild': parameter 1 is not of type 'Node'.");
    if (!__d_insert(this.__id, c.__id, -1)) throw herr('The new child element contains the parent.', 'HierarchyRequestError');
    if (afterIns) afterIns(c);
    return c;
  };
  NodeProto.insertBefore = function (c, ref) {
    if (!c || c.__id === undefined) throw new TypeError("Failed to execute 'insertBefore': parameter 1 is not of type 'Node'.");
    if (ref && ref.parentNode !== this) throw herr('The node before which the new node is to be inserted is not a child of this node.', 'NotFoundError');
    if (!__d_insert(this.__id, c.__id, ref ? ref.__id : -1)) throw herr('The new child element contains the parent.', 'HierarchyRequestError');
    if (afterIns) afterIns(c);
    return c;
  };
  NodeProto.removeChild = function (c) {
    if (!c || c.__id === undefined) throw new TypeError("Failed to execute 'removeChild': parameter 1 is not of type 'Node'.");
    if (c.parentNode !== this) throw herr('The node to be removed is not a child of this node.', 'NotFoundError');
    rmNode(c);
    return c;
  };
  NodeProto.replaceChild = function (n, old) {
    if (!old || old.parentNode !== this) throw herr('The node to be replaced is not a child of this node.', 'NotFoundError');
    var own = moBegin();
    try {
      this.insertBefore(n, old);
      if (n !== old) rmNode(old);
    } finally { moEnd(own); }
    return old;
  };
  // A clone keeps the node document of the original (matters for template content).
  NodeProto.cloneNode = function (deep) {
    var c = wrap(__d_clone(this.__id, !!deep)), od = this.ownerDocument;
    if (od && od !== doc) owners[c.__id] = od;
    return c;
  };
  var many = function (args) { return Array.prototype.map.call(args, toNode); };
  ElementProto.append = function () {
    var s = this, own = moBegin();
    try { many(arguments).forEach(function (n) { s.appendChild(n); }); } finally { moEnd(own); }
  };
  ElementProto.prepend = function () {
    var s = this, first = this.firstChild, own = moBegin();
    try { many(arguments).forEach(function (n) { s.insertBefore(n, first); }); } finally { moEnd(own); }
  };
  ElementProto.replaceChildren = function () {
    var s = this, nodes = many(arguments), own = moBegin();
    try {
      while (s.firstChild) rmNode(s.firstChild);
      nodes.forEach(function (n) { s.appendChild(n); });
    } finally { moEnd(own); }
  };
  ElementProto.remove = function () { rmNode(this); };
  ElementProto.before = function () {
    var p = this.parentNode, s = this, own = moBegin();
    try { if (p) many(arguments).forEach(function (n) { p.insertBefore(n, s); }); } finally { moEnd(own); }
  };
  ElementProto.after = function () {
    var p = this.parentNode, nx = this.nextSibling, own = moBegin();
    try { if (p) many(arguments).forEach(function (n) { p.insertBefore(n, nx); }); } finally { moEnd(own); }
  };
  ElementProto.replaceWith = function () {
    var own = moBegin();
    try { this.before.apply(this, arguments); this.remove(); } finally { moEnd(own); }
  };
  // QY6f: Document.importNode / adoptNode on the main document (second documents
  // have their own in makeDoc). Imported clones are upgraded (if customElements
  // is in use) like any node created in the main document.
  doc.importNode = function (n, deep) {
    if (!n || n.__id === undefined) throw new TypeError("Failed to execute 'importNode' on 'Document': parameter 1 is not of type 'Node'.");
    if (n.nodeType === 9) throw herr("Failed to execute 'importNode' on 'Document': The node provided is a document.", 'NotSupportedError');
    var c = wrap(__d_clone(n.__id, !!deep));
    if (ceOn) ceScan(c.__id, true);
    return c;
  };
  doc.adoptNode = function (n) {
    if (!n || n.__id === undefined) throw new TypeError("Failed to execute 'adoptNode' on 'Document': parameter 1 is not of type 'Node'.");
    if (n.nodeType === 9) throw herr("Failed to execute 'adoptNode' on 'Document': The node provided is a document.", 'NotSupportedError');
    __d_remove(n.__id); delete owners[n.__id]; return n;
  };
  doc.createElement = function (t) { t = String(t); checkName('createElement', t); return wrap(__d_create(1, t)); };
  doc.createElementNS = function (ns, t) {
    ns = ns == null || ns === '' ? null : String(ns); t = String(t);
    checkName('createElementNS', t);
    var i = t.indexOf(':'), pre = i < 0 ? null : t.slice(0, i), loc = i < 0 ? t : t.slice(i + 1);
    if (i === 0 || loc === '' || loc.indexOf(':') >= 0 || (pre !== null && (ns === null ||
      (pre === 'xml' && ns !== 'http://www.w3.org/XML/1998/namespace'))) ||
      ((t === 'xmlns' || pre === 'xmlns') !== (ns === 'http://www.w3.org/2000/xmlns/')))
      throw herr("Failed to execute 'createElementNS': The namespace is invalid for the qualified name.", 'NamespaceError');
    return wrap(__d_create_ns(ns === null ? '' : ns, t));
  };
  doc.createTextNode = function (s) { return wrap(__d_create(3, String(s))); };
  doc.createComment = function (s) { return wrap(__d_create(8, String(s))); };
  doc.createDocumentFragment = function () { return wrap(__d_create(11, '')); };
  // QA17: DOM Traversal (NodeIterator, TreeWalker) in JS over the wrappers.
  var NF = g.NodeFilter, liveIters = [];
  var firstKid = function (n) { return n.childNodes[0] || null; };
  var lastKid = function (n) { var c = n.childNodes; return c[c.length - 1] || null; };
  var following = function (n, root) {
    var c = firstKid(n); if (c) return c;
    for (; n && n !== root; n = n.parentNode) { var s = n.nextSibling; if (s) return s; }
    return null;
  };
  var lastIncl = function (n) { for (var c; (c = lastKid(n)); ) n = c; return n; };
  var preceding = function (n, root) {
    if (n === root) return null;
    var s = n.previousSibling; if (s) return lastIncl(s);
    return n.parentNode;
  };
  var travArgs = function (fn, root, what, filter) {
    if (!root || root.__id === undefined) throw new TypeError("Failed to execute '" + fn + "' on 'Document': parameter 1 is not of type 'Node'.");
    return { root: root, what: what === undefined ? 0xFFFFFFFF : what >>> 0,
      filter: filter == null ? null : filter };
  };
  var runFilter = function (st, node) {
    if (st.active) throw herr('The filter is already running.', 'InvalidStateError');
    var bit = node.nodeType - 1;
    if (!((st.what >>> bit) & 1)) return NF.FILTER_SKIP;
    if (st.filter === null) return NF.FILTER_ACCEPT;
    st.active = true;
    try {
      var f = st.filter;
      var r = typeof f === 'function' ? f.call(undefined, node) : f.acceptNode(node);
      return r & 0xFFFF;
    } finally { st.active = false; }
  };
  var ST = '__trav';
  var hid = function (o, st) { Object.defineProperty(o, ST, { value: st }); return o; };
  var stOf = function (o) {
    var s = o && o[ST];
    if (!s) throw new TypeError('Illegal invocation');
    return s;
  };
  var NodeIterator = function NodeIterator() { throw new TypeError('Illegal constructor'); };
  var TreeWalker = function TreeWalker() { throw new TypeError('Illegal constructor'); };
  g.NodeIterator = NodeIterator; g.TreeWalker = TreeWalker;
  var acc = function (P, name, get, set) {
    Object.defineProperty(P, name, { get: function () { return get(stOf(this)); },
      set: set && function (v) { set(stOf(this), v); }, configurable: true, enumerable: true });
  };
  ['root', 'whatToShow', 'filter'].forEach(function (k) {
    var f = function (s) { return k === 'whatToShow' ? s.what : s[k]; };
    acc(NodeIterator.prototype, k, f); acc(TreeWalker.prototype, k, f);
  });
  acc(NodeIterator.prototype, 'referenceNode', function (s) { return s.ref; });
  acc(NodeIterator.prototype, 'pointerBeforeReferenceNode', function (s) { return s.before; });
  NodeIterator.prototype.detach = function () {
    stOf(this);
    var me = this;
    liveIters = liveIters.filter(function (w) { var o = w.deref ? w.deref() : w; return o && o !== me; });
  };
  var iterStep = function (it, next) {
    var s = stOf(it), node = s.ref, before = s.before;
    for (;;) {
      if (next) { if (!before) node = following(node, s.root); else before = false; }
      else { if (before) node = preceding(node, s.root); else before = true; }
      if (!node) return null;
      if (runFilter(s, node) === NF.FILTER_ACCEPT) break;
    }
    s.ref = node; s.before = before;
    return node;
  };
  NodeIterator.prototype.nextNode = function () { return iterStep(this, true); };
  NodeIterator.prototype.previousNode = function () { return iterStep(this, false); };
  doc.createNodeIterator = function (root, what, filter) {
    var a = travArgs('createNodeIterator', root, what, filter);
    var it = hid(Object.create(NodeIterator.prototype), { root: a.root, what: a.what, filter: a.filter, ref: a.root, before: true, active: false });
    liveIters.push(typeof WeakRef === 'function' ? new WeakRef(it) : it);
    return it;
  };
  // Spec "removing steps": fix up each live iterator before `n` leaves the tree.
  var preRemove = function (n) {
    liveIters.forEach(function (w) {
      var it = w.deref ? w.deref() : w; if (!it) return;
      var s = it[ST];
      if (n === s.root || !n.contains(s.ref)) return;
      if (s.before) {
        var nx = null;
        for (var t = n; t && t !== s.root && !nx; t = t.parentNode) nx = t.nextSibling;
        if (nx && s.root.contains(nx)) { s.ref = nx; return; }
        s.before = false;
      }
      var ps = n.previousSibling;
      s.ref = ps ? lastIncl(ps) : n.parentNode;
    });
  };
  rmNode = function (n) { if (liveIters.length) preRemove(n); __d_remove(n.__id); };
  var twChildren = function (tw, first) {
    var s = stOf(tw), node = first ? firstKid(s.cur) : lastKid(s.cur);
    while (node) {
      var r = runFilter(s, node);
      if (r === NF.FILTER_ACCEPT) { s.cur = node; return node; }
      if (r === NF.FILTER_SKIP) { var c = first ? firstKid(node) : lastKid(node); if (c) { node = c; continue; } }
      while (node) {
        var sb = first ? node.nextSibling : node.previousSibling;
        if (sb) { node = sb; break; }
        var p = node.parentNode;
        if (!p || p === s.root || p === s.cur) return null;
        node = p;
      }
    }
    return null;
  };
  var twSiblings = function (tw, next) {
    var s = stOf(tw), node = s.cur;
    if (node === s.root) return null;
    for (;;) {
      var sb = next ? node.nextSibling : node.previousSibling;
      while (sb) {
        node = sb;
        var r = runFilter(s, node);
        if (r === NF.FILTER_ACCEPT) { s.cur = node; return node; }
        sb = next ? firstKid(node) : lastKid(node);
        if (r === NF.FILTER_REJECT || !sb) sb = next ? node.nextSibling : node.previousSibling;
      }
      node = node.parentNode;
      if (!node || node === s.root) return null;
      if (runFilter(s, node) === NF.FILTER_ACCEPT) return null;
    }
  };
  var TW = TreeWalker.prototype;
  acc(TW, 'currentNode', function (s) { return s.cur; }, function (s, v) {
    if (!v || v.__id === undefined) throw new TypeError("Failed to set the 'currentNode' property on 'TreeWalker': The provided value is not of type 'Node'.");
    s.cur = v;
  });
  TW.parentNode = function () {
    var s = stOf(this), node = s.cur;
    while (node && node !== s.root) {
      node = node.parentNode;
      if (node && runFilter(s, node) === NF.FILTER_ACCEPT) { s.cur = node; return node; }
    }
    return null;
  };
  TW.firstChild = function () { return twChildren(this, true); };
  TW.lastChild = function () { return twChildren(this, false); };
  TW.nextSibling = function () { return twSiblings(this, true); };
  TW.previousSibling = function () { return twSiblings(this, false); };
  TW.previousNode = function () {
    var s = stOf(this), node = s.cur;
    while (node !== s.root) {
      var sb = node.previousSibling;
      while (sb) {
        node = sb;
        var r = runFilter(s, node);
        while (r !== NF.FILTER_REJECT && firstKid(node)) { node = lastKid(node); r = runFilter(s, node); }
        if (r === NF.FILTER_ACCEPT) { s.cur = node; return node; }
        sb = node.previousSibling;
      }
      if (node === s.root || !node.parentNode) return null;
      node = node.parentNode;
      if (runFilter(s, node) === NF.FILTER_ACCEPT) { s.cur = node; return node; }
    }
    return null;
  };
  TW.nextNode = function () {
    var s = stOf(this), node = s.cur, r = NF.FILTER_ACCEPT;
    for (;;) {
      while (r !== NF.FILTER_REJECT && firstKid(node)) {
        node = firstKid(node); r = runFilter(s, node);
        if (r === NF.FILTER_ACCEPT) { s.cur = node; return node; }
      }
      var sb = null, t = node;
      while (t) {
        if (t === s.root) return null;
        sb = t.nextSibling;
        if (sb) { node = sb; break; }
        t = t.parentNode;
      }
      if (!sb) return null;
      r = runFilter(s, node);
      if (r === NF.FILTER_ACCEPT) { s.cur = node; return node; }
    }
  };
  doc.createTreeWalker = function (root, what, filter) {
    var a = travArgs('createTreeWalker', root, what, filter);
    return hid(Object.create(TreeWalker.prototype), { root: a.root, what: a.what, filter: a.filter, cur: a.root, active: false });
  };
  def(doc, 'defaultView', function () { return g; });
  // QY4/QA15: legacy createEvent.
  doc.createEvent = function (iface) {
    var C = { event: Event, events: Event, htmlevents: Event, svgevents: Event, customevent: CustomEvent,
      uievent: UIEvent, uievents: UIEvent, mouseevent: MouseEvent, mouseevents: MouseEvent,
      keyboardevent: KeyboardEvent, focusevent: FocusEvent, errorevent: ErrorEvent }[String(iface).toLowerCase()];
    if (!C) throw herr('The provided event type (\'' + iface + '\') is invalid.', 'NotSupportedError');
    var e = new C('');
    return e;
  };
  doc.appendChild = NodeProto.appendChild;
  doc.insertBefore = NodeProto.insertBefore;
  doc.removeChild = NodeProto.removeChild;
  // Q33d: entry point for events born in Rust (clicks, keys, focus).
  Object.defineProperty(g, '__scratchy_dn', { value: function (id, type, init) {
    var t = id === -2 ? (doc.body || doc) : wrap(id);
    if (!t) return true;
    var C = { click: MouseEvent, mousedown: MouseEvent, mouseup: MouseEvent, keydown: KeyboardEvent,
      keyup: KeyboardEvent, mouseover: MouseEvent, mouseout: MouseEvent, focus: FocusEvent, blur: FocusEvent, focusin: FocusEvent, focusout: FocusEvent }[type] || Event;
    if (init && init.relatedTargetId !== undefined) init.relatedTarget = wrap(init.relatedTargetId);
    var ev = new C(type, init);
    ev.isTrusted = true;
    return __scratchy_dispatch(t, ev);
  } });
  // Q33d1: `el.click()`: a synthetic (untrusted) click, then the activation
  // behavior unless a handler cancelled it. Checkbox/radio state is the
  // `checked` attribute for now (Q33e), toggled before dispatch and
  // reverted on cancel, as browsers do.
  var clicking = false;
  HTMLElement.prototype.click = function () {
    var tag = this.localName;
    if (clicking || ((tag === 'input' || tag === 'button' || tag === 'select' || tag === 'textarea') && this.disabled)) return;
    clicking = true;
    try {
      var type = tag === 'input' ? this.type : '';
      var undo = null;
      if (type === 'checkbox') {
        var was = __d_attr(this.__id, 'checked') != null;
        this.checked = !was;
        undo = function (el) { el.checked = was; };
      } else if (type === 'radio' && __d_attr(this.__id, 'checked') == null) {
        var form = formOf(this), prevs = [], me = this, nm = ga(this, 'name');
        if (nm) doc.querySelectorAll('input').forEach(function (o) {
          if (o !== me && o.type === 'radio' && ga(o, 'name') === nm && formOf(o) === form && __d_attr(o.__id, 'checked') != null) prevs.push(o);
        });
        prevs.forEach(function (o) { o.checked = false; });
        this.checked = true;
        undo = function (el) { el.checked = false; prevs.forEach(function (o) { o.checked = true; }); };
      }
      var ev = new MouseEvent('click', { bubbles: true, cancelable: true });
      if (!__scratchy_dispatch(this, ev)) { if (undo) undo(this); return; }
      if (undo) {
        __scratchy_dispatch(this, new Event('input', { bubbles: true }));
        __scratchy_dispatch(this, new Event('change', { bubbles: true }));
      }
      var href = (tag === 'a' || tag === 'area') ? __d_attr(this.__id, 'href') : null;
      if (href != null) { __d_navigate(href); return; }
      var submit = (tag === 'button' && this.type === 'submit') || (tag === 'input' && (type === 'submit' || type === 'image'));
      if (submit) { var f = formOf(this); if (f) f.requestSubmit(this); }
    } finally { clicking = false; }
  };
  // QA20: table and form element interfaces, as reflections of attributes and
  // of the tree (live form state is Q33e). Collections are plain arrays with
  // `item`/`namedItem` and name/id properties added.
  tagProto.dialog = HTMLDialogElement.prototype;
  tagProto.canvas = HTMLCanvasElement.prototype;
  var mkProto = function (name, tags, base) {
    // QY6e: `class X extends HTMLButtonElement` (customized built-ins) reaches ceNew via super().
    var C = ({ [name]: function () { if (!new.target) throw new TypeError('Illegal constructor'); return ceNew(new.target); } })[name];
    C.prototype = Object.create((base || HTMLElement).prototype, { constructor: { value: C, writable: true, configurable: true } });
    g[name] = C;
    tags.forEach(function (t) { tagProto[t] = C.prototype; });
    return C.prototype;
  };
  var ga = function (el, n) { var v = __d_attr(el.__id, n); return v == null ? '' : v; };
  var rStr = function (P, prop, attr, dflt) {
    defs(P, prop, function () { var v = __d_attr(this.__id, attr); return v == null ? (dflt || '') : v; },
      function (v) { __d_setattr(this.__id, attr, String(v)); });
  };
  var rBool = function (P, prop, attr) {
    defs(P, prop, function () { return __d_attr(this.__id, attr) != null; },
      function (v) { if (v) __d_setattr(this.__id, attr, ''); else __d_rmattr(this.__id, attr); });
  };
  var rInt = function (P, prop, attr, dflt, min) {
    defs(P, prop, function () { var v = parseInt(__d_attr(this.__id, attr), 10); return v >= (min || 0) ? v : dflt; },
      function (v) { __d_setattr(this.__id, attr, String(Math.floor(Number(v)) || 0)); });
  };
  var coll = function (arr) {
    var named = function (n) {
      n = String(n);
      for (var i = 0; i < arr.length; i++) if (arr[i].id === n || ga(arr[i], 'name') === n) return arr[i];
      return null;
    };
    Object.defineProperty(arr, 'item', { value: function (i) { return arr[i] || null; }, configurable: true });
    Object.defineProperty(arr, 'namedItem', { value: named, configurable: true });
    arr.forEach(function (e) {
      [e.id, ga(e, 'name')].forEach(function (n) {
        if (n && !(n in arr) && !/^\d+$/.test(n)) Object.defineProperty(arr, n, { value: e, configurable: true });
      });
    });
    return arr;
  };
  var kidsOf = function (el, tags) { return el.children.filter(function (c) { return tags.indexOf(c.localName) >= 0; }); };
  var newEl = function (t) { return doc.createElement(t); };
  var idxErr = function (m) { return herr(m, 'IndexSizeError'); };
  def(doc, 'forms', function () { return coll(doc.querySelectorAll('form')); });
  def(doc, 'images', function () { return coll(doc.querySelectorAll('img')); });
  def(doc, 'links', function () { return coll(doc.querySelectorAll('a[href], area[href]')); });
  def(doc, 'anchors', function () { return coll(doc.querySelectorAll('a[name]')); });

  // ---- tables
  var secRows = function (sec) { return kidsOf(sec, ['tr']); };
  var tableRows = function (t) {
    var out = [], h = [], f = [];
    t.children.forEach(function (c) {
      var n = c.localName;
      if (n === 'thead') h = h.concat(secRows(c));
      else if (n === 'tfoot') f = f.concat(secRows(c));
      else if (n === 'tbody') out = out.concat(secRows(c));
      else if (n === 'tr') out.push(c);
    });
    return h.concat(out, f);
  };
  var insertRowIn = function (host, rows, idx, append) {
    idx = idx === undefined ? -1 : Math.floor(Number(idx)) || 0;
    if (idx < -1 || idx > rows.length) throw idxErr("Failed to execute 'insertRow': The provided index (" + idx + ') is outside the range [-1, ' + rows.length + '].');
    var tr = newEl('tr');
    if (idx === -1 || idx === rows.length) append(host, rows, tr);
    else rows[idx].parentNode.insertBefore(tr, rows[idx]);
    return tr;
  };
  var deleteRowIn = function (rows, idx) {
    idx = idx === undefined ? NaN : Math.floor(Number(idx));
    if (idx === -1) idx = rows.length - 1;
    if (!(idx >= -1 && idx < rows.length)) throw idxErr("Failed to execute 'deleteRow': The provided index (" + idx + ') is outside the range [-1, ' + (rows.length - 1) + '].');
    if (idx >= 0) rows[idx].parentNode.removeChild(rows[idx]);
  };
  var TP = mkProto('HTMLTableElement', ['table']);
  def(TP, 'rows', function () { return coll(tableRows(this)); });
  def(TP, 'tBodies', function () { return coll(kidsOf(this, ['tbody'])); });
  def(TP, 'tHead', function () { return kidsOf(this, ['thead'])[0] || null; });
  def(TP, 'tFoot', function () { return kidsOf(this, ['tfoot'])[0] || null; });
  def(TP, 'caption', function () { return kidsOf(this, ['caption'])[0] || null; });
  TP.createTHead = function () {
    var h = this.tHead; if (h) return h;
    h = newEl('thead');
    var after = this.children.filter(function (c) { return c.localName === 'caption' || c.localName === 'colgroup'; }).pop();
    this.insertBefore(h, after ? after.nextSibling : this.firstChild);
    return h;
  };
  TP.createTFoot = function () { var f = this.tFoot; if (f) return f; f = newEl('tfoot'); this.appendChild(f); return f; };
  TP.createCaption = function () { var c = this.caption; if (c) return c; c = newEl('caption'); this.insertBefore(c, this.firstChild); return c; };
  TP.createTBody = function () {
    var b = newEl('tbody'), last = kidsOf(this, ['tbody']).pop();
    this.insertBefore(b, last ? last.nextSibling : null);
    return b;
  };
  TP.deleteTHead = function () { var h = this.tHead; if (h) h.parentNode.removeChild(h); };
  TP.deleteTFoot = function () { var f = this.tFoot; if (f) f.parentNode.removeChild(f); };
  TP.deleteCaption = function () { var c = this.caption; if (c) c.parentNode.removeChild(c); };
  TP.insertRow = function (idx) {
    return insertRowIn(this, tableRows(this), idx, function (t, rows, tr) {
      if (!rows.length) {
        var bs = kidsOf(t, ['tbody']), b = bs[bs.length - 1];
        if (!b) { b = newEl('tbody'); t.appendChild(b); }
        b.appendChild(tr);
      } else rows[rows.length - 1].parentNode.appendChild(tr);
    });
  };
  TP.deleteRow = function (idx) { deleteRowIn(tableRows(this), idx); };
  ['summary', 'align', 'bgColor', 'border', 'cellPadding', 'cellSpacing', 'frame', 'rules', 'width'].forEach(function (p) { rStr(TP, p, p.toLowerCase()); });
  var SP = mkProto('HTMLTableSectionElement', ['tbody', 'thead', 'tfoot']);
  def(SP, 'rows', function () { return coll(secRows(this)); });
  SP.insertRow = function (idx) {
    return insertRowIn(this, secRows(this), idx, function (s, rows, tr) { s.appendChild(tr); });
  };
  SP.deleteRow = function (idx) { deleteRowIn(secRows(this), idx); };
  var RP = mkProto('HTMLTableRowElement', ['tr']);
  var cellsOf = function (r) { return kidsOf(r, ['td', 'th']); };
  def(RP, 'cells', function () { return coll(cellsOf(this)); });
  def(RP, 'rowIndex', function () {
    var t = this.parentNode;
    while (t && t.localName !== 'table') t = t.parentNode;
    return t ? tableRows(t).indexOf(this) : -1;
  });
  def(RP, 'sectionRowIndex', function () {
    var p = this.parentNode;
    return p ? kidsOf(p, ['tr']).indexOf(this) : -1;
  });
  RP.insertCell = function (idx) {
    var cells = cellsOf(this);
    idx = idx === undefined ? -1 : Math.floor(Number(idx)) || 0;
    if (idx < -1 || idx > cells.length) throw idxErr("Failed to execute 'insertCell': The provided index (" + idx + ') is outside the range [-1, ' + cells.length + '].');
    var td = newEl('td');
    if (idx === -1 || idx === cells.length) this.appendChild(td); else this.insertBefore(td, cells[idx]);
    return td;
  };
  RP.deleteCell = function (idx) {
    var cells = cellsOf(this);
    idx = idx === undefined ? NaN : Math.floor(Number(idx));
    if (idx === -1) idx = cells.length - 1;
    if (!(idx >= -1 && idx < cells.length)) throw idxErr("Failed to execute 'deleteCell': The provided index (" + idx + ') is outside the range [-1, ' + (cells.length - 1) + '].');
    if (idx >= 0) this.removeChild(cells[idx]);
  };
  var CEP = mkProto('HTMLTableCellElement', ['td', 'th']);
  def(CEP, 'cellIndex', function () { var p = this.parentNode; return p ? cellsOf(p).indexOf(this) : -1; });
  rInt(CEP, 'colSpan', 'colspan', 1, 1);
  rInt(CEP, 'rowSpan', 'rowspan', 1, 0);
  ['abbr', 'axis', 'headers', 'scope', 'align', 'vAlign', 'bgColor', 'width', 'height'].forEach(function (p) { rStr(CEP, p, p.toLowerCase()); });
  rBool(CEP, 'noWrap', 'nowrap');
  var CAP = mkProto('HTMLTableCaptionElement', ['caption']); rStr(CAP, 'align', 'align');
  var COLP = mkProto('HTMLTableColElement', ['col', 'colgroup']); rInt(COLP, 'span', 'span', 1, 1);

  // ---- forms
  var CTL = ['input', 'select', 'textarea', 'button', 'fieldset', 'object', 'output'];
  var formOf = function (el) {
    var fid = __d_attr(el.__id, 'form');
    if (fid != null) { var f = doc.getElementById(fid); return f && f.localName === 'form' ? f : null; }
    for (var n = el.parentNode; n; n = n.parentNode) if (n.localName === 'form') return n;
    return null;
  };
  var FP = mkProto('HTMLFormElement', ['form']);
  var formControls = function (f) {
    var inside = f.querySelectorAll(CTL.join(','));
    var ext = f.id ? doc.querySelectorAll(CTL.join(',')).filter(function (c) { return inside.indexOf(c) < 0; }) : [];
    return inside.concat(ext).filter(function (c) {
      return !(c.localName === 'input' && c.type === 'image') && formOf(c) === f;
    });
  };
  def(FP, 'elements', function () { return coll(formControls(this)); });
  def(FP, 'length', function () { return formControls(this).length; });
  // Q33d1: there is no form submission yet (Q30), so the default action of
  // a submit is a no-op once `submit` has not been cancelled. `submit()`
  // fires no event; `requestSubmit()` and a submit-button click do.
  FP.submit = function () {};
  FP.requestSubmit = function (submitter) {
    if (submitter !== undefined && submitter !== null && (!submitter || submitter.form !== this)) {
      throw herr('The specified element is not a submit button of this form.', 'NotFoundError');
    }
    var ev = new Event('submit', { bubbles: true, cancelable: true });
    ev.submitter = submitter || null;
    __scratchy_dispatch(this, ev);
  };
  FP.reset = function () {};
  ['name', 'action', 'method', 'target', 'enctype', 'encoding', 'acceptCharset', 'autocomplete'].forEach(function (p) {
    rStr(FP, p, p === 'acceptCharset' ? 'accept-charset' : p === 'encoding' ? 'enctype' : p);
  });
  var formProp = function (P) { def(P, 'form', function () { return formOf(this); }); };
  var IP = mkProto('HTMLInputElement', ['input']);
  var INPUT_TYPES = ['hidden', 'text', 'search', 'tel', 'url', 'email', 'password', 'date', 'month', 'week', 'time', 'datetime-local', 'number', 'range', 'color', 'checkbox', 'radio', 'file', 'submit', 'image', 'reset', 'button'];
  defs(IP, 'type', function () {
    var t = ga(this, 'type').toLowerCase();
    return INPUT_TYPES.indexOf(t) >= 0 ? t : 'text';
  }, function (v) { __d_setattr(this.__id, 'type', String(v)); });
  formProp(IP);
  ['name', 'alt', 'src', 'accept', 'placeholder', 'min', 'max', 'step', 'pattern', 'autocomplete', 'align', 'useMap'].forEach(function (p) { rStr(IP, p, p.toLowerCase()); });
  // The value property is a dirty value held on the wrapper (the attribute
  // is the default); wiring it to live widget state is Q33e.
  defs(IP, 'value', function () { return this.__v !== undefined ? this.__v : ga(this, 'value'); },
    function (v) { this.__v = String(v); });
  defs(IP, 'defaultValue', function () { return ga(this, 'value'); }, function (v) { __d_setattr(this.__id, 'value', String(v)); });
  defs(IP, 'defaultChecked', function () { return __d_attr(this.__id, 'checked') != null; },
    function (v) { if (v) __d_setattr(this.__id, 'checked', ''); else __d_rmattr(this.__id, 'checked'); });
  ['readOnly', 'multiple', 'indeterminate'].forEach(function (p) { rBool(IP, p, p.toLowerCase()); });
  rInt(IP, 'size', 'size', 20, 1); rInt(IP, 'maxLength', 'maxlength', -1, 0);
  var BP = mkProto('HTMLButtonElement', ['button']);
  defs(BP, 'type', function () {
    var t = ga(this, 'type').toLowerCase();
    return t === 'reset' || t === 'button' ? t : 'submit';
  }, function (v) { __d_setattr(this.__id, 'type', String(v)); });
  formProp(BP);
  ['name', 'value'].forEach(function (p) { rStr(BP, p, p); });
  var TAP = mkProto('HTMLTextAreaElement', ['textarea']);
  formProp(TAP);
  def(TAP, 'type', function () { return 'textarea'; });
  ['name', 'placeholder', 'wrap'].forEach(function (p) { rStr(TAP, p, p); });
  defs(TAP, 'value', function () { return __d_text(this.__id); }, function (v) { __d_settext(this.__id, String(v)); });
  defs(TAP, 'defaultValue', function () { return __d_text(this.__id); }, function (v) { __d_settext(this.__id, String(v)); });
  rInt(TAP, 'rows', 'rows', 2, 1); rInt(TAP, 'cols', 'cols', 20, 1); rBool(TAP, 'readOnly', 'readonly');
  var optionsOf = function (s) {
    var out = [];
    s.children.forEach(function (c) {
      if (c.localName === 'option') out.push(c);
      else if (c.localName === 'optgroup') out = out.concat(kidsOf(c, ['option']));
    });
    return out;
  };
  var SEP = mkProto('HTMLSelectElement', ['select']);
  formProp(SEP);
  def(SEP, 'options', function () { return coll(optionsOf(this)); });
  def(SEP, 'length', function () { return optionsOf(this).length; });
  def(SEP, 'type', function () { return this.multiple ? 'select-multiple' : 'select-one'; });
  rBool(SEP, 'multiple', 'multiple'); rStr(SEP, 'name', 'name');
  rInt(SEP, 'size', 'size', 0, 1);
  defs(SEP, 'selectedIndex', function () {
    var o = optionsOf(this);
    for (var i = 0; i < o.length; i++) if (__d_attr(o[i].__id, 'selected') != null) return i;
    if (!this.multiple && this.size <= 1) for (i = 0; i < o.length; i++) if (!o[i].disabled) return i;
    return -1;
  }, function (v) {
    var o = optionsOf(this); v = Math.floor(Number(v));
    o.forEach(function (x, i) { if (i === v) __d_setattr(x.__id, 'selected', ''); else __d_rmattr(x.__id, 'selected'); });
  });
  defs(SEP, 'value', function () { var o = optionsOf(this)[this.selectedIndex]; return o ? o.value : ''; },
    function (v) {
      v = String(v);
      var o = optionsOf(this), hit = -1;
      o.forEach(function (x, i) { if (hit < 0 && x.value === v) hit = i; });
      this.selectedIndex = hit;
    });
  def(SEP, 'selectedOptions', function () { return optionsOf(this).filter(function (o) { return o.selected; }); });
  SEP.add = function (opt, before) {
    if (typeof before === 'number') before = optionsOf(this)[before] || null;
    var host = before && before.parentNode ? before.parentNode : this;
    host.insertBefore(opt, before || null);
  };
  SEP.item = function (i) { return optionsOf(this)[i] || null; };
  SEP.namedItem = function (n) { return coll(optionsOf(this)).namedItem(n); };
  var elRemove = ElementProto.remove;
  SEP.remove = function (i) {
    if (arguments.length === 0) { if (elRemove) elRemove.call(this); return; }
    var o = optionsOf(this)[i]; if (o) o.parentNode.removeChild(o);
  };
  var OP = mkProto('HTMLOptionElement', ['option']);
  formProp(OP);
  defs(OP, 'value', function () { var v = __d_attr(this.__id, 'value'); return v == null ? this.text : v; },
    function (v) { __d_setattr(this.__id, 'value', String(v)); });
  defs(OP, 'text', function () { return __d_text(this.__id).replace(/[ \t\n\f\r]+/g, ' ').replace(/^ | $/g, ''); },
    function (v) { __d_settext(this.__id, String(v)); });
  defs(OP, 'label', function () { var v = __d_attr(this.__id, 'label'); return v == null ? this.text : v; },
    function (v) { __d_setattr(this.__id, 'label', String(v)); });
  rBool(OP, 'selected', 'selected'); rBool(OP, 'defaultSelected', 'selected');
  def(OP, 'index', function () {
    for (var s = this.parentNode; s; s = s.parentNode) if (s.localName === 'select') return optionsOf(s).indexOf(this);
    return 0;
  });
  var OGP = mkProto('HTMLOptGroupElement', ['optgroup']); rStr(OGP, 'label', 'label');
  var LP = mkProto('HTMLLabelElement', ['label']);
  formProp(LP);
  defs(LP, 'htmlFor', function () { return ga(this, 'for'); }, function (v) { __d_setattr(this.__id, 'for', String(v)); });
  def(LP, 'control', function () {
    var f = __d_attr(this.__id, 'for');
    if (f != null) return doc.getElementById(f);
    return this.querySelector('input,select,textarea,button,output');
  });
  var FSP = mkProto('HTMLFieldSetElement', ['fieldset']); formProp(FSP); rStr(FSP, 'name', 'name');
  def(FSP, 'type', function () { return 'fieldset'; });
  def(FSP, 'elements', function () { return coll(this.querySelectorAll(CTL.join(','))); });
  var OUP = mkProto('HTMLOutputElement', ['output']); formProp(OUP); rStr(OUP, 'name', 'name');
  def(OUP, 'type', function () { return 'output'; });
  ['input', 'select', 'textarea', 'button', 'fieldset', 'output'].forEach(function (t) {
    rBool(tagProto[t], 'disabled', 'disabled');
  });
  // ---- image maps
  var MP = mkProto('HTMLMapElement', ['map']);
  rStr(MP, 'name', 'name');
  def(MP, 'areas', function () { return coll(this.querySelectorAll('area')); });
  def(MP, 'images', function () {
    var n = ga(this, 'name');
    return n ? coll(doc.querySelectorAll('img,object')).filter(function (i) { return ga(i, 'usemap') === '#' + n; }) : [];
  });
  var METP = mkProto('HTMLMetaElement', ['meta']);
  ['name', 'content', 'scheme'].forEach(function (p) { rStr(METP, p, p); });
  rStr(METP, 'httpEquiv', 'http-equiv');
  var ARP = mkProto('HTMLAreaElement', ['area']);
  ['alt', 'coords', 'shape', 'href', 'target', 'rel'].forEach(function (p) { rStr(ARP, p, p); });
  cache[0] = doc;
  Object.defineProperty(doc, '__id', { value: 0 });
  Object.defineProperty(doc, 'nodeType', { value: 9 });
  Object.defineProperty(doc, 'nodeName', { value: '#document' });
  queries(doc);
  doc.hasChildNodes = NodeProto.hasChildNodes;
  doc.contains = NodeProto.contains;
  def(doc, 'childNodes', Object.getOwnPropertyDescriptor(NodeProto, 'childNodes').get);
  doc.getElementById = function (id) { return wrap(__d_byid(String(id), this.__id)); };
  def(doc, 'documentElement', function () { return wrap(__d_special(0, this.__id)); });
  // QSD1: Document as ParentNode (same accessors as Element/DocumentFragment).
  ['children', 'firstElementChild', 'lastElementChild', 'childElementCount'].forEach(function (k) {
    Object.defineProperty(doc, k, Object.getOwnPropertyDescriptor(ElementProto, k));
  });
  doc.getElementsByName = function (n) { return wrapAll(q(this.__id, '[name="' + String(n).replace(/["\\]/g, '\\$&') + '"]', false)); };
  // QSD1: `document.domain`. The getter is the page's hostname ('' for an
  // opaque origin). The setter follows the spec's shape: the same value or a
  // dot-boundary suffix of it that is not a single label (no public suffix
  // list: "com" is refused, "co.uk" is not) is accepted and remembered,
  // anything else throws SecurityError. Origins are not actually relaxed.
  var domainOv = null;
  var isIpHost = function (h) { return /^[0-9.]+$/.test(h) || h.indexOf(':') >= 0 || h.charAt(0) === '['; };
  Object.defineProperty(doc, 'domain', { configurable: true, enumerable: true,
    get: function () { return domainOv !== null ? domainOv : (g.location && g.location.hostname) || ''; },
    set: function (v) {
      var cur = (g.location && g.location.hostname) || '', val = String(v).toLowerCase().replace(/\.$/, '');
      if (cur === '') throw herr("Failed to set the 'domain' property on 'Document': Assignment is only allowed for documents with a tuple origin.", 'SecurityError');
      var eff = this.domain;
      var ok = val === eff || val === cur ||
        (!isIpHost(cur) && val.indexOf('.') > 0 && (cur === val || cur.slice(-val.length - 1) === '.' + val));
      if (!ok) throw herr("Failed to set the 'domain' property on 'Document': '" + String(v) + "' is not a suffix of '" + cur + "'.", 'SecurityError');
      domainOv = val;
    } });
  // QB8: `document.currentScript`: the executing classic <script>, else null.
  var curScript = null;
  var wlast = -1;
  Object.defineProperty(g, '__scratchy_cs', { value: function (id) { curScript = id < 0 ? null : wrap(id); wlast = id; } });
  // QA13: only while a classic script element runs: insert after it (and
  // after what earlier writes of the same script inserted). Otherwise the
  // spec would wipe the document; we log and ignore. <script> in the
  // written markup is inserted but not executed.
  function dwrite(nl) {
    return function () {
      if (!curScript || wlast < 0) { console.warn('document.' + (nl ? 'writeln' : 'write') + ' after page load ignored'); return; }
      var s = Array.prototype.join.call(arguments, '') + (nl ? '\n' : '');
      var r = __d_write(wlast, s);
      if (r >= 0) wlast = r;
    };
  }
  doc.write = dwrite(false);
  doc.writeln = dwrite(true);
  doc.open = function () { return doc; };
  doc.close = function () {};
  def(doc, 'currentScript', function () { return curScript; });
  def(doc, 'head', function () { return wrap(__d_special(1, this.__id)); });
  def(doc, 'body', function () { return wrap(__d_special(2, this.__id)); });
  // QA18a: second documents. `Object.create(doc)` shares every method and
  // getter (they are keyed by `this.__id`); what differs is overridden.
  var DocumentType = g.DocumentType = function DocumentType() { throw new TypeError('Illegal constructor'); };
  DocumentType.prototype = Object.create(Node.prototype, { constructor: { value: DocumentType, writable: true, configurable: true } });
  ['name', 'publicId', 'systemId'].forEach(function (k, i) {
    def(DocumentType.prototype, k, function () { return __d_doctype(this.__id)[i]; });
  });
  var DOMImpl = g.DOMImplementation = function DOMImplementation() { throw new TypeError('Illegal constructor'); };
  var makeDoc = function (html, ct) {
    var id = __d_newdoc();
    var d = Object.create(doc);
    cache[id] = d;
    Object.defineProperty(d, '__id', { value: id });
    var own = { readyState: 'complete', URL: 'about:blank', documentURI: 'about:blank', baseURI: 'about:blank',
      contentType: ct, compatMode: html ? 'CSS1Compat' : 'CSS1Compat', characterSet: 'UTF-8', title: undefined };
    Object.keys(own).forEach(function (k) {
      if (own[k] !== undefined) Object.defineProperty(d, k, { value: own[k], writable: true, configurable: true, enumerable: true });
    });
    ['location', 'defaultView', 'currentScript'].forEach(function (k) { def(d, k, function () { return null; }); });
    var mk = function (f) { return function () { var n = f.apply(d, arguments); owners[n.__id] = d; return n; }; };
    ['createElement', 'createElementNS', 'createTextNode', 'createComment', 'createDocumentFragment'].forEach(function (k) {
      d[k] = mk(doc[k]);
    });
    d.write = d.writeln = function () { console.warn('document.write on a created document ignored'); };
    d.importNode = function (n, deep) {
      if (!n || n.__id === undefined || n.nodeType === 9) throw herr("Failed to execute 'importNode' on 'Document': The node provided is a document.", 'NotSupportedError');
      var c = wrap(__d_clone(n.__id, !!deep)); owners[c.__id] = d; return c;
    };
    d.adoptNode = function (n) {
      if (!n || n.__id === undefined) throw new TypeError("Failed to execute 'adoptNode' on 'Document': parameter 1 is not of type 'Node'.");
      if (n.nodeType === 9) throw herr("Failed to execute 'adoptNode' on 'Document': The node provided is a document.", 'NotSupportedError');
      __d_remove(n.__id); owners[n.__id] = d; return n;
    };
    def(d, 'doctype', function () {
      var c = this.childNodes; for (var i = 0; i < c.length; i++) if (c[i].nodeType === 10) return c[i];
      return null;
    });
    d.getElementsByName = function (n) { return wrapAll(q(this.__id, '[name="' + String(n).replace(/["\\]/g, '\\$&') + '"]', false)); };
    Object.defineProperty(d, 'title', { configurable: true, enumerable: true,
      get: function () {
        var t = this.getElementsByTagName('title')[0];
        return t ? t.textContent.replace(/[ \t\n\f\r]+/g, ' ').replace(/^ | $/g, '') : '';
      },
      set: function (v) {
        var t = this.getElementsByTagName('title')[0];
        if (!t) {
          var h = this.head; if (!h) return;
          t = this.createElement('title'); h.appendChild(t);
        }
        t.textContent = String(v);
      } });
    return d;
  };
  var impl = Object.create(DOMImpl.prototype);
  DOMImpl.prototype.createDocumentType = function (name, pub, sys) {
    if (arguments.length < 3) throw new TypeError("Failed to execute 'createDocumentType' on 'DOMImplementation': 3 arguments required, but only " + arguments.length + " present.");
    name = String(name); checkName('createDocumentType', name);
    var n = wrap(__d_create_doctype(name, String(pub), String(sys)));
    owners[n.__id] = doc; return n;
  };
  DOMImpl.prototype.createHTMLDocument = function (title) {
    var d = makeDoc(true, 'text/html');
    var dt = wrap(__d_create_doctype('html', '', '')); d.appendChild(dt);
    var html = d.createElement('html'); d.appendChild(html);
    var head = d.createElement('head'); html.appendChild(head);
    if (title !== undefined) { var t = d.createElement('title'); t.textContent = String(title); head.appendChild(t); }
    html.appendChild(d.createElement('body'));
    return d;
  };
  DOMImpl.prototype.createDocument = function (ns, qname, dt) {
    ns = ns == null || ns === '' ? null : String(ns);
    qname = qname == null ? '' : String(qname);
    var xml = ns === 'http://www.w3.org/1999/xhtml' ? 'application/xhtml+xml' : ns === 'http://www.w3.org/2000/svg' ? 'image/svg+xml' : 'application/xml';
    var d = makeDoc(false, xml);
    if (dt != null) { if (!dt || dt.nodeType !== 10) throw new TypeError("Failed to execute 'createDocument' on 'DOMImplementation': parameter 3 is not of type 'DocumentType'."); d.appendChild(dt); owners[dt.__id] = d; }
    if (qname !== '') d.insertBefore(d.createElementNS(ns, qname), null);
    return d;
  };
  DOMImpl.prototype.hasFeature = function () { return true; };
  // QA18b: `iframe.contentDocument`/`contentWindow` for iframes with an
  // empty, about:blank or data: src: a fresh empty second document (made
  // lazily, kept per iframe element), a minimal window-like object, and an
  // asynchronous `load` event after insertion. Network iframes give null.
  var IFP = mkProto('HTMLIFrameElement', ['iframe']);
  var frames = {};
  var inPage = function (n) { for (; n && n.parentNode; n = n.parentNode); return !!n && n.__id === 0; };
  var frameSrcOk = function (el) {
    var s = (__d_attr(el.__id, 'src') || '').replace(/^\s+|\s+$/g, '');
    return s === '' || /^about:blank/i.test(s) || /^data:/i.test(s) || __d_iframe_page(el.__id).length > 0;
  };
  var frameOf = function (el) {
    if (!inPage(el) || !frameSrcOk(el)) return null;
    var f = frames[el.__id];
    if (f) return f;
    var d = makeDoc(true, 'text/html');
    var pg = __d_iframe_page(el.__id), href = 'about:blank';
    if (pg.length) {
      // QA18b1: pre-fetched same-origin document; scripts in it never run.
      __d_load_html(d.__id, pg[1]); href = pg[0];
      ['URL', 'documentURI', 'baseURI'].forEach(function (k) { Object.defineProperty(d, k, { value: href, writable: true, configurable: true, enumerable: true }); });
    } else {
      var html = d.createElement('html'); d.appendChild(html);
      html.appendChild(d.createElement('head')); html.appendChild(d.createElement('body'));
    }
    var win = Object.create(null);
    var props = { document: d, window: win, self: win, parent: g, top: g, frameElement: el, name: '', length: 0,
      location: { href: href, toString: function () { return href; } } };
    Object.keys(props).forEach(function (k) { Object.defineProperty(win, k, { value: props[k], writable: true, configurable: true, enumerable: true }); });
    win.getComputedStyle = function (e, p) { return g.getComputedStyle(e, p); };
    Object.defineProperty(d, 'defaultView', { get: function () { return win; }, configurable: true, enumerable: true });
    return (frames[el.__id] = { doc: d, win: win });
  };
  def(IFP, 'contentDocument', function () { var f = frameOf(this); return f ? f.doc : null; });
  def(IFP, 'contentWindow', function () { var f = frameOf(this); return f ? f.win : null; });
  IFP.getSVGDocument = function () { return this.contentDocument; };
  rStr(IFP, 'src', 'src'); rStr(IFP, 'name', 'name'); rStr(IFP, 'width', 'width'); rStr(IFP, 'height', 'height');
  afterIns = function (c) {
    if (c.nodeType !== 1 || __d_tag(c.__id).toLowerCase() !== 'iframe' || !inPage(c)) return;
    g.setTimeout(function () { if (inPage(c) && frameSrcOk(c)) c.dispatchEvent(new Event('load')); }, 0);
  };
  def(doc, 'implementation', function () { return impl; });
  // ---- QY11: reflected URL attributes, hyperlink utils, dataset,
  // attributes, getAttributeNames, iterable classList. Pure JS over the
  // attribute natives; the base is `document.baseURI` (honours <base href>).
  var docBase = function () {
    var b = doc.querySelector('base[href]'), loc = g.location.href;
    if (b) { try { return new URL(__d_attr(b.__id, 'href'), loc).href; } catch (e) {} }
    return loc;
  };
  Object.defineProperty(doc, 'baseURI', { get: docBase, configurable: true, enumerable: true });
  def(ElementProto, 'baseURI', function () { return doc.baseURI; });
  var absUrl = function (el, attr) {
    var v = __d_attr(el.__id, attr);
    if (v == null) return '';
    var t = v.replace(/^[ \t\n\f\r]+|[ \t\n\f\r]+$/g, '');
    if (t === '' && attr !== 'href' && attr !== 'action') return '';
    try { return new URL(t, doc.baseURI).href; } catch (e) { return v; }
  };
  var rUrl = function (P, prop, attr) {
    defs(P, prop, function () { return absUrl(this, attr); },
      function (v) { __d_setattr(this.__id, attr, String(v)); });
  };
  var rStrs = function (P, list) { list.forEach(function (p) { rStr(P, p, p.toLowerCase()); }); };
  var rBools = function (P, list) { list.forEach(function (p) { rBool(P, p, p.toLowerCase()); }); };
  var SCP = mkProto('HTMLScriptElement', ['script']);
  rUrl(SCP, 'src', 'src'); rStrs(SCP, ['type', 'charset', 'nonce', 'crossOrigin', 'integrity', 'referrerPolicy']); rBools(SCP, ['async', 'defer', 'noModule']);
  defs(SCP, 'text', function () { return this.textContent; }, function (v) { this.textContent = String(v); });
  var IMP = mkProto('HTMLImageElement', ['img']);
  rUrl(IMP, 'src', 'src'); rStrs(IMP, ['alt', 'srcset', 'sizes', 'crossOrigin', 'useMap', 'decoding', 'loading', 'referrerPolicy', 'name', 'align']);
  rBool(IMP, 'isMap', 'ismap');
  def(IMP, 'currentSrc', function () { return absUrl(this, 'src'); });
  var LKP = mkProto('HTMLLinkElement', ['link']);
  rUrl(LKP, 'href', 'href'); rStrs(LKP, ['rel', 'type', 'media', 'as', 'hreflang', 'charset', 'nonce', 'crossOrigin', 'integrity', 'referrerPolicy', 'sizes']);
  rBool(LKP, 'disabled', 'disabled');
  var BAP = mkProto('HTMLBaseElement', ['base']);
  rUrl(BAP, 'href', 'href'); rStr(BAP, 'target', 'target');
  var SOP = mkProto('HTMLSourceElement', ['source']);
  rUrl(SOP, 'src', 'src'); rStrs(SOP, ['type', 'srcset', 'sizes', 'media']);
  var MEP = mkProto('HTMLMediaElement', []);
  rUrl(MEP, 'src', 'src'); rStr(MEP, 'crossOrigin', 'crossorigin'); rStr(MEP, 'preload', 'preload');
  rBools(MEP, ['autoplay', 'loop', 'controls', 'muted']);
  def(MEP, 'currentSrc', function () { return absUrl(this, 'src'); });
  var VIP = mkProto('HTMLVideoElement', ['video'], g.HTMLMediaElement);
  rUrl(VIP, 'poster', 'poster'); rBool(VIP, 'playsInline', 'playsinline');
  mkProto('HTMLAudioElement', ['audio'], g.HTMLMediaElement);
  var EMP = mkProto('HTMLEmbedElement', ['embed']);
  rUrl(EMP, 'src', 'src'); rStrs(EMP, ['type', 'width', 'height']);
  var OBP = mkProto('HTMLObjectElement', ['object']);
  rUrl(OBP, 'data', 'data'); rStrs(OBP, ['type', 'name', 'width', 'height']);
  var TRP = mkProto('HTMLTrackElement', ['track']);
  rUrl(TRP, 'src', 'src'); rStrs(TRP, ['kind', 'srclang', 'label']);
  var ANP = mkProto('HTMLAnchorElement', ['a']);
  rStrs(ANP, ['target', 'download', 'rel', 'rev', 'hreflang', 'type', 'referrerPolicy', 'ping', 'coords', 'name', 'shape', 'charset']);
  defs(ANP, 'text', function () { return this.textContent; }, function (v) { this.textContent = String(v); });
  rUrl(FP, 'action', 'action');
  rUrl(IFP, 'src', 'src');
  rUrl(IP, 'src', 'src');
  // HTMLHyperlinkElementUtils on a and area.
  var linkURL = function (el) {
    if (__d_attr(el.__id, 'href') == null) return null;
    try { return new URL(__d_attr(el.__id, 'href'), doc.baseURI); } catch (e) { return null; }
  };
  var hl = function (P) {
    rUrl(P, 'href', 'href');
    ['protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash', 'username', 'password'].forEach(function (k) {
      defs(P, k, function () {
        var u = linkURL(this);
        return u ? u[k] : '';
      }, function (v) {
        var u = linkURL(this);
        if (!u) return;
        u[k] = String(v);
        __d_setattr(this.__id, 'href', u.href);
      });
    });
    def(P, 'origin', function () { var u = linkURL(this); return u ? u.origin : ''; });
    P.toString = function () { return this.href; };
  };
  hl(ANP); hl(ARP);
  // classList: a DOMTokenList keyed on the element.
  var DOMTokenList = g.DOMTokenList = function DOMTokenList() { throw new TypeError('Illegal constructor'); };
  var DTP = DOMTokenList.prototype;
  var tokcheck = function (fn, t) {
    t = String(t);
    if (t === '') throw herr("Failed to execute '" + fn + "' on 'DOMTokenList': The token provided must not be empty.", 'SyntaxError');
    if (/[ \t\n\f\r]/.test(t)) throw herr("Failed to execute '" + fn + "' on 'DOMTokenList': The token provided ('" + t + "') contains HTML space characters, which are not valid in tokens.", 'InvalidCharacterError');
    return t;
  };
  var toks = function (l) { return classes(l._el); };
  var putToks = function (l, c) { l._el.className = c.join(' '); };
  DTP.contains = function (c) { return toks(this).indexOf(String(c)) >= 0; };
  DTP.add = function () {
    var c = toks(this), a = Array.prototype.map.call(arguments, function (x) { return tokcheck('add', x); });
    a.forEach(function (t) { if (c.indexOf(t) < 0) c.push(t); });
    putToks(this, c);
  };
  DTP.remove = function () {
    var a = Array.prototype.map.call(arguments, function (x) { return tokcheck('remove', x); });
    putToks(this, toks(this).filter(function (x) { return a.indexOf(x) < 0; }));
  };
  DTP.toggle = function (c, force) {
    c = tokcheck('toggle', c);
    var has = this.contains(c), want = force === undefined ? !has : !!force;
    if (want && !has) this.add(c); else if (!want && has) this.remove(c);
    return want;
  };
  DTP.replace = function (a, b) {
    a = tokcheck('replace', a); b = tokcheck('replace', b);
    var c = toks(this), i = c.indexOf(a);
    if (i < 0) return false;
    if (c.indexOf(b) >= 0 && c.indexOf(b) !== i) c.splice(i, 1); else c[i] = b;
    putToks(this, c); return true;
  };
  DTP.item = function (i) { var c = toks(this); return c[i] === undefined ? null : c[i]; };
  DTP.supports = function () { return true; };
  DTP.toString = function () { return this._el.className; };
  DTP.forEach = function (f, t) { toks(this).forEach(function (x, i) { f.call(t, x, i, this); }, this); };
  DTP.keys = function () { return toks(this).keys(); };
  DTP.values = function () { return toks(this).values(); };
  DTP.entries = function () { return toks(this).entries(); };
  DTP[Symbol.iterator] = DTP.values;
  def(DTP, 'length', function () { return toks(this).length; });
  defs(DTP, 'value', function () { return this._el.className; }, function (v) { this._el.className = String(v); });
  Object.defineProperty(DTP, Symbol.toStringTag, { value: 'DOMTokenList', configurable: true });
  var tlists = {};
  var isIdx = function (k) { return typeof k === 'string' && /^(0|[1-9][0-9]*)$/.test(k); };
  defs(ElementProto, 'classList', function () {
    if (tlists[this.__id]) return tlists[this.__id];
    var l = Object.create(DTP);
    Object.defineProperty(l, '_el', { value: this });
    var p = new Proxy(l, {
      get: function (t, k) { return isIdx(k) ? toks(t)[+k] : Reflect.get(t, k, t); },
      has: function (t, k) { return isIdx(k) ? +k < toks(t).length : k in t; }
    });
    return (tlists[this.__id] = p);
  }, function (v) { this.classList.value = v; });
  // dataset: live DOMStringMap over data-* attributes.
  var dsName = function (attr) { return attr.slice(5).replace(/-([a-z])/g, function (m, c) { return c.toUpperCase(); }); };
  var dsAttr = function (p) { return 'data-' + p.replace(/[A-Z]/g, function (m) { return '-' + m.toLowerCase(); }); };
  var dsKeys = function (el) {
    return __d_attr_names(el.__id).filter(function (n) { return n.indexOf('data-') === 0 && !/[A-Z]/.test(n); }).map(dsName);
  };
  var dsets = {};
  var DOMStringMap = g.DOMStringMap = function DOMStringMap() { throw new TypeError('Illegal constructor'); };
  def(HTMLElement.prototype, 'dataset', function () {
    var el = this;
    if (dsets[el.__id]) return dsets[el.__id];
    var p = new Proxy(Object.create(DOMStringMap.prototype), {
      get: function (t, k) {
        if (typeof k !== 'string') return Reflect.get(t, k);
        var v = __d_attr(el.__id, dsAttr(k)); return v == null ? undefined : v;
      },
      set: function (t, k, v) {
        if (typeof k !== 'string') return false;
        if (/-[a-z]/.test(k)) throw herr("Failed to set the '" + k + "' property on 'DOMStringMap': '" + k + "' is not a valid property name.", 'SyntaxError');
        __d_setattr(el.__id, dsAttr(k), String(v)); return true;
      },
      has: function (t, k) { return typeof k === 'string' ? __d_attr(el.__id, dsAttr(k)) != null : k in t; },
      deleteProperty: function (t, k) { if (typeof k === 'string') __d_rmattr(el.__id, dsAttr(k)); return true; },
      ownKeys: function () { return dsKeys(el); },
      getOwnPropertyDescriptor: function (t, k) {
        if (typeof k !== 'string') return undefined;
        var v = __d_attr(el.__id, dsAttr(k));
        return v == null ? undefined : { value: v, writable: true, enumerable: true, configurable: true };
      },
      defineProperty: function (t, k, d) { if (typeof k === 'string' && 'value' in d) __d_setattr(el.__id, dsAttr(k), String(d.value)); return true; }
    });
    return (dsets[el.__id] = p);
  });
  // attributes: a live NamedNodeMap-like Proxy; Attr objects are lightweight
  // views whose `value` reads/writes through (not nodes in the tree).
  var AttrP = g.Attr.prototype;
  var mkAttr = function (el, name) {
    var a = Object.create(AttrP);
    Object.defineProperty(a, '_el', { value: el });
    Object.defineProperty(a, '_n', { value: name });
    return a;
  };
  def(AttrP, 'name', function () { return this._n; });
  def(AttrP, 'localName', function () { return this._n; });
  def(AttrP, 'nodeName', function () { return this._n; });
  def(AttrP, 'nodeType', function () { return 2; });
  def(AttrP, 'namespaceURI', function () { return null; });
  def(AttrP, 'prefix', function () { return null; });
  def(AttrP, 'specified', function () { return true; });
  def(AttrP, 'ownerElement', function () { return this._el; });
  defs(AttrP, 'value', function () { var v = __d_attr(this._el.__id, this._n); return v == null ? '' : v; },
    function (v) { __d_setattr(this._el.__id, this._n, String(v)); });
  defs(AttrP, 'nodeValue', function () { return this.value; }, function (v) { this.value = v; });
  defs(AttrP, 'textContent', function () { return this.value; }, function (v) { this.value = v; });
  var NNM = g.NamedNodeMap = function NamedNodeMap() { throw new TypeError('Illegal constructor'); };
  var NP = NNM.prototype;
  var attrList = function (el) { return __d_attr_names(el.__id).map(function (n) { return mkAttr(el, n); }); };
  NP.item = function (i) { return attrList(this._el)[i] || null; };
  NP.getNamedItem = function (n) {
    n = String(n).toLowerCase();
    return __d_attr(this._el.__id, n) == null ? null : mkAttr(this._el, n);
  };
  NP.getNamedItemNS = function (ns, n) { return this.getNamedItem(n); };
  NP.removeNamedItem = function (n) {
    var a = this.getNamedItem(n);
    if (!a) throw herr("Failed to execute 'removeNamedItem' on 'NamedNodeMap': No item with name '" + n + "' was found.", 'NotFoundError');
    __d_rmattr(this._el.__id, a.name); return a;
  };
  NP.setNamedItem = function (a) { __d_setattr(this._el.__id, a.name, a.value); return null; };
  NP[Symbol.iterator] = function () { return attrList(this._el)[Symbol.iterator](); };
  def(NP, 'length', function () { return __d_attr_names(this._el.__id).length; });
  Object.defineProperty(NP, Symbol.toStringTag, { value: 'NamedNodeMap', configurable: true });
  var amaps = {};
  def(ElementProto, 'attributes', function () {
    if (amaps[this.__id]) return amaps[this.__id];
    var m = Object.create(NP);
    Object.defineProperty(m, '_el', { value: this, configurable: true });
    var el = this;
    return (amaps[this.__id] = new Proxy(m, {
      get: function (t, k) {
        if (typeof k === 'string') {
          if (isIdx(k)) return t.item(+k) || undefined;
          if (!(k in t)) { var a = t.getNamedItem(k); if (a) return a; }
        }
        return Reflect.get(t, k, t);
      },
      has: function (t, k) {
        if (typeof k !== 'string') return k in t;
        return isIdx(k) ? +k < t.length : (k in t || __d_attr(el.__id, k.toLowerCase()) != null);
      },
      ownKeys: function () { var n = __d_attr_names(el.__id); return n.map(function (x, i) { return String(i); }).concat(n); },
      getOwnPropertyDescriptor: function (t, k) {
        if (typeof k !== 'string') return undefined;
        var ix = isIdx(k), v = ix ? t.item(+k) : t.getNamedItem(k);
        return v ? { value: v, writable: false, enumerable: ix, configurable: true } : undefined;
      }
    }));
  });
  ElementProto.getAttributeNames = function () { return __d_attr_names(this.__id); };
  ElementProto.hasAttributes = function () { return __d_attr_names(this.__id).length > 0; };
  ElementProto.getAttributeNode = function (n) { return this.attributes.getNamedItem(n); };
  // QY12: interface objects, table driven. Tags without an interface above get an
  // inert HTML<Name>Element (not callable, prototype under HTMLElement); never
  // replaces one defined earlier.
  var illegal = function (name) { return ({ [name]: function () { throw new TypeError('Illegal constructor'); } })[name]; };
  [['HTMLDivElement', 'div'], ['HTMLSpanElement', 'span'], ['HTMLAnchorElement', 'a'], ['HTMLImageElement', 'img'],
    ['HTMLScriptElement', 'script'], ['HTMLStyleElement', 'style'], ['HTMLLinkElement', 'link'],
    ['HTMLBodyElement', 'body'], ['HTMLHeadElement', 'head'], ['HTMLHtmlElement', 'html'],
    ['HTMLParagraphElement', 'p'], ['HTMLUListElement', 'ul'], ['HTMLOListElement', 'ol'], ['HTMLLIElement', 'li'],
    ['HTMLTitleElement', 'title'], ['HTMLBaseElement', 'base'], ['HTMLHRElement', 'hr'], ['HTMLBRElement', 'br'],
    ['HTMLPictureElement', 'picture'], ['HTMLSlotElement', 'slot'], ['HTMLTemplateElement', 'template'],
    ['HTMLDListElement', 'dl'], ['HTMLQuoteElement', 'blockquote', 'q'], ['HTMLPreElement', 'pre'],
    ['HTMLModElement', 'ins', 'del'], ['HTMLTimeElement', 'time'], ['HTMLDataElement', 'data'],
    ['HTMLEmbedElement', 'embed'], ['HTMLObjectElement', 'object'], ['HTMLParamElement', 'param'],
    ['HTMLMenuElement', 'menu'], ['HTMLDetailsElement', 'details'], ['HTMLProgressElement', 'progress'],
    ['HTMLMeterElement', 'meter'], ['HTMLLegendElement', 'legend'], ['HTMLDataListElement', 'datalist'],
    ['HTMLHeadingElement', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6'], ['HTMLUnknownElement']].forEach(function (r) {
    if (!g[r[0]]) mkProto(r[0], r.slice(1));
  });
  var HTMLUnknownElement = g.HTMLUnknownElement;
  // QY6f: `template.content` is a DocumentFragment owned by ONE inert document (created
  // lazily): it never renders, never upgrades custom elements, never runs scripts.
  var inertDoc = null;
  def(g.HTMLTemplateElement.prototype, 'content', function () {
    var id = __d_template_content(this.__id);
    if (id < 0) return undefined;
    if (!inertDoc) inertDoc = makeDoc(true, 'text/html');
    owners[id] = inertDoc;
    return wrap(id);
  });
  // Media: inert placeholders only (AGENTS.md "Off limits"): the methods log a
  // `not implemented` error and do nothing; no state, no events.
  var ens = function (n, tags, base) { return g[n] ? g[n].prototype : mkProto(n, tags, base); };
  var MEP = ens('HTMLMediaElement', []);
  var nimp = function (what) { __scratchy_err('scratchy: js: not implemented: ' + what); };
  ['load', 'pause', 'addTextTrack'].forEach(function (m) { MEP[m] = function () { nimp('HTMLMediaElement.' + m); }; });
  MEP.play = function () {
    nimp('HTMLMediaElement.play');
    return Promise.reject(new DOMException('HTMLMediaElement.play is not implemented', 'NotSupportedError'));
  };
  MEP.canPlayType = function () { return ''; };
  ens('HTMLVideoElement', ['video'], g.HTMLMediaElement); ens('HTMLAudioElement', ['audio'], g.HTMLMediaElement);
  ens('HTMLSourceElement', ['source']); ens('HTMLTrackElement', ['track']);
  // SVG: by local name in the SVG namespace (see `wrap`).
  [['SVGElement', 'Element'], ['SVGGraphicsElement', 'SVGElement'], ['SVGGeometryElement', 'SVGGraphicsElement'],
    ['SVGSVGElement', 'SVGGraphicsElement', 'svg'], ['SVGGElement', 'SVGGraphicsElement', 'g'],
    ['SVGUseElement', 'SVGGraphicsElement', 'use'], ['SVGImageElement', 'SVGGraphicsElement', 'image'],
    ['SVGTextContentElement', 'SVGGraphicsElement'], ['SVGTextPositioningElement', 'SVGTextContentElement'],
    ['SVGTextElement', 'SVGTextPositioningElement', 'text'], ['SVGTSpanElement', 'SVGTextPositioningElement', 'tspan'],
    ['SVGPathElement', 'SVGGeometryElement', 'path'], ['SVGRectElement', 'SVGGeometryElement', 'rect'],
    ['SVGCircleElement', 'SVGGeometryElement', 'circle'], ['SVGEllipseElement', 'SVGGeometryElement', 'ellipse'],
    ['SVGLineElement', 'SVGGeometryElement', 'line'], ['SVGPolygonElement', 'SVGGeometryElement', 'polygon'],
    ['SVGPolylineElement', 'SVGGeometryElement', 'polyline'], ['SVGDefsElement', 'SVGElement', 'defs'],
    ['SVGSymbolElement', 'SVGElement', 'symbol'], ['SVGStyleElement', 'SVGElement', 'style'],
    ['SVGScriptElement', 'SVGElement', 'script'], ['SVGGradientElement', 'SVGElement'],
    ['SVGLinearGradientElement', 'SVGGradientElement', 'linearGradient'],
    ['SVGRadialGradientElement', 'SVGGradientElement', 'radialGradient'], ['SVGStopElement', 'SVGElement', 'stop'],
    ['SVGClipPathElement', 'SVGElement', 'clipPath'], ['SVGMaskElement', 'SVGElement', 'mask'],
    ['SVGForeignObjectElement', 'SVGGraphicsElement', 'foreignObject']].forEach(function (r) {
    var P = r[1] === 'Element' ? Element : g[r[1]], C = illegal(r[0]);
    C.prototype = Object.create(P.prototype, { constructor: { value: C, writable: true, configurable: true } });
    g[r[0]] = C;
    if (r[2]) svgProto[r[2]] = C.prototype;
  });
  var SVGElement = g.SVGElement;
  // CDATASection / ProcessingInstruction exist as interfaces (the parser never produces them).
  [['CDATASection', Text], ['ProcessingInstruction', CharacterData]].forEach(function (r) {
    var C = illegal(r[0]);
    C.prototype = Object.create(r[1].prototype, { constructor: { value: C, writable: true, configurable: true } });
    g[r[0]] = C;
  });
  // Range is a stub until QA19: the interface object exists, using it throws.
  g.Range = function Range() { throw new DOMException('Range is not implemented', 'NotSupportedError'); };
  g.Range.prototype = {};
  Object.defineProperty(g.Range.prototype, 'constructor', { value: g.Range, writable: true, configurable: true });
  ['Selection', 'NamedNodeMap', 'DOMTokenList', 'StyleSheet', 'CSSStyleSheet', 'CSSRule', 'CSSStyleDeclaration',
    'Screen', 'Storage', 'ShadowRoot'].forEach(function (n) { if (!g[n]) g[n] = illegal(n); });
  // ParentNode on DocumentFragment.
  ['children', 'firstElementChild', 'lastElementChild', 'childElementCount'].forEach(function (k) {
    Object.defineProperty(DocumentFragment.prototype, k, Object.getOwnPropertyDescriptor(ElementProto, k));
  });
  ['append', 'prepend', 'replaceChildren'].forEach(function (k) {
    if (ElementProto[k]) DocumentFragment.prototype[k] = ElementProto[k];
  });

  // ---- QY6e: Custom Elements (window.customElements). Everything is JS over the
  // wrappers; the DOM mutation natives (`__d_insert/remove/sethtml/settext/
  // setattr/rmattr/write`, which every API path funnels through) are rebound
  // to hooked versions at the FIRST `define()`, so a page that never uses
  // custom elements runs the bare natives (`__ce_stats.hooks` stays 0).
  // Upgrade = run the class constructor on the existing wrapper: HTMLElement's
  // constructor (`ceNew`, reached through `super()`) returns the element
  // being upgraded (per-definition construction stack). Reactions run
  // synchronously after the API call (a microtask-free simplification of
  // the CEReactions stack). Only the main document (id 0) is ever upgraded.
  // Cuts: adoptedCallback, `:defined`, no shadow DOM (attachShadow throws).
  var ceOn = false, ceByName = Object.create(null), ceByCtor = new Map(), ceWhen = Object.create(null);
  var ceState = Object.create(null), ceConnFlag = Object.create(null), ceNConn = 0, ceSel = '';
  var ceStats = { hooks: 0 };
  Object.defineProperty(g, '__ce_stats', { value: ceStats });
  var ceReport = function (e, what) {
    __scratchy_err('scratchy: js: uncaught exception in custom element ' + what + ': ' + (e && e.stack ? e.stack : String(e)));
  };
  var ceCall = function (el, fn, args) {
    if (typeof fn !== 'function') return;
    try { fn.apply(el, args); } catch (e) { ceReport(e, fn.name || 'callback'); }
  };
  var ceIsConn = function (id) { var p; while ((p = __d_parent(id)) >= 0) id = p; return id === 0; };
  var ceDefFor = function (id) {
    var raw = __d_tag(id);
    if (raw !== raw.toUpperCase()) return null; // not an HTML-namespace element
    var tg = raw.toLowerCase(), is = __d_attr(id, 'is'), d;
    if (is != null && (d = ceByName[is]) && d.ext === tg) return d;
    d = ceByName[tg];
    return d && !d.ext ? d : null;
  };
  var ceSetConn = function (id) {
    ceConnFlag[id] = 1; ceNConn++;
    var d = ceState[id]; ceCall(cache[id], d.connected, []);
  };
  var ceCollect = function (id, self) {
    var out = [];
    if (self && ceConnFlag[id]) out.push(id);
    var l = __d_query(id, ceSel, false);
    for (var i = 0; i < l.length; i++) if (ceConnFlag[l[i]]) out.push(l[i]);
    return out;
  };
  var ceFire = function (ids) {
    for (var i = 0; i < ids.length; i++) {
      var id = ids[i];
      if (!ceConnFlag[id]) continue;
      ceConnFlag[id] = 0; ceNConn--;
      ceCall(cache[id], ceState[id].disconnected, []);
    }
  };
  var ceUpgrade = function (id, d, conn) {
    var el = wrap(id), ok = false;
    d.stack.push({ el: el, done: false });
    try {
      var r = Reflect.construct(d.ctor, [], d.ctor);
      if (r !== el) throw new TypeError('The custom element constructor did not produce the element being upgraded.');
      ok = true;
    } catch (e) { ceReport(e, d.name + ' constructor'); }
    d.stack.pop();
    if (!ok) { ceState[id] = -1; return; }
    ceState[id] = d;
    if (d.observed.length) {
      var names = __d_attr_names(id);
      for (var i = 0; i < names.length; i++) {
        var n = names[i].toLowerCase();
        if (d.observed.indexOf(n) >= 0) ceCall(el, d.changed, [n, null, __d_attr(id, n)]);
      }
    }
    if (conn) ceSetConn(id);
  };
  var ceProcess = function (id, conn) {
    var st = ceState[id];
    if (!st) { var d = ceDefFor(id); if (d) ceUpgrade(id, d, conn); return; }
    if (st !== -1 && conn && !ceConnFlag[id]) ceSetConn(id);
  };
  // Upgrade (and fire connectedCallback for) candidates under `id`, tree order.
  // `force` also upgrades inside a disconnected tree (customElements.upgrade, clones).
  var ceScan = function (id, force) {
    var conn = ceIsConn(id);
    if (!conn && !force) return;
    var l = __d_query(id, ceSel, false);
    if (__d_kind(id) === 1) l.unshift(id);
    for (var i = 0; i < l.length; i++) ceProcess(l[i], conn);
  };
  // The ONE shared mutation layer (QY6e custom elements + QY6a MutationObserver):
  // installed by the first `define()` or `observe()`, rebinding the natives to
  // hooked versions. Each hook does the CE work when `ceOn` and the record
  // work when observers exist (`moN > 0`); neither runs otherwise.
  var hookOn = false;
  var hookInstall = function () {
    if (hookOn) return;
    hookOn = true;
    var oIns = __d_insert, oRem = __d_remove, oHtml = __d_sethtml, oTxt = __d_settext, oSet = __d_setattr, oRm = __d_rmattr, oW = __d_write;
    __d_insert = function (p, c, b) {
      ceStats.hooks++;
      var mo = moN > 0, frag = __d_kind(c) === 11, kids = frag ? __d_children(c, false) : [c];
      var oldP = mo && !frag ? __d_parent(c) : -1, oPrev = -1, oNext = -1;
      if (oldP >= 0) { oPrev = __d_sibling(c, -1, false); oNext = __d_sibling(c, 1, false); }
      var pre = ceNConn && __d_parent(c) >= 0 ? ceCollect(c, true) : null; // a move = disconnect + connect
      var ok = oIns(p, c, b);
      if (ok && mo) {
        if (oldP >= 0) moChild(oldP, [], [c], oPrev, oNext);
        if (kids.length) moChild(p, kids, [], __d_sibling(kids[0], -1, false), __d_sibling(kids[kids.length - 1], 1, false));
      }
      if (pre) ceFire(pre);
      if (ceOn) for (var i = 0; i < kids.length; i++) ceScan(kids[i], false);
      return ok;
    };
    __d_remove = function (id) {
      ceStats.hooks++;
      var p = moN > 0 ? __d_parent(id) : -1, pv = -1, nx = -1;
      if (p >= 0) { pv = __d_sibling(id, -1, false); nx = __d_sibling(id, 1, false); }
      var pre = ceNConn ? ceCollect(id, true) : null;
      oRem(id);
      if (p >= 0) moChild(p, [], [id], pv, nx);
      if (pre) ceFire(pre);
    };
    __d_sethtml = function (id, html) {
      ceStats.hooks++;
      var old = moN > 0 ? __d_children(id, false) : null;
      var pre = ceNConn ? ceCollect(id, false) : null;
      oHtml(id, html);
      if (old) { var nw = __d_children(id, false); if (old.length || nw.length) moChild(id, nw, old, -1, -1); }
      if (pre) ceFire(pre);
      if (ceOn) ceScan(id, false);
    };
    __d_settext = function (id, t) {
      ceStats.hooks++;
      var k = __d_kind(id), mo = moN > 0, old = null, oldTxt = null;
      if (mo) { if (k === 3) oldTxt = __d_text(id); else if (k !== 8) old = __d_children(id, false); }
      var pre = ceNConn && (k === 1 || k === 11) ? ceCollect(id, false) : null;
      oTxt(id, t);
      if (old) { var nw = __d_children(id, false); if (old.length || nw.length) moChild(id, nw, old, -1, -1); }
      else if (oldTxt !== null) moChar(id, oldTxt);
      if (pre) ceFire(pre);
    };
    __d_write = function (a, h) {
      ceStats.hooks++;
      var p = __d_parent(a), nx = moN > 0 && p >= 0 ? __d_sibling(a, 1, false) : -2;
      var r = oW(a, h);
      if (nx !== -2) {
        var added = [], c = __d_sibling(a, 1, false);
        while (c >= 0 && c !== nx) { added.push(c); c = __d_sibling(c, 1, false); }
        if (added.length) moChild(p, added, [], a, nx);
      }
      if (ceOn && p >= 0) ceScan(p, false);
      return r;
    };
    __d_setattr = function (id, n, v) {
      ceStats.hooks++;
      var d = ceState[id], ce = d && d !== -1 && d.observed.length, mo = moN > 0, ln = '', old = null;
      if (ce || mo) { ln = String(n).toLowerCase(); old = __d_attr(id, ln); if (old == null) old = null; }
      oSet(id, n, v);
      if (mo) moAttr(id, ln, old);
      if (ce && d.observed.indexOf(ln) >= 0) ceCall(cache[id], d.changed, [ln, old, __d_attr(id, ln)]);
    };
    __d_rmattr = function (id, n) {
      ceStats.hooks++;
      var d = ceState[id], ce = d && d !== -1 && d.observed.length, mo = moN > 0, ln = '', old = null;
      if (ce || mo) { ln = String(n).toLowerCase(); old = __d_attr(id, ln); }
      oRm(id, n);
      if (old != null) {
        if (mo) moAttr(id, ln, old);
        if (ce && d.observed.indexOf(ln) >= 0) ceCall(cache[id], d.changed, [ln, old, null]);
      }
    };
  };
  var ceInstall = function () {
    hookInstall();
    var oCE = doc.createElement, oCENS = doc.createElementNS, oClone = NodeProto.cloneNode;
    var created = function (el, opts, self) {
      if (self && self.__id > 0) return el; // second documents: never upgraded
      var id = el.__id, raw = __d_tag(id), tg = raw.toLowerCase(), is = opts && opts.is != null ? String(opts.is) : null, d = null;
      if (raw !== raw.toUpperCase()) return el;
      if (is !== null && ceByName[is] && ceByName[is].ext === tg) { d = ceByName[is]; oSet(id, 'is', is); }
      else if (ceByName[tg] && !ceByName[tg].ext) d = ceByName[tg];
      if (d) ceUpgrade(id, d, false);
      return el;
    };
    doc.createElement = function (t, o) { return created(oCE.call(this, t), o, this); };
    doc.createElementNS = function (ns, t, o) {
      var el = oCENS.call(this, ns, t);
      return ns === 'http://www.w3.org/1999/xhtml' ? created(el, o, this) : el;
    };
    NodeProto.cloneNode = function (deep) {
      var c = oClone.call(this, deep);
      if (c && c.__id !== undefined && c.ownerDocument === doc) ceScan(c.__id, true); // inert docs: never upgraded
      return c;
    };
  };
  var NAME_RE = /^[a-z][-._0-9a-z·À-ÖØ-öø-ͽͿ-῿‌‍‿⁀⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�]*$/;
  var RESERVED = ['annotation-xml', 'color-profile', 'font-face', 'font-face-src', 'font-face-uri', 'font-face-format', 'font-face-name', 'missing-glyph'];
  var ceValidName = function (n) { return NAME_RE.test(n) && n.indexOf('-') > 0 && RESERVED.indexOf(n) < 0; };
  // The HTMLElement constructor (js.rs forwards `new.target` here).
  ceNew = function (nt) {
    var d = ceByCtor.get(nt);
    if (!d) throw new TypeError('Illegal constructor');
    var top = d.stack[d.stack.length - 1];
    if (top) {
      if (top.done) throw new TypeError('This instance is already constructed');
      top.done = true;
      Object.setPrototypeOf(top.el, nt.prototype);
      return top.el;
    }
    var id = __d_create(1, d.ext || d.name), el = wrap(id);
    if (d.ext) __d_setattr(id, 'is', d.name);
    Object.setPrototypeOf(el, nt.prototype);
    ceState[id] = d;
    return el;
  };
  Object.defineProperty(g, '__ce_new', { value: ceNew });
  var CER = g.CustomElementRegistry = function CustomElementRegistry() { throw new TypeError('Illegal constructor'); };
  var cerErr = function (m, n) { return new DOMException(m, n); };
  CER.prototype.define = function (name, ctor, opts) {
    name = String(name);
    if (typeof ctor !== 'function' || !ctor.prototype || typeof ctor.prototype !== 'object')
      throw new TypeError("Failed to execute 'define' on 'CustomElementRegistry': parameter 2 is not of type 'Function'.");
    if (!ceValidName(name)) throw cerErr("Failed to execute 'define' on 'CustomElementRegistry': \"" + name + '" is not a valid custom element name', 'SyntaxError');
    if (ceByName[name]) throw cerErr("Failed to execute 'define' on 'CustomElementRegistry': the name \"" + name + '" has already been used with this registry', 'NotSupportedError');
    if (ceByCtor.has(ctor)) throw cerErr("Failed to execute 'define' on 'CustomElementRegistry': this constructor has already been used with this registry", 'NotSupportedError');
    var ext = null;
    if (opts && opts.extends != null) {
      ext = String(opts.extends).toLowerCase();
      if (ceValidName(ext)) throw cerErr("Failed to execute 'define' on 'CustomElementRegistry': \"" + ext + '" is a valid custom element name', 'NotSupportedError');
    }
    var P = ctor.prototype, observed = [], oa = ctor.observedAttributes;
    if (oa != null) observed = Array.prototype.map.call(Array.from(oa), String);
    var d = { name: name, ctor: ctor, ext: ext, stack: [], observed: observed,
      connected: P.connectedCallback, disconnected: P.disconnectedCallback, changed: observed.length ? P.attributeChangedCallback : undefined };
    if (!ceOn) { ceOn = true; ceInstall(); }
    ceByName[name] = d; ceByCtor.set(ctor, d);
    var names = Object.keys(ceByName);
    ceSel = names.some(function (k) { return k.indexOf('.') >= 0; }) ? '*'
      : names.map(function (k) { return ceByName[k].ext ? ceByName[k].ext + '[is]' : k; }).join(',');
    var l = __d_query(0, ceSel, false);
    for (var i = 0; i < l.length; i++) { if (!ceState[l[i]] && ceDefFor(l[i]) === d) ceUpgrade(l[i], d, true); }
    if (ceWhen[name]) { var w = ceWhen[name]; delete ceWhen[name]; w.resolve(ctor); }
  };
  CER.prototype.get = function (name) { var d = ceByName[String(name)]; return d ? d.ctor : undefined; };
  CER.prototype.getName = function (ctor) { var d = ceByCtor.get(ctor); return d ? d.name : null; };
  CER.prototype.whenDefined = function (name) {
    name = String(name);
    if (!ceValidName(name)) return Promise.reject(cerErr("Failed to execute 'whenDefined' on 'CustomElementRegistry': \"" + name + '" is not a valid custom element name', 'SyntaxError'));
    if (ceByName[name]) return Promise.resolve(ceByName[name].ctor);
    if (!ceWhen[name]) { var w = {}; w.promise = new Promise(function (res) { w.resolve = res; }); ceWhen[name] = w; }
    return ceWhen[name].promise;
  };
  CER.prototype.upgrade = function (root) {
    if (!root || root.__id === undefined) throw new TypeError("Failed to execute 'upgrade' on 'CustomElementRegistry': parameter 1 is not of type 'Node'.");
    if (ceOn) ceScan(root.__id, true);
  };
  Object.defineProperty(CER.prototype, Symbol.toStringTag, { value: 'CustomElementRegistry', configurable: true });
  Object.defineProperty(g, 'customElements', { value: Object.create(CER.prototype), writable: true, configurable: true, enumerable: true });
  // ---- QY6a: MutationObserver. Records are produced by the shared hooks above
  // (`moChild`/`moAttr`/`moChar`) for observers registered on the target or, with
  // `subtree`, on an ancestor; delivery is one Promise job per batch, observers
  // in creation order, `callback.call(observer, records, observer)`.
  // Transient registered observers: a node removed from an observed subtree
  // inherits the (subtree) registrations of its old ancestors until the next
  // delivery to that observer, so mutations inside the removed subtree still
  // reach it. Cuts: writes from the parser or from Rust (form state, `load_html`)
  // do not notify; `Comment.data = x` is a native no-op (QA18e) so it has no
  // record either; `attributeNamespace` is always null.
  var moReg = Object.create(null), moSeq = 0, moPending = [], moQueued = false;
  var MO_S = Symbol('MutationObserver'), MR_S = Symbol('MutationRecord');
  var moErr = function (e) {
    __scratchy_err('scratchy: js: uncaught exception in MutationObserver callback: ' + (e && e.stack ? String(e) + '\n' + e.stack : String(e)));
  };
  // Observers interested in a mutation of `type` on node `id` -> [{st, old}] or null.
  var moTargets = function (id, type, name) {
    var out = null, n = id, rs, i, j, r, o, e;
    while (n >= 0) {
      rs = moReg[n];
      if (rs) for (i = 0; i < rs.length; i++) {
        r = rs[i]; o = r.opts;
        if (n !== id && !o.subtree) continue;
        if (type === 'childList' ? !o.childList
          : type === 'attributes' ? (!o.attributes || (o.attributeFilter && o.attributeFilter.indexOf(name) < 0))
          : !o.characterData) continue;
        var want = type === 'attributes' ? o.attributeOldValue : type === 'characterData' ? o.characterDataOldValue : false;
        if (!out) out = [];
        for (j = 0, e = null; j < out.length; j++) if (out[j].st === r.st) { e = out[j]; break; }
        if (e) e.old = e.old || want; else out.push({ st: r.st, old: want });
      }
      n = __d_parent(n);
    }
    return out;
  };
  var moEnqueue = function (list, mk) {
    for (var i = 0; i < list.length; i++) {
      var st = list[i].st;
      st.q.push(mk(list[i].old));
      if (!st.pending) { st.pending = true; moPending.push(st); }
    }
    if (!moQueued) { moQueued = true; Promise.resolve().then(moNotify); }
  };
  var moRec = function (type, target, added, removed, prev, next, name, old) {
    var r = Object.create(MR.prototype);
    r[MR_S] = { type: type, target: wrap(target), addedNodes: wrapAll(added), removedNodes: wrapAll(removed),
      previousSibling: wrap(prev), nextSibling: wrap(next), attributeName: name, oldValue: old };
    return r;
  };
  // Removed nodes inherit the subtree registrations of the ancestors of `parent`.
  var moTrans = function (parent, removed) {
    var n = parent, rs, i, j, t;
    while (n >= 0) {
      rs = moReg[n];
      if (rs) for (i = 0; i < rs.length; i++) {
        if (!rs[i].opts.subtree || rs[i].src) continue;
        for (j = 0; j < removed.length; j++) {
          t = { st: rs[i].st, opts: rs[i].opts, src: rs[i] };
          (moReg[removed[j]] || (moReg[removed[j]] = [])).push(t);
          rs[i].st.trans.push({ id: removed[j], reg: t });
          moN++;
        }
      }
      n = __d_parent(n);
    }
  };
  var moChildNow = function (target, added, removed, prev, next, ts) {
    if (removed.length) moTrans(target, removed);
    if (!ts) return;
    moEnqueue(ts, function () { return moRec('childList', target, added, removed, prev, next, null, null); });
  };
  var moChild = function (target, added, removed, prev, next) {
    var ts = moTargets(target, 'childList', null);
    if (moBatch) {
      if (removed.length) moTrans(target, removed);
      if (ts) moBatch.push({ ts: ts, target: target, added: added, removed: removed, prev: prev, next: next });
    } else moChildNow(target, added, removed, prev, next, ts);
  };
  // Pieces of one API call (replaceChild, append(a, b)) on the same target become one record.
  moFlush = function () {
    var b = moBatch, i, j, m, first = [];
    moBatch = null;
    for (i = 0; i < b.length; i++) {
      for (j = 0, m = null; j < first.length; j++) if (first[j].target === b[i].target) { m = first[j]; break; }
      if (!m) { first.push(m = { ts: b[i].ts, target: b[i].target, added: [], removed: [], prev: b[i].prev, next: b[i].next }); }
      m.added = m.added.concat(b[i].added); m.removed = m.removed.concat(b[i].removed); m.next = b[i].next;
    }
    first.forEach(function (m) {
      moEnqueue(m.ts, function () { return moRec('childList', m.target, m.added, m.removed, m.prev, m.next, null, null); });
    });
  };
  var moAttr = function (id, name, old) {
    var ts = moTargets(id, 'attributes', name);
    if (ts) moEnqueue(ts, function (w) { return moRec('attributes', id, [], [], -1, -1, name, w ? old : null); });
  };
  var moChar = function (id, old) {
    var ts = moTargets(id, 'characterData', null);
    if (ts) moEnqueue(ts, function (w) { return moRec('characterData', id, [], [], -1, -1, null, w ? old : null); });
  };
  var moClearTrans = function (st) {
    for (var i = 0; i < st.trans.length; i++) {
      var t = st.trans[i], rs = moReg[t.id], k = rs ? rs.indexOf(t.reg) : -1;
      if (k >= 0) { rs.splice(k, 1); moN--; if (!rs.length) delete moReg[t.id]; }
    }
    st.trans = [];
  };
  var moNotify = function () {
    moQueued = false;
    var list = moPending;
    moPending = [];
    list.sort(function (a, b) { return a.id - b.id; });
    for (var i = 0; i < list.length; i++) {
      var st = list[i], recs = st.q;
      st.pending = false; st.q = [];
      moClearTrans(st);
      if (recs.length) { try { st.cb.call(st.obs, recs, st.obs); } catch (e) { moErr(e); } }
    }
  };
  var MO = g.MutationObserver = function MutationObserver(callback) {
    if (!new.target) throw new TypeError("Failed to construct 'MutationObserver': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError("Failed to construct 'MutationObserver': 1 argument required, but only 0 present.");
    if (typeof callback !== 'function') throw new TypeError("Failed to construct 'MutationObserver': The callback provided as parameter 1 is not a function.");
    this[MO_S] = { cb: callback, obs: this, id: ++moSeq, q: [], nodes: [], trans: [], pending: false };
  };
  var moSt = function (o, fn) {
    var st = o && o[MO_S];
    if (!st) throw new TypeError("Failed to execute '" + fn + "' on 'MutationObserver': Illegal invocation");
    return st;
  };
  MO.prototype.observe = function (target, options) {
    var st = moSt(this, 'observe'), P = "Failed to execute 'observe' on 'MutationObserver': ";
    if (arguments.length < 1) throw new TypeError(P + '1 argument required, but only 0 present.');
    if (!target || target.__id === undefined) throw new TypeError(P + "parameter 1 is not of type 'Node'.");
    if (options != null && typeof options !== 'object' && typeof options !== 'function') throw new TypeError(P + "parameter 2 is not of type 'MutationObserverInit'.");
    var i = options == null ? {} : options;
    var o = { childList: !!i.childList, subtree: !!i.subtree };
    o.attributes = i.attributes !== undefined ? !!i.attributes : (i.attributeOldValue !== undefined || i.attributeFilter !== undefined);
    o.characterData = i.characterData !== undefined ? !!i.characterData : i.characterDataOldValue !== undefined;
    o.attributeOldValue = !!i.attributeOldValue; o.characterDataOldValue = !!i.characterDataOldValue;
    if (i.attributeFilter !== undefined) o.attributeFilter = Array.prototype.map.call(Array.from(i.attributeFilter), String);
    if (!o.childList && !o.attributes && !o.characterData)
      throw new TypeError(P + "The options object must set at least one of 'attributes', 'characterData', or 'childList' to true.");
    if (o.attributeOldValue && !o.attributes) throw new TypeError(P + "The options object may only set 'attributeOldValue' to true when 'attributes' is true or not present.");
    if (o.attributeFilter && !o.attributes) throw new TypeError(P + "The options object may only set 'attributeFilter' when 'attributes' is true or not present.");
    if (o.characterDataOldValue && !o.characterData) throw new TypeError(P + "The options object may only set 'characterDataOldValue' to true when 'characterData' is true or not present.");
    hookInstall();
    var id = target.__id, rs = moReg[id] || (moReg[id] = []), found = null, k;
    for (k = 0; k < rs.length; k++) if (rs[k].st === st && !rs[k].src) { found = rs[k]; break; }
    if (found) {
      found.opts = o;
      st.trans = st.trans.filter(function (t) {
        if (t.reg.src !== found) return true;
        var l = moReg[t.id], x = l ? l.indexOf(t.reg) : -1;
        if (x >= 0) { l.splice(x, 1); moN--; if (!l.length) delete moReg[t.id]; }
        return false;
      });
    } else { rs.push({ st: st, opts: o, src: null }); st.nodes.push(id); moN++; }
  };
  MO.prototype.disconnect = function () {
    var st = moSt(this, 'disconnect');
    st.nodes.forEach(function (id) {
      var rs = moReg[id], k;
      if (!rs) return;
      for (k = rs.length - 1; k >= 0; k--) if (rs[k].st === st && !rs[k].src) { rs.splice(k, 1); moN--; }
      if (!rs.length) delete moReg[id];
    });
    moClearTrans(st);
    st.nodes = []; st.q = [];
  };
  MO.prototype.takeRecords = function () { var st = moSt(this, 'takeRecords'), q = st.q; st.q = []; return q; };
  Object.defineProperty(MO.prototype, Symbol.toStringTag, { value: 'MutationObserver', configurable: true });
  g.WebKitMutationObserver = MO;
  var MR = g.MutationRecord = function MutationRecord() { throw new TypeError('Illegal constructor'); };
  ['type', 'target', 'addedNodes', 'removedNodes', 'previousSibling', 'nextSibling', 'attributeName', 'oldValue'].forEach(function (k) {
    Object.defineProperty(MR.prototype, k, { get: function () {
      var d = this[MR_S];
      if (!d) throw new TypeError('Illegal invocation');
      return d[k];
    }, configurable: true, enumerable: true });
  });
  Object.defineProperty(MR.prototype, 'attributeNamespace', { get: function () { return null; }, configurable: true, enumerable: true });
  Object.defineProperty(MR.prototype, Symbol.toStringTag, { value: 'MutationRecord', configurable: true });
  // ---- QY6g: IntersectionObserver / ResizeObserver / PerformanceObserver with
  // HONEST MINIMAL semantics. JS cannot see layout (no geometry bridge yet,
  // QY-IO-GEOMETRY): so every observed target gets ONE initial notification
  // (timer 0, one batch per observer, observers in creation order) and then
  // nothing more. IO reports "visible once" (isIntersecting true, ratio 1; a
  // lazy loader loads its content) unless the target is detached or inside a
  // `display:none` subtree (false, ratio 0). RO reports the size from the
  // `width`/`height` attributes, else 0x0 (a lie of omission). Rects are at
  // x=y=0 with the attribute size when present; the IO root bounds are the
  // fixed viewport. No hooks or state exist until the first `observe()`.
  var ROD = Symbol('DOMRectReadOnly');
  var rectProps = ['x', 'y', 'width', 'height'];
  var DRO = g.DOMRectReadOnly = function DOMRectReadOnly(x, y, w, h) {
    if (!new.target) throw new TypeError("Failed to construct 'DOMRectReadOnly': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    this[ROD] = { x: +x || 0, y: +y || 0, width: +w || 0, height: +h || 0 };
  };
  DRO.fromRect = function (o) { o = o || {}; return new DRO(o.x, o.y, o.width, o.height); };
  rectProps.forEach(function (k) {
    Object.defineProperty(DRO.prototype, k, { get: function () {
      var d = this[ROD]; if (!d) throw new TypeError('Illegal invocation'); return d[k];
    }, configurable: true, enumerable: true });
  });
  var rdef = function (name, f) { Object.defineProperty(DRO.prototype, name, { get: function () { var d = this[ROD]; if (!d) throw new TypeError('Illegal invocation'); return f(d); }, configurable: true, enumerable: true }); };
  rdef('top', function (d) { return Math.min(d.y, d.y + d.height); });
  rdef('bottom', function (d) { return Math.max(d.y, d.y + d.height); });
  rdef('left', function (d) { return Math.min(d.x, d.x + d.width); });
  rdef('right', function (d) { return Math.max(d.x, d.x + d.width); });
  DRO.prototype.toJSON = function () { var o = {}; rectProps.concat(['top', 'right', 'bottom', 'left']).forEach(function (k) { o[k] = this[k]; }, this); return o; };
  Object.defineProperty(DRO.prototype, Symbol.toStringTag, { value: 'DOMRectReadOnly', configurable: true });
  var DR = g.DOMRect = function DOMRect(x, y, w, h) {
    if (!new.target) throw new TypeError("Failed to construct 'DOMRect': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    this[ROD] = { x: +x || 0, y: +y || 0, width: +w || 0, height: +h || 0 };
  };
  DR.prototype = Object.create(DRO.prototype, { constructor: { value: DR, writable: true, configurable: true } });
  DR.fromRect = function (o) { o = o || {}; return new DR(o.x, o.y, o.width, o.height); };
  rectProps.forEach(function (k) {
    Object.defineProperty(DR.prototype, k, { get: function () { return this[ROD][k]; }, set: function (v) { this[ROD][k] = +v || 0; }, configurable: true, enumerable: true });
  });
  Object.defineProperty(DR.prototype, Symbol.toStringTag, { value: 'DOMRect', configurable: true });
  var roRect = function (w, h) { return new DRO(0, 0, w, h); };
  var attrPx = function (el, a) { var v = parseFloat(el.getAttribute(a)); return v >= 0 && isFinite(v) ? v : 0; };
  // Connected and no `display:none` on the element or an ancestor.
  var ioShown = function (el) {
    var id = el.__id;
    if (!ceIsConn(id)) return false;
    try {
      for (var n = id; n >= 0 && __d_kind(n) === 1; n = __d_parent(n)) if (__d_cs(n, '', 'display') === 'none') return false;
    } catch (e) { /* no style info: assume shown */ }
    return true;
  };
  var obsErr = function (what, e) {
    __scratchy_err('scratchy: js: uncaught exception in ' + what + ' callback: ' + (e && e.stack ? String(e) + '\n' + e.stack : String(e)));
  };
  var obsSeq = 0, obsPending = [], obsQueued = false;
  var obsFlush = function () {
    obsQueued = false;
    var list = obsPending; obsPending = [];
    list.sort(function (a, b) { return a.id - b.id; });
    for (var i = 0; i < list.length; i++) {
      var st = list[i], recs = st.q;
      st.pending = false; st.q = [];
      if (recs.length) { try { st.cb.call(st.obs, recs, st.obs); } catch (e) { obsErr(st.name, e); } }
    }
  };
  var obsSchedule = function (st) {
    if (!st.pending) { st.pending = true; obsPending.push(st); }
    if (!obsQueued) { obsQueued = true; g.setTimeout(obsFlush, 0); }
  };
  var defGetters = function (P, S, keys) {
    keys.forEach(function (k) {
      Object.defineProperty(P, k, { get: function () { var d = this[S]; if (!d) throw new TypeError('Illegal invocation'); return d[k]; }, configurable: true, enumerable: true });
    });
  };
  // -- IntersectionObserver
  var IO_S = Symbol('IntersectionObserver'), IE_S = Symbol('IntersectionObserverEntry');
  var IE = g.IntersectionObserverEntry = function IntersectionObserverEntry() { throw new TypeError('Illegal constructor'); };
  defGetters(IE.prototype, IE_S, ['time', 'rootBounds', 'boundingClientRect', 'intersectionRect', 'isIntersecting', 'intersectionRatio', 'target']);
  Object.defineProperty(IE.prototype, Symbol.toStringTag, { value: 'IntersectionObserverEntry', configurable: true });
  var ioMargin = function (s) {
    var parts = String(s).trim().split(/\s+/);
    if (parts.length === 1 && parts[0] === '') return '0px 0px 0px 0px';
    if (parts.length > 4) throw domErr("Failed to construct 'IntersectionObserver': Failed to parse rootMargin: " + s, 'SyntaxError');
    parts.forEach(function (p) {
      if (!/^[+-]?(\d+\.?\d*|\.\d+)(px|%)$/.test(p)) throw domErr("Failed to construct 'IntersectionObserver': rootMargin must be specified in pixels or percent.", 'SyntaxError');
    });
    var a = parts[0], b = parts[1] !== undefined ? parts[1] : a, c = parts[2] !== undefined ? parts[2] : a, d = parts[3] !== undefined ? parts[3] : b;
    return [a, b, c, d].join(' ');
  };
  var IO = g.IntersectionObserver = function IntersectionObserver(callback, options) {
    var P = "Failed to construct 'IntersectionObserver': ";
    if (!new.target) throw new TypeError(P + "Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError(P + '1 argument required, but only 0 present.');
    if (typeof callback !== 'function') throw new TypeError(P + 'The callback provided as parameter 1 is not a function.');
    if (options != null && typeof options !== 'object' && typeof options !== 'function') throw new TypeError(P + "parameter 2 is not of type 'IntersectionObserverInit'.");
    var o = options || {}, root = o.root === undefined ? null : o.root;
    if (root !== null && (root.__id === undefined || (root.nodeType !== 1 && root.nodeType !== 9))) throw new TypeError(P + "Failed to read the 'root' property from 'IntersectionObserverInit': The provided value is not of type '(Document or Element)'.");
    var margin = ioMargin(o.rootMargin === undefined ? '0px' : o.rootMargin);
    var th = o.threshold === undefined ? [0] : (o.threshold !== null && typeof o.threshold === 'object' && typeof o.threshold[Symbol.iterator] === 'function' ? Array.from(o.threshold) : [o.threshold]);
    th = th.map(function (t) {
      t = Number(t);
      if (!(t >= 0 && t <= 1)) throw new RangeError(P + 'Threshold values must be numbers between 0 and 1.');
      return t;
    }).sort(function (a, b) { return a - b; });
    if (!th.length) th = [0];
    this[IO_S] = { cb: callback, obs: this, id: ++obsSeq, q: [], targets: [], pending: false, name: 'IntersectionObserver', root: root, rootMargin: margin, thresholds: Object.freeze(th) };
  };
  var ioSt = function (o, fn) {
    var st = o && o[IO_S];
    if (!st) throw new TypeError("Failed to execute '" + fn + "' on 'IntersectionObserver': Illegal invocation");
    return st;
  };
  var ioTarget = function (fn, t) {
    if (!t || t.__id === undefined || t.nodeType !== 1) throw new TypeError("Failed to execute '" + fn + "' on 'IntersectionObserver': parameter 1 is not of type 'Element'.");
  };
  IO.prototype.observe = function (target) {
    var st = ioSt(this, 'observe');
    if (arguments.length < 1) throw new TypeError("Failed to execute 'observe' on 'IntersectionObserver': 1 argument required, but only 0 present.");
    ioTarget('observe', target);
    if (st.targets.indexOf(target) >= 0) return;
    st.targets.push(target);
    var shown = ioShown(target), w = shown ? attrPx(target, 'width') : 0, h = shown ? attrPx(target, 'height') : 0;
    var e = Object.create(IE.prototype);
    e[IE_S] = { time: g.performance && g.performance.now ? g.performance.now() : 0, target: target,
      isIntersecting: shown, intersectionRatio: shown ? 1 : 0,
      boundingClientRect: roRect(w, h), intersectionRect: roRect(w, h),
      rootBounds: st.root ? roRect(0, 0) : roRect(Number(g.innerWidth) || 0, Number(g.innerHeight) || 0) };
    st.q.push(e);
    obsSchedule(st);
  };
  IO.prototype.unobserve = function (target) {
    var st = ioSt(this, 'unobserve');
    if (arguments.length < 1) throw new TypeError("Failed to execute 'unobserve' on 'IntersectionObserver': 1 argument required, but only 0 present.");
    ioTarget('unobserve', target);
    var k = st.targets.indexOf(target);
    if (k >= 0) { st.targets.splice(k, 1); st.q = st.q.filter(function (e) { return e[IE_S].target !== target; }); }
  };
  IO.prototype.disconnect = function () { var st = ioSt(this, 'disconnect'); st.targets = []; st.q = []; };
  IO.prototype.takeRecords = function () { var st = ioSt(this, 'takeRecords'), q = st.q; st.q = []; return q; };
  defGetters(IO.prototype, IO_S, ['root', 'rootMargin', 'thresholds']);
  Object.defineProperty(IO.prototype, Symbol.toStringTag, { value: 'IntersectionObserver', configurable: true });
  // -- ResizeObserver
  var RO_S = Symbol('ResizeObserver'), RE_S = Symbol('ResizeObserverEntry'), RZ_S = Symbol('ResizeObserverSize');
  var RZ = g.ResizeObserverSize = function ResizeObserverSize() { throw new TypeError('Illegal constructor'); };
  defGetters(RZ.prototype, RZ_S, ['inlineSize', 'blockSize']);
  Object.defineProperty(RZ.prototype, Symbol.toStringTag, { value: 'ResizeObserverSize', configurable: true });
  var rzSize = function (w, h) { var s = Object.create(RZ.prototype); s[RZ_S] = { inlineSize: w, blockSize: h }; return s; };
  var RE = g.ResizeObserverEntry = function ResizeObserverEntry() { throw new TypeError('Illegal constructor'); };
  defGetters(RE.prototype, RE_S, ['target', 'contentRect', 'borderBoxSize', 'contentBoxSize', 'devicePixelContentBoxSize']);
  Object.defineProperty(RE.prototype, Symbol.toStringTag, { value: 'ResizeObserverEntry', configurable: true });
  var RO = g.ResizeObserver = function ResizeObserver(callback) {
    var P = "Failed to construct 'ResizeObserver': ";
    if (!new.target) throw new TypeError(P + "Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError(P + '1 argument required, but only 0 present.');
    if (typeof callback !== 'function') throw new TypeError(P + 'The callback provided as parameter 1 is not a function.');
    this[RO_S] = { cb: callback, obs: this, id: ++obsSeq, q: [], targets: [], pending: false, name: 'ResizeObserver' };
  };
  var roSt = function (o, fn) {
    var st = o && o[RO_S];
    if (!st) throw new TypeError("Failed to execute '" + fn + "' on 'ResizeObserver': Illegal invocation");
    return st;
  };
  var roForget = function (st, target) {
    var k = st.targets.indexOf(target);
    if (k >= 0) { st.targets.splice(k, 1); st.q = st.q.filter(function (e) { return e[RE_S].target !== target; }); }
  };
  RO.prototype.observe = function (target, options) {
    var st = roSt(this, 'observe'), P = "Failed to execute 'observe' on 'ResizeObserver': ";
    if (arguments.length < 1) throw new TypeError(P + '1 argument required, but only 0 present.');
    if (!target || target.__id === undefined || target.nodeType !== 1) throw new TypeError(P + "parameter 1 is not of type 'Element'.");
    if (options != null && typeof options !== 'object' && typeof options !== 'function') throw new TypeError(P + "parameter 2 is not of type 'ResizeObserverOptions'.");
    var box = options && options.box !== undefined ? String(options.box) : 'content-box';
    if (box !== 'content-box' && box !== 'border-box' && box !== 'device-pixel-content-box')
      throw new TypeError(P + "Failed to read the 'box' property from 'ResizeObserverOptions': The provided value '" + box + "' is not a valid enum value of type ResizeObserverBoxOptions.");
    roForget(st, target);
    st.targets.push(target);
    var w = attrPx(target, 'width'), h = attrPx(target, 'height');
    var e = Object.create(RE.prototype);
    e[RE_S] = { target: target, contentRect: roRect(w, h), borderBoxSize: [rzSize(w, h)], contentBoxSize: [rzSize(w, h)], devicePixelContentBoxSize: [rzSize(w, h)] };
    st.q.push(e);
    obsSchedule(st);
  };
  RO.prototype.unobserve = function (target) {
    var st = roSt(this, 'unobserve');
    if (arguments.length < 1) throw new TypeError("Failed to execute 'unobserve' on 'ResizeObserver': 1 argument required, but only 0 present.");
    if (!target || target.__id === undefined || target.nodeType !== 1) throw new TypeError("Failed to execute 'unobserve' on 'ResizeObserver': parameter 1 is not of type 'Element'.");
    roForget(st, target);
  };
  RO.prototype.disconnect = function () { var st = roSt(this, 'disconnect'); st.targets = []; st.q = []; };
  Object.defineProperty(RO.prototype, Symbol.toStringTag, { value: 'ResizeObserver', configurable: true });
  // -- PerformanceObserver: inert (no performance entries are ever produced).
  var PO_S = Symbol('PerformanceObserver');
  var PO = g.PerformanceObserver = function PerformanceObserver(callback) {
    var P = "Failed to construct 'PerformanceObserver': ";
    if (!new.target) throw new TypeError(P + "Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError(P + '1 argument required, but only 0 present.');
    if (typeof callback !== 'function') throw new TypeError(P + 'The callback provided as parameter 1 is not a function.');
    this[PO_S] = { cb: callback };
  };
  var poSt = function (o, fn) {
    if (!o || !o[PO_S]) throw new TypeError("Failed to execute '" + fn + "' on 'PerformanceObserver': Illegal invocation");
  };
  PO.prototype.observe = function (options) {
    poSt(this, 'observe');
    var P = "Failed to execute 'observe' on 'PerformanceObserver': ";
    if (options == null || typeof options !== 'object') throw new TypeError(P + "parameter 1 is not of type 'PerformanceObserverInit'.");
    if (options.entryTypes === undefined && options.type === undefined) throw new TypeError(P + 'Required member is undefined.');
    if (options.entryTypes !== undefined && options.type !== undefined) throw new TypeError(P + 'An observe() call must not include both entryTypes and type arguments.');
  };
  PO.prototype.disconnect = function () { poSt(this, 'disconnect'); };
  PO.prototype.takeRecords = function () { poSt(this, 'takeRecords'); return []; };
  Object.defineProperty(PO, 'supportedEntryTypes', { get: function () { return Object.freeze([]); }, configurable: true, enumerable: true });
  Object.defineProperty(PO.prototype, Symbol.toStringTag, { value: 'PerformanceObserver', configurable: true });
  // No native Shadow DOM (QY6 memo): fail loudly and harmlessly.
  ElementProto.attachShadow = function () {
    __scratchy_err('scratchy: js: not implemented: Element.attachShadow');
    throw new DOMException('Element.attachShadow is not implemented', 'NotSupportedError');
  };
})(globalThis);
"#;
