// QY13: small common web APIs, plain JS over the timer table. Runs after
// the main, timers and url preludes. Everything is in-process and
// per-engine: nothing is persisted and nothing touches the network.
(function (g) {
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function tm(fn) { g.setTimeout(fn, 0); }
  function report(e) { __scratchy_err('Uncaught ' + String(e) + (e && e.stack ? '\n' + e.stack : '')); }
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
    if (g.closed) return;
    // QPT1d (R53 D8): runs in the TARGET realm (`g`): cloned with the target's intrinsics, a target-realm
    // MessageEvent. The sender is the incumbent window (`__scratchy_incumbent`: the realm Rust or
    // `__scratchy_enter` entered), so `parent.postMessage` from a realm has that realm as `source`/`origin`.
    var d = clone(msg), src = g, sorigin = origin;
    try { src = g.__scratchy_incumbent(); sorigin = src.location.origin; } catch (e) { src = g; sorigin = origin; }
    // QPT1-D1: across an opaque (data:) boundary the receiver only gets a cross-origin view of the sender.
    if (src !== g && g.__scratchy_xo && (src.__scratchy_opaque || g.__scratchy_opaque)) src = g.__scratchy_xo(src);
    tm(function () { if (!g.closed) g.dispatchEvent(new MessageEvent('message', { data: d, origin: sorigin, source: src, ports: [] })); });
  });

  // ---- requestIdleCallback ----
  var idle = new Map(), idleSeq = 1;
  def(g, 'requestIdleCallback', function (cb, opts) {
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'requestIdleCallback' on 'Window': The callback provided as parameter 1 is not a function.");
    var id = idleSeq++, start = g.__scratchy_now(), to = opts && Number(opts.timeout) > 0 ? Number(opts.timeout) : 0;
    var w = function () {
      idle.delete(id);
      var now = g.__scratchy_now(), dt = to > 0 && now - start >= to;
      cb({ didTimeout: dt, timeRemaining: function () { return dt ? 0 : Math.max(0, 50 - (g.__scratchy_now() - now)); } });
    };
    // QOBS4b: --perf-report names the page's callback, not this shim (only while recording).
    if (g.__perf_src) Object.defineProperty(w, '__scratchy_cb', { value: cb });
    var t = g.setTimeout(w, 1);
    idle.set(id, t);
    return id;
  });
  def(g, 'cancelIdleCallback', function (id) {
    if (idle.has(id)) { g.clearTimeout(idle.get(id)); idle.delete(id); }
  });

  // ---- Storage. localStorage over stash (QSTASH2, STASH.md): an http(s) page's area is filled from
  // the state directory before its first script (`__scratchy_ls_fill`) and its setItem/removeItem/clear
  // ops are recorded (`__scratchy_ls_op`) for Rust to commit at the end of the task. Opaque origins
  // (file:, data:, about:), runs without a state directory and sessionStorage stay in memory. ----
  var STOK = {};
  var Storage = sub('Storage', Object, function (tok) { if (tok !== STOK) throw new TypeError('Illegal constructor'); });
  // stash::QUOTA_UNITS: 5 Mi UTF-16 code units of keys plus values per origin (Firefox's rule); checked here
  // so the page sees QuotaExceededError synchronously, before the op is recorded.
  var LS_QUOTA = 5 * 1024 * 1024;
  function makeArea() { return { data: new Map(), units: 0, persist: false }; }
  function areaSet(a, k, v) {
    var old = a.data.get(k);
    if (old === v) return;
    var after = a.units - (old === undefined ? 0 : k.length + old.length) + k.length + v.length;
    if (after > LS_QUOTA) throw dom("Failed to execute 'setItem' on 'Storage': Setting the value of '" + k + "' exceeded the quota.", 'QuotaExceededError');
    a.data.set(k, v); a.units = after;
    if (a.persist) __scratchy_ls_op(0, k, v);
  }
  function areaRemove(a, k) {
    if (!a.data.has(k)) return;
    a.units -= k.length + a.data.get(k).length;
    a.data.delete(k);
    if (a.persist) __scratchy_ls_op(1, k);
  }
  function areaClear(a) {
    if (!a.data.size) return;
    a.data.clear(); a.units = 0;
    if (a.persist) __scratchy_ls_op(2);
  }
  function makeStorage(a) {
    var data = a.data, t = new Storage(STOK);
    def(t, 'getItem', function (k) { if (!arguments.length) throw new TypeError("Failed to execute 'getItem' on 'Storage': 1 argument required, but only 0 present."); k = String(k); return data.has(k) ? data.get(k) : null; });
    def(t, 'setItem', function (k, v) { if (arguments.length < 2) throw new TypeError("Failed to execute 'setItem' on 'Storage': 2 arguments required, but only " + arguments.length + " present."); areaSet(a, String(k), String(v)); });
    def(t, 'removeItem', function (k) { areaRemove(a, String(k)); });
    def(t, 'clear', function () { areaClear(a); });
    def(t, 'key', function (i) { var ks = Array.from(data.keys()); i = Number(i); return i >= 0 && i < ks.length ? ks[i] : null; });
    Object.defineProperty(t, 'length', { get: function () { return data.size; }, configurable: true });
    return new Proxy(t, {
      get: function (o, p) {
        if (typeof p === 'string' && !(p in o) && data.has(p)) return data.get(p);
        return Reflect.get(o, p, o);
      },
      set: function (o, p, v) { if (typeof p === 'string') areaSet(a, p, String(v)); return true; },
      has: function (o, p) { return (typeof p === 'string' && data.has(p)) || p in o; },
      deleteProperty: function (o, p) { if (typeof p === 'string') areaRemove(a, p); return true; },
      ownKeys: function () { return Array.from(data.keys()); },
      getOwnPropertyDescriptor: function (o, p) {
        if (typeof p === 'string' && data.has(p)) return { value: data.get(p), writable: true, enumerable: true, configurable: true };
        return undefined;
      },
    });
  }
  var lsArea = makeArea(), ssArea = makeArea();
  def(g, 'localStorage', makeStorage(lsArea));
  def(g, 'sessionStorage', makeStorage(ssArea));
  // Page start (Rust, after the env): the stored pairs as [k0, v0, k1, v1, ...] sorted by key, or
  // undefined for an in-memory page. `key(i)` follows that order, new keys after.
  def(g, '__scratchy_ls_fill', function () {
    var r = __scratchy_ls_load();
    if (!r) return;
    lsArea.persist = true;
    for (var i = 0; i + 1 < r.length; i += 2) { lsArea.data.set(r[i], r[i + 1]); lsArea.units += r[i].length + r[i + 1].length; }
  });
  // The site was forgotten while the page was open: its copy is emptied (nothing is recorded).
  def(g, '__scratchy_ls_forgotten', function () { lsArea.data.clear(); lsArea.units = 0; });
  // A same-origin iframe realm (`__scratchy_realm_init`) uses its parent's areas: one storage per origin.
  def(g, '__scratchy_ls_areas', [lsArea, ssArea]);
  def(g, '__scratchy_ls_share', function (p) {
    var as = p.__scratchy_ls_areas;
    lsArea = as[0]; ssArea = as[1];
    def(g, '__scratchy_ls_areas', as);
    def(g, 'localStorage', makeStorage(lsArea));
    def(g, 'sessionStorage', makeStorage(ssArea));
  });

  // ---- history: in-engine entry list; URL changes go to location and
  // document.URL; back/forward/go are asynchronous, fire `popstate` and
  // stay JS-side (they never ask the App to navigate: that is Q33f's job).
  var loc = g.location, doc = g.document;
  var entries = [{ state: null, href: loc.href }], idx = 0, scrollRestoration = 'auto';
  // QV8-4: the first history entry is the page's URL, known only after the startup snapshot's env is applied.
  if (g.__scratchy_env_hooks) g.__scratchy_env_hooks.push(function () { entries[0].href = loc.href; });
  function apply(href) {
    if (!href) return; // no page URL (fixtures): nothing to mirror
    var u = new g.URL(href);
    g.__loc_set(u); // QF11: the private path (the public `location` setters navigate)
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
    if (g.__d_url_update) g.__d_url_update(href, false); // QY48: the App's URL/history
  });
  def(H, 'replaceState', function (state, title, url) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'replaceState' on 'History': 2 arguments required, but only 0 present.");
    var href = target(url);
    entries[idx] = { state: clone(state), href: href };
    apply(href);
    if (g.__d_url_update) g.__d_url_update(href, true);
  });
  // QF11: a script's fragment-only navigation (location.hash = ..., assign('#x')):
  // a new (or replaced) entry, the URL mirror, and an async `hashchange`.
  Object.defineProperty(g, '__hist_frag', { value: function (href, replace) {
    var old = loc.href;
    if (replace) entries[idx] = { state: entries[idx].state, href: href };
    else { entries.length = idx + 1; entries.push({ state: null, href: href }); idx++; }
    apply(href);
    if (old !== href) tm(function () { g.dispatchEvent(new g.HashChangeEvent('hashchange', { oldURL: old, newURL: href })); });
  } });
  // `notify`: tell the App the entry moved (page-initiated go/back/forward);
  // false when the App itself drove the traversal (__hist_to).
  function go(n, notify, href) {
    var to = idx + n;
    if (n === 0 || to < 0 || to >= entries.length) return false;
    if (href !== undefined && entries[to].href !== href) return false;
    tm(function () {
      var old = entries[idx].href, from = idx;
      idx = to;
      var e = entries[idx];
      apply(e.href);
      if (notify && g.__d_url_move) g.__d_url_move(to - from);
      g.dispatchEvent(new g.PopStateEvent('popstate', { state: e.state }));
      if (old !== e.href && old.split('#')[0] === e.href.split('#')[0]) {
        g.dispatchEvent(new g.HashChangeEvent('hashchange', { oldURL: old, newURL: e.href }));
      }
    });
    return true;
  }
  def(H, 'go', function (n) { go(Number(n) || 0, true); });
  // QY48: the App stepped to a same-document entry: fire popstate with its state.
  Object.defineProperty(g, '__hist_to', { value: function (n, href) { return go(n, false, href); } });
  def(H, 'back', function () { this.go(-1); });
  def(H, 'forward', function () { this.go(1); });
  def(g, 'history', new History(STOK));

  // ---- QY14: unhandled promise rejections. The engine's rejection
  // tracker calls __scratchy_rej_push(promise, reason, handled); after the
  // promise jobs drain, __scratchy_rej_flush() reports each rejection
  // still unhandled once (`unhandledrejection`, then the QJ2-format log
  // line unless a listener called preventDefault) and fires
  // `rejectionhandled` when a reported promise later gets a handler.
  var rejQ = [], rejReported = [];
  // QPT1a (R53 D5): iframe realms keep their own queue (V8 reports a rejection to the realm it
  // happened in, so `unhandledrejection` fires on the iframe's window); the page's flush, which Rust
  // calls, flushes theirs first.
  var rejSubs = [];
  def(g, '__scratchy_rej_sub', function (f) { rejSubs.push(f); });
  def(g, '__scratchy_rej_unsub', function (f) { var i = rejSubs.indexOf(f); if (i >= 0) rejSubs.splice(i, 1); });
  def(g, '__scratchy_rej_push', function (p, reason, handled) { rejQ.push([p, reason, handled]); });
  def(g, '__scratchy_rej_flush', function () {
    var subMore = false;
    for (var s = 0; s < rejSubs.length; s++) { try { if (rejSubs[s]()) subMore = true; } catch (e) { report(e); } }
    if (!rejQ.length) return subMore;
    var q = rejQ; rejQ = [];
    var fresh = [], handledLater = [], i, j;
    for (i = 0; i < q.length; i++) {
      var p = q[i][0];
      if (!q[i][2]) { fresh.push(q[i]); continue; }
      var found = false;
      for (j = 0; j < fresh.length; j++) if (fresh[j][0] === p) { fresh.splice(j, 1); found = true; break; }
      if (!found) for (j = 0; j < rejReported.length; j++) if (rejReported[j] === p) { rejReported.splice(j, 1); handledLater.push(q[i]); break; }
    }
    var fire = function (type, p, reason, cancelable) {
      var ev;
      try { ev = new g.PromiseRejectionEvent(type, { promise: p, reason: reason, cancelable: cancelable }); } catch (e) { return false; }
      try { ev.promise = p; ev.reason = reason; g.dispatchEvent(ev); } catch (e) { report(e); }
      var h = g['on' + type];
      if (typeof h === 'function') { try { h.call(g, ev); } catch (e) { report(e); } }
      return ev.defaultPrevented;
    };
    for (i = 0; i < fresh.length; i++) {
      var r = fresh[i][1];
      rejReported.push(fresh[i][0]);
      if (rejReported.length > 1000) rejReported.shift();
      if (!fire('unhandledrejection', fresh[i][0], r, true)) {
        var text;
        try { text = (r && r.name !== undefined && r.message !== undefined) ? r.name + ': ' + r.message : String(r); } catch (e) { text = '<unprintable value>'; }
        __scratchy_err('Uncaught (in promise) ' + text + (r && r.stack ? '\n' + r.stack : ''));
      }
    }
    for (i = 0; i < handledLater.length; i++) fire('rejectionhandled', handledLater[i][0], handledLater[i][1], false);
    return true;
  });
  // ---- QY15b: TextEncoder / TextDecoder (UTF-8, UTF-16LE, windows-1252)
  // and atob/btoa. The UTF-8 decoder is the WHATWG state machine, kept on
  // the instance between calls so `{stream: true}` can split a character
  // across chunks. Output is built as UTF-16 code units.
  var REPL = 0xFFFD;
  var WS_RE = /^[\t\n\f\r ]+|[\t\n\f\r ]+$/g;
  var priv = new WeakMap();
  function newTarget(name, t) {
    if (!t) throw new TypeError("Failed to construct '" + name + "': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
  }
  function toStr(v) { return v === undefined ? '' : String(v); }
  function pushCp(u, cp) {
    if (cp > 0xFFFF) { cp -= 0x10000; u.push(0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF)); } else u.push(cp);
  }
  function unitsToString(u) {
    var s = '';
    for (var i = 0; i < u.length; i += 8192) s += String.fromCharCode.apply(null, u.slice(i, i + 8192));
    return s;
  }
  // The code point at index i and how many code units it spans; a lone
  // surrogate counts as one unit and as U+FFFD.
  function cpAt(s, i) {
    var c = s.charCodeAt(i);
    if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length) {
      var d = s.charCodeAt(i + 1);
      if (d >= 0xDC00 && d <= 0xDFFF) return [((c - 0xD800) << 10) + (d - 0xDC00) + 0x10000, 2];
    }
    if (c >= 0xD800 && c <= 0xDFFF) return [REPL, 1];
    return [c, 1];
  }
  function utf8Push(cp, out) {
    if (cp < 0x80) out.push(cp);
    else if (cp < 0x800) out.push(0xC0 | (cp >> 6), 0x80 | (cp & 63));
    else if (cp < 0x10000) out.push(0xE0 | (cp >> 12), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
    else out.push(0xF0 | (cp >> 18), 0x80 | ((cp >> 12) & 63), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
  }

  function TextEncoder() { newTarget('TextEncoder', new.target); }
  Object.defineProperty(TextEncoder.prototype, 'encoding', { get: function () { return 'utf-8'; }, enumerable: true, configurable: true });
  def(TextEncoder.prototype, 'encode', function (input) {
    var s = toStr(input), out = [], i = 0, r;
    while (i < s.length) { r = cpAt(s, i); utf8Push(r[0], out); i += r[1]; }
    return new Uint8Array(out);
  });
  def(TextEncoder.prototype, 'encodeInto', function (src, dest) {
    if (!(dest instanceof Uint8Array)) throw new TypeError("Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'.");
    var s = toStr(src), i = 0, written = 0, r, tmp, k;
    while (i < s.length) {
      r = cpAt(s, i);
      tmp = [];
      utf8Push(r[0], tmp);
      if (written + tmp.length > dest.length) break;
      for (k = 0; k < tmp.length; k++) dest[written++] = tmp[k];
      i += r[1];
    }
    return { read: i, written: written };
  });
  Object.defineProperty(TextEncoder.prototype, Symbol.toStringTag, { value: 'TextEncoder', configurable: true });
  def(g, 'TextEncoder', TextEncoder);

  var DEC_LABELS = Object.create(null);
  [['utf-8', 'utf-8', 'utf8', 'unicode-1-1-utf-8', 'unicode11utf8', 'unicode20utf8', 'x-unicode20utf8'],
   ['utf-16le', 'utf-16le', 'utf-16', 'ucs-2', 'unicode', 'csunicode', 'iso-10646-ucs-2', 'unicodefeff'],
   ['windows-1252', 'windows-1252', 'latin1', 'iso-8859-1', 'iso8859-1', 'iso_8859-1', 'ascii', 'us-ascii', 'l1', 'cp1252', 'x-cp1252', 'iso-ir-100', 'csisolatin1']
  ].forEach(function (row) { for (var i = 1; i < row.length; i++) DEC_LABELS[row[i]] = row[0]; });
  var W1252 = [0x20AC, 0x81, 0x201A, 0x192, 0x201E, 0x2026, 0x2020, 0x2021, 0x2C6, 0x2030, 0x160, 0x2039, 0x152, 0x8D, 0x17D, 0x8F,
    0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x2DC, 0x2122, 0x161, 0x203A, 0x153, 0x9D, 0x17E, 0x178];

  function utf8Reset(st) { st.need = 0; st.seen = 0; st.cp = 0; st.lo = 0x80; st.hi = 0xBF; }
  function decodeError(enc) { return new TypeError('The encoded data was not valid for encoding ' + enc + '.'); }
  function utf8Feed(st, bytes, u, fatal) {
    for (var i = 0; i < bytes.length; i++) {
      var b = bytes[i];
      if (st.need === 0) {
        if (b < 0x80) { u.push(b); continue; }
        if (b >= 0xC2 && b <= 0xDF) { st.need = 1; st.cp = b & 0x1F; }
        else if (b >= 0xE0 && b <= 0xEF) { st.lo = b === 0xE0 ? 0xA0 : 0x80; st.hi = b === 0xED ? 0x9F : 0xBF; st.need = 2; st.cp = b & 0x0F; }
        else if (b >= 0xF0 && b <= 0xF4) { st.lo = b === 0xF0 ? 0x90 : 0x80; st.hi = b === 0xF4 ? 0x8F : 0xBF; st.need = 3; st.cp = b & 0x07; }
        else { if (fatal) throw decodeError('utf-8'); u.push(REPL); }
        continue;
      }
      if (b < st.lo || b > st.hi) {
        // Ill-formed: one U+FFFD for the bytes so far, then reprocess b.
        utf8Reset(st);
        if (fatal) throw decodeError('utf-8');
        u.push(REPL);
        i--;
        continue;
      }
      st.lo = 0x80; st.hi = 0xBF;
      st.cp = (st.cp << 6) | (b & 0x3F);
      st.seen++;
      if (st.seen === st.need) { pushCp(u, st.cp); utf8Reset(st); }
    }
  }
  function utf16Feed(st, bytes, u, fatal) {
    for (var i = 0; i < bytes.length; i++) {
      if (st.odd < 0) { st.odd = bytes[i]; continue; }
      var unit = st.odd | (bytes[i] << 8);
      st.odd = -1;
      if (st.high >= 0) {
        if (unit >= 0xDC00 && unit <= 0xDFFF) { u.push(st.high, unit); st.high = -1; continue; }
        if (fatal) throw decodeError('utf-16le');
        u.push(REPL); st.high = -1;
      }
      if (unit >= 0xD800 && unit <= 0xDBFF) st.high = unit;
      else if (unit >= 0xDC00 && unit <= 0xDFFF) { if (fatal) throw decodeError('utf-16le'); u.push(REPL); }
      else u.push(unit);
    }
  }
  // End of input (no `stream`): anything half-read is an error.
  function utf16Finish(st, u, fatal) {
    if (st.odd >= 0 || st.high >= 0) { if (fatal) throw decodeError('utf-16le'); u.push(REPL); }
    st.odd = -1; st.high = -1;
  }
  function bytesOf(input) {
    if (input === undefined) return new Uint8Array(0);
    if (input instanceof ArrayBuffer) return new Uint8Array(input);
    if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    throw new TypeError("Failed to execute 'decode' on 'TextDecoder': The provided value is not of type '(ArrayBuffer or ArrayBufferView)'.");
  }

  function TextDecoder(label, options) {
    newTarget('TextDecoder', new.target);
    var name = label === undefined ? 'utf-8' : String(label).replace(WS_RE, '').toLowerCase();
    var enc = Object.prototype.hasOwnProperty.call(DEC_LABELS, name) ? DEC_LABELS[name] : null;
    if (!enc) throw new RangeError('The "' + toStr(label) + '" encoding is not supported.');
    priv.set(this, {
      enc: enc,
      fatal: !!(options && options.fatal),
      ignoreBOM: !!(options && options.ignoreBOM),
      bom: false,
      u8: { need: 0, seen: 0, cp: 0, lo: 0x80, hi: 0xBF },
      u16: { odd: -1, high: -1 }
    });
  }
  function decPriv(t) {
    var p = t && typeof t === 'object' ? priv.get(t) : undefined;
    if (!p) throw new TypeError('Illegal invocation');
    return p;
  }
  Object.defineProperty(TextDecoder.prototype, 'encoding', { get: function () { return decPriv(this).enc; }, enumerable: true, configurable: true });
  Object.defineProperty(TextDecoder.prototype, 'fatal', { get: function () { return decPriv(this).fatal; }, enumerable: true, configurable: true });
  Object.defineProperty(TextDecoder.prototype, 'ignoreBOM', { get: function () { return decPriv(this).ignoreBOM; }, enumerable: true, configurable: true });
  def(TextDecoder.prototype, 'decode', function (input, options) {
    var p = decPriv(this), stream = !!(options && options.stream), bytes = bytesOf(input), u = [];
    try {
      if (p.enc === 'utf-8') {
        utf8Feed(p.u8, bytes, u, p.fatal);
        if (!stream && p.u8.need !== 0) { if (p.fatal) throw decodeError('utf-8'); u.push(REPL); }
      } else if (p.enc === 'utf-16le') {
        utf16Feed(p.u16, bytes, u, p.fatal);
        if (!stream) utf16Finish(p.u16, u, p.fatal);
      } else {
        for (var i = 0; i < bytes.length; i++) u.push(bytes[i] >= 0x80 && bytes[i] < 0xA0 ? W1252[bytes[i] - 0x80] : bytes[i]);
      }
    } catch (e) {
      utf8Reset(p.u8); p.u16.odd = -1; p.u16.high = -1; p.bom = false;
      throw e;
    }
    if (!p.bom && u.length && (p.enc === 'utf-8' || p.enc === 'utf-16le')) {
      if (u[0] === 0xFEFF && !p.ignoreBOM) u.shift();
      p.bom = true;
    }
    if (!stream) { utf8Reset(p.u8); p.u16.odd = -1; p.u16.high = -1; p.bom = false; }
    return unitsToString(u);
  });
  Object.defineProperty(TextDecoder.prototype, Symbol.toStringTag, { value: 'TextDecoder', configurable: true });
  def(g, 'TextDecoder', TextDecoder);

})(globalThis);
