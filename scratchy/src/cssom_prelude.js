// QSD4: constructable stylesheets and `document.adoptedStyleSheets`.
// A constructed sheet keeps its rules as text; the adopted sheets' text
// (in adoption order, disabled ones skipped, media wrapped in @media) is
// published to the engine with `__d_adopted(text)` ONLY when it changes, so
// pages that never adopt anything pay nothing. The App appends that text
// after the page's own author CSS (see `App::adopted_css`).
(function (g) {
  var S = new WeakMap(), A = [], lastText = '', MAKE = {};
  var DE = function (m, n) { return g.DOMException ? new g.DOMException(m, n) : new Error(m); };

  // ---- text splitting ----
  // Splits CSS text into top-level rules (comments dropped, strings/parens/braces respected).
  function splitRules(text) {
    var out = [], i = 0, n = text.length, start = -1, depth = 0, paren = 0, q = '';
    while (i < n) {
      var c = text[i];
      if (q) {
        if (c === '\\') i++; else if (c === q) q = '';
      } else if (c === '/' && text[i + 1] === '*') {
        var e = text.indexOf('*/', i + 2);
        if (start < 0) { i = e < 0 ? n : e + 2; continue; }
        i = e < 0 ? n : e + 1;
      } else if (start < 0 && /\s/.test(c)) {
        // skip leading whitespace
      } else {
        if (start < 0) start = i;
        if (c === '"' || c === "'") q = c;
        else if (c === '(') paren++;
        else if (c === ')') { if (paren) paren--; }
        else if (c === '{') depth++;
        else if (c === '}') {
          if (depth) depth--;
          if (!depth && !paren) { out.push(text.slice(start, i + 1)); start = -1; }
        } else if (c === ';' && !depth && !paren) { out.push(text.slice(start, i + 1)); start = -1; }
      }
      i++;
    }
    if (start >= 0 && depth === 0 && paren === 0) out.push(text.slice(start).trim());
    else if (start >= 0) out.push(text.slice(start).trim() + '}');
    return out;
  }
  // Splits declarations on top-level ';'.
  function splitDecls(body) {
    var out = [], i = 0, n = body.length, st = 0, paren = 0, q = '';
    for (; i < n; i++) {
      var c = body[i];
      if (q) { if (c === '\\') i++; else if (c === q) q = ''; }
      else if (c === '"' || c === "'") q = c;
      else if (c === '(') paren++;
      else if (c === ')') { if (paren) paren--; }
      else if (c === ';' && !paren) { out.push(body.slice(st, i)); st = i + 1; }
    }
    out.push(body.slice(st));
    return out.map(function (s) { return s.trim(); }).filter(Boolean);
  }
  function parseDecls(body) {
    var d = [];
    splitDecls(body.replace(/\/\*[\s\S]*?\*\//g, '')).forEach(function (s) {
      var k = s.indexOf(':');
      if (k < 1) return;
      var name = s.slice(0, k).trim(), val = s.slice(k + 1).trim(), imp = false;
      var m = /\s*!\s*important\s*$/i.exec(val);
      if (m) { imp = true; val = val.slice(0, m.index).trim(); }
      if (!name || val === '') return;
      if (name.slice(0, 2) !== '--') name = name.toLowerCase();
      for (var i = 0; i < d.length; i++) if (d[i][0] === name) { d.splice(i, 1); break; }
      d.push([name, val, imp]);
    });
    return d;
  }
  var kebab = function (s) { return s.replace(/[A-Z]/g, function (c) { return '-' + c.toLowerCase(); }); };

  // ---- classes ----
  function illegalCtor(name) { return ({ [name]: function () { throw new TypeError('Illegal constructor'); } })[name]; }
  var StyleSheet = illegalCtor('StyleSheet');
  var CSSRule = illegalCtor('CSSRule');
  [['STYLE_RULE', 1], ['CHARSET_RULE', 2], ['IMPORT_RULE', 3], ['MEDIA_RULE', 4], ['FONT_FACE_RULE', 5], ['PAGE_RULE', 6],
    ['KEYFRAMES_RULE', 7], ['KEYFRAME_RULE', 8], ['NAMESPACE_RULE', 10], ['SUPPORTS_RULE', 12]].forEach(function (p) {
    Object.defineProperty(CSSRule, p[0], { value: p[1] });
    Object.defineProperty(CSSRule.prototype, p[0], { value: p[1] });
  });
  var CSSStyleRule = illegalCtor('CSSStyleRule');
  CSSStyleRule.prototype = Object.create(CSSRule.prototype, { constructor: { value: CSSStyleRule, writable: true, configurable: true } });
  var CSSGroupingRule = illegalCtor('CSSGroupingRule');
  CSSGroupingRule.prototype = Object.create(CSSRule.prototype, { constructor: { value: CSSGroupingRule, writable: true, configurable: true } });
  var CSSRuleList = illegalCtor('CSSRuleList');
  var MediaList = illegalCtor('MediaList');
  var CSSSD = ({ CSSStyleDeclaration: function CSSStyleDeclaration() { throw new TypeError('Illegal constructor'); } }).CSSStyleDeclaration;
  var sdProto = CSSSD.prototype;
  var CSSStyleSheet = function CSSStyleSheet(opts) {
    if (!(this instanceof CSSStyleSheet)) throw new TypeError("Failed to construct 'CSSStyleSheet': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var st = { rules: [], list: Object.create(CSSRuleList.prototype), disabled: false, media: null, base: null, owner: null, node: null, orig: '', last: null };
    if (opts != null && typeof opts !== 'object' && typeof opts !== 'function') throw new TypeError("Failed to construct 'CSSStyleSheet': The provided value is not of type 'CSSStyleSheetInit'.");
    var ml = Object.create(MediaList.prototype);
    S.set(ml, { text: '', owner: st });
    st.media = ml;
    if (opts) {
      if (opts.media != null) {
        var m = opts.media;
        S.get(ml).text = (m && S.has(m)) ? S.get(m).text : String(m);
      }
      st.disabled = !!opts.disabled;
      if (opts.baseURL != null) st.base = String(opts.baseURL);
    }
    st.obj = this;
    S.set(this, st);
    Object.defineProperty(st.list, 'length', { value: 0, writable: true, configurable: true });
  };
  CSSStyleSheet.prototype = Object.create(StyleSheet.prototype, { constructor: { value: CSSStyleSheet, writable: true, configurable: true } });
  var st_ = function (o, what) {
    var s = S.get(o);
    if (!s) throw new TypeError('Illegal invocation');
    return s;
  };

  // ---- publishing ----
  function ruleText(r) { return r.kind === 'style' && !r.dirty ? r.raw : r.text(); }
  function sheetText(sh) {
    var st = S.get(sh);
    if (st.disabled) return '';
    var body = st.rules.map(ruleText).join('\n');
    var mt = S.get(st.media).text.trim();
    if (mt && mt.toLowerCase() !== 'all' && body) body = '@media ' + mt + ' {\n' + body + '\n}';
    return body;
  }
  function publish() {
    var t = A.map(sheetText).filter(Boolean).join('\n');
    if (t === lastText) return;
    lastText = t;
    g.__d_adopted(t);
  }
  function touch(st) {
    if (st.node) { publishNode(st); return; }
    if (A.length) { for (var i = 0; i < A.length; i++) if (S.get(A[i]) === st) { publish(); return; } } }
  // QY44c: a `<style>` element's own sheet: its effective CSS (all rules,
  // media-wrapped, [] when disabled) goes to the engine, which uses it in place
  // of the element's text while that text is unchanged since the sheet was made.
  function publishNode(st) {
    if (st.kind !== 'style') return;
    var t = sheetText(st.obj);
    if (t === st.last) return;
    st.last = t;
    g.__d_sheet_text(st.node.__id, st.orig, t);
  }
  function syncList(st) {
    var l = st.list, old = l.length, n = st.rules.length;
    for (var i = n; i < old; i++) delete l[i];
    for (i = 0; i < n; i++) l[i] = st.rules[i].obj;
    l.length = n;
  }

  // ---- rules ----
  function makeDecl(rule) {
    var d = Object.create(sdProto), target = {};
    S.set(d, { rule: rule });
    var px = new Proxy(d, {
      get: function (t, p, r) {
        if (typeof p === 'string' && !(p in t)) {
          if (/^\d+$/.test(p)) { var e = rule.decls[+p]; return e ? e[0] : undefined; }
          if (p === 'cssFloat') p = 'float';
          return getProp(rule, kebab(p));
        }
        return Reflect.get(t, p, r);
      },
      set: function (t, p, v) {
        if (typeof p === 'string' && !(p in t)) { if (p === 'cssFloat') p = 'float'; setProp(rule, kebab(p), v == null ? '' : String(v), ''); return true; }
        return Reflect.set(t, p, v);
      }
    });
    S.set(px, { rule: rule });
    return px;
  }
  function getProp(rule, name) { for (var i = 0; i < rule.decls.length; i++) if (rule.decls[i][0] === name) return rule.decls[i][1]; return ''; }
  function setProp(rule, name, val, prio) {
    rule.dirty = true;
    if (name.slice(0, 2) !== '--') name = name.toLowerCase();
    for (var i = 0; i < rule.decls.length; i++) if (rule.decls[i][0] === name) { rule.decls.splice(i, 1); break; }
    if (val !== '') rule.decls.push([name, val, /^important$/i.test(prio || '')]);
    touch(rule.sheet);
  }
  Object.defineProperties(sdProto, {
    cssText: {
      get: function () { return S.get(this).rule.declText(); },
      set: function (v) { var r = S.get(this).rule; r.dirty = true; r.decls = parseDecls(String(v)); touch(r.sheet); }, configurable: true, enumerable: true },
    length: { get: function () { return S.get(this).rule.decls.length; }, configurable: true, enumerable: true },
    parentRule: { get: function () { return S.get(this).rule.obj; }, configurable: true, enumerable: true }
  });
  sdProto.getPropertyValue = function (n) { return getProp(S.get(this).rule, String(n).toLowerCase()); };
  sdProto.getPropertyPriority = function (n) {
    var r = S.get(this).rule, k = String(n).toLowerCase();
    for (var i = 0; i < r.decls.length; i++) if (r.decls[i][0] === k) return r.decls[i][2] ? 'important' : '';
    return '';
  };
  sdProto.setProperty = function (n, v, p) { setProp(S.get(this).rule, String(n), v == null ? '' : String(v), p); };
  sdProto.removeProperty = function (n) {
    var r = S.get(this).rule, old = getProp(r, String(n).toLowerCase());
    setProp(r, String(n), '', ''); return old;
  };
  sdProto.item = function (i) { var e = S.get(this).rule.decls[i]; return e ? e[0] : ''; };

  function newRule(sheet, raw) {
    raw = raw.trim();
    var r = { sheet: sheet, raw: raw, decls: null, selector: '', kind: 'other', dirty: false };
    var brace = raw.indexOf('{');
    if (raw[0] !== '@' && brace > 0 && raw[raw.length - 1] === '}') {
      r.kind = 'style';
      r.selector = raw.slice(0, brace).trim();
      r.decls = parseDecls(raw.slice(brace + 1, raw.length - 1));
    }
    r.declText = function () {
      return r.decls.map(function (d) { return d[0] + ': ' + d[1] + (d[2] ? ' !important' : '') + ';'; }).join(' ');
    };
    r.text = function () {
      if (r.kind !== 'style') return r.raw;
      var body = r.declText();
      return r.selector + ' { ' + (body ? body + ' ' : '') + '}';
    };
    if (r.kind === 'style') {
      r.obj = Object.create(CSSStyleRule.prototype);
      S.set(r.obj, r);
      r.style = makeDecl(r);
    } else {
      var at = /^@([a-zA-Z-]+)/.exec(raw), name = at ? at[1].toLowerCase() : '';
      r.obj = Object.create((name === 'media' || name === 'supports' || name === 'layer' && brace > 0) ? CSSGroupingRule.prototype : CSSRule.prototype);
      r.type = { media: 4, 'font-face': 5, keyframes: 7, supports: 12, charset: 2, namespace: 10, page: 6 }[name] || 0;
      S.set(r.obj, r);
    }
    return r;
  }
  Object.defineProperties(CSSRule.prototype, {
    cssText: { get: function () { return S.get(this).text(); }, set: function () {}, configurable: true, enumerable: true },
    type: { get: function () { var r = S.get(this); return r.kind === 'style' ? 1 : r.type; }, configurable: true, enumerable: true },
    parentRule: { get: function () { return null; }, configurable: true, enumerable: true },
    parentStyleSheet: { get: function () { return S.get(this).sheet.obj; }, configurable: true, enumerable: true }
  });
  Object.defineProperties(CSSStyleRule.prototype, {
    selectorText: {
      get: function () { return S.get(this).selector; },
      set: function (v) { var r = S.get(this); v = String(v).trim(); if (v && v.indexOf('{') < 0 && v.indexOf('}') < 0) { r.selector = v; r.dirty = true; touch(r.sheet); } },
      configurable: true, enumerable: true },
    style: { get: function () { return S.get(this).style; }, configurable: true, enumerable: true }
  });
  Object.defineProperties(CSSRuleList.prototype, {
    item: { value: function (i) { return this[i] || null; }, writable: true, configurable: true }
  });
  CSSRuleList.prototype[Symbol.iterator] = Array.prototype[Symbol.iterator];

  // ---- MediaList ----
  var mlist = function (o) { return S.get(o); };
  var mediaArr = function (o) { var t = mlist(o).text.trim(); return t ? t.split(',').map(function (s) { return s.trim(); }) : []; };
  var setMedia = function (o, a) { var m = mlist(o); m.text = a.join(', '); touch(m.owner); };
  Object.defineProperties(MediaList.prototype, {
    mediaText: { get: function () { return mlist(this).text; }, set: function (v) { var m = mlist(this); m.text = v == null ? '' : String(v); touch(m.owner); }, configurable: true, enumerable: true },
    length: { get: function () { return mediaArr(this).length; }, configurable: true, enumerable: true }
  });
  MediaList.prototype.item = function (i) { var a = mediaArr(this); return i >= 0 && i < a.length ? a[i] : null; };
  MediaList.prototype.appendMedium = function (m) { var a = mediaArr(this); m = String(m); if (a.indexOf(m) < 0) { a.push(m); setMedia(this, a); } };
  MediaList.prototype.deleteMedium = function (m) { var a = mediaArr(this), i = a.indexOf(String(m)); if (i >= 0) { a.splice(i, 1); setMedia(this, a); } };
  MediaList.prototype.toString = function () { return mlist(this).text; };

  // ---- CSSStyleSheet ----
  function importWarn() { if (g.console && console.warn) console.warn('@import rules are not allowed here. See https://github.com/WICG/construct-stylesheets/issues/119#issuecomment-588352418.'); }
  function load(sheet, text, owned) {
    var st = S.get(sheet);
    st.rules = [];
    var warned = false;
    splitRules(String(text)).forEach(function (raw) {
      if (/^@import\b/i.test(raw) && !owned) { if (!warned) { warned = true; importWarn(); } return; }
      if (/^@charset\b/i.test(raw)) return;
      var r = newRule(st, raw); st.rules.push(r);
    });
    syncList(st);
    touch(st);
  }
  Object.defineProperties(StyleSheet.prototype, {
    type: { get: function () { return 'text/css'; }, configurable: true, enumerable: true },
    href: { get: function () { var n = st_(this).node; return n && st_(this).kind === 'link' ? (n.href || null) : null; }, configurable: true, enumerable: true },
    ownerNode: { get: function () { return st_(this).node; }, configurable: true, enumerable: true },
    parentStyleSheet: { get: function () { return null; }, configurable: true, enumerable: true },
    title: { get: function () { var n = st_(this).node; return n && n.getAttribute('title') || null; }, configurable: true, enumerable: true },
    media: { get: function () { return st_(this).media; }, set: function (v) { S.get(st_(this).media).text = String(v); touch(st_(this)); }, configurable: true, enumerable: true },
    disabled: {
      get: function () { return st_(this).disabled; },
      set: function (v) { var st = st_(this); v = !!v; if (st.disabled !== v) { st.disabled = v; touch(st); } },
      configurable: true, enumerable: true }
  });
  Object.defineProperties(CSSStyleSheet.prototype, {
    ownerRule: { get: function () { return null; }, configurable: true, enumerable: true },
    cssRules: { get: function () { return st_(this).list; }, configurable: true, enumerable: true },
    rules: { get: function () { return st_(this).list; }, configurable: true, enumerable: true }
  });
  CSSStyleSheet.prototype.replaceSync = function (text) {
    st_(this);
    load(this, text === undefined ? 'undefined' : text);
  };
  CSSStyleSheet.prototype.replace = function (text) {
    var self = this;
    return new Promise(function (res) { st_(self); load(self, text === undefined ? 'undefined' : text); res(self); });
  };
  CSSStyleSheet.prototype.insertRule = function (rule, index) {
    var st = st_(this);
    index = index === undefined ? 0 : (index >>> 0);
    if (index > st.rules.length) throw DE("Failed to execute 'insertRule' on 'CSSStyleSheet': The index provided (" + index + ") is larger than the maximum index (" + st.rules.length + ').', 'IndexSizeError');
    var parts = splitRules(String(rule));
    if (parts.length !== 1) throw DE("Failed to execute 'insertRule' on 'CSSStyleSheet': Failed to parse the rule '" + rule + "'.", 'SyntaxError');
    if (!st.node && /^@import\b/i.test(parts[0])) throw DE("Failed to execute 'insertRule' on 'CSSStyleSheet': Can't insert @import rules into a constructed stylesheet.", 'SyntaxError');
    var r = newRule(st, parts[0]);
    st.rules.splice(index, 0, r);
    syncList(st); touch(st);
    return index;
  };
  CSSStyleSheet.prototype.deleteRule = function (index) {
    var st = st_(this);
    index = index >>> 0;
    if (index >= st.rules.length) throw DE("Failed to execute 'deleteRule' on 'CSSStyleSheet': The index provided (" + index + ") is larger than the maximum index (" + (st.rules.length - 1) + ').', 'IndexSizeError');
    st.rules.splice(index, 1);
    syncList(st); touch(st);
  };
  CSSStyleSheet.prototype.addRule = function (sel, style, index) {
    var st = st_(this);
    return this.insertRule(String(sel) + ' { ' + String(style === undefined ? '' : style) + ' }', index === undefined ? st.rules.length : index), -1;
  };
  CSSStyleSheet.prototype.removeRule = function (index) { this.deleteRule(index === undefined ? 0 : index); };

  // ---- element sheets (QY44c) ----
  var NS = new WeakMap();
  function elSheet(el) {
    var kind = el.localName === 'style' ? 'style' : 'link';
    if (!el.isConnected) return null;
    if (kind === 'link') {
      var rel = (el.getAttribute('rel') || '').toLowerCase().split(/\s+/);
      if (rel.indexOf('stylesheet') < 0) return null;
    } else {
      var ty = (el.getAttribute('type') || '').trim().toLowerCase();
      if (ty && ty !== 'text/css') return null;
    }
    var text = kind === 'style' ? el.textContent : '';
    var sh = NS.get(el);
    if (sh && S.get(sh).orig === text) return sh;
    sh = new CSSStyleSheet();
    var st = S.get(sh);
    st.kind = kind; st.orig = text;
    var mt = el.getAttribute('media');
    if (mt) S.get(st.media).text = mt;
    if (kind === 'style') load(sh, text, true);
    st.node = el; st.last = sheetText(sh);
    NS.set(el, sh);
    return sh;
  }
  Object.defineProperty(g.HTMLStyleElement.prototype, 'sheet', { get: function () { return elSheet(this); }, configurable: true, enumerable: true });
  Object.defineProperty(g.HTMLLinkElement.prototype, 'sheet', { get: function () { return elSheet(this); }, configurable: true, enumerable: true });
  var StyleSheetList = illegalCtor('StyleSheetList');
  StyleSheetList.prototype.item = function (i) { return this[i] || null; };
  StyleSheetList.prototype[Symbol.iterator] = Array.prototype[Symbol.iterator];
  Object.defineProperty(g.document, 'styleSheets', {
    configurable: true, enumerable: true,
    get: function () {
      var l = Object.create(StyleSheetList.prototype), n = 0;
      var els = g.document.querySelectorAll('style, link');
      for (var i = 0; i < els.length; i++) { var sh = elSheet(els[i]); if (sh) l[n++] = sh; }
      Object.defineProperty(l, 'length', { value: n });
      return l;
    }
  });

  // ---- document.adoptedStyleSheets ----
  function check(v) {
    if (!v || typeof v !== 'object' || !S.has(v) || !(v instanceof CSSStyleSheet)) throw new TypeError("Failed to set the 'adoptedStyleSheets' property on 'Document': Failed to convert value to 'CSSStyleSheet'.");
    return v;
  }
  var proxy = new Proxy(A, {
    set: function (t, p, v) {
      if (typeof p === 'string' && /^\d+$/.test(p)) check(v);
      t[p] = v; publish(); return true;
    },
    deleteProperty: function (t, p) { delete t[p]; publish(); return true; }
  });
  Object.defineProperty(g.document, 'adoptedStyleSheets', {
    configurable: true, enumerable: true,
    get: function () { return proxy; },
    set: function (v) {
      if (v == null || typeof v !== 'object' || typeof v.length !== 'number') throw new TypeError("Failed to set the 'adoptedStyleSheets' property on 'Document': The provided value cannot be converted to a sequence.");
      var copy = Array.prototype.slice.call(v);
      copy.forEach(check);
      A.length = 0;
      for (var i = 0; i < copy.length; i++) A[i] = copy[i];
      publish();
    }
  });

  [['StyleSheet', StyleSheet], ['CSSStyleSheet', CSSStyleSheet], ['CSSRule', CSSRule], ['CSSStyleRule', CSSStyleRule],
    ['CSSGroupingRule', CSSGroupingRule], ['CSSRuleList', CSSRuleList], ['StyleSheetList', StyleSheetList], ['MediaList', MediaList]].forEach(function (p) {
    Object.defineProperty(g, p[0], { value: p[1], writable: true, configurable: true, enumerable: false });
  });
})(globalThis);
