// QM1k1d: MediaSource, SourceBufferList and the attachment to a media element (the QM1k-PLAN "JS"
// paragraph). Evaluated after media_prelude.js. The state machine of the attached element lives in
// src/media_el.rs; this file keeps the JS-visible object model (readyState, lists, errors) and
// routes the events the natives queue ('o' sourceopen, 'n' sourceended, 'c' sourceclose, see
// media_prelude.js `run`). Nothing is created until a page does `new MediaSource()`; the Rust
// feed is made when the MediaSource is attached. Every event is dispatched from a timer task.
// SourceBuffer is a shell here: appendBuffer/remove/abort come with QM1k1e.
(function (g) {
  if (!g.__mse_new || !g.__media_kick || !g.EventTarget) return;
  var ET = g.EventTarget;
  var ms_ = new WeakMap();      // MediaSource -> state
  var sb_ = new WeakMap();      // SourceBuffer -> state
  var list_ = new WeakMap();    // SourceBufferList -> its array
  var srcObj = new WeakMap();   // media element -> MediaSource set as srcObject
  var byFeed = {};
  var MAX_BUFFERS = 2;

  var illegal = function (n) { return function () { throw new TypeError('Illegal constructor'); }; };
  var report = function (e) { g.__scratchy_err('Uncaught ' + String(e)); };
  var fireNow = function (t, type) {
    try { t.dispatchEvent(new Event(type)); } catch (e) { report(e); }
  };
  var fireTask = function (t, type) { setTimeout(function () { fireNow(t, type); }, 0); };
  var handlers = function (P, names) {
    var store = new WeakMap();
    names.forEach(function (t) {
      Object.defineProperty(P, 'on' + t, {
        get: function () { var h = store.get(this); h = h && h[t]; return h === undefined ? null : h; },
        set: function (f) { var h = store.get(this); if (!h) store.set(this, h = {}); h[t] = typeof f === 'function' ? f : null; },
        enumerable: true, configurable: true,
      });
    });
  };
  var subclass = function (C, P) {
    C.prototype = Object.create(ET.prototype, { constructor: { value: C, writable: true, configurable: true } });
    Object.setPrototypeOf(C, ET);
    Object.defineProperty(C.prototype, Symbol.toStringTag, { value: C.name, configurable: true });
  };
  var dom = function (msg, name) { return new DOMException(msg, name); };

  // ---- SourceBuffer (shell) and SourceBufferList
  var SourceBuffer = function SourceBuffer() { throw new TypeError('Illegal constructor'); };
  subclass(SourceBuffer);
  handlers(SourceBuffer.prototype, ['updatestart', 'update', 'updateend', 'error', 'abort']);
  var sbOf = function (o) {
    var s = o !== null && typeof o === 'object' ? sb_.get(o) : undefined;
    if (!s) throw new TypeError('Illegal invocation');
    return s;
  };
  Object.defineProperty(SourceBuffer.prototype, 'updating', { get: function () { return sbOf(this).updating; }, enumerable: true, configurable: true });
  ['appendBuffer', 'abort', 'remove'].forEach(function (m) {
    SourceBuffer.prototype[m] = function () {
      sbOf(this);
      g.__scratchy_err('scratchy: js: not implemented: SourceBuffer.' + m);
      throw dom('SourceBuffer.' + m + ' is not implemented yet.', 'NotSupportedError');
    };
  });
  var mkSB = function (ms, st, type, bi) {
    var sb = Object.create(SourceBuffer.prototype);
    sb_.set(sb, { ms: ms, st: st, type: type, bi: bi, updating: false, removed: false });
    return sb;
  };

  var SourceBufferList = function SourceBufferList() { throw new TypeError('Illegal constructor'); };
  subclass(SourceBufferList);
  handlers(SourceBufferList.prototype, ['addsourcebuffer', 'removesourcebuffer']);
  var arr = function (o) {
    var a = o !== null && typeof o === 'object' ? list_.get(o) : undefined;
    if (!a) throw new TypeError('Illegal invocation');
    return a;
  };
  Object.defineProperty(SourceBufferList.prototype, 'length', { get: function () { return arr(this).length; }, enumerable: true, configurable: true });
  SourceBufferList.prototype.item = function (i) { var a = arr(this); i = i >>> 0; return i < a.length ? a[i] : null; };
  SourceBufferList.prototype[Symbol.iterator] = function () { return arr(this).slice()[Symbol.iterator](); };
  var mkList = function () {
    var l = Object.create(SourceBufferList.prototype);
    list_.set(l, []);
    return l;
  };
  var syncIdx = function (l) {
    var a = list_.get(l), n = a.length;
    for (var i = 0; i < n; i++) Object.defineProperty(l, i, { value: a[i], enumerable: true, configurable: true });
    for (var j = n; l.hasOwnProperty(j); j++) delete l[j];
  };
  var listAdd = function (l, sb) { list_.get(l).push(sb); syncIdx(l); };
  var listDel = function (l, sb) {
    var a = list_.get(l), i = a.indexOf(sb);
    if (i >= 0) a.splice(i, 1);
    var n = a.length;
    syncIdx(l);
    delete l[n];
  };

  // ---- MediaSource
  var MediaSource = function MediaSource() {
    if (!new.target) throw new TypeError("Failed to construct 'MediaSource': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var ms = Reflect.construct(ET, [], new.target);
    ms_.set(ms, { rs: 'closed', feed: 0, dur: NaN, el: null, buffers: [], list: null, active: null, lsr: null });
    return ms;
  };
  subclass(MediaSource);
  handlers(MediaSource.prototype, ['sourceopen', 'sourceended', 'sourceclose']);
  var msOf = function (o) {
    var s = o !== null && typeof o === 'object' ? ms_.get(o) : undefined;
    if (!s) throw new TypeError('Illegal invocation');
    return s;
  };
  var lists = function (st) {
    if (!st.list) { st.list = mkList(); st.active = mkList(); }
    return st;
  };
  var P = MediaSource.prototype;
  var getter = function (name, f) { Object.defineProperty(P, name, { get: f, enumerable: true, configurable: true }); };
  getter('readyState', function () { return msOf(this).rs; });
  getter('sourceBuffers', function () { return lists(msOf(this)).list; });
  getter('activeSourceBuffers', function () { return lists(msOf(this)).active; });
  Object.defineProperty(P, 'duration', {
    get: function () { var s = msOf(this); return s.rs === 'closed' ? NaN : s.dur; },
    set: function (v) {
      var s = msOf(this);
      v = Number(v);
      if (v !== v) throw new TypeError("Failed to set the 'duration' property on 'MediaSource': The value provided is non-finite.");
      if (v < 0) throw new TypeError("Failed to set the 'duration' property on 'MediaSource': The value provided (" + v + ') is negative.');
      if (s.rs !== 'open') throw dom("Failed to set the 'duration' property on 'MediaSource': The MediaSource's readyState is not 'open'.", 'InvalidStateError');
      if (s.buffers.some(function (b) { return sb_.get(b).updating; })) throw dom("Failed to set the 'duration' property on 'MediaSource': There are SourceBuffers whose 'updating' attribute is true.", 'InvalidStateError');
      if (v < g.__mse_buffered_end(s.feed)) throw dom("Failed to set the 'duration' property on 'MediaSource': Setting duration to " + v + ' would truncate buffered media; remove() the media first.', 'InvalidStateError');
      s.dur = v;
      g.__mse_set_duration(s.feed, v);
      g.__media_kick();
    },
    enumerable: true, configurable: true,
  });
  P.addSourceBuffer = function addSourceBuffer(type) {
    var s = msOf(this);
    if (arguments.length < 1) throw new TypeError("Failed to execute 'addSourceBuffer' on 'MediaSource': 1 argument required, but only 0 present.");
    type = String(type);
    if (type === '') throw new TypeError("Failed to execute 'addSourceBuffer' on 'MediaSource': The type provided ('') is empty.");
    if (!g.__mse_supported(type)) throw dom("Failed to execute 'addSourceBuffer' on 'MediaSource': The type provided ('" + type + "') is unsupported.", 'NotSupportedError');
    if (s.buffers.length >= MAX_BUFFERS) throw dom("Failed to execute 'addSourceBuffer' on 'MediaSource': This MediaSource has reached the limit of SourceBuffer objects it can handle. No additional SourceBuffer objects may be added.", 'QuotaExceededError');
    if (s.rs !== 'open') throw dom("Failed to execute 'addSourceBuffer' on 'MediaSource': The MediaSource's readyState is not 'open'.", 'InvalidStateError');
    var bi = g.__mse_add_buffer(s.feed, type);
    if (bi < 0) throw dom("Failed to execute 'addSourceBuffer' on 'MediaSource': The MediaSource is closed.", 'InvalidStateError');
    var sb = mkSB(this, s, type, bi);
    lists(s);
    s.buffers.push(sb);
    listAdd(s.list, sb);
    listAdd(s.active, sb);
    fireTask(s.list, 'addsourcebuffer');
    return sb;
  };
  P.removeSourceBuffer = function removeSourceBuffer(sb) {
    var s = msOf(this);
    if (!(sb !== null && typeof sb === 'object' && sb_.has(sb))) throw new TypeError("Failed to execute 'removeSourceBuffer' on 'MediaSource': parameter 1 is not of type 'SourceBuffer'.");
    var i = s.buffers.indexOf(sb);
    if (i < 0) throw dom("Failed to execute 'removeSourceBuffer' on 'MediaSource': The SourceBuffer provided is not contained in this MediaSource.", 'NotFoundError');
    s.buffers.splice(i, 1);
    sb_.get(sb).removed = true;
    for (var j = i; j < s.buffers.length; j++) sb_.get(s.buffers[j]).bi--;
    g.__mse_remove_buffer(s.feed, i, g.__scratchy_now());
    listDel(s.active, sb);
    listDel(s.list, sb);
    fireTask(s.active, 'removesourcebuffer');
    fireTask(s.list, 'removesourcebuffer');
    g.__media_kick();
  };
  P.endOfStream = function endOfStream(error) {
    var s = msOf(this), kind = 0;
    if (error !== undefined) {
      error = String(error);
      if (error === 'network') kind = 1;
      else if (error === 'decode') kind = 2;
      else if (error !== '') throw new TypeError("Failed to execute 'endOfStream' on 'MediaSource': The provided value '" + error + "' is not a valid enum value of type EndOfStreamError.");
    }
    if (s.rs !== 'open') throw dom("Failed to execute 'endOfStream' on 'MediaSource': The MediaSource's readyState is not 'open'.", 'InvalidStateError');
    if (s.buffers.some(function (b) { return sb_.get(b).updating; })) throw dom("Failed to execute 'endOfStream' on 'MediaSource': One or more SourceBuffers are still updating.", 'InvalidStateError');
    s.rs = 'ended';
    var d = g.__mse_eos(s.feed, kind);
    if (kind === 0 && d === d) s.dur = d;
    g.__media_kick();
  };
  P.setLiveSeekableRange = function setLiveSeekableRange(start, end) {
    var s = msOf(this);
    if (arguments.length < 2) throw new TypeError("Failed to execute 'setLiveSeekableRange' on 'MediaSource': 2 arguments required, but only " + arguments.length + ' present.');
    if (s.rs !== 'open') throw dom("Failed to execute 'setLiveSeekableRange' on 'MediaSource': The MediaSource's readyState is not 'open'.", 'InvalidStateError');
    start = Number(start); end = Number(end);
    if (!(start >= 0) || start > end || end !== end) throw new TypeError("Failed to execute 'setLiveSeekableRange' on 'MediaSource': The start value provided is invalid (negative, NaN or greater than the end).");
    s.lsr = [start, end];
  };
  P.clearLiveSeekableRange = function clearLiveSeekableRange() {
    var s = msOf(this);
    if (s.rs !== 'open') throw dom("Failed to execute 'clearLiveSeekableRange' on 'MediaSource': The MediaSource's readyState is not 'open'.", 'InvalidStateError');
    s.lsr = null;
  };
  MediaSource.isTypeSupported = function isTypeSupported(t) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'isTypeSupported' on 'MediaSource': 1 argument required, but only 0 present.");
    return g.__mse_supported(String(t));
  };
  Object.defineProperty(MediaSource, 'canConstructInDedicatedWorker', { value: false, enumerable: true, configurable: true });
  g.MediaSource = MediaSource;
  g.SourceBuffer = SourceBuffer;
  g.SourceBufferList = SourceBufferList;

  // ---- attachment (called by media_prelude.js doLoad, and by the events the natives queue)
  Object.defineProperty(g, '__mse_is', { value: function (o) { return o !== null && typeof o === 'object' && ms_.has(o); }, configurable: true });
  Object.defineProperty(g, '__mse_try', {
    value: function (el, list) {
      var ms = srcObj.get(el) || null;
      if (!ms && list.length === 2 && list[0].slice(0, 5) === 'blob:') {
        var b = g.__blob_lookup(list[0]);
        if (b && ms_.has(b)) ms = b;
        else if (b === undefined) { g.__mse_fail(el.__id, String(g.location.href)); return true; }   // revoked / unknown
      }
      if (!ms) return false;
      var s = ms_.get(ms);
      if (s.rs !== 'closed' || s.feed) return false;   // already attached: the load fails like any bad src
      s.feed = g.__mse_new();
      s.el = el;
      byFeed[s.feed] = ms;
      g.__mse_attach(el.__id, s.feed, s.dur, String(g.location.href));
      return true;
    },
    configurable: true,
  });
  Object.defineProperty(g, '__mse_event', {
    value: function (kind, feed) {
      var ms = byFeed[feed];
      if (!ms) return;
      var s = ms_.get(ms);
      if (kind === 'o') {
        s.rs = 'open';
        fireNow(ms, 'sourceopen');
      } else if (kind === 'n') {
        s.rs = 'ended';
        fireNow(ms, 'sourceended');
      } else {
        // detached: the SourceBuffers go, the duration is NaN again
        delete byFeed[feed];
        s.rs = 'closed';
        s.feed = 0;
        s.el = null;
        s.dur = NaN;
        s.lsr = null;
        var gone = s.buffers;
        s.buffers = [];
        gone.forEach(function (b) { sb_.get(b).removed = true; });
        if (s.list && gone.length) {
          gone.forEach(function (b) { listDel(s.active, b); listDel(s.list, b); });
          fireNow(s.active, 'removesourcebuffer');
          fireNow(s.list, 'removesourcebuffer');
        }
        fireNow(ms, 'sourceclose');
      }
    },
    configurable: true,
  });
  var MEP = g.HTMLMediaElement && g.HTMLMediaElement.prototype;
  if (MEP) {
    Object.defineProperty(MEP, 'srcObject', {
      get: function () { var o = srcObj.get(this); return o === undefined ? null : o; },
      set: function (v) {
        if (v === null || v === undefined) {
          if (srcObj.has(this)) { srcObj.delete(this); g.__media_reload(this); }
          return;
        }
        if (!ms_.has(v)) return;   // MediaStream and friends: not supported, as before
        srcObj.set(this, v);
        g.__media_reload(this);
      },
      configurable: true, enumerable: true,
    });
  }
})(globalThis);
