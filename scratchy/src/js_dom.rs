//! Q33b: the read-only DOM API for page scripts. Nodes live in Rust
//! (`Handle`s in a table); JS sees small integer ids through native
//! `__d_*` functions, and a JS-side wrapper per id (cached, so `===`
//! holds) turns them into `Element`-like objects. Document id is 0.
use super::*;
use markup5ever_rcdom::Node;
use rquickjs::{Ctx, Function};

#[derive(Default)]
struct DomTable {
    nodes: Vec<Handle>,
    ids: HashMap<usize, i32>,
}

impl DomTable {
    fn intern(&mut self, h: &Handle) -> i32 {
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
    let ctx = match &h.data {
        NodeData::Element { name, .. } => name.clone(),
        _ => html_name("div"),
    };
    let mut opts = html5ever::driver::ParseOpts::default();
    opts.tree_builder.scripting_enabled = js_enabled();
    let dom = html5ever::driver::parse_fragment(RcDom::default(), opts, ctx, Vec::new(), false).one(html);
    // The parser wraps the fragment in one synthetic root element.
    let root = dom.document.children.borrow().first().cloned();
    if let Some(root) = root {
        let kids: Vec<Handle> = root.children.borrow_mut().drain(..).collect();
        for k in kids {
            k.parent.set(Some(Rc::downgrade(h)));
            h.children.borrow_mut().push(k);
        }
    }
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
pub(crate) fn install_dom(ctx: &Ctx<'_>, document: &Handle, dirty: Rc<Cell<bool>>) {
    let table = Rc::new(RefCell::new(DomTable::default()));
    table.borrow_mut().intern(document);
    let globals = ctx.globals();
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
    // ---- Q33c mutation. Every write marks the page dirty; the event
    // loop rebuilds layout once per turn, not once per call.
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_setattr", move |id: i32, name: String, val: String| {
        if let Some(h) = t.borrow().get(id) {
            set_attr(&h, &name.to_ascii_lowercase(), &val);
            d.set(true);
        }
    });
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_rmattr", move |id: i32, name: String| {
        if let Some(h) = t.borrow().get(id) {
            remove_attr(&h, &name.to_ascii_lowercase());
            d.set(true);
        }
    });
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_settext", move |id: i32, text: String| {
        if let Some(h) = t.borrow().get(id) {
            match &h.data {
                NodeData::Text { contents } => *contents.borrow_mut() = text.as_str().into(),
                NodeData::Comment { .. } => {}
                _ => set_children_text(&h, &text),
            }
            d.set(true);
        }
    });
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_sethtml", move |id: i32, html: String| {
        if let Some(h) = t.borrow().get(id) {
            set_inner_html(&h, &html);
            d.set(true);
        }
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
    let t = table.clone();
    native!("__d_clone", move |id: i32, deep: bool| -> i32 {
        let mut t = t.borrow_mut();
        match t.get(id) {
            Some(h) => t.intern(&clone_node(&h, deep)),
            None => -1,
        }
    });
    // `before` < 0 appends. `false` = HierarchyRequestError.
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_insert", move |parent: i32, child: i32, before: i32| -> bool {
        let t = t.borrow();
        let (Some(p), Some(c)) = (t.get(parent), t.get(child)) else { return false };
        if !matches!(p.data, NodeData::Element { .. } | NodeData::Document) {
            return false;
        }
        let ok = insert_child(&p, &c, t.get(before).as_ref(), is_fragment(child, &c));
        d.set(true);
        ok
    });
    let (t, d) = (table.clone(), dirty.clone());
    native!("__d_remove", move |id: i32| {
        if let Some(h) = t.borrow().get(id) {
            detach(&h);
            d.set(true);
        }
    });
    let _ = ctx.eval::<(), _>(DOM_PRELUDE);
}

/// Wrappers over the natives; runs after the base prelude (which defines
/// `document`). Element event listeners are stored but never fired yet.
const DOM_PRELUDE: &str = r#"
(function (g) {
  var doc = g.document, cache = {}, elListeners = {};
  var queries;
  var wrap = function (id) {
    if (id === undefined || id < 0) return null;
    if (cache[id]) return cache[id];
    var o = Object.create(__d_kind(id) === 1 ? HTMLElement.prototype : Node.prototype);
    Object.defineProperty(o, '__id', { value: id });
    if (__d_kind(id) === 11) { queries(o); }
    return (cache[id] = o);
  };
  var wrapAll = function (ids) { return ids.map(function (id) { return wrap(id); }); };
  var bad = function (sel) {
    var e = new Error("'" + sel + "' is not a valid selector");
    e.name = 'SyntaxError';
    return e;
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
  NodeProto.addEventListener = function (type, fn) {
    if (typeof fn !== 'function') return;
    var k = this.__id + ':' + type;
    (elListeners[k] = elListeners[k] || []).push(fn);
  };
  NodeProto.removeEventListener = function (type, fn) {
    var l = elListeners[this.__id + ':' + type] || [], i = l.indexOf(fn);
    if (i >= 0) l.splice(i, 1);
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
  var herr = function (msg, name) { var e = new Error(msg); e.name = name; return e; };
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
  ElementProto.setAttribute = function (n, v) { __d_setattr(this.__id, String(n), String(v)); };
  ElementProto.removeAttribute = function (n) { __d_rmattr(this.__id, String(n)); };
  ElementProto.toggleAttribute = function (n, force) {
    var has = this.hasAttribute(n), want = force === undefined ? !has : !!force;
    if (want && !has) this.setAttribute(n, ''); else if (!want && has) this.removeAttribute(n);
    return want;
  };
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
    if (!c || c.parentNode !== this) throw herr('The node to be removed is not a child of this node.', 'NotFoundError');
    __d_remove(c.__id);
    return c;
  };
  NodeProto.replaceChild = function (n, old) {
    if (!old || old.parentNode !== this) throw herr('The node to be replaced is not a child of this node.', 'NotFoundError');
    this.insertBefore(n, old);
    if (n !== old) __d_remove(old.__id);
    return old;
  };
  NodeProto.cloneNode = function (deep) { return wrap(__d_clone(this.__id, !!deep)); };
  var many = function (args) { return Array.prototype.map.call(args, toNode); };
  ElementProto.append = function () { var s = this; many(arguments).forEach(function (n) { s.appendChild(n); }); };
  ElementProto.prepend = function () {
    var s = this, first = this.firstChild;
    many(arguments).forEach(function (n) { s.insertBefore(n, first); });
  };
  ElementProto.remove = function () { __d_remove(this.__id); };
  ElementProto.before = function () {
    var p = this.parentNode, s = this;
    if (p) many(arguments).forEach(function (n) { p.insertBefore(n, s); });
  };
  ElementProto.after = function () {
    var p = this.parentNode, nx = this.nextSibling;
    if (p) many(arguments).forEach(function (n) { p.insertBefore(n, nx); });
  };
  ElementProto.replaceWith = function () { this.before.apply(this, arguments); this.remove(); };
  doc.createElement = function (t) { return wrap(__d_create(1, String(t))); };
  doc.createElementNS = function (ns, t) { return wrap(__d_create(1, String(t))); };
  doc.createTextNode = function (s) { return wrap(__d_create(3, String(s))); };
  doc.createComment = function (s) { return wrap(__d_create(8, String(s))); };
  doc.createDocumentFragment = function () { return wrap(__d_create(11, '')); };
  doc.appendChild = NodeProto.appendChild;
  doc.insertBefore = NodeProto.insertBefore;
  doc.removeChild = NodeProto.removeChild;
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
  def(doc, 'head', function () { return wrap(__d_special(1)); });
  def(doc, 'body', function () { return wrap(__d_special(2)); });
})(globalThis);
"#;
