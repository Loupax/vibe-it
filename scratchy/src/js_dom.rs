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
    fn get(&self, id: i32) -> Option<Handle> {
        usize::try_from(id).ok().and_then(|i| self.nodes.get(i)).cloned()
    }
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
    clear_children(h);
    for k in parse_fragment_in(h, html) {
        k.parent.set(Some(Rc::downgrade(h)));
        h.children.borrow_mut().push(k);
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
    use html5ever::serialize::{serialize, SerializeOpts, TraversalScope};
    let mut out = Vec::new();
    let opts = SerializeOpts {
        traversal_scope: if outer { TraversalScope::IncludeNode } else { TraversalScope::ChildrenOnly(None) },
        ..Default::default()
    };
    let _ = serialize(&mut out, &markup5ever_rcdom::SerializableHandle::from(h.clone()), opts);
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
    if deep {
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
    native!("__d_byid", move |id: String| -> i32 {
        let mut t = t.borrow_mut();
        let root = t.get(0).unwrap();
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
        t.borrow().get(id).map_or(0, |h| if is_fragment(id, &h) { 11 } else { node_type(&h) })
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
            NodeData::Document if id != 0 => "#document-fragment".into(),
            NodeData::Document => "#document".into(),
            _ => String::new(),
        }
    });
    let t = table.clone();
    native!("__d_attr", move |id: i32, name: String| -> Option<String> {
        let h = t.borrow().get(id)?;
        element_attr(&h, &name.to_ascii_lowercase()).map(|v| v.to_string())
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
    let t = table.clone();
    native!("__d_parent", move |id: i32| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id).and_then(|h| parent_handle(&h)) {
            Some(p) => t.intern(&p),
            None => -1,
        }
    });
    // 0 = documentElement, 1 = head, 2 = body.
    let t = table.clone();
    native!("__d_special", move |which: i32| -> i32 {
        let mut t = t.borrow_mut();
        let root = t.get(0).unwrap();
        let mut all = Vec::new();
        descendants(&root, &mut all);
        let want = ["html", "head", "body"][which.clamp(0, 2) as usize];
        match all.iter().find(|h| element_tag(h) == want) {
            Some(h) => t.intern(h),
            None => -1,
        }
    });
    // QA16: serialised computed value of `prop` (dashed) for element `id`.
    let (t, gn) = (table.clone(), wgen.clone());
    let cache = Rc::new(RefCell::new(js_style::StyleCache::default()));
    native!("__d_cs", move |id: i32, pseudo: String, prop: String| -> String {
        let (root, h) = {
            let t = t.borrow();
            (t.get(0).unwrap(), t.get(id))
        };
        let Some(h) = h else { return String::new() };
        let p = pseudo.trim_start_matches(':').to_ascii_lowercase();
        let p = match p.as_str() {
            "before" => Some("before"),
            "after" => Some("after"),
            _ => None,
        };
        match cache.borrow_mut().style_of(&root, gn.get(), &h, p) {
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
        let ok = insert_child(&p, &c, t.get(before).as_ref(), is_fragment(child, &c));
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
  var doc = g.document, cache = {};
  var queries;
  var wrap = function (id) {
    if (id === undefined || id < 0) return null;
    if (cache[id]) return cache[id];
    var k = __d_kind(id), tg = k === 1 ? __d_tag(id).toLowerCase() : '';
    var o = Object.create(k === 1 ? (tg === 'dialog' ? HTMLDialogElement.prototype : tg === 'canvas' ? HTMLCanvasElement.prototype : HTMLElement.prototype)
      : k === 3 ? Text.prototype : k === 8 ? Comment.prototype : k === 11 ? DocumentFragment.prototype : Node.prototype);
    Object.defineProperty(o, '__id', { value: id });
    if (__d_kind(id) === 11) { queries(o); }
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
  def(NodeProto, 'ownerDocument', function () { return this.nodeType === 9 ? null : doc; });
  def(CharacterData.prototype, 'data', function () { return __d_text(this.__id); });
  def(CharacterData.prototype, 'length', function () { return __d_text(this.__id).length; });
  def(NodeProto, 'parentNode', function () { return wrap(__d_parent(this.__id)); });
  def(NodeProto, 'parentElement', function () {
    var p = this.parentNode; return p && p.nodeType === 1 ? p : null;
  });
  def(NodeProto, 'childNodes', function () { return wrapAll(__d_children(this.__id, false)); });
  def(NodeProto, 'firstChild', function () { return this.childNodes[0] || null; });
  def(NodeProto, 'lastChild', function () { var c = this.childNodes; return c[c.length - 1] || null; });
  var sib = function (n, d) {
    var p = n.parentNode; if (!p) return null;
    var c = p.childNodes;
    return c[c.indexOf(n) + d] || null;
  };
  def(NodeProto, 'nextSibling', function () { return sib(this, 1); });
  def(NodeProto, 'previousSibling', function () { return sib(this, -1); });
  NodeProto.hasChildNodes = function () { return __d_children(this.__id, false).length > 0; };
  NodeProto.contains = function (n) {
    for (; n; n = n.parentNode) if (n === this) return true;
    return false;
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
  def(ElementProto, 'tagName', function () { return __d_tag(this.__id); });
  def(ElementProto, 'localName', function () { return __d_tag(this.__id).toLowerCase(); });
  def(ElementProto, 'id', function () { var v = __d_attr(this.__id, 'id'); return v == null ? '' : v; });
  def(ElementProto, 'className', function () { var v = __d_attr(this.__id, 'class'); return v == null ? '' : v; });
  def(ElementProto, 'children', function () { return wrapAll(__d_children(this.__id, true)); });
  def(ElementProto, 'firstElementChild', function () { return this.children[0] || null; });
  def(ElementProto, 'lastElementChild', function () { var c = this.children; return c[c.length - 1] || null; });
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
  defs(ElementProto, 'innerText', function () { return __d_text(this.__id); },
    function (v) { __d_settext(this.__id, v == null ? '' : String(v)); });
  defs(ElementProto, 'innerHTML', function () { return __d_gethtml(this.__id, false); },
    function (v) { __d_sethtml(this.__id, v == null ? '' : String(v)); });
  defs(ElementProto, 'outerHTML', function () { return __d_gethtml(this.__id, true); }, function () {});
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
  var rmNode = function (n) { __d_remove(n.__id); };
  NodeProto.appendChild = function (c) {
    if (!c || c.__id === undefined) throw new TypeError("Failed to execute 'appendChild': parameter 1 is not of type 'Node'.");
    if (!__d_insert(this.__id, c.__id, -1)) throw herr('The new child element contains the parent.', 'HierarchyRequestError');
    return c;
  };
  NodeProto.insertBefore = function (c, ref) {
    if (!c || c.__id === undefined) throw new TypeError("Failed to execute 'insertBefore': parameter 1 is not of type 'Node'.");
    if (ref && ref.parentNode !== this) throw herr('The node before which the new node is to be inserted is not a child of this node.', 'NotFoundError');
    if (!__d_insert(this.__id, c.__id, ref ? ref.__id : -1)) throw herr('The new child element contains the parent.', 'HierarchyRequestError');
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
    this.insertBefore(n, old);
    if (n !== old) rmNode(old);
    return old;
  };
  NodeProto.cloneNode = function (deep) { return wrap(__d_clone(this.__id, !!deep)); };
  var many = function (args) { return Array.prototype.map.call(args, toNode); };
  ElementProto.append = function () { var s = this; many(arguments).forEach(function (n) { s.appendChild(n); }); };
  ElementProto.prepend = function () {
    var s = this, first = this.firstChild;
    many(arguments).forEach(function (n) { s.insertBefore(n, first); });
  };
  ElementProto.remove = function () { rmNode(this); };
  ElementProto.before = function () {
    var p = this.parentNode, s = this;
    if (p) many(arguments).forEach(function (n) { p.insertBefore(n, s); });
  };
  ElementProto.after = function () {
    var p = this.parentNode, nx = this.nextSibling;
    if (p) many(arguments).forEach(function (n) { p.insertBefore(n, nx); });
  };
  ElementProto.replaceWith = function () { this.before.apply(this, arguments); this.remove(); };
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
      keyup: KeyboardEvent, focus: FocusEvent, blur: FocusEvent, focusin: FocusEvent, focusout: FocusEvent }[type] || Event;
    var ev = new C(type, init);
    ev.isTrusted = true;
    return __scratchy_dispatch(t, ev);
  } });
  cache[0] = doc;
  Object.defineProperty(doc, '__id', { value: 0 });
  Object.defineProperty(doc, 'nodeType', { value: 9 });
  Object.defineProperty(doc, 'nodeName', { value: '#document' });
  queries(doc);
  doc.hasChildNodes = NodeProto.hasChildNodes;
  doc.contains = NodeProto.contains;
  def(doc, 'childNodes', Object.getOwnPropertyDescriptor(NodeProto, 'childNodes').get);
  doc.getElementById = function (id) { return wrap(__d_byid(String(id))); };
  def(doc, 'documentElement', function () { return wrap(__d_special(0)); });
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
  def(doc, 'head', function () { return wrap(__d_special(1)); });
  def(doc, 'body', function () { return wrap(__d_special(2)); });
})(globalThis);
"#;
