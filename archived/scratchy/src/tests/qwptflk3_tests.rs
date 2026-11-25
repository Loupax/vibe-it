// QWPTFLK3: a custom element constructor that inserts its own element must not re-upgrade it (DOM "upgrade an
// element" steps 1 and 4), and a live collection whose producer throws must not cache stale ids as current.
#[allow(unused_imports)]
use super::*;

/// define() upgrades two parsed customized built-ins whose constructors append them to a container (the
/// WPT custom-elements/parser/parser-sets-attributes-after-built-in-constructor.html pattern). Each constructor
/// runs once (before: the insert hook upgraded the element again from inside its own constructor, recursing until
/// V8's stack overflowed), nothing is reported as an error, and the live getElementsByClassName read in
/// attributeChangedCallback sees the class element.
#[test]
fn qwptflk3_constructor_inserting_itself_is_not_upgraded_again() {
    let log = run_in_html(
        r#"<html><body><div id=c></div><div class="pc" is="class-div"></div><div class="" is="class-div"></div></body></html>"#,
        r#"const c = document.getElementById('c'), m = document.getElementsByClassName('pc');
         let ctors = 0, conns = 0, attrs = 0;
         customElements.define('class-div', class extends HTMLDivElement {
           static observedAttributes = ['class'];
           constructor() { super(); ctors++; c.append(this); m.length; }
           connectedCallback() { conns++; }
           attributeChangedCallback() { attrs++; this.seen = Array.from(m); }
         }, { extends: 'div' });
         const [a, b] = document.querySelectorAll('[is=class-div]');
         console.log(ctors, conns, attrs, c.children.length, a.className, b.className);
         console.log(a.seen.length, a.seen[0] === a, b.seen.length, b.seen[0] === a);"#,
    );
    assert_eq!(log, ["log:2 2 2 2 pc ", "log:1 true 1 true"], "{log:?}");
}

/// Same for document.createElement (the synchronous path): the constructor runs once, no error.
#[test]
fn qwptflk3_created_element_inserting_itself_constructs_once() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"let ctors = 0;
         customElements.define('x-self', class extends HTMLElement {
           constructor() { super(); ctors++; document.body.append(this); }
         });
         const e = document.createElement('x-self');
         console.log(ctors, e.parentNode === document.body, document.body.children.length);"#,
    );
    assert_eq!(log, ["log:1 true 1"], "{log:?}");
}

/// A live collection read that overflows the stack inside its producer, then is read again in a shallower
/// frame, recomputes there (before: the generation was recorded before the producer ran, so the shallower read
/// and every later one in that generation returned the old ids).
#[test]
fn qwptflk3_live_collection_producer_throw_is_not_cached() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"const m = document.getElementsByClassName('k');
         console.log(m.length);
         const p = document.createElement('p'); p.className = 'k'; document.body.appendChild(p);
         let inner = -1;
         function r() { try { return r(); } catch (e) { try { inner = m.length; } catch (e2) { throw e2; } return inner; } }
         try { r(); } catch (e) {}
         console.log(m.length, inner);"#,
    );
    assert_eq!(log, ["log:0", "log:1 1"], "{log:?}");
}
