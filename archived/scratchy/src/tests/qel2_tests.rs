// QEL2: ResizeObserver with real sizes, depth loop, loop error.
#[allow(unused_imports)]
use super::*;

/// QEL2: `run_html_with_timers` plus the shorthand `$`.
fn ro_run(html: &str, code: &str) -> Vec<String> {
    run_html_with_timers(html, &format!("const $ = id => document.getElementById(id);\n{code}"))
}

const RO_HTML: &str = "<html><body style='margin:0'><div id=a style='width:100px;height:50px;padding:5px 10px;border:2px solid'></div>\
    <div id=hid style='display:none'><p id=inner>x</p></div><span id=sp>inline</span><div id=z></div></body></html>";

#[test]
fn qel2_resize_observer_real_sizes_in_every_box() {
    let log = ro_run(
        RO_HTML,
        r#"const f = (e, n) => n.inlineSize + 'x' + n.blockSize;
           const ro = new ResizeObserver(function (es, o) {
             console.log('cb', es.length, this === ro, o === ro);
             es.forEach(e => console.log(e.target.id, e.contentRect.x, e.contentRect.y, e.contentRect.width, e.contentRect.height,
               f(e, e.contentBoxSize[0]), f(e, e.borderBoxSize[0]), f(e, e.devicePixelContentBoxSize[0]), e instanceof ResizeObserverEntry));
           });
           ro.observe($('a')); ro.observe($('hid')); ro.observe($('sp'));
           try { ro.observe($('a'), { box: 'bogus' }); } catch (e) { console.log(e.name); }
           try { new ResizeObserver(); } catch (e) { console.log(e.name); }
           const r2 = new ResizeObserver(() => console.log('NO')); r2.observe($('a')); r2.disconnect();"#,
    );
    // The first observation always reports (last-reported starts at -1x-1), also display:none and
    // non-replaced inline targets, with 0x0 (WPT observe-017).
    assert_eq!(
        log,
        [
            "log:TypeError",
            "log:TypeError",
            "log:cb 3 true true",
            "log:a 10 5 100 50 100x50 124x64 100x50 true",
            "log:hid 0 0 0 0 0x0 0x0 0x0 true",
            "log:sp 0 0 0 0 0x0 0x0 0x0 true"
        ],
        "{log:?}"
    );
}

#[test]
fn qel2_resize_observer_box_options_and_changes() {
    let log = ro_run(
        RO_HTML,
        r#"const a = $('a');
           const ro = new ResizeObserver(es => es.forEach(e => console.log(e.target.id, 'content', e.contentBoxSize[0].inlineSize, e.contentBoxSize[0].blockSize)));
           const rb = new ResizeObserver(es => es.forEach(e => console.log(e.target.id, 'border', e.borderBoxSize[0].inlineSize, e.borderBoxSize[0].blockSize)));
           ro.observe(a); rb.observe(a, { box: 'border-box' });
           let step = 0;
           requestAnimationFrame(() => requestAnimationFrame(() => { a.style.padding = '0'; a.style.height = '70px'; }));
           requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => { a.style.padding = '0'; a.style.width = '100px'; console.log('noop'); })));"#,
    );
    assert_eq!(
        log,
        ["log:a content 100 50", "log:a border 124 64", "log:a content 100 70", "log:a border 104 74", "log:noop"],
        "{log:?}"
    );
}

#[test]
fn qel2_resize_observer_unobserve_and_disconnect_deliver_nothing() {
    let log = ro_run(
        RO_HTML,
        r#"const ro = new ResizeObserver(es => console.log('NO', es.length));
           ro.observe($('a')); ro.unobserve($('a'));
           const r2 = new ResizeObserver(es => console.log('NO2')); r2.observe($('a')); r2.disconnect();
           setTimeout(() => console.log('end'), 100);"#,
    );
    assert_eq!(log, ["log:end"], "{log:?}");
}

#[test]
fn qel2_resize_observer_depth_loop_and_loop_error() {
    // A callback that resizes an ancestor of the observed target: the shallower change is skipped
    // (depth <= the depth just delivered) and the loop error fires at window.
    let log = ro_run(
        "<html><body style='margin:0'><div id=outer style='width:100px;height:10px'><div id=inner style='width:20px;height:10px'></div></div></body></html>",
        r#"window.addEventListener('error', e => { console.log('error', e.message); e.preventDefault(); });
           const order = [];
           const ro = new ResizeObserver(es => {
             es.forEach(e => order.push(e.target.id));
             console.log('cb', es.map(e => e.target.id).join(','));
             if (es.some(e => e.target.id === 'inner')) $('outer').style.width = '200px';
           });
           ro.observe($('inner')); ro.observe($('outer'));"#,
    );
    // Depth 0 pass delivers both (inner is deeper but first gather has depth 0: every target is deeper).
    // The callback then changes outer (shallower than inner): skipped, so the loop error is reported.
    assert_eq!(log[0], "log:cb inner,outer", "{log:?}");
    assert!(log.contains(&"log:error ResizeObserver loop completed with undelivered notifications.".to_string()), "{log:?}");
}

#[test]
fn qel2_resize_observer_depth_loop_delivers_deeper_in_next_pass() {
    // A callback that resizes a DEEPER element: delivered in the same update, no error.
    let log = ro_run(
        "<html><body style='margin:0'><div id=outer style='width:100px;height:10px'><div id=inner style='width:20px;height:10px'></div></div></body></html>",
        r#"window.addEventListener('error', e => console.log('error', e.message));
           const ro = new ResizeObserver(es => {
             console.log('cb', es.map(e => e.target.id + ':' + e.contentRect.width).join(','));
             if (es.some(e => e.target.id === 'outer' && e.contentRect.width === 100)) $('inner').style.width = '50px';
           });
           ro.observe($('outer'));
           const r2 = new ResizeObserver(es => console.log('inner', es[0].contentRect.width));
           r2.observe($('inner'));"#,
    );
    assert!(log.iter().all(|l| !l.contains("error")), "{log:?}");
    assert!(log.contains(&"log:inner 20".to_string()) && log.contains(&"log:inner 50".to_string()), "{log:?}");
}


#[test]
fn qel2_resize_observer_target_in_shadow_tree_no_error() {
    // WPT calculate-depth-for-node: observing a node inside a (not rendered) shadow tree must not throw.
    let log = ro_run(
        RO_HTML,
        r#"window.onerror = (m) => console.log('onerror', m);
           const ro = new ResizeObserver(() => {
             const root = $('a').attachShadow({ mode: 'open' });
             const child = root.appendChild(document.createElement('div'));
             new ResizeObserver(es => console.log('shadow', es.length)).observe(child);
             requestAnimationFrame(() => console.log('raf'));
           });
           ro.observe($('a'));"#,
    );
    assert!(log.iter().all(|l| !l.contains("onerror") && !l.contains("ncaught")), "{log:?}");
    assert!(log.contains(&"log:raf".to_string()), "{log:?}");
}
