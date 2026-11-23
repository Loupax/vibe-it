// QEDIT-B: DataTransfer, DataTransferItemList, DataTransferItem and FileList (HTML "drag data store"), plain JS.
// Data store only: the App has no drag-and-drop UI, and nothing here is wired to real drags. A script-made
// `new DataTransfer()` is a read/write store. `__dt_make(mode)` (hidden, non-enumerable) builds one in the
// "readwrite", "readonly" or "protected" mode for a host that fires drag events. Evaluated after the fetch
// prelude (File) and the events in the base prelude (DragEvent takes a DataTransfer or null).
(function (g) {
  'use strict';
  var stores = new WeakMap();   // DataTransfer -> store
  var lists = new WeakMap();    // DataTransferItemList -> store
  var items = new WeakMap();    // DataTransferItem -> entry
  var fileLists = new WeakMap();
  function meth(o, name, fn) { Object.defineProperty(o, name, { value: fn, writable: true, configurable: true, enumerable: true }); }
  function getter(o, name, get, set) { Object.defineProperty(o, name, { get: get, set: set, configurable: true, enumerable: true }); }
  function tag(C, name) { Object.defineProperty(C.prototype, Symbol.toStringTag, { value: name, configurable: true }); }
  function illegal() { return new TypeError('Illegal invocation'); }
  function ascii(s) { return String(s).replace(/[A-Z]/g, function (c) { return c.toLowerCase(); }); }
  function needArgs(n, got, name, iface) {
    if (got < n) throw new TypeError("Failed to execute '" + name + "' on '" + iface + "': " + n + ' argument' + (n > 1 ? 's' : '') + ' required, but only ' + got + ' present.');
  }
  function isFile(v) { return typeof g.File === 'function' && v instanceof g.File; }

  // ---- FileList ----
  function FileList() { throw new TypeError('Illegal constructor'); }
  getter(FileList.prototype, 'length', function () { var f = fileLists.get(this); if (!f) throw illegal(); return f.length; });
  meth(FileList.prototype, 'item', function item(i) {
    var f = fileLists.get(this); if (!f) throw illegal();
    needArgs(1, arguments.length, 'item', 'FileList');
    i = Number(i) >>> 0;
    return i < f.length ? f[i] : null;
  });
  Object.defineProperty(FileList.prototype, Symbol.iterator, { value: Array.prototype[Symbol.iterator], writable: true, configurable: true });
  tag(FileList, 'FileList');
  function makeFileList(arr) {
    var l = Object.create(FileList.prototype);
    fileLists.set(l, arr);
    arr.forEach(function (f, i) { Object.defineProperty(l, i, { value: f, enumerable: true, configurable: true, writable: false }); });
    return l;
  }

  // ---- the store: { mode, entries: [{kind, type, data|file, live, item, store}], typesCache, filesCache, list } ----
  var listLen = new WeakMap();
  function newStore(mode) {
    return { mode: mode, entries: [], typesCache: null, filesCache: null, list: null, effectAllowed: 'none', dropEffect: 'none' };
  }
  function changed(s) { s.typesCache = null; s.filesCache = null; syncList(s); }
  function disable(e) { e.live = false; }
  // The list's own index properties mirror the entries (enumerable, configurable, non-writable).
  function syncList(s) {
    var l = s.list; if (!l) return;
    var n = s.entries.length, old = listLen.get(l) || 0, i;
    for (i = n; i < old; i++) delete l[i];
    for (i = 0; i < n; i++) {
      var it = s.entries[i].item;
      if (!Object.prototype.hasOwnProperty.call(l, i) || l[i] !== it) {
        delete l[i];
        Object.defineProperty(l, i, { value: it, enumerable: true, configurable: true, writable: false });
      }
    }
    listLen.set(l, n);
  }
  function addEntry(s, e) {
    e.live = true; e.store = s;
    e.item = Object.create(DataTransferItem.prototype); items.set(e.item, e);
    s.entries.push(e); changed(s);
    return e.item;
  }
  function findString(s, type) {
    for (var i = 0; i < s.entries.length; i++) if (s.entries[i].kind === 'string' && s.entries[i].type === type) return i;
    return -1;
  }
  function removeAt(s, i) { var e = s.entries.splice(i, 1)[0]; disable(e); changed(s); }
  function rw(s) { return s.mode === 'readwrite'; }

  // ---- DataTransferItem ----
  function DataTransferItem() { throw new TypeError('Illegal constructor'); }
  function itemEntry(it) { var e = items.get(it); if (!e) throw illegal(); return e; }
  getter(DataTransferItem.prototype, 'kind', function () { var e = itemEntry(this); return e.live ? e.kind : ''; });
  getter(DataTransferItem.prototype, 'type', function () { var e = itemEntry(this); return e.live ? e.type : ''; });
  meth(DataTransferItem.prototype, 'getAsString', function getAsString(cb) {
    var e = itemEntry(this);
    needArgs(1, arguments.length, 'getAsString', 'DataTransferItem');
    if (cb !== null && typeof cb !== 'function') throw new TypeError("Failed to execute 'getAsString' on 'DataTransferItem': The callback provided as parameter 1 is not a function.");
    if (cb === null || !e.live || e.kind !== 'string' || e.store.mode === 'protected') return;
    var d = e.data;
    g.setTimeout(function () { cb.call(undefined, d); }, 0);
  });
  meth(DataTransferItem.prototype, 'getAsFile', function getAsFile() {
    var e = itemEntry(this);
    if (!e.live || e.kind !== 'file' || e.store.mode === 'protected') return null;
    return e.file;
  });
  tag(DataTransferItem, 'DataTransferItem');

  // ---- DataTransferItemList ----
  function DataTransferItemList() { throw new TypeError('Illegal constructor'); }
  function listStore(l) { var s = lists.get(l); if (!s) throw illegal(); return s; }
  getter(DataTransferItemList.prototype, 'length', function () { return listStore(this).entries.length; });
  meth(DataTransferItemList.prototype, 'add', function add(data, type) {
    var s = listStore(this);
    needArgs(1, arguments.length, 'add', 'DataTransferItemList');
    if (!rw(s)) return null;
    if (isFile(data) && arguments.length < 2) return addEntry(s, { kind: 'file', type: data.type, file: data });
    if (arguments.length < 2) throw new TypeError("Failed to execute 'add' on 'DataTransferItemList': parameter 1 is not of type 'File'.");
    var t = ascii(type);
    if (findString(s, t) >= 0) throw new g.DOMException("Failed to execute 'add' on 'DataTransferItemList': An item already exists for type '" + t + "'.", 'NotSupportedError');
    return addEntry(s, { kind: 'string', type: t, data: String(data) });
  });
  meth(DataTransferItemList.prototype, 'remove', function remove(index) {
    var s = listStore(this);
    needArgs(1, arguments.length, 'remove', 'DataTransferItemList');
    if (!rw(s)) throw new g.DOMException("Failed to execute 'remove' on 'DataTransferItemList': The list is not writable.", 'InvalidStateError');
    var i = Number(index);
    i = isFinite(i) ? Math.trunc(i) : 0;
    i = ((i % 4294967296) + 4294967296) % 4294967296;
    if (i < s.entries.length) removeAt(s, i);
  });
  meth(DataTransferItemList.prototype, 'clear', function clear() {
    var s = listStore(this);
    if (!rw(s) || !s.entries.length) return;
    s.entries.forEach(disable); s.entries = []; changed(s);
  });
  Object.defineProperty(DataTransferItemList.prototype, Symbol.iterator, { value: Array.prototype[Symbol.iterator], writable: true, configurable: true });
  tag(DataTransferItemList, 'DataTransferItemList');

  // ---- DataTransfer ----
  var effectsAllowed = { none: 1, copy: 1, copyLink: 1, copyMove: 1, link: 1, linkMove: 1, move: 1, all: 1, uninitialized: 1 };
  var dropEffects = { none: 1, copy: 1, link: 1, move: 1 };
  function DataTransfer() {
    if (!new.target) throw new TypeError("Failed to construct 'DataTransfer': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    stores.set(this, newStore('readwrite'));
  }
  function S(dt) { var s = stores.get(dt); if (!s) throw illegal(); return s; }
  getter(DataTransfer.prototype, 'dropEffect', function () { return S(this).dropEffect; }, function (v) {
    var s = S(this); v = String(v); if (dropEffects.hasOwnProperty(v)) s.dropEffect = v;
  });
  getter(DataTransfer.prototype, 'effectAllowed', function () { return S(this).effectAllowed; }, function (v) {
    var s = S(this); v = String(v);
    if (s.mode === 'readwrite' && effectsAllowed.hasOwnProperty(v)) s.effectAllowed = v;
  });
  getter(DataTransfer.prototype, 'items', function () {
    var s = S(this);
    if (!s.list) { var l = Object.create(DataTransferItemList.prototype); lists.set(l, s); s.list = l; syncList(s); }
    return s.list;
  });
  getter(DataTransfer.prototype, 'types', function () {
    var s = S(this);
    if (!s.typesCache) {
      var t = [], f = false;
      s.entries.forEach(function (e) { if (e.kind === 'string') t.push(e.type); else f = true; });
      if (f) t.push('Files');
      s.typesCache = Object.freeze(t);
    }
    return s.typesCache;
  });
  getter(DataTransfer.prototype, 'files', function () {
    var s = S(this);
    if (!s.filesCache) {
      var a = [];
      if (s.mode !== 'protected') s.entries.forEach(function (e) { if (e.kind === 'file') a.push(e.file); });
      s.filesCache = makeFileList(a);
    }
    return s.filesCache;
  });
  // text -> text/plain, url -> text/uri-list (the format matched ASCII case-insensitively).
  function alias(f) { f = ascii(f); return f === 'text' ? 'text/plain' : f === 'url' ? 'text/uri-list' : f; }
  meth(DataTransfer.prototype, 'getData', function getData(format) {
    var s = S(this);
    needArgs(1, arguments.length, 'getData', 'DataTransfer');
    if (s.mode === 'protected') return '';
    var f = ascii(String(format).replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, ''));
    var conv = f === 'url';
    f = alias(f);
    var i = findString(s, f);
    // A MIME parameter on the request ("text/uri-list;charset=utf-8") falls back to the bare type.
    if (i < 0 && f.indexOf(';') > 0) i = findString(s, f.slice(0, f.indexOf(';')));
    if (i < 0) return '';
    var d = s.entries[i].data;
    if (conv) {
      var lines = d.split(/\r\n|\n|\r/);
      for (var k = 0; k < lines.length; k++) if (lines[k] !== '' && lines[k][0] !== '#') return lines[k];
      return '';
    }
    return d;
  });
  meth(DataTransfer.prototype, 'setData', function setData(format, data) {
    var s = S(this);
    needArgs(2, arguments.length, 'setData', 'DataTransfer');
    if (!rw(s)) return;
    var f = alias(String(format)), d = String(data);
    var i = findString(s, f);
    if (i >= 0) { disable(s.entries[i]); s.entries.splice(i, 1); }
    addEntry(s, { kind: 'string', type: f, data: d });
  });
  meth(DataTransfer.prototype, 'clearData', function clearData(format) {
    var s = S(this);
    if (!rw(s)) return;
    if (format === undefined) {
      var keep = s.entries.filter(function (e) { return e.kind === 'file'; });
      if (keep.length === s.entries.length) return;
      s.entries.forEach(function (e) { if (e.kind === 'string') disable(e); });
      s.entries = keep; changed(s); return;
    }
    var i = findString(s, alias(String(format)));
    if (i >= 0) removeAt(s, i);
  });
  meth(DataTransfer.prototype, 'setDragImage', function setDragImage(el, x, y) {
    S(this);
    needArgs(3, arguments.length, 'setDragImage', 'DataTransfer');
    if (!(el instanceof g.Element)) throw new TypeError("Failed to execute 'setDragImage' on 'DataTransfer': parameter 1 is not of type 'Element'.");
  });
  tag(DataTransfer, 'DataTransfer');

  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  def(g, 'DataTransfer', DataTransfer); def(g, 'DataTransferItemList', DataTransferItemList);
  def(g, 'DataTransferItem', DataTransferItem); def(g, 'FileList', FileList);
  // Host hook: a store in the given mode ("readwrite" | "readonly" | "protected").
  def(g, '__dt_make', function (mode) { var d = new DataTransfer(); S(d).mode = mode; return d; });
})(globalThis);
