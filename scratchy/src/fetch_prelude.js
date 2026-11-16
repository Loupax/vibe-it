// QF2: Blob, File, FileReader and the blob: URL store. Plain in-memory data
// classes (no file picker, no I/O, no network). Evaluated after the globals
// and intl preludes (see js.rs). If QF1 (TextEncoder) lands, its classes are
// separate; the UTF-8 helpers below are private so this file needs nothing.
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
    if (!isBlob(obj)) throw new TypeError("Failed to execute 'createObjectURL' on 'URL': Overload resolution failed.");
    var o = g.location && g.location.origin;
    var u = 'blob:' + (o && o !== 'null' ? o : 'null') + '/' + uuid();
    store.set(u, obj);
    return u;
  };
  g.URL.revokeObjectURL = function revokeObjectURL(u) { store.delete(key(u)); };
  def(g, '__blob_lookup', function (u) { return store.get(key(u)); });

  def(g, 'Blob', Blob); def(g, 'File', File); def(g, 'FileReader', FileReader);
})(globalThis);
