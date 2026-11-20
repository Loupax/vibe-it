// QF2: Blob, File, FileReader and the blob: URL store. Plain in-memory data
// classes (no file picker, no I/O, no network). Evaluated after the globals
// and intl preludes (see js.rs). If QF1 (TextEncoder) lands, its classes are
// separate; the UTF-8 helpers below are private so this file needs nothing.
//
// QF3 adds FormData: a data class (no form reading, no submission). Request
// and Response bodies from a FormData are serialised as multipart/form-data.
//
// QF4 adds ReadableStream (minimal), Headers, Request and Response: plain data
// classes with no network (fetch/XHR are QF7/QF8). The network layer builds
// real responses with the INTERNAL hook `__make_response(status, statusText,
// headersPairs, bodyBytes, url, redirected, type)` (documented near the end
// of this file with its rules).
//
// Engine-local blob URL store: `URL.createObjectURL(blob)` registers
// `blob:<origin>/<uuid>` -> blob; `URL.revokeObjectURL(url)` removes it.
// For QF8/QF9 (fetch/XHR of blob: URLs) the lookup is the global
// `__blob_lookup(url)`: it returns the registered Blob (use its
// arrayBuffer()/bytes()/type) or `undefined` if the URL was never
// registered or was revoked. A fragment on the URL is ignored.
(function (g) {
  'use strict';
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  var bytesOf = new WeakMap();   // Blob -> Uint8Array (never mutated)

  function utf8(str) {
    var out = [], i = 0, n = str.length;
    for (; i < n; i++) {
      var c = str.charCodeAt(i);
      if (c >= 0xD800 && c <= 0xDBFF && i + 1 < n) {
        var d = str.charCodeAt(i + 1);
        if (d >= 0xDC00 && d <= 0xDFFF) { c = 0x10000 + ((c - 0xD800) << 10) + (d - 0xDC00); i++; }
        else c = 0xFFFD;
      } else if (c >= 0xD800 && c <= 0xDFFF) c = 0xFFFD;
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xC0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xE0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xF0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  }
  function unutf8(b) {
    var s = '', i = 0, n = b.length, parts = [];
    if (n >= 3 && b[0] === 0xEF && b[1] === 0xBB && b[2] === 0xBF) i = 3;
    while (i < n) {
      var c = b[i], need = 0, min = 0, cp = 0;
      if (c < 0x80) { cp = c; }
      else if (c >= 0xC2 && c <= 0xDF) { need = 1; cp = c & 31; min = 0x80; }
      else if (c >= 0xE0 && c <= 0xEF) { need = 2; cp = c & 15; min = 0x800; }
      else if (c >= 0xF0 && c <= 0xF4) { need = 3; cp = c & 7; min = 0x10000; }
      else { cp = -1; }
      i++;
      if (need) {
        var ok = true, k = 0;
        for (; k < need; k++) {
          var x = b[i + k];
          if (x === undefined || (x & 0xC0) !== 0x80) { ok = false; break; }
          cp = (cp << 6) | (x & 63);
        }
        if (ok && (cp < min || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF))) ok = false;
        if (ok) i += need; else { i += k; cp = -1; }
      }
      if (cp < 0) cp = 0xFFFD;
      if (cp >= 0x10000) { cp -= 0x10000; s += String.fromCharCode(0xD800 + (cp >> 10), 0xDC00 + (cp & 1023)); }
      else s += String.fromCharCode(cp);
      if (s.length > 8192) { parts.push(s); s = ''; }
    }
    return parts.join('') + s;
  }
  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  function b64(b) {
    var s = '', i = 0, n = b.length;
    for (; i + 2 < n; i += 3) {
      var v = (b[i] << 16) | (b[i + 1] << 8) | b[i + 2];
      s += B64[v >> 18] + B64[(v >> 12) & 63] + B64[(v >> 6) & 63] + B64[v & 63];
    }
    if (n - i === 1) { var v1 = b[i] << 16; s += B64[v1 >> 18] + B64[(v1 >> 12) & 63] + '=='; }
    else if (n - i === 2) { var v2 = (b[i] << 16) | (b[i + 1] << 8); s += B64[v2 >> 18] + B64[(v2 >> 12) & 63] + B64[(v2 >> 6) & 63] + '='; }
    return s;
  }
  function normType(t) {
    t = String(t);
    for (var i = 0; i < t.length; i++) { var c = t.charCodeAt(i); if (c < 0x20 || c > 0x7E) return ''; }
    return t.toLowerCase();
  }
  function native(s) { return s.replace(/\r\n|\r|\n/g, '\n'); }
  function isBlob(x) { return x !== null && typeof x === 'object' && bytesOf.has(x); }
  function concat(chunks, total) {
    var out = new Uint8Array(total), o = 0;
    for (var i = 0; i < chunks.length; i++) { out.set(chunks[i], o); o += chunks[i].length; }
    return out;
  }
  function data(b) {
    var d = bytesOf.get(b);
    if (!d) throw new TypeError('Illegal invocation');
    return d;
  }
  function asyncOf(fn) {
    return function () {
      var self = this, args = arguments;
      return new Promise(function (res) { res(fn.apply(self, args)); });
    };
  }

  function Blob(parts, opts) {
    if (!new.target) throw new TypeError("Failed to construct 'Blob': Please use the 'new' operator");
    initBlob(this, parts, opts);
  }
  function initBlob(self, parts, opts) {
    var chunks = [], total = 0, endings = 'transparent', type = '';
    if (parts !== undefined) {
      if (parts === null || typeof parts !== 'object' || typeof parts[Symbol.iterator] !== 'function')
        throw new TypeError("Failed to construct 'Blob': The provided value cannot be converted to a sequence.");
    }
    if (opts !== undefined && opts !== null) {
      if (typeof opts !== 'object' && typeof opts !== 'function') throw new TypeError("Failed to construct 'Blob': parameter 2 is not of type 'BlobPropertyBag'.");
      if (opts.endings !== undefined) {
        endings = String(opts.endings);
        if (endings !== 'transparent' && endings !== 'native') throw new TypeError("Failed to construct 'Blob': The provided value '" + endings + "' is not a valid enum value of type EndingType.");
      }
      if (opts.type !== undefined) type = normType(opts.type);
    }
    if (parts !== undefined) {
      for (var p of parts) {
        var c;
        if (isBlob(p)) c = bytesOf.get(p);
        else if (p instanceof ArrayBuffer) c = new Uint8Array(p.slice(0));
        else if (ArrayBuffer.isView(p)) c = new Uint8Array(p.buffer.slice(p.byteOffset, p.byteOffset + p.byteLength));
        else { var s = String(p); c = utf8(endings === 'native' ? native(s) : s); }
        chunks.push(c); total += c.length;
      }
    }
    bytesOf.set(self, concat(chunks, total));
    def(self, '_type', type);
  }
  var BP = Blob.prototype;
  Object.defineProperties(BP, {
    size: { get: function () { return data(this).length; }, enumerable: true, configurable: true },
    type: { get: function () { data(this); return this._type; }, enumerable: true, configurable: true },
  });
  def(BP, 'slice', function slice(start, end, contentType) {
    var d = data(this), n = d.length;
    function rel(v, dflt) {
      if (v === undefined) return dflt;
      v = Math.trunc(Number(v)) || 0;
      return v < 0 ? Math.max(n + v, 0) : Math.min(v, n);
    }
    var s = rel(start, 0), e = rel(end, n);
    var b = Object.create(BP);
    bytesOf.set(b, d.slice(s, Math.max(e, s)));
    def(b, '_type', contentType === undefined ? '' : normType(contentType));
    return b;
  });
  def(BP, 'text', asyncOf(function () { return unutf8(data(this)); }));
  def(BP, 'arrayBuffer', asyncOf(function () { var d = data(this); return d.buffer.slice(d.byteOffset, d.byteOffset + d.length); }));
  def(BP, 'bytes', asyncOf(function () { return new Uint8Array(data(this)); }));
  // ---- QF4: ReadableStream (minimal: default reader only; no tee, pipeTo,
  // pipeThrough, BYOB, TransformStream, WritableStream: leave them absent,
  // pages like duckduckgo feature-test and polyfill what is missing).
  // Chunks count 1 each (strategy.size is ignored). See TODO.txt QF4a.
  var rsMap = new WeakMap();
  function rsSt(s) { var x = s !== null && typeof s === 'object' ? rsMap.get(s) : undefined; if (!x) throw new TypeError('Illegal invocation'); return x; }
  function deferred() { var d = {}; d.promise = new Promise(function (a, b) { d.resolve = a; d.reject = b; }); return d; }
  function handled(p) { p.catch(function () {}); return p; }
  function rsDesired(s) { return s.st === 'errored' ? null : s.st === 'closed' ? 0 : s.hwm - s.q.length; }
  function rsFinish(s) {
    s.st = 'closed';
    var r = s.reads; s.reads = [];
    r.forEach(function (x) { x.res({ value: undefined, done: true }); });
    if (s.reader) s.reader.closed.resolve();
  }
  function rsError(s, e) {
    if (s.st !== 'readable') return;
    s.st = 'errored'; s.err = e; s.q = [];
    var r = s.reads; s.reads = [];
    r.forEach(function (x) { x.rej(e); });
    if (s.reader) { s.reader.closed.reject(e); handled(s.reader.closed.promise); }
  }
  function rsPull(s) {
    if (s.st !== 'readable' || s.closeReq || !s.started) return;
    if (!(s.reader && s.reads.length) && !(rsDesired(s) > 0)) return;
    if (typeof s.src.pull !== 'function') return;
    if (s.pulling) { s.pullAgain = true; return; }
    s.pulling = true;
    var p;
    try { p = Promise.resolve(s.src.pull.call(s.src, s.ctrl)); } catch (e) { rsError(s, e); return; }
    p.then(function () { s.pulling = false; if (s.pullAgain) { s.pullAgain = false; rsPull(s); } }, function (e) { rsError(s, e); });
  }
  function rsCancel(s, reason) {
    s.disturbed = true;
    if (s.st === 'closed') return Promise.resolve();
    if (s.st === 'errored') return Promise.reject(s.err);
    s.q = [];
    rsFinish(s);
    var p;
    try { p = Promise.resolve(typeof s.src.cancel === 'function' ? s.src.cancel.call(s.src, reason) : undefined); } catch (e) { return Promise.reject(e); }
    return p.then(function () {});
  }
  var CTOK = {};
  function ReadableStreamDefaultController(tok, s) {
    if (tok !== CTOK) throw new TypeError('Illegal constructor');
    def(this, '_s', s);
  }
  var RCP = ReadableStreamDefaultController.prototype;
  function cst(c) { if (!c || !c._s) throw new TypeError('Illegal invocation'); return c._s; }
  def(RCP, 'enqueue', function enqueue(chunk) {
    var s = cst(this);
    if (s.closeReq || s.st !== 'readable') throw new TypeError("Failed to execute 'enqueue' on 'ReadableStreamDefaultController': Cannot enqueue a chunk into a closed readable stream");
    if (s.reads.length) s.reads.shift().res({ value: chunk, done: false });
    else s.q.push(chunk);
    rsPull(s);
  });
  def(RCP, 'close', function close() {
    var s = cst(this);
    if (s.closeReq || s.st !== 'readable') throw new TypeError("Failed to execute 'close' on 'ReadableStreamDefaultController': Cannot close a closed readable stream");
    s.closeReq = true;
    if (!s.q.length) rsFinish(s);
  });
  def(RCP, 'error', function error(e) { rsError(cst(this), e); });
  Object.defineProperty(RCP, 'desiredSize', { get: function () { return rsDesired(cst(this)); }, enumerable: true, configurable: true });
  Object.defineProperty(RCP, Symbol.toStringTag, { value: 'ReadableStreamDefaultController', configurable: true });

  function ReadableStream(source, strategy) {
    if (!new.target) throw new TypeError("Failed to construct 'ReadableStream': Please use the 'new' operator");
    var src = source === undefined || source === null ? {} : Object(source);
    if (src.type !== undefined && String(src.type) === 'bytes') throw new TypeError("Failed to construct 'ReadableStream': byte streams are not implemented");
    if (src.type !== undefined) throw new RangeError("Failed to construct 'ReadableStream': Invalid type is specified");
    var hwm = 1;
    if (strategy !== undefined && strategy !== null && strategy.highWaterMark !== undefined) {
      hwm = Number(strategy.highWaterMark);
      if (!(hwm >= 0)) throw new RangeError("Failed to construct 'ReadableStream': Invalid highWaterMark");
    }
    var s = { src: src, st: 'readable', err: undefined, q: [], reads: [], reader: null, hwm: hwm, closeReq: false, started: false, pulling: false, pullAgain: false, disturbed: false, ctrl: null };
    rsMap.set(this, s);
    s.ctrl = new ReadableStreamDefaultController(CTOK, s);
    var r = typeof src.start === 'function' ? src.start.call(src, s.ctrl) : undefined;
    Promise.resolve(r).then(function () { s.started = true; rsPull(s); }, function (e) { rsError(s, e); });
  }
  var RSP = ReadableStream.prototype;
  Object.defineProperty(RSP, 'locked', { get: function () { return !!rsSt(this).reader; }, enumerable: true, configurable: true });
  def(RSP, 'cancel', function cancel(reason) {
    var s = rsSt(this);
    if (s.reader) return Promise.reject(new TypeError("Failed to execute 'cancel' on 'ReadableStream': Cannot cancel a locked stream"));
    return rsCancel(s, reason);
  });
  function ReadableStreamDefaultReader(stream) {
    if (!new.target) throw new TypeError("Failed to construct 'ReadableStreamDefaultReader': Please use the 'new' operator");
    var s = rsSt(stream);
    if (s.reader) throw new TypeError("Failed to construct 'ReadableStreamDefaultReader': ReadableStream is locked");
    var r = { s: s, closed: deferred() };
    s.reader = r;
    if (s.st === 'closed') r.closed.resolve();
    else if (s.st === 'errored') { r.closed.reject(s.err); handled(r.closed.promise); }
    def(this, '_r', r);
  }
  def(RSP, 'getReader', function getReader(opts) {
    rsSt(this);
    if (opts !== undefined && opts !== null && opts.mode !== undefined) throw new TypeError("Failed to execute 'getReader' on 'ReadableStream': BYOB readers are not implemented");
    return new ReadableStreamDefaultReader(this);
  });
  var RRP = ReadableStreamDefaultReader.prototype;
  function rr(x) { if (!x || !x._r) throw new TypeError('Illegal invocation'); return x._r; }
  Object.defineProperty(RRP, 'closed', { get: function () { return rr(this).closed.promise; }, enumerable: true, configurable: true });
  def(RRP, 'read', function read() {
    var r = rr(this), s = r.s;
    if (!s) return Promise.reject(new TypeError("Failed to execute 'read' on 'ReadableStreamDefaultReader': Cannot read from a released reader"));
    s.disturbed = true;
    if (s.st === 'closed') return Promise.resolve({ value: undefined, done: true });
    if (s.st === 'errored') return Promise.reject(s.err);
    if (s.q.length) {
      var v = s.q.shift();
      if (s.closeReq && !s.q.length) rsFinish(s); else rsPull(s);
      return Promise.resolve({ value: v, done: false });
    }
    return new Promise(function (res, rej) { s.reads.push({ res: res, rej: rej }); rsPull(s); });
  });
  def(RRP, 'cancel', function cancel(reason) {
    var r = rr(this);
    if (!r.s) return Promise.reject(new TypeError("Failed to execute 'cancel' on 'ReadableStreamDefaultReader': Cannot cancel a released reader"));
    return rsCancel(r.s, reason);
  });
  def(RRP, 'releaseLock', function releaseLock() {
    var r = rr(this), s = r.s;
    if (!s) return;
    var pend = s.reads; s.reads = [];
    var e = new TypeError('Reader was released');
    pend.forEach(function (x) { x.rej(e); });
    if (s.st === 'readable') { r.closed.reject(e); handled(r.closed.promise); }
    else if (s.st === 'closed') r.closed = { promise: handled(Promise.reject(e)) };
    s.reader = null; r.s = null;
  });
  Object.defineProperty(RRP, Symbol.toStringTag, { value: 'ReadableStreamDefaultReader', configurable: true });
  def(RSP, 'values', function values(opts) {
    var reader = this.getReader(), keep = !!(opts && opts.preventCancel);
    var it = {
      next: function () {
        return reader.read().then(function (x) { if (x.done) reader.releaseLock(); return x; }, function (e) { reader.releaseLock(); throw e; });
      },
      'return': function (v) {
        if (keep) { reader.releaseLock(); return Promise.resolve({ value: v, done: true }); }
        return reader.cancel(v).then(function () { reader.releaseLock(); return { value: v, done: true }; });
      },
    };
    it[Symbol.asyncIterator] = function () { return this; };
    return it;
  });
  def(RSP, Symbol.asyncIterator, RSP.values);
  Object.defineProperty(RSP, Symbol.toStringTag, { value: 'ReadableStream', configurable: true });
  def(g, 'ReadableStream', ReadableStream);
  def(g, 'ReadableStreamDefaultReader', ReadableStreamDefaultReader);
  def(g, 'ReadableStreamDefaultController', ReadableStreamDefaultController);

  if (typeof g.ReadableStream === 'function') {
    def(BP, 'stream', function stream() {
      var d = data(this);
      return new g.ReadableStream({ start: function (c) { if (d.length) c.enqueue(new Uint8Array(d)); c.close(); } });
    });
  }
  Object.defineProperty(BP, Symbol.toStringTag, { value: 'Blob', configurable: true });

  function File(parts, name, opts) {
    if (!new.target) throw new TypeError("Failed to construct 'File': Please use the 'new' operator");
    if (arguments.length < 2) throw new TypeError("Failed to construct 'File': 2 arguments required, but only " + arguments.length + ' present.');
    initBlob(this, parts, opts);
    def(this, '_name', String(name).replace(/\//g, ':'));
    var lm = opts && opts.lastModified !== undefined ? Number(opts.lastModified) : Date.now();
    def(this, '_lm', Number.isFinite(lm) ? Math.trunc(lm) : 0);
  }
  File.prototype = Object.create(BP, { constructor: { value: File, writable: true, configurable: true } });
  Object.setPrototypeOf(File, Blob);
  Object.defineProperties(File.prototype, {
    name: { get: function () { data(this); return this._name; }, enumerable: true, configurable: true },
    lastModified: { get: function () { data(this); return this._lm; }, enumerable: true, configurable: true },
    webkitRelativePath: { get: function () { data(this); return ''; }, enumerable: true, configurable: true },
  });
  Object.defineProperty(File.prototype, Symbol.toStringTag, { value: 'File', configurable: true });

  // ---- FileReader ----
  var PE = typeof g.ProgressEvent === 'function' ? g.ProgressEvent : (function () {
    function ProgressEvent(type, init) {
      var e = new g.Event(type, init);
      init = init || {};
      def(e, 'lengthComputable', !!init.lengthComputable);
      def(e, 'loaded', init.loaded || 0);
      def(e, 'total', init.total || 0);
      return e;
    }
    def(g, 'ProgressEvent', ProgressEvent);
    return ProgressEvent;
  })();
  function FileReader() {
    if (!new.target) throw new TypeError("Failed to construct 'FileReader': Please use the 'new' operator");
    var et = Reflect.construct(g.EventTarget, [], new.target);
    def(et, '_st', { state: 0, result: null, error: null, token: 0, h: {} });
    return et;
  }
  FileReader.prototype = Object.create(g.EventTarget.prototype, { constructor: { value: FileReader, writable: true, configurable: true } });
  Object.setPrototypeOf(FileReader, g.EventTarget);
  var FRP = FileReader.prototype;
  [['EMPTY', 0], ['LOADING', 1], ['DONE', 2]].forEach(function (p) {
    Object.defineProperty(FileReader, p[0], { value: p[1], enumerable: true });
    Object.defineProperty(FRP, p[0], { value: p[1], enumerable: true });
  });
  function st(r) { if (!r || !r._st) throw new TypeError('Illegal invocation'); return r._st; }
  Object.defineProperties(FRP, {
    readyState: { get: function () { return st(this).state; }, enumerable: true, configurable: true },
    result: { get: function () { return st(this).result; }, enumerable: true, configurable: true },
    error: { get: function () { return st(this).error; }, enumerable: true, configurable: true },
  });
  ['loadstart', 'progress', 'load', 'abort', 'error', 'loadend'].forEach(function (t) {
    Object.defineProperty(FRP, 'on' + t, {
      get: function () { var h = st(this).h[t]; return h === undefined ? null : h; },
      set: function (f) { st(this).h[t] = typeof f === 'function' ? f : null; },
      enumerable: true, configurable: true,
    });
  });
  function fire(r, type, loaded, total) {
    var e = new PE(type, { lengthComputable: true, loaded: loaded, total: total });
    r.dispatchEvent(e);
  }
  function domErr(name, msg) {
    if (typeof g.DOMException === 'function') return new g.DOMException(msg, name);
    var e = new Error(msg); e.name = name; return e;
  }
  function read(r, blob, how) {
    var s = st(r);
    if (!isBlob(blob)) throw new TypeError("Failed to execute '" + how + "' on 'FileReader': parameter 1 is not of type 'Blob'.");
    if (s.state === 1) throw domErr('InvalidStateError', "Failed to execute '" + how + "' on 'FileReader': The object is already busy reading Blobs.");
    s.state = 1; s.result = null; s.error = null;
    var tok = ++s.token, d = bytesOf.get(blob), type = blob._type;
    setTimeout(function () {
      if (s.token !== tok || s.state !== 1) return;
      fire(r, 'loadstart', 0, d.length);
      if (s.token !== tok || s.state !== 1) return;
      var res;
      if (how === 'readAsArrayBuffer') res = d.buffer.slice(d.byteOffset, d.byteOffset + d.length);
      else if (how === 'readAsText') res = unutf8(d);
      else if (how === 'readAsDataURL') res = 'data:' + (type || 'application/octet-stream') + ';base64,' + b64(d);
      else { res = ''; for (var i = 0; i < d.length; i++) res += String.fromCharCode(d[i]); }
      fire(r, 'progress', d.length, d.length);
      if (s.token !== tok || s.state !== 1) return;
      s.state = 2; s.result = res;
      fire(r, 'load', d.length, d.length);
      fire(r, 'loadend', d.length, d.length);
    }, 0);
  }
  ['readAsArrayBuffer', 'readAsText', 'readAsDataURL', 'readAsBinaryString'].forEach(function (how) {
    def(FRP, how, function (blob) { read(this, blob, how); });
  });
  def(FRP, 'abort', function abort() {
    var s = st(this);
    if (s.state !== 1) { s.result = null; return; }
    s.token++; s.state = 2; s.result = null;
    s.error = domErr('AbortError', 'The user aborted a request.');
    fire(this, 'abort', 0, 0);
    fire(this, 'loadend', 0, 0);
  });
  Object.defineProperty(FRP, Symbol.toStringTag, { value: 'FileReader', configurable: true });

  // ---- blob: URL store ----
  var store = new Map();
  function hex(n) { var s = ''; while (n--) s += Math.floor(Math.random() * 16).toString(16); return s; }
  function uuid() { return hex(8) + '-' + hex(4) + '-4' + hex(3) + '-' + '89ab'[Math.floor(Math.random() * 4)] + hex(3) + '-' + hex(12); }
  function key(u) { u = String(u); var i = u.indexOf('#'); return i < 0 ? u : u.slice(0, i); }
  g.URL.createObjectURL = function createObjectURL(obj) {
    if (!isBlob(obj) && !(typeof g.__mse_is === 'function' && g.__mse_is(obj))) throw new TypeError("Failed to execute 'createObjectURL' on 'URL': Overload resolution failed.");
    var o = g.location && g.location.origin;
    var u = 'blob:' + (o && o !== 'null' ? o : 'null') + '/' + uuid();
    store.set(u, obj);
    return u;
  };
  g.URL.revokeObjectURL = function revokeObjectURL(u) { store.delete(key(u)); };
  def(g, '__blob_lookup', function (u) { return store.get(key(u)); });
  // QF19w2: the bytes of a Blob (undefined for anything else), for WebSocket.send.
  def(g, '__blob_bytes', function (b) { return isBlob(b) ? bytesOf.get(b) : undefined; });

  def(g, 'Blob', Blob); def(g, 'File', File); def(g, 'FileReader', FileReader);

  // ---- QF3: FormData (data class only; no form reading, no submission) ----
  // Entries live in a WeakMap: FormData -> array of [name, value], where a
  // value is a string or a File. `new FormData(form)` never reads the form's
  // controls (AGENTS.md forms rule): it logs once and stays empty.
  var fdMap = new WeakMap();
  function fdList(x) { var l = x !== null && typeof x === 'object' ? fdMap.get(x) : undefined; if (!l) throw new TypeError('Illegal invocation'); return l; }
  // USVString: a lone surrogate becomes U+FFFD.
  function usv(s) {
    var out = '', n = s.length;
    for (var i = 0; i < n; i++) {
      var c = s.charCodeAt(i), d = i + 1 < n ? s.charCodeAt(i + 1) : 0;
      if (c >= 0xD800 && c <= 0xDBFF && d >= 0xDC00 && d <= 0xDFFF) { out += s[i] + s[i + 1]; i++; }
      else if (c >= 0xD800 && c <= 0xDFFF) out += '�';
      else out += s[i];
    }
    return out;
  }
  // A Blob value becomes a File: a plain Blob gets `filename` or 'blob', a
  // File keeps its name unless a filename is given. Anything else is a string.
  function fdValue(value, filename) {
    if (isBlob(value)) {
      if (value instanceof File && filename === undefined) return value;
      var nm = filename === undefined ? 'blob' : usv(String(filename));
      return new File([value], nm, { type: value.type });
    }
    return usv(String(value));
  }
  function FormData(form, submitter) {
    if (!new.target) throw new TypeError("Failed to construct 'FormData': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    fdMap.set(this, []);
    if (form !== undefined) {
      // Q30e: the form's data set (no submitter), through the same code as a submission.
      if (form === null || typeof form !== 'object' || form.localName !== 'form' || typeof form.__id !== 'number')
        throw new TypeError("Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.");
      var sid = submitter && typeof submitter.__id === 'number' ? submitter.__id : -1;
      var l = fdMap.get(this), sp = new g.URLSearchParams(g.__d_formdata(form.__id, sid));
      sp.forEach(function (v, k) { l.push([k, v]); });
    }
  }
  var FP = FormData.prototype;
  function fdArgs(n, args, name) {
    if (args < n) throw new TypeError("Failed to execute '" + name + "' on 'FormData': " + n + ' arguments required, but only ' + args + ' present.');
  }
  def(FP, 'append', function append(name, value, filename) {
    var l = fdList(this); fdArgs(2, arguments.length, 'append');
    l.push([usv(String(name)), fdValue(value, filename)]);
  });
  def(FP, 'set', function set(name, value, filename) {
    var l = fdList(this); fdArgs(2, arguments.length, 'set');
    var n = usv(String(name)), v = fdValue(value, filename), done = false;
    for (var i = 0; i < l.length;) {
      if (l[i][0] !== n) { i++; continue; }
      if (done) l.splice(i, 1);
      else { l[i] = [n, v]; done = true; i++; }
    }
    if (!done) l.push([n, v]);
  });
  def(FP, 'delete', function (name) {
    var l = fdList(this), n = usv(String(name)), j = 0;
    for (var i = 0; i < l.length; i++) if (l[i][0] !== n) l[j++] = l[i];
    l.length = j;
  });
  def(FP, 'get', function get(name) {
    var l = fdList(this), n = usv(String(name));
    for (var i = 0; i < l.length; i++) if (l[i][0] === n) return l[i][1];
    return null;
  });
  def(FP, 'getAll', function getAll(name) {
    var l = fdList(this), n = usv(String(name)), out = [];
    for (var i = 0; i < l.length; i++) if (l[i][0] === n) out.push(l[i][1]);
    return out;
  });
  def(FP, 'has', function has(name) {
    var l = fdList(this), n = usv(String(name));
    return l.some(function (e) { return e[0] === n; });
  });
  def(FP, 'forEach', function forEach(cb, thisArg) {
    var l = fdList(this), self = this;
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'forEach' on 'FormData': parameter 1 is not of type 'Function'.");
    for (var i = 0; i < l.length; i++) cb.call(thisArg, l[i][1], l[i][0], self);
  });
  // Live iterators over the entry list, insertion order (a later append is seen).
  function fdIter(fd, pick) {
    var l = fdList(fd), i = 0, it = {};
    def(it, 'next', function next() {
      if (i < l.length) return { value: pick(l[i++]), done: false };
      i = Infinity;
      return { value: undefined, done: true };
    });
    def(it, Symbol.iterator, function () { return this; });
    Object.defineProperty(it, Symbol.toStringTag, { value: 'FormData Iterator', configurable: true });
    return it;
  }
  def(FP, 'entries', function entries() { return fdIter(this, function (e) { return [e[0], e[1]]; }); });
  def(FP, 'keys', function keys() { return fdIter(this, function (e) { return e[0]; }); });
  def(FP, 'values', function values() { return fdIter(this, function (e) { return e[1]; }); });
  def(FP, Symbol.iterator, FP.entries);
  Object.defineProperty(FP, Symbol.toStringTag, { value: 'FormData', configurable: true });
  // Request/Response body from a FormData: multipart/form-data with a random
  // boundary (serialisation only; formData() still parses urlencoded only).
  function fdBody(fd) {
    var boundary = '----scratchyBoundary' + hex(16), chunks = [], total = 0;
    function put(u8) { chunks.push(u8); total += u8.length; }
    function txt(s) { put(utf8(s)); }
    function esc(s) { return s.replace(/\n/g, '%0A').replace(/\r/g, '%0D').replace(/"/g, '%22'); }
    fdList(fd).forEach(function (e) {
      var v = e[1];
      txt('--' + boundary + '\r\nContent-Disposition: form-data; name="' + esc(e[0]) + '"');
      if (typeof v === 'string') txt('\r\n\r\n' + v.replace(/\r\n|\r|\n/g, '\r\n'));
      else {
        txt('; filename="' + esc(v.name) + '"\r\nContent-Type: ' + (v.type || 'application/octet-stream') + '\r\n\r\n');
        put(data(v));
      }
      txt('\r\n');
    });
    txt('--' + boundary + '--\r\n');
    return { bytes: concat(chunks, total), type: 'multipart/form-data; boundary=' + boundary };
  }
  def(g, 'FormData', FormData);

  // ---- QF4: Headers, Request, Response (data classes, no network) ----
  var TOKEN_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  var FORBIDDEN_REQ = ['accept-charset', 'accept-encoding', 'access-control-request-headers', 'access-control-request-method', 'connection', 'content-length', 'cookie', 'cookie2', 'date', 'dnt', 'expect', 'host', 'keep-alive', 'origin', 'referer', 'set-cookie', 'te', 'trailer', 'transfer-encoding', 'upgrade', 'via'];
  function forbiddenReq(n) { return FORBIDDEN_REQ.indexOf(n) >= 0 || n.indexOf('proxy-') === 0 || n.indexOf('sec-') === 0; }
  function forbiddenRes(n) { return n === 'set-cookie' || n === 'set-cookie2'; }
  var hdrMap = new WeakMap();   // Headers -> { list: [[lowername, value]], guard }
  function hs(h) { var x = h !== null && typeof h === 'object' ? hdrMap.get(h) : undefined; if (!x) throw new TypeError('Illegal invocation'); return x; }
  function hName(n, fn) {
    n = String(n);
    if (!TOKEN_RE.test(n)) throw new TypeError("Failed to execute '" + fn + "' on 'Headers': Invalid name");
    return n.toLowerCase();
  }
  function hValue(v, fn) {
    v = String(v).replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, '');
    if (/[\0\r\n]/.test(v)) throw new TypeError("Failed to execute '" + fn + "' on 'Headers': Invalid value");
    for (var i = 0; i < v.length; i++) if (v.charCodeAt(i) > 255) throw new TypeError("Failed to execute '" + fn + "' on 'Headers': String contains non ISO-8859-1 code point.");
    return v;
  }
  // Returns false when the mutation must be silently ignored; throws for immutable.
  function hCheck(st, name, fn) {
    if (st.guard === 'immutable') throw new TypeError("Failed to execute '" + fn + "' on 'Headers': Headers are immutable");
    if (st.guard === 'request' && forbiddenReq(name)) return false;
    if (st.guard === 'response' && forbiddenRes(name)) return false;
    return true;
  }
  function hAppend(h, n, v, fn) {
    var st = hs(h), name = hName(n, fn), val = hValue(v, fn);
    if (!hCheck(st, name, fn)) return;
    st.list.push([name, val]);
  }
  function sortedEntries(h) {
    var list = hs(h).list, names = [], i;
    for (i = 0; i < list.length; i++) if (names.indexOf(list[i][0]) < 0) names.push(list[i][0]);
    names.sort();
    var out = [];
    names.forEach(function (n) {
      var vals = [];
      list.forEach(function (e) { if (e[0] === n) vals.push(e[1]); });
      if (n === 'set-cookie') vals.forEach(function (v) { out.push([n, v]); });
      else out.push([n, vals.join(', ')]);
    });
    return out;
  }
  function fillHeaders(h, init) {
    if (init === undefined || init === null) return;
    if (typeof init !== 'object' && typeof init !== 'function') throw new TypeError("Failed to construct 'Headers': The provided value is not of type '(record<ByteString, ByteString> or sequence<sequence<ByteString>>)'.");
    if (hdrMap.has(init)) { sortedEntries(init).forEach(function (e) { hAppend(h, e[0], e[1], 'append'); }); return; }
    if (typeof init[Symbol.iterator] === 'function') {
      for (var p of init) {
        var pair = Array.from(p);
        if (pair.length !== 2) throw new TypeError("Failed to construct 'Headers': Invalid value");
        hAppend(h, pair[0], pair[1], 'append');
      }
    } else {
      Reflect.ownKeys(init).forEach(function (k) {
        if (typeof k !== 'string') return;
        var d = Object.getOwnPropertyDescriptor(init, k);
        if (d && d.enumerable) hAppend(h, k, init[k], 'append');
      });
    }
  }
  function Headers(init) {
    if (!new.target) throw new TypeError("Failed to construct 'Headers': Please use the 'new' operator");
    hdrMap.set(this, { list: [], guard: 'none' });
    fillHeaders(this, init);
  }
  function mkHeaders(guard, init) {
    var h = new Headers();
    hs(h).guard = guard;
    fillHeaders(h, init);
    return h;
  }
  function copyHeaders(src, guard) {
    var h = new Headers(), st = hs(h);
    st.list = hs(src).list.map(function (e) { return [e[0], e[1]]; });
    st.guard = guard;
    return h;
  }
  var HP = Headers.prototype;
  def(HP, 'append', function append(n, v) {
    if (arguments.length < 2) throw new TypeError("Failed to execute 'append' on 'Headers': 2 arguments required, but only " + arguments.length + ' present.');
    hAppend(this, n, v, 'append');
  });
  def(HP, 'delete', function (n) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'delete' on 'Headers': 1 argument required, but only 0 present.");
    var st = hs(this), name = hName(n, 'delete');
    if (!hCheck(st, name, 'delete')) return;
    st.list = st.list.filter(function (e) { return e[0] !== name; });
  });
  def(HP, 'get', function get(n) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'get' on 'Headers': 1 argument required, but only 0 present.");
    var st = hs(this), name = hName(n, 'get'), vals = [];
    st.list.forEach(function (e) { if (e[0] === name) vals.push(e[1]); });
    return vals.length ? vals.join(', ') : null;
  });
  def(HP, 'getSetCookie', function getSetCookie() {
    var out = [];
    hs(this).list.forEach(function (e) { if (e[0] === 'set-cookie') out.push(e[1]); });
    return out;
  });
  def(HP, 'has', function has(n) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'has' on 'Headers': 1 argument required, but only 0 present.");
    var st = hs(this), name = hName(n, 'has');
    return st.list.some(function (e) { return e[0] === name; });
  });
  def(HP, 'set', function set(n, v) {
    if (arguments.length < 2) throw new TypeError("Failed to execute 'set' on 'Headers': 2 arguments required, but only " + arguments.length + ' present.');
    var st = hs(this), name = hName(n, 'set'), val = hValue(v, 'set');
    if (!hCheck(st, name, 'set')) return;
    var done = false, out = [];
    st.list.forEach(function (e) {
      if (e[0] !== name) out.push(e);
      else if (!done) { done = true; out.push([name, val]); }
    });
    if (!done) out.push([name, val]);
    st.list = out;
  });
  def(HP, 'forEach', function forEach(cb, thisArg) {
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'forEach' on 'Headers': parameter 1 is not of type 'Function'.");
    var self = this;
    sortedEntries(this).forEach(function (e) { cb.call(thisArg, e[1], e[0], self); });
  });
  def(HP, 'entries', function entries() { return sortedEntries(this).values(); });
  def(HP, 'keys', function keys() { return sortedEntries(this).map(function (e) { return e[0]; }).values(); });
  def(HP, 'values', function values() { return sortedEntries(this).map(function (e) { return e[1]; }).values(); });
  def(HP, Symbol.iterator, HP.entries);
  Object.defineProperty(HP, Symbol.toStringTag, { value: 'Headers', configurable: true });

  // ---- body mixin ----
  // A body is { bytes: Uint8Array|null, src: ReadableStream|null (a stream
  // given by the page, read fully on use), stream: the exposed stream
  // (created lazily over the bytes), type: content type or null, used }.
  function newBody(bytes, src, type) { return { bytes: bytes, src: src, stream: src, type: type, used: false }; }
  function hasBody(b) { return b.bytes !== null || b.src !== null; }
  function isUsed(b) { return b.used || (b.stream !== null && rsSt(b.stream).disturbed); }
  function extract(x) {
    if (typeof x === 'string') return newBody(utf8(x), null, 'text/plain;charset=UTF-8');
    if (isBlob(x)) return newBody(new Uint8Array(bytesOf.get(x)), null, x.type || null);
    if (g.URLSearchParams && x instanceof g.URLSearchParams) return newBody(utf8(x.toString()), null, 'application/x-www-form-urlencoded;charset=UTF-8');
    if (x instanceof ArrayBuffer) return newBody(new Uint8Array(x.slice(0)), null, null);
    if (ArrayBuffer.isView(x)) return newBody(new Uint8Array(x.buffer.slice(x.byteOffset, x.byteOffset + x.byteLength)), null, null);
    if (typeof g.FormData === 'function' && x instanceof g.FormData) { var mp = fdBody(x); return newBody(mp.bytes, null, mp.type); }
    if (x instanceof ReadableStream) {
      var s = rsSt(x);
      if (s.reader || s.disturbed) throw new TypeError('Response/Request body: the ReadableStream is locked or disturbed');
      return newBody(null, x, null);
    }
    return newBody(utf8(String(x)), null, 'text/plain;charset=UTF-8');
  }
  function bodyStream(b) {
    if (!hasBody(b)) return null;
    if (!b.stream) {
      var d = b.bytes;
      b.stream = new ReadableStream({ start: function (c) { if (d.length) c.enqueue(new Uint8Array(d)); c.close(); } });
    }
    return b.stream;
  }
  function readAll(stream) {
    var reader = stream.getReader(), chunks = [], total = 0;
    function step() {
      return reader.read().then(function (r) {
        if (r.done) return concat(chunks, total);
        var v = r.value, c;
        if (v instanceof ArrayBuffer) c = new Uint8Array(v);
        else if (ArrayBuffer.isView(v)) c = new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
        else throw new TypeError('Body stream chunk is not a Uint8Array');
        chunks.push(c); total += c.length;
        return step();
      });
    }
    return step();
  }
  function consume(b, how, cls) {
    if (isUsed(b) || (b.stream && rsSt(b.stream).reader))
      return Promise.reject(new TypeError("Failed to execute '" + how + "' on '" + cls + "': body already used"));
    b.used = true;
    if (b.src) return readAll(b.src);
    if (b.bytes === null) return Promise.resolve(new Uint8Array(0));
    if (b.stream) { var st = rsSt(b.stream); st.disturbed = true; }
    return Promise.resolve(b.bytes);
  }
  function mixin(P, cls, bodyOf, headersOf) {
    def(P, 'text', function text() { var self = this; try { return consume(bodyOf(self), 'text', cls).then(unutf8); } catch (e) { return Promise.reject(e); } });
    def(P, 'json', function json() { var self = this; try { return consume(bodyOf(self), 'json', cls).then(function (b) { return JSON.parse(unutf8(b)); }); } catch (e) { return Promise.reject(e); } });
    def(P, 'arrayBuffer', function arrayBuffer() { try { return consume(bodyOf(this), 'arrayBuffer', cls).then(function (b) { return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength); }); } catch (e) { return Promise.reject(e); } });
    def(P, 'bytes', function bytes() { try { return consume(bodyOf(this), 'bytes', cls).then(function (b) { return new Uint8Array(b); }); } catch (e) { return Promise.reject(e); } });
    def(P, 'blob', function blob() {
      var self = this;
      try {
        return consume(bodyOf(self), 'blob', cls).then(function (b) { return new Blob([b], { type: headersOf(self).get('content-type') || '' }); });
      } catch (e) { return Promise.reject(e); }
    });
    def(P, 'formData', function formData() {
      var self = this;
      try {
        var ct = headersOf(self).get('content-type') || '';
        if (typeof g.FormData !== 'function') return Promise.reject(domErr('NotSupportedError', "Failed to execute 'formData' on '" + cls + "': FormData is not implemented"));
        return consume(bodyOf(self), 'formData', cls).then(function (b) {
          if (!/^application\/x-www-form-urlencoded/i.test(ct)) throw new TypeError("Failed to execute 'formData' on '" + cls + "': Could not parse content as FormData.");
          var fd = new g.FormData(), p = new g.URLSearchParams(unutf8(b));
          p.forEach(function (v, k) { fd.append(k, v); });
          return fd;
        });
      } catch (e) { return Promise.reject(e); }
    });
    Object.defineProperty(P, 'body', { get: function () { return bodyStream(bodyOf(this)); }, enumerable: true, configurable: true });
    Object.defineProperty(P, 'bodyUsed', { get: function () { return isUsed(bodyOf(this)); }, enumerable: true, configurable: true });
  }
  function cloneBody(b, cls) {
    if (isUsed(b)) throw new TypeError("Failed to execute 'clone' on '" + cls + "': body is already used");
    if (b.src) throw new TypeError("Failed to execute 'clone' on '" + cls + "': cloning a streamed body is not implemented (no tee)");
    return newBody(b.bytes, null, b.type);
  }
  // QF10: synchronous body extraction for navigator.sendBeacon (internal).
  Object.defineProperty(g, '__beacon_body', { value: function (x) {
    if (x instanceof ReadableStream) throw new TypeError("Failed to execute 'sendBeacon' on 'Navigator': ReadableStream bodies are not supported.");
    var b = extract(x);
    return { bytes: b.bytes, type: b.type };
  }, writable: true, configurable: true, enumerable: false });
  function baseURL() {
    var d = g.document, b = d && d.baseURI;
    return b && b !== 'about:blank' ? b : (g.location && g.location.href) || undefined;
  }
  function parseURL(s, cls) {
    var u;
    try { u = new g.URL(String(s), baseURL()); } catch (e) { throw new TypeError("Failed to construct '" + cls + "': Failed to parse URL from " + s); }
    if (u.username || u.password) throw new TypeError("Failed to construct '" + cls + "': Request cannot be constructed from a URL that includes credentials: " + s);
    return u.href;
  }
  function isSignal(x) { return g.AbortSignal && x instanceof g.AbortSignal; }
  function follow(src) {
    var c = new g.AbortController();
    if (src) {
      if (src.aborted) c.abort(src.reason);
      else src.addEventListener('abort', function () { c.abort(src.reason); });
    }
    return c.signal;
  }
  function enumOf(v, list, name, cls, dflt) {
    if (v === undefined) return dflt;
    v = String(v);
    if (list.indexOf(v) < 0) throw new TypeError("Failed to construct '" + cls + "': The provided value '" + v + "' is not a valid enum value of type Request" + name[0].toUpperCase() + name.slice(1) + '.');
    return v;
  }

  // ---- Request ----
  var reqMap = new WeakMap();
  function rq(r) { var x = r !== null && typeof r === 'object' ? reqMap.get(r) : undefined; if (!x) throw new TypeError('Illegal invocation'); return x; }
  var METHOD_NORM = ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT'];
  function Request(input, init) {
    if (!new.target) throw new TypeError("Failed to construct 'Request': Please use the 'new' operator");
    if (arguments.length < 1) throw new TypeError("Failed to construct 'Request': 1 argument required, but only 0 present.");
    if (init !== undefined && init !== null && typeof init !== 'object' && typeof init !== 'function') throw new TypeError("Failed to construct 'Request': The provided value is not of type 'RequestInit'.");
    init = init || {};
    var inR = input !== null && typeof input === 'object' && reqMap.has(input) ? reqMap.get(input) : null;
    var r = inR
      ? { url: inR.url, method: inR.method, mode: inR.mode === 'navigate' ? 'same-origin' : inR.mode, credentials: inR.credentials, cache: inR.cache, redirect: inR.redirect, referrer: inR.referrer, referrerPolicy: inR.referrerPolicy, integrity: inR.integrity, keepalive: inR.keepalive }
      : { url: parseURL(input, 'Request'), method: 'GET', mode: 'cors', credentials: 'same-origin', cache: 'default', redirect: 'follow', referrer: 'about:client', referrerPolicy: '', integrity: '', keepalive: false };
    if (init.mode !== undefined) {
      if (String(init.mode) === 'navigate') throw new TypeError("Failed to construct 'Request': Cannot construct a Request with a RequestInit whose mode member is set as 'navigate'.");
      r.mode = enumOf(init.mode, ['same-origin', 'cors', 'no-cors'], 'mode', 'Request');
    }
    r.credentials = enumOf(init.credentials, ['omit', 'same-origin', 'include'], 'credentials', 'Request', r.credentials);
    r.cache = enumOf(init.cache, ['default', 'no-store', 'reload', 'no-cache', 'force-cache', 'only-if-cached'], 'cache', 'Request', r.cache);
    r.redirect = enumOf(init.redirect, ['follow', 'error', 'manual'], 'redirect', 'Request', r.redirect);
    if (init.referrerPolicy !== undefined) r.referrerPolicy = enumOf(init.referrerPolicy, ['', 'no-referrer', 'no-referrer-when-downgrade', 'same-origin', 'origin', 'strict-origin', 'origin-when-cross-origin', 'strict-origin-when-cross-origin', 'unsafe-url'], 'referrerPolicy', 'Request');
    if (init.referrer !== undefined) {
      var rf = String(init.referrer);
      r.referrer = rf === '' ? '' : rf === 'about:client' ? rf : parseURL(rf, 'Request');
    }
    if (init.integrity !== undefined) r.integrity = String(init.integrity);
    if (init.keepalive !== undefined) r.keepalive = !!init.keepalive;
    if (init.method !== undefined) {
      var m = String(init.method);
      if (!TOKEN_RE.test(m)) throw new TypeError("Failed to construct 'Request': '" + m + "' is not a valid HTTP method.");
      var mu = m.toUpperCase();
      if (mu === 'CONNECT' || mu === 'TRACE' || mu === 'TRACK') throw new TypeError("Failed to construct 'Request': '" + m + "' HTTP method is unsupported.");
      r.method = METHOD_NORM.indexOf(mu) >= 0 ? mu : m;
    }
    var srcSig;
    if (init.signal !== undefined) {
      if (init.signal !== null && !isSignal(init.signal)) throw new TypeError("Failed to construct 'Request': Failed to read the 'signal' property from 'RequestInit': Failed to convert value to 'AbortSignal'.");
      srcSig = init.signal;
    } else srcSig = inR ? inR.signal : null;
    r.signal = follow(srcSig);
    var body = null, ib = init.body, hasIb = ib !== undefined && ib !== null;
    if ((hasIb || (inR && hasBody(inR.body))) && (r.method === 'GET' || r.method === 'HEAD'))
      throw new TypeError("Failed to construct 'Request': Request with GET/HEAD method cannot have body.");
    if (hasIb) body = extract(ib);
    else if (inR && hasBody(inR.body)) {
      if (isUsed(inR.body)) throw new TypeError("Failed to construct 'Request': Cannot construct a Request with a Request object that has already been used.");
      body = newBody(inR.body.bytes, inR.body.src, inR.body.type);
      inR.body.used = true;
    }
    r.headers = mkHeaders('request', init.headers !== undefined ? init.headers : inR ? inR.headers : undefined);
    if (hasIb && body.type && !r.headers.has('content-type')) r.headers.append('content-type', body.type);
    r.body = body || newBody(null, null, null);
    reqMap.set(this, r);
  }
  var RQP = Request.prototype;
  ['url', 'method', 'mode', 'credentials', 'cache', 'redirect', 'referrerPolicy', 'integrity', 'keepalive', 'headers', 'signal'].forEach(function (k) {
    Object.defineProperty(RQP, k, { get: function () { return rq(this)[k]; }, enumerable: true, configurable: true });
  });
  Object.defineProperty(RQP, 'referrer', { get: function () { return rq(this).referrer; }, enumerable: true, configurable: true });
  Object.defineProperty(RQP, 'destination', { get: function () { rq(this); return ''; }, enumerable: true, configurable: true });
  Object.defineProperty(RQP, 'duplex', { get: function () { rq(this); return 'half'; }, enumerable: true, configurable: true });
  mixin(RQP, 'Request', function (x) { return rq(x).body; }, function (x) { return rq(x).headers; });
  def(RQP, 'clone', function clone() {
    var r = rq(this), b = cloneBody(r.body, 'Request');
    var c = Object.create(Request.prototype), n = {};
    Object.keys(r).forEach(function (k) { n[k] = r[k]; });
    n.headers = copyHeaders(r.headers, 'request');
    n.signal = follow(r.signal);
    n.body = b;
    reqMap.set(c, n);
    return c;
  });
  Object.defineProperty(RQP, Symbol.toStringTag, { value: 'Request', configurable: true });

  // ---- Response ----
  var resMap = new WeakMap();
  function rp(r) { var x = r !== null && typeof r === 'object' ? resMap.get(r) : undefined; if (!x) throw new TypeError('Illegal invocation'); return x; }
  var NULL_BODY = [101, 103, 204, 205, 304];
  function initResponse(self, bodyInit, init, fixed) {
    init = init || {};
    var status = 200, text = '';
    if (init.status !== undefined) {
      status = Math.trunc(Number(init.status));
      if (!(status >= 200 && status <= 599)) throw new RangeError("Failed to construct 'Response': The status provided (" + init.status + ') is outside the range [200, 599].');
    }
    if (init.statusText !== undefined) {
      text = String(init.statusText);
      if (!/^[\t\x20-\x7E\x80-\xFF]*$/.test(text)) throw new TypeError("Failed to construct 'Response': Invalid statusText");
    }
    var r = { status: status, statusText: text, type: 'default', url: '', redirected: false };
    r.headers = mkHeaders('response', init.headers);
    var body = null;
    if (bodyInit !== undefined && bodyInit !== null) {
      if (NULL_BODY.indexOf(status) >= 0) throw new TypeError("Failed to construct 'Response': Response with null body status cannot have body");
      body = extract(bodyInit);
      if (body.type && !r.headers.has('content-type')) r.headers.append('content-type', body.type);
    }
    r.body = body || newBody(null, null, null);
    if (fixed) fixed(r);
    resMap.set(self, r);
  }
  function Response(body, init) {
    if (!new.target) throw new TypeError("Failed to construct 'Response': Please use the 'new' operator");
    if (init !== undefined && init !== null && typeof init !== 'object' && typeof init !== 'function') throw new TypeError("Failed to construct 'Response': The provided value is not of type 'ResponseInit'.");
    initResponse(this, body, init);
  }
  var RPP = Response.prototype;
  ['status', 'statusText', 'type', 'url', 'redirected', 'headers'].forEach(function (k) {
    Object.defineProperty(RPP, k, { get: function () { return rp(this)[k]; }, enumerable: true, configurable: true });
  });
  Object.defineProperty(RPP, 'ok', { get: function () { var s = rp(this).status; return s >= 200 && s <= 299; }, enumerable: true, configurable: true });
  mixin(RPP, 'Response', function (x) { return rp(x).body; }, function (x) { return rp(x).headers; });
  def(RPP, 'clone', function clone() {
    var r = rp(this), b = cloneBody(r.body, 'Response');
    var c = Object.create(Response.prototype), n = {};
    Object.keys(r).forEach(function (k) { n[k] = r[k]; });
    n.headers = copyHeaders(r.headers, hs(r.headers).guard);
    n.body = b;
    resMap.set(c, n);
    return c;
  });
  Object.defineProperty(RPP, Symbol.toStringTag, { value: 'Response', configurable: true });
  def(Response, 'error', function error() {
    var r = Object.create(Response.prototype);
    initResponse(r, null, { status: 200 }, function (s) { s.status = 0; s.type = 'error'; hs(s.headers).guard = 'immutable'; });
    return r;
  });
  def(Response, 'redirect', function redirect(url, status) {
    status = status === undefined ? 302 : Math.trunc(Number(status));
    if ([301, 302, 303, 307, 308].indexOf(status) < 0) throw new RangeError("Failed to execute 'redirect' on 'Response': Invalid status code");
    var u;
    try { u = new g.URL(String(url), baseURL()).href; } catch (e) { throw new TypeError("Failed to execute 'redirect' on 'Response': Invalid URL"); }
    var r = Object.create(Response.prototype);
    initResponse(r, null, { status: 200 }, function (s) { s.status = status; s.headers.append('location', u); hs(s.headers).guard = 'immutable'; });
    return r;
  });
  def(Response, 'json', function json(data, init) {
    var s = JSON.stringify(data);
    if (s === undefined) throw new TypeError("Failed to execute 'json' on 'Response': The data is not JSON serializable");
    var r = Object.create(Response.prototype);
    var hh = new Headers(init && init.headers);
    if (!hh.has('content-type')) hh.set('content-type', 'application/json');
    var ini = {}; if (init) Object.keys(init).forEach(function (k) { ini[k] = init[k]; });
    ini.headers = hh;
    initResponse(r, s, ini);
    return r;
  });
  Object.defineProperty(Response.prototype, 'constructor', { value: Response, writable: true, configurable: true });

  // INTERNAL HOOK for the network layer (QF7/QF8): build a real Response.
  //   __make_response(status, statusText, headers, bodyBytes, url, redirected, type)
  // headers: array of [name, value] pairs in wire order (Set-Cookie kept
  // as separate pairs); bodyBytes: Uint8Array/ArrayBuffer/null; type:
  // 'basic' | 'cors' | 'opaque' | 'opaqueredirect' | 'error' | 'default'.
  // The headers object gets the 'immutable' guard; basic/cors responses
  // have Set-Cookie/Set-Cookie2 filtered out; opaque/opaqueredirect/error
  // get status 0, no headers, no body and url ''. (CORS-safelisted header
  // filtering for 'cors' is the caller's job.)
  def(g, '__make_response', function (status, statusText, headers, bodyBytes, url, redirected, type) {
    var r = Object.create(Response.prototype);
    type = type === undefined ? 'basic' : String(type);
    var hidden = type === 'opaque' || type === 'opaqueredirect' || type === 'error';
    var h = new Headers();
    if (!hidden) (headers || []).forEach(function (p) {
      var n = String(p[0]).toLowerCase();
      if ((type === 'basic' || type === 'cors') && forbiddenRes(n)) return;
      hs(h).list.push([hName(n, 'append'), hValue(p[1], 'append')]);
    });
    hs(h).guard = 'immutable';
    var b = null;
    if (!hidden && bodyBytes !== null && bodyBytes !== undefined) b = bodyBytes instanceof ArrayBuffer ? new Uint8Array(bodyBytes) : new Uint8Array(bodyBytes.buffer.slice(bodyBytes.byteOffset, bodyBytes.byteOffset + bodyBytes.byteLength));
    resMap.set(r, {
      status: hidden ? 0 : Number(status) | 0, statusText: hidden ? '' : String(statusText || ''), type: type,
      url: hidden ? '' : String(url || ''), redirected: !!redirected, headers: h,
      body: newBody(b, null, null),
    });
    return r;
  });
  def(g, 'Headers', Headers); def(g, 'Request', Request); def(g, 'Response', Response);
  // Shared with the XMLHttpRequest layer (QF9) below; not part of the platform.
  def(g, '__xhr_internals', {
    forbiddenReq: forbiddenReq, extract: extract, unutf8: unutf8, tokenRe: TOKEN_RE,
    resBytes: function (r) { var b = rp(r).body.bytes; return b === null ? new Uint8Array(0) : b; },
  });
})(globalThis);

// QF7: the INTERNAL request helper fetch (QF8) and XMLHttpRequest (QF9) are
// built on. Not part of the web platform; the names are non-enumerable.
//
//   __net_request(init) -> { id, promise, abort(reason) }          (async)
//   __net_request(init) -> { id, response }                        (init.sync)
//
// init: { method, url (absolute), headers: [[name, value]...], body (Uint8Array
// | null), mode: 'cors'|'no-cors'|'same-origin', credentials, keepalive,
// api: 'fetch'|'xhr'|'beacon', sync, signal }.
// The promise resolves with a Response (built by __make_response) or rejects
// with TypeError('Failed to fetch') (policy, CORS, transport), the signal's
// reason / an AbortError, or a TimeoutError DOMException. A sync request
// throws a 'NetworkError' DOMException instead.
// Native side: `__net_send(desc) -> id`, `__net_abort(id)`,
// `__net_sync_result(id)`; the engine calls `__net_complete(result)`.
(function (g) {
  'use strict';
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function domErr(name, msg) {
    if (typeof g.DOMException === 'function') return new g.DOMException(msg, name);
    var e = new Error(msg); e.name = name; return e;
  }
  var pending = {};
  function binary(b) {
    if (b === null || b === undefined) return '';
    var s = '', i = 0, CH = 8192;
    for (; i < b.length; i += CH) s += String.fromCharCode.apply(null, b.subarray(i, i + CH));
    return s;
  }
  function descOf(init) {
    return {
      method: String(init.method || 'GET'), url: String(init.url), headers: init.headers || [],
      body: binary(init.body), mode: init.mode || 'cors', credentials: init.credentials || 'same-origin',
      keepalive: !!init.keepalive, api: init.api || 'fetch', sync: !!init.sync,
    };
  }
  function build(res) {
    return g.__make_response(res.status, res.statusText, res.headers, res.body, res.finalUrl, res.redirected, res.type);
  }
  function failure(res) {
    if (res.error === 'timeout') return domErr('TimeoutError', 'The request timed out.');
    return new TypeError('Failed to fetch');
  }
  // `mk` (QPT1a): an iframe realm's own response builder, so its fetch/XHR answers are its own
  // `Response` objects although the request goes through the page's queue (`__net_fwd`).
  def(g, '__net_request', function (init, mk, owner) {
    var signal = init.signal;
    mk = mk || build;
    if (init.sync) {
      var sid = g.__net_send(descOf(init));
      var sres = g.__net_sync_result(sid);
      if (!sres || sres.error) throw domErr('NetworkError', "Failed to execute 'send' on 'XMLHttpRequest': Failed to load '" + init.url + "'.");
      return { id: sid, response: mk(sres) };
    }
    if (signal && signal.aborted) {
      return { id: 0, promise: Promise.reject(signal.reason), abort: function () {} };
    }
    var h = { id: 0 };
    h.promise = new Promise(function (resolve, reject) {
      var onabort = null;
      var entry = { raw: !!init.raw, mk: mk, owner: owner, resolve: resolve, reject: reject, cleanup: function () { if (signal && onabort) signal.removeEventListener('abort', onabort); } };
      h.id = g.__net_send(descOf(init));
      pending[h.id] = entry;
      h.abort = function (reason) {
        if (!pending[h.id]) return;
        delete pending[h.id];
        entry.cleanup();
        g.__net_abort(h.id);
        reject(reason === undefined ? domErr('AbortError', 'The user aborted a request.') : reason);
      };
      if (signal) {
        onabort = function () { h.abort(signal.reason); };
        signal.addEventListener('abort', onabort);
      }
    });
    return h;
  });
  // QPT1b (R53 D6): a dropped realm's pending requests (`owner` = its window) are aborted and forgotten:
  // their promises stay unsettled, so nothing runs in (or keeps alive) the dead realm.
  def(g, '__net_cancel', function (o) {
    Object.keys(pending).forEach(function (id) {
      var e = pending[id];
      if (e.owner !== o) return;
      delete pending[id];
      e.cleanup();
      g.__net_abort(+id);
    });
  });
  def(g, '__net_complete', function (res) {
    var p = pending[res.id];
    if (!p) return;   // aborted
    delete pending[res.id];
    p.cleanup();
    if (res.error) p.reject(failure(res));
    else p.resolve(p.raw ? res : p.mk(res));
  });
  // QPT1a (R53 D5): an iframe realm sends through its parent's request table (Rust completes requests
  // in the page context only), building the answers with its own classes.
  def(g, '__net_fwd', function (p) {
    var pr = p.__net_request;
    // QPT1d3: a CLOSED window sends nothing: a sync XHR throws NetworkError, an async request/fetch gets a
    // handle whose promise never settles (same as the pending requests QPT1b cancels on release).
    def(g, '__net_request', function (init, mk, owner) {
      if (g.closed) {
        if (init.sync) throw domErr('NetworkError', "Failed to execute 'send' on 'XMLHttpRequest': Failed to load '" + init.url + "'.");
        return { id: 0, promise: new Promise(function () {}), abort: function () {} };
      }
      return pr(init, mk || build, owner || g);
    });
    // QPT1b (R53 D6): the requests of a dropped realm are cancelled in the table that holds them.
    var pc = p.__net_cancel;
    def(g, '__net_cancel', function (o) { return pc(o); });
    delete g.__net_fwd;
  });
})(globalThis);

// QF10: navigator.sendBeacon(url, data) over __net_request (api 'beacon').
// Arguments are validated here (TypeError for a bad or non-http(s) URL),
// the 64 KiB queue limit (policy `beacon_queue_bytes`, via
// `__net_beacon_limit`) makes it return false; everything else, the policy
// checks, the detailed log line and `beacons_send` (QF-D3: false = logged,
// NOT sent) lives in the native `__net_send`. It returns true on queueing.
(function (g) {
  'use strict';
  var queued = 0;
  var SAFE = /^(text\/plain|multipart\/form-data|application\/x-www-form-urlencoded)\s*(;|$)/i;
  function sendBeacon(url) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'sendBeacon' on 'Navigator': 1 argument required, but only 0 present.");
    var base = (g.document && g.document.baseURI) || (g.location && g.location.href) || undefined, u;
    try { u = new g.URL(String(url), base); } catch (e) { throw new TypeError("Failed to execute 'sendBeacon' on 'Navigator': Invalid URL '" + url + "'."); }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') throw new TypeError("Failed to execute 'sendBeacon' on 'Navigator': Beacons are only supported over HTTP(S).");
    var data = arguments[1], bytes = null, type = null;
    if (data !== undefined && data !== null) { var b = g.__beacon_body(data); bytes = b.bytes; type = b.type; }
    var size = bytes ? bytes.length : 0;
    var limit = typeof g.__net_beacon_limit === 'function' ? g.__net_beacon_limit() : 65536;
    if (queued + size > limit) return false;
    var hs = type ? [['Content-Type', type]] : [];
    var h;
    try {
      h = g.__net_request({ method: 'POST', url: u.href, headers: hs, body: bytes, mode: type && !SAFE.test(type) ? 'cors' : 'no-cors',
        credentials: 'include', keepalive: true, api: 'beacon' });
    } catch (e) { return false; }
    queued += size;
    var done = function () { queued -= size; };
    h.promise.then(done, done);
    return true;
  }
  Object.defineProperty(sendBeacon, 'name', { value: 'sendBeacon', configurable: true });
  Object.defineProperty(g.Navigator.prototype, 'sendBeacon', { value: sendBeacon, writable: true, configurable: true, enumerable: true });
})(globalThis);

// QF8: window.fetch(input, init), a thin layer over __net_request (QF7).
// Request (QF4) validates and normalises the arguments; this layer only
// routes: data: and blob: URLs are answered locally (no network, no policy);
// http(s) (and file:, which the policy warns about) goes to __net_request,
// which applies the request policy, CORS, beacon suppression and returns a
// real Response. Other schemes reject with TypeError.
// Ignored on purpose (documented in NOTES.md): cache, referrer,
// referrerPolicy, integrity, priority; redirect:'manual' behaves as
// 'follow'; credentials are accepted but no cookie jar exists yet (QF14).
(function (g) {
  'use strict';
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: true }); }
  var te = function (m) { return new TypeError(m); };
  function local(bytes, type, url) {
    var h = [['content-type', type], ['content-length', String(bytes.length)]];
    return g.__make_response(200, 'OK', h, bytes, url, false, 'basic');
  }
  function pctBytes(s) {
    return s.replace(/%([0-9a-fA-F]{2})/g, function (_, h) { return String.fromCharCode(parseInt(h, 16)); });
  }
  function dataURL(url) {
    var rest = url.slice(5), hash = rest.indexOf('#');
    if (hash >= 0) rest = rest.slice(0, hash);
    var comma = rest.indexOf(',');
    if (comma < 0) return null;
    var meta = rest.slice(0, comma), data = rest.slice(comma + 1), b64 = false;
    var m = /;\s*base64\s*$/i.exec(meta);
    if (m) { b64 = true; meta = meta.slice(0, m.index); }
    meta = meta.trim();
    var type = meta === '' || meta[0] === ';' ? 'text/plain' + (meta ? meta : ';charset=US-ASCII') : meta;
    if (b64) {
      var bin;
      try { bin = g.atob(pctBytes(data).replace(/[\t\n\f\r ]/g, '')); } catch (e) { return null; }
      var out = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return { bytes: out, type: type };
    }
    var bytes = [];
    for (var j = 0; j < data.length; j++) {
      var c = data.charCodeAt(j);
      if (c === 37 && /^[0-9a-fA-F]{2}$/.test(data.substr(j + 1, 2))) { bytes.push(parseInt(data.substr(j + 1, 2), 16)); j += 2; }
      else if (c < 128) bytes.push(c);
      else {
        var ch = data.charAt(j) + (c >= 0xD800 && c <= 0xDBFF ? data.charAt(++j) : '');
        var u = new g.TextEncoder().encode(ch);
        for (var k = 0; k < u.length; k++) bytes.push(u[k]);
      }
    }
    return { bytes: new Uint8Array(bytes), type: type };
  }
  function fetch(input) {
    var init = arguments[1];
    var req;
    if (arguments.length < 1) return Promise.reject(te("Failed to execute 'fetch' on 'Window': 1 argument required, but only 0 present."));
    try { req = new g.Request(input, init); } catch (e) { return Promise.reject(e); }
    var signal = req.signal;
    if (signal.aborted) return Promise.reject(signal.reason);
    var url = req.url, scheme = /^([a-z][a-z0-9+.-]*):/i.exec(url);
    scheme = scheme ? scheme[1].toLowerCase() : '';
    if (scheme === 'data' || scheme === 'blob') {
      if (req.method !== 'GET') return Promise.reject(te('Failed to fetch'));
      if (scheme === 'data') {
        var d = dataURL(url);
        if (!d) return Promise.reject(te('Failed to fetch'));
        return Promise.resolve(local(d.bytes, d.type, url));
      }
      var blob = g.__blob_lookup(url);
      if (!blob || typeof blob.arrayBuffer !== 'function') return Promise.reject(te('Failed to fetch'));
      return blob.arrayBuffer().then(function (ab) {
        if (signal.aborted) throw signal.reason;
        return local(new Uint8Array(ab), blob.type || '', url);
      });
    }
    if (scheme !== 'http' && scheme !== 'https' && scheme !== 'file') return Promise.reject(te('Failed to fetch'));
    var go = function (bytes) {
      if (signal.aborted) throw signal.reason;
      var hs = [];
      req.headers.forEach(function (v, n) { hs.push([n, v]); });
      var h = g.__net_request({
        method: req.method, url: url, headers: hs, body: bytes, mode: req.mode, credentials: req.credentials,
        keepalive: req.keepalive, api: 'fetch', signal: signal,
      });
      return h.promise.then(function (res) {
        if (req.redirect === 'error' && res.redirected) throw te('Failed to fetch');
        return res;
      });
    };
    if (req.body === null) { try { return Promise.resolve(go(null)); } catch (e) { return Promise.reject(e); } }
    return req.arrayBuffer().then(function (ab) { return go(new Uint8Array(ab)); });
  }
  Object.defineProperty(fetch, 'name', { value: 'fetch', configurable: true });
  def(g, 'fetch', fetch);
})(globalThis);

// QF9: XMLHttpRequest, a thin layer over __net_request (QF7), so it shares
// the request policy, CORS checks, size/time limits and logging with fetch.
// Async: events fire from the promise the helper returns. Sync
// (`open(.., false)`): `send()` BLOCKS the engine thread in __net_request
// (logged by QF7; bounded by the policy's sync_xhr_timeout_ms) and throws a
// NetworkError DOMException on failure. Documented deviations: `document`
// responseType yields null (and logs) until a DOMParser exists; the body
// arrives in one piece (one `progress`, state 3 once); no cookies
// (withCredentials is stored only, QF14); `upload` fires loadstart/progress/
// load/loadend at the end but never reports real upload progress; user and
// password arguments of open() are ignored; as in browsers, responseType and
// timeout on a sync XHR throw InvalidAccessError.
(function (g) {
  'use strict';
  var H = g.__xhr_internals;
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: true }); }
  function domErr(name, msg) {
    if (typeof g.DOMException === 'function') return new g.DOMException(msg, name);
    var e = new Error(msg); e.name = name; return e;
  }
  function inherit(Child, Parent) {
    Child.prototype = Object.create(Parent.prototype, { constructor: { value: Child, writable: true, configurable: true } });
    Object.setPrototypeOf(Child, Parent);
  }
  var map = new WeakMap();
  function st(x) { var s = x !== null && typeof x === 'object' ? map.get(x) : undefined; if (!s) throw new TypeError('Illegal invocation'); return s; }
  function XMLHttpRequestEventTarget() { throw new TypeError('Illegal constructor'); }
  inherit(XMLHttpRequestEventTarget, g.EventTarget);
  function XMLHttpRequestUpload() { throw new TypeError('Illegal constructor'); }
  inherit(XMLHttpRequestUpload, XMLHttpRequestEventTarget);
  function XMLHttpRequest() {
    if (!new.target) throw new TypeError("Failed to construct 'XMLHttpRequest': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var x = Reflect.construct(g.EventTarget, [], new.target);
    var up = Reflect.construct(g.EventTarget, [], XMLHttpRequestUpload);
    map.set(up, { h: {} });
    map.set(x, {
      state: 0, sent: false, async: true, method: 'GET', url: '', headers: [], timeout: 0, withCredentials: false,
      responseType: '', status: 0, statusText: '', resHeaders: [], body: null, finalUrl: '', mime: null, gen: 0,
      timer: 0, handle: null, hasBody: false, h: {}, upload: up, cache: undefined, docLogged: false,
    });
    return x;
  }
  inherit(XMLHttpRequest, XMLHttpRequestEventTarget);
  var XP = XMLHttpRequest.prototype;
  [['UNSENT', 0], ['OPENED', 1], ['HEADERS_RECEIVED', 2], ['LOADING', 3], ['DONE', 4]].forEach(function (p) {
    Object.defineProperty(XMLHttpRequest, p[0], { value: p[1], enumerable: true });
    Object.defineProperty(XP, p[0], { value: p[1], enumerable: true });
  });
  ['loadstart', 'progress', 'abort', 'error', 'load', 'timeout', 'loadend'].forEach(function (t) {
    Object.defineProperty(XMLHttpRequestEventTarget.prototype, 'on' + t, {
      get: function () { var h = st(this).h[t]; return h === undefined ? null : h; },
      set: function (f) { st(this).h[t] = typeof f === 'function' ? f : null; },
      enumerable: true, configurable: true,
    });
  });
  Object.defineProperty(XP, 'onreadystatechange', {
    get: function () { var h = st(this).h.readystatechange; return h === undefined ? null : h; },
    set: function (f) { st(this).h.readystatechange = typeof f === 'function' ? f : null; },
    enumerable: true, configurable: true,
  });
  function fire(target, type, loaded, total, lc) {
    var e = type === 'readystatechange' ? new g.Event(type) : new g.ProgressEvent(type, { lengthComputable: !!lc, loaded: loaded || 0, total: total || 0 });
    target.dispatchEvent(e);
  }
  function rsc(x) { fire(x, 'readystatechange'); }
  function bad(what, name, msg) { throw domErr(name, "Failed to execute '" + what + "' on 'XMLHttpRequest': " + msg); }
  function terminate(s) {
    s.gen++;
    if (s.timer) { g.clearTimeout(s.timer); s.timer = 0; }
    var h = s.handle; s.handle = null;
    if (h && h.abort) h.abort();
  }
  function resetResponse(s) {
    s.status = 0; s.statusText = ''; s.resHeaders = []; s.body = null; s.finalUrl = ''; s.cache = undefined; s.xcache = undefined;
  }
  var METHODS = ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT'];

  def(XP, 'open', function open(method, url) {
    var x = this, s = st(x);
    if (arguments.length < 2) throw new TypeError("Failed to execute 'open' on 'XMLHttpRequest': 2 arguments required, but only " + arguments.length + ' present.');
    method = String(method);
    if (!H.tokenRe.test(method)) bad('open', 'SyntaxError', "'" + method + "' is not a valid HTTP method.");
    var up = method.toUpperCase();
    if (up === 'CONNECT' || up === 'TRACE' || up === 'TRACK') bad('open', 'SecurityError', "'" + method + "' HTTP method is unsupported.");
    if (METHODS.indexOf(up) >= 0) method = up;
    var base = (g.document && g.document.baseURI) || (g.location && g.location.href) || undefined, u;
    try { u = new g.URL(String(url), base); } catch (e) { bad('open', 'SyntaxError', "Invalid URL '" + String(url) + "'"); }
    var async = arguments.length > 2 ? !!arguments[2] : true;
    if (!async && (s.timeout !== 0 || s.responseType !== '')) {
      bad('open', 'InvalidAccessError', 'Synchronous requests must not set a timeout or responseType.');
    }
    terminate(s);
    s.method = method; s.url = u.href; s.async = async; s.headers = []; s.sent = false; s.hasBody = false;
    resetResponse(s);
    var was = s.state;
    s.state = 1;
    if (was !== 1) rsc(x);
  });
  def(XP, 'setRequestHeader', function setRequestHeader(name, value) {
    var s = st(this);
    if (arguments.length < 2) throw new TypeError("Failed to execute 'setRequestHeader' on 'XMLHttpRequest': 2 arguments required, but only " + arguments.length + ' present.');
    if (s.state !== 1 || s.sent) bad('setRequestHeader', 'InvalidStateError', "The object's state must be OPENED.");
    name = String(name);
    if (!H.tokenRe.test(name)) bad('setRequestHeader', 'SyntaxError', "'" + name + "' is not a valid HTTP header field name.");
    value = String(value).replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, '');
    if (/[\0\r\n]/.test(value)) bad('setRequestHeader', 'SyntaxError', "'" + value + "' is not a valid HTTP header field value.");
    var n = name.toLowerCase();
    if (H.forbiddenReq(n)) return;
    for (var i = 0; i < s.headers.length; i++) {
      if (s.headers[i][0] === n) { s.headers[i][1] += ', ' + value; return; }
    }
    s.headers.push([n, value]);
  });
  function hasHeader(list, n) { return list.some(function (e) { return e[0] === n; }); }
  function finishError(x, s, type) {
    s.sent = false; s.state = 4; s.handle = null; resetResponse(s);
    rsc(x);
    if (s.hasBody) { fire(s.upload, type, 0, 0); fire(s.upload, 'loadend', 0, 0); }
    fire(x, type, 0, 0);
    fire(x, 'loadend', 0, 0);
  }
  function setResponse(s, res) {
    s.status = res.status; s.statusText = res.statusText; s.finalUrl = String(res.url || '').replace(/#.*$/, '');
    var list = [];
    res.headers.forEach(function (v, n) { if (n !== 'set-cookie' && n !== 'set-cookie2') list.push([n, v]); });
    s.resHeaders = list;
    s.body = H.resBytes(res);
  }
  function headerVal(s, n) {
    for (var i = 0; i < s.resHeaders.length; i++) if (s.resHeaders[i][0] === n) return s.resHeaders[i][1];
    return null;
  }
  function handleResponse(x, s, gen, res) {
    if (s.gen !== gen) return;
    if (s.timer) { g.clearTimeout(s.timer); s.timer = 0; }
    var bytes = H.resBytes(res);
    if (s.hasBody) {
      fire(s.upload, 'progress', 0, 0); fire(s.upload, 'load', 0, 0); fire(s.upload, 'loadend', 0, 0);
      if (s.gen !== gen) return;
    }
    setResponse(s, res); s.body = null;
    var cl = headerVal(s, 'content-length'), lc = cl !== null && /^\d+$/.test(cl);
    var total = lc ? parseInt(cl, 10) : bytes.length;
    s.state = 2; rsc(x);
    if (s.gen !== gen) return;
    s.body = bytes;
    if (bytes.length) {
      s.state = 3; rsc(x);
      if (s.gen !== gen) return;
      fire(x, 'progress', bytes.length, total, lc);
      if (s.gen !== gen) return;
    }
    s.sent = false; s.handle = null; s.state = 4; rsc(x);
    if (s.gen !== gen) return;
    fire(x, 'load', bytes.length, total, lc);
    fire(x, 'loadend', bytes.length, total, lc);
  }
  def(XP, 'send', function send(body) {
    var x = this, s = st(x);
    if (s.state !== 1 || s.sent) bad('send', 'InvalidStateError', "The object's state must be OPENED.");
    var bytes = null, ctype = null;
    if (body !== undefined && body !== null && s.method !== 'GET' && s.method !== 'HEAD') {
      if (typeof body === 'object' && body.nodeType === 9) {
        var dt = body.doctype;
        bytes = new g.TextEncoder().encode((dt ? '<!DOCTYPE ' + (dt.name || 'html') + '>' : '') + (body.documentElement ? body.documentElement.outerHTML : ''));
        ctype = 'text/html;charset=UTF-8';
      } else {
        var b = H.extract(body);
        bytes = b.bytes; ctype = b.type;
      }
    }
    var hs = s.headers.map(function (e) { return [e[0], e[1]]; });
    if (ctype && !hasHeader(hs, 'content-type')) hs.push(['content-type', ctype]);
    s.hasBody = bytes !== null;
    resetResponse(s);
    var init = {
      method: s.method, url: s.url, headers: hs, body: bytes, mode: 'cors',
      credentials: s.withCredentials ? 'include' : 'same-origin', api: 'xhr', sync: !s.async,
    };
    if (!s.async) {
      s.sent = true;
      var h;
      try { h = g.__net_request(init); }
      catch (e) { s.sent = false; s.state = 4; resetResponse(s); throw e; }
      s.sent = false;
      setResponse(s, h.response);
      s.state = 4; rsc(x);
      fire(x, 'load', s.body.length, s.body.length, true);
      fire(x, 'loadend', s.body.length, s.body.length, true);
      return;
    }
    s.sent = true;
    s.gen++;
    var gen = s.gen;
    fire(x, 'loadstart', 0, 0);
    if (s.gen !== gen) return;
    if (s.hasBody) { fire(s.upload, 'loadstart', 0, bytes.length, true); if (s.gen !== gen) return; }
    var hd = g.__net_request(init);
    s.handle = hd;
    if (s.timeout > 0) {
      s.timer = g.setTimeout(function () {
        if (s.gen !== gen) return;
        s.timer = 0;
        terminate(s);
        finishError(x, s, 'timeout');
      }, s.timeout);
    }
    hd.promise.then(function (res) { handleResponse(x, s, gen, res); }, function () {
      if (s.gen !== gen) return;
      if (s.timer) { g.clearTimeout(s.timer); s.timer = 0; }
      s.handle = null;
      finishError(x, s, 'error');
    });
  });
  def(XP, 'abort', function abort() {
    var x = this, s = st(x);
    terminate(s);
    if ((s.state === 1 && s.sent) || s.state === 2 || s.state === 3) finishError(x, s, 'abort');
    if (s.state === 4) { s.state = 0; resetResponse(s); }
  });
  function isText(s) { return s.responseType === '' || s.responseType === 'text'; }
  function charsetOf(s) {
    var ct = s.mime || headerVal(s, 'content-type') || '', m = /charset\s*=\s*"?([^";\s]+)/i.exec(ct);
    return m ? m[1] : 'utf-8';
  }
  function textOf(s) {
    if (s.body === null) return '';
    var cs = charsetOf(s);
    if (/^utf-?8$/i.test(cs) || typeof g.TextDecoder !== 'function') return H.unutf8(s.body);
    try { return new g.TextDecoder(cs).decode(s.body); } catch (e) { return H.unutf8(s.body); }
  }
  // QF9a: the response as an inert Document through DOMParser's machinery.
  function docOf(s, htmlOk) {
    if (s.xcache !== undefined) return s.xcache;
    var ct = (s.mime || headerVal(s, 'content-type') || '').split(';')[0].replace(/^\s+|\s+$/g, '').toLowerCase();
    var d = null;
    if (typeof g.__parseDocument === 'function') {
      if (ct === 'text/html') { if (htmlOk && s.async) d = g.__parseDocument(textOf(s), 'text/html'); }
      else if (ct === '') d = g.__parseDocument(textOf(s), 'text/xml');
      else if (ct === 'text/xml' || ct === 'application/xml' || ct === 'application/xhtml+xml' || ct === 'image/svg+xml') d = g.__parseDocument(textOf(s), ct);
      else if (/\+xml$/.test(ct)) d = g.__parseDocument(textOf(s), 'application/xml');
    }
    s.xcache = d;
    return d;
  }
  function responseValue(s) {
    var t = s.responseType;
    if (isText(s)) return s.state === 3 || s.state === 4 ? textOf(s) : '';
    if (s.state !== 4) return null;
    if (s.cache !== undefined) return s.cache;
    var v = null;
    if (t === 'arraybuffer') v = s.body.buffer.slice(s.body.byteOffset, s.body.byteOffset + s.body.length);
    else if (t === 'blob') v = new g.Blob([s.body], { type: s.mime || headerVal(s, 'content-type') || '' });
    else if (t === 'json') { try { v = JSON.parse(textOf(s)); } catch (e) { v = null; } }
    else if (t === 'document') {
      v = docOf(s, true);
    }
    s.cache = v;
    return v;
  }
  Object.defineProperties(XP, {
    readyState: { get: function () { return st(this).state; }, enumerable: true, configurable: true },
    status: { get: function () { return st(this).status; }, enumerable: true, configurable: true },
    statusText: { get: function () { return st(this).statusText; }, enumerable: true, configurable: true },
    responseURL: { get: function () { return st(this).finalUrl; }, enumerable: true, configurable: true },
    upload: { get: function () { return st(this).upload; }, enumerable: true, configurable: true },
    response: { get: function () { return responseValue(st(this)); }, enumerable: true, configurable: true },
    responseText: {
      get: function () {
        var s = st(this);
        if (!isText(s)) bad('responseText', 'InvalidStateError', "The value is only accessible if the object's 'responseType' is '' or 'text' (was '" + s.responseType + "').");
        return responseValue(s);
      }, enumerable: true, configurable: true,
    },
    responseXML: {
      get: function () {
        var s = st(this);
        if (s.responseType !== '' && s.responseType !== 'document') bad('responseXML', 'InvalidStateError', "The value is only accessible if the object's 'responseType' is '' or 'document' (was '" + s.responseType + "').");
        if (s.state !== 4) return null;
        return s.responseType === 'document' ? responseValue(s) : docOf(s, false);
      }, enumerable: true, configurable: true,
    },
    responseType: {
      get: function () { return st(this).responseType; },
      set: function (v) {
        var s = st(this);
        v = String(v);
        if (['', 'text', 'json', 'arraybuffer', 'blob', 'document'].indexOf(v) < 0) return;
        if (s.state === 3 || s.state === 4) bad('responseType', 'InvalidStateError', "The response type cannot be set if the object's state is LOADING or DONE.");
        if (!s.async && s.state === 1) bad('responseType', 'InvalidAccessError', 'The response type cannot be changed for synchronous requests made from a document.');
        s.responseType = v;
      }, enumerable: true, configurable: true,
    },
    timeout: {
      get: function () { return st(this).timeout; },
      set: function (v) {
        var s = st(this);
        if (!s.async && s.state === 1) bad('timeout', 'InvalidAccessError', 'Timeouts cannot be set for synchronous requests made from a document.');
        v = Number(v);
        s.timeout = Number.isFinite(v) && v > 0 ? Math.floor(v) : 0;
      }, enumerable: true, configurable: true,
    },
    withCredentials: {
      get: function () { return st(this).withCredentials; },
      set: function (v) {
        var s = st(this);
        if ((s.state !== 0 && s.state !== 1) || s.sent) bad('withCredentials', 'InvalidStateError', "The value may only be set if the object's state is UNSENT or OPENED.");
        s.withCredentials = !!v;
      }, enumerable: true, configurable: true,
    },
  });
  def(XP, 'getResponseHeader', function getResponseHeader(name) {
    var s = st(this);
    if (s.state < 2) return null;
    return headerVal(s, String(name).toLowerCase());
  });
  def(XP, 'getAllResponseHeaders', function getAllResponseHeaders() {
    var s = st(this), out = '';
    if (s.state < 2) return '';
    s.resHeaders.forEach(function (e) { out += e[0] + ': ' + e[1] + '\r\n'; });
    return out;
  });
  def(XP, 'overrideMimeType', function overrideMimeType(mime) {
    var s = st(this);
    if (arguments.length < 1) throw new TypeError("Failed to execute 'overrideMimeType' on 'XMLHttpRequest': 1 argument required, but only 0 present.");
    if (s.state === 3 || s.state === 4) bad('overrideMimeType', 'InvalidStateError', 'MimeType cannot be overridden when the state is LOADING or DONE.');
    s.mime = String(mime);
    s.cache = undefined;
  });
  Object.defineProperty(XMLHttpRequestEventTarget.prototype, Symbol.toStringTag, { value: 'XMLHttpRequestEventTarget', configurable: true });
  Object.defineProperty(XP, Symbol.toStringTag, { value: 'XMLHttpRequest', configurable: true });
  Object.defineProperty(XMLHttpRequestUpload.prototype, Symbol.toStringTag, { value: 'XMLHttpRequestUpload', configurable: true });
  [['XMLHttpRequestEventTarget', XMLHttpRequestEventTarget], ['XMLHttpRequestUpload', XMLHttpRequestUpload], ['XMLHttpRequest', XMLHttpRequest]].forEach(function (p) {
    Object.defineProperty(g, p[0], { value: p[1], writable: true, configurable: true, enumerable: false });
  });
})(globalThis);
