// C-AUTOFOCUS (KEYS.md T29, T32, `gi`): run by the renderer in every frame's main world before any
// page script (renderer.rs step 4, field.rs). It remembers the document's last focused text field and
// returns the function the host calls with "blur", "last" or "first"; only the renderer holds that
// function, so pages cannot call it. It returns whether a field was blurred / has the focus now.
(function () {
  var last = null;
  var types = /^(text|search|url|tel|email|password|number)$/i;
  // A text field `gi` may focus: not disabled or read-only.
  function textField(el) {
    if (!el || el.nodeType !== 1) return false;
    if (el.nodeName === "INPUT") return types.test(el.type) && !el.disabled && !el.readOnly;
    if (el.nodeName === "TEXTAREA") return !el.disabled && !el.readOnly;
    return el.isContentEditable === true;
  }
  function rendered(el) {
    if (!el.isConnected || !el.getClientRects().length) return false;
    return !el.checkVisibility || el.checkVisibility({ visibilityProperty: true });
  }
  function inView(el) {
    var r = el.getBoundingClientRect();
    return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  }
  // The focused element, through open shadow roots.
  function active() {
    var a = document.activeElement;
    while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement;
    return a;
  }
  function focus(el) {
    el.focus();
    return active() === el;
  }
  // The focused text field is the last one. Also read when the host calls: a page whose window does
  // not have the keyboard focus yet (the first page, before any click) moves its focus without focus
  // events, so `autofocus` there never reaches the listener below.
  function note() {
    var a = active();
    if (textField(a)) last = a;
    return a;
  }
  // Installed before any page script: the first capture listener on window, so a page cannot stop it.
  addEventListener("focusin", function (e) {
    var t = e.composedPath ? e.composedPath()[0] : e.target;
    if (textField(t)) last = t;
  }, true);
  return function (op) {
    try {
      var a = note();
      if (op === "blur") {
        if (!a || !(a.nodeName === "INPUT" || a.nodeName === "TEXTAREA" || a.isContentEditable)) return false;
        a.blur();
        return true;
      }
      if (op === "last") return !!last && textField(last) && rendered(last) && focus(last);
      if (op === "first") {
        var all = document.querySelectorAll("input, textarea, [contenteditable]");
        var first = null;
        for (var i = 0; i < all.length; i++) {
          var el = all[i];
          if (!textField(el) || !rendered(el)) continue;
          if (inView(el)) return focus(el);
          if (!first) first = el;
        }
        return !!first && focus(first);
      }
    } catch (e) {}
    return false;
  };
})
