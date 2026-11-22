// QCE1: elements the HTML parser has not reached when customElements.define() runs in a parser script are
// created like "create an element for a token" with the synchronous custom elements flag: constructor first,
// then the attributes appended one by one (records, attributeChangedCallback), then inserted where parsed.
#[allow(unused_imports)]
use super::*;

/// The constructor sees no attributes; they arrive afterwards in source order, each as an attributeChangedCallback
/// and a mutation record on an observed container the constructor moved the element into.
#[test]
fn qce1_builtin_attributes_are_appended_after_the_constructor() {
    let log = qa13_run(
        r#"<html><body><div id=c></div>
        <script>
        const c = document.getElementById('c'), recs = [], seen = [];
        new MutationObserver(r => recs.push(...r)).observe(c, { subtree: true, attributes: true, attributeOldValue: true });
        customElements.define('x-d', class extends HTMLDivElement {
          static observedAttributes = ['class', 'title'];
          constructor() { super(); seen.push('ctor:' + this.attributes.length); c.append(this); }
          attributeChangedCallback(n, o, v) { seen.push(n + '=' + v + ':' + (this.parentNode === c)); }
          connectedCallback() { seen.push('conn'); }
        }, { extends: 'div' });
        </script>
        <div class="a" title="t" data-x="1" is="x-d"></div>
        <script>
        const e = document.querySelector('[is=x-d]');
        recs.push(...new MutationObserver(() => {}).takeRecords());
        console.log(seen.join(' '));
        console.log(e.parentNode === document.body, e.getAttribute('class'), e.getAttribute('data-x'));
        </script></body></html>"#,
    );
    assert_eq!(log, ["log:ctor:0 class=a:true title=t:true conn", "log:true a 1"], "{log:?}");
}

/// Mutation records for the parser-appended attributes reach an observer on the container, in attribute order.
#[test]
fn qce1_parser_attributes_queue_mutation_records() {
    let log = qa13_run(
        r#"<html><body><div id=c></div>
        <script>
        const c = document.getElementById('c'), recs = [];
        const mo = new MutationObserver(r => recs.push(...r));
        mo.observe(c, { subtree: true, attributes: true, attributeOldValue: true });
        customElements.define('x-d', class extends HTMLDivElement {
          constructor() { super(); c.append(this); }
        }, { extends: 'div' });
        </script>
        <div class="a" title="t" is="x-d"></div>
        <script>
        recs.push(...mo.takeRecords());
        console.log(recs.map(r => r.attributeName + ':' + r.oldValue).join(' '));
        </script></body></html>"#,
    );
    assert_eq!(log, ["log:class:null title:null is:null"], "{log:?}");
}

/// document.anchors sees a name attribute the parser appended after the constructor moved the element,
/// and the parser then inserts each element where it was parsed (the second anchor precedes the first).
#[test]
fn qce1_anchors_reflect_parser_appended_name() {
    let log = qa13_run(
        r#"<html><body><div id=c></div>
        <script>
        const c = document.getElementById('c');
        customElements.define('x-a', class extends HTMLAnchorElement {
          static observedAttributes = ['name'];
          constructor() { super(); c.append(this); document.anchors.length; }
          attributeChangedCallback() { this.at = Array.from(document.anchors); }
        }, { extends: 'a' });
        </script>
        <a is="x-a" name="one"></a><a is="x-a" name=""></a>
        <script>
        const [a, b] = document.querySelectorAll('[is=x-a]');
        console.log(a.at.length, a.at[0] === a, b.at.length, b.at[0] === b, b.at[1] === a);
        </script></body></html>"#,
    );
    assert_eq!(log, ["log:1 true 2 true true"], "{log:?}");
}

/// Elements before the defining script are upgraded as before (attributes already there, callbacks after the
/// constructor); elements after it are constructed bare and get their attributes afterwards.
#[test]
fn qce1_earlier_elements_keep_upgrade_behaviour() {
    let log = qa13_run(
        r#"<html><body><p is="x-p" id="p1"></p>
        <script>
        const seen = [];
        customElements.define('x-p', class extends HTMLParagraphElement {
          static observedAttributes = ['id'];
          constructor() { super(); seen.push('ctor:' + this.attributes.length); }
          attributeChangedCallback(n, o, v) { seen.push(n + '=' + v); }
        }, { extends: 'p' });
        </script>
        <p is="x-p" id="p2"></p>
        <script>console.log(seen.join(' '));</script></body></html>"#,
    );
    assert_eq!(log, ["log:ctor:2 id=p1 ctor:0 id=p2"], "{log:?}");
}
