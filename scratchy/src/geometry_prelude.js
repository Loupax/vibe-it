// QGEOM1: Geometry Interfaces Module Level 1 (DOMPoint, DOMRect, DOMQuad, DOMMatrix and their read-only bases).
// Runs at the head of DOM_PRELUDE (once per realm). The DOM prelude takes DOMRect / DOMMatrix from
// `g.__scratchy_geom` (removed again) so a page that overwrites the globals does not change getBoundingClientRect.
(function (g) {
  var TE = function (m) { return new TypeError(m); };
  // Brand stores. A symbol from the page-wide registry (not a per-realm WeakMap) so a method of another realm
  // (an iframe's DOMRectReadOnly.prototype.toJSON.call(thisRealmRect)) finds the state too.
  var store = function (name) {
    var key = Symbol.for('scratchy.geometry.' + name);
    return {
      get: function (o) { return Object.prototype.hasOwnProperty.call(o, key) ? o[key] : undefined; },
      set: function (o, v) { Object.defineProperty(o, key, { value: v, writable: true, enumerable: false, configurable: false }); }
    };
  };
  var illegal = function () { return TE('Illegal invocation'); };
  var num = function (v) { return +v; }; // ToNumber (a symbol throws), `unrestricted double`
  var defMethod = function (proto, name, len, fn) {
    Object.defineProperty(fn, 'name', { value: name, configurable: true });
    Object.defineProperty(fn, 'length', { value: len, configurable: true });
    Object.defineProperty(proto, name, { value: fn, writable: true, enumerable: true, configurable: true });
  };
  var defGetter = function (proto, name, get, set) {
    Object.defineProperty(get, 'name', { value: 'get ' + name, configurable: true });
    var d = { get: get, enumerable: true, configurable: true };
    if (set) { Object.defineProperty(set, 'name', { value: 'set ' + name, configurable: true }); d.set = set; }
    Object.defineProperty(proto, name, d);
  };
  var defClass = function (ctor, name, proto) {
    Object.defineProperty(ctor, 'prototype', { value: proto, writable: false, enumerable: false, configurable: false });
    Object.defineProperty(proto, 'constructor', { value: ctor, writable: true, enumerable: false, configurable: true });
    Object.defineProperty(proto, Symbol.toStringTag, { value: name, configurable: true });
    Object.defineProperty(g, name, { value: ctor, writable: true, enumerable: false, configurable: true });
  };
  var ctorName = function (name, len) {
    return function (fn) {
      Object.defineProperty(fn, 'name', { value: name, configurable: true });
      Object.defineProperty(fn, 'length', { value: len, configurable: true });
      return fn;
    };
  };
  // A dictionary argument: undefined / null = empty, an object is read, anything else is a TypeError.
  var dict = function (v, what) {
    if (v === undefined || v === null) return {};
    if (typeof v !== 'object' && typeof v !== 'function') throw TE('The provided value is not of type \'' + what + '\'.');
    return v;
  };
  // Members in the lexicographic order WebIDL reads them (inherited dictionary first, which the callers list).
  var read = function (d, keys, dflt) {
    var o = {};
    for (var i = 0; i < keys.length; i++) {
      var v = d[keys[i]];
      o[keys[i]] = v === undefined ? dflt[keys[i]] : num(v);
    }
    return o;
  };
  var has = function (d, k) { return d[k] !== undefined; };
  var sameZero = function (a, b) { return a === b || (a !== a && b !== b); };
  var isZero = function (v) { return v === 0; };

  // ---------- DOMPoint ----------
  var PS = store('PS');
  var pst = function (o, rw) {
    var s = (o !== null && typeof o === 'object') ? PS.get(o) : undefined;
    if (!s || (rw && !s.rw)) throw illegal();
    return s;
  };
  var PKEYS = ['w', 'x', 'y', 'z'];
  var PDEF = { x: 0, y: 0, z: 0, w: 1 };
  var mkPoint = function (proto, rw, x, y, z, w) {
    var o = Object.create(proto);
    PS.set(o, { x: x, y: y, z: z, w: w, rw: rw });
    return o;
  };
  var DOMPointReadOnly = ctorName('DOMPointReadOnly', 0)(function () {
    if (!new.target) throw TE("Failed to construct 'DOMPointReadOnly': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var a = arguments;
    PS.set(this, { x: a[0] === undefined ? 0 : num(a[0]), y: a[1] === undefined ? 0 : num(a[1]),
      z: a[2] === undefined ? 0 : num(a[2]), w: a[3] === undefined ? 1 : num(a[3]), rw: false });
  });
  var DOMPoint = ctorName('DOMPoint', 0)(function () {
    if (!new.target) throw TE("Failed to construct 'DOMPoint': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var a = arguments;
    PS.set(this, { x: a[0] === undefined ? 0 : num(a[0]), y: a[1] === undefined ? 0 : num(a[1]),
      z: a[2] === undefined ? 0 : num(a[2]), w: a[3] === undefined ? 1 : num(a[3]), rw: true });
  });
  var DPRp = {}, DPp = Object.create(DPRp);
  defClass(DOMPointReadOnly, 'DOMPointReadOnly', DPRp);
  defClass(DOMPoint, 'DOMPoint', DPp);
  Object.setPrototypeOf(DOMPoint, DOMPointReadOnly);
  ['x', 'y', 'z', 'w'].forEach(function (k) {
    defGetter(DPRp, k, function () { return pst(this)[k]; });
    defGetter(DPp, k, function () { return pst(this)[k]; }, function (v) { pst(this, true)[k] = num(v); });
  });
  var pointInit = function (v) { var d = read(dict(v, 'DOMPointInit'), PKEYS, PDEF); return d; };
  defMethod(DOMPointReadOnly, 'fromPoint', 0, function (o) { var d = pointInit(o); return mkPoint(DPRp, false, d.x, d.y, d.z, d.w); });
  defMethod(DOMPoint, 'fromPoint', 0, function (o) { var d = pointInit(o); return mkPoint(DPp, true, d.x, d.y, d.z, d.w); });
  defMethod(DPRp, 'toJSON', 0, function () { var s = pst(this); return { x: s.x, y: s.y, z: s.z, w: s.w }; });

  // ---------- DOMRect ----------
  var RS = store('RS');
  var rst = function (o, rw) {
    var s = (o !== null && typeof o === 'object') ? RS.get(o) : undefined;
    if (!s || (rw && !s.rw)) throw illegal();
    return s;
  };
  var RKEYS = ['height', 'width', 'x', 'y'];
  var RDEF = { x: 0, y: 0, width: 0, height: 0 };
  var mkRect = function (proto, rw, x, y, w, h) {
    var o = Object.create(proto);
    RS.set(o, { x: x, y: y, width: w, height: h, rw: rw });
    return o;
  };
  var rectCtor = function (name, rw) {
    return ctorName(name, 0)(function () {
      if (!new.target) throw TE("Failed to construct '" + name + "': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
      var a = arguments;
      RS.set(this, { x: a[0] === undefined ? 0 : num(a[0]), y: a[1] === undefined ? 0 : num(a[1]),
        width: a[2] === undefined ? 0 : num(a[2]), height: a[3] === undefined ? 0 : num(a[3]), rw: rw });
    });
  };
  var DOMRectReadOnly = rectCtor('DOMRectReadOnly', false), DOMRect = rectCtor('DOMRect', true);
  var DRRp = {}, DRp = Object.create(DRRp);
  defClass(DOMRectReadOnly, 'DOMRectReadOnly', DRRp);
  defClass(DOMRect, 'DOMRect', DRp);
  Object.setPrototypeOf(DOMRect, DOMRectReadOnly);
  ['x', 'y', 'width', 'height'].forEach(function (k) {
    defGetter(DRRp, k, function () { return rst(this)[k]; });
    defGetter(DRp, k, function () { return rst(this)[k]; }, function (v) { rst(this, true)[k] = num(v); });
  });
  var rdef = function (name, f) { defGetter(DRRp, name, function () { return f(rst(this)); }); };
  rdef('top', function (d) { return Math.min(d.y, d.y + d.height); });
  rdef('right', function (d) { return Math.max(d.x, d.x + d.width); });
  rdef('bottom', function (d) { return Math.max(d.y, d.y + d.height); });
  rdef('left', function (d) { return Math.min(d.x, d.x + d.width); });
  var rectInit = function (v) { return read(dict(v, 'DOMRectInit'), RKEYS, RDEF); };
  defMethod(DOMRectReadOnly, 'fromRect', 0, function (o) { var d = rectInit(o); return mkRect(DRRp, false, d.x, d.y, d.width, d.height); });
  defMethod(DOMRect, 'fromRect', 0, function (o) { var d = rectInit(o); return mkRect(DRp, true, d.x, d.y, d.width, d.height); });
  defMethod(DRRp, 'toJSON', 0, function () {
    var s = rst(this);
    return { x: s.x, y: s.y, width: s.width, height: s.height, top: this.top, right: this.right, bottom: this.bottom, left: this.left };
  });

  // ---------- DOMMatrix ----------
  // m: 16 numbers in m11, m12, ..., m44 order (column-major: the math matrix P has P[r][c] = m[4c + r]).
  var MS = store('MS');
  var mst = function (o, rw) {
    var s = (o !== null && typeof o === 'object') ? MS.get(o) : undefined;
    if (!s || (rw && !s.rw)) throw illegal();
    return s;
  };
  var IDENT = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  var M2D = function (a, b, c, d, e, f) { return [a, b, 0, 0, c, d, 0, 0, 0, 0, 1, 0, e, f, 0, 1]; };
  var DMRp = {}, DMp = Object.create(DMRp);
  var mkMatrix = function (proto, rw, m, is2D) {
    var o = Object.create(proto);
    MS.set(o, { m: m, is2D: is2D, rw: rw });
    return o;
  };
  var DOMEx = function (msg, name) { return new g.DOMException(msg, name); };
  var N2D = ['a', 'b', 'c', 'd', 'e', 'f', 'm11', 'm12', 'm21', 'm22', 'm41', 'm42'];
  var N3D = ['is2D', 'm13', 'm14', 'm23', 'm24', 'm31', 'm32', 'm33', 'm34', 'm43', 'm44'];
  // DOMMatrix2DInit "validate and fixup": returns [m11, m12, m21, m22, m41, m42] (all present).
  var fixup2D = function (d, what) {
    var o = {}, i;
    for (i = 0; i < N2D.length; i++) { var v = d[N2D[i]]; if (v !== undefined) o[N2D[i]] = num(v); }
    var pairs = [['a', 'm11', 1], ['b', 'm12', 0], ['c', 'm21', 0], ['d', 'm22', 1], ['e', 'm41', 0], ['f', 'm42', 0]];
    for (i = 0; i < pairs.length; i++) {
      var p = pairs[i], x = o[p[0]], y = o[p[1]];
      if (x !== undefined && y !== undefined && !sameZero(x, y)) throw TE("The '" + p[0] + "' and '" + p[1] + "' members must be equal.");
      o[p[1]] = y !== undefined ? y : (x !== undefined ? x : p[2]);
    }
    return o;
  };
  // DOMMatrixInit "validate and fixup": returns { m: [16], is2D }.
  var fixupInit = function (v) {
    var d = dict(v, 'DOMMatrixInit'), o = {}, i;
    var two = fixup2D(d);
    var is2D = d.is2D === undefined ? undefined : !!d.is2D;
    var m3 = {};
    for (i = 1; i < N3D.length; i++) { var x = d[N3D[i]]; if (x !== undefined) m3[N3D[i]] = num(x); }
    var def3 = { m13: 0, m14: 0, m23: 0, m24: 0, m31: 0, m32: 0, m33: 1, m34: 0, m43: 0, m44: 1 };
    var non2D = false;
    for (var k in def3) if (m3[k] !== undefined && m3[k] !== def3[k]) non2D = true;
    if (is2D === true && non2D) throw TE('The is2D member is set to true but the input matrix is a 3d matrix.');
    if (is2D === undefined) is2D = !non2D;
    for (var k2 in def3) if (m3[k2] === undefined) m3[k2] = def3[k2];
    var m = [two.m11, two.m12, m3.m13, m3.m14, two.m21, two.m22, m3.m23, m3.m24, m3.m31, m3.m32, m3.m33, m3.m34, two.m41, two.m42, m3.m43, m3.m44];
    if (is2D) m = M2D(m[0], m[1], m[4], m[5], m[12], m[13]);
    return { m: m, is2D: is2D };
  };
  var mul4 = function (A, B) { // P_A * P_B
    var R = new Array(16);
    for (var c = 0; c < 4; c++) for (var r = 0; r < 4; r++) {
      var s = 0;
      for (var k = 0; k < 4; k++) s += A[4 * k + r] * B[4 * c + k];
      R[4 * c + r] = s;
    }
    return R;
  };
  var mul2 = function (A, B) { // 2D parts only (a b c d e f)
    var a = A[0], b = A[1], c = A[4], d = A[5], e = A[12], f = A[13];
    var a2 = B[0], b2 = B[1], c2 = B[4], d2 = B[5], e2 = B[12], f2 = B[13];
    return M2D(a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f);
  };
  // this = this * B (post) or B * this (pre); `b2D` says B is a 2D matrix.
  var applyMul = function (s, B, b2D, pre) {
    if (s.is2D && b2D) s.m = pre ? mul2(B, s.m) : mul2(s.m, B);
    else { s.m = pre ? mul4(B, s.m) : mul4(s.m, B); s.is2D = false; }
  };
  var allFinite = function (m) { for (var i = 0; i < m.length; i++) if (!isFinite(m[i])) return false; return true; };
  var invert = function (s) {
    var m = s.m, out = null;
    if (allFinite(m)) {
      if (s.is2D) {
        var a = m[0], b = m[1], c = m[4], d = m[5], e = m[12], f = m[13], det = a * d - b * c;
        if (det !== 0 && isFinite(det)) out = M2D(d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det);
      } else {
        // Cofactor expansion (the classic 4x4 adjugate / determinant).
        var inv = new Array(16);
        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
        var det4 = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if (det4 !== 0 && isFinite(det4)) {
          for (var q = 0; q < 16; q++) inv[q] /= det4;
          if (allFinite(inv)) out = inv;
        }
      }
    }
    if (out) s.m = out;
    else { s.m = [NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN, NaN]; s.is2D = false; }
  };
  var rad = function (deg) { return deg * Math.PI / 180; };
  var rotZ = function (deg) { var a = rad(deg), c = Math.cos(a), s = Math.sin(a); return M2D(c, s, -s, c, 0, 0); };
  var rot3 = function (x, y, z, deg) {
    var len = Math.sqrt(x * x + y * y + z * z);
    if (len === 0) return IDENT.slice();
    x /= len; y /= len; z /= len;
    var a = rad(deg) / 2, sc = Math.sin(a) * Math.cos(a), sq = Math.sin(a) * Math.sin(a);
    return [1 - 2 * (y * y + z * z) * sq, 2 * (x * y * sq + z * sc), 2 * (x * z * sq - y * sc), 0,
      2 * (x * y * sq - z * sc), 1 - 2 * (x * x + z * z) * sq, 2 * (y * z * sq + x * sc), 0,
      2 * (x * z * sq + y * sc), 2 * (y * z * sq - x * sc), 1 - 2 * (x * x + y * y) * sq, 0,
      0, 0, 0, 1];
  };
  var trans = function (x, y, z) { var m = IDENT.slice(); m[12] = x; m[13] = y; m[14] = z; return m; };
  var arg = function (a, i, d) { return a[i] === undefined ? d : num(a[i]); };
  var doTranslate = function (s, tx, ty, tz) {
    if (tz !== 0) { applyMul(s, trans(tx, ty, tz), false); } else applyMul(s, M2D(1, 0, 0, 1, tx, ty), true);
  };
  var doScale = function (s, sx, sy, sz, ox, oy, oz) {
    var org = ox !== 0 || oy !== 0 || oz !== 0;
    if (org) doTranslate(s, ox, oy, oz);
    if (sz !== 1) { var m = IDENT.slice(); m[0] = sx; m[5] = sy; m[10] = sz; applyMul(s, m, false); } else applyMul(s, M2D(sx, 0, 0, sy, 0, 0), true);
    if (org) doTranslate(s, -ox, -oy, -oz);
  };
  var doRotate = function (s, rx, ry, rz) {
    if (rz !== 0) { if (rx !== 0 || ry !== 0) applyMul(s, rot3(0, 0, 1, rz), false); else applyMul(s, rotZ(rz), true); }
    if (ry !== 0) applyMul(s, rot3(0, 1, 0, ry), false);
    if (rx !== 0) applyMul(s, rot3(1, 0, 0, rx), false);
  };
  var doSkewX = function (s, deg) { applyMul(s, M2D(1, 0, Math.tan(rad(deg)), 1, 0, 0), true); };
  var doSkewY = function (s, deg) { applyMul(s, M2D(1, Math.tan(rad(deg)), 0, 1, 0, 0), true); };
  var mulArg = function (o) { var f = fixupInit(o); return f; };
  var doMul = function (s, o, pre) { var f = mulArg(o); applyMul(s, f.m, f.is2D, pre); };

  // ---- CSS transform list parsing (DOMMatrix(string), setMatrixValue) ----
  var NUM = '[+-]?(?:\\d+\\.?\\d*|\\.\\d+)(?:[eE][+-]?\\d+)?';
  var LEN_RE = new RegExp('^(' + NUM + ')(px|cm|mm|q|in|pt|pc)?$', 'i');
  var ANG_RE = new RegExp('^(' + NUM + ')(deg|grad|rad|turn)?$', 'i');
  var NUM_RE = new RegExp('^' + NUM + '$');
  var LENU = { px: 1, cm: 96 / 2.54, mm: 96 / 25.4, q: 96 / 101.6, 'in': 96, pt: 96 / 72, pc: 16 };
  var ANGU = { deg: 1, grad: 0.9, rad: 180 / Math.PI, turn: 360 };
  var synErr = function (what) { return DOMEx('Failed to parse \'' + what + '\' as a transform list.', 'SyntaxError'); };
  var parseTransformList = function (str) {
    var s = { m: IDENT.slice(), is2D: true };
    str = String(str);
    var rest = str.replace(/\/\*[\s\S]*?\*\//g, ' ');
    if (str === '') return s;
    rest = rest.replace(/^[ \t\r\n\f]+|[ \t\r\n\f]+$/g, '');
    if (/^none$/i.test(rest)) return s;
    var fail = function () { throw synErr(str); };
    // calc(): + - * / and parentheses over numbers, lengths and angles; returns {v, t} (t: n, l, a).
    var calc = function (e) {
      var toks = e.match(new RegExp(NUM + '(?:[a-zA-Z]+)?|[-+*/()]', 'g')) || [], pos = 0;
      if (toks.join('').length !== e.replace(/[ \t\r\n\f]+/g, '').length) fail();
      var unit = function (t) {
        var r = new RegExp('^(' + NUM + ')([a-zA-Z]*)$').exec(t);
        if (!r) fail();
        var u = r[2].toLowerCase();
        if (u === '') return { v: +r[1], t: 'n' };
        if (LENU[u] !== undefined) return { v: +r[1] * LENU[u], t: 'l' };
        if (ANGU[u] !== undefined) return { v: +r[1] * ANGU[u], t: 'a' };
        fail();
      };
      var sum, prod, atom = function () {
        var t = toks[pos++];
        if (t === undefined) fail();
        if (t === '(') { var v = sum(); if (toks[pos++] !== ')') fail(); return v; }
        if (t === '-' || t === '+' || t === '*' || t === '/' || t === ')') fail();
        return unit(t);
      };
      prod = function () {
        var l = atom();
        while (toks[pos] === '*' || toks[pos] === '/') {
          var op = toks[pos++], r = atom();
          if (op === '*') { if (l.t !== 'n' && r.t !== 'n') fail(); l = { v: l.v * r.v, t: l.t === 'n' ? r.t : l.t }; }
          else { if (r.t !== 'n') fail(); l = { v: l.v / r.v, t: l.t }; }
        }
        return l;
      };
      sum = function () {
        var l = prod();
        while (toks[pos] === '+' || toks[pos] === '-') {
          var op = toks[pos++], r = prod();
          if (l.t !== r.t) fail();
          l = { v: op === '+' ? l.v + r.v : l.v - r.v, t: l.t };
        }
        return l;
      };
      var res = sum();
      if (pos !== toks.length) fail();
      return res;
    };
    var withCalc = function (t, kind, plain) {
      var m = /^calc\(([\s\S]*)\)$/i.exec(t);
      if (!m) return plain(t);
      var r = calc(m[1]);
      if (r.t !== kind) fail();
      return r.v;
    };
    var len = function (t) {
      return withCalc(t, 'l', function (t) {
        var r = LEN_RE.exec(t);
        if (!r) fail();
        if (!r[2]) { if (+r[1] !== 0) fail(); return 0; }
        return +r[1] * LENU[r[2].toLowerCase()];
      });
    };
    var ang = function (t) {
      return withCalc(t, 'a', function (t) {
        var r = ANG_RE.exec(t);
        if (!r) fail();
        if (!r[2]) { if (+r[1] !== 0) fail(); return 0; }
        return +r[1] * ANGU[r[2].toLowerCase()];
      });
    };
    var number = function (t) { return withCalc(t, 'n', function (t) { if (!NUM_RE.test(t)) fail(); return +t; }); };
    if (rest === '') fail();
    while (rest !== '') {
      var mm = /^([a-zA-Z][a-zA-Z0-9]*)\(/.exec(rest);
      if (!mm) fail();
      var depth = 1, j = mm[0].length;
      for (; j < rest.length && depth > 0; j++) { if (rest[j] === '(') depth++; else if (rest[j] === ')') depth--; }
      if (depth !== 0) fail();
      var inner = rest.slice(mm[0].length, j - 1);
      rest = rest.slice(j).replace(/^[ \t\r\n\f]+/, '');
      var fn = mm[1].toLowerCase(), raw = inner.replace(/^[ \t\r\n\f]+|[ \t\r\n\f]+$/g, '');
      var args = [], d2 = 0, st = 0;
      if (raw !== '') {
        for (var q = 0; q <= raw.length; q++) {
          if (q === raw.length || (raw[q] === ',' && d2 === 0)) { args.push(raw.slice(st, q).replace(/^[ \t\r\n\f]+|[ \t\r\n\f]+$/g, '')); st = q + 1; }
          else if (raw[q] === '(') d2++; else if (raw[q] === ')') d2--;
        }
      }
      for (var i = 0; i < args.length; i++) if (args[i] === '' || (!/^calc\(/i.test(args[i]) && /[ \t\r\n\f]/.test(args[i]))) fail();
      var n = args.length, f;
      var need = function (lo, hi) { if (n < lo || n > hi) fail(); };
      switch (fn) {
        case 'matrix': need(6, 6); f = args.map(number); applyMul(s, M2D(f[0], f[1], f[2], f[3], f[4], f[5]), true); break;
        case 'matrix3d': need(16, 16); applyMul(s, args.map(number), false); break;
        case 'translate': need(1, 2); doTranslate(s, len(args[0]), n > 1 ? len(args[1]) : 0, 0); break;
        case 'translatex': need(1, 1); doTranslate(s, len(args[0]), 0, 0); break;
        case 'translatey': need(1, 1); doTranslate(s, 0, len(args[0]), 0); break;
        case 'translatez': need(1, 1); applyMul(s, trans(0, 0, len(args[0])), false); break;
        case 'translate3d': need(3, 3); applyMul(s, trans(len(args[0]), len(args[1]), len(args[2])), false); break;
        case 'scale': need(1, 2); f = number(args[0]); doScale(s, f, n > 1 ? number(args[1]) : f, 1, 0, 0, 0); break;
        case 'scalex': need(1, 1); doScale(s, number(args[0]), 1, 1, 0, 0, 0); break;
        case 'scaley': need(1, 1); doScale(s, 1, number(args[0]), 1, 0, 0, 0); break;
        case 'scalez': need(1, 1); f = IDENT.slice(); f[10] = number(args[0]); applyMul(s, f, false); break;
        case 'scale3d': need(3, 3); f = IDENT.slice(); f[0] = number(args[0]); f[5] = number(args[1]); f[10] = number(args[2]); applyMul(s, f, false); break;
        case 'rotate': need(1, 1); doRotate(s, 0, 0, ang(args[0])); break;
        case 'rotatez': need(1, 1); applyMul(s, rotZ(ang(args[0])), false); break;
        case 'rotatex': need(1, 1); applyMul(s, rot3(1, 0, 0, ang(args[0])), false); break;
        case 'rotatey': need(1, 1); applyMul(s, rot3(0, 1, 0, ang(args[0])), false); break;
        case 'rotate3d': need(4, 4); applyMul(s, rot3(number(args[0]), number(args[1]), number(args[2]), ang(args[3])), false); break;
        case 'skew': need(1, 2); doSkewX(s, ang(args[0])); if (n > 1) doSkewY(s, ang(args[1])); break;
        case 'skewx': need(1, 1); doSkewX(s, ang(args[0])); break;
        case 'skewy': need(1, 1); doSkewY(s, ang(args[0])); break;
        case 'perspective':
          need(1, 1);
          if (args[0].toLowerCase() === 'none') break;
          f = len(args[0]); if (f < 0) fail();
          if (f > 0) { var pm = IDENT.slice(); pm[11] = -1 / f; applyMul(s, pm, false); }
          break;
        default: fail();
      }
    }
    return s;
  };
  // 3D function calls make the matrix 3D even when the numbers are trivially 2D.
  var parse3DFlag = function (str, s) {
    if (/(?:matrix3d|translatez|translate3d|scalez|scale3d|rotatex|rotatey|rotatez|rotate3d|perspective)\s*\(/i.test(String(str)) && !/^\s*(none)?\s*$/i.test(String(str))) {
      var fnl = String(str).match(/[a-zA-Z0-9]+(?=\s*\()/g) || [];
      for (var i = 0; i < fnl.length; i++) if (/^(?:matrix3d|translatez|translate3d|scalez|scale3d|rotatex|rotatey|rotatez|rotate3d|perspective)$/i.test(fnl[i])) { s.is2D = false; break; }
    }
    return s;
  };
  var fromString = function (str) { return parse3DFlag(str, parseTransformList(str)); };

  var fromSeq = function (init) {
    var l = Array.from(init, num);
    if (l.length === 6) return { m: M2D(l[0], l[1], l[2], l[3], l[4], l[5]), is2D: true };
    if (l.length === 16) return { m: l, is2D: false };
    throw TE('The sequence must contain 6 elements for a 2D matrix or 16 elements for a 3D matrix.');
  };
  var matrixCtor = function (name, rw) {
    return ctorName(name, 0)(function () {
      if (!new.target) throw TE("Failed to construct '" + name + "': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
      var init = arguments[0], st;
      if (init === undefined) st = { m: IDENT.slice(), is2D: true };
      else if (init !== null && typeof init === 'object' && typeof init[Symbol.iterator] === 'function') st = fromSeq(init);
      else st = fromString(String(init));
      st.rw = rw;
      MS.set(this, st);
    });
  };
  var DOMMatrixReadOnly = matrixCtor('DOMMatrixReadOnly', false), DOMMatrix = matrixCtor('DOMMatrix', true);
  defClass(DOMMatrixReadOnly, 'DOMMatrixReadOnly', DMRp);
  defClass(DOMMatrix, 'DOMMatrix', DMp);
  Object.setPrototypeOf(DOMMatrix, DOMMatrixReadOnly);
  Object.defineProperty(g, 'WebKitCSSMatrix', { value: DOMMatrix, writable: true, enumerable: false, configurable: true });

  var NAMES = ['m11', 'm12', 'm13', 'm14', 'm21', 'm22', 'm23', 'm24', 'm31', 'm32', 'm33', 'm34', 'm41', 'm42', 'm43', 'm44'];
  var ALIAS = { a: 0, b: 1, c: 4, d: 5, e: 12, f: 13 };
  var attrs = [];
  NAMES.forEach(function (n, i) { attrs.push([n, i]); });
  Object.keys(ALIAS).forEach(function (n) { attrs.push([n, ALIAS[n]]); });
  attrs.forEach(function (p) {
    var name = p[0], i = p[1];
    defGetter(DMRp, name, function () { return mst(this).m[i]; });
    defGetter(DMp, name, function () { return mst(this).m[i]; }, function (v) {
      var s = mst(this, true);
      v = num(v);
      s.m[i] = v;
      // m13 m14 m23 m24 m31 m32 m34 m43 are 0 and m33 m44 are 1 in a 2D matrix.
      if (s.is2D && !(i === 0 || i === 1 || i === 4 || i === 5 || i === 12 || i === 13)) {
        if (i === 10 || i === 15 ? v !== 1 : !isZero(v)) s.is2D = false;
      }
    });
  });
  defGetter(DMRp, 'is2D', function () { return mst(this).is2D; });
  defGetter(DMRp, 'isIdentity', function () {
    var m = mst(this).m;
    for (var i = 0; i < 16; i++) if (m[i] !== IDENT[i]) return false;
    return true;
  });

  var copyOf = function (o, proto) { var s = mst(o); return mkMatrix(proto || DMp, true, s.m.slice(), s.is2D); };
  var fixed = function (st, proto, rw) { return mkMatrix(proto, rw, st.m, st.is2D); };
  defMethod(DOMMatrixReadOnly, 'fromMatrix', 0, function (o) { var f = fixupInit(o); return fixed(f, DMRp, false); });
  defMethod(DOMMatrix, 'fromMatrix', 0, function (o) { var f = fixupInit(o); return fixed(f, DMp, true); });
  var fromArr = function (cls, name, rw, proto) {
    defMethod(cls, name, 1, function (a) {
      var T = name === 'fromFloat32Array' ? g.Float32Array : g.Float64Array;
      if (arguments.length < 1) throw TE("Failed to execute '" + name + "' on '" + cls.name + "': 1 argument required, but only 0 present.");
      if (!(a instanceof T)) throw TE("Failed to execute '" + name + "' on '" + cls.name + "': parameter 1 is not of type '" + (T === g.Float32Array ? 'Float32Array' : 'Float64Array') + "'.");
      return fixed(fromSeq(a), proto, rw);
    });
  };
  fromArr(DOMMatrixReadOnly, 'fromFloat32Array', false, DMRp);
  fromArr(DOMMatrixReadOnly, 'fromFloat64Array', false, DMRp);
  fromArr(DOMMatrix, 'fromFloat32Array', true, DMp);
  fromArr(DOMMatrix, 'fromFloat64Array', true, DMp);

  // Self variants (DOMMatrix) and their copying twins (DOMMatrixReadOnly).
  var variants = [
    ['translate', 0, function (s, a) { doTranslate(s, arg(a, 0, 0), arg(a, 1, 0), arg(a, 2, 0)); }],
    ['scale', 0, function (s, a) {
      var sx = arg(a, 0, 1), sy = a[1] === undefined ? sx : num(a[1]), sz = arg(a, 2, 1), ox = arg(a, 3, 0), oy = arg(a, 4, 0), oz = arg(a, 5, 0);
      doScale(s, sx, sy, sz, ox, oy, oz);
      if (sz !== 1 || oz !== 0) s.is2D = false;
    }],
    ['scale3d', 0, function (s, a) {
      var k = arg(a, 0, 1);
      var oz = arg(a, 3, 0);
      doScale(s, k, k, k, arg(a, 1, 0), arg(a, 2, 0), oz);
      if (k !== 1 || oz !== 0) s.is2D = false;
    }],
    ['rotate', 0, function (s, a) {
      var rx = arg(a, 0, 0), ry = a[1] === undefined ? undefined : num(a[1]), rz = a[2] === undefined ? undefined : num(a[2]);
      if (ry === undefined && rz === undefined) { rz = rx; rx = 0; ry = 0; }
      doRotate(s, rx, ry === undefined ? 0 : ry, rz === undefined ? 0 : rz);
      if (rx !== 0 || (ry !== undefined && ry !== 0)) s.is2D = false;
    }],
    ['rotateFromVector', 0, function (s, a) {
      var x = arg(a, 0, 0), y = arg(a, 1, 0);
      applyMul(s, rotZ(x === 0 && y === 0 ? 0 : Math.atan2(y, x) * 180 / Math.PI), true);
    }],
    ['rotateAxisAngle', 0, function (s, a) {
      var x = arg(a, 0, 0), y = arg(a, 1, 0), z = arg(a, 2, 0), ang = arg(a, 3, 0);
      if (x === 0 && y === 0 && z !== 0) applyMul(s, rotZ(z < 0 ? -ang : ang), true);
      else { applyMul(s, rot3(x, y, z, ang), false); }
      if (x !== 0 || y !== 0) s.is2D = false;
    }],
    ['skewX', 0, function (s, a) { doSkewX(s, arg(a, 0, 0)); }],
    ['skewY', 0, function (s, a) { doSkewY(s, arg(a, 0, 0)); }],
    ['flipX', 0, function (s) { applyMul(s, M2D(-1, 0, 0, 1, 0, 0), true); }, true],
    ['flipY', 0, function (s) { applyMul(s, M2D(1, 0, 0, -1, 0, 0), true); }, true],
    ['multiply', 0, function (s, a) { doMul(s, a[0], false); }, 'multiplySelf'],
    ['preMultiplySelf', 0, function (s, a) { doMul(s, a[0], true); }, 'only'],
    ['inverse', 0, function (s) { invert(s); }, 'invertSelf']
  ];
  variants.forEach(function (v) {
    var name = v[0], fn = v[2], selfName = v[3] === 'only' ? name : (typeof v[3] === 'string' ? v[3] : name + 'Self');
    if (v[3] === true) { // flipX / flipY have no Self variant
      defMethod(DMRp, name, 0, function () { var s = mst(this), r = { m: s.m.slice(), is2D: s.is2D }; fn(r, arguments); return fixed(r, DMp, true); });
      return;
    }
    if (v[3] !== 'only') {
      defMethod(DMRp, name, v[1], function () { var s = mst(this), r = { m: s.m.slice(), is2D: s.is2D }; fn(r, arguments); return fixed(r, DMp, true); });
    }
    defMethod(DMp, selfName, v[1], function () { var s = mst(this, true); fn(s, arguments); return this; });
  });
  defMethod(DMRp, 'scaleNonUniform', 0, function () {
    var s = mst(this), r = { m: s.m.slice(), is2D: s.is2D };
    doScale(r, arguments[0] === undefined ? 1 : num(arguments[0]), arguments[1] === undefined ? 1 : num(arguments[1]), 1, 0, 0, 0);
    return fixed(r, DMp, true);
  });
  defMethod(DMRp, 'transformPoint', 0, function (p) {
    var s = mst(this), d = pointInit(p), m = s.m;
    return mkPoint(DPp, true,
      m[0] * d.x + m[4] * d.y + m[8] * d.z + m[12] * d.w,
      m[1] * d.x + m[5] * d.y + m[9] * d.z + m[13] * d.w,
      m[2] * d.x + m[6] * d.y + m[10] * d.z + m[14] * d.w,
      m[3] * d.x + m[7] * d.y + m[11] * d.z + m[15] * d.w);
  });
  defMethod(DPRp, 'matrixTransform', 0, function (o) {
    var s = pst(this), f = fixupInit(o), m = f.m;
    return mkPoint(DPp, true,
      m[0] * s.x + m[4] * s.y + m[8] * s.z + m[12] * s.w,
      m[1] * s.x + m[5] * s.y + m[9] * s.z + m[13] * s.w,
      m[2] * s.x + m[6] * s.y + m[10] * s.z + m[14] * s.w,
      m[3] * s.x + m[7] * s.y + m[11] * s.z + m[15] * s.w);
  });
  defMethod(DMRp, 'toFloat32Array', 0, function () { return new g.Float32Array(mst(this).m); });
  defMethod(DMRp, 'toFloat64Array', 0, function () { return new g.Float64Array(mst(this).m); });
  defMethod(DMRp, 'toJSON', 0, function () {
    var s = mst(this), o = {};
    Object.keys(ALIAS).forEach(function (n) { o[n] = s.m[ALIAS[n]]; });
    NAMES.forEach(function (n, i) { o[n] = s.m[i]; });
    o.is2D = s.is2D;
    o.isIdentity = this.isIdentity;
    return o;
  });
  defMethod(DMRp, 'toString', 0, function () {
    var s = mst(this), m = s.m;
    if (!allFinite(m)) throw DOMEx('The matrix contains non-finite values and cannot be serialized.', 'InvalidStateError');
    if (s.is2D) return 'matrix(' + [m[0], m[1], m[4], m[5], m[12], m[13]].join(', ') + ')';
    return 'matrix3d(' + m.join(', ') + ')';
  });
  defMethod(DMp, 'setMatrixValue', 1, function (str) {
    var s = mst(this, true);
    if (arguments.length < 1) throw TE("Failed to execute 'setMatrixValue' on 'DOMMatrix': 1 argument required, but only 0 present.");
    var r = fromString(String(str));
    s.m = r.m; s.is2D = r.is2D;
    return this;
  });

  // ---------- DOMQuad ----------
  var QS = store('QS');
  var qst = function (o) { var s = (o !== null && typeof o === 'object') ? QS.get(o) : undefined; if (!s) throw illegal(); return s; };
  var qpt = function (v) { var d = pointInit(v); return mkPoint(DPp, true, d.x, d.y, d.z, d.w); };
  var DOMQuad = ctorName('DOMQuad', 0)(function () {
    if (!new.target) throw TE("Failed to construct 'DOMQuad': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    var a = arguments;
    QS.set(this, { p: [qpt(a[0]), qpt(a[1]), qpt(a[2]), qpt(a[3])] });
  });
  var DQp = {};
  defClass(DOMQuad, 'DOMQuad', DQp);
  var mkQuad = function (p1, p2, p3, p4) { var o = Object.create(DQp); QS.set(o, { p: [p1, p2, p3, p4] }); return o; };
  [1, 2, 3, 4].forEach(function (n) { defGetter(DQp, 'p' + n, function () { return qst(this).p[n - 1]; }); });
  defMethod(DOMQuad, 'fromRect', 0, function (o) {
    var d = rectInit(o), x = d.x, y = d.y, w = d.width, h = d.height;
    return mkQuad(mkPoint(DPp, true, x, y, 0, 1), mkPoint(DPp, true, x + w, y, 0, 1), mkPoint(DPp, true, x + w, y + h, 0, 1), mkPoint(DPp, true, x, y + h, 0, 1));
  });
  defMethod(DOMQuad, 'fromQuad', 0, function (o) {
    var d = dict(o, 'DOMQuadInit');
    return mkQuad(qpt(d.p1), qpt(d.p2), qpt(d.p3), qpt(d.p4));
  });
  defMethod(DQp, 'getBounds', 0, function () {
    var p = qst(this).p.map(function (q) { return PS.get(q); });
    var xs = p.map(function (q) { return q.x; }), ys = p.map(function (q) { return q.y; });
    var l = Math.min.apply(null, xs), r = Math.max.apply(null, xs), t = Math.min.apply(null, ys), b = Math.max.apply(null, ys);
    return mkRect(DRp, true, l, t, r - l, b - t);
  });
  defMethod(DQp, 'toJSON', 0, function () {
    var p = qst(this).p;
    return { p1: p[0].toJSON(), p2: p[1].toJSON(), p3: p[2].toJSON(), p4: p[3].toJSON() };
  });

  Object.defineProperty(g, '__scratchy_geom', { value: {
    rect: function (x, y, w, h) { return mkRect(DRp, true, x, y, w, h); },
    rectRO: function (x, y, w, h) { return mkRect(DRRp, false, x, y, w, h); },
    matrix2d: function (a, b, c, d, e, f) { return mkMatrix(DMp, true, M2D(a, b, c, d, e, f), true); },
    fixup2D: fixup2D, dict: dict
  }, configurable: true });
})(globalThis);
