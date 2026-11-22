// QAPI4: `document.fonts` (FontFaceSet), `FontFace` and `FontFaceSetLoadEvent`. Runs after the CSSOM prelude.
// The document's set mirrors the @font-face rules of its style sheets (read lazily through document.styleSheets,
// one FontFace per rule) plus the faces the page adds. QFF1a: the faces are real. A rule's face follows the
// load the renderer started for it (`__scratchy_font_state(key)`, re-read by `__scratchy_fonts_sync`, which the
// App calls when a load starts or ends). A face's `load()` fetches its first supported url() source through the
// page's `fetch` (so only wire ever sees the request: this file opens no sockets); the bytes are decoded by
// `__scratchy_font_add` (a face in document.fonts: used for rendering) or `__scratchy_font_check`, and the face
// ends 'loaded' or 'error'. Families that no face names are "system fonts": `check()` is true for them and
// `load()` resolves with an empty list.
(function (g) {
  var ET = g.EventTarget;
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function hid(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function DE(m, n) { return new g.DOMException(m, n); }
  function quiet(p) { p.then(null, function () {}); return p; }
  function ctor(name, fn) { return ({ [name]: fn })[name]; }
  var TOKEN = {};
  var nat = function (n) { return typeof g[n] === 'function' ? g[n] : null; };
  var fState = nat('__scratchy_font_state'), fAdd = nat('__scratchy_font_add'), fCheck = nat('__scratchy_font_check'), fRemove = nat('__scratchy_font_remove');
  var serial = 0;

  // ---- descriptor parsing helpers ----
  var SRC_RE = /^(?:\s*(?:url\(\s*(?:"[^"]*"|'[^']*'|[^)\s]*)\s*\)|local\(\s*[^)]*\))(?:\s*format\([^)]*\))?(?:\s*tech\([^)]*\))?\s*(?:,|$))+\s*$/;
  function firstUrl(src) {
    var m = /url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)/.exec(src);
    return m ? (m[1] !== undefined ? m[1] : m[2] !== undefined ? m[2] : m[3]) : null;
  }
  // QFF1a: the registry key of a face: family (whitespace collapsed, lower case) | the first source as written.
  function famKey(f) { return unquote(f).replace(/\s+/g, ' ').trim().toLowerCase(); }
  function srcKey(src) {
    var first = splitTop(String(src || ''), ',')[0] || '', m;
    if (/^\s*local\(/i.test(first)) { m = /local\(([\s\S]*)\)/i.exec(first); return 'local(' + unquote(m ? m[1] : '').replace(/\s+/g, ' ').trim() + ')'; }
    return firstUrl(first) || '';
  }
  // The first url() whose format scratchy decodes (ttf/otf/woff; woff2, svg and eot are skipped).
  function usableUrl(src) {
    var items = splitTop(String(src || ''), ','), i, u, f;
    for (i = 0; i < items.length; i++) {
      u = firstUrl(items[i]);
      if (u === null) continue;
      f = /format\(\s*["']?([^"')]*)/i.exec(items[i]);
      if (f) { if (/^(truetype|opentype|woff|collection)$/i.test(f[1].trim())) return u; continue; }
      if (!/\.(woff2|svgz?|eot)(?:[?#]|$)/i.test(u)) return u;
    }
    return null;
  }
  function unquote(s) { s = String(s).trim(); var m = /^(["'])(.*)\1$/.exec(s); return m ? m[2] : s; }
  function splitTop(text, sep) {
    var out = [], cur = '', q = '', depth = 0, i, c;
    for (i = 0; i < text.length; i++) {
      c = text[i];
      if (q) { cur += c; if (c === '\\') cur += text[++i] || ''; else if (c === q) q = ''; continue; }
      if (c === '"' || c === "'") q = c;
      else if (c === '(') depth++;
      else if (c === ')') { if (depth) depth--; }
      else if (c === sep && !depth) { out.push(cur); cur = ''; continue; }
      cur += c;
    }
    out.push(cur);
    return out;
  }

  // ---- FontFace ----
  var DESC = ['style', 'weight', 'stretch', 'unicodeRange', 'variant', 'featureSettings', 'variationSettings', 'display', 'ascentOverride', 'descentOverride', 'lineGapOverride'];
  var DEFAULTS = { style: 'normal', weight: 'normal', stretch: 'normal', unicodeRange: 'U+0-10FFFF', variant: 'normal', featureSettings: 'normal', variationSettings: 'normal', display: 'auto', ascentOverride: 'normal', descentOverride: 'normal', lineGapOverride: 'normal' };
  var CSSNAME = { style: 'font-style', weight: 'font-weight', stretch: 'font-stretch', unicodeRange: 'unicode-range', variant: 'font-variant', featureSettings: 'font-feature-settings', variationSettings: 'font-variation-settings', display: 'font-display', ascentOverride: 'ascent-override', descentOverride: 'descent-override', lineGapOverride: 'line-gap-override' };

  function setStatus(face, st) { face.__status = st; }
  var FontFace = ctor('FontFace', function FontFace(family, source) {
    if (!new.target) throw new TypeError("Failed to construct 'FontFace': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 2) throw new TypeError("Failed to construct 'FontFace': 2 arguments required, but only " + arguments.length + ' present.');
    var desc = arguments[2] || {}, self = this, i, k;
    hid(this, '__family', String(family));
    hid(this, '__status', 'unloaded');
    hid(this, '__src', null);
    hid(this, '__sets', []);
    hid(this, '__d', {});
    for (i = 0; i < DESC.length; i++) {
      k = DESC[i];
      this.__d[k] = desc[k] !== undefined ? String(desc[k]) : DEFAULTS[k];
    }
    var res, rej;
    hid(this, '__loaded', quiet(new Promise(function (a, b) { res = a; rej = b; })));
    hid(this, '__res', res);
    hid(this, '__rej', rej);
    hid(this, '__inflight', null);
    hid(this, '__key', 'js:' + (++serial));
    hid(this, '__bytes', null);
    hid(this, '__css', false);
    if (typeof source === 'string') {
      if (SRC_RE.test(source)) this.__src = source;
      else { setStatus(this, 'error'); rej(DE("Failed to construct 'FontFace': The value provided ('" + source + "') could not be parsed as a value list.", 'SyntaxError')); }
    } else if (source && (source instanceof ArrayBuffer || ArrayBuffer.isView(source))) {
      this.__src = '<binary>';
      setStatus(this, 'loading');
      var view = source instanceof ArrayBuffer ? new Uint8Array(source.slice(0)) : new Uint8Array(source.buffer.slice(source.byteOffset, source.byteOffset + source.byteLength));
      // Decoded in the background (a microtask), like browsers do.
      g.queueMicrotask(function () { gotBytes(self, view); });
    } else {
      setStatus(this, 'error');
      rej(DE("Failed to construct 'FontFace': The source provided is neither a string nor a buffer.", 'SyntaxError'));
    }
  });
  Object.defineProperty(FontFace.prototype, Symbol.toStringTag, { value: 'FontFace', configurable: true });
  Object.defineProperty(FontFace.prototype, 'family', {
    get: function () { return this.__family; },
    set: function (v) { this.__family = String(v); }, configurable: true, enumerable: true });
  DESC.forEach(function (k) {
    Object.defineProperty(FontFace.prototype, k, {
      get: function () { return this.__d[k]; },
      set: function (v) { this.__d[k] = String(v); }, configurable: true, enumerable: true });
  });
  Object.defineProperty(FontFace.prototype, 'status', { get: function () { return this.__status; }, configurable: true, enumerable: true });
  Object.defineProperty(FontFace.prototype, 'loaded', { get: function () { return this.__loaded; }, configurable: true, enumerable: true });
  def(FontFace.prototype, 'load', function () {
    if (this.__status === 'unloaded') startLoad(this);
    return this.__loaded; // the face's own [[FontStatusPromise]] (spec), already marked handled
  });

  // Face state changes are reported to every set holding the face.
  function inDoc(face) { return face.__sets.indexOf(docSet) >= 0; }
  // Decoded bytes: a face in document.fonts is added to the font systems, any other one only checked.
  function gotBytes(face, bytes) {
    face.__bytes = bytes;
    var ok = inDoc(face) && fAdd ? fAdd(face.__key, face.__family, bytes, face.__d.weight, face.__d.style)
      : fCheck ? fCheck(bytes) : true;
    finish(face, ok ? null : new Error('not a font'));
  }
  function startLoad(face) {
    setStatus(face, 'loading');
    face.__sets.forEach(function (s) { s.__begin(face); });
    // A stylesheet face the renderer is already loading: `__scratchy_fonts_sync` finishes it.
    if (face.__css && fState) {
      var st = fState(face.__key);
      if (st === 'loading') return;
      if (st === 'loaded') { g.queueMicrotask(function () { finish(face, null); }); return; }
    }
    var url = face.__src ? usableUrl(face.__src) : null;
    // Only local() sources: the system font (or the fallback) is what renders; nothing to fetch.
    if (!url && face.__src && /local\(/i.test(face.__src)) { g.queueMicrotask(function () { finish(face, null); }); return; }
    if (!url || typeof g.fetch !== 'function') { g.queueMicrotask(function () { finish(face, new Error('no usable source')); }); return; }
    var abs;
    try { abs = new g.URL(url, face.__base || g.location.href).href; } catch (e) { abs = url; }
    g.fetch(abs).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.arrayBuffer();
    }).then(function (ab) { gotBytes(face, new Uint8Array(ab)); }, function (e) { finish(face, e || new Error('fetch failed')); });
  }
  // QFF1a: stylesheet faces follow the renderer's loads (called by the App when a load starts or ends).
  function syncCss() {
    if (!fState) return;
    var faces = cssFaces(), i, f, st;
    for (i = 0; i < faces.length; i++) {
      f = faces[i];
      st = fState(f.__key);
      if (f.__status === 'unloaded' && st !== 'unloaded') {
        setStatus(f, 'loading');
        f.__sets.forEach(function (s) { s.__begin(f); });
      }
      if (f.__status === 'loading' && (st === 'loaded' || st === 'error')) finish(f, st === 'error' ? new Error('font load failed') : null);
    }
  }
  hid(g, '__scratchy_fonts_sync', syncCss);
  function finish(face, err) {
    if (face.__status !== 'loading') return;
    if (err) {
      setStatus(face, 'error');
      face.__rej(DE('A network error occurred.', 'NetworkError'));
    } else {
      setStatus(face, 'loaded');
      face.__res(face);
    }
    face.__sets.forEach(function (s) { s.__end(face, !!err); });
  }

  // ---- FontFaceSetLoadEvent ----
  var FontFaceSetLoadEvent = ctor('FontFaceSetLoadEvent', function FontFaceSetLoadEvent(type, init) {
    if (!new.target) throw new TypeError("Failed to construct 'FontFaceSetLoadEvent': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var ev = Reflect.construct(g.Event, [type, init], new.target);
    hid(ev, '__ff', init && init.fontfaces ? Array.prototype.slice.call(init.fontfaces) : []);
    return ev;
  });
  FontFaceSetLoadEvent.prototype = Object.create(g.Event.prototype, { constructor: { value: FontFaceSetLoadEvent, writable: true, configurable: true } });
  Object.defineProperty(FontFaceSetLoadEvent.prototype, 'fontfaces', { get: function () { return this.__ff.slice(); }, configurable: true, enumerable: true });

  // ---- FontFaceSet ----
  var FontFaceSet = ctor('FontFaceSet', function FontFaceSet(initial) {
    if (!new.target) throw new TypeError("Failed to construct 'FontFaceSet': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var self = Reflect.construct(ET, [], new.target);
    init(self);
    if (initial != null) Array.prototype.forEach.call(initial, function (f) { self.add(f); });
    return self;
  });
  FontFaceSet.prototype = Object.create(ET.prototype, { constructor: { value: FontFaceSet, writable: true, configurable: true } });
  Object.defineProperty(FontFaceSet.prototype, Symbol.toStringTag, { value: 'FontFaceSet', configurable: true });
  function init(s) {
    hid(s, '__faces', []);       // script-added faces
    hid(s, '__css', null);       // the document's set: function giving the stylesheet faces, else null
    hid(s, '__pending', 0);
    hid(s, '__errs', []);
    hid(s, '__done', []);
    hid(s, '__st', 'loaded');
    hid(s, '__onl', null); hid(s, '__ond', null); hid(s, '__one', null);
    // Created on first read: a promise made while the prelude runs leaves a job in the isolate's queue that
    // keeps a dropped iframe realm alive (QPT1b heap check).
    hid(s, '__ready', null);
    hid(s, '__readyRes', null);
    hid(s, '__readyRej', null);
    hid(s, '__begin', function (face) {
      if (!s.__pending++ && s.__st !== 'loading') {
        s.__st = 'loading';
        s.__ready = quiet(new Promise(function (a, b) { s.__readyRes = a; s.__readyRej = b; }));
        s.__done = []; s.__errs = [];
        s.dispatchEvent(new FontFaceSetLoadEvent('loading', { fontfaces: [] }));
      }
    });
    hid(s, '__end', function (face, bad) {
      (bad ? s.__errs : s.__done).push(face);
      if (--s.__pending > 0) return;
      s.__pending = 0;
      // Fired from a task, so `status` reads 'loading' until the batch is over, like in browsers.
      g.setTimeout(function () {
        if (s.__pending) return;
        s.__st = 'loaded';
        var done = s.__done, errs = s.__errs, rr = s.__readyRes, rj = s.__readyRej;
        s.__done = []; s.__errs = [];
        s.dispatchEvent(new FontFaceSetLoadEvent('loadingdone', { fontfaces: done }));
        if (errs.length) s.dispatchEvent(new FontFaceSetLoadEvent('loadingerror', { fontfaces: errs }));
        if (errs.length) { if (rj) rj(DE('A font failed to load.', 'NetworkError')); } else if (rr) rr(s);
      }, 0);
    });
  }
  function members(s) {
    var out = s.__faces.slice(), css = s.__css ? s.__css() : [], i;
    for (i = 0; i < css.length; i++) if (out.indexOf(css[i]) < 0) out.push(css[i]);
    return out;
  }
  var P = FontFaceSet.prototype;
  ['loading', 'loadingdone', 'loadingerror'].forEach(function (t) {
    var k = { loading: '__onl', loadingdone: '__ond', loadingerror: '__one' }[t];
    Object.defineProperty(P, 'on' + t, {
      get: function () { return this[k] || null; },
      set: function (f) {
        var self = this;
        if (this[k] && this[k].__w) this.removeEventListener(t, this[k].__w);
        if (typeof f === 'function') {
          var w = function (e) { return f.call(self, e); };
          this[k] = { f: f, __w: w };
          this.addEventListener(t, w);
        } else this[k] = null;
      }, configurable: true, enumerable: true });
    // The getter returns the function, not the holder.
    var d = Object.getOwnPropertyDescriptor(P, 'on' + t);
    Object.defineProperty(P, 'on' + t, {
      get: function () { var h = this[k]; return h ? h.f : null; },
      set: d.set, configurable: true, enumerable: true });
  });
  Object.defineProperty(P, 'ready', { get: function () { return this.__ready || (this.__ready = quiet(Promise.resolve(this))); }, configurable: true, enumerable: true });
  Object.defineProperty(P, 'status', { get: function () { return this.__st; }, configurable: true, enumerable: true });
  Object.defineProperty(P, 'size', { get: function () { return members(this).length; }, configurable: true, enumerable: true });
  def(P, 'add', function (face) {
    if (!(face instanceof FontFace)) throw new TypeError("Failed to execute 'add' on 'FontFaceSet': parameter 1 is not of type 'FontFace'.");
    if (members(this).indexOf(face) >= 0) return this;
    if (face.__status === 'error') throw DE("Failed to execute 'add' on 'FontFaceSet': Cannot add a FontFace with a status of 'error'.", 'InvalidModificationError');
    this.__faces.push(face);
    if (face.__sets.indexOf(this) < 0) face.__sets.push(this);
    if (face.__status === 'loading') this.__begin(face);
    // QFF1a: a loaded script face added to document.fonts is used for rendering from now on.
    if (this === docSet && face.__status === 'loaded' && face.__bytes && fAdd) fAdd(face.__key, face.__family, face.__bytes, face.__d.weight, face.__d.style);
    return this;
  });
  def(P, 'delete', function (face) {
    var i = this.__faces.indexOf(face), j;
    if (i < 0) return false;
    this.__faces.splice(i, 1);
    j = face.__sets.indexOf(this);
    if (j >= 0) face.__sets.splice(j, 1);
    if (this === docSet && fRemove && !face.__css) fRemove(face.__key);
    return true;
  });
  def(P, 'clear', function () {
    var self = this;
    this.__faces.slice().forEach(function (f) { self['delete'](f); });
  });
  def(P, 'has', function (face) { return members(this).indexOf(face) >= 0; });
  def(P, 'forEach', function (cb, thisArg) {
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'forEach' on 'FontFaceSet': The callback provided as parameter 1 is not a function.");
    var self = this;
    members(this).forEach(function (f) { cb.call(thisArg, f, f, self); });
  });
  def(P, 'values', function () { return members(this)[Symbol.iterator](); });
  def(P, 'keys', P.values);
  def(P, 'entries', function () { return members(this).map(function (f) { return [f, f]; })[Symbol.iterator](); });
  def(P, Symbol.iterator, P.values);

  // `font` is a CSS font shorthand ("italic 700 16px/2 Open Sans, serif"); only the family list matters here.
  function families(font, who) {
    var s = String(font).trim(), m, list;
    // size token: a length, percentage or size keyword, optionally followed by /line-height; the families follow it
    m = /(?:^|\s)(?:(?:\d*\.?\d+(?:px|pt|em|rem|ex|ch|vw|vh|vmin|vmax|cm|mm|in|pc|q|%))|xx-small|x-small|small|medium|large|x-large|xx-large|xxx-large|smaller|larger)(?:\s*\/\s*\S+)?\s+(\S[\s\S]*)$/i.exec(s);
    if (!m) throw DE("Failed to execute '" + who + "' on 'FontFaceSet': Could not resolve '" + s + "' as a font.", 'SyntaxError');
    list = splitTop(m[1], ',').map(function (x) { return unquote(x).replace(/\s+/g, ' ').toLowerCase(); }).filter(Boolean);
    if (!list.length) throw DE("Failed to execute '" + who + "' on 'FontFaceSet': Could not resolve '" + s + "' as a font.", 'SyntaxError');
    return list;
  }
  function matching(set, font, who) {
    var fam = families(font, who);
    return members(set).filter(function (f) { return fam.indexOf(unquote(f.family).replace(/\s+/g, ' ').toLowerCase()) >= 0; });
  }
  def(P, 'check', function (font) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'check' on 'FontFaceSet': 1 argument required, but only 0 present.");
    var list = matching(this, font, 'check');
    return list.every(function (f) { return f.status === 'loaded'; });
  });
  def(P, 'load', function (font) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'load' on 'FontFaceSet': 1 argument required, but only 0 present.");
    var list;
    try { list = matching(this, font, 'load'); } catch (e) { return Promise.reject(e); }
    return Promise.all(list.map(function (f) { return f.load(); }));
  });

  // ---- the document's set: stylesheet faces, one FontFace per @font-face rule ----
  var byRule = new WeakMap();
  function declsOf(raw) {
    var b = raw.indexOf('{'), e = raw.lastIndexOf('}'), out = {};
    if (b < 0) return out;
    splitTop(raw.slice(b + 1, e < 0 ? raw.length : e), ';').forEach(function (d) {
      var i = d.indexOf(':');
      if (i > 0) out[d.slice(0, i).trim().toLowerCase()] = d.slice(i + 1).replace(/!important\s*$/i, '').trim();
    });
    return out;
  }
  function faceFor(rule, sheet) {
    var f = byRule.get(rule);
    if (f) return f;
    var d = declsOf(rule.cssText || ''), desc = {}, k;
    for (k in CSSNAME) if (d[CSSNAME[k]] !== undefined) desc[k] = d[CSSNAME[k]];
    f = new FontFace(unquote(d['font-family'] || ''), d.src || 'local(x)', desc);
    if (f.__status === 'error') { f.__status = 'unloaded'; f.__src = null; } // an unparsable `src` stays a declared, never-loadable face
    try { f.__base = sheet.href || g.document.baseURI; } catch (e) {}
    f.__css = true;
    f.__key = famKey(d['font-family'] || '') + '|' + srcKey(d.src || '');
    f.__sets.push(docSet);
    byRule.set(rule, f);
    // The renderer may have loaded it already (a face it is still loading is picked up by the next sync).
    var st = fState ? fState(f.__key) : 'unloaded';
    if (st === 'loaded') { setStatus(f, 'loaded'); f.__res(f); }
    else if (st === 'error') { setStatus(f, 'error'); f.__rej(DE('A network error occurred.', 'NetworkError')); }
    return f;
  }
  function cssFaces() {
    var out = [], sheets, i, j, rules;
    try { sheets = g.document.styleSheets; } catch (e) { return out; }
    for (i = 0; i < sheets.length; i++) {
      try {
        rules = sheets[i].cssRules;
        for (j = 0; j < rules.length; j++) if (rules[j].type === 5) out.push(faceFor(rules[j], sheets[i]));
      } catch (e) { /* a sheet whose rules cannot be read declares nothing */ }
    }
    return out;
  }
  var docSet = Reflect.construct(FontFaceSet, [], FontFaceSet);
  docSet.__css = cssFaces;
  Object.defineProperty(g.document, 'fonts', { get: function () { return docSet; }, configurable: true, enumerable: true });

  def(g, 'FontFace', FontFace);
  def(g, 'FontFaceSet', FontFaceSet);
  def(g, 'FontFaceSetLoadEvent', FontFaceSetLoadEvent);
})(globalThis);
