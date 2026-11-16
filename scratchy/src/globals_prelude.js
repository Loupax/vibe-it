// QY13: small common web APIs, plain JS over the timer table. Runs after
// the main, timers and url preludes. Everything is in-process and
// per-engine: nothing is persisted and nothing touches the network.
(function (g) {
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function tm(fn) { g.setTimeout(fn, 0); }
  function report(e) { console.error('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); }
  function dom(msg, name) { return new g.DOMException(msg, name); }
  function sub(name, parent, init) {
    var f = new Function('init', 'return function ' + name + '() { if (init) init.apply(this, arguments); };')(init);
    f.prototype = Object.create(parent.prototype, { constructor: { value: f, writable: true, configurable: true } });
    def(g, name, f);
    return f;
  }

  // ---- queueMicrotask / reportError ----
  if (typeof g.queueMicrotask !== 'function') {
    def(g, 'queueMicrotask', function (fn) {
      if (typeof fn !== 'function') throw new TypeError("Failed to execute 'queueMicrotask' on 'Window': parameter 1 is not of type 'Function'.");
      Promise.resolve().then(function () { try { fn(); } catch (e) { report(e); } });
    });
  }
  if (typeof g.reportError !== 'function') {
    def(g, 'reportError', function (e) { report(e); });
  }

  // ---- structuredClone: plain data, Date, RegExp, Map, Set, arrays,
  // Error, cycles. Functions and DOM nodes throw DataCloneError; typed
  // arrays/ArrayBuffer are copied through slice().
  if (typeof g.structuredClone !== 'function') {
    def(g, 'structuredClone', function (v) {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'structuredClone' on 'Window': 1 argument required, but only 0 present.");
      var seen = new Map();
      function bad(x) { return dom((typeof x === 'function' ? String(x) : 'object') + ' could not be cloned.', 'DataCloneError'); }
      function walk(x) {
        if (typeof x === 'function' || typeof x === 'symbol') throw bad(x);
        if (x === null || typeof x !== 'object') return x;
        if (seen.has(x)) return seen.get(x);
        var r, i;
        if (x instanceof Date) { r = new Date(x.getTime()); seen.set(x, r); return r; }
        if (x instanceof RegExp) { r = new RegExp(x.source, x.flags); seen.set(x, r); return r; }
        if (x instanceof Boolean || x instanceof Number || x instanceof String) { r = Object(x.valueOf()); seen.set(x, r); return r; }
        if (typeof g.Node === 'function' && x instanceof g.Node) throw bad(x);
        if (x instanceof Map) { r = new Map(); seen.set(x, r); x.forEach(function (val, k) { r.set(walk(k), walk(val)); }); return r; }
        if (x instanceof Set) { r = new Set(); seen.set(x, r); x.forEach(function (val) { r.add(walk(val)); }); return r; }
        if (x instanceof Error) {
          r = new (g[x.name] && g[x.name].prototype instanceof Error ? g[x.name] : Error)(x.message); seen.set(x, r);
          if (x.stack) r.stack = x.stack; return r;
        }
        if (typeof ArrayBuffer !== 'undefined' && (x instanceof ArrayBuffer || ArrayBuffer.isView(x))) { r = x.slice ? x.slice(0) : x; seen.set(x, r); return r; }
        if (Array.isArray(x)) { r = new Array(x.length); seen.set(x, r); for (i = 0; i < x.length; i++) if (i in x) r[i] = walk(x[i]); return r; }
        r = {}; seen.set(x, r);
        Object.keys(x).forEach(function (k) { r[k] = walk(x[k]); });
        return r;
      }
      return walk(v);
    });
  }
  var clone = g.structuredClone;

  // ---- AbortController / AbortSignal ----
  var ET = g.EventTarget, TOKEN = {};
  function abortReason() { return dom('signal is aborted without reason', 'AbortError'); }
  var AbortSignal = sub('AbortSignal', ET, function (tok) {
    if (tok !== TOKEN) throw new TypeError('Illegal constructor');
    this.aborted = false; this.reason = undefined; this.onabort = null;
    Object.defineProperty(this, '__deps', { value: [], writable: true });
  });
  function newSignal() { return new AbortSignal(TOKEN); }
  function fire(s, reason) {
    if (s.aborted) return;
    s.aborted = true; s.reason = reason === undefined ? abortReason() : reason;
    var ev = new g.Event('abort');
    s.dispatchEvent(ev);
    var d = s.__deps; s.__deps = [];
    d.forEach(function (c) { fire(c, s.reason); });
  }
  def(AbortSignal.prototype, 'throwIfAborted', function () { if (this.aborted) throw this.reason; });
  def(AbortSignal, 'abort', function (reason) { var s = newSignal(); fire(s, reason); return s; });
  def(AbortSignal, 'timeout', function (ms) {
    var s = newSignal();
    g.setTimeout(function () { fire(s, dom('signal timed out', 'TimeoutError')); }, Number(ms) || 0);
    return s;
  });
  def(AbortSignal, 'any', function (signals) {
    var s = newSignal(), list = Array.prototype.slice.call(signals || []), i;
    for (i = 0; i < list.length; i++) if (list[i].aborted) { fire(s, list[i].reason); return s; }
    list.forEach(function (x) { x.__deps.push(s); });
    return s;
  });
  var AbortController = sub('AbortController', Object, function () {
    Object.defineProperty(this, 'signal', { value: newSignal(), enumerable: true, configurable: true });
  });
  def(AbortController.prototype, 'abort', function (reason) { fire(this.signal, reason); });

  // ---- MessageEvent / MessageChannel / MessagePort / window.postMessage ----
  var MessageEvent = g.MessageEvent; // defined by the main prelude (QY12)
  var PTOKEN = {};
  var MessagePort = sub('MessagePort', ET, function (tok) {
    if (tok !== PTOKEN) throw new TypeError('Illegal constructor');
    Object.defineProperty(this, '__q', { value: [], writable: true });
    Object.defineProperty(this, '__started', { value: false, writable: true });
    Object.defineProperty(this, '__closed', { value: false, writable: true });
    Object.defineProperty(this, '__peer', { value: null, writable: true });
    Object.defineProperty(this, '__h', { value: null, writable: true });
  });
  Object.defineProperty(MessagePort.prototype, 'onmessage', {
    get: function () { return this.__h; },
    set: function (f) { this.__h = f; if (typeof f === 'function') this.start(); },
    configurable: true, enumerable: true });
  MessagePort.prototype.onmessageerror = null;
  function flush(p) {
    if (!p.__started || p.__closed) return;
    tm(function () {
      while (p.__started && !p.__closed && p.__q.length) {
        var m = p.__q.shift();
        var ev = new MessageEvent('message', { data: m, origin: '', ports: [] });
        p.dispatchEvent(ev);
        if (typeof p.__h === 'function') { /* dispatch already invoked the on-handler */ }
      }
    });
  }
  def(MessagePort.prototype, 'start', function () { this.__started = true; flush(this); });
  def(MessagePort.prototype, 'close', function () { this.__closed = true; if (this.__peer) this.__peer.__peer = null; this.__peer = null; });
  def(MessagePort.prototype, 'postMessage', function (m) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'postMessage' on 'MessagePort': 1 argument required, but only 0 present.");
    var d = clone(m), peer = this.__peer;
    if (!peer || this.__closed || peer.__closed) return;
    peer.__q.push(d);
    flush(peer);
  });
  sub('MessageChannel', Object, function () {
    var a = new MessagePort(PTOKEN), b = new MessagePort(PTOKEN);
    a.__peer = b; b.__peer = a;
    this.port1 = a; this.port2 = b;
  });
  def(g, 'postMessage', function (msg, target) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'postMessage' on 'Window': 1 argument required, but only 0 present.");
    var origin = g.location.origin;
    if (target && typeof target === 'object') target = target.targetOrigin;
    if (target !== undefined && target !== '*' && target !== '/' && String(target) !== origin) return;
    var d = clone(msg);
    tm(function () { g.dispatchEvent(new MessageEvent('message', { data: d, origin: origin, source: g, ports: [] })); });
  });

  // ---- requestIdleCallback ----
  var idle = new Map(), idleSeq = 1;
  def(g, 'requestIdleCallback', function (cb, opts) {
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'requestIdleCallback' on 'Window': The callback provided as parameter 1 is not a function.");
    var id = idleSeq++, start = g.__scratchy_now(), to = opts && Number(opts.timeout) > 0 ? Number(opts.timeout) : 0;
    var t = g.setTimeout(function () {
      idle.delete(id);
      var now = g.__scratchy_now(), dt = to > 0 && now - start >= to;
      cb({ didTimeout: dt, timeRemaining: function () { return dt ? 0 : Math.max(0, 50 - (g.__scratchy_now() - now)); } });
    }, 1);
    idle.set(id, t);
    return id;
  });
  def(g, 'cancelIdleCallback', function (id) {
    if (idle.has(id)) { g.clearTimeout(idle.get(id)); idle.delete(id); }
  });

  // ---- Storage: in-memory, per engine, NOT persisted (product decision) ----
  var STOK = {};
  var Storage = sub('Storage', Object, function (tok) { if (tok !== STOK) throw new TypeError('Illegal constructor'); });
  function makeStorage() {
    var data = new Map(), t = new Storage(STOK);
    def(t, 'getItem', function (k) { if (!arguments.length) throw new TypeError("Failed to execute 'getItem' on 'Storage': 1 argument required, but only 0 present."); k = String(k); return data.has(k) ? data.get(k) : null; });
    def(t, 'setItem', function (k, v) { if (arguments.length < 2) throw new TypeError("Failed to execute 'setItem' on 'Storage': 2 arguments required, but only " + arguments.length + " present."); data.set(String(k), String(v)); });
    def(t, 'removeItem', function (k) { data.delete(String(k)); });
    def(t, 'clear', function () { data.clear(); });
    def(t, 'key', function (i) { var a = Array.from(data.keys()); i = Number(i); return i >= 0 && i < a.length ? a[i] : null; });
    Object.defineProperty(t, 'length', { get: function () { return data.size; }, configurable: true });
    return new Proxy(t, {
      get: function (o, p) {
        if (typeof p === 'string' && !(p in o) && data.has(p)) return data.get(p);
        return Reflect.get(o, p, o);
      },
      set: function (o, p, v) { if (typeof p === 'string') data.set(p, String(v)); return true; },
      has: function (o, p) { return (typeof p === 'string' && data.has(p)) || p in o; },
      deleteProperty: function (o, p) { if (typeof p === 'string') data.delete(p); return true; },
      ownKeys: function () { return Array.from(data.keys()); },
      getOwnPropertyDescriptor: function (o, p) {
        if (typeof p === 'string' && data.has(p)) return { value: data.get(p), writable: true, enumerable: true, configurable: true };
        return undefined;
      },
    });
  }
  def(g, 'localStorage', makeStorage());
  def(g, 'sessionStorage', makeStorage());

  // ---- history: in-engine entry list; URL changes go to location and
  // document.URL; back/forward/go are asynchronous, fire `popstate` and
  // stay JS-side (they never ask the App to navigate: that is Q33f's job).
  var loc = g.location, doc = g.document;
  var entries = [{ state: null, href: loc.href }], idx = 0, scrollRestoration = 'auto';
  function apply(href) {
    if (!href) return; // no page URL (fixtures): nothing to mirror
    var u = new g.URL(href);
    loc.href = u.href; loc.protocol = u.protocol; loc.host = u.host; loc.hostname = u.hostname;
    loc.port = u.port; loc.pathname = u.pathname; loc.search = u.search; loc.hash = u.hash; loc.origin = u.origin;
    try { doc.URL = u.href; doc.documentURI = u.href; } catch (e) {}
  }
  function target(url) {
    if (url === undefined || url === null) return loc.href;
    var u;
    try { u = new g.URL(String(url), loc.href); } catch (e) { throw dom("Failed to execute 'pushState' on 'History': '" + url + "' cannot be parsed.", 'SecurityError'); }
    if (u.origin !== loc.origin && loc.origin !== 'null') throw dom("A history state object with URL '" + u.href + "' cannot be created in a document with origin '" + loc.origin + "'.", 'SecurityError');
    return u.href;
  }
  var History = sub('History', Object, function (tok) { if (tok !== STOK) throw new TypeError('Illegal constructor'); });
  var H = History.prototype;
  Object.defineProperty(H, 'length', { get: function () { return entries.length; }, configurable: true, enumerable: true });
  Object.defineProperty(H, 'state', { get: function () { return entries[idx].state; }, configurable: true, enumerable: true });
  Object.defineProperty(H, 'scrollRestoration', {
    get: function () { return scrollRestoration; },
    set: function (v) { v = String(v); if (v === 'auto' || v === 'manual') scrollRestoration = v; },
    configurable: true, enumerable: true });
  def(H, 'pushState', function (state, title, url) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'pushState' on 'History': 2 arguments required, but only 0 present.");
    var href = target(url), s = clone(state);
    entries.length = idx + 1;
    entries.push({ state: s, href: href }); idx++;
    apply(href);
  });
  def(H, 'replaceState', function (state, title, url) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'replaceState' on 'History': 2 arguments required, but only 0 present.");
    var href = target(url);
    entries[idx] = { state: clone(state), href: href };
    apply(href);
  });
  def(H, 'go', function (n) {
    n = Number(n) || 0;
    var to = idx + n;
    if (n === 0 || to < 0 || to >= entries.length) return;
    tm(function () {
      var old = entries[idx].href;
      idx = to;
      var e = entries[idx];
      apply(e.href);
      g.dispatchEvent(new g.PopStateEvent('popstate', { state: e.state }));
      if (old !== e.href && old.split('#')[0] === e.href.split('#')[0]) {
        g.dispatchEvent(new g.HashChangeEvent('hashchange', { oldURL: old, newURL: e.href }));
      }
    });
  });
  def(H, 'back', function () { this.go(-1); });
  def(H, 'forward', function () { this.go(1); });
  def(g, 'history', new History(STOK));
})(globalThis);
