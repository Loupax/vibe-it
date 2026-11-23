// C5b (KEYS.md T23): put the selection, collapsed, at the first text at or below the top of the
// screen, so the engine's find, which starts from the selection, makes the first match at or
// below the top of the screen current. Called with (query, match_case). It tells the host with
// a console line `web-host.find-start:<0|1>`; 1 = no match at or below the top of the screen,
// so the find wraps to the first match (the selection is then put at the document start).
// 0 also when the page has no match this walk can see.
// Text nodes and the values of text fields are looked at, in document order.
(function (q, mc) {
  var tell = function (wrapped) { console.log("web-host.find-start:" + (wrapped ? 1 : 0)); };
  try {
    var root = document.body || document.documentElement;
    var norm = function (s) { return mc ? s : s.toLowerCase(); };
    var nq = norm(q);
    var skip = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1 };
    var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    var r = document.createRange();
    var start = null, hit = false, before = false, n;   // start: the collapsed selection to make, [node, offset]
    while ((n = walker.nextNode())) {
      var t, rs, field = n.nodeType === 1;
      if (field) {
        if (n.nodeName !== "TEXTAREA" && !(n.nodeName === "INPUT" && /^(text|search|url|tel|email|)$/i.test(n.type))) continue;
        t = n.value || "";
        rs = n.getClientRects();
      } else {
        t = n.nodeValue;
        if (skip[n.parentNode.nodeName] || n.parentNode.nodeName === "TEXTAREA") continue;
        r.selectNodeContents(n);
        rs = r.getClientRects();
      }
      if (!/\S/.test(t) || !rs.length) continue;      // empty, or not rendered
      var from = 0;
      if (!start) {
        if (rs[rs.length - 1].top < 0) {              // every line of it is above the screen
          if (norm(t).indexOf(nq) >= 0) before = true;
          continue;
        }
        if (field) {
          start = [n.parentNode, Array.prototype.indexOf.call(n.parentNode.childNodes, n)];
        } else {
          if (rs[0].top < 0) {                        // some lines above: the first offset on a line at/below
            var lo = 0, hi = t.length - 1;
            while (lo < hi) {
              var mid = (lo + hi) >> 1;
              r.setStart(n, mid); r.setEnd(n, mid + 1);
              var b = r.getClientRects()[0];
              if (!b || b.top >= 0) hi = mid; else lo = mid + 1;
            }
            from = lo;
          }
          start = [n, from];
        }
      }
      if (norm(from ? t.slice(from) : t).indexOf(nq) >= 0) { hit = true; break; }
    }
    var sel = window.getSelection();
    if (start && hit) {
      sel.collapse(start[0], start[1]);
      tell(false);
    } else {
      sel.collapse(root, 0);
      // Wrapped when a match was seen above the screen and none at or below it. (A match the
      // walk cannot see, e.g. split over elements, is no reason to claim a wrap.)
      tell(!hit && before);
    }
  } catch (e) {
    tell(false);
  }
})
