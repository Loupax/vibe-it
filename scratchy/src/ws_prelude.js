// QF19w2: the page `WebSocket`, a JS class over the natives of js_ws.rs
// (`__ws_open/send_text/send_bin/close/abort`). The engine calls `__ws_ev` and
// `__ws_ev_bin` for every event, one task each, in order. The framing and the
// connection run in Rust over wire's unix socket.
(function (g) {
  'use strict';
  var N = { open: g.__ws_open, text: g.__ws_send_text, bin: g.__ws_send_bin, close: g.__ws_close, abort: g.__ws_abort };
  ['__ws_open', '__ws_send_text', '__ws_send_bin', '__ws_close', '__ws_abort'].forEach(function (k) { delete g[k]; });
  function domErr(name, msg) {
    if (typeof g.DOMException === 'function') return new g.DOMException(msg, name);
    var e = new Error(msg); e.name = name; return e;
  }
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  var map = new WeakMap(), socks = {};
  function st(x) { var s = x !== null && typeof x === 'object' ? map.get(x) : undefined; if (!s) throw new TypeError('Illegal invocation'); return s; }
  function utf8len(s) {
    var n = 0;
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x80) n += 1;
      else if (c < 0x800) n += 2;
      else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xFC00) === 0xDC00) { n += 4; i++; }
      else n += 3;
    }
    return n;
  }
  var TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  function WebSocket(url, protocols) {
    if (!new.target) throw new TypeError("Failed to construct 'WebSocket': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length < 1) throw new TypeError("Failed to construct 'WebSocket': 1 argument required, but only 0 present.");
    var bad = function (m) { return domErr('SyntaxError', "Failed to construct 'WebSocket': " + m); };
    var u;
    try { u = new g.URL(String(url), g.location.href); } catch (e) { throw bad("The URL '" + url + "' is invalid."); }
    var sch = u.protocol, href = u.href;
    if (sch === 'http:') href = 'ws:' + href.slice(5);
    else if (sch === 'https:') href = 'wss:' + href.slice(6);
    else if (sch !== 'ws:' && sch !== 'wss:') throw bad("The URL's scheme must be either 'http', 'https', 'ws', or 'wss'. '" + sch.slice(0, -1) + "' is not allowed.");
    if (href.indexOf('#') >= 0) throw bad("The URL contains a fragment identifier ('" + href.slice(href.indexOf('#')) + "'). Fragment identifiers are not allowed in WebSocket URLs.");
    var list = [];
    if (protocols !== undefined) {
      list = typeof protocols === 'string' ? [protocols] : Array.prototype.map.call(Array.from(protocols), String);
    }
    for (var i = 0; i < list.length; i++) {
      if (!TOKEN.test(list[i])) throw bad("The subprotocol '" + list[i] + "' is invalid.");
      if (list.indexOf(list[i]) !== i) throw bad("The subprotocol '" + list[i] + "' is duplicated.");
    }
    var ws = Reflect.construct(g.EventTarget, [], new.target);
    var uu = new g.URL(href);
    var s = { rs: 0, url: uu.href, origin: uu.protocol + '//' + uu.host, protocol: '', buffered: 0, bt: 'blob', h: {}, id: 0 };
    map.set(ws, s);
    s.id = N.open(uu.href, list);
    socks[s.id] = ws;
    return ws;
  }
  WebSocket.prototype = Object.create(g.EventTarget.prototype, { constructor: { value: WebSocket, writable: true, configurable: true } });
  Object.setPrototypeOf(WebSocket, g.EventTarget);
  var P = WebSocket.prototype;
  [['CONNECTING', 0], ['OPEN', 1], ['CLOSING', 2], ['CLOSED', 3]].forEach(function (p) {
    Object.defineProperty(WebSocket, p[0], { value: p[1], enumerable: true });
    Object.defineProperty(P, p[0], { value: p[1], enumerable: true });
  });
  // The dispatcher reads `node['on' + type]`; these accessors answer it.
  ['open', 'message', 'error', 'close'].forEach(function (t) {
    Object.defineProperty(P, 'on' + t, {
      get: function () { var h = st(this).h[t]; return h === undefined ? null : h; },
      set: function (f) { st(this).h[t] = typeof f === 'function' ? f : null; },
      enumerable: true, configurable: true,
    });
  });
  Object.defineProperties(P, {
    readyState: { get: function () { return st(this).rs; }, enumerable: true, configurable: true },
    url: { get: function () { return st(this).url; }, enumerable: true, configurable: true },
    protocol: { get: function () { return st(this).protocol; }, enumerable: true, configurable: true },
    extensions: { get: function () { st(this); return ''; }, enumerable: true, configurable: true },
    bufferedAmount: { get: function () { return st(this).buffered; }, enumerable: true, configurable: true },
    binaryType: {
      get: function () { return st(this).bt; },
      set: function (v) { v = String(v); if (v === 'blob' || v === 'arraybuffer') st(this).bt = v; },
      enumerable: true, configurable: true,
    },
  });
  def(P, 'send', function send(data) {
    var s = st(this);
    if (arguments.length < 1) throw new TypeError("Failed to execute 'send' on 'WebSocket': 1 argument required, but only 0 present.");
    if (s.rs === 0) throw domErr('InvalidStateError', "Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.");
    var bytes = null, text = null, n;
    var blob = data !== null && typeof data === 'object' && typeof g.__blob_bytes === 'function' ? g.__blob_bytes(data) : undefined;
    if (blob) bytes = blob;
    else if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
    else if (ArrayBuffer.isView(data)) bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    else text = String(data);
    n = text !== null ? utf8len(text) : bytes.length;
    s.buffered += n;
    if (s.rs !== 1) return;
    if (text !== null) N.text(s.id, text); else N.bin(s.id, bytes);
  });
  def(P, 'close', function close(code, reason) {
    var s = st(this);
    if (code !== undefined) {
      code = Number(code) % 65536;
      if (code !== 1000 && !(code >= 3000 && code <= 4999)) throw domErr('InvalidAccessError', "Failed to execute 'close' on 'WebSocket': The code must be either 1000, or between 3000 and 4999. " + code + " is neither.");
    }
    if (reason !== undefined) {
      reason = String(reason);
      if (utf8len(reason) > 123) throw domErr('SyntaxError', "Failed to execute 'close' on 'WebSocket': The message must not be greater than 123 bytes.");
    }
    if (s.rs === 2 || s.rs === 3) return;
    if (s.rs === 0) { s.rs = 2; N.abort(s.id); return; }
    s.rs = 2;
    N.close(s.id, code === undefined ? -1 : code, code === undefined || reason === undefined ? '' : reason);
  });
  Object.defineProperty(P, Symbol.toStringTag, { value: 'WebSocket', configurable: true });
  function fire(ws, ev) { ws.dispatchEvent(ev); }
  function message(ws, s, data) {
    if (s.rs !== 1) return;
    fire(ws, new g.MessageEvent('message', { data: data, origin: s.origin, lastEventId: '', source: null, ports: [] }));
  }
  def(g, '__ws_ev', function (id, kind, str, n, flag) {
    var ws = socks[id];
    if (!ws) return;
    var s = map.get(ws);
    switch (kind) {
      case 'open':
        if (s.rs !== 0) return;
        s.rs = 1; s.protocol = str;
        fire(ws, new g.Event('open'));
        break;
      case 'text': message(ws, s, str); break;
      case 'error':
        if (s.rs !== 3) fire(ws, new g.Event('error'));
        break;
      case 'close':
        delete socks[id];
        if (s.rs === 3) return;
        s.rs = 3;
        fire(ws, new g.CloseEvent('close', { wasClean: !!flag, code: n, reason: str }));
        break;
      case 'flushed': s.buffered = Math.max(0, s.buffered - n); break;
    }
  });
  def(g, '__ws_ev_bin', function (id, u8) {
    var ws = socks[id];
    if (!ws) return;
    var s = map.get(ws);
    message(ws, s, s.bt === 'arraybuffer' ? u8.buffer : new g.Blob([u8]));
  });
  def(g, 'WebSocket', WebSocket);
})(globalThis);
