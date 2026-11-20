//! Q33b: the read-only DOM API for page scripts. Nodes live in Rust
//! (`Handle`s in a table); JS sees small integer ids through native
//! `__d_*` functions, and a JS-side wrapper per id (cached, so `===`
//! holds) turns them into `Element`-like objects. Document id is 0.
use super::*;
use markup5ever_rcdom::Node;
use crate::js_engine::{self, native, Ctx};

#[derive(Default)]
pub(crate) struct DomTable {
    nodes: Vec<Handle>,
    ids: HashMap<usize, i32>,
    /// QA18a: ids of extra (detached) `Document` roots; id 0 is the page.
    docs: std::collections::HashSet<i32>,
    /// QSD3: shadow roots, host id -> root id and root id -> host id. Kept out
    /// of rcdom (its `template_contents` field changes serialization).
    shadow_of: HashMap<i32, i32>,
    shadow_host: HashMap<i32, i32>,
    // QY21: defined custom element names / customized-built-in tags, per realm (QPT1f: keyed by
    // the id of the realm's document, 0 = the page; each realm has its own CustomElementRegistry).
    ce: HashMap<i32, CeNames>,
}

#[derive(Default)]
struct CeNames {
    names: std::collections::HashSet<String>,
    exts: std::collections::HashSet<String>,
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
    pub(crate) fn id_of_ptr(&self, p: usize) -> Option<i32> {
        self.ids.get(&p).copied()
    }
    pub(crate) fn ptr_of(&self, id: i32) -> Option<usize> {
        self.get(id).map(|h| Rc::as_ptr(&h) as usize)
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

pub(crate) fn set_attr(h: &Handle, name: &str, value: &str) {
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

/// QY45: does the attribute's qualified name (`prefix:local`, or `local`) equal `n`?
fn attr_qname_eq(a: &html5ever::Attribute, n: &str) -> bool {
    match &a.name.prefix {
        Some(p) if !p.is_empty() => {
            let l = a.name.local.as_ref();
            n.len() == p.len() + 1 + l.len() && n.starts_with(p.as_ref()) && n[p.len()..].starts_with(':') && n.ends_with(l)
        }
        _ => a.name.local.as_ref() == n,
    }
}

fn attr_find(h: &Handle, f: impl Fn(&html5ever::Attribute) -> bool) -> Option<String> {
    match &h.data {
        NodeData::Element { attrs, .. } => attrs.borrow().iter().find(|a| f(a)).map(|a| a.value.to_string()),
        _ => None,
    }
}

/// QY45: the name HTML lookups use: lowercased on an HTML-namespace element.
fn attr_lookup_name(h: &Handle, n: &str) -> String {
    match &h.data {
        NodeData::Element { name, .. } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" => n.to_ascii_lowercase(),
        _ => n.to_string(),
    }
}

pub(crate) fn remove_attr(h: &Handle, name: &str) {
    if let NodeData::Element { attrs, .. } = &h.data {
        attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
    }
}

/// QF9a: converts one roxmltree node (and its subtree) into DOM nodes.
fn xml_to_dom(x: roxmltree::Node) -> Option<Handle> {
    use html5ever::{LocalName, Namespace, QualName};
    match x.node_type() {
        roxmltree::NodeType::Element => {
            let tn = x.tag_name();
            let ns = Namespace::from(tn.namespace().unwrap_or(""));
            let mut attrs: Vec<html5ever::Attribute> = Vec::new();
            let parent_ns: Vec<(Option<&str>, &str)> =
                x.parent().map(|p| p.namespaces().map(|n| (n.name(), n.uri())).collect()).unwrap_or_default();
            for n in x.namespaces() {
                if parent_ns.contains(&(n.name(), n.uri())) || n.name() == Some("xml") {
                    continue;
                }
                let (prefix, local) = match n.name() {
                    Some(p) => (Some(html5ever::Prefix::from("xmlns")), p),
                    None => (None, "xmlns"),
                };
                attrs.push(html5ever::Attribute {
                    name: QualName::new(prefix, Namespace::from("http://www.w3.org/2000/xmlns/"), LocalName::from(local)),
                    value: n.uri().into(),
                });
            }
            for a in x.attributes() {
                let prefix = a.namespace().and_then(|u| x.lookup_prefix(u)).filter(|p| !p.is_empty());
                attrs.push(html5ever::Attribute {
                    name: QualName::new(
                        prefix.map(html5ever::Prefix::from),
                        Namespace::from(a.namespace().unwrap_or("")),
                        LocalName::from(a.name()),
                    ),
                    value: a.value().into(),
                });
            }
            let n = Node::new(NodeData::Element {
                name: QualName::new(None, ns, LocalName::from(tn.name())),
                attrs: RefCell::new(attrs),
                template_contents: RefCell::new(None),
                mathml_annotation_xml_integration_point: false,
            });
            for c in x.children() {
                if let Some(cc) = xml_to_dom(c) {
                    cc.parent.set(Some(Rc::downgrade(&n)));
                    n.children.borrow_mut().push(cc);
                }
            }
            Some(n)
        }
        roxmltree::NodeType::Text => Some(Node::new(NodeData::Text { contents: RefCell::new(x.text().unwrap_or("").into()) })),
        roxmltree::NodeType::Comment => Some(Node::new(NodeData::Comment { contents: x.text().unwrap_or("").into() })),
        _ => None,
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
        // QSTK1x: recursive per nesting level; grow the stack (debug frames overflow 2 MB at 5000 deep).
        crate::stack::grow(|| node_body(s, h))
    }
    fn node_body<S: Serializer>(s: &mut S, h: &Handle) -> std::io::Result<()> {
        match &h.data {
            NodeData::Element { name, attrs, .. } => {
                // QY45: html5ever's writer ignores attribute prefixes (it prints
                // `unknown_namespace:`); print `prefix:local` as a plain name.
                let list: Vec<(html5ever::QualName, String)> = attrs
                    .borrow()
                    .iter()
                    .map(|a| match &a.name.prefix {
                        Some(p) if !p.is_empty() => (
                            html5ever::QualName::new(None, html5ever::ns!(), html5ever::LocalName::from(format!("{p}:{}", a.name.local).as_str())),
                            a.value.to_string(),
                        ),
                        _ => (a.name.clone(), a.value.to_string()),
                    })
                    .collect();
                s.start_elem(name.clone(), list.iter().map(|(n, v)| (n, &v[..])))?;
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
    // QSTK1x: recursive per nesting level; grow the stack.
    crate::stack::grow(|| clone_node_body(h, deep))
}

fn clone_node_body(h: &Handle, deep: bool) -> Handle {
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

/// QY48: a change of the page's own URL that is not a navigation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UrlEv {
    /// `history.pushState` (`replace` false) / `replaceState` (true) to this absolute URL.
    Set(String, bool),
    /// `history.back/forward/go` moved the JS-side entry by this many steps.
    Move(isize),
}

/// Installs the `__d_*` natives for `document` and the JS wrappers on top.
/// QB5: requests page script made of the App (which owns focus and the
/// scroll offset); drained by `App::js_sync` through `take_requests`.
#[derive(Default, Debug, Clone)]
pub(crate) struct JsRequests {
    /// QY56: raw `<img src>` values script set (the App fetches them once).
    pub(crate) imgs: Vec<String>,
    pub(crate) focus: Option<FocusReq>,
    /// `window.scroll*`: target axes (`None` = keep), `relative` = scrollBy.
    pub(crate) scroll: Option<ScrollReq>,
    /// QJ6: `el.scrollTop = y` on a scroll container: (element, absolute y).
    pub(crate) el_scroll: Vec<(Handle, f64)>,
    /// Q33d1: `el.click()` on a link asks the App to follow it.
    pub(crate) nav: Option<String>,
    /// QF11: `nav` came from `location.replace` (no new history entry).
    pub(crate) nav_replace: bool,
    /// QF11: `location.reload()`.
    pub(crate) reload: bool,
    /// QY48: pushState/replaceState/history.go URL changes, in order, for the
    /// App's URL and history (no fetch).
    pub(crate) url_events: Vec<UrlEv>,
    /// Q30e: `form.submit()` / an uncancelled `requestSubmit()`: (form, submitter).
    pub(crate) submit: Option<(Handle, Option<Handle>)>,
    /// QF7: validated page-network requests for the App's scheduler.
    pub(crate) net: Vec<crate::js_net::NetJob>,
    /// QF7: results known at once (refused requests, suppressed beacons);
    /// delivered asynchronously like any other result.
    pub(crate) net_done: Vec<(u64, Result<WireResponse, NetError>)>,
    /// QF7: ids the page gave up on that may already be queued or running.
    pub(crate) net_abort: Vec<u64>,
    /// QSD4: the document's adopted-stylesheet CSS text, set whenever it
    /// changes (the App appends it after the page's own author CSS).
    pub(crate) adopted_css: Option<String>,
    /// Q30c: script changed a checkbox/radio's live state (repaint, or restyle
    /// when a stylesheet uses `:checked`).
    pub(crate) check_changed: bool,
    /// Q30d: script changed a text control's live value/selection (repaint only).
    pub(crate) text_changed: bool,
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
    is_click_focusable(h)
}

thread_local! {
    /// QY56: outcome of every image fetch this page, by raw `src`: the decoded
    /// size, or `None` for a failed one. Read by `img.complete` / `naturalWidth`.
    static IMAGE_STATES: RefCell<HashMap<String, Option<(u32, u32)>>> = RefCell::new(HashMap::new());
}

/// QY56: records that the fetch of `src` ended (`None` = failed).
pub(crate) fn note_image_state(src: &str, size: Option<(u32, u32)>) {
    IMAGE_STATES.with(|m| m.borrow_mut().insert(src.to_string(), size));
}

/// QY56: forgets the previous page's image outcomes.
pub(crate) fn clear_image_states() {
    IMAGE_STATES.with(|m| m.borrow_mut().clear());
}

/// QY56: `""` while unknown, `"e"` for a failed image, else `"w,h"`.
fn image_state_text(src: &str) -> String {
    if let Some(bytes) = crate::images::decode_data_uri(src) {
        return crate::images::decode_image_bytes(&bytes).map(|d| format!("{},{}", d.width, d.height)).unwrap_or_else(|| "e".to_string());
    }
    IMAGE_STATES.with(|m| match m.borrow().get(src) {
        None => String::new(),
        Some(None) => "e".to_string(),
        Some(Some((w, h))) => format!("{w},{h}"),
    })
}

thread_local! {
    /// QJ8: the element focus is on (set by `focus()`/`blur()` at once and by
    /// the App for click/Tab focus), weak so a dropped node never lingers.
    static ACTIVE_ELEMENT: RefCell<Option<std::rc::Weak<Node>>> = const { RefCell::new(None) };
}

/// QJ8: the App's focus changed (`None` = blurred).
pub(crate) fn set_active_element(h: Option<&Handle>) {
    ACTIVE_ELEMENT.with(|a| *a.borrow_mut() = h.map(Rc::downgrade));
}

fn active_element() -> Option<Handle> {
    ACTIVE_ELEMENT.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade()))
}

thread_local! {
    /// QY53c: the GEOMETRY generation. Bumped only by a DOM write that changed a
    /// value of a node connected to the page (or by a write of unknown effect),
    /// so no-op writes and writes to detached subtrees do not make `GeomCache`
    /// and the IntersectionObserver signature redo a forced layout.
    static GEOM_GEN: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn geom_gen() -> u64 {
    GEOM_GEN.with(Cell::get)
}

fn geom_bump() {
    GEOM_GEN.with(|g| g.set(g.get() + 1));
}

/// Test-only: a geometry bump of unknown cause.
#[cfg(test)]
pub(crate) fn geom_bump_for_tests() {
    geom_bump();
}

/// Is `h` part of the page document `t.get(0)`?
fn page_connected(t: &DomTable, h: &Handle) -> bool {
    t.get(0).is_some_and(|root| is_ancestor_or_self(&root, h.clone()))
}

/// Bumps the geometry generation when a change happened to a connected node.
fn geom_bump_if(t: &DomTable, h: &Handle, changed: bool) {
    if changed && page_connected(t, h) {
        geom_bump();
    }
}

/// QY53l2: an attribute write bumps the geometry generation unless it changed
/// nothing, missed the page, or landed strictly inside a `display: none`
/// subtree of the last forced layout (js_geom::attr_write_hidden).
fn geom_bump_attr(t: &DomTable, h: &Handle, changed: bool, name: &str) {
    if changed && page_connected(t, h) && !crate::js_geom::attr_write_hidden(h) && !crate::js_geom::attr_write_unread(h, name) {
        geom_bump();
    }
}

/// QY53l2: the child list of `p` (connected) changed; bumps unless the
/// pre-check found it inside a hidden subtree and the root kept its `:empty`-ness.
fn geom_bump_children(p: &Handle, pre: Option<crate::js_geom::HiddenChildren>) {
    if !crate::js_geom::children_write_post(p, pre) {
        geom_bump();
    }
}

/// Does a text-content write of `text` change `h`'s children?
fn text_write_changes(h: &Handle, text: &str) -> bool {
    match &h.data {
        NodeData::Text { contents } => &*contents.borrow() != text,
        NodeData::Comment { .. } => false,
        _ => {
            let kids = h.children.borrow();
            match kids.as_slice() {
                [] => !text.is_empty(),
                [k] => match &k.data {
                    NodeData::Text { contents } => &*contents.borrow() != text,
                    _ => true,
                },
                _ => true,
            }
        }
    }
}

pub(crate) fn install_dom(ctx: &Ctx<'_>, document: &Handle, dirty: Rc<Cell<bool>>, reqs: Rc<RefCell<JsRequests>>) -> Rc<RefCell<DomTable>> {
    let table = Rc::new(RefCell::new(DomTable::default()));
    table.borrow_mut().intern(document);
    // QA16: DOM write generation (invalidates the getComputedStyle cache).
    let wgen: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let t = table.clone();
    native!(ctx, "__d_byid", move |id: String, doc: i32| -> i32 {
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
    native!(ctx, "__d_query", move |root: i32, sel: String, first: bool| -> Option<Vec<i32>> {
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
    // QY21: `__d_ce_add(realm, name, ext)` registers a definition (`ext` = the
    // extended built-in tag or ""); `__d_ce_candidates(realm, root)` walks `root`
    // ONCE and returns, in tree order, the elements whose local name is a
    // defined name, or that carry `is` and whose local name is an extended tag.
    // QPT1f: `realm` = the id of the defining realm's document (0 = the page);
    // `__d_ce_drop(realm)` forgets a dropped iframe realm's names.
    let t = table.clone();
    native!(ctx, "__d_ce_add", move |realm: i32, name: String, ext: String| {
        let mut t = t.borrow_mut();
        let e = t.ce.entry(realm).or_default();
        if ext.is_empty() { e.names.insert(name); } else { e.exts.insert(ext); }
    });
    let t = table.clone();
    native!(ctx, "__d_ce_drop", move |realm: i32| {
        t.borrow_mut().ce.remove(&realm);
    });
    let t = table.clone();
    native!(ctx, "__d_ce_candidates", move |realm: i32, root: i32| -> Vec<i32> {
        let mut t = t.borrow_mut();
        let Some(root) = t.get(root) else { return Vec::new() };
        let Some(ce) = t.ce.get(&realm) else { return Vec::new() };
        let mut all = Vec::new();
        descendants(&root, &mut all);
        let hits: Vec<Handle> = all
            .into_iter()
            .filter(|h| {
                let NodeData::Element { name, .. } = &h.data else { return false };
                let local = name.local.as_ref();
                ce.names.contains(local) || (ce.exts.contains(local) && element_attr(h, "is").is_some())
            })
            .collect();
        hits.iter().map(|h| t.intern(h)).collect()
    });
    let t = table.clone();
    native!(ctx, "__d_matches", move |id: i32, sel: String| -> Option<bool> {
        let list = parse_selector(&sel)?;
        let h = t.borrow().get(id)?;
        Some(is_element(&h) && matches_list(&list, &h))
    });
    let t = table.clone();
    native!(ctx, "__d_kind", move |id: i32| -> i32 {
        let t = t.borrow();
        t.get(id).map_or(0, |h| if t.is_frag(id, &h) { 11 } else { node_type(&h) })
    });
    let t = table.clone();
    native!(ctx, "__d_ns", move |id: i32| -> String {
        match t.borrow().get(id).as_deref().map(|h| &h.data) {
            Some(NodeData::Element { name, .. }) => name.ns.as_ref().to_string(),
            _ => String::new(),
        }
    });
    let t = table.clone();
    native!(ctx, "__d_tag", move |id: i32| -> String {
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
    native!(ctx, "__d_attr", move |id: i32, name: String| -> Option<String> {
        let h = t.borrow().get(id)?;
        element_attr(&h, &name.to_ascii_lowercase()).map(|v| v.to_string())
    });
    // QY11: attribute names in stored order (for `attributes`, `dataset`).
    let t = table.clone();
    native!(ctx, "__d_attr_names", move |id: i32| -> Vec<String> {
        let Some(h) = t.borrow().get(id) else { return Vec::new() };
        match &h.data {
            NodeData::Element { attrs, .. } => attrs.borrow().iter().map(|a| a.name.local.as_ref().to_string()).collect(),
            _ => Vec::new(),
        }
    });
    let t = table.clone();
    native!(ctx, "__d_text", move |id: i32| -> String {
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
    native!(ctx, "__d_children", move |id: i32, elements_only: bool| -> Vec<i32> {
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
    native!(ctx, "__d_sibling", move |id: i32, dir: i32, elements_only: bool| -> i32 {
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
    native!(ctx, "__d_edge_child", move |id: i32, last: bool, elements_only: bool| -> i32 {
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
    // QPT1c: the root of `id`'s tree (a document, or a detached node; `id` itself when it has no parent).
    native!(ctx, "__d_root", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        let Some(mut h) = t.get(id) else { return id };
        let mut moved = false;
        while let Some(p) = parent_handle(&h) {
            h = p;
            moved = true;
        }
        if moved { t.intern(&h) } else { id }
    });
    let t = table.clone();
    native!(ctx, "__d_parent", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id).and_then(|h| parent_handle(&h)) {
            Some(p) => t.intern(&p),
            None => -1,
        }
    });
    // 0 = documentElement, 1 = head, 2 = body, of document `doc`.
    let t = table.clone();
    native!(ctx, "__d_special", move |which: i32, doc: i32| -> i32 {
        let mut t = t.borrow_mut();
        let Some(root) = t.get(doc) else { return -1 };
        // QY51: no whole-tree walk. The document element is the first element
        // child of the root; head/body are the first matching child of it
        // (body: the first `body` or `frameset` child).
        let first = root.children.borrow().iter().find(|c| is_element(c)).cloned();
        let found = match (which, first) {
            (0, f) => f,
            (w, Some(html)) => {
                let kids = html.children.borrow();
                kids.iter()
                    .find(|c| {
                        is_element(c) && {
                            let t = element_tag(c);
                            if w == 1 { t == "head" } else { t == "body" || t == "frameset" }
                        }
                    })
                    .cloned()
            }
            _ => None,
        };
        match found.as_ref() {
            Some(h) => t.intern(h),
            None => -1,
        }
    });
    // QJ8: id of the focused element when it is inside document `doc`, else -1.
    let t = table.clone();
    native!(ctx, "__d_active", move |doc: i32| -> i32 {
        let mut t = t.borrow_mut();
        let (Some(root), Some(a)) = (t.get(doc), active_element()) else { return -1 };
        let mut top = a.clone();
        while let Some(p) = parent_handle(&top) {
            top = p;
        }
        if Rc::ptr_eq(&top, &root) { t.intern(&a) } else { -1 }
    });
    // QA16: serialised computed value of `prop` (dashed) for element `id`.
    let (t, gn) = (table.clone(), wgen.clone());
    let cache = Rc::new(RefCell::new(js_style::StyleCache::default()));
    let (cache2, cache3) = (cache.clone(), cache.clone());
    {
        let w = Rc::downgrade(&cache);
        crate::perf::register_probe("style_cache", move || w.upgrade().and_then(|c| c.try_borrow().ok().map(|c| c.mem_estimate())).unwrap_or((0, 0)));
    }
    native!(ctx, "__d_cs", move |id: i32, pseudo: String, prop: String| -> String {
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
    // QSTK1b: 1 when `display:none` is on element `id` or any ancestor (one cascade walk, not one per ancestor).
    let (t, gn) = (table.clone(), wgen.clone());
    native!(ctx, "__d_cs_hidden", move |id: i32| -> bool {
        let (page, h) = {
            let t = t.borrow();
            (t.get(0).unwrap(), t.get(id))
        };
        let Some(h) = h else { return false };
        let mut top = h.clone();
        while let Some(p) = parent_handle(&top) {
            top = p;
        }
        if !Rc::ptr_eq(&top, &page) {
            return false;
        }
        cache2.borrow_mut().display_none_in_chain(&page, gn.get(), &h, true)
    });
    // QSTK1b: id of `offsetParent`'s answer (nearest body/table/td/th/positioned element ancestor),
    // -1 when unresolvable (the caller falls back to the body), in one cascade walk, not one per ancestor.
    let (t, gn) = (table.clone(), wgen.clone());
    native!(ctx, "__d_offset_parent", move |id: i32| -> i32 {
        let (page, h) = {
            let t = t.borrow();
            (t.get(0).unwrap(), t.get(id))
        };
        let Some(h) = h else { return -1 };
        let found = cache3.borrow_mut().offset_parent_in_chain(&page, gn.get(), &h, true);
        match found {
            Some(n) => t.borrow_mut().intern(&n),
            None => -1,
        }
    });
    // QJ4: layout rect of element `id` (document space), see js_geom.rs.
    let t = table.clone();
    crate::js_geom::clear_rendered();
    let geom = Rc::new(RefCell::new(crate::js_geom::GeomCache::default()));
    let geom2 = geom.clone();
    for glyphs in [false, true] {
        let w = Rc::downgrade(&geom);
        let name = if glyphs { "geom_glyphs" } else { "geom_rects" };
        crate::perf::register_probe(name, move || {
            w.upgrade()
                .and_then(|g| {
                    g.try_borrow().ok().map(|g| {
                        let (r, gl) = g.mem_estimate();
                        if glyphs { gl } else { r }
                    })
                })
                .unwrap_or((0, 0))
        });
    }
    let (geom3, geom4) = (geom.clone(), geom.clone());
    let geom5 = geom.clone();
    let geom7 = geom.clone();
    native!(ctx, "__d_rect", move |c: Ctx<'_>, id: i32, vw: f64, vh: f64| -> Vec<f64> {
        let (page, h) = {
            let t = t.borrow();
            (t.get(0), t.get(id))
        };
        let (Some(page), Some(h)) = (page, h) else { return Vec::new() };
        // QOBS3: the page caller is only looked up when a layout is about to run under --perf-report.
        if crate::perf::enabled() && !crate::perf::in_io() && !geom5.borrow().is_fresh(geom_gen(), vw as f32, vh as f32) {
            crate::perf::set_caller(crate::js_engine::caller_frame(&c, crate::js::PRELUDE_SCRIPTS));
        }
        geom.borrow_mut().rect_of(&page, geom_gen(), &h, vw as f32, vh as f32)
    });
    // QJ7: per-UTF-16-unit glyph rects of text node `id` (js_geom::text_rects),
    // plus the write generation so script can cache them between DOM writes.
    let t = table.clone();
    let geom6 = geom2.clone();
    native!(ctx, "__d_textrects", move |c: Ctx<'_>, id: i32, vw: f64, vh: f64| -> Vec<f64> {
        let (page, h) = {
            let t = t.borrow();
            (t.get(0), t.get(id))
        };
        let (Some(page), Some(h)) = (page, h) else { return Vec::new() };
        if crate::perf::enabled() && !crate::perf::in_io() && !geom6.borrow().is_fresh(geom_gen(), vw as f32, vh as f32) {
            crate::perf::set_caller(crate::js_engine::caller_frame(&c, crate::js::PRELUDE_SCRIPTS));
        }
        geom2.borrow_mut().text_rects(&page, geom_gen(), &h, vw as f32, vh as f32)
    });
    // QY53e2: the App's own layout (js_geom::set_rendered) as a read-only rect source for IntersectionObserver.
    let t = table.clone();
    native!(ctx, "__d_rect_r", move |id: i32| -> Vec<f64> {
        let Some(h) = t.borrow().get(id) else { return Vec::new() };
        crate::js_geom::rendered_rect(&h)
    });
    native!(ctx, "__d_rend_ok", |vw: f64, vh: f64| -> bool { crate::js_geom::rendered_ok(vw as u32, vh as u32) });
    native!(ctx, "__d_rend_clear", crate::js_geom::clear_rendered);
    // QY53e1: is the geometry cache current for this viewport (no layout), and what the last layout cost.
    native!(ctx, "__d_geom_fresh", move |vw: f64, vh: f64| -> bool { geom3.borrow().is_fresh(geom_gen(), vw as f32, vh as f32) });
    native!(ctx, "__d_geom_cost", move || -> f64 { geom4.borrow().last_cost_ms() });
    // QOBS3: scratchy's IntersectionObserver evaluation is running (a forced layout it causes is `io`).
    native!(ctx, "__d_io", |on: bool| crate::perf::set_io(on));
    let gn = wgen.clone();
    native!(ctx, "__d_wgen", move || -> f64 { gn.get() as f64 });
    // QYTC1a: plus the in-place shifts, so script caches keyed on it (IntersectionObserver,
    // Range glyph rects) still see a box that moved without a geometry-generation bump.
    native!(ctx, "__d_ggen", move || -> f64 { (geom_gen() + crate::js_geom::geom_shifts()) as f64 });
    native!(ctx, "__d_csprops", move || -> Vec<String> { js_style::PROPS.iter().map(|s| s.to_string()).collect() });
    // ---- Q33c mutation. Every write marks the page dirty; the event
    // loop rebuilds layout once per turn, not once per call.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_setattr", move |id: i32, name: String, val: String| {
        if let Some(h) = t.borrow().get(id) {
            let name = name.to_ascii_lowercase();
            let old = element_attr(&h, &name).map(|s| s.to_string());
            let changed = old.as_deref() != Some(val.as_str());
            // QYTC1d: a `class` write that changes no element's matched rules keeps the layout.
            let probe = (changed && name == "class" && crate::js_geom::class_probe_possible() && page_connected(&t.borrow(), &h))
                .then(|| crate::js_geom::class_write_pre(&h, old.as_deref(), Some(&val)))
                .flatten();
            set_attr(&h, &name, &val);
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            // QYTC1a: a contained `style` write moves the cached boxes in place instead.
            let in_place = changed
                && name == "style"
                && page_connected(&t.borrow(), &h)
                && geom7.try_borrow_mut().is_ok_and(|mut g| g.style_write_in_place(&h, old.as_deref(), Some(&val), geom_gen()));
            let neutral = probe.is_some_and(crate::js_geom::class_write_post);
            if !in_place && !neutral {
                geom_bump_attr(&t.borrow(), &h, changed, &name);
            }
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_rmattr", move |id: i32, name: String| {
        if let Some(h) = t.borrow().get(id) {
            let name = name.to_ascii_lowercase();
            let changed = element_attr(&h, &name).is_some();
            let probe = (changed && name == "class" && crate::js_geom::class_probe_possible() && page_connected(&t.borrow(), &h))
                .then(|| crate::js_geom::class_write_pre(&h, element_attr(&h, "class").as_deref(), None))
                .flatten();
            remove_attr(&h, &name);
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            if !probe.is_some_and(crate::js_geom::class_write_post) {
                geom_bump_attr(&t.borrow(), &h, changed, &name);
            }
        }
    });
    // QY45: qualified-name / namespaced attribute access over the stored
    // QualName {prefix, ns, local}. Namespace "" = no namespace.
    let t = table.clone();
    native!(ctx, "__d_attr_list", move |id: i32| -> Vec<String> {
        let Some(h) = t.borrow().get(id) else { return Vec::new() };
        match &h.data {
            NodeData::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .flat_map(|a| {
                    [
                        a.name.ns.as_ref().to_string(),
                        a.name.prefix.as_ref().map(|p| p.as_ref().to_string()).unwrap_or_default(),
                        a.name.local.as_ref().to_string(),
                        a.value.to_string(),
                    ]
                })
                .collect(),
            _ => Vec::new(),
        }
    });
    let t = table.clone();
    native!(ctx, "__d_getq", move |id: i32, name: String| -> Option<String> {
        let h = t.borrow().get(id)?;
        let n = attr_lookup_name(&h, &name);
        attr_find(&h, |a| attr_qname_eq(a, &n))
    });
    let t = table.clone();
    native!(ctx, "__d_getns", move |id: i32, ns: String, local: String| -> Option<String> {
        let h = t.borrow().get(id)?;
        attr_find(&h, |a| a.name.ns.as_ref() == ns && a.name.local.as_ref() == local)
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_setq", move |id: i32, name: String, val: String| {
        if let Some(h) = t.borrow().get(id) {
            let n = attr_lookup_name(&h, &name);
            let mut changed = false;
            if let NodeData::Element { attrs, .. } = &h.data {
                let mut attrs = attrs.borrow_mut();
                if let Some(a) = attrs.iter_mut().find(|a| attr_qname_eq(a, &n)) {
                    changed = &*a.value != val.as_str();
                    a.value = val.as_str().into();
                } else {
                    changed = true;
                    let qn = html5ever::QualName::new(None, html5ever::ns!(), html5ever::LocalName::from(n.as_str()));
                    attrs.push(html5ever::Attribute { name: qn, value: val.as_str().into() });
                }
            }
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            geom_bump_attr(&t.borrow(), &h, changed, &n);
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_rmq", move |id: i32, name: String| {
        if let Some(h) = t.borrow().get(id) {
            let n = attr_lookup_name(&h, &name);
            let mut changed = false;
            if let NodeData::Element { attrs, .. } = &h.data {
                let mut attrs = attrs.borrow_mut();
                let before = attrs.len();
                attrs.retain(|a| !attr_qname_eq(a, &n));
                changed = attrs.len() != before;
            }
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            geom_bump_attr(&t.borrow(), &h, changed, &n);
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_setns", move |id: i32, ns: String, prefix: String, local: String, val: String| {
        if let Some(h) = t.borrow().get(id) {
            let mut changed = false;
            if let NodeData::Element { attrs, .. } = &h.data {
                let mut attrs = attrs.borrow_mut();
                if let Some(a) = attrs.iter_mut().find(|a| a.name.ns.as_ref() == ns && a.name.local.as_ref() == local) {
                    changed = &*a.value != val.as_str();
                    a.value = val.as_str().into();
                } else {
                    changed = true;
                    let qn = html5ever::QualName::new(
                        if prefix.is_empty() { None } else { Some(html5ever::Prefix::from(prefix.as_str())) },
                        html5ever::Namespace::from(ns.as_str()),
                        html5ever::LocalName::from(local.as_str()),
                    );
                    attrs.push(html5ever::Attribute { name: qn, value: val.as_str().into() });
                }
            }
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            geom_bump_attr(&t.borrow(), &h, changed, "");
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_rmns", move |id: i32, ns: String, local: String| {
        if let Some(h) = t.borrow().get(id) {
            let mut changed = false;
            if let NodeData::Element { attrs, .. } = &h.data {
                let mut attrs = attrs.borrow_mut();
                let before = attrs.len();
                attrs.retain(|a| !(a.name.ns.as_ref() == ns && a.name.local.as_ref() == local));
                changed = attrs.len() != before;
            }
            // QY53m: a write that changed no value does not wake the App's restyle.
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            geom_bump_attr(&t.borrow(), &h, changed, "");
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_settext", move |id: i32, text: String| {
        if let Some(h) = t.borrow().get(id) {
            let changed = text_write_changes(&h, &text);
            // QY53l2: the element whose child list (or child text) changes.
            let owner = match &h.data {
                NodeData::Text { .. } | NodeData::Comment { .. } => parent_handle(&h),
                _ => Some(h.clone()),
            };
            let conn = changed && page_connected(&t.borrow(), &h);
            let pre = match &owner {
                Some(o) if conn => {
                    let moved: Vec<Handle> = if Rc::ptr_eq(o, &h) { h.children.borrow().clone() } else { Vec::new() };
                    crate::js_geom::children_write_pre(o, &moved)
                }
                _ => None,
            };
            match &h.data {
                NodeData::Text { contents } => *contents.borrow_mut() = text.as_str().into(),
                NodeData::Comment { .. } => {}
                _ => set_children_text(&h, &text),
            }
            d.set(d.get() | changed);
            gn.set(gn.get() + 1);
            if conn {
                match &owner {
                    Some(o) => geom_bump_children(o, pre),
                    None => geom_bump(),
                }
            }
        }
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_sethtml", move |id: i32, html: String| {
        if let Some(h) = t.borrow().get(id) {
            let conn = page_connected(&t.borrow(), &h);
            let pre = if conn && !crate::js_geom::html_sensitive(&html) {
                let old: Vec<Handle> = h.children.borrow().clone();
                crate::js_geom::children_write_pre(&h, &old)
            } else {
                None
            };
            set_inner_html(&h, &html);
            d.set(true);
            gn.set(gn.get() + 1);
            if conn {
                geom_bump_children(&h, pre);
            }
        }
    });
    // QA13: returns the id of the last inserted node, or -1.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_write", move |after: i32, html: String| -> i32 {
        let mut t = t.borrow_mut();
        let Some(a) = t.get(after) else { return -1 };
        let last = write_after(&a, &html);
        d.set(true);
        gn.set(gn.get() + 1);
        geom_bump_if(&t, &a, true);
        t.intern(&last)
    });
    let t = table.clone();
    native!(ctx, "__d_gethtml", move |id: i32, outer: bool| -> String {
        t.borrow().get(id).map(|h| serialize_node(&h, outer)).unwrap_or_default()
    });
    // kind: 1 element (`s` = tag), 3 text, 8 comment, 11 fragment.
    let t = table.clone();
    native!(ctx, "__d_create", move |kind: i32, s: String| -> i32 {
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
    native!(ctx, "__d_template_content", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        let content = t.get(id).as_ref().and_then(template_content);
        match content {
            Some(c) => t.intern(&c),
            None => -1,
        }
    });
    // QSD3: shadow-root side table. `__d_shadow_set(host, root)` registers,
    // `__d_shadow_of(host)` -> root id or -1, `__d_shadow_host(root)` -> host id or -1.
    let t = table.clone();
    native!(ctx, "__d_shadow_set", move |host: i32, root: i32| {
        let mut t = t.borrow_mut();
        t.shadow_of.insert(host, root);
        t.shadow_host.insert(root, host);
    });
    let t = table.clone();
    native!(ctx, "__d_shadow_of", move |host: i32| -> i32 { t.borrow().shadow_of.get(&host).copied().unwrap_or(-1) });
    let t = table.clone();
    native!(ctx, "__d_shadow_host", move |root: i32| -> i32 { t.borrow().shadow_host.get(&root).copied().unwrap_or(-1) });
    // QA18a: a new detached Document root (its id is registered as a
    // document, not a fragment); a doctype node (name, public, system).
    let t = table.clone();
    native!(ctx, "__d_newdoc", move || -> i32 {
        let mut t = t.borrow_mut();
        let id = t.intern(&Node::new(NodeData::Document));
        t.docs.insert(id);
        id
    });
    // QA18b1: [absolute url, html] of a pre-fetched iframe `src` (empty when
    // the iframe's src was not pre-fetched); `__d_load_html` parses a full
    // document into a second document's root.
    let t = table.clone();
    native!(ctx, "__d_iframe_page", move |id: i32| -> Vec<String> {
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
    native!(ctx, "__d_load_html", move |id: i32, html: String| {
        let Some(doc) = t.borrow().get(id) else { return };
        let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return };
        clear_children(&doc);
        let kids: Vec<Handle> = dom.document.children.borrow_mut().drain(..).collect();
        for k in kids {
            k.parent.set(Some(Rc::downgrade(&doc)));
            doc.children.borrow_mut().push(k);
        }
    });
    // QF9a: DOMParser text/html: a full parse with scripting DISABLED (so
    // `<noscript>` content is real elements). Scripts made here are never
    // marked eligible to run.
    let t = table.clone();
    native!(ctx, "__d_parse_html", move |id: i32, html: String| {
        let Some(doc) = t.borrow().get(id) else { return };
        let mut opts = ParseOpts::default();
        opts.tree_builder.scripting_enabled = false;
        let Ok(dom) = parse_document(RcDom::default(), opts).from_utf8().read_from(&mut html.as_bytes()) else { return };
        clear_children(&doc);
        let kids: Vec<Handle> = dom.document.children.borrow_mut().drain(..).collect();
        for k in kids {
            k.parent.set(Some(Rc::downgrade(&doc)));
            doc.children.borrow_mut().push(k);
        }
    });
    // QF9a: DOMParser / XHR XML: a well-formedness-checking parse (roxmltree)
    // into the document's root. Returns "" on success, else "message\nline\ncol".
    let t = table.clone();
    native!(ctx, "__d_load_xml", move |id: i32, text: String| -> String {
        let Some(doc) = t.borrow().get(id) else { return String::new() };
        let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        match roxmltree::Document::parse_with_options(&text, opts) {
            Ok(x) => {
                clear_children(&doc);
                for c in x.root().children() {
                    if let Some(n) = xml_to_dom(c) {
                        n.parent.set(Some(Rc::downgrade(&doc)));
                        doc.children.borrow_mut().push(n);
                    }
                }
                String::new()
            }
            Err(e) => {
                let p = e.pos();
                format!("{}\n{}\n{}", e.to_string().split(" at ").next().unwrap_or(""), p.row, p.col)
            }
        }
    });
    let t = table.clone();
    native!(ctx, "__d_create_doctype", move |name: String, public_id: String, system_id: String| -> i32 {
        let data = NodeData::Doctype { name: name.as_str().into(), public_id: public_id.as_str().into(), system_id: system_id.as_str().into() };
        t.borrow_mut().intern(&Node::new(data))
    });
    let t = table.clone();
    native!(ctx, "__d_doctype", move |id: i32| -> Vec<String> {
        match t.borrow().get(id).as_deref().map(|h| &h.data) {
            Some(NodeData::Doctype { name, public_id, system_id }) => vec![name.to_string(), public_id.to_string(), system_id.to_string()],
            _ => Vec::new(),
        }
    });
    // QA14: createElementNS keeps the qualified name's case outside XHTML.
    let t = table.clone();
    native!(ctx, "__d_create_ns", move |ns: String, name: String| -> i32 {
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
    native!(ctx, "__d_clone", move |id: i32, deep: bool| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id) {
            Some(h) => t.intern(&clone_node(&h, deep)),
            None => -1,
        }
    });
    // `before` < 0 appends. `false` = HierarchyRequestError.
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_insert", move |parent: i32, child: i32, before: i32| -> bool {
        let t = t.borrow();
        let (Some(p), Some(c)) = (t.get(parent), t.get(child)) else { return false };
        if !matches!(p.data, NodeData::Element { .. } | NodeData::Document) {
            return false;
        }
        let frag = t.is_frag(child, &c);
        let old_parent = if frag { None } else { parent_handle(&c) };
        // QY53l2: a move between (or within) hidden subtrees changes no box; both ends are checked.
        let new_conn = page_connected(&t, &p);
        let old_conn = old_parent.filter(|o| page_connected(&t, o));
        let moved: Vec<Handle> = if new_conn || old_conn.is_some() {
            if frag { c.children.borrow().clone() } else { vec![c.clone()] }
        } else {
            Vec::new()
        };
        let pre_new = if new_conn { crate::js_geom::children_write_pre(&p, &moved) } else { None };
        let pre_old = old_conn.as_ref().map(|o| crate::js_geom::children_write_pre(o, &moved));
        let ok = insert_child(&p, &c, t.get(before).as_ref(), frag);
        d.set(true);
        gn.set(gn.get() + 1);
        if ok && (new_conn || old_conn.is_some()) {
            let skip_new = !new_conn || crate::js_geom::children_write_post(&p, pre_new);
            let skip_old = match (&old_conn, pre_old) {
                (Some(o), Some(pre)) => crate::js_geom::children_write_post(o, pre),
                _ => true,
            };
            if !(skip_new && skip_old) {
                geom_bump();
            }
        }
        ok
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_remove", move |id: i32| {
        if let Some(h) = t.borrow().get(id) {
            let old_parent = parent_handle(&h).filter(|o| page_connected(&t.borrow(), o));
            let pre = old_parent.as_ref().and_then(|o| crate::js_geom::children_write_pre(o, std::slice::from_ref(&h)));
            detach(&h);
            d.set(true);
            gn.set(gn.get() + 1);
            if let Some(o) = &old_parent {
                geom_bump_children(o, pre);
            }
        }
    });
    let ct = table.clone();
    let (t_sc, r_sc, gn_sc) = (table.clone(), reqs.clone(), wgen.clone());
    let t_i = table.clone();
    let (t_si, r_si, gn_si) = (table.clone(), reqs.clone(), wgen.clone());
    let t_rp = table.clone();
    let (t_rf, r_rf, gn_rf) = (table.clone(), reqs.clone(), wgen.clone());
    // Q30c: live checkbox/radio state (forms.rs CHECK_LIVE). Writes only ask the
    // App for a repaint/restyle (`check_changed`), never a DOM-dirty rebuild.
    native!(ctx, "__d_checked", move |id: i32| -> bool { ct.borrow().get(id).is_some_and(|h| crate::live_checked(&h)) });
    native!(ctx, "__d_setchecked", move |id: i32, v: bool| {
        if let Some(h) = t_sc.borrow().get(id)
            && crate::set_checked(&h, v)
        {
            r_sc.borrow_mut().check_changed = true;
            gn_sc.set(gn_sc.get() + 1);
            geom_bump();
        }
    });
    native!(ctx, "__d_indet", move |id: i32| -> bool { t_i.borrow().get(id).is_some_and(|h| crate::live_indeterminate(&h)) });
    native!(ctx, "__d_setindet", move |id: i32, v: bool| {
        if let Some(h) = t_si.borrow().get(id) {
            crate::set_indeterminate(&h, v);
            r_si.borrow_mut().check_changed = true;
            gn_si.set(gn_si.get() + 1);
            geom_bump();
        }
    });
    // The other checked radio of `id`'s group, or -1.
    native!(ctx, "__d_radiopeer", move |id: i32| -> i32 {
        let h = t_rp.borrow().get(id);
        match h.and_then(|h| crate::checked_radio_peer(&h)) {
            Some(p) => t_rp.borrow_mut().intern(&p),
            None => -1,
        }
    });
    native!(ctx, "__d_resetform", move |id: i32| {
        if let Some(h) = t_rf.borrow().get(id)
            && crate::reset_form_state(&h)
        {
            r_rf.borrow_mut().check_changed = true;
            gn_rf.set(gn_rf.get() + 1);
            geom_bump();
        }
    });
    // Q30g: live select state (forms_select.rs).
    let t_si2 = table.clone();
    native!(ctx, "__d_selidx", move |id: i32| -> i32 { t_si2.borrow().get(id).map_or(-1, |h| crate::selected_index(&h)) });
    let (t_ssi, r_ssi, gn_ssi) = (table.clone(), reqs.clone(), wgen.clone());
    native!(ctx, "__d_setselidx", move |id: i32, v: i32| {
        if let Some(h) = t_ssi.borrow().get(id) {
            let idx: Vec<usize> = if v >= 0 { vec![v as usize] } else { Vec::new() };
            crate::set_selected_indices(&h, &idx);
            r_ssi.borrow_mut().check_changed = true;
            gn_ssi.set(gn_ssi.get() + 1);
            geom_bump();
        }
    });
    let t_os = table.clone();
    native!(ctx, "__d_optsel", move |id: i32| -> bool { t_os.borrow().get(id).is_some_and(|h| crate::option_is_selected(&h)) });
    let (t_sos, r_sos, gn_sos) = (table.clone(), reqs.clone(), wgen.clone());
    native!(ctx, "__d_setoptsel", move |id: i32, v: bool| {
        if let Some(h) = t_sos.borrow().get(id) {
            crate::set_option_selected(&h, v);
            r_sos.borrow_mut().check_changed = true;
            gn_sos.set(gn_sos.get() + 1);
            geom_bump();
        }
    });
    // Q30d: live text value / selection of input and textarea (forms_text.rs).
    let t_tv = table.clone();
    native!(ctx, "__d_tvalue", move |id: i32| -> String { t_tv.borrow().get(id).map(|h| crate::text_value(&h)).unwrap_or_default() });
    let (t_stv, r_stv, gn_stv) = (table.clone(), reqs.clone(), wgen.clone());
    native!(ctx, "__d_settvalue", move |id: i32, v: String| {
        if let Some(h) = t_stv.borrow().get(id) {
            crate::set_text_value(&h, &v);
            r_stv.borrow_mut().text_changed = true;
            gn_stv.set(gn_stv.get() + 1);
            geom_bump();
        }
    });
    let t_sg = table.clone();
    native!(ctx, "__d_selget", move |id: i32, end: bool| -> i32 {
        t_sg.borrow().get(id).map_or(0, |h| crate::text_selection(&h, end) as i32)
    });
    let (t_ss, r_ss, gn_ss) = (table.clone(), reqs.clone(), wgen.clone());
    native!(ctx, "__d_selset", move |id: i32, a: i32, b: i32| {
        if let Some(h) = t_ss.borrow().get(id) {
            crate::set_text_selection(&h, a.max(0) as usize, b.max(0) as usize);
            r_ss.borrow_mut().text_changed = true;
            gn_ss.set(gn_ss.get() + 1);
            geom_bump();
        }
    });
    // QB5: focus()/blur() only record a request; the App applies it.
    let (t, r) = (table.clone(), reqs.clone());
    native!(ctx, "__d_focus", move |id: i32, focus: bool| {
        if let Some(h) = t.borrow().get(id) {
            if focus && js_focusable(&h) {
                set_active_element(Some(&h));
                r.borrow_mut().focus = Some(FocusReq::Focus(h));
            } else if !focus {
                if active_element().is_some_and(|a| Rc::ptr_eq(&a, &h)) {
                    set_active_element(None);
                }
                r.borrow_mut().focus = Some(FocusReq::Blur(h));
            }
        }
    });
    let (r, d, gn) = (reqs.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_adopted", move |css: String| {
        crate::js_style::set_js_adopted_css(&css);
        r.borrow_mut().adopted_css = Some(css);
        d.set(true);
        gn.set(gn.get() + 1);
        geom_bump();
    });
    let (t, d, gn) = (table.clone(), dirty.clone(), wgen.clone());
    native!(ctx, "__d_sheet_text", move |id: i32, orig: String, css: String| {
        if let Some(h) = t.borrow().get(id) {
            crate::images::set_style_override(std::rc::Rc::as_ptr(&h) as usize, orig, css);
            d.set(true);
            gn.set(gn.get() + 1);
            geom_bump();
        }
    });
    native!(ctx, "__d_img_state", move |src: String| -> String { image_state_text(&src) });
    let r = reqs.clone();
    native!(ctx, "__d_img_req", move |src: String| {
        r.borrow_mut().imgs.push(src);
    });
    let r = reqs.clone();
    native!(ctx, "__d_navigate", move |href: String| {
        r.borrow_mut().nav = Some(href);
    });
    let r = reqs.clone();
    native!(ctx, "__d_nav_replace", move |href: String| {
        let mut r = r.borrow_mut();
        r.nav = Some(href);
        r.nav_replace = true;
    });
    let r = reqs.clone();
    native!(ctx, "__d_url_update", move |href: String, replace: bool| {
        r.borrow_mut().url_events.push(UrlEv::Set(href, replace));
    });
    let r = reqs.clone();
    native!(ctx, "__d_url_move", move |delta: i32| {
        r.borrow_mut().url_events.push(UrlEv::Move(delta as isize));
    });
    let r = reqs.clone();
    native!(ctx, "__d_reload", move || {
        r.borrow_mut().reload = true;
    });
    let t_fd = table.clone();
    native!(ctx, "__d_formdata", move |form: i32, submitter: i32| -> String {
        let f = t_fd.borrow().get(form);
        let s = if submitter >= 0 { t_fd.borrow().get(submitter) } else { None };
        f.map(|f| crate::encode_urlencoded(&crate::form_data_set(&f, s.as_ref()))).unwrap_or_default()
    });
    let (t, r) = (table.clone(), reqs.clone());
    native!(ctx, "__d_submit", move |form: i32, submitter: i32| {
        let f = t.borrow().get(form);
        let s = if submitter >= 0 { t.borrow().get(submitter) } else { None };
        if let Some(f) = f {
            r.borrow_mut().submit = Some((f, s));
        }
    });
    let r = reqs.clone();
    native!(ctx, "__d_scroll", move |x: f64, y: f64, relative: bool| {
        let f = |v: f64| if v.is_finite() { Some(v) } else { None };
        r.borrow_mut().scroll = Some(ScrollReq { x: f(x), y: f(y), relative });
    });
    // QJ6: element scrolling. `__d_nid` is the layout/App node id of a JS
    // wrapper id (what the App keys its per-box scroll offsets by).
    let t = table.clone();
    native!(ctx, "__d_nid", move |id: i32| -> f64 { t.borrow().get(id).map_or(0.0, |h| node_id(&h) as f64) });
    let (t, r) = (table.clone(), reqs.clone());
    native!(ctx, "__d_scrollel", move |id: i32, y: f64| {
        if let Some(h) = t.borrow().get(id) {
            let mut r = r.borrow_mut();
            r.el_scroll.retain(|(e, _)| !Rc::ptr_eq(e, &h));
            r.el_scroll.push((h, y));
        }
    });
    // QV8-4: a startup-snapshot engine already ran the DOM prelude; the snapshot BUILD keeps the
    // plain `__d_all` global (the undetectable wrapper cannot be baked) and each page engine makes it.
    if !js_engine::prebuilt(ctx) {
        js_engine::eval_quiet(ctx, DOM_PRELUDE);
    }
    if !js_engine::baking() {
        // QY40: give `document.all` the [[IsHTMLDDA]] slot (falsy, typeof 'undefined').
        js_engine::mark_htmldda(ctx, "__d_all");
        js_engine::remove_global(ctx, "__d_all");
    }
    table
}

/// Wrappers over the natives; runs after the base prelude (which defines
/// `document`). Element event listeners are stored but never fired yet.
pub(crate) const DOM_PRELUDE: &str = r#"
(function (g) {
  var doc = g.document, cache = {}, tagProto = Object.create(null);
  // QY6e: the mutation natives the custom-element hooks rebind (once, at the
  // first `customElements.define`); until then nothing is wrapped (zero cost).
  var __d_insert = g.__d_insert, __d_remove = g.__d_remove, __d_sethtml = g.__d_sethtml, __d_settext = g.__d_settext,
    __d_setattr = g.__d_setattr, __d_rmattr = g.__d_rmattr, __d_setq = g.__d_setq, __d_rmq = g.__d_rmq, __d_setns = g.__d_setns, __d_rmns = g.__d_rmns, __d_write = g.__d_write, ceNew;
  // QY6a: live MutationObserver registrations (0 until the first `observe()`),
  // and the childList batch used by multi-step APIs (replaceChild, append(...a, b)).
  var moBatch = null, moFlush;
  var moBegin = function () { if (xr.mo.n > 0 && !moBatch) { moBatch = []; return true; } return false; };
  var moEnd = function (own) { if (own) moFlush(); };
  var queries, svgProto = Object.create(null);
  // QPT1c (R53 D7): ONE wrapper per node across every realm of the page, made by (and with the
  // prototypes of) the realm that owns the node's document, as in Chrome. An iframe realm shares
  // the page's `cache`, `owners` and `xr` (bound in `__d_bind_doc`); `xr.byDoc` maps a document id
  // to the `mk` of the realm that owns it, and `xr.n` counts the live iframe realms (0: no lookup).
  // `mine`/`myDocs`: the wrappers and documents an iframe realm made (purged when it is dropped).
  // QPT1f: `xr.prep` maps a browsing-context document (the page's, an iframe realm's) to the
  // `sPrepare` of its realm (script-inserted scripts run in the realm of the document they enter),
  // `xr.s` is the page-wide table of script-inserted <script> elements that may still run.
  var xr = { n: 0, byDoc: {}, prep: {}, s: { el: {}, n: 0, ord: {}, cr: {} },
    // QPT1c2: the page-wide MutationObserver registry (`reg`: node id -> registrations, `n` live count,
    // `seq` observer ids) and `hk`, each realm's `hookInstall` (an observer anywhere hooks every realm's natives).
    mo: { reg: Object.create(null), n: 0, seq: 0 }, hk: [],
    // QPT1f4: custom-element reactions are page-wide: `ce` = doc id -> a realm's reaction API (realms that
    // called define()), `cs` = element id -> the API of the realm whose definition upgraded it, `cn` = live `ce` count.
    ce: {}, cs: {}, cn: 0 }, mine = null, myDocs = null, moSts = null;
  var ownerDocId = function (id) {
    var r = __d_root(id);
    if (__d_kind(r) === 9) return r;
    var od = owners[r] || owners[id];
    return od ? od.__id : -1;
  };
  var wrap = function (id) {
    if (id === undefined || id < 0) return null;
    var o = cache[id];
    if (o) return o;
    if (xr.n) {
      var ow = xr.byDoc[ownerDocId(id)];
      if (ow && ow !== mk) return ow(id);
    }
    return mk(id);
  };
  var mk = function (id) {
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
    cache[id] = o;
    if (mine) {
      mine.push(id);
      // A detached root made here belongs to this realm's document (so another realm delegates).
      if (owners[id] === undefined && __d_parent(id) < 0) owners[id] = doc;
    }
    return o;
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
  // QSD2: tree roots. A node's root is its topmost ancestor: the document for an
  // attached node (or a node of a second QA18a document), the topmost element or
  // DocumentFragment for a detached one (a `<template>` content fragment is its own
  // root). `composed` is accepted and ignored until shadow roots exist (QSD3 adds
  // "a ShadowRoot's host" as the next step of the composed walk).
  var rootId = function (id) { var p; while ((p = __d_parent(id)) >= 0) id = p; return id; };
  // QSD3: the shadow-including root: a ShadowRoot continues at its host.
  var composedRootId = function (id) { var r = rootId(id), h; while ((h = __d_shadow_host(r)) >= 0) r = rootId(h); return r; };
  NodeProto.getRootNode = function (options) {
    return wrap(options && options.composed ? composedRootId(this.__id) : rootId(this.__id));
  };
  def(NodeProto, 'isConnected', function () { return __d_kind(composedRootId(this.__id)) === 9; });
  // QSD2: DOM spec "compare position". Different roots: DISCONNECTED |
  // IMPLEMENTATION_SPECIFIC plus a consistent PRECEDING/FOLLOWING (by node id).
  [['DISCONNECTED', 1], ['PRECEDING', 2], ['FOLLOWING', 4], ['CONTAINS', 8], ['CONTAINED_BY', 16], ['IMPLEMENTATION_SPECIFIC', 32]]
    .forEach(function (kv) { Node['DOCUMENT_POSITION_' + kv[0]] = NodeProto['DOCUMENT_POSITION_' + kv[0]] = kv[1]; });
  var chainOf = function (id) { var c = [id], p; while ((p = __d_parent(id)) >= 0) { c.push(p); id = p; } return c.reverse(); };
  NodeProto.compareDocumentPosition = function (other) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'compareDocumentPosition' on 'Node': 1 argument required, but only 0 present.");
    if (other == null || other.__id === undefined) throw new TypeError("Failed to execute 'compareDocumentPosition' on 'Node': parameter 1 is not of type 'Node'.");
    var a = this.__id, b = other.__id;
    if (a === b) return 0;
    var ca = chainOf(a), cb = chainOf(b);
    if (ca[0] !== cb[0]) return 1 | 32 | (b < a ? 2 : 4);
    var i = 0;
    while (i < ca.length && i < cb.length && ca[i] === cb[i]) i++;
    if (i === cb.length) return 8 | 2;   // other is an ancestor of this
    if (i === ca.length) return 16 | 4;  // other is a descendant of this
    var kids = __d_children(ca[i - 1], false);
    return kids.indexOf(cb[i]) < kids.indexOf(ca[i]) ? 2 : 4;
  };
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
  def(ElementProto, 'namespaceURI', function () { return __d_ns(this.__id) || null; });
  // QY45: createElementNS keeps `prefix:local` in the stored local name (QA14), so
  // outside XHTML the prefix is the part before ':'. Case is kept outside XHTML.
  var elQ = function (el) { var t = __d_tag(el.__id), i = t.indexOf(':'); return __d_ns(el.__id) === 'http://www.w3.org/1999/xhtml' || i <= 0 ? ['', t] : [t.slice(0, i), t.slice(i + 1)]; };
  def(ElementProto, 'localName', function () { var q = elQ(this)[1]; return __d_ns(this.__id) === 'http://www.w3.org/1999/xhtml' ? q.toLowerCase() : q; });
  def(ElementProto, 'prefix', function () { return elQ(this)[0] || null; });
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
  // QY45: public attribute access matches the QUALIFIED name (`xlink:href`), per
  // DOM; HTML-namespace elements lowercase the name, others keep its case.
  ElementProto.getAttribute = function (n) { var v = __d_getq(this.__id, String(n)); return v == null ? null : v; };
  ElementProto.hasAttribute = function (n) { return __d_getq(this.__id, String(n)) != null; };
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
  // QY29: `data` lives on CharacterData only (never Node/Element: polymer-resin
  // treats `'data' in element` as a native property and blanks bound values).
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
  // QY29: `disabled`/`required`/`checked` are defined on the specific form
  // interfaces further down, not on every element.
  reflectBool('autofocus', 'autofocus');
  // QB5: focus/blur record a request the App applies in `js_sync`.
  HTMLElement.prototype.focus = function () { __d_focus(this.__id, true); };
  HTMLElement.prototype.blur = function () { __d_focus(this.__id, false); };
  // QJ9: window.open/close and alert/confirm/prompt: no tabs, no dialogs. Each call
  // logs one `not implemented` line (argument truncated) and returns the inert value.
  var qj9 = function (what, arg) {
    var t = String(arg === undefined ? '' : arg);
    if (t.length > 120) t = t.slice(0, 120) + '...';
    __scratchy_err('scratchy: js: not implemented: ' + what + (t ? ' ' + JSON.stringify(t) : ''));
  };
  g.open = function open(url, target, features) {
    // QF11: no tabs, so the self-like targets navigate this page.
    if (url !== undefined && url !== '' && /^(_self|_top|_parent)$/i.test(String(target === undefined ? '' : target))) {
      g.__loc_nav(url, false);
      return null;
    }
    qj9('window.open', url === undefined ? '' : url + (target ? ' target=' + target : '') + (features ? ' features=' + features : ''));
    return null;
  };
  g.close = function close() { qj9('window.close'); };
  g.alert = function alert(m) { qj9('window.alert', m); };
  g.confirm = function confirm(m) { qj9('window.confirm', m); return false; };
  g.prompt = function prompt(m) { qj9('window.prompt', m); return null; };
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
    if (g.__scratchy_ioNudge) g.__scratchy_ioNudge();
  };
  g.scrollBy = function () {
    var a = scrollArgs(arguments[0], arguments[1]);
    __d_scroll(a[0] === a[0] ? a[0] : 0, a[1] === a[1] ? a[1] : 0, true);
    if (a[0] === a[0]) g.scrollX = g.pageXOffset = Math.max(0, g.scrollX + a[0]);
    if (a[1] === a[1]) g.scrollY = g.pageYOffset = Math.max(0, g.scrollY + a[1]);
    if (g.__scratchy_ioNudge) g.__scratchy_ioNudge();
  };
  ElementProto.setAttribute = function (n, v) { checkName('setAttribute', String(n)); __d_setq(this.__id, String(n), String(v)); };
  ElementProto.removeAttribute = function (n) { __d_rmq(this.__id, String(n)); };
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
  var classes = function (el) { var v = __d_attr(el.__id, 'class'); return (v == null ? '' : v).split(/\s+/).filter(Boolean); };
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
  // QY53l: CSSOM setProperty updates an existing declaration IN PLACE (only a new property is
  // appended), so writing the value a property already has leaves the `style` attribute
  // byte-identical: the write is a no-op (no geometry generation bump, no forced layout).
  var styleSet = function (el, p, v) {
    var d = [], at = -1;
    v = v == null ? '' : String(v).trim();
    readStyle(el).forEach(function (x) {
      if (x[0] !== p) d.push(x);
      else if (at < 0 && v) { at = d.length; d.push([p, v]); }
    });
    if (v && at < 0) d.push([p, v]);
    writeStyle(el, d);
  };
  var styles = {};
  // QY23: [PutForwards=cssText]: `el.style = "..."` writes cssText.
  var styleSetter = function (v) {
    if (v == null || v === '') __d_rmattr(this.__id, 'style'); else __d_setattr(this.__id, 'style', String(v));
  };
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
  Object.defineProperty(ElementProto, 'style', { get: Object.getOwnPropertyDescriptor(ElementProto, 'style').get, set: styleSetter, configurable: true, enumerable: true });
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
  // QY61: insertAdjacentElement / insertAdjacentText / insertAdjacentHTML
  // (skins.vector.js movePinnableElement died on the missing method).
  var adjacent = function (self, where, node, fn) {
    var w = String(where).toLowerCase(), p = self.parentNode, own = moBegin();
    try {
      if (w === 'beforebegin') { if (!p) return null; p.insertBefore(node, self); }
      else if (w === 'afterbegin') self.insertBefore(node, self.firstChild);
      else if (w === 'beforeend') self.appendChild(node);
      else if (w === 'afterend') { if (!p) return null; p.insertBefore(node, self.nextSibling); }
      else throw herr("Failed to execute '" + fn + "' on 'Element': The value provided ('" + where + "') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or 'afterEnd'.", 'SyntaxError');
    } finally { moEnd(own); }
    return node;
  };
  ElementProto.insertAdjacentElement = function (where, el) {
    if (!el || el.__id === undefined) throw new TypeError("Failed to execute 'insertAdjacentElement' on 'Element': parameter 2 is not of type 'Element'.");
    return adjacent(this, where, el, 'insertAdjacentElement');
  };
  ElementProto.insertAdjacentText = function (where, text) {
    adjacent(this, where, doc.createTextNode(String(text)), 'insertAdjacentText');
  };
  ElementProto.insertAdjacentHTML = function (where, html) {
    var t = doc.createElement('template');
    t.innerHTML = String(html);
    adjacent(this, where, t.content, 'insertAdjacentHTML');
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
    // QPT1c: the adopter is recorded (not just forgotten): the node may be another realm's wrapper.
    var od = xr.cn ? ownerDocId(n.__id) : -1;
    __d_remove(n.__id); owners[n.__id] = doc;
    if (xr.cn && n.nodeType === 1 && (od < 0 ? 0 : od) !== doc.__id) ceAdopt([n.__id], [od], doc.__id);
    return n;
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
    var PE = g.PointerEvent;
    var C = { mousemove: MouseEvent, mouseenter: MouseEvent, mouseleave: MouseEvent, pointermove: PE, pointerover: PE, pointerout: PE,
      pointerenter: PE, pointerleave: PE, click: MouseEvent, mousedown: MouseEvent, mouseup: MouseEvent, keydown: KeyboardEvent, keypress: KeyboardEvent,
      keyup: KeyboardEvent, mouseover: MouseEvent, mouseout: MouseEvent, focus: FocusEvent, blur: FocusEvent, focusin: FocusEvent, focusout: FocusEvent,
      animationstart: AnimationEvent, animationend: AnimationEvent, animationiteration: AnimationEvent, animationcancel: AnimationEvent }[type] || Event;
    if (type === '__dialogclose') { t.close(init && init.rv !== undefined ? init.rv : undefined); return true; }
    if (type === 'submit') C = SubmitEvent;
    if (init && init.submitterId !== undefined) init.submitter = wrap(init.submitterId);
    if (init && init.relatedTargetId !== undefined) init.relatedTarget = wrap(init.relatedTargetId);
    if (g.__media_activate && (type === 'click' || type === 'keydown' || type === 'mousedown' || type === 'mouseup' || type === 'keyup')) g.__media_activate();
    var ev = new C(type, init);
    ev.isTrusted = true;
    var ok = __scratchy_dispatch(t, ev);
    // QM1n2: default action of a trusted click on `<video controls>`: toggle playback
    // (not a page-visible listener; `el.click()` goes through HTMLElement.prototype.click).
    if (ok && type === 'click' && t.localName === 'video' && t.hasAttribute('controls')) {
      if (t.paused) { var pr = t.play(); if (pr && pr.catch) pr.catch(function () {}); } else t.pause();
    }
    return ok;
  } });
  // QM1g2d: actions of scratchy's own controls bar / keys, through the element's public API
  // (so page listeners see play/pause/seeked/volumechange); a user gesture (D5).
  Object.defineProperty(g, '__scratchy_media_ui', { value: function (id, op, arg) {
    var v = wrap(id);
    if (!v || (v.localName !== 'video' && v.localName !== 'audio')) return false;
    if (g.__media_activate) g.__media_activate();
    if (op === 'toggle') { if (v.paused) { var pr = v.play(); if (pr && pr.catch) pr.catch(function () {}); } else v.pause(); }
    else if (op === 'seek') v.currentTime = arg;
    else if (op === 'mute') v.muted = !!arg;
    else if (op === 'volume') { v.volume = Math.max(0, Math.min(1, arg)); if (arg > 0 && v.muted) v.muted = false; }
    return true;
  } });
  // Q33d1: `el.click()`: a synthetic (untrusted) click, then the activation
  // behavior unless a handler cancelled it. Checkbox/radio use the live state
  // (Q30c), toggled before dispatch and reverted on cancel (legacy
  // pre-activation), exactly like a real user click in the App.
  var clicking = false;
  HTMLElement.prototype.click = function () {
    var tag = this.localName;
    if (clicking || ((tag === 'input' || tag === 'button' || tag === 'select' || tag === 'textarea') && this.disabled)) return;
    clicking = true;
    try {
      var type = tag === 'input' ? this.type : '';
      var undo = null;
      if (type === 'checkbox') {
        var was = __d_checked(this.__id), wasInd = __d_indet(this.__id);
        __d_setindet(this.__id, false);
        __d_setchecked(this.__id, !was);
        undo = function (el) { __d_setchecked(el.__id, was); __d_setindet(el.__id, wasInd); };
      } else if (type === 'radio' && !__d_checked(this.__id)) {
        var pid = __d_radiopeer(this.__id), prev = pid >= 0 ? wrap(pid) : null;
        __d_setchecked(this.__id, true);
        undo = function (el) { __d_setchecked(el.__id, false); if (prev) __d_setchecked(prev.__id, true); };
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
      if ((tag === 'button' && this.type === 'reset') || (tag === 'input' && type === 'reset')) { var rf = formOf(this); if (rf) rf.reset(); }
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
  def(doc, 'forms', function () { return coll(this.querySelectorAll('form')); });
  def(doc, 'images', function () { return coll(this.querySelectorAll('img')); });
  def(doc, 'links', function () { return coll(this.querySelectorAll('a[href], area[href]')); });
  def(doc, 'anchors', function () { return coll(this.querySelectorAll('a[name]')); });
  def(doc, 'scripts', function () { return coll(this.querySelectorAll('script')); });
  def(doc, 'embeds', function () { return coll(this.querySelectorAll('embed')); });
  def(doc, 'plugins', function () { return coll(this.querySelectorAll('embed')); });

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
  FP.submit = function () { __d_submit(this.__id, -1); };
  FP.requestSubmit = function (submitter) {
    if (submitter !== undefined && submitter !== null && (!submitter || submitter.form !== this)) {
      throw herr('The specified element is not a submit button of this form.', 'NotFoundError');
    }
    var ev = new Event('submit', { bubbles: true, cancelable: true });
    ev.submitter = submitter || null;
    if (__scratchy_dispatch(this, ev)) __d_submit(this.__id, submitter ? submitter.__id : -1);
  };
  // Q30c: reset restores the checkbox/radio defaults (text values wait for Q30d).
  FP.reset = function () {
    var ev = new Event('reset', { bubbles: true, cancelable: true });
    if (__scratchy_dispatch(this, ev)) __d_resetform(this.__id);
  };
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
  // Q30d: text-like inputs read/write the live value (forms_text.rs); other
  // types keep a dirty value on the wrapper.
  var TEXTY = ['text', 'search', 'tel', 'url', 'email', 'password', 'number'];
  var isTexty = function (el) { return TEXTY.indexOf(el.type) >= 0; };
  defs(IP, 'value', function () { return isTexty(this) ? __d_tvalue(this.__id) : (this.__v !== undefined ? this.__v : ga(this, 'value')); },
    function (v) { v = v === null ? '' : String(v); if (isTexty(this)) __d_settvalue(this.__id, v); else this.__v = v; });
  var selProps = function (P, ok) {
    defs(P, 'selectionStart', function () { return ok(this) ? __d_selget(this.__id, false) : null; },
      function (v) { if (ok(this)) __d_selset(this.__id, Math.floor(Number(v)) || 0, __d_selget(this.__id, true)); });
    defs(P, 'selectionEnd', function () { return ok(this) ? __d_selget(this.__id, true) : null; },
      function (v) { if (ok(this)) { var e = Math.floor(Number(v)) || 0; __d_selset(this.__id, Math.min(__d_selget(this.__id, false), e), e); } });
    defs(P, 'selectionDirection', function () { return ok(this) ? 'forward' : null; }, function () {});
    P.setSelectionRange = function (s, e) { if (ok(this)) __d_selset(this.__id, Math.max(0, Math.floor(Number(s)) || 0), Math.max(0, Math.floor(Number(e)) || 0)); };
    P.select = function () { if (ok(this)) __d_selset(this.__id, 0, this.value.length); };
  };
  selProps(IP, isTexty);
  defs(IP, 'defaultValue', function () { return ga(this, 'value'); }, function (v) { __d_setattr(this.__id, 'value', String(v)); });
  defs(IP, 'defaultChecked', function () { return __d_attr(this.__id, 'checked') != null; },
    function (v) { if (v) __d_setattr(this.__id, 'checked', ''); else __d_rmattr(this.__id, 'checked'); });
  // Q30c: `checked`/`indeterminate` are live state, not attributes.
  defs(IP, 'checked', function () { return __d_checked(this.__id); }, function (v) { __d_setchecked(this.__id, !!v); });
  defs(IP, 'indeterminate', function () { return __d_indet(this.__id); }, function (v) { __d_setindet(this.__id, !!v); });
  ['readOnly', 'multiple'].forEach(function (p) { rBool(IP, p, p.toLowerCase()); });
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
  defs(TAP, 'value', function () { return __d_tvalue(this.__id); }, function (v) { __d_settvalue(this.__id, v === null ? '' : String(v)); });
  selProps(TAP, function () { return true; });
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
  defs(SEP, 'length', function () { return optionsOf(this).length; }, function (v) {
    v = Math.max(0, Math.floor(Number(v)) || 0);
    var o = optionsOf(this);
    while (o.length > v) { var x = o.pop(); x.parentNode.removeChild(x); }
    while (o.length < v) { this.appendChild(doc.createElement('option')); o.push(0); }
  });
  def(SEP, 'type', function () { return this.multiple ? 'select-multiple' : 'select-one'; });
  rBool(SEP, 'multiple', 'multiple'); rStr(SEP, 'name', 'name');
  rInt(SEP, 'size', 'size', 0, 1);
  // Q30g: the selection is live state (forms_select.rs), not the `selected` attributes.
  defs(SEP, 'selectedIndex', function () { return __d_selidx(this.__id); },
    function (v) { v = Math.floor(Number(v)); __d_setselidx(this.__id, isNaN(v) ? -1 : v); });
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
  defs(OP, 'selected', function () { return __d_optsel(this.__id); }, function (v) { __d_setoptsel(this.__id, !!v); });
  rBool(OP, 'defaultSelected', 'selected');
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
  ['input', 'select', 'textarea', 'button', 'fieldset', 'option', 'optgroup'].forEach(function (t) {
    rBool(tagProto[t], 'disabled', 'disabled');
  });
  ['input', 'select', 'textarea'].forEach(function (t) { rBool(tagProto[t], 'required', 'required'); });
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
  xr.byDoc[0] = mk;
  // `configurable` only so that an iframe realm can rebind it once (`__d_bind_doc`).
  Object.defineProperty(doc, '__id', { value: 0, configurable: true });
  // QPT1a (R53 D2): an iframe realm (a context from the startup snapshot, whose `document` is id 0
  // here) binds its `document` to the iframe's second document `id`; `base` = the creator's base URL
  // (an about:blank document's fallback base). Builds <html><head><body> in an empty document.
  var baseFallback = null, frameDoc = false, frameOnLoad = null;
  Object.defineProperty(g, '__d_bind_doc', { value: function (id, base, shared) {
    delete g.__d_bind_doc;
    delete cache[doc.__id];
    if (shared) {
      // QPT1c: the page-wide wrapper cache, owners and realm table (this realm's own are dropped).
      cache = shared.cache; owners = shared.owners; xr = shared.xr;
      mine = []; myDocs = [id]; xr.byDoc[id] = mk; xr.n++;
      xr.hk.push(hookInstall); if (xr.mo.n > 0 || xr.cn > 0) hookInstall();
      xr.prep[id] = function (sid) { sPrepare(sid); };
      frameDoc = true; frameOnLoad = shared.onLoad || null;
    }
    cache[id] = doc;
    Object.defineProperty(doc, '__id', { value: id, configurable: false });
    baseFallback = base || null;
    Object.defineProperty(doc, 'title', titleDesc);
    if (!__d_children(id, false).length) {
      var html = doc.createElement('html'); doc.appendChild(html);
      html.appendChild(doc.createElement('head')); html.appendChild(doc.createElement('body'));
    }
  }, configurable: true });
  Object.defineProperty(doc, 'nodeType', { value: 9 });
  Object.defineProperty(doc, 'nodeName', { value: '#document' });
  queries(doc);
  doc.hasChildNodes = NodeProto.hasChildNodes;
  doc.contains = NodeProto.contains;
  def(doc, 'childNodes', Object.getOwnPropertyDescriptor(NodeProto, 'childNodes').get);
  doc.getElementById = function (id) { return wrap(__d_byid(String(id), this.__id)); };
  def(doc, 'documentElement', function () { return wrap(__d_special(0, this.__id)); });
  // QY40: `document.all`, an HTMLAllCollection with [[IsHTMLDDA]] (marked from Rust
  // right after the prelude: falsy, typeof 'undefined', == null). A Proxy over a
  // function so it is callable; every read lists the elements afresh (live).
  var allList = function () { return wrapAll(q(doc.__id, '*', false)); };
  var allItem = function (i) { var l = allList(); i = Number(i); return (i >= 0 && i < l.length && i % 1 === 0 ? l[i] : null); };
  var allNamed = function (n) {
    n = String(n);
    var l = allList().filter(function (e) {
      return e.getAttribute('id') === n || (/^(a|applet|button|embed|form|iframe|img|input|map|meta|object|select|textarea|frame|frameset)$/i.test(e.localName) && e.getAttribute('name') === n);
    });
    return l.length === 0 ? null : l.length === 1 ? l[0] : l;
  };
  var isIdx = function (k) { return typeof k === 'string' && /^\d+$/.test(k); };
  var allColl = new Proxy(function all() {}, {
    apply: function (t, th, a) { return a.length ? (typeof a[0] === 'string' && !isIdx(a[0]) ? allNamed(a[0]) : allItem(a[0])) : null; },
    get: function (t, k) {
      if (k === 'length') return allList().length;
      if (k === 'item') return function (i) { return typeof i === 'string' && !isIdx(i) ? allNamed(i) : allItem(i); };
      if (k === 'namedItem') return allNamed;
      if (k === Symbol.iterator) return function () { return allList()[Symbol.iterator](); };
      if (k === Symbol.toStringTag) return 'HTMLAllCollection';
      if (isIdx(k)) return allItem(k) || undefined;
      if (typeof k === 'string') { var n = allNamed(k); if (n !== null) return n; }
      return Reflect.get(t, k);
    },
    has: function (t, k) { return isIdx(k) ? allItem(k) !== null : k in t || k === 'length' || k === 'item' || k === 'namedItem'; },
  });
  Object.defineProperty(doc, 'all', { get: function () { return allColl; }, configurable: true, enumerable: true });
  Object.defineProperty(g, '__d_all', { value: allColl, configurable: true });
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
  // QPT1f (R53 D9): an iframe realm's document (about:blank, about:srcdoc) also supports
  // `open()`/`write()`/`close()` outside its own parser scripts: `open()` empties it, `write()`
  // collects markup (opening it first if needed), `close()` parses the markup as a new document and
  // runs it like a srcdoc load (scripts in order, DOMContentLoaded, load, the <iframe>'s load).
  // Deviation: nothing is parsed or run before `close()` (Chrome parses as it is written).
  var wBuf = null;
  function dwrite(nl) {
    return function () {
      var s = Array.prototype.join.call(arguments, '') + (nl ? '\n' : '');
      if (frameDoc && this === doc && (!curScript || wlast < 0)) {
        if (g.closed) return;
        if (wBuf === null) doc.open();
        wBuf += s;
        return;
      }
      if (!curScript || wlast < 0) { console.warn('document.' + (nl ? 'writeln' : 'write') + ' after page load ignored'); return; }
      var r = __d_write(wlast, s);
      if (r >= 0) wlast = r;
    };
  }
  doc.write = dwrite(false);
  doc.writeln = dwrite(true);
  doc.open = function () {
    if (!frameDoc || this !== doc || (curScript && wlast >= 0) || g.closed) return doc;
    var kids = __d_children(doc.__id, false);
    for (var i = 0; i < kids.length; i++) __d_remove(kids[i]);
    wBuf = ''; doc.readyState = 'loading';
    return doc;
  };
  doc.close = function () {
    if (!frameDoc || this !== doc || wBuf === null) return;
    var h = wBuf; wBuf = null;
    frameLoad(h, false);
  };
  def(doc, 'currentScript', function () { return curScript; });
  def(doc, 'head', function () { return wrap(__d_special(1, this.__id)); });
  def(doc, 'body', function () { return wrap(__d_special(2, this.__id)); });
  // QJ8: the focused element, else body (null only without a body).
  def(doc, 'activeElement', function () {
    var a = __d_active(this.__id);
    return a >= 0 ? wrap(a) : wrap(__d_special(2, this.__id));
  });
  doc.hasFocus = function () { return true; };
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
    xr.byDoc[id] = mk;
    if (myDocs) myDocs.push(id);
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
    Object.defineProperty(d, 'title', titleDesc);
    return d;
  };
  // A second document's `title` reads its <title> (also an iframe realm's document: `__d_bind_doc`).
  var titleDesc = { configurable: true, enumerable: true,
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
    } };
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
  // QF9a: DOMParser. The result is an inert second document (scripts never
  // run, nothing loads, defaultView null, URL about:blank).
  var parseXmlInto = function (d, text) {
    var r = __d_load_xml(d.__id, text);
    if (r === '') return true;
    var p = r.split('\n');
    while (d.firstChild) d.removeChild(d.firstChild);
    var NS = 'http://www.mozilla.org/newlayout/xml/parsererror.xml';
    var pe = d.createElementNS(NS, 'parsererror');
    pe.appendChild(d.createTextNode('XML Parsing Error: ' + p[0] + '\nLocation: ' + d.URL + '\nLine Number ' + p[1] + ', Column ' + p[2] + ':'));
    var st = d.createElementNS(NS, 'sourcetext'); st.appendChild(d.createTextNode(String(text).split('\n')[(+p[1] || 1) - 1] || ''));
    pe.appendChild(st); d.appendChild(pe);
    return false;
  };
  g.__parseDocument = function (text, type) {
    text = String(text);
    if (type === 'text/html') {
      var h = makeDoc(true, 'text/html'); __d_parse_html(h.__id, text); return h;
    }
    var d = makeDoc(false, type);
    parseXmlInto(d, text);
    return d;
  };
  var DOMParser = g.DOMParser = function DOMParser() {
    if (!(this instanceof DOMParser)) throw new TypeError("Failed to construct 'DOMParser': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
  };
  DOMParser.prototype.parseFromString = function parseFromString(str, type) {
    if (arguments.length < 2) throw new TypeError("Failed to execute 'parseFromString' on 'DOMParser': 2 arguments required, but only " + arguments.length + " present.");
    type = String(type);
    if (['text/html', 'text/xml', 'application/xml', 'application/xhtml+xml', 'image/svg+xml'].indexOf(type) < 0)
      throw new TypeError("Failed to execute 'parseFromString' on 'DOMParser': The provided value '" + type + "' is not a valid enum value of type DOMParserSupportedType.");
    return g.__parseDocument(str, type);
  };
  Object.defineProperty(DOMParser.prototype, Symbol.toStringTag, { value: 'DOMParser', configurable: true });
  Object.defineProperty(g, '__parseDocument', { enumerable: false });
  // QA18b: `iframe.contentDocument`/`contentWindow` for iframes with an
  // empty, about:blank or data: src: a fresh empty second document (made
  // lazily, kept per iframe element), a minimal window-like object, and an
  // asynchronous `load` event after insertion. Network iframes give null.
  var IFP = mkProto('HTMLIFrameElement', ['iframe']);
  var frames = {};
  // (QPT1a: "in this realm's document", which is the page in the page realm.)
  var inPage = function (n) { for (; n && n.parentNode; n = n.parentNode); return !!n && n.__id === doc.__id; };
  var frameSrc = function (el) { return (__d_attr(el.__id, 'src') || '').replace(/^\s+|\s+$/g, ''); };
  // QPT1f (R53 D9): `srcdoc` wins over `src` (an about:srcdoc document, same origin as the parent).
  var frameSrcdoc = function (el) { return __d_attr(el.__id, 'srcdoc'); };
  var frameSrcOk = function (el) {
    if (frameSrcdoc(el) != null) return true;
    var s = frameSrc(el);
    return s === '' || /^about:blank/i.test(s) || /^data:/i.test(s) || __d_iframe_page(el.__id).length > 0;
  };
  // QPT1a (R53 D1-D4): an iframe with an empty or about:blank src, or a pre-fetched same-origin
  // one (QA18b1), gets a real window: its own V8 realm (from the startup snapshot, or without it a
  // fresh context with the preludes replayed, QPT1e), whose `document` is the iframe's second
  // document. `data:` iframes (D4) keep the stand-in object below.
  var capNoted = false, nFrames = 0;
  var frameOf = function (el) {
    if (!inPage(el) || !frameSrcOk(el)) return null;
    var f = frames[el.__id];
    if (f) return f;
    var sd = frameSrcdoc(el) != null, pg = sd ? [] : __d_iframe_page(el.__id), href = sd ? 'about:srcdoc' : 'about:blank';
    if (sd || !/^data:/i.test(frameSrc(el))) {
      var w = g.__scratchy_realm_new();
      if (w) {
        var did = __d_newdoc();
        if (pg.length) { __d_load_html(did, pg[1]); href = pg[0]; }
        var dim = function (a, dflt) { var v = parseInt(__d_attr(el.__id, a) || '', 10); return v >= 0 ? v : dflt; };
        // QPT1f: the realm reports its window `load` (end of a srcdoc load or of document.close()),
        // and the parent then fires `load` on the <iframe> (while it still shows that window).
        var fr = null, onLoad = function () { if (fr && frames[el.__id] === fr && inPage(el)) el.dispatchEvent(new g.Event('load')); };
        w.__scratchy_realm_init({ docId: did, dom: { cache: cache, owners: owners, xr: xr, onLoad: onLoad }, href: href, base: pg.length ? null : doc.baseURI, parent: g, top: g.top,
          frameElement: el, name: __d_attr(el.__id, 'name') || '', onName: function () { syncWin(); }, width: dim('width', 300), height: dim('height', 150) });
        nFrames++;
        return (frames[el.__id] = fr = { doc: w.document, win: w, el: el, realm: true, loaded: false });
      }
      if (w === null) {
        // QPT1b (R53 D6): over the cap of live realms the extra frame gets the stand-in (an older
        // frame's window is never pulled from under its scripts), and this is logged once.
        if (!capNoted) {
          capNoted = true;
          __scratchy_log('warn', 'scratchy: too many iframe windows (64): the extra frames get a stand-in object');
        }
      }
    }
    var d = makeDoc(true, 'text/html');
    if (pg.length) {
      // QA18b1: pre-fetched same-origin document; scripts in it never run.
      __d_load_html(d.__id, pg[1]); href = pg[0];
      ['URL', 'documentURI', 'baseURI'].forEach(function (k) { Object.defineProperty(d, k, { value: href, writable: true, configurable: true, enumerable: true }); });
    } else {
      var html = d.createElement('html'); d.appendChild(html);
      html.appendChild(d.createElement('head')); html.appendChild(d.createElement('body'));
    }
    var win = Object.create(null);
    var props = { document: d, window: win, self: win, frames: win, parent: g, top: g, frameElement: el, name: __d_attr(el.__id, 'name') || '', length: 0, closed: false,
      location: { href: href, toString: function () { return href; } } };
    Object.keys(props).forEach(function (k) { Object.defineProperty(win, k, { value: props[k], writable: true, configurable: true, enumerable: true }); });
    win.getComputedStyle = function (e, p) { return g.getComputedStyle(e, p); };
    Object.defineProperty(d, 'defaultView', { get: function () { return win; }, configurable: true, enumerable: true });
    nFrames++;
    return (frames[el.__id] = { doc: d, win: win, el: el, realm: false });
  };
  // QPT1b (R53 D6): a frame's window lives while its iframe is connected and keeps its src. Dropping
  // cancels the realm's timers, requests and rejection reports, drops the nested frames, and detaches
  // the global (`contentWindow` of a removed iframe is null; a re-inserted one gets a NEW realm).
  var dropFrame = function (id) {
    var f = frames[id];
    if (!f) return;
    delete frames[id]; nFrames--;
    if (!f.realm) f.win.closed = true;
    if (f.realm) {
      try { f.win.__scratchy_realm_release(); } catch (e) {}
      g.__scratchy_realm_drop(f.win);
    }
  };
  Object.defineProperty(g, '__d_frames_drop_all', { value: function () { for (var k in frames) dropFrame(k); } });
  // QPT1c (R53 D7): a dropped realm takes out of the page-wide tables what is still its own: the
  // wrappers of nodes in its documents (or detached ones it made), its documents, and the owners
  // entries naming them, so nothing keeps its context alive. A node moved into another document
  // keeps its wrapper (and that wrapper keeps the realm's prototypes, as in Chrome).
  Object.defineProperty(g, '__d_realm_purge', { value: function () {
    if (!mine) return;
    var docs = {}, i, id;
    for (i = 0; i < myDocs.length; i++) docs[myDocs[i]] = 1;
    for (i = 0; i < mine.length; i++) {
      var d = ownerDocId(mine[i]);
      if (d < 0 || docs[d]) delete cache[mine[i]];
    }
    for (id in owners) if (docs[owners[id].__id]) delete owners[id];
    for (i = 0; i < myDocs.length; i++) { delete cache[myDocs[i]]; if (xr.byDoc[myDocs[i]] === mk) delete xr.byDoc[myDocs[i]]; delete xr.prep[myDocs[i]]; }
    __d_ce_drop(doc.__id);
    if (ceOn) { for (id in xr.cs) if (xr.cs[id] === ceApi) delete xr.cs[id]; delete xr.ce[doc.__id]; xr.cn--; ceOn = false; }
    if (moSts) { for (i = 0; i < moSts.length; i++) { moDisc(moSts[i]); moSts[i].pending = false; } moSts = null; }
    moPending = [];
    i = xr.hk.indexOf(hookInstall); if (i >= 0) xr.hk.splice(i, 1);
    xr.n--; mine = myDocs = null;
  } });
  var sweepFrames = function () {
    if (nFrames) for (var k in frames) if (!inPage(frames[k].el)) dropFrame(k);
    if (ixN) syncWin();
  };
  // QPT1d (R53 D8): the window's child list. `window.length`, `window[i]` and `window[name]` follow the
  // connected iframes of this realm's document that have a window, in document order (Chrome's order; a
  // window made lazily by `contentWindow` still counts). V8 has no global interceptor here, so the
  // indexed/named properties are getters (each resolves live, creating the window on first use) that
  // `syncWin` adds/removes when an iframe is inserted or removed or its src/srcdoc/name changes.
  // Deviation: an iframe that enters through a subtree (appendChild of a wrapper, innerHTML without the
  // text `<iframe`) is counted by `length` at once but gets its `window[i]` property at the next sync
  // (a `length` read, a `contentWindow` read, or the next iframe change).
  var ixN = 0, nmSet = {};
  var frameList = function () {
    var out = [], l = doc.getElementsByTagName('iframe');
    for (var i = 0; i < l.length; i++) if (frameSrcOk(l[i])) out.push(l[i]);
    return out;
  };
  var frameName = function (el) { var f = frames[el.__id]; return String(f ? f.win.name : (__d_attr(el.__id, 'name') || '')); };
  var winAt = function (i) { var el = frameList()[i]; var f = el && frameOf(el); return f ? f.win : undefined; };
  var winNamed = function (nm) {
    var l = frameList();
    for (var i = 0; i < l.length; i++) if (frameName(l[i]) === nm) { var f = frameOf(l[i]); return f ? f.win : undefined; }
    return undefined;
  };
  var syncWin = function () {
    var l = frameList(), n = l.length, i, nm;
    for (i = ixN; i < n; i++) (function (k) {
      Object.defineProperty(g, k, { get: function () { return winAt(k); }, set: function (v) { Object.defineProperty(g, k, { value: v, writable: true, configurable: true, enumerable: true }); }, configurable: true, enumerable: true });
    })(i);
    for (i = n; i < ixN; i++) delete g[i];
    ixN = n;
    var want = {};
    for (i = 0; i < n; i++) { nm = frameName(l[i]); if (nm !== '' && !/^\d+$/.test(nm) && (nmSet[nm] || !(nm in g))) want[nm] = 1; }
    for (nm in nmSet) if (!want[nm]) { delete g[nm]; delete nmSet[nm]; }
    for (nm in want) if (!nmSet[nm]) (function (k) {
      Object.defineProperty(g, k, { get: function () { return winNamed(k); }, set: function (v) { delete nmSet[k]; Object.defineProperty(g, k, { value: v, writable: true, configurable: true, enumerable: false }); }, configurable: true, enumerable: false });
      nmSet[k] = 1;
    })(nm);
  };
  Object.defineProperty(g, 'length', { get: function () { var n = frameList().length; if (n !== ixN) syncWin(); return n; }, set: function () {}, configurable: true, enumerable: true });
  var sfRem = __d_remove, sfHtml = __d_sethtml, sfTxt = __d_settext, sfSet = __d_setattr, sfRm = __d_rmattr, sfSq = __d_setq, sfRq = __d_rmq;
  var isSrc = function (n) { n = String(n).toLowerCase(); return n === 'src' || n === 'srcdoc'; };
  // QPT1c: while iframe realms exist, a removed node remembers the document it left (its wrapper
  // may come from another realm, whose `ownerDocument` fallback is that realm's document).
  __d_remove = function (id) {
    if (xr.n) { var r = __d_root(id); if (r !== id && __d_kind(r) === 9) owners[id] = wrap(r); }
    sfRem(id); sweepFrames();
  };
  // QPT1c: a node with a recorded owner that is inserted under a document now belongs to that
  // document (so it still does after a later removal; e.g. an iframe node moved into the page).
  var sfIns = __d_insert;
  __d_insert = function (p, c, b) {
    var ok = sfIns(p, c, b);
    if (ok && owners[c] !== undefined) { var r = __d_root(p); if (__d_kind(r) === 9) owners[c] = wrap(r); }
    return ok;
  };
  __d_sethtml = function (id, h) {
    sfHtml(id, h); sweepFrames();
    if (!ixN && /<iframe/i.test(h)) syncWin();
    // QPT1f: srcdoc iframes made by innerHTML load (as tasks).
    if (/srcdoc/i.test(h) && /<iframe/i.test(h)) {
      var l = __d_query(id, 'iframe[srcdoc]', false) || [];
      for (var i = 0; i < l.length; i++) if (!frames[l[i]] && inPage(wrap(l[i]))) loadFrameLater(wrap(l[i]));
    }
  };
  __d_settext = function (id, t) { sfTxt(id, t); sweepFrames(); };
  // QPT1d: a src/srcdoc/name change of an iframe changes the window list (and `window[name]`).
  var ifrAttr = function (id, n) {
    n = String(n).toLowerCase();
    var s = n === 'src' || n === 'srcdoc', nm = n === 'name';
    if (!s && !nm) return;
    if (__d_tag(id).toLowerCase() !== 'iframe') return;
    if (s && nFrames) dropFrame(id);
    if (s && __d_attr(id, 'srcdoc') != null && inPage(wrap(id))) loadFrameLater(wrap(id));
    else if (nm && frames[id] && frames[id].realm && inPage(frames[id].el)) frames[id].win.name = __d_attr(id, 'name') || '';
    if (inPage(wrap(id))) syncWin();
  };
  __d_setattr = function (id, n, v) { sfSet(id, n, v); ifrAttr(id, n); };
  __d_rmattr = function (id, n) { sfRm(id, n); ifrAttr(id, n); };
  __d_setq = function (id, n, v) { sfSq(id, n, v); ifrAttr(id, n); };
  __d_rmq = function (id, n) { sfRq(id, n); ifrAttr(id, n); };
  def(IFP, 'contentDocument', function () { var f = frameOf(this); return f ? f.doc : null; });
  def(IFP, 'contentWindow', function () { var f = frameOf(this); return f ? f.win : null; });
  IFP.getSVGDocument = function () { return this.contentDocument; };
  rStr(IFP, 'src', 'src'); rStr(IFP, 'srcdoc', 'srcdoc'); rStr(IFP, 'name', 'name'); rStr(IFP, 'width', 'width'); rStr(IFP, 'height', 'height');
  // QPT1f (R53 D9): load an iframe's `srcdoc` into its window: the realm parses it, runs its scripts
  // in document order (realm entered), fires DOMContentLoaded and its window `load`, then the parent
  // fires `load` on the <iframe> (`onLoad` above). Once per window; a removed iframe never loads.
  // `top`: nothing else on the JS stack (a task), so each script gets the page-load CPU budget.
  var loadFrame = function (el, top) {
    if (!inPage(el)) return;
    var html = frameSrcdoc(el);
    if (html == null) return;
    var f = frameOf(el);
    if (!f || f.loaded) return;
    f.loaded = true;
    if (f.realm) { f.win.__scratchy_frame_load(html, top); return; }
    // The stand-in (no snapshot, QPT1e): the document only, no scripts.
    __d_load_html(f.doc.__id, html);
    el.dispatchEvent(new g.Event('load'));
  };
  var loadFrameLater = function (el) { g.setTimeout(function () { loadFrame(el, true); }, 0); };
  // The srcdoc iframes already in this realm's document (page load; a frame document's parse).
  var bootFrames = function () {
    var l = __d_query(doc.__id, 'iframe[srcdoc]', false) || [];
    for (var i = 0; i < l.length; i++) loadFrame(wrap(l[i]), true);
  };
  Object.defineProperty(g, '__scratchy_frames_boot', { value: function () { bootFrames(); } });
  afterIns = function (c) {
    if (c.nodeType !== 1 || __d_tag(c.__id).toLowerCase() !== 'iframe' || !inPage(c)) return;
    syncWin();
    if (frameSrcdoc(c) != null) { loadFrameLater(c); return; }
    g.setTimeout(function () { if (inPage(c) && frameSrcOk(c)) c.dispatchEvent(new Event('load')); }, 0);
  };
  def(doc, 'implementation', function () { return impl; });
  // ---- QY11: reflected URL attributes, hyperlink utils, dataset,
  // attributes, getAttributeNames, iterable classList. Pure JS over the
  // attribute natives; the base is `document.baseURI` (honours <base href>).
  var docBase = function () {
    // QPT1a: an about:blank (QPT1f: or about:srcdoc) iframe document's fallback base is its creator's base URL (HTML spec).
    var b = doc.querySelector('base[href]'), loc = baseFallback && /^about:(blank|srcdoc)$/.test(g.location.href) ? baseFallback : g.location.href;
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
  // ---- QY34: script-inserted <script> elements (HTML "prepare the script
  // element"). Only scripts made by createElement/createElementNS (or cloned
  // from a not-yet-started one) are eligible: parser-made, innerHTML, document.
  // write and DOMParser scripts never run through here (they are "parser
  // inserted"/"already started"). A script stays eligible until it is connected
  // AND has something to run, then it is marked started (never re-run when moved).
  // Inline code runs synchronously inside the inserting call; `src` goes through
  // the QF7 queue (api 'script': RequestPolicy, mixed content, CORS apply) and
  // runs as its own task (async), or in insertion order after `async = false`.
  // QPT1f: the eligible scripts are a page-wide table (`xr.s`: el, n, ord, cr) shared by every realm;
  // a script runs in the realm whose document it is connected to (`xr.prep`), with that realm entered.
  var sQ = [], sNames = 0, sTimeout = g.setTimeout;
  var sFire = function (el, type) { sTimeout.call(g, function () { el.dispatchEvent(new g.Event(type)); }, 0); };
  var sMark = function (el) { var S = xr.s; if (el && el.__id !== undefined) { S.cr[el.__id] = 1; if (!S.el[el.__id]) { S.el[el.__id] = 1; S.n++; } } return el; };
  var sOCreate = doc.createElement, sOCreateNS = doc.createElementNS, sOClone = NodeProto.cloneNode;
  doc.createElement = function (t) {
    var e = sOCreate.apply(this, arguments);
    if (String(t).toLowerCase() === 'script') sMark(e);
    return e;
  };
  doc.createElementNS = function (ns, t) {
    var e = sOCreateNS.apply(this, arguments);
    if (ns === 'http://www.w3.org/1999/xhtml' && String(t).toLowerCase() === 'script') sMark(e);
    return e;
  };
  var sCopy = function (a, b) {
    if (a.__id === undefined || b.__id === undefined) return;
    if (xr.s.el[a.__id]) sMark(b);
    var ka = a.childNodes, kb = b.childNodes;
    for (var i = 0; i < ka.length && i < kb.length; i++) sCopy(ka[i], kb[i]);
  };
  NodeProto.cloneNode = function () {
    var c = sOClone.apply(this, arguments);
    if (xr.s.n > 0 && c) sCopy(this, c);
    return c;
  };
  var sCur = function (el, f, w) {
    var prev = curScript, pw = wlast;
    curScript = el; wlast = w === undefined ? -1 : w;
    try { return f(); } finally { curScript = prev; wlast = pw; }
  };
  // QPT1f: runs classic script text as `el` (document.currentScript), compiled and run in this realm
  // with the realm ENTERED in an iframe (so the incumbent window, e.g. postMessage's source, is it).
  // `wl` >= 0: document.write inserts after that node (a parser script of a frame document).
  var sRun = function (el, text, name, top, wl) {
    sCur(el, function () {
      if (frameDoc) g.__scratchy_enter(g, function () { __scratchy_run_global(text, name, top); });
      else __scratchy_run_global(text, name, top);
    }, wl);
  };
  var sErrText = function (e) { return (e && e.name ? e.name + ': ' + e.message : 'Uncaught ' + String(e)) + (e && e.stack ? '\n' + e.stack : ''); };
  // One request through the QF7 queue; ok(text) / bad(message) run as promise jobs.
  var sFetch = function (url, cors, cred, ok, bad) {
    var h;
    try {
      h = g.__net_request({ method: 'GET', url: url, headers: [], body: null, mode: cors ? 'cors' : 'no-cors',
        credentials: cred, api: 'script', raw: true });
    } catch (e) { bad(String(e)); return; }
    h.promise.then(function (res) {
      if (res.status < 200 || res.status > 299) { __scratchy_err(url + ': failed to load script (HTTP ' + res.status + ')'); bad('HTTP ' + res.status); return; }
      ok(__scratchy_utf8(res.body));
    }, function (e) { bad(String(e)); });
  };
  var sModuleGraph = function (name, code, budget, ok, bad) {
    var deps = __mod_deps(name, code), left = deps.length, failed = false;
    if (!left) { ok(); return; }
    deps.forEach(function (u) {
      if (failed) return;
      if (__mod_has(u)) { if (--left === 0) ok(); return; }
      if (--budget.n < 0) { failed = true; __scratchy_err(u + ': module graph over 200 modules; not fetching more'); bad(); return; }
      sFetch(u, true, 'same-origin', function (text) {
        if (failed) return;
        __mod_register(u, text);
        sModuleGraph(u, text, budget, function () { if (--left === 0) ok(); }, function () { if (!failed) { failed = true; bad(); } });
      }, function () { if (!failed) { failed = true; bad(); } });
    });
  };
  var sImport = function (el, name, fireLoad) {
    import(name).then(function () { if (fireLoad) el.dispatchEvent(new g.Event('load')); }, function (e) {
      __scratchy_err(name + ': ' + sErrText(e));
      if (fireLoad) el.dispatchEvent(new g.Event('load'));
    });
  };
  var sExternal = function (el, id, src, mod) {
    var url;
    try { url = new g.URL(src, doc.baseURI).href; } catch (e) {
      __scratchy_err('scratchy: js: <script src=' + JSON.stringify(src) + '>: invalid URL'); sFire(el, 'error'); return;
    }
    var cors = __d_attr(id, 'crossorigin'), cred;
    if (mod) cred = cors != null && cors.toLowerCase() === 'use-credentials' ? 'include' : 'same-origin';
    else cred = cors == null || cors.toLowerCase() === 'use-credentials' ? 'include' : 'same-origin';
    var slot = xr.s.ord[id] ? { done: false, fn: null } : null;
    if (slot) sQ.push(slot);
    var settle = function (fn) {
      if (!slot) { fn(); return; }
      slot.fn = fn; slot.done = true;
      while (sQ.length && sQ[0].done) sQ.shift().fn();
    };
    var fail = function () { settle(function () { el.dispatchEvent(new g.Event('error')); }); };
    sFetch(url, mod || cors != null, cred, function (text) {
      if (mod) {
        if (!__mod_has(url)) __mod_register(url, text);
        sModuleGraph(url, text, { n: 200 }, function () { settle(function () { sImport(el, url, true); }); }, fail);
      } else {
        settle(function () {
          sRun(el, text, url, true);
          el.dispatchEvent(new g.Event('load'));
        });
      }
    }, fail);
  };
  var JSMIME = /^(text\/javascript|application\/javascript|application\/x-javascript|text\/ecmascript|application\/ecmascript|text\/jscript|text\/livescript|text\/x-javascript|text\/x-ecmascript)$/;
  // 'classic', 'module', or null (not a script this browser runs).
  var sType = function (id) {
    var ty = __d_attr(id, 'type'), lang = __d_attr(id, 'language'), t;
    if (ty != null) t = ty.trim() === '' ? 'text/javascript' : ty.trim().toLowerCase();
    else if (lang != null && lang.trim() !== '') t = 'text/' + lang.trim().toLowerCase();
    else t = 'text/javascript';
    if (t === 'module') return 'module';
    if (!JSMIME.test(t) || __d_attr(id, 'nomodule') != null) return null;
    return 'classic';
  };
  var sPrepare = function (id) {
    var st = sType(id), src = __d_attr(id, 'src');
    if (!st) return;
    var mod = st === 'module';
    var el = wrap(id), text = src == null ? el.textContent : '';
    if (src == null && text === '') return; // empty: stays eligible (text or src may come later)
    delete xr.s.el[id]; xr.s.n--;
    if (src != null) {
      if (src.trim() === '') { sFire(el, 'error'); return; }
      sExternal(el, id, src, mod);
    } else if (mod) {
      var name = String(g.location.href).split('#')[0] + '#inline-module-s' + (++sNames);
      __mod_register(name, text);
      sModuleGraph(name, text, { n: 200 }, function () { sImport(el, name, false); }, function () {});
    } else {
      sRun(el, text, 'inline script (inserted)', false);
    }
  };
  // QPT1f: every connected eligible script, in the realm of the document it is in (the page's, or a
  // live iframe realm's; a script in a dropped frame's document or in an inert document never runs).
  var sSweep = function () {
    var S = xr.s, ids = Object.keys(S.el);
    for (var i = 0; i < ids.length; i++) {
      var id = +ids[i];
      if (!S.el[id]) continue;
      var p = xr.prep[composedRootId(id)];
      if (p) p(id);
    }
  };
  xr.prep[0] = sPrepare;
  // QPT1f (R53 D9): load `html` as this iframe realm's document (a srcdoc, or what document.write
  // collected until close()): parse it (scripting on), run its classic and module scripts in document
  // order with the realm entered (an external one through the page's request queue, blocking the
  // ones after it), then DOMContentLoaded, nested srcdoc frames, the window `load`, and the parent's
  // `load` on the <iframe>. Stops as soon as the frame is dropped (`closed`).
  var frameLoad = function (html, top) {
    if (!frameDoc || g.closed) return;
    wBuf = null;
    var kids = __d_children(doc.__id, false);
    for (var i = 0; i < kids.length; i++) __d_remove(kids[i]);
    doc.readyState = 'loading';
    __d_load_html(doc.__id, String(html));
    if (ceOn) ceScan(doc.__id, false);
    if (/<iframe/i.test(html)) syncWin();
    var ids = __d_query(doc.__id, 'script', false) || [], k = 0, n = 0;
    var finish = function () {
      if (g.closed) return;
      doc.readyState = 'interactive';
      g.__scratchy_dispatch(doc, new g.Event('DOMContentLoaded', { bubbles: true }));
      if (g.closed) return;
      bootFrames();
      if (g.closed) return;
      doc.readyState = 'complete';
      g.__scratchy_dispatch(g, new g.Event('load'));
      if (frameOnLoad && !g.closed) frameOnLoad();
    };
    var step = function (top) {
      while (k < ids.length) {
        if (g.closed) return;
        var id = ids[k++];
        if (composedRootId(id) !== doc.__id) continue; // removed by an earlier script
        var t = sType(id);
        if (!t) continue;
        var el = wrap(id), src = __d_attr(id, 'src');
        if (t === 'module') {
          if (src != null) { if (src.trim() !== '') sExternal(el, id, src, true); continue; }
          var mn = String(g.location.href).split('#')[0] + '#inline-module-f' + (++sNames), mt = el.textContent;
          __mod_register(mn, mt);
          sModuleGraph(mn, mt, { n: 200 }, (function (nm, e) { return function () { sImport(e, nm, false); }; })(mn, el), function () {});
          continue;
        }
        if (src != null) {
          if (src.trim() === '') { sFire(el, 'error'); continue; }
          var url;
          try { url = new g.URL(src, doc.baseURI).href; } catch (e) { sFire(el, 'error'); continue; }
          var cors = __d_attr(id, 'crossorigin');
          sFetch(url, cors != null, cors == null || cors.toLowerCase() === 'use-credentials' ? 'include' : 'same-origin', (function (e, u) {
            return function (text) {
              if (g.closed) return;
              sRun(e, text, u, true, e.__id);
              e.dispatchEvent(new g.Event('load'));
              step(true);
            };
          })(el, url), (function (e) {
            return function () { if (g.closed) return; e.dispatchEvent(new g.Event('error')); step(true); };
          })(el));
          return;
        }
        sRun(el, el.textContent, doc.URL + ' inline script #' + (++n), top, id);
      }
      finish();
    };
    step(top);
  };
  Object.defineProperty(g, '__scratchy_frame_load', { value: function (html, top) { frameLoad(html, !!top); } });
  var sOAfter = afterIns, sOTxt = __d_settext, sOSet = __d_setattr;
  afterIns = function (c) { if (sOAfter) sOAfter(c); if (xr.s.n > 0) sSweep(); };
  __d_settext = function (id, t) { sOTxt(id, t); if (xr.s.n > 0 && xr.s.el[id]) sSweep(); };
  __d_setattr = function (id, n, v) { sOSet(id, n, v); if (xr.s.n > 0 && xr.s.el[id] && String(n).toLowerCase() === 'src') sSweep(); };
  defs(SCP, 'async', function () { return __d_attr(this.__id, 'async') != null || (!!xr.s.cr[this.__id] && !xr.s.ord[this.__id]); },
    function (v) { if (v) { delete xr.s.ord[this.__id]; __d_setattr(this.__id, 'async', ''); } else { xr.s.ord[this.__id] = 1; __d_rmattr(this.__id, 'async'); } });
  var IMP = mkProto('HTMLImageElement', ['img']);
  rUrl(IMP, 'src', 'src'); rStrs(IMP, ['alt', 'srcset', 'sizes', 'crossOrigin', 'useMap', 'decoding', 'loading', 'referrerPolicy', 'name', 'align']);
  rBool(IMP, 'isMap', 'ismap');
  def(IMP, 'currentSrc', function () { return absUrl(this, 'src'); });
  // ---- QY56: image loading state. The App fetches every `src` (load-time parse,
  // layout misses, `__d_img_req` for elements that may never be laid out) and
  // records the outcome natively (`__d_img_state`: '' unknown, 'e' failed, 'w,h');
  // `complete`/`naturalWidth`/`naturalHeight`/`decode()` read it and `load`/`error`
  // fire once per element and `src` (as a task: from the App when the fetch ends,
  // or a timer when the outcome was already known).
  var imRaw = function (el) { var s = __d_attr(el.__id, 'src'); return (s == null || /^[ \t\n\f\r]*$/.test(s)) ? null : s; };
  var imSt = function (raw) { var t = __d_img_state(raw); return t === '' ? undefined : t === 'e' ? 0 : t.split(',').map(Number); };
  var imW = {}, imTimeout = g.setTimeout;
  var imFire = function (el, raw) {
    if (el.__imF === raw || imRaw(el) !== raw) return;
    var st = imSt(raw);
    if (st === undefined) return;
    el.__imF = raw;
    var ds = el.__imD; el.__imD = null;
    g.__scratchy_dispatch(el, new g.Event(st ? 'load' : 'error'));
    if (ds) ds.forEach(function (d) {
      if (st) d[0](); else d[1](typeof g.DOMException === 'function' ? new g.DOMException('The source image cannot be decoded.', 'EncodingError') : new Error('EncodingError'));
    });
  };
  var imSweep = function (raw) {
    var els = [], k;
    for (k in imW) if ((raw === undefined || k === raw) && imSt(k) !== undefined) { els = els.concat(imW[k]); delete imW[k]; }
    var all = doc.querySelectorAll('img');
    for (var i = 0; i < all.length; i++) els.push(all[i]);
    els.forEach(function (el) { var r = imRaw(el); if (r !== null && (raw === undefined || r === raw)) imFire(el, r); });
  };
  Object.defineProperty(g, '__scratchy_img_done', { value: function (raw) { imSweep(raw); } });
  Object.defineProperty(g, '__scratchy_img_sweep', { value: function () { imSweep(); } });
  var imSrcSet = function (el) {
    el.__imF = undefined;
    var raw = imRaw(el);
    if (raw === null) return;
    if (imSt(raw) !== undefined) { imTimeout(function () { imFire(el, raw); }, 0); return; }
    var w = imW[raw] || (imW[raw] = []);
    if (w.indexOf(el) < 0) w.push(el);
    __d_img_req(raw);
  };
  var imOSet = __d_setattr;
  __d_setattr = function (id, n, v) {
    imOSet(id, n, v);
    if (String(n).toLowerCase() !== 'src') return;
    var el = wrap(id);
    if (el && el.tagName === 'IMG') imSrcSet(el);
  };
  var imOSetq = __d_setq;
  __d_setq = function (id, n, v) {
    imOSetq(id, n, v);
    if (String(n).toLowerCase() !== 'src') return;
    var el = wrap(id);
    if (el && el.tagName === 'IMG') imSrcSet(el);
  };
  def(IMP, 'complete', function () { var r = imRaw(this); return r === null || imSt(r) !== undefined; });
  def(IMP, 'naturalWidth', function () { var r = imRaw(this), st = r !== null && imSt(r); return st ? st[0] : 0; });
  def(IMP, 'naturalHeight', function () { var r = imRaw(this), st = r !== null && imSt(r); return st ? st[1] : 0; });
  ['width', 'height'].forEach(function (a, ix) {
    defs(IMP, a, function () {
      var v = parseInt(__d_attr(this.__id, a), 10);
      if (v >= 0) return v;
      var r = imRaw(this), st = r !== null && imSt(r);
      return st ? st[ix] : 0;
    }, function (v) { __d_setattr(this.__id, a, String(Math.max(0, Math.floor(Number(v)) || 0))); });
  });
  Object.defineProperty(IMP, 'decode', { configurable: true, writable: true, enumerable: true, value: function () {
    var el = this;
    return new Promise(function (res, rej) {
      var r = imRaw(el), st = r === null ? undefined : imSt(r);
      var err = function () { return typeof g.DOMException === 'function' ? new g.DOMException('The source image cannot be decoded.', 'EncodingError') : new Error('EncodingError'); };
      if (r === null) return rej(err());
      if (st !== undefined) return st ? res() : rej(err());
      (el.__imD || (el.__imD = [])).push([res, rej]);
    });
  } });
  var ImageCtor = ({ Image: function (w, h) {
    var e = doc.createElement('img');
    if (w !== undefined) e.setAttribute('width', String(w));
    if (h !== undefined) e.setAttribute('height', String(h));
    return e;
  } }).Image;
  ImageCtor.prototype = IMP;
  try { Object.defineProperty(g, 'Image', { value: ImageCtor, writable: true, configurable: true }); } catch (e) {}
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
  var putToks = function (l, c) { __d_setattr(l._el.__id, 'class', c.join(' ')); };
  var clsAttr = function (l) { var v = __d_attr(l._el.__id, 'class'); return v == null ? '' : v; };
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
  DTP.toString = function () { return clsAttr(this); };
  DTP.forEach = function (f, t) { toks(this).forEach(function (x, i) { f.call(t, x, i, this); }, this); };
  DTP.keys = function () { return toks(this).keys(); };
  DTP.values = function () { return toks(this).values(); };
  DTP.entries = function () { return toks(this).entries(); };
  DTP[Symbol.iterator] = DTP.values;
  def(DTP, 'length', function () { return toks(this).length; });
  defs(DTP, 'value', function () { return clsAttr(this); }, function (v) { __d_setattr(this._el.__id, 'class', String(v)); });
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
  // views (not nodes in the tree) whose `value` reads/writes through. QY45: an
  // attached Attr is (element, qualified name); a detached one (createAttribute,
  // after removeAttributeNode) carries its own {ns, prefix, local, value}.
  var AttrP = g.Attr.prototype;
  var XML_NS = 'http://www.w3.org/XML/1998/namespace', XMLNS_NS = 'http://www.w3.org/2000/xmlns/';
  var ncName = /^[A-Za-z_À-￿][-A-Za-z0-9_.·-￿]*$/;
  var nsArg = function (ns) { return ns == null || ns === '' ? '' : String(ns); };
  var mkAttr = function (el, name) {
    var a = Object.create(AttrP);
    Object.defineProperty(a, '_el', { value: el, writable: true, configurable: true });
    Object.defineProperty(a, '_n', { value: name, writable: true, configurable: true });
    Object.defineProperty(a, '_d', { value: null, writable: true, configurable: true });
    return a;
  };
  var mkDetached = function (ns, prefix, local, value) {
    var a = mkAttr(null, null);
    a._d = { ns: ns, prefix: prefix, local: local, value: value };
    return a;
  };
  var qn = function (prefix, local) { return prefix ? prefix + ':' + local : local; };
  // {ns, prefix, local, value} of an attribute view, or null once it is gone.
  var aInfo = function (a) {
    if (!a._el) return a._d;
    var l = __d_attr_list(a._el.__id);
    for (var i = 0; i < l.length; i += 4) if (qn(l[i + 1], l[i + 2]) === a._n) return { ns: l[i], prefix: l[i + 1], local: l[i + 2], value: l[i + 3] };
    return null;
  };
  var htmlEl = function (el) { return __d_ns(el.__id) === 'http://www.w3.org/1999/xhtml'; };
  var findQ = function (el, n) { n = String(n); if (htmlEl(el)) n = n.toLowerCase(); return __d_getq(el.__id, n) == null ? null : n; };
  var findNS = function (el, ns, local) {
    var l = __d_attr_list(el.__id);
    for (var i = 0; i < l.length; i += 4) if (l[i] === ns && l[i + 2] === local) return qn(l[i + 1], l[i + 2]);
    return null;
  };
  def(AttrP, 'name', function () { if (this._el) return this._n; var d = this._d; return qn(d.prefix, d.local); });
  def(AttrP, 'localName', function () { var i = aInfo(this); return i ? i.local : this._n; });
  def(AttrP, 'nodeName', function () { return this.name; });
  def(AttrP, 'nodeType', function () { return 2; });
  def(AttrP, 'namespaceURI', function () { var i = aInfo(this); return i && i.ns ? i.ns : null; });
  def(AttrP, 'prefix', function () { var i = aInfo(this); return i && i.prefix ? i.prefix : null; });
  def(AttrP, 'specified', function () { return true; });
  def(AttrP, 'ownerElement', function () { return this._el || null; });
  defs(AttrP, 'value', function () { if (!this._el) return this._d.value; var v = __d_getq(this._el.__id, this._n); return v == null ? '' : v; },
    function (v) { if (this._el) __d_setq(this._el.__id, this._n, String(v)); else this._d.value = String(v); });
  defs(AttrP, 'nodeValue', function () { return this.value; }, function (v) { this.value = v; });
  defs(AttrP, 'textContent', function () { return this.value; }, function (v) { this.value = v; });
  var NNM = g.NamedNodeMap = function NamedNodeMap() { throw new TypeError('Illegal constructor'); };
  var NP = NNM.prototype;
  var attrNames = function (el) {
    var l = __d_attr_list(el.__id), out = [];
    for (var i = 0; i < l.length; i += 4) out.push(qn(l[i + 1], l[i + 2]));
    return out;
  };
  var attrList = function (el) { return attrNames(el).map(function (n) { return mkAttr(el, n); }); };
  NP.item = function (i) { return attrList(this._el)[i] || null; };
  NP.getNamedItem = function (n) { var q = findQ(this._el, n); return q === null ? null : mkAttr(this._el, q); };
  NP.getNamedItemNS = function (ns, n) { var q = findNS(this._el, nsArg(ns), String(n)); return q === null ? null : mkAttr(this._el, q); };
  NP.removeNamedItem = function (n) {
    var a = this.getNamedItem(n);
    if (!a) throw herr("Failed to execute 'removeNamedItem' on 'NamedNodeMap': No item with name '" + n + "' was found.", 'NotFoundError');
    return this._el.removeAttributeNode(a);
  };
  NP.removeNamedItemNS = function (ns, n) {
    var a = this.getNamedItemNS(ns, n);
    if (!a) throw herr("Failed to execute 'removeNamedItemNS' on 'NamedNodeMap': No item with name '" + n + "' in namespace '" + ns + "' was found.", 'NotFoundError');
    return this._el.removeAttributeNode(a);
  };
  NP.setNamedItem = function (a) {
    if (AttrP.isPrototypeOf(a)) return this._el.setAttributeNode(a);
    __d_setq(this._el.__id, a.name, a.value); return null;
  };
  NP.setNamedItemNS = NP.setNamedItem;
  NP[Symbol.iterator] = function () { return attrList(this._el)[Symbol.iterator](); };
  def(NP, 'length', function () { return __d_attr_list(this._el.__id).length / 4; });
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
        return isIdx(k) ? +k < t.length : (k in t || __d_getq(el.__id, k) != null);
      },
      ownKeys: function () { var n = attrNames(el); return n.map(function (x, i) { return String(i); }).concat(n); },
      getOwnPropertyDescriptor: function (t, k) {
        if (typeof k !== 'string') return undefined;
        var ix = isIdx(k), v = ix ? t.item(+k) : t.getNamedItem(k);
        return v ? { value: v, writable: false, enumerable: ix, configurable: true } : undefined;
      }
    }));
  });
  ElementProto.getAttributeNames = function () { return attrNames(this); };
  ElementProto.hasAttributes = function () { return __d_attr_list(this.__id).length > 0; };
  ElementProto.getAttributeNode = function (n) { return this.attributes.getNamedItem(n); };
  // QY45: the namespaced family. "validate and extract" per the DOM spec.
  var extractName = function (fn, owner, ns, q) {
    ns = ns == null || ns === '' ? null : String(ns); q = String(q);
    var P = "Failed to execute '" + fn + "' on '" + owner + "': ";
    if (!validName(q)) throw herr(P + 'The string contains invalid characters.', 'InvalidCharacterError');
    var i = q.indexOf(':'), pre = i < 0 ? null : q.slice(0, i), loc = i < 0 ? q : q.slice(i + 1);
    if (i >= 0 && !(ncName.test(pre) && ncName.test(loc))) throw herr(P + 'The qualified name provided is not a valid XML name.', 'InvalidCharacterError');
    if ((pre !== null && ns === null) || (pre === 'xml' && ns !== XML_NS) ||
      ((q === 'xmlns' || pre === 'xmlns') && ns !== XMLNS_NS) || (ns === XMLNS_NS && q !== 'xmlns' && pre !== 'xmlns'))
      throw herr(P + 'The namespace provided is invalid for the qualified name.', 'NamespaceError');
    return [ns === null ? '' : ns, pre || '', loc];
  };
  ElementProto.setAttributeNS = function (ns, q, v) {
    var x = extractName('setAttributeNS', 'Element', ns, q);
    __d_setns(this.__id, x[0], x[1], x[2], String(v));
  };
  ElementProto.getAttributeNS = function (ns, l) { var v = __d_getns(this.__id, nsArg(ns), String(l)); return v == null ? null : v; };
  ElementProto.hasAttributeNS = function (ns, l) { return __d_getns(this.__id, nsArg(ns), String(l)) != null; };
  ElementProto.removeAttributeNS = function (ns, l) { __d_rmns(this.__id, nsArg(ns), String(l)); };
  ElementProto.getAttributeNodeNS = function (ns, l) { return this.attributes.getNamedItemNS(ns, l); };
  ElementProto.setAttributeNode = function (a) {
    if (!a || !AttrP.isPrototypeOf(a)) throw new TypeError("Failed to execute 'setAttributeNode' on 'Element': parameter 1 is not of type 'Attr'.");
    if (a._el === this) return a;
    if (a._el) throw herr("Failed to execute 'setAttributeNode' on 'Element': The node provided is an attribute node that is already an attribute of another Element; attribute nodes must be explicitly cloned.", 'InUseAttributeError');
    var d = a._d, oq = findNS(this, d.ns, d.local), old = null;
    if (oq !== null) { var oi = aInfo(mkAttr(this, oq)); old = mkDetached(oi.ns, oi.prefix, oi.local, oi.value); }
    __d_setns(this.__id, d.ns, d.prefix, d.local, d.value);
    a._el = this; a._n = oq !== null ? oq : qn(d.prefix, d.local); a._d = null;
    return old;
  };
  ElementProto.setAttributeNodeNS = ElementProto.setAttributeNode;
  ElementProto.removeAttributeNode = function (a) {
    if (!a || !AttrP.isPrototypeOf(a)) throw new TypeError("Failed to execute 'removeAttributeNode' on 'Element': parameter 1 is not of type 'Attr'.");
    var i = a._el === this ? aInfo(a) : null;
    if (!i) throw herr("Failed to execute 'removeAttributeNode' on 'Element': The node provided is not an attribute of this element.", 'NotFoundError');
    __d_rmns(this.__id, i.ns, i.local);
    a._d = i; a._el = null; a._n = null;
    return a;
  };
  doc.createAttribute = function (n) {
    n = String(n);
    if (!validName(n)) throw herr("Failed to execute 'createAttribute' on 'Document': String contains an invalid character.", 'InvalidCharacterError');
    return mkDetached('', '', n.toLowerCase(), '');
  };
  doc.createAttributeNS = function (ns, q) {
    var x = extractName('createAttributeNS', 'Document', ns, q);
    return mkDetached(x[0], x[1], x[2], '');
  };
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
  rBool(g.HTMLStyleElement.prototype, 'disabled', 'disabled');
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
  // Media: the real API is `media_prelude.js` (QM1f); only the not-yet-built
  // pieces stay `not implemented` stubs here.
  var ens = function (n, tags, base) { return g[n] ? g[n].prototype : mkProto(n, tags, base); };
  var MEP = ens('HTMLMediaElement', []);
  var nimp = function (what) { __scratchy_err('scratchy: js: not implemented: ' + what); };
  MEP.addTextTrack = function () { nimp('HTMLMediaElement.addTextTrack'); };
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
  ['Selection', 'NamedNodeMap', 'DOMTokenList', 'StyleSheet', 'CSSStyleSheet', 'CSSRule', 'CSSStyleDeclaration',
    'Screen', 'Storage'].forEach(function (n) { if (!g[n]) g[n] = illegal(n); });
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
  // the CEReactions stack). Only the realm's own document is ever upgraded
  // (the page: id 0; an iframe realm: its document, with its own registry, QPT1f).
  // Cuts: `:defined`, no shadow DOM (attachShadow throws).
  var ceOn = false, ceByName = Object.create(null), ceByCtor = new Map(), ceWhen = Object.create(null);
  var ceState = Object.create(null), ceConnFlag = Object.create(null), ceNConn = 0;
  var ceStats = { hooks: 0, defineQueries: 0 };
  Object.defineProperty(g, '__ce_stats', { value: ceStats });
  var ceReport = function (e, what) {
    __scratchy_err('scratchy: js: uncaught exception in custom element ' + what + ': ' + (e && e.stack ? e.stack : String(e)));
  };
  // Reactions run in the realm that owns the definition, with that realm entered (an iframe realm).
  var ceCall = function (el, fn, args) {
    if (typeof fn !== 'function') return;
    try {
      if (frameDoc) g.__scratchy_enter(g, function () { fn.apply(el, args); }); else fn.apply(el, args);
    } catch (e) { ceReport(e, fn.name || 'callback'); }
  };
  // QPT1f: "connected" = in this realm's document (the page, or an iframe realm's own document).
  var ceIsConn = function (id) { return composedRootId(id) === doc.__id; };
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
  // QSD3: candidates under `id`, including those inside shadow trees whose host is under `id`.
  var srHosts = [];
  var srUnder = function (h, id) {
    var x = h, p, hh;
    for (;;) {
      if (x === id) return true;
      p = __d_parent(x);
      if (p >= 0) x = p; else { hh = __d_shadow_host(x); if (hh < 0) return false; x = hh; }
    }
  };
  var ceQueryDeep = function (id) {
    var l = __d_ce_candidates(doc.__id, id);
    for (var i = 0; i < srHosts.length; i++) {
      if (srUnder(srHosts[i], id)) l = l.concat(__d_ce_candidates(doc.__id, __d_shadow_of(srHosts[i])));
    }
    return l;
  };
  var ceCollect = function (id, self) {
    var out = [];
    if (self && ceConnFlag[id]) out.push(id);
    var l = ceQueryDeep(id);
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
      var run = function () {
        var r = Reflect.construct(d.ctor, [], d.ctor);
        if (r !== el) throw new TypeError('The custom element constructor did not produce the element being upgraded.');
      };
      if (frameDoc) g.__scratchy_enter(g, run); else run();
      ok = true;
    } catch (e) { ceReport(e, d.name + ' constructor'); }
    d.stack.pop();
    if (!ok) { ceState[id] = -1; return; }
    ceState[id] = d; xr.cs[id] = ceApi;
    if (d.observed.length) {
      var names = __d_attr_names(id);
      for (var i = 0; i < names.length; i++) {
        var n = names[i].toLowerCase();
        if (d.observed.indexOf(n) >= 0) ceCall(el, d.changed, [n, null, __d_attr(id, n)]);
      }
    }
    if (conn) ceSetConn(id);
  };
  var ceProcess = function (id, conn, up) {
    var st = ceState[id];
    if (!st) { if (!up || xr.cs[id]) return; var d = ceDefFor(id); if (d) ceUpgrade(id, d, conn); return; }
    if (st !== -1 && conn && !ceConnFlag[id]) ceSetConn(id);
  };
  // Upgrade (and fire connectedCallback for) candidates under `id`, tree order.
  // `force` also upgrades inside a disconnected tree (customElements.upgrade, clones).
  // QPT1f4: a realm scans for ITS elements wherever they are connected (any live realm's document), and
  // upgrades with its own registry only what is in its own document (or `force`: upgrade(), clones, define()).
  var ceScan = function (id, force) {
    var rd = composedRootId(id), conn = xr.byDoc[rd] !== undefined;
    if (!conn && !force) return;
    var up = force || rd === doc.__id;
    var l = ceQueryDeep(id);
    if (__d_kind(id) === 1) l.unshift(id);
    for (var i = 0; i < l.length; i++) ceProcess(l[i], conn, up);
  };
  // The page-wide reaction drivers the hooks call (every realm with a registry takes part).
  var cePre = function (id, self) {
    var out = null;
    for (var k in xr.ce) {
      var a = xr.ce[k];
      if (!a.n()) continue;
      var l = a.collect(id, self);
      if (l.length) (out || (out = [])).push([a, l]);
    }
    return out;
  };
  var ceFirePre = function (pre) { if (pre) for (var i = 0; i < pre.length; i++) pre[i][0].fire(pre[i][1]); };
  var ceScanAll = function (id) { for (var k in xr.ce) xr.ce[k].scan(id); };
  // adoptedCallback (old document, new document) for the upgraded elements under each root.
  var ceAdopt = function (roots, olds, nd) {
    for (var i = 0; i < roots.length; i++) {
      var od = olds[i] < 0 ? 0 : olds[i];
      if (od === nd) continue;
      for (var k in xr.ce) xr.ce[k].adopt(roots[i], cache[od] || null, cache[nd] || null);
    }
  };
  var ceOwners = function (ids) { return ids.map(ownerDocId); };
  var ceApi = {
    on: function () { return ceOn; }, n: function () { return ceNConn; }, st: function (id) { return ceState[id]; },
    collect: function (id, self) { return ceCollect(id, self); }, fire: function (ids) { ceFire(ids); },
    scan: function (id) { ceScan(id, false); },
    call: function (id, fn, args) { ceCall(cache[id], fn, args); },
    adopt: function (root, od, nd) {
      var l = ceQueryDeep(root);
      if (__d_kind(root) === 1) l.unshift(root);
      for (var i = 0; i < l.length; i++) {
        var st = ceState[l[i]];
        if (st && st !== -1) ceCall(cache[l[i]], st.adopted, [od, nd]);
      }
    }
  };
  // The ONE shared mutation layer (QY6e custom elements + QY6a MutationObserver):
  // installed by the first `define()` or `observe()`, rebinding the natives to
  // hooked versions. Each hook does the CE work when `ceOn` and the record
  // work when observers exist (`xr.mo.n > 0`); neither runs otherwise.
  var hookOn = false;
  // The realm whose definition upgraded element `id`, when it observes attributes (else null).
  var ceHolder = function (id) {
    var a = xr.cs[id], s = a && a.st(id);
    return s && s !== -1 && s.observed.length ? a : null;
  };
  var hookInstall = function () {
    if (hookOn) return;
    hookOn = true;
    var oIns = __d_insert, oRem = __d_remove, oHtml = __d_sethtml, oTxt = __d_settext, oSet = __d_setattr, oRm = __d_rmattr, oW = __d_write;
    __d_insert = function (p, c, b) {
      ceStats.hooks++;
      var mo = xr.mo.n > 0, frag = __d_kind(c) === 11, kids = frag ? __d_children(c, false) : [c];
      var oldP = mo && !frag ? __d_parent(c) : -1, oPrev = -1, oNext = -1;
      if (oldP >= 0) { oPrev = __d_sibling(c, -1, false); oNext = __d_sibling(c, 1, false); }
      var pre = xr.cn && __d_parent(c) >= 0 ? cePre(c, true) : null; // a move = disconnect + connect
      var olds = xr.cn ? ceOwners(kids) : null;
      var ok = oIns(p, c, b);
      if (ok && mo) {
        if (oldP >= 0) moChild(oldP, [], [c], oPrev, oNext);
        if (kids.length) moChild(p, kids, [], __d_sibling(kids[0], -1, false), __d_sibling(kids[kids.length - 1], 1, false));
      }
      ceFirePre(pre);
      if (xr.cn) {
        if (ok) ceAdopt(kids, olds, ownerDocId(p) < 0 ? 0 : ownerDocId(p));
        for (var i = 0; i < kids.length; i++) ceScanAll(kids[i]);
      }
      return ok;
    };
    __d_remove = function (id) {
      ceStats.hooks++;
      var p = xr.mo.n > 0 ? __d_parent(id) : -1, pv = -1, nx = -1;
      if (p >= 0) { pv = __d_sibling(id, -1, false); nx = __d_sibling(id, 1, false); }
      var pre = xr.cn ? cePre(id, true) : null;
      oRem(id);
      if (p >= 0) moChild(p, [], [id], pv, nx);
      ceFirePre(pre);
    };
    __d_sethtml = function (id, html) {
      ceStats.hooks++;
      var old = xr.mo.n > 0 ? __d_children(id, false) : null;
      var pre = xr.cn ? cePre(id, false) : null;
      oHtml(id, html);
      if (old) { var nw = __d_children(id, false); if (old.length || nw.length) moChild(id, nw, old, -1, -1); }
      ceFirePre(pre);
      if (xr.cn) ceScanAll(id);
    };
    __d_settext = function (id, t) {
      ceStats.hooks++;
      var k = __d_kind(id), mo = xr.mo.n > 0, old = null, oldTxt = null;
      if (mo) { if (k === 3) oldTxt = __d_text(id); else if (k !== 8) old = __d_children(id, false); }
      var pre = xr.cn && (k === 1 || k === 11) ? cePre(id, false) : null;
      oTxt(id, t);
      if (old) { var nw = __d_children(id, false); if (old.length || nw.length) moChild(id, nw, old, -1, -1); }
      else if (oldTxt !== null) moChar(id, oldTxt);
      ceFirePre(pre);
    };
    __d_write = function (a, h) {
      ceStats.hooks++;
      var p = __d_parent(a), nx = xr.mo.n > 0 && p >= 0 ? __d_sibling(a, 1, false) : -2;
      var r = oW(a, h);
      if (nx !== -2) {
        var added = [], c = __d_sibling(a, 1, false);
        while (c >= 0 && c !== nx) { added.push(c); c = __d_sibling(c, 1, false); }
        if (added.length) moChild(p, added, [], a, nx);
      }
      if (xr.cn && p >= 0) ceScanAll(p);
      return r;
    };
    __d_setattr = function (id, n, v) {
      ceStats.hooks++;
      var ca = ceHolder(id), d = ca && ca.st(id), ce = !!ca, mo = xr.mo.n > 0, ln = '', old = null;
      if (ce || mo) { ln = String(n).toLowerCase(); old = __d_attr(id, ln); if (old == null) old = null; }
      oSet(id, n, v);
      if (mo) moAttr(id, ln, old);
      if (ce && d.observed.indexOf(ln) >= 0) ca.call(id, d.changed, [ln, old, __d_attr(id, ln)]);
    };
    // QY45: the qualified-name / namespaced variants share the same hook work.
    var attrHook = function (oSetter, oRemover, getOld, key) {
      return [function (id, a, b, c, d) {
        ceStats.hooks++;
        var ca = ceHolder(id), st = ca && ca.st(id), ce = !!ca, mo = xr.mo.n > 0, old = null, nm = key(id, a, b);
        if (ce || mo) { old = getOld(id, a, b); if (old == null) old = null; }
        oSetter(id, a, b, c, d);
        if (mo) moAttr(id, nm, old);
        if (ce && st.observed.indexOf(nm) >= 0) ca.call(id, st.changed, [nm, old, getOld(id, a, b)]);
      }, function (id, a, b) {
        ceStats.hooks++;
        var ca = ceHolder(id), st = ca && ca.st(id), ce = !!ca, mo = xr.mo.n > 0, old = null, nm = key(id, a, b);
        if (ce || mo) old = getOld(id, a, b);
        oRemover(id, a, b);
        if (old != null) {
          if (mo) moAttr(id, nm, old);
          if (ce && st.observed.indexOf(nm) >= 0) ca.call(id, st.changed, [nm, old, null]);
        }
      }];
    };
    var oSetq = __d_setq, oRmq = __d_rmq, oSetns = __d_setns, oRmns = __d_rmns;
    var hq = attrHook(oSetq, oRmq, function (id, n) { return __d_getq(id, n); }, function (id, n) { return __d_ns(id) === 'http://www.w3.org/1999/xhtml' ? String(n).toLowerCase() : String(n); });
    __d_setq = function (id, n, v) { hq[0](id, n, v); };
    __d_rmq = function (id, n) { hq[1](id, n); };
    var hn = attrHook(function (id, ns, loc, pre, v) { oSetns(id, ns, pre, loc, v); }, oRmns,
      function (id, ns, loc) { return __d_getns(id, ns, loc); }, function (id, ns, loc) { return loc; });
    __d_setns = function (id, ns, pre, loc, v) { hn[0](id, ns, loc, pre, v); };
    __d_rmns = function (id, ns, loc) { hn[1](id, ns, loc); };
    __d_rmattr = function (id, n) {
      ceStats.hooks++;
      var ca = ceHolder(id), d = ca && ca.st(id), ce = !!ca, mo = xr.mo.n > 0, ln = '', old = null;
      if (ce || mo) { ln = String(n).toLowerCase(); old = __d_attr(id, ln); }
      oRm(id, n);
      if (old != null) {
        if (mo) moAttr(id, ln, old);
        if (ce && d.observed.indexOf(ln) >= 0) ca.call(id, d.changed, [ln, old, null]);
      }
    };
  };
  xr.hk.push(hookInstall);
  var ceInstall = function () {
    hookInstall();
    var oCE = doc.createElement, oCENS = doc.createElementNS, oClone = NodeProto.cloneNode;
    var created = function (el, opts, self) {
      if (self && self.__id !== doc.__id) return el; // second documents: never upgraded
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
    ceState[id] = d; xr.cs[id] = ceApi;
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
      connected: P.connectedCallback, disconnected: P.disconnectedCallback, adopted: P.adoptedCallback, changed: observed.length ? P.attributeChangedCallback : undefined };
    if (!ceOn) {
      ceOn = true; ceInstall(); xr.ce[doc.__id] = ceApi; xr.cn++;
      for (var hk = 0; hk < xr.hk.length; hk++) xr.hk[hk](); // every realm's natives drive this registry's reactions
    }
    ceByName[name] = d; ceByCtor.set(ctor, d);
    __d_ce_add(doc.__id, name, ext || '');
    // QY21: only the NEW name is looked up (the old all-names selector made define() quadratic).
    ceStats.defineQueries++;
    var l = __d_query(doc.__id, name.indexOf('.') >= 0 ? '*' : ext ? ext + '[is]' : name, false);
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
  var moPending = [], moQueued = false;
  var MO_S = Symbol('MutationObserver'), MR_S = Symbol('MutationRecord');
  var moErr = function (e) {
    __scratchy_err('scratchy: js: uncaught exception in MutationObserver callback: ' + (e && e.stack ? String(e) + '\n' + e.stack : String(e)));
  };
  // Observers interested in a mutation of `type` on node `id` -> [{st, old}] or null.
  var moTargets = function (id, type, name) {
    var out = null, n = id, rs, i, j, r, o, e;
    while (n >= 0) {
      rs = xr.mo.reg[n];
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
      st.q.push(mk(list[i].old, st.rec));
      if (!st.pending) { st.pending = true; st.sched(st); }
    }
  };
  // Delivery runs in the observer's realm: its own pending list and microtask.
  var moSched = function (st) {
    moPending.push(st);
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
      rs = xr.mo.reg[n];
      if (rs) for (i = 0; i < rs.length; i++) {
        if (!rs[i].opts.subtree || rs[i].src) continue;
        for (j = 0; j < removed.length; j++) {
          t = { st: rs[i].st, opts: rs[i].opts, src: rs[i] };
          (xr.mo.reg[removed[j]] || (xr.mo.reg[removed[j]] = [])).push(t);
          rs[i].st.trans.push({ id: removed[j], reg: t });
          xr.mo.n++;
        }
      }
      n = __d_parent(n);
    }
  };
  var moChildNow = function (target, added, removed, prev, next, ts) {
    if (removed.length) moTrans(target, removed);
    if (!ts) return;
    moEnqueue(ts, function (w, rec) { return rec('childList', target, added, removed, prev, next, null, null); });
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
      moEnqueue(m.ts, function (w, rec) { return rec('childList', m.target, m.added, m.removed, m.prev, m.next, null, null); });
    });
  };
  var moAttr = function (id, name, old) {
    var ts = moTargets(id, 'attributes', name);
    if (ts) moEnqueue(ts, function (w, rec) { return rec('attributes', id, [], [], -1, -1, name, w ? old : null); });
  };
  var moChar = function (id, old) {
    var ts = moTargets(id, 'characterData', null);
    if (ts) moEnqueue(ts, function (w, rec) { return rec('characterData', id, [], [], -1, -1, null, w ? old : null); });
  };
  var moClearTrans = function (st) {
    for (var i = 0; i < st.trans.length; i++) {
      var t = st.trans[i], rs = xr.mo.reg[t.id], k = rs ? rs.indexOf(t.reg) : -1;
      if (k >= 0) { rs.splice(k, 1); xr.mo.n--; if (!rs.length) delete xr.mo.reg[t.id]; }
    }
    st.trans = [];
  };
  var moNotify = function () {
    moQueued = false;
    var list = moPending;
    moPending = [];
    list.sort(function (a, b) { return a.id - b.id; });
    if (g.__perf_b) g.__perf_b();
    for (var i = 0; i < list.length; i++) {
      var st = list[i], recs = st.q;
      st.pending = false; st.q = [];
      moClearTrans(st);
      if (recs.length) { try { st.cb.call(st.obs, recs, st.obs); } catch (e) { moErr(e); } }
    }
    if (g.__perf_e) g.__perf_e();
  };
  var MO = g.MutationObserver = function MutationObserver(callback) {
    if (!new.target) throw new TypeError("Failed to construct 'MutationObserver': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError("Failed to construct 'MutationObserver': 1 argument required, but only 0 present.");
    if (typeof callback !== 'function') throw new TypeError("Failed to construct 'MutationObserver': The callback provided as parameter 1 is not a function.");
    this[MO_S] = { cb: callback, obs: this, id: ++xr.mo.seq, q: [], nodes: [], trans: [], pending: false, rec: moRec, sched: moSched, listed: false };
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
    for (k = 0; k < xr.hk.length; k++) xr.hk[k]();
    if (!st.listed) { st.listed = true; (moSts || (moSts = [])).push(st); }
    var id = target.__id, rs = xr.mo.reg[id] || (xr.mo.reg[id] = []), found = null, k;
    for (k = 0; k < rs.length; k++) if (rs[k].st === st && !rs[k].src) { found = rs[k]; break; }
    if (found) {
      found.opts = o;
      st.trans = st.trans.filter(function (t) {
        if (t.reg.src !== found) return true;
        var l = xr.mo.reg[t.id], x = l ? l.indexOf(t.reg) : -1;
        if (x >= 0) { l.splice(x, 1); xr.mo.n--; if (!l.length) delete xr.mo.reg[t.id]; }
        return false;
      });
    } else { rs.push({ st: st, opts: o, src: null }); st.nodes.push(id); xr.mo.n++; }
  };
  var moDisc = function (st) {
    st.nodes.forEach(function (id) {
      var rs = xr.mo.reg[id], k;
      if (!rs) return;
      for (k = rs.length - 1; k >= 0; k--) if (rs[k].st === st && !rs[k].src) { rs.splice(k, 1); xr.mo.n--; }
      if (!rs.length) delete xr.mo.reg[id];
    });
    moClearTrans(st);
    st.nodes = []; st.q = [];
  };
  MO.prototype.disconnect = function () { moDisc(moSt(this, 'disconnect')); };
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
  // ---- QY6g: IntersectionObserver / ResizeObserver / PerformanceObserver.
  // IO (QY57) uses the real layout rects (the QJ4 `gRect`/`gViewRect`
  // geometry): rootBounds = viewport (or the explicit root's padding box)
  // grown by rootMargin, the target rect clipped by overflow ancestors and
  // the root, thresholds per the spec's previous-threshold-index rule. Entries
  // are computed at delivery time (one flush per timer turn, one cached
  // layout shared by every observer) and queued only when the crossing state
  // changes; re-checks follow scrolling, DOM writes and viewport resizes.
  // RO still reports the size from the `width`/`height` attributes, else 0x0
  // (a lie of omission). No hooks or state exist until the first `observe()`.
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
  var roRect = function (x, y, w, h) { return new DRO(x, y, w, h); };
  var attrPx = function (el, a) { var v = parseFloat(el.getAttribute(a)); return v >= 0 && isFinite(v) ? v : 0; };
  // Connected and no `display:none` on the element or an ancestor.
  var ioShown = function (el) {
    var id = el.__id;
    if (!ceIsConn(id)) return false;
    try {
      if (__d_cs_hidden(id)) return false;
    } catch (e) { /* no style info: assume shown */ }
    return true;
  };
  // QJ4: geometry reads. `__d_rect` lays the page out for the current DOM
  // (cached until the next DOM write) and returns the element's document-
  // space box [x, y, w, h, fixed, inkR, inkB, pageW, pageH], or [] when the
  // element has no box. Detached / display:none elements read as zeros.
  // QY53e2: while `gRend` is on (IO evaluating on the App's own layout) rects come from it, no layout.
  var gRend = false;
  var gRect = function (el) {
    if (!ioShown(el)) return null;
    var r = gRend ? __d_rect_r(el.__id) : __d_rect(el.__id, Number(g.innerWidth) || 0, Number(g.innerHeight) || 0);
    return r.length ? r : null;
  };
  var gElScroll = new Map(), gScrollGen = 0, gScrollable, gNid, gAncestorScroll, gSetScroll, gScrollEvent;
  var gRoot = function (el) { return el === document.documentElement; };
  var gPx = function (el, p) { return parseFloat(__d_cs(el.__id, '', p)) || 0; };
  var gClient = function (el) {
    if (gRoot(el)) return [Number(g.innerWidth) || 0, Number(g.innerHeight) || 0];
    var r = gRect(el);
    if (!r || __d_cs(el.__id, '', 'display') === 'inline') return [0, 0];
    return [Math.max(0, r[2] - gPx(el, 'border-left-width') - gPx(el, 'border-right-width')),
            Math.max(0, r[3] - gPx(el, 'border-top-width') - gPx(el, 'border-bottom-width'))];
  };
  var gViewRect = function (el) {
    var r = gRect(el);
    if (!r) return null;
    // QJ6: scrolled ancestors move their content (only walked once some
    // element has actually been scrolled, so pages that never scroll pay nothing).
    var dy = gElScroll.size ? gAncestorScroll(el, null) : 0;
    return new DR(r[0] - (r[4] ? 0 : (g.scrollX || 0)), r[1] - dy - (r[4] ? 0 : (g.scrollY || 0)), r[2], r[3]);
  };
  ElementProto.getBoundingClientRect = function () { return gViewRect(this) || new DR(0, 0, 0, 0); };
  ElementProto.getClientRects = function () {
    var v = gViewRect(this), l = v ? [v] : [];
    Object.defineProperty(l, 'item', { value: function (i) { return this[i] || null; } });
    return l;
  };
  // QJ5/QJ6: scrollIntoView scrolls each scrollable ancestor (nearest first),
  // then the window; no smooth animation.
  ElementProto.scrollIntoView = function (arg) {
    var r = gRect(this);
    if (!r) return;
    var block = 'start', inline = 'nearest';
    if (arg === false) block = 'end';
    else if (arg !== null && typeof arg === 'object') {
      if (arg.block) block = String(arg.block);
      if (arg.inline) inline = String(arg.inline);
    }
    var vw = Number(g.innerWidth) || 0, vh = Number(g.innerHeight) || 0;
    var pick = function (pos, size, view, cur, mode, page) {
      var t;
      if (mode === 'center') t = pos - (view - size) / 2;
      else if (mode === 'end') t = pos + size - view;
      else if (mode === 'nearest') {
        var s = pos - cur, e = s + size;
        if (s >= 0 && e <= view) return cur;
        if (s < 0 && e > view) return cur;
        t = (s < 0 || (size > view)) ? pos : pos + size - view;
      } else t = pos;
      return Math.max(0, Math.min(t, Math.max(0, page - view)));
    };
    // Ancestors, nearest first: only real scroll containers matter.
    var inner = 0;
    for (var a = this.parentNode; a && a.nodeType === 1; a = a.parentNode) {
      if (gRoot(a) || !gScrollable(a)) continue;
      var ar = gRect(a);
      if (!ar) continue;
      var c = gClient(a), top = ar[1] + gPx(a, 'border-top-width');
      var cur = gElScroll.get(gNid(a)) || 0;
      var ny = pick(r[1] - inner - top, r[3], c[1], cur, block, gScrollSize(a, 1));
      if (ny !== cur) gSetScroll(a, ny);
      inner += ny;
      // `inner` = scroll of the containers already handled (inside the next one).
    }
    if (r[4]) return;
    var wy = r[1] - gAncestorScroll(this, null);
    var y = pick(wy, r[3], vh, g.scrollY || 0, block, r[8]);
    var x = pick(r[0], r[2], vw, g.scrollX || 0, inline, r[7]);
    if (x !== (g.scrollX || 0) || y !== (g.scrollY || 0)) g.scrollTo(x, y);
  };
  // QJ7: DOM Range, first slice of QA19: boundary points (spec rules: IndexSizeError,
  // InvalidNodeTypeError, root change collapses, start/end ordering), comparisons,
  // toString and client rects from the laid-out glyphs of each text node. Not yet:
  // extract/clone/insert/surroundContents, live fixups on DOM mutation (QA19).
  (function () {
    var RS = Symbol('range'), ST = Symbol('rangeStates');
    var nlen = function (n) {
      var t = n.nodeType;
      return t === 3 || t === 4 || t === 7 || t === 8 ? n.length : n.childNodes.length;
    };
    var checkNode = function (fn, n) {
      if (n == null || n.__id === undefined) throw new TypeError("Failed to execute '" + fn + "' on 'Range': parameter 1 is not of type 'Node'.");
      if (n.nodeType === 10) throw herr("Failed to execute '" + fn + "' on 'Range': The node provided is a doctype.", 'InvalidNodeTypeError');
    };
    var checkOff = function (fn, n, o) {
      if (o > nlen(n)) throw herr("Failed to execute '" + fn + "' on 'Range': The offset " + o + " is larger than the node's length (" + nlen(n) + ').', 'IndexSizeError');
    };
    var idxOf = function (n) { return Array.prototype.indexOf.call(n.parentNode.childNodes, n); };
    // -1 before, 0 equal, 1 after: spec "position of a boundary point relative to another".
    var cmp = function (na, oa, nb, ob) {
      if (na === nb) return oa === ob ? 0 : (oa < ob ? -1 : 1);
      var p = na.compareDocumentPosition(nb);
      if (p & 4 && !(p & 16)) return -1;          // nb follows na, not inside it
      if (p & 2 && !(p & 8)) return 1;            // nb precedes na, not around it
      if (p & 16) {                                // na is an ancestor of nb
        var c = nb;
        while (c.parentNode !== na) c = c.parentNode;
        return idxOf(c) < oa ? 1 : -1;
      }
      // nb is an ancestor of na: compare reversed
      var c2 = na;
      while (c2.parentNode !== nb) c2 = c2.parentNode;
      return idxOf(c2) < ob ? -1 : 1;
    };
    var rootOf = function (n) { return n.getRootNode(); };
    var set = function (r, which, n, o) {
      var s = r[RS];
      if (which === 'start') {
        if (rootOf(n) !== rootOf(s.sc) || cmp(n, o, s.ec, s.eo) > 0) { s.ec = n; s.eo = o; }
        s.sc = n; s.so = o;
      } else {
        if (rootOf(n) !== rootOf(s.sc) || cmp(n, o, s.sc, s.so) < 0) { s.sc = n; s.so = o; }
        s.ec = n; s.eo = o;
      }
    };
    var Range = g.Range = function Range() {
      if (!new.target) throw new TypeError("Failed to construct 'Range': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
      this[RS] = { sc: doc, so: 0, ec: doc, eo: 0 };
    };
    Range.prototype = {};
    Object.defineProperty(Range.prototype, 'constructor', { value: Range, writable: true, configurable: true });
    Object.defineProperty(Range.prototype, Symbol.toStringTag, { value: 'Range', configurable: true });
    var st = function (r) {
      var s = r && r[RS];
      if (!s) throw new TypeError('Illegal invocation');
      return s;
    };
    [['START_TO_START', 0], ['START_TO_END', 1], ['END_TO_END', 2], ['END_TO_START', 3]].forEach(function (kv) {
      Range[kv[0]] = Range.prototype[kv[0]] = kv[1];
    });
    var getter = function (name, f) {
      Object.defineProperty(Range.prototype, name, { get: function () { return f(st(this)); }, configurable: true, enumerable: true });
    };
    getter('startContainer', function (s) { return s.sc; });
    getter('startOffset', function (s) { return s.so; });
    getter('endContainer', function (s) { return s.ec; });
    getter('endOffset', function (s) { return s.eo; });
    getter('collapsed', function (s) { return s.sc === s.ec && s.so === s.eo; });
    var commonAnc = function (s) {
      var c = s.sc;
      while (c && !(c === s.ec || c.contains(s.ec))) c = c.parentNode;
      return c;
    };
    getter('commonAncestorContainer', commonAnc);
    var meth = function (name, len, f) {
      var m = { [name]: function () {
        if (arguments.length < len) throw new TypeError("Failed to execute '" + name + "' on 'Range': " + len + ' argument' + (len > 1 ? 's' : '') + ' required, but only ' + arguments.length + ' present.');
        st(this);
        return f.apply(this, arguments);
      } }[name];
      Object.defineProperty(m, 'length', { value: len });
      Object.defineProperty(Range.prototype, name, { value: m, writable: true, configurable: true, enumerable: true });
    };
    var off = function (v) { v = Number(v); return v !== v || !isFinite(v) ? 0 : (v < 0 ? Math.ceil(v) % 4294967296 + 4294967296 : Math.floor(v) % 4294967296); };
    meth('setStart', 2, function (n, o) { checkNode('setStart', n); o = off(o); checkOff('setStart', n, o); set(this, 'start', n, o); });
    meth('setEnd', 2, function (n, o) { checkNode('setEnd', n); o = off(o); checkOff('setEnd', n, o); set(this, 'end', n, o); });
    var beforeAfter = function (name, which, after) {
      meth(name, 1, function (n) {
        checkNode(name, n);
        if (!n.parentNode) throw herr("Failed to execute '" + name + "' on 'Range': the given Node has no parent.", 'InvalidNodeTypeError');
        set(this, which, n.parentNode, idxOf(n) + (after ? 1 : 0));
      });
    };
    beforeAfter('setStartBefore', 'start', false); beforeAfter('setStartAfter', 'start', true);
    beforeAfter('setEndBefore', 'end', false); beforeAfter('setEndAfter', 'end', true);
    meth('collapse', 0, function (toStart) {
      var s = st(this);
      if (toStart) { s.ec = s.sc; s.eo = s.so; } else { s.sc = s.ec; s.so = s.eo; }
    });
    meth('selectNode', 1, function (n) {
      checkNode('selectNode', n);
      if (!n.parentNode) throw herr("Failed to execute 'selectNode' on 'Range': the given Node has no parent.", 'InvalidNodeTypeError');
      var s = st(this), i = idxOf(n);
      s.sc = s.ec = n.parentNode; s.so = i; s.eo = i + 1;
    });
    meth('selectNodeContents', 1, function (n) {
      checkNode('selectNodeContents', n);
      var s = st(this);
      s.sc = s.ec = n; s.so = 0; s.eo = nlen(n);
    });
    meth('cloneRange', 0, function () {
      var s = st(this), r = new Range(), d = r[RS];
      d.sc = s.sc; d.so = s.so; d.ec = s.ec; d.eo = s.eo;
      return r;
    });
    meth('detach', 0, function () {});
    meth('compareBoundaryPoints', 2, function (how, other) {
      var a = st(this), b = st(other);
      how = how >>> 0;
      if (how > 3) throw herr("Failed to execute 'compareBoundaryPoints' on 'Range': The comparison method provided must be one of 'START_TO_START', 'START_TO_END', 'END_TO_END', or 'END_TO_START'.", 'NotSupportedError');
      if (rootOf(a.sc) !== rootOf(b.sc)) throw herr("Failed to execute 'compareBoundaryPoints' on 'Range': The two Ranges are not in the same tree.", 'WrongDocumentError');
      var an, ao, bn, bo;
      if (how === 0) { an = a.sc; ao = a.so; bn = b.sc; bo = b.so; }
      else if (how === 1) { an = a.ec; ao = a.eo; bn = b.sc; bo = b.so; }
      else if (how === 2) { an = a.ec; ao = a.eo; bn = b.ec; bo = b.eo; }
      else { an = a.sc; ao = a.so; bn = b.ec; bo = b.eo; }
      return cmp(an, ao, bn, bo);
    });
    meth('isPointInRange', 2, function (n, o) {
      checkNode('isPointInRange', n); o = off(o);
      var s = st(this);
      if (rootOf(n) !== rootOf(s.sc)) return false;
      checkOff('isPointInRange', n, o);
      return cmp(n, o, s.sc, s.so) >= 0 && cmp(n, o, s.ec, s.eo) <= 0;
    });
    meth('comparePoint', 2, function (n, o) {
      checkNode('comparePoint', n); o = off(o);
      var s = st(this);
      if (rootOf(n) !== rootOf(s.sc)) throw herr("Failed to execute 'comparePoint' on 'Range': The node provided and the Range are not in the same tree.", 'WrongDocumentError');
      checkOff('comparePoint', n, o);
      if (cmp(n, o, s.sc, s.so) < 0) return -1;
      if (cmp(n, o, s.ec, s.eo) > 0) return 1;
      return 0;
    });
    meth('intersectsNode', 1, function (n) {
      checkNode('intersectsNode', n);
      var s = st(this);
      if (rootOf(n) !== rootOf(s.sc)) return false;
      var p = n.parentNode;
      if (!p) return true;
      var i = idxOf(n);
      return cmp(p, i, s.ec, s.eo) < 0 && cmp(p, i + 1, s.sc, s.so) > 0;
    });
    // Text nodes covered by the range, each with its [from, to) slice, in tree order.
    var covered = function (s, cb) {
      var root = s.sc === s.ec ? s.sc : commonAnc(s);
      var visit = function (n) {
        if (n.nodeType === 3 || n.nodeType === 4) {
          var a = n === s.sc ? s.so : 0, b = n === s.ec ? s.eo : n.length;
          if (b > a) cb(n, a, b);
          return;
        }
        if (n.nodeType === 8 || n.nodeType === 7) return;
        var k = n.childNodes;
        for (var i = 0; i < k.length; i++) {
          var c = k[i];
          // Skip subtrees entirely before the start or after the end.
          if (cmp(n, i + 1, s.sc, s.so) <= 0 && c !== s.sc && !c.contains(s.sc)) continue;
          if (cmp(n, i, s.ec, s.eo) >= 0 && c !== s.ec && !c.contains(s.ec)) break;
          visit(c);
        }
      };
      visit(root);
    };
    meth('toString', 0, function () {
      var s = st(this), out = '';
      covered(s, function (n, a, b) { out += n.data.slice(a, b); });
      return out;
    });
    // Per-text-node glyph rects, cached until the next DOM write or resize.
    var cacheGen = -1, cacheKey = '', tcache = Object.create(null);
    var glyphs = function (n) {
      var vw = Number(g.innerWidth) || 0, vh = Number(g.innerHeight) || 0;
      var gen = __d_ggen(), key = vw + 'x' + vh;
      if (gen !== cacheGen || key !== cacheKey) { cacheGen = gen; cacheKey = key; tcache = Object.create(null); }
      var e = tcache[n.__id];
      if (!e) e = tcache[n.__id] = __d_textrects(n.__id, vw, vh);
      return e;
    };
    var rectsOf = function (s) {
      var out = [], sx = g.scrollX || 0, sy = g.scrollY || 0;
      covered(s, function (n, a, b) {
        if (!ioShown(n.parentNode || n)) return;
        var e = glyphs(n), cur = null;
        for (var i = a; i < b; i++) {
          if (!e[i * 5]) continue;
          var x = e[i * 5 + 1], y = e[i * 5 + 2], w = e[i * 5 + 3], h = e[i * 5 + 4];
          if (cur && Math.abs(cur.y - y) < 0.5) { cur.r = Math.max(cur.r, x + w); cur.x = Math.min(cur.x, x); }
          else { cur = { x: x, y: y, r: x + w, h: h }; out.push(cur); }
        }
      });
      return out.map(function (c) { return new DR(c.x - sx, c.y - sy, c.r - c.x, c.h); });
    };
    meth('getClientRects', 0, function () {
      var l = rectsOf(st(this));
      Object.defineProperty(l, 'item', { value: function (i) { return this[i] || null; } });
      return l;
    });
    meth('getBoundingClientRect', 0, function () {
      var l = rectsOf(st(this));
      if (!l.length) return new DR(0, 0, 0, 0);
      var x = Infinity, y = Infinity, r = -Infinity, b = -Infinity;
      l.forEach(function (q) { x = Math.min(x, q.left); y = Math.min(y, q.top); r = Math.max(r, q.right); b = Math.max(b, q.bottom); });
      return new DR(x, y, r - x, b - y);
    });
    doc.createRange = function () {
      var r = new Range(), s = r[RS];
      s.sc = s.ec = this;
      return r;
    };
  })();
  defs(ElementProto, 'offsetWidth', function () { var r = gRect(this); return r ? Math.round(r[2]) : 0; });
  defs(ElementProto, 'offsetHeight', function () { var r = gRect(this); return r ? Math.round(r[3]) : 0; });
  var gOffParent = function (el) {
    var r = gRect(el);
    if (!r || r[4] || gRoot(el) || el === document.body) return null;
    var pid = __d_offset_parent(el.__id);
    return pid >= 0 ? wrap(pid) : document.body;
  };
  var gOffset = function (el, axis) {
    var r = gRect(el);
    if (!r) return 0;
    var p = gOffParent(el), v = r[axis];
    if (p && p !== document.body) {
      var pr = gRect(p);
      if (pr) v -= pr[axis] + gPx(p, axis ? 'border-top-width' : 'border-left-width');
    }
    return Math.round(v);
  };
  defs(ElementProto, 'offsetParent', function () { return gOffParent(this); });
  defs(ElementProto, 'offsetLeft', function () { return gOffset(this, 0); });
  defs(ElementProto, 'offsetTop', function () { return gOffset(this, 1); });
  defs(ElementProto, 'clientWidth', function () { return Math.round(gClient(this)[0]); });
  defs(ElementProto, 'clientHeight', function () { return Math.round(gClient(this)[1]); });
  defs(ElementProto, 'clientLeft', function () { return gRect(this) ? Math.round(gPx(this, 'border-left-width')) : 0; });
  defs(ElementProto, 'clientTop', function () { return gRect(this) ? Math.round(gPx(this, 'border-top-width')) : 0; });
  var gScrollSize = function (el, axis) {
    var c = gClient(el)[axis], r = gRect(el);
    if (!r) return 0;
    if (gRoot(el)) return Math.round(Math.max(c, r[7 + axis]));
    if (axis ? r[6] <= r[1] + r[3] : r[5] <= r[0] + r[2]) return Math.round(c);
    var extent = axis ? r[6] - r[1] - gPx(el, 'border-top-width') : r[5] - r[0] - gPx(el, 'border-left-width');
    return Math.round(Math.max(c, extent));
  };
  defs(ElementProto, 'scrollWidth', function () { return gScrollSize(this, 0); });
  defs(ElementProto, 'scrollHeight', function () { return gScrollSize(this, 1); });
  // scrollTop/Left: the root element reads/writes the window offset. Other
  // elements scroll for real when they are overflow:auto/scroll boxes (QJ6):
  // the offset is clamped to scrollHeight - clientHeight, handed to the App
  // (`__d_scrollel`) and a `scroll` event follows. Vertical only: scratchy has
  // no horizontal scroll regions, so scrollLeft stays 0 (QJ6b).
  gScrollable = function (el) {
    var o = __d_cs(el.__id, '', 'overflow-y');
    return o === 'auto' || o === 'scroll';
  };
  gNid = function (el) { return __d_nid(el.__id); };
  gAncestorScroll = function (el, stop) {
    var sum = 0;
    for (var a = el.parentNode; a && a.nodeType === 1 && a !== stop; a = a.parentNode) sum += gElScroll.get(gNid(a)) || 0;
    return sum;
  };
  gSetScroll = function (el, v) {
    if (!gScrollable(el)) return;
    var max = Math.max(0, gScrollSize(el, 1) - Math.round(gClient(el)[1]));
    v = Math.max(0, Math.min(Number(v) || 0, max));
    var nid = gNid(el), cur = gElScroll.get(nid) || 0;
    if (v === cur) return;
    if (v) gElScroll.set(nid, v); else gElScroll.delete(nid);
    __d_scrollel(el.__id, v);
    gScrollGen++;
    gScrollEvent(el);
  };
  gScrollEvent = function (el) {
    sTimeout.call(g, function () { el.dispatchEvent(new g.Event('scroll')); }, 0);
  };
  var gScrollProp = function (name, axis) {
    defs(ElementProto, name, function () {
      if (gRoot(this)) return (axis ? g.scrollY : g.scrollX) || 0;
      return axis ? (gElScroll.get(gNid(this)) || 0) : 0;
    }, function (v) {
      if (gRoot(this)) { v = Math.max(0, Number(v) || 0); if (axis) g.scrollTo(g.scrollX, v); else g.scrollTo(v, g.scrollY); return; }
      if (axis) gSetScroll(this, v);
    });
  };
  gScrollProp('scrollLeft', 0); gScrollProp('scrollTop', 1);
  // Element.scroll/scrollTo/scrollBy(x, y | {top, left}); NaN keeps the axis.
  var gElScrollTo = function (el, a, rel) {
    if (gRoot(el)) { rel ? g.scrollBy.apply(g, a) : g.scrollTo.apply(g, a); return; }
    var t = scrollArgs(a[0], a[1])[1];
    if (t !== t) return;
    gSetScroll(el, rel ? (gElScroll.get(gNid(el)) || 0) + t : t);
  };
  ElementProto.scroll = ElementProto.scrollTo = function () { gElScrollTo(this, arguments, false); };
  ElementProto.scrollBy = function () { gElScrollTo(this, arguments, true); };
  // The App scrolled boxes itself (wheel): `{nodeId: y}` replaces what script
  // reads back (no scroll event for these yet: QJ6b).
  // Q33b2: the window size changed (once per frame from the App); fires `resize` when it did.
  g.__scratchy_setvp = function (w, h) {
    if (g.innerWidth === w && g.innerHeight === h) return false;
    g.innerWidth = w; g.innerHeight = h; g.outerWidth = w; g.outerHeight = h;
    g.dispatchEvent(new g.Event('resize'));
    return true;
  };
  g.__scratchy_elscroll = function (m) {
    gElScroll.clear();
    gScrollGen++;
    for (var k in m) if (m[k]) gElScroll.set(Number(k), m[k]);
  };
  var obsErr = function (what, e) {
    __scratchy_err('scratchy: js: uncaught exception in ' + what + ' callback: ' + (e && e.stack ? String(e) + '\n' + e.stack : String(e)));
  };
  var obsSeq = 0, obsPending = [], obsQueued = false;
  var obsFlush = function () {
    obsQueued = false;
    var list = obsPending; obsPending = [];
    list.sort(function (a, b) { return a.id - b.id; });
    if (g.__perf_b) g.__perf_b();
    for (var i = 0; i < list.length; i++) {
      var st = list[i];
      st.pending = false;
      if (st.need) ioEval(st);
      var recs = st.q; st.q = [];
      if (recs.length) { try { st.cb.call(st.obs, recs, st.obs); } catch (e) { obsErr(st.name, e); } }
    }
    if (g.__perf_e) g.__perf_e();
    if (ioRendNext) { ioRendNext = false; __d_rend_clear(); }
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
    this[IO_S] = { cb: callback, obs: this, id: ++obsSeq, q: [], targets: [], prev: new Map(), need: false, pending: false, name: 'IntersectionObserver', root: root, rootMargin: margin, thresholds: Object.freeze(th) };
  };
  var ioSt = function (o, fn) {
    var st = o && o[IO_S];
    if (!st) throw new TypeError("Failed to execute '" + fn + "' on 'IntersectionObserver': Illegal invocation");
    return st;
  };
  var ioTarget = function (fn, t) {
    if (!t || t.__id === undefined || t.nodeType !== 1) throw new TypeError("Failed to execute '" + fn + "' on 'IntersectionObserver': parameter 1 is not of type 'Element'.");
  };
  // QY57: observers with targets, re-checked when the page scrolls, writes
  // the DOM or resizes (cheap signature compare, only while one exists).
  // QY53e1: a recheck caused ONLY by a geometry-generation change is deferred: free when the
  // geometry is already fresh (a script read laid it out), else evaluated at the first of the end
  // of a task after which it is fresh, or `ioDue` (one timer; later writes do not move it).
  // Scroll, viewport and element scroll keep the immediate path.
  var ioActive = new Set(), ioSig = '', ioGen = -1, ioTickQ = false, ioDueT = null, ioRendNext = false;
  var ioHardSig = function () { return g.innerWidth + 'x' + g.innerHeight + ':' + g.scrollX + ',' + g.scrollY + ':' + gScrollGen; };
  var ioRecheck = function () {
    ioTickQ = false;
    if (ioDueT !== null) { g.clearTimeout(ioDueT); ioDueT = null; }
    ioSig = ioHardSig(); ioGen = __d_ggen();
    ioActive.forEach(function (st) { st.need = true; obsSchedule(st); });
  };
  var ioNudge = function () {
    if (!ioActive.size || ioTickQ) return;
    ioTickQ = true;
    sTimeout.call(g, ioRecheck, 0);
  };
  g.__scratchy_ioNudge = ioNudge;
  g.__scratchy_ioTick = function () {
    if (!ioActive.size || ioTickQ) return;
    if (ioHardSig() !== ioSig) { ioNudge(); return; }
    var gen = __d_ggen();
    if (gen === ioGen) return;
    if (__d_geom_fresh(Number(g.innerWidth) || 0, Number(g.innerHeight) || 0)) { ioNudge(); return; }
    if (ioDueT === null) {
      var d = Math.min(2000, Math.max(100, 4 * __d_geom_cost()));
      ioDueT = sTimeout.call(g, function () { ioDueT = null; ioNudge(); }, d);
    }
  };
  // QY53e2: the App finished a complete layout of this very DOM (`__d_rend_ok`): is a geometry-only
  // recheck waiting? (cheap; the App builds its rect map only when this says yes)
  g.__scratchy_ioWant = function () {
    return ioActive.size > 0 && !ioTickQ && ioHardSig() === ioSig && __d_ggen() !== ioGen;
  };
  // ... then evaluate every observer on it, with no layout of our own.
  g.__scratchy_ioRendered = function () {
    if (!g.__scratchy_ioWant() || !__d_rend_ok(Number(g.innerWidth) || 0, Number(g.innerHeight) || 0)) return;
    ioRendNext = true;
    ioRecheck();
  };
  var ioPx = function (v, size) { var n = parseFloat(v) || 0; return v.slice(-1) === '%' ? size * n / 100 : n; };
  // [l, t, r, b] viewport-space padding box of an overflow ancestor / root.
  var ioPad = function (el) {
    var v = gViewRect(el);
    if (!v) return null;
    var bl = gPx(el, 'border-left-width'), bt = gPx(el, 'border-top-width');
    return [v.x + bl, v.y + bt, v.x + v.width - gPx(el, 'border-right-width'), v.y + v.height - gPx(el, 'border-bottom-width')];
  };
  var ioThIdx = function (th, ratio) {
    for (var i = 0; i < th.length; i++) if (th[i] > ratio) return i;
    return th.length;
  };
  var ioEval0 = function (st) {
    st.need = false;
    var rootEl = st.root && st.root.nodeType === 1 && !gRoot(st.root) ? st.root : null;
    var base = rootEl ? ioPad(rootEl) : [0, 0, Number(g.innerWidth) || 0, Number(g.innerHeight) || 0];
    var rb = null;
    if (base) {
      var m = st.rootMargin.split(' '), bw = base[2] - base[0], bh = base[3] - base[1];
      rb = [base[0] - ioPx(m[3], bw), base[1] - ioPx(m[0], bh), base[2] + ioPx(m[1], bw), base[3] + ioPx(m[2], bh)];
    }
    var clips = new Map();
    var now = g.performance && g.performance.now ? g.performance.now() : 0;
    for (var i = 0; i < st.targets.length; i++) {
      var t = st.targets[i], v = rb ? gViewRect(t) : null;
      var l = 0, tp = 0, r = 0, b = 0, inRoot = !rootEl, isInt = false, il = 0, it = 0, ir = 0, ib = 0;
      if (v) {
        l = v.x; tp = v.y; r = l + v.width; b = tp + v.height;
        il = l; it = tp; ir = r; ib = b;
        isInt = true;
        // Clip by overflow ancestors between the target and the root.
        for (var a = t.parentNode; a && a.nodeType === 1; a = a.parentNode) {
          if (a === rootEl) { inRoot = true; break; }
          if (gRoot(a)) continue;
          var c = clips.get(a);
          if (c === undefined) {
            var cs = __d_cs(a.__id, '', 'overflow-x') + __d_cs(a.__id, '', 'overflow-y');
            c = cs.indexOf('hidden') >= 0 || cs.indexOf('auto') >= 0 || cs.indexOf('scroll') >= 0 || cs.indexOf('clip') >= 0 ? ioPad(a) : null;
            clips.set(a, c);
          }
          if (c) { il = Math.max(il, c[0]); it = Math.max(it, c[1]); ir = Math.min(ir, c[2]); ib = Math.min(ib, c[3]); }
        }
        il = Math.max(il, rb[0]); it = Math.max(it, rb[1]); ir = Math.min(ir, rb[2]); ib = Math.min(ib, rb[3]);
        isInt = inRoot && ir >= il && ib >= it;
      }
      var ratio = 0, area = (r - l) * (b - tp);
      if (isInt) ratio = area > 0 ? Math.min(1, (ir - il) * (ib - it) / area) : 1;
      else { il = it = ir = ib = 0; }
      var idx = ioThIdx(st.thresholds, ratio), pv = st.prev.get(t);
      if (pv && pv.idx === idx && pv.int === isInt) continue;
      st.prev.set(t, { idx: idx, int: isInt });
      var e = Object.create(IE.prototype);
      e[IE_S] = { time: now, target: t, isIntersecting: isInt, intersectionRatio: ratio,
        boundingClientRect: roRect(l, tp, r - l, b - tp), intersectionRect: new DRO(il, it, ir - il, ib - it),
        rootBounds: rb ? new DRO(rb[0], rb[1], rb[2] - rb[0], rb[3] - rb[1]) : null };
      st.q.push(e);
    }
  };
  // QOBS3: geometry reads made while this runs are the `io` reason of a forced layout (--perf-report).
  // QY53e2: on the App's layout when it is still the page as it is (else the forced path, as before).
  var ioEval = function (st) {
    __d_io(true);
    gRend = ioRendNext && __d_rend_ok(Number(g.innerWidth) || 0, Number(g.innerHeight) || 0);
    try { ioEval0(st); } finally { gRend = false; __d_io(false); }
  };
  IO.prototype.observe =function (target) {
    var st = ioSt(this, 'observe');
    if (arguments.length < 1) throw new TypeError("Failed to execute 'observe' on 'IntersectionObserver': 1 argument required, but only 0 present.");
    ioTarget('observe', target);
    if (st.targets.indexOf(target) >= 0) return;
    st.targets.push(target);
    ioActive.add(st);
    st.need = true;
    obsSchedule(st);
  };
  IO.prototype.unobserve = function (target) {
    var st = ioSt(this, 'unobserve');
    if (arguments.length < 1) throw new TypeError("Failed to execute 'unobserve' on 'IntersectionObserver': 1 argument required, but only 0 present.");
    ioTarget('unobserve', target);
    var k = st.targets.indexOf(target);
    if (k >= 0) {
      st.targets.splice(k, 1); st.prev.delete(target);
      st.q = st.q.filter(function (e) { return e[IE_S].target !== target; });
      if (!st.targets.length) ioActive.delete(st);
    }
  };
  IO.prototype.disconnect = function () { var st = ioSt(this, 'disconnect'); st.targets = []; st.q = []; st.prev.clear(); st.need = false; ioActive.delete(st); };
  IO.prototype.takeRecords = function () { var st = ioSt(this, 'takeRecords'); if (st.need) ioEval(st); var q = st.q; st.q = []; return q; };
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
    e[RE_S] = { target: target, contentRect: roRect(0, 0, w, h), borderBoxSize: [rzSize(w, h)], contentBoxSize: [rzSize(w, h)], devicePixelContentBoxSize: [rzSize(w, h)] };
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
  // QSD3 (B1): real ShadowRoot nodes that JS can see and use, NOT rendered: only the
  // host's light children render (QSD6 will walk the composed tree). A shadow root is
  // a detached kind-11 node registered in the Rust side table host <-> root.
  var SR = function ShadowRoot() { throw new TypeError('Illegal constructor'); };
  SR.prototype = Object.create(DocumentFragment.prototype, { constructor: { value: SR, writable: true, configurable: true } });
  g.ShadowRoot = SR;
  var srMode = {}, srOpts = {}, srAdopted = {}, srWarned = false;
  var SRP = SR.prototype;
  Object.defineProperty(SRP, Symbol.toStringTag, { value: 'ShadowRoot', configurable: true });
  def(SRP, 'host', function () { return wrap(__d_shadow_host(this.__id)); });
  def(SRP, 'mode', function () { return srMode[this.__id]; });
  def(SRP, 'delegatesFocus', function () { return !!srOpts[this.__id].delegatesFocus; });
  def(SRP, 'slotAssignment', function () { return srOpts[this.__id].slotAssignment; });
  def(SRP, 'clonable', function () { return !!srOpts[this.__id].clonable; });
  def(SRP, 'serializable', function () { return !!srOpts[this.__id].serializable; });
  def(SRP, 'activeElement', function () { return null; });
  def(SRP, 'pointerLockElement', function () { return null; });
  def(SRP, 'fullscreenElement', function () { return null; });
  def(SRP, 'styleSheets', function () { return []; });
  defs(SRP, 'innerHTML', function () { return __d_gethtml(this.__id, false); },
    function (v) { __d_sethtml(this.__id, v == null ? '' : String(v)); });
  SRP.getElementById = function (i) { return wrap(__d_byid(String(i), this.__id)); };
  if (!DocumentFragment.prototype.getElementById) DocumentFragment.prototype.getElementById = SRP.getElementById;
  Object.defineProperty(g, '__scratchy_shadow_host', { value: function (n) { return n && n.__id !== undefined ? wrap(__d_shadow_host(n.__id)) : null; } });
  SRP.getSelection = function () { return null; };
  SRP.elementFromPoint = function () { return null; };
  SRP.elementsFromPoint = function () { return []; };
  defs(SRP, 'adoptedStyleSheets', function () { return srAdopted[this.__id] || (srAdopted[this.__id] = []); },
    function (v) {
      var m = "Failed to set the 'adoptedStyleSheets' property on 'ShadowRoot': ";
      if (v == null || typeof v !== 'object' || typeof v.length !== 'number') throw new TypeError(m + 'The provided value cannot be converted to a sequence.');
      var copy = Array.prototype.slice.call(v);
      copy.forEach(function (x) { if (!(x instanceof g.CSSStyleSheet)) throw new TypeError(m + "Failed to convert value to 'CSSStyleSheet'."); });
      srAdopted[this.__id] = copy; // stored only; applied by QSD7
    });
  var SHADOW_HOSTS = ['article', 'aside', 'blockquote', 'body', 'div', 'footer', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'header', 'main', 'nav', 'p', 'section', 'span'];
  var validCustom = function (t) { return t.indexOf('-') > 0 && t === t.toLowerCase() && !/^(annotation-xml|color-profile|font-face|font-face-src|font-face-uri|font-face-format|font-face-name|missing-glyph)$/.test(t); };
  ElementProto.attachShadow = function (init) {
    var M = "Failed to execute 'attachShadow' on 'Element': ";
    if (init == null || (typeof init !== 'object' && typeof init !== 'function')) throw new TypeError(M + '1 argument required, but only 0 present.');
    var mode = init.mode === undefined ? undefined : String(init.mode);
    if (mode !== 'open' && mode !== 'closed') throw new TypeError(M + "Failed to read the 'mode' property from 'ShadowRootInit': The provided value '" + mode + "' is not a valid enum value of type ShadowRootMode.");
    var sa = init.slotAssignment === undefined ? 'named' : String(init.slotAssignment);
    if (sa !== 'named' && sa !== 'manual') throw new TypeError(M + "Failed to read the 'slotAssignment' property from 'ShadowRootInit': The provided value '" + sa + "' is not a valid enum value of type SlotAssignmentMode.");
    var raw = __d_tag(this.__id), tg = raw.toLowerCase();
    if (raw !== raw.toUpperCase() || (SHADOW_HOSTS.indexOf(tg) < 0 && !validCustom(tg))) throw new DOMException(M + 'This element does not support attachShadow', 'NotSupportedError');
    if (__d_shadow_of(this.__id) >= 0) throw new DOMException(M + 'Shadow root cannot be created on a host which already hosts a shadow tree.', 'NotSupportedError');
    var root = wrap(__d_create(11, ''));
    Object.setPrototypeOf(root, SRP);
    __d_shadow_set(this.__id, root.__id); srHosts.push(this.__id);
    srMode[root.__id] = mode;
    srOpts[root.__id] = { delegatesFocus: !!init.delegatesFocus, slotAssignment: sa, clonable: !!init.clonable, serializable: !!init.serializable };
    owners[root.__id] = this.ownerDocument;
    if (!srWarned) { srWarned = true; __scratchy_err("scratchy: js: not implemented: shadow root rendering (QSD6); shadow content is not rendered, only the host's light children"); }
    return root;
  };
  def(ElementProto, 'shadowRoot', function () {
    var r = __d_shadow_of(this.__id);
    return r >= 0 && srMode[r] === 'open' ? wrap(r) : null;
  });
})(globalThis);
"#;
