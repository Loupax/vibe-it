// QMAPI1 (R64 section 5): report mode only. ONE forwarding Proxy goes at the end of the prototype chains of
// window and the DOM objects (the DOM interfaces' prototypes whose parent is Object.prototype, EventTarget's
// among them). A lookup reaches it only for a name found nowhere before, i.e. exactly a miss; the traps record
// it and forward to Object.prototype, so every answer the page gets is unchanged.
// The IDL placeholder below is replaced by src/missing_api_idl.json (tools/missing-api/gen-idl.py) before the eval.
(function (g) {
  'use strict';
  var IDL = __MAPI_IDL__;
  var OP = Object.prototype;
  var hasOwn = OP.hasOwnProperty;
  var ifaces = IDL.interfaces, spaces = IDL.namespaces;
  // member name -> true, per interface (own members only; the receiver's chain is walked at lookup time)
  var own = Object.create(null), anyMember = Object.create(null), globalNames = Object.create(null);
  Object.keys(ifaces).forEach(function (n) {
    var s = own[n] = Object.create(null);
    ifaces[n].m.forEach(function (m) { s[m] = 1; anyMember[m] = 1; });
    globalNames[n] = 1;
  });
  Object.keys(spaces).forEach(function (n) {
    var s = own[n] = Object.create(null);
    spaces[n].forEach(function (m) { s[m] = 1; });
    globalNames[n] = 1;
  });
  var table = new Map();      // kind \0 iface \0 name -> {iface, kind, name, hits, script}
  var ignored = 0, dropped = 0;
  var MAX = 20000;
  var inTrap = false;

  function chainNames(o) {
    var names = [], p = o, n = 0;
    while (p && p !== OP && n++ < 12) {
      if (hasOwn.call(p, 'constructor')) {
        var c = p.constructor;
        if (typeof c === 'function' && typeof c.name === 'string') names.push(c.name);
      }
      p = Object.getPrototypeOf(p);
    }
    return names;
  }
  function firstScript() {
    var st = '';
    try { st = String(new Error().stack); } catch (e) { return ''; }
    var lines = st.split('\n');
    for (var i = 1; i < lines.length; i++) {
      var l = lines[i];
      if (l.indexOf('missing_api') >= 0) continue;
      var m = /\(([^()]*?)(?::\d+:\d+)?\)\s*$/.exec(l) || /at (.*?)(?::\d+:\d+)?\s*$/.exec(l);
      if (m) return m[1];
    }
    return '';
  }
  function note(kind, name, receiver) {
    var names = kind === 'get' && receiver !== undefined && receiver !== null && (typeof receiver === 'object' || typeof receiver === 'function') ? chainNames(receiver) : null;
    var iface = '?', std = false;
    if (names === null) {
      std = anyMember[name] === 1 || globalNames[name] === 1;
    } else {
      iface = names.length ? names[0] : '?';
      for (var i = 0; i < names.length && !std; i++) {
        var s = own[names[i]];
        if (s && s[name] === 1) std = true;
      }
      if (!std && names.indexOf('Window') >= 0 && globalNames[name] === 1) std = true;
      if (!std && names.length === 0 && receiver === g) { iface = 'Window'; std = globalNames[name] === 1; }
    }
    if (!std) { ignored++; return; }
    var key = kind + '\0' + iface + '\0' + name;
    var e = table.get(key);
    if (e) { e.hits++; return; }
    if (table.size >= MAX) { dropped++; return; }
    table.set(key, { iface: iface, kind: kind, name: name, hits: 1, script: firstScript() });
  }
  var handler = {
    get: function (t, name, receiver) {
      if (typeof name !== 'symbol' && !(name in OP) && !inTrap) {
        inTrap = true;
        try { note('get', name, receiver); } catch (e) { /* never change the page's answer */ }
        inTrap = false;
      }
      return Reflect.get(OP, name, receiver);
    },
    has: function (t, name) {
      if (typeof name !== 'symbol' && !(name in OP) && !inTrap) {
        inTrap = true;
        try { note('has', name); } catch (e) { /* idem */ }
        inTrap = false;
      }
      return Reflect.has(OP, name);
    },
  };
  var proxy = new Proxy(OP, handler);
  var done = new Set();
  function hook(p) {
    if (p && typeof p === 'object' && !done.has(p) && Object.getPrototypeOf(p) === OP) {
      done.add(p);
      Object.setPrototypeOf(p, proxy);
    }
  }
  Object.keys(ifaces).forEach(function (n) {
    var c = Object.getOwnPropertyDescriptor(g, n);
    if (c && typeof c.value === 'function') hook(c.value.prototype);
  });
  Object.keys(spaces).forEach(function (n) {
    var c = Object.getOwnPropertyDescriptor(g, n);
    if (c && c.value && typeof c.value === 'object') hook(c.value);
  });
  // Window itself: its chain ends at EventTarget.prototype -> proxy already; a global without that link (a
  // realm quirk) is hooked directly.
  if (Object.getPrototypeOf(g) === OP) hook(g);
  Object.defineProperty(g, '__mapi_dump', {
    value: function () {
      var out = [];
      table.forEach(function (e) { out.push(e); });
      out.sort(function (a, b) { return b.hits - a.hits || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0); });
      return JSON.stringify({ members: out, ignored: ignored, dropped: dropped });
    },
    enumerable: false,
  });
})(globalThis);
