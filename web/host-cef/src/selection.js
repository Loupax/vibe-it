// C-YANK (KEYS.md T54, `:yank`): run by the renderer in every frame's main world before any page
// script (renderer.rs step 6, selection.rs). It keeps its own references to what it calls, so a page
// that replaces `window.getSelection` (or the getters below) later does not change the answer, and
// returns the function the host calls; only the renderer holds that function, so pages cannot call it.
// The function returns the frame's selected text: a focused <input>/<textarea>'s selected part
// (nothing for a password field), else the document's selection; "" when there is none.
(function () {
  var getSelection = window.getSelection;
  var toString = String;
  var slice = String.prototype.slice;
  var call = Function.prototype.call;
  function getter(proto, name) {
    var d = Object.getOwnPropertyDescriptor(proto, name);
    return d && d.get;
  }
  function method(proto, name) {
    var d = Object.getOwnPropertyDescriptor(proto, name);
    return d && d.value;
  }
  var activeElement = getter(Document.prototype, "activeElement");
  var shadowRoot = getter(Element.prototype, "shadowRoot");
  var shadowActive = getter(ShadowRoot.prototype, "activeElement");
  var nodeName = getter(Node.prototype, "nodeName");
  var inputType = getter(HTMLInputElement.prototype, "type");
  var inputValue = getter(HTMLInputElement.prototype, "value");
  var inputStart = getter(HTMLInputElement.prototype, "selectionStart");
  var inputEnd = getter(HTMLInputElement.prototype, "selectionEnd");
  var areaValue = getter(HTMLTextAreaElement.prototype, "value");
  var areaStart = getter(HTMLTextAreaElement.prototype, "selectionStart");
  var areaEnd = getter(HTMLTextAreaElement.prototype, "selectionEnd");
  var selToString = method(Selection.prototype, "toString");
  var doc = document;
  var win = window;
  // `f.call(self, ...)` through the saved Function.prototype.call.
  function run(f, self, a, b) {
    return call.call(f, self, a, b);
  }
  // The focused element, through open shadow roots.
  function active() {
    var a = run(activeElement, doc);
    for (;;) {
      var root = a && run(shadowRoot, a);
      var inner = root && run(shadowActive, root);
      if (!inner) return a;
      a = inner;
    }
  }
  // A field's selected part: null when it has no selection (or its type has none).
  function fieldPart(value, start, end, el) {
    var s, e;
    try {
      s = run(start, el);
      e = run(end, el);
    } catch (x) {
      return null;
    }
    if (s === null || e === null || !(s < e)) return null;
    return run(slice, run(value, el), s, e);
  }
  return function () {
    try {
      var a = active();
      var name = a ? run(nodeName, a) : "";
      if (name === "INPUT") {
        if (toString(run(inputType, a)).toLowerCase() === "password") return "";
        var p = fieldPart(inputValue, inputStart, inputEnd, a);
        if (p !== null) return p;
      } else if (name === "TEXTAREA") {
        var q = fieldPart(areaValue, areaStart, areaEnd, a);
        if (q !== null) return q;
      }
      var sel = run(getSelection, win);
      return sel ? toString(run(selToString, sel)) : "";
    } catch (e) {}
    return "";
  };
})
