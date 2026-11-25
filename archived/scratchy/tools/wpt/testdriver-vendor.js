// scratchy's own testdriver-vendor.js (QWPT7): the browser half of WPT's testdriver.js.
// wire serves it in place of WPT's empty file: `--mount-override /resources/testdriver-vendor.js=THIS`.
// test_driver.click / send_keys / action_sequence (and bless, which clicks) become queued "ticks"
// of primitive input operations. `scratchy --wpt-th-one` pulls them with `__scratchy_td_next()`
// when the page is idle and runs each op through scratchy's trusted-input paths (src/wpt_input.rs:
// hit testing on the real layout, isTrusted events, focus, checkbox/label/submit default actions,
// text editing). The virtual clock moves by each tick's duration. Elsewhere (no
// `__scratchy_td_wake` native) the driver is not automated and testdriver.js keeps its defaults.
// Tick format (one op per line): `m X Y` pointer move (viewport px), `d B` / `u B` button down/up
// (WebDriver button numbers), `kd C` / `ku C` key down/up (C = code points, comma-separated),
// `w X Y DX DY` wheel, `f ID` focus element ID (wrapper id), `e ID` caret to the end, `t MS`
// tick duration. A job's promise settles when `__scratchy_td_next` finds its ticks used up.
(function () {
  'use strict';
  if (typeof __scratchy_td_wake !== 'function' || !window.test_driver_internal) return;
  var I = window.test_driver_internal;
  I.in_automation = true;
  var queue = []; // jobs: {ticks: [function -> string], i, resolve, reject}
  var cur = null;
  var pointer = { x: 0, y: 0 };

  function cps(s) {
    var out = [];
    for (var i = 0; i < s.length; i++) {
      var c = s.codePointAt(i);
      if (c > 0xffff) i++;
      out.push(c);
    }
    return out.join(',');
  }
  // WebDriver "get the in-view center point" of an element's first client rect.
  function center(el) {
    var r = el.getClientRects()[0];
    if (!r) throw new Error('move target out of bounds: element has no box');
    var l = Math.max(0, r.left), rt = Math.min(window.innerWidth, r.right);
    var t = Math.max(0, r.top), b = Math.min(window.innerHeight, r.bottom);
    return [Math.floor((l + rt) / 2), Math.floor((t + b) / 2)];
  }
  function originXY(origin, x, y) {
    x = Number(x) || 0; y = Number(y) || 0;
    if (origin === undefined || origin === null || origin === 'viewport') return [x, y];
    if (origin === 'pointer') return [pointer.x + x, pointer.y + y];
    if (origin && origin.nodeType === 1) { var c = center(origin); return [c[0] + x, c[1] + y]; }
    throw new Error('invalid argument: origin ' + origin);
  }
  function inView(x, y) {
    if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) throw new Error('move target out of bounds');
  }
  function button(a) { return a.button === undefined ? 0 : Number(a.button) | 0; }
  function push(ticks) {
    return new Promise(function (resolve, reject) {
      queue.push({ ticks: ticks, i: 0, resolve: resolve, reject: reject });
      __scratchy_td_wake();
    });
  }
  // One WebDriver tick: every source's action at index `i`, in source order.
  function actionTick(actions, i) {
    return function () {
      var ops = [], dur = 0;
      actions.forEach(function (src) {
        var a = src.actions[i];
        if (!a) return;
        if (a.duration !== undefined && a.duration !== null) dur = Math.max(dur, Number(a.duration) || 0);
        if (a.type === 'pause') return;
        if (src.type === 'key') {
          if (a.type === 'keyDown') ops.push('kd ' + cps(String(a.value)));
          else if (a.type === 'keyUp') ops.push('ku ' + cps(String(a.value)));
        } else if (src.type === 'pointer') {
          if (a.type === 'pointerMove') {
            var p = originXY(a.origin, a.x, a.y);
            inView(p[0], p[1]);
            pointer.x = p[0]; pointer.y = p[1];
            ops.push('m ' + p[0] + ' ' + p[1]);
          } else if (a.type === 'pointerDown') ops.push('d ' + button(a));
          else if (a.type === 'pointerUp') ops.push('u ' + button(a));
        } else if (src.type === 'wheel' && a.type === 'scroll') {
          var q = originXY(a.origin, a.x, a.y);
          inView(q[0], q[1]);
          ops.push('w ' + q[0] + ' ' + q[1] + ' ' + (Number(a.deltaX) || 0) + ' ' + (Number(a.deltaY) || 0));
        }
      });
      ops.push('t ' + dur);
      return ops.join('\n');
    };
  }

  I.action_sequence = function (actions, context) {
    if (!Array.isArray(actions)) return Promise.reject(new Error('invalid argument: actions'));
    var n = 0;
    actions.forEach(function (s) { n = Math.max(n, (s.actions || []).length); });
    var ticks = [];
    for (var i = 0; i < n; i++) ticks.push(actionTick(actions, i));
    // WebDriver releases what the sequence left pressed only on "release actions"; nothing here.
    return push(ticks);
  };
  I.click = function (element, coords) {
    var x = coords.x, y = coords.y;
    return push([
      function () { pointer.x = x; pointer.y = y; return 'm ' + x + ' ' + y + '\nt 0'; },
      function () { return 'd 0\nt 0'; },
      function () { return 'u 0\nt 0'; },
    ]);
  };
  // WebDriver Element Send Keys: focus the element (caret to the end of a text control), then
  // type each character: modifiers stay down until the end, the rest are pressed and released.
  var MODS = { 0xE008: 1, 0xE009: 1, 0xE00A: 1, 0xE03D: 1, 0xE050: 1, 0xE051: 1, 0xE052: 1, 0xE053: 1 };
  I.send_keys = function (element, keys) {
    var ticks = [function () { return 'f ' + element.__id + '\ne ' + element.__id + '\nt 0'; }];
    var held = [];
    var s = String(keys);
    for (var i = 0; i < s.length; i++) {
      var c = s.codePointAt(i);
      if (c > 0xffff) i++;
      (function (c) {
        if (c === 0xE000) { // NULL: release the modifiers
          var h = held; held = [];
          ticks.push(function () { return h.map(function (m) { return 'ku ' + m; }).join('\n') + '\nt 0'; });
        } else if (MODS[c]) {
          held.push(c);
          ticks.push(function () { return 'kd ' + c + '\nt 0'; });
        } else {
          ticks.push(function () { return 'kd ' + c + '\nku ' + c + '\nt 0'; });
        }
      })(c);
    }
    if (held.length) ticks.push(function () { return held.map(function (m) { return 'ku ' + m; }).join('\n') + '\nt 0'; });
    return push(ticks);
  };

  // Accessibility: a small subset of computed role / accessible name (enough for plain markup).
  var ROLES = { a: function (e) { return e.hasAttribute('href') ? 'link' : 'generic'; }, article: 'article', aside: 'complementary',
    button: 'button', dialog: 'dialog', footer: 'contentinfo', form: 'form', h1: 'heading', h2: 'heading', h3: 'heading',
    h4: 'heading', h5: 'heading', h6: 'heading', header: 'banner', hr: 'separator', img: function (e) { return e.getAttribute('alt') === '' ? 'presentation' : 'img'; },
    li: 'listitem', main: 'main', nav: 'navigation', ol: 'list', option: 'option', p: 'paragraph', progress: 'progressbar',
    section: 'region', select: 'combobox', table: 'table', tbody: 'rowgroup', td: 'cell', textarea: 'textbox', th: 'columnheader',
    thead: 'rowgroup', tr: 'row', ul: 'list', div: 'generic', span: 'generic', slot: '',
    input: function (e) {
      var t = (e.getAttribute('type') || 'text').toLowerCase();
      return { checkbox: 'checkbox', radio: 'radio', button: 'button', submit: 'button', reset: 'button', image: 'button',
        range: 'slider', number: 'spinbutton', search: 'searchbox', hidden: '' }[t] || 'textbox';
    } };
  I.get_computed_role = function (el) {
    var r = el.getAttribute && el.getAttribute('role');
    if (r) return Promise.resolve(r.trim().split(/\s+/)[0]);
    var f = ROLES[el.localName];
    return Promise.resolve(typeof f === 'function' ? f(el) : (f || ''));
  };
  function textOf(n) {
    if (n.nodeType === 3) return n.data;
    if (n.nodeType !== 1) return '';
    if (n.localName === 'img') return n.getAttribute('alt') || '';
    var s = '';
    var kids = n.shadowRoot ? n.shadowRoot.childNodes : n.childNodes;
    for (var i = 0; i < kids.length; i++) s += textOf(kids[i]);
    return s;
  }
  function nameOf(el) {
    var by = el.getAttribute('aria-labelledby');
    if (by) {
      var parts = by.trim().split(/\s+/).map(function (id) { var t = el.ownerDocument.getElementById(id); return t ? textOf(t) : ''; });
      return parts.join(' ');
    }
    var al = el.getAttribute('aria-label');
    if (al && al.trim()) return al;
    if (el.labels && el.labels.length) return Array.prototype.map.call(el.labels, textOf).join(' ');
    if (el.localName === 'img') return el.getAttribute('alt') || el.getAttribute('title') || '';
    var role = el.getAttribute('role') || (typeof ROLES[el.localName] === 'function' ? ROLES[el.localName](el) : ROLES[el.localName]);
    if (/^(button|link|heading|cell|columnheader|option|listitem|checkbox|radio)$/.test(role || '')) return textOf(el);
    return el.getAttribute('title') || '';
  }
  I.get_computed_label = function (el) {
    return Promise.resolve(nameOf(el).replace(/\s+/g, ' ').trim());
  };

  // Called by the runner when the page is idle: the next tick's ops, or '' when nothing is queued.
  // A job whose ticks are used up is resolved here; a tick that throws rejects its job.
  Object.defineProperty(window, '__scratchy_td_next', { value: function () {
    for (;;) {
      if (!cur) { cur = queue.shift() || null; if (!cur) return ''; }
      if (cur.i >= cur.ticks.length) { var done = cur; cur = null; done.resolve(); continue; }
      try {
        return cur.ticks[cur.i++]();
      } catch (e) {
        var bad = cur; cur = null; bad.reject(e);
      }
    }
  } });
  // UA default action of an uncancelled wheel event at element ID: scroll the nearest scroll
  // container that can move in that direction, else the viewport.
  Object.defineProperty(window, '__scratchy_td_wheel', { value: function (el, dx, dy) {
    for (var n = el; n && n.nodeType === 1 && n !== document.documentElement && n !== document.body; n = n.parentNode) {
      var cs = getComputedStyle(n);
      var sy = /auto|scroll/.test(cs.overflowY) && n.scrollHeight > n.clientHeight;
      var sx = /auto|scroll/.test(cs.overflowX) && n.scrollWidth > n.clientWidth;
      if ((dy && sy) || (dx && sx)) { n.scrollBy(sx ? dx : 0, sy ? dy : 0); return; }
    }
    window.scrollBy(dx, dy);
  } });
})();
