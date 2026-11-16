(function (g) {
  var parse = g.__url_parse, set = g.__url_set;
  delete g.__url_parse; delete g.__url_set;
  function def(name, v) { Object.defineProperty(g, name, { value: v, writable: true, configurable: true }); }
  function enc(s) {
    var out = '', b = unescape(encodeURIComponent(s));
    for (var i = 0; i < b.length; i++) {
      var c = b.charCodeAt(i), ch = b.charAt(i);
      if ((c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || ch === '*' || ch === '-' || ch === '.' || ch === '_') out += ch;
      else if (c === 32) out += '+';
      else out += '%' + (c < 16 ? '0' : '') + c.toString(16).toUpperCase();
    }
    return out;
  }
  function dec(s) {
    s = s.replace(/\+/g, ' ');
    try { return decodeURIComponent(s); } catch (e) {
      return s.replace(/(%[0-9a-fA-F]{2})+/g, function (m) { try { return decodeURIComponent(m); } catch (x) { return m; } });
    }
  }
  function USVString(v) {
    var s = String(v), out = '';
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xFC00) === 0xDC00) { out += s.charAt(i) + s.charAt(++i); }
      else if (c >= 0xD800 && c <= 0xDFFF) out += '�';
      else out += s.charAt(i);
    }
    return out;
  }
  function URLSearchParams(init) {
    if (!(this instanceof URLSearchParams)) throw new TypeError("Failed to construct 'URLSearchParams': Please use the 'new' operator");
    Object.defineProperty(this, '_l', { value: [], writable: true });
    Object.defineProperty(this, '_u', { value: null, writable: true });
    if (init === undefined || init === null) return;
    if (typeof init === 'object' || typeof init === 'function') {
      if (typeof init[Symbol.iterator] === 'function') {
        for (var p of init) {
          var pair = Array.from(p);
          if (pair.length !== 2) throw new TypeError("Failed to construct 'URLSearchParams': Each query pair must be an iterable [name, value] tuple");
          this._l.push([USVString(pair[0]), USVString(pair[1])]);
        }
      } else {
        var ks = Object.keys(init);
        for (var i = 0; i < ks.length; i++) this._l.push([USVString(ks[i]), USVString(init[ks[i]])]);
      }
    } else this._parse(String(init));
  }
  var UP = URLSearchParams.prototype;
  Object.defineProperty(UP, '_parse', { value: function (s) {
    this._l = [];
    if (s.charAt(0) === '?') s = s.slice(1);
    if (s === '') return;
    var parts = s.split('&');
    for (var i = 0; i < parts.length; i++) {
      if (parts[i] === '') continue;
      var k = parts[i].indexOf('=');
      this._l.push(k < 0 ? [dec(parts[i]), ''] : [dec(parts[i].slice(0, k)), dec(parts[i].slice(k + 1))]);
    }
  } });
  Object.defineProperty(UP, '_upd', { value: function () {
    if (this._u) this._u._setQuery(this._l.length ? this.toString() : '');
  } });
  function need(n, a, name) { if (a.length < n) throw new TypeError("Failed to execute '" + name + "' on 'URLSearchParams': " + n + " arguments required, but only " + a.length + " present."); }
  UP.append = function (n, v) { need(2, arguments, 'append'); this._l.push([USVString(n), USVString(v)]); this._upd(); };
  UP['delete'] = function (n, v) {
    need(1, arguments, 'delete'); n = USVString(n);
    var hv = v !== undefined; if (hv) v = USVString(v);
    this._l = this._l.filter(function (p) { return !(p[0] === n && (!hv || p[1] === v)); });
    this._upd();
  };
  UP.get = function (n) { need(1, arguments, 'get'); n = USVString(n); for (var i = 0; i < this._l.length; i++) if (this._l[i][0] === n) return this._l[i][1]; return null; };
  UP.getAll = function (n) { need(1, arguments, 'getAll'); n = USVString(n); return this._l.filter(function (p) { return p[0] === n; }).map(function (p) { return p[1]; }); };
  UP.has = function (n, v) {
    need(1, arguments, 'has'); n = USVString(n);
    var hv = v !== undefined; if (hv) v = USVString(v);
    return this._l.some(function (p) { return p[0] === n && (!hv || p[1] === v); });
  };
  UP.set = function (n, v) {
    need(2, arguments, 'set'); n = USVString(n); v = USVString(v);
    var found = false, out = [];
    for (var i = 0; i < this._l.length; i++) {
      var p = this._l[i];
      if (p[0] === n) { if (!found) { found = true; out.push([n, v]); } } else out.push(p);
    }
    if (!found) out.push([n, v]);
    this._l = out; this._upd();
  };
  UP.sort = function () {
    var a = this._l.map(function (p, i) { return [p, i]; });
    a.sort(function (x, y) { return x[0][0] < y[0][0] ? -1 : x[0][0] > y[0][0] ? 1 : x[1] - y[1]; });
    this._l = a.map(function (x) { return x[0]; }); this._upd();
  };
  UP.toString = function () { return this._l.map(function (p) { return enc(p[0]) + '=' + enc(p[1]); }).join('&'); };
  UP.forEach = function (cb, thisArg) {
    if (typeof cb !== 'function') throw new TypeError("Failed to execute 'forEach' on 'URLSearchParams': parameter 1 is not of type 'Function'.");
    for (var i = 0; i < this._l.length; i++) cb.call(thisArg, this._l[i][1], this._l[i][0], this);
  };
  function iter(self, kind) {
    var i = 0;
    var it = { next: function () {
      if (i >= self._l.length) return { value: undefined, done: true };
      var p = self._l[i++];
      return { value: kind === 0 ? p[0] : kind === 1 ? p[1] : [p[0], p[1]], done: false };
    } };
    it[Symbol.iterator] = function () { return it; };
    return it;
  }
  UP.keys = function () { return iter(this, 0); };
  UP.values = function () { return iter(this, 1); };
  UP.entries = function () { return iter(this, 2); };
  UP[Symbol.iterator] = UP.entries;
  Object.defineProperty(UP, 'size', { get: function () { return this._l.length; }, configurable: true });
  Object.defineProperty(UP, Symbol.toStringTag, { value: 'URLSearchParams', configurable: true });
  Object.keys(UP).forEach(function (k) { Object.defineProperty(UP, k, { enumerable: false }); });

  var KEYS = ['href', 'protocol', 'username', 'password', 'host', 'hostname', 'port', 'pathname', 'search', 'hash', 'origin'];
  function URL(url, base) {
    if (!(this instanceof URL)) throw new TypeError("Failed to construct 'URL': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    if (arguments.length === 0) throw new TypeError("Failed to construct 'URL': 1 argument required, but only 0 present.");
    var r = parse(USVString(url), base === undefined ? undefined : USVString(base));
    if (!r) throw new TypeError("Failed to construct 'URL': Invalid URL");
    Object.defineProperty(this, '_p', { value: r, writable: true });
    var sp = new URLSearchParams(r[8]);
    sp._u = this;
    Object.defineProperty(this, '_sp', { value: sp });
  }
  var UU = URL.prototype;
  Object.defineProperty(UU, '_setQuery', { value: function (q) {
    this._p = set(this._p[0], 'search', q) || this._p;
  } });
  KEYS.forEach(function (k, idx) {
    Object.defineProperty(UU, k, {
      get: function () { return this._p[idx]; },
      set: k === 'origin' ? undefined : function (v) {
        v = String(v);
        var r = set(this._p[0], k, v);
        if (k === 'href' && !r) throw new TypeError("Failed to set the 'href' property on 'URL': Invalid URL");
        if (r) this._p = r;
        if (k === 'href' || k === 'search') this._sp._parse(this._p[8]);
      },
      enumerable: true, configurable: true,
    });
  });
  Object.defineProperty(UU, 'searchParams', { get: function () { return this._sp; }, enumerable: true, configurable: true });
  UU.toString = function () { return this._p[0]; };
  UU.toJSON = function () { return this._p[0]; };
  Object.defineProperty(UU, Symbol.toStringTag, { value: 'URL', configurable: true });
  URL.canParse = function (u, b) { try { return !!parse(USVString(u), b === undefined ? undefined : USVString(b)); } catch (e) { return false; } };
  URL.parse = function (u, b) { try { return new URL(u, b); } catch (e) { return null; } };
  var blobN = 0;
  URL.createObjectURL = function () { return 'blob:' + (g.location && g.location.origin !== 'null' ? g.location.origin : 'null') + '/' + (++blobN) + '-scratchy'; };
  URL.revokeObjectURL = function () {};
  def('URL', URL); def('URLSearchParams', URLSearchParams);
})(globalThis);
