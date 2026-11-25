// QEL2b: ResizeObserver on the rendering update's layout (ro_geom.rs): App/engine layout boxes, the
// App's wait for its frame layout, size changes with no DOM write, a microtask checkpoint per callback.
#[allow(unused_imports)]
use super::*;

/// A page on the simulated headless clock, with the engine kept so a test can act between turns.
struct Page {
    doc: Handle,
    js: crate::js::JsEngine,
    clock: Rc<std::cell::Cell<f64>>,
    out: Rc<RefCell<Vec<String>>>,
}

impl Page {
    fn new(html: &str, code: &str) -> Page {
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let clock = Rc::new(std::cell::Cell::new(0.0));
        let doc = parse_html(html);
        let js = crate::js::start_scripts_clocked(
            &doc,
            &[src(&format!("const $ = id => document.getElementById(id);\n{code}"))],
            Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
            Some(clock.clone()),
        )
        .unwrap();
        Page { doc, js, clock, out }
    }

    /// Runs tasks and rendering updates until nothing is due (bounded).
    fn settle(&self) {
        for _ in 0..2_000 {
            let Some(due) = crate::event_loop::headless_next_due(&self.js) else { break };
            self.clock.set(self.clock.get().max(due));
            crate::event_loop::headless_turn(&self.js, self.clock.get());
        }
    }

    fn take(&self) -> Vec<String> {
        self.out.take()
    }

    /// The App's frame layout of the DOM as it is now (with `images`), handed to the RO boxes.
    fn app_frame(&self, images: &HashMap<String, Arc<DecodedImage>>) {
        let (vw, vh) = (1024.0, 768.0);
        let tree = build_layout_tree_with_resources(&self.doc, vw, Some(vh), "", images);
        let mut fs = new_font_system();
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let laid = measure_document(&tree, &mut fs, vw, vh, &mut next_id, &mut cache, far, &mut incomplete);
        crate::ro_geom::app_layout(&self.doc, &laid, crate::js_geom::render_stamp(), vw as u32, vh as u32);
    }
}

fn pic() -> HashMap<String, Arc<DecodedImage>> {
    let mut images = HashMap::new();
    images.insert(
        "pic.png".to_string(),
        Arc::new(DecodedImage { width: 40, height: 30, pixels: vec![TextColor::rgb(1, 2, 3); 1200], svg: None, mask: None }),
    );
    images
}

const SZ: &str = "const sz = e => e.target.id + ' ' + e.contentBoxSize[0].inlineSize + 'x' + e.contentBoxSize[0].blockSize;";

#[test]
fn qel2b_microtask_checkpoint_after_each_callback() {
    // HTML "clean up after running script": promise jobs a callback queues run before the next callback.
    let p = Page::new(
        "<html><body><div id=a style='width:10px;height:10px'></div></body></html>",
        r#"const a = $('a');
           new ResizeObserver(() => { console.log('cb1'); Promise.resolve().then(() => console.log('micro1')); }).observe(a);
           new ResizeObserver(() => { console.log('cb2'); Promise.resolve().then(() => console.log('micro2')); }).observe(a);"#,
    );
    p.settle();
    assert_eq!(p.take(), ["log:cb1", "log:micro1", "log:cb2", "log:micro2"]);
}

#[test]
fn qel2b_headless_step6_layout_is_the_one_ro_reads() {
    // The headless rendering update lays out in step 6; step 7 reads that layout's boxes: no layout of
    // its own (the forced `ro-run` layouts before QEL2b).
    let p = Page::new(
        "<html><body style='margin:0'><div id=a style='width:100px;height:50px;padding:5px'></div></body></html>",
        &format!(
            "{SZ}
             const a = $('a');
             new ResizeObserver(es => es.forEach(e => console.log(sz(e)))).observe(a);
             requestAnimationFrame(() => requestAnimationFrame(() => {{ a.style.width = '70px'; }}));"
        ),
    );
    let before = crate::ro_geom::stats();
    p.settle();
    let after = crate::ro_geom::stats();
    assert_eq!(p.take(), ["log:a 100x50", "log:a 70x50"]);
    assert!(after.forced_collects > before.forced_collects, "step 6 recorded the boxes");
    assert_eq!(after.ro_layouts, before.ro_layouts, "step 7 never laid out on its own");
    assert!(after.runs >= before.runs + 2 && after.callbacks == before.callbacks + 2, "two gathers delivered two callbacks");
}

#[test]
fn qel2b_callback_dom_write_relays_out_in_the_depth_loop() {
    // A callback that resizes a deeper target: the spec's loop recalculates layout and delivers it in the
    // same rendering update (a layout inside the RO step, labelled as such).
    let p = Page::new(
        "<html><body style='margin:0'><div id=o style='width:100px;height:20px'><div id=i style='width:20px;height:10px'></div></div></body></html>",
        &format!(
            "{SZ}
             const o = $('o'), i = $('i');
             new ResizeObserver(es => es.forEach(e => {{ console.log(sz(e)); if (e.target === o) i.style.width = '30px'; }})).observe(o);
             new ResizeObserver(es => es.forEach(e => console.log(sz(e)))).observe(i);"
        ),
    );
    let before = crate::ro_geom::stats();
    p.settle();
    let after = crate::ro_geom::stats();
    // Both are delivered in the first pass (sizes gathered before the callbacks); the write makes the
    // deeper target active again and the next pass, deeper than o, reports it.
    assert_eq!(p.take(), ["log:o 100x20", "log:i 20x10", "log:i 30x10"]);
    assert!(after.ro_layouts > before.ro_layouts, "the depth loop laid the callback's write out");
}

#[test]
fn qel2b_size_change_without_dom_write_is_noticed() {
    // An image decode changes a box with no DOM write: the App's next frame layout records the new box,
    // which moves the observation signature (`__d_ro_epoch`) and the observer reports it.
    let p = Page::new(
        "<html><body><img id=im src='pic.png'></body></html>",
        &format!("{SZ} new ResizeObserver(es => es.forEach(e => console.log(sz(e)))).observe($('im'));"),
    );
    p.settle();
    let first = p.take();
    assert_eq!(first.len(), 1, "{first:?}");
    assert_ne!(first[0], "log:im 40x30", "no decoded image yet: {first:?}");
    let e0 = crate::ro_geom::epoch();
    p.app_frame(&pic());
    assert!(crate::ro_geom::epoch() > e0, "a changed target box bumps the epoch");
    p.settle();
    assert_eq!(p.take(), ["log:im 40x30"]);
    // The same layout again changes nothing: no epoch bump, no callback.
    let e1 = crate::ro_geom::epoch();
    p.app_frame(&pic());
    assert_eq!(crate::ro_geom::epoch(), e1);
    p.settle();
    assert!(p.take().is_empty());
}

#[test]
fn qel2b_waits_for_the_app_frame_layout() {
    // While the App's frame layout is still to come the observers wait (no rendering update is owed, so
    // nothing spins), then deliver from that layout with no layout of their own.
    let p = Page::new(
        "<html><body style='margin:0'><div id=a style='width:100px;height:50px'></div></body></html>",
        &format!("{SZ} new ResizeObserver(es => es.forEach(e => console.log(sz(e)))).observe($('a'));"),
    );
    crate::ro_geom::set_wait(true);
    p.settle();
    assert!(p.take().is_empty(), "waiting for the frame layout");
    assert!(crate::event_loop::headless_frame_due(&p.js).is_none(), "a waiting observer asks for no frame");
    let before = crate::ro_geom::stats();
    p.app_frame(&HashMap::new());
    assert!(!crate::ro_geom::waiting(), "the frame layout ends the wait");
    p.settle();
    let after = crate::ro_geom::stats();
    assert_eq!(p.take(), ["log:a 100x50"]);
    assert_eq!(after.ro_layouts, before.ro_layouts, "the App's layout was enough");
}

#[test]
fn qel2b_tracking_follows_observe_unobserve_disconnect() {
    let p = Page::new(
        "<html><body><div id=a></div><div id=b></div></body></html>",
        r#"window.r1 = new ResizeObserver(() => {}); window.r2 = new ResizeObserver(() => {});
           r1.observe($('a')); r2.observe($('a')); r2.observe($('b'));"#,
    );
    assert!(crate::ro_geom::active());
    p.js.eval("t", "r2.unobserve($('b')); r2.disconnect();");
    assert!(crate::ro_geom::active(), "r1 still observes a");
    p.js.eval("t", "r1.unobserve($('a'));");
    assert!(!crate::ro_geom::active(), "nothing observed");
}

#[test]
fn qel2b_app_install_walks_only_when_the_observers_need_it() {
    // A frame for DOM writes is not walked at install (the observation signature moved with the write;
    // step 7 records the shown layout); a frame of the recorded DOM state (an animated or :hover size) is.
    let p = Page::new(
        "<html><body style='margin:0'><div id=a style='width:100px;height:50px'></div></body></html>",
        &format!("{SZ} new ResizeObserver(es => es.forEach(e => console.log(sz(e)))).observe($('a'));"),
    );
    p.settle();
    assert_eq!(p.take(), ["log:a 100x50"]);
    let laid = |doc: &Handle| {
        let tree = build_layout_tree_with_resources(doc, 1024.0, Some(768.0), "", &HashMap::new());
        let (mut fs, mut cache, mut next, mut inc) = (new_font_system(), ShapeCache::new(), 0usize, false);
        let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
        measure_document(&tree, &mut fs, 1024.0, 768.0, &mut next, &mut cache, far, &mut inc)
    };
    let l = laid(&p.doc);
    let c0 = crate::ro_geom::stats().app_collects;
    crate::ro_geom::app_install(&p.doc, &l, crate::js_geom::render_stamp(), 1024, 768);
    assert_eq!(crate::ro_geom::stats().app_collects, c0 + 1, "same DOM state as recorded: walked");
    p.js.eval("t", "$('a').style.width = '60px';");
    let l = laid(&p.doc);
    crate::ro_geom::set_wait(false);
    let now = crate::js_geom::render_stamp();
    crate::ro_geom::app_install(&p.doc, &l, (now.0 + 1, now.1), 1024, 768);
    assert_eq!(crate::ro_geom::stats().app_collects, c0 + 1, "a DOM-write frame nobody waits for: not walked");
    crate::ro_geom::set_wait(true);
    crate::ro_geom::app_install(&p.doc, &l, crate::js_geom::render_stamp(), 1024, 768);
    assert_eq!(crate::ro_geom::stats().app_collects, c0 + 2, "the observers wait for it: walked");
    assert!(!crate::ro_geom::waiting());
    p.settle();
    assert_eq!(p.take(), ["log:a 60x50"]);
}
