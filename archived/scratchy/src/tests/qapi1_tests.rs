//! QAPI1: `window.trustedTypes` (Trusted Types without enforcement).
use super::*;

fn js(code: &str) -> Vec<String> {
    run_in_html("<html><body><div id=d></div></body></html>", code)
}

#[test]
fn qapi1_surface_and_defaults() {
    let log = js("const t = window.trustedTypes; \
         console.log(typeof t.createPolicy, t.defaultPolicy, t.getAttributeType('script', 'src'), t.getPropertyType('div', 'innerHTML'), \
           String(t.emptyHTML) === '', String(t.emptyScript) === '', t.isHTML(t.emptyHTML), t.isScript(t.emptyHTML), t.isScriptURL('x'), \
           'trustedTypes' in window, typeof TrustedHTML, typeof TrustedTypePolicyFactory);");
    assert_eq!(log, ["log:function null null null true true true false false true function function"], "{log:?}");
}

#[test]
fn qapi1_policy_runs_callbacks() {
    let log = js("const p = trustedTypes.createPolicy('p', { createHTML: (s, a) => s + '!' + a, createScript: s => s.toUpperCase() }); \
         const h = p.createHTML('<b>x</b>', 'y'), s = p.createScript('abc'), u = p.createScriptURL('/a.js'); \
         console.log(p.name, String(h), h.toString(), JSON.stringify(h), `${s}`, `${u}`, \
           trustedTypes.isHTML(h), trustedTypes.isScript(h), trustedTypes.isScript(s), trustedTypes.isScriptURL(u), trustedTypes.isScriptURL(s), \
           h instanceof TrustedHTML, Object.prototype.toString.call(u));");
    assert_eq!(
        log,
        ["log:p <b>x</b>!y <b>x</b>!y \"<b>x</b>!y\" ABC /a.js true false true true false true [object TrustedScriptURL]"],
        "{log:?}"
    );
}

#[test]
fn qapi1_errors() {
    let log = js("const r = []; const t = (f) => { try { f(); r.push('ok'); } catch (e) { r.push(e.constructor.name); } }; \
         t(() => new TrustedHTML()); t(() => trustedTypes.createPolicy('x', { createHTML: 5 })); \
         const p = trustedTypes.createPolicy('x'); t(() => p.createHTML()); \
         console.log(r.join(','));");
    assert_eq!(log, ["log:TypeError,TypeError,TypeError"], "{log:?}");
}

#[test]
fn qapi1_sinks_accept_trusted_values() {
    let log = js("const p = trustedTypes.createPolicy('s', { createHTML: s => s, createScriptURL: s => s }); \
         const d = document.getElementById('d'); d.innerHTML = p.createHTML('<i>hi</i>'); \
         const sc = document.createElement('script'); sc.src = p.createScriptURL('/x.js'); \
         d.setAttribute('title', p.createHTML('t')); \
         console.log(d.innerHTML, sc.getAttribute('src'), d.title);");
    assert_eq!(log, ["log:<i>hi</i> /x.js t"], "{log:?}");
}
