// QM1f: HTMLMediaElement over the Rust state machine (src/media_el.rs).
// Evaluated after the DOM is installed. Thin on purpose: attribute reflection,
// candidate collection, and delivery of events / play() promises that the
// natives queue (`__media_*`, src/js_media.rs). Events are always dispatched
// from a timer task, never synchronously from the call that caused them.
(function (g) {
  var MEP = g.HTMLMediaElement && g.HTMLMediaElement.prototype;
  if (!MEP || !g.__media_load) return;
  var C = g.HTMLMediaElement;
  var consts = { NETWORK_EMPTY: 0, NETWORK_IDLE: 1, NETWORK_LOADING: 2, NETWORK_NO_SOURCE: 3,
    HAVE_NOTHING: 0, HAVE_METADATA: 1, HAVE_CURRENT_DATA: 2, HAVE_FUTURE_DATA: 3, HAVE_ENOUGH_DATA: 4 };
  Object.keys(consts).forEach(function (k) {
    Object.defineProperty(C, k, { value: consts[k], enumerable: true });
    Object.defineProperty(MEP, k, { value: consts[k], enumerable: true });
  });
  var def = function (name, get, set) {
    Object.defineProperty(MEP, name, { get: get, set: set, configurable: true, enumerable: true });
  };
  // ---- MediaError, TimeRanges
  var MediaError = function MediaError() { throw new TypeError('Illegal constructor'); };
  [['MEDIA_ERR_ABORTED', 1], ['MEDIA_ERR_NETWORK', 2], ['MEDIA_ERR_DECODE', 3], ['MEDIA_ERR_SRC_NOT_SUPPORTED', 4]].forEach(function (p) {
    Object.defineProperty(MediaError, p[0], { value: p[1], enumerable: true });
    Object.defineProperty(MediaError.prototype, p[0], { value: p[1], enumerable: true });
  });
  var mkErr = function (code, msg) {
    var e = Object.create(MediaError.prototype);
    Object.defineProperty(e, 'code', { value: code, enumerable: true });
    Object.defineProperty(e, 'message', { value: msg, enumerable: true });
    return e;
  };
  g.MediaError = MediaError;
  var TimeRanges = function TimeRanges() { throw new TypeError('Illegal constructor'); };
  var mkRanges = function (flat) {
    var r = Object.create(TimeRanges.prototype);
    Object.defineProperty(r, '__r', { value: flat });
    return r;
  };
  Object.defineProperty(g, '__mk_ranges', { value: mkRanges, configurable: true });
  var chk = function (r, i) {
    i = i >>> 0;
    if (i * 2 >= r.__r.length) throw new DOMException('The index provided (' + i + ') is greater than or equal to the maximum bound (' + (r.__r.length / 2) + ').', 'IndexSizeError');
    return i * 2;
  };
  Object.defineProperty(TimeRanges.prototype, 'length', { get: function () { return this.__r.length / 2; }, configurable: true, enumerable: true });
  TimeRanges.prototype.start = function (i) { return this.__r[chk(this, i)]; };
  TimeRanges.prototype.end = function (i) { return this.__r[chk(this, i) + 1]; };
  g.TimeRanges = TimeRanges;

  // ---- per-element bookkeeping
  var S = {}, els = {}, tokN = 0, timer = 0;
  var st = function (el) {
    var id = el.__id, s = S[id];
    if (!s) {
      observe();
      s = S[id] = { key: null, proms: {} };
      els[id] = el;
      if (el.hasAttribute('muted')) g.__media_setflag(id, 'muted0', true);
      sync(el);
    }
    return s;
  };
  var sync = function (el) {
    g.__media_setflag(el.__id, 'loop', el.hasAttribute('loop'));
    g.__media_setflag(el.__id, 'autoplay', el.hasAttribute('autoplay'));
  };
  var abs = function (v) {
    try { return new URL(String(v).trim(), document.baseURI).href; } catch (e) { return String(v); }
  };
  var candidates = function (el) {
    var src = el.getAttribute('src');
    if (src !== null) return { attr: true, list: src.trim() === '' ? [] : [abs(src), ''] };
    var list = [];
    for (var c = el.firstElementChild; c; c = c.nextElementSibling) {
      if (c.localName !== 'source') continue;
      var s = c.getAttribute('src');
      if (s === null || s.trim() === '') continue;
      list.push(abs(s), c.getAttribute('type') || '');
    }
    return { attr: false, list: list };
  };
  var doLoad = function (el, force) {
    var s = st(el), c = candidates(el);
    var key = (c.attr ? 'a' : 's') + JSON.stringify(c.list);
    if (!force && s.key === key) return;
    s.key = key;
    sync(el);
    // QM1k1d: a MediaSource (srcObject, or a blob: URL made by createObjectURL(ms)) attaches instead
    if (g.__mse_try && g.__mse_try(el, c.attr ? c.list : [])) { kick(0); return; }
    g.__media_load(el.__id, c.list, c.attr, el.getAttribute('crossorigin') === null ? '' : (el.getAttribute('crossorigin') === 'use-credentials' ? 'use-credentials' : 'anonymous'), String(g.location.href));
    kick(0);
  };
  var ensure = function (el) { var s = st(el); var c = candidates(el); if (s.key !== (c.attr ? 'a' : 's') + JSON.stringify(c.list)) doLoad(el); };
  var schedule = function (el) { setTimeout(function () { doLoad(el); }, 0); };

  // ---- event / promise delivery
  var fire = function (el, name, src) {
    try { el.dispatchEvent(new Event(name)); } catch (e) { g.__scratchy_err('Uncaught ' + String(e)); }
  };
  var kick = function (delay) {
    if (timer) return;
    timer = setTimeout(run, delay);
  };
  var run = function () {
    timer = 0;
    var out = g.__media_pump(g.__scratchy_now(), g.__media_wait || 0), busy = false;
    for (var i = 0; i < out.length; i++) {
      var p = out[i].split(' '), id = +p[0], el = els[id];
      if (p[1] === 'b') { busy = p[2] === '1'; continue; }
      if (p[1] === 'o' || p[1] === 'n' || p[1] === 'c') { if (g.__mse_event) g.__mse_event(p[1], +p[2]); continue; }
      var s = S[id];
      if (!el || !s) continue;
      if (p[1] === 'e') fire(el, p[2]);
      else if (p[1] === 's') {
        var kids = [];
        for (var c = el.firstElementChild; c; c = c.nextElementSibling) {
          if (c.localName === 'source' && c.getAttribute('src') !== null && c.getAttribute('src').trim() !== '') kids.push(c);
        }
        if (kids[+p[2]]) fire(kids[+p[2]], 'error');
      } else if (p[1] === 'r' || p[1] === 'j') {
        var pr = s.proms[p[2]];
        if (pr) {
          delete s.proms[p[2]];
          if (p[1] === 'r') pr.res(); else pr.rej(new DOMException(
            p[3] === 'NotAllowedError' ? 'play() failed because the user didn\'t interact with the document first (autoplay of unmuted media is not allowed).'
              : p[3] === 'AbortError' ? 'The play() request was interrupted.' : 'The element has no supported sources.', p[3]));
        }
      }
    }
    if (busy) kick(30);
  };

  // ---- API
  var num = function (el, k) { st(el); return g.__media_num(el.__id, k); };
  def('currentSrc', function () { st(this); return g.__media_text(this.__id, 1); });
  def('src', function () { var v = this.getAttribute('src'); return v === null ? '' : abs(v); },
    function (v) { this.setAttribute('src', String(v)); schedule(this); });
  def('networkState', function () { return num(this, 0); });
  def('readyState', function () { return num(this, 1); });
  def('paused', function () { return num(this, 2) === 1; });
  def('seeking', function () { return num(this, 3) === 1; });
  def('ended', function () { return num(this, 8) === 1; });
  def('currentTime', function () { return num(this, 4); },
    function (v) { v = Number(v); if (v !== v) throw new TypeError('The provided double value is non-finite.'); ensure(this); g.__media_seek(this.__id, v); kick(0); });
  def('duration', function () { return num(this, 5); });
  def('volume', function () { return num(this, 6); }, function (v) {
    ensure(this);
    var r = g.__media_setvol(this.__id, Number(v));
    kick(0);
    if (r) throw new DOMException('The volume provided (' + v + ') is outside the range [0, 1].', r);
  });
  def('muted', function () { return num(this, 13) === 1; }, function (v) { ensure(this); g.__media_setmuted(this.__id, !!v); kick(0); });
  def('defaultMuted', function () { return this.hasAttribute('muted'); },
    function (v) { if (v) this.setAttribute('muted', ''); else this.removeAttribute('muted'); });
  def('playbackRate', function () { return num(this, 7); }, function (v) {
    ensure(this);
    var r = g.__media_setrate(this.__id, Number(v));
    kick(0);
    if (r) throw new DOMException('The provided playback rate (' + v + ') is not supported.', r);
  });
  def('defaultPlaybackRate', function () { return num(this, 12); }, function (v) { st(this); g.__media_setdefrate(this.__id, Number(v)); kick(0); });
  def('loop', function () { return this.hasAttribute('loop'); },
    function (v) { if (v) this.setAttribute('loop', ''); else this.removeAttribute('loop'); sync(this); });
  def('autoplay', function () { return this.hasAttribute('autoplay'); },
    function (v) { if (v) this.setAttribute('autoplay', ''); else this.removeAttribute('autoplay'); sync(this); });
  def('error', function () { var c = num(this, 11); return c ? mkErr(c, g.__media_text(this.__id, 0)) : null; });
  def('buffered', function () { st(this); return mkRanges(g.__media_ranges(this.__id, 0)); });
  def('seekable', function () { st(this); return mkRanges(g.__media_ranges(this.__id, 1)); });
  def('played', function () { return mkRanges([]); });
  def('srcObject', function () { return null; }, function () {});
  // QM1k1d: what mse_prelude.js needs (the element layer's kick and a forced reload task)
  Object.defineProperty(g, '__media_kick', { value: function () { kick(0); }, configurable: true });
  Object.defineProperty(g, '__media_reload', { value: function (el) { setTimeout(function () { doLoad(el, true); }, 0); }, configurable: true });
  MEP.load = function () { doLoad(this, true); };
  MEP.play = function () {
    var el = this, s = st(el);
    ensure(el);
    sync(el);
    return new Promise(function (res, rej) {
      var t = ++tokN;
      s.proms[t] = { res: res, rej: rej };
      g.__media_play(el.__id, t);
      kick(0);
    });
  };
  MEP.pause = function () { ensure(this); g.__media_pause(this.__id); kick(0); };
  MEP.canPlayType = function (t) { return g.__media_can_play(String(t)); };
  MEP.fastSeek = function (t) { this.currentTime = t; };
  var VP = g.HTMLVideoElement.prototype;
  Object.defineProperty(VP, 'videoWidth', { get: function () { return num(this, 9); }, configurable: true, enumerable: true });
  Object.defineProperty(VP, 'videoHeight', { get: function () { return num(this, 10); }, configurable: true, enumerable: true });

  g.Audio = function Audio(src) {
    var a = document.createElement('audio');
    a.setAttribute('preload', 'auto');
    if (src !== undefined) { a.setAttribute('src', String(src)); schedule(a); }
    return a;
  };
  g.Audio.prototype = g.HTMLAudioElement.prototype;

  // ---- document hooks: loads and removal follow the DOM
  var isMedia = function (n) { return n && n.nodeType === 1 && (n.localName === 'video' || n.localName === 'audio'); };
  var each = function (n, f) {
    if (!n || n.nodeType !== 1) return;
    if (isMedia(n)) f(n);
    else if (n.querySelectorAll) { var l = n.querySelectorAll('video,audio'); for (var i = 0; i < l.length; i++) f(l[i]); }
  };
  var touched = function (el) {
    var c = candidates(el);
    if (c.list.length || S[el.__id]) schedule(el);
  };
  // The document observer is installed lazily (a page without media pays nothing,
  // QY6a/QY6e zero-cost rule): on the first media element seen, created or touched.
  var mo = null;
  var observe = function () {
    if (mo || !g.MutationObserver) return;
    mo = new g.MutationObserver(function (recs) {
      recs.forEach(function (r) {
        if (r.type === 'attributes') {
          var t = r.target;
          if (isMedia(t)) {
            if (r.attributeName === 'src') schedule(t); else if (S[t.__id]) sync(t);
          } else if (t.localName === 'source' && isMedia(t.parentNode) && t.parentNode.getAttribute('src') === null) schedule(t.parentNode);
        } else {
          for (var i = 0; i < r.addedNodes.length; i++) {
            var n = r.addedNodes[i];
            each(n, touched);
            if (n.nodeType === 1 && n.localName === 'source' && isMedia(r.target) && r.target.getAttribute('src') === null) schedule(r.target);
          }
          for (var j = 0; j < r.removedNodes.length; j++) {
            each(r.removedNodes[j], function (el) {
              setTimeout(function () { if (!el.isConnected && S[el.__id] && !el.paused) { g.__media_pause(el.__id); kick(0); } }, 0);
            });
          }
        }
      });
    });
    mo.observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['src', 'loop', 'autoplay'] });
  };
  var ce = document.createElement;
  document.createElement = function (n) {
    var l = String(n).toLowerCase();
    if (l === 'video' || l === 'audio') observe();
    return ce.apply(this, arguments);
  };
  // QV8-4: the initial sweep over the page's elements is a function the V8 startup snapshot's
  // page engines call again once their real document is attached.
  var sweep = function () {
    var first = document.querySelectorAll('video,audio');
    if (first.length) observe();
    for (var i = 0; i < first.length; i++) touched(first[i]);
  };
  sweep();
  Object.defineProperty(globalThis, '__media_sweep', { value: sweep, configurable: true });
})(globalThis);
