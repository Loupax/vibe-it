use super::*;

// Shared helpers live here; every area file is a child module and sees them through `use super::*;`.
// New tests go in the matching area file (see AGENTS.md); a new area gets a new `<area>_tests.rs`.
mod animation_tests;
mod box_layout_tests;
mod cascade_tests;
mod fetch_tests;
mod flex_grid_tests;
mod forms_tests;
mod frame_stats_tests;
mod frame_budget_tests;
mod images_tests;
mod interaction_tests;
mod js_core_tests;
mod js_dom_tests;
mod js_engine_tests;
mod js_geometry_tests;
mod js_observers_tests;
mod qapi5_tests;
mod qdom10_tests;
mod qdom14_tests;
mod qdom3_tests;
mod js_geometry_incr_forced_tests;
mod js_observers_incr_forced_tests;
mod qdom12_incr_forced_tests;
mod qdom12_tests;
mod qdomatom_tests;
mod qdom13_tests;
mod incr_forced_tests;
mod media_tests;
mod media_render_tests;
mod mse_tests;
mod mse_mpv_tests;
mod navigation_tests;
mod net_policy_tests;
mod paint_tests;
mod perf_tests;
mod qmsg1_tests;
mod qapi7_tests;
mod missing_api_tests;
mod incr_fuzz_tests;
mod zhoist_tests;
mod positioning_tests;
mod qf19w2_tests;
mod qnet1_tests;
mod scrollbar_tests;
mod qwu1_tests;
mod tables_tests;
mod wpt_tests;
mod wpt_th_tests;
mod js_snapshot_tests;
mod deep_nesting_tests;
mod xhr_tests;
mod qmem3a_tests;
mod qy53n1_tests;
mod qmem3d_tests;
mod shape_memo_tests;
mod qy53n2_tests;
mod qy53n3_tests;
mod qapi4_tests;
mod wl_drain_tests;
mod link_css_tests;
mod invalidation_tests;
mod para_id_tests;
mod incr_log_tests;
mod incr_style_tests;
mod incr_box_tests;
mod incr_layout_tests;
mod incr_frame_tests;
mod stash_tests;
mod status_bar_tests;
mod keys_tests;
mod search_tests;
mod qkeys4_tests;
mod qscr404_tests;
mod forget_tests;
mod history_tests;
mod completion_tests;
mod event_loop_tests;
mod qel4_tests;
mod qel5_tests;
mod qel2_tests;
mod hscroll_tests;
mod qel3_tests;
mod qanimhl_tests;
mod qsb2b_tests;
mod qanimhlb_tests;
mod qce1_tests;
mod qwptflk3_tests;
mod qsmooth1_tests;
mod qdom8_tests;
mod qdom5_tests;
mod qel6_tests;
mod qel7_tests;
mod qdom15_tests;
mod qdom6b_tests;
mod qapi1_tests;
mod qdom16_tests;
mod qdom17_tests;
mod qdom19_tests;
#[cfg(feature = "memprof")]
mod memprof_tests;
mod qapi2_tests;
mod qapi3_tests;

// ---------- QFLK7: the one shared winit event loop for App-level tests ----------
//
// winit allows ONE event loop per process, so tests that need an `App` (it takes an `EventLoopProxy`)
// used to build their own and silently `return` when the second build failed: in a parallel run most
// bodies never executed. Now the loop is built once, leaked, and every test gets a handle that
// hands out clones of the same proxy. No display (headless CI): the test is skipped with a visible
// `QFLK7-SKIP` line on stderr (shown with `--nocapture`), or FAILS when `SCRATCHY_REQUIRE_DISPLAY`
// is set (use it on any machine that has a compositor). A test that really ran prints `QFLK7-RAN`.

/// A handle to the shared test loop: only `create_proxy` is exposed.
pub(crate) struct TestLoop(EventLoopProxy<AppEvent>);

impl TestLoop {
    pub(crate) fn create_proxy(&self) -> EventLoopProxy<AppEvent> {
        self.0.clone()
    }
}

/// The skip / fail decision for a test that needs a display and found none.
pub(crate) fn no_display_policy(test: &str) {
    if std::env::var_os("SCRATCHY_REQUIRE_DISPLAY").is_some() {
        panic!("{test}: no display available and SCRATCHY_REQUIRE_DISPLAY is set");
    }
    eprintln!("QFLK7-SKIP {test}: no display available (set SCRATCHY_REQUIRE_DISPLAY=1 to fail instead)");
}

/// The shared loop's handle, or `Err(())` without a display (after applying the policy above).
pub(crate) fn test_event_loop() -> Result<TestLoop, ()> {
    static LOOP: std::sync::OnceLock<Option<std::sync::Mutex<EventLoopProxy<AppEvent>>>> = std::sync::OnceLock::new();
    let who = std::thread::current().name().unwrap_or("?").to_string();
    let proxy = LOOP
        .get_or_init(|| {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            let l = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
            let p = l.create_proxy();
            std::mem::forget(l); // the proxy must stay valid for the whole process
            Some(std::sync::Mutex::new(p))
        })
        .as_ref()
        .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).clone());
    match proxy {
        Some(p) => {
            eprintln!("QFLK7-RAN {who}");
            Ok(TestLoop(p))
        }
        None => {
            no_display_policy(&who);
            Err(())
        }
    }
}

// ---------- v37: `:link`/`:visited` pseudo-classes ----------

fn find_first_span_color(b: &LayoutBox) -> Option<TextColor> {
    if let BoxContent::Inline(items) = &b.content {
        if let Some(InlineItem::Text(_, s)) = items.first() {
            return Some(s.color);
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_first_span_color(c) {
                return Some(found);
            }
        }
    }
    None
}

// ---------- v42: `width`/`height` CSS properties ----------

fn measure_fixture(html: &str, available_width: f32) -> Laid {
    measure_fixture_h(html, available_width, None)
}

/// Same as `measure_fixture`, but also lets a test supply a definite
/// containing-block height for the document root (see `measure`'s
/// own `containing_height` docs) — needed to exercise a `%` `height`/
/// `min-height`/`max-height` declared on a top-level element, which
/// `measure_fixture`'s plain `None` (indefinite) deliberately can't
/// resolve, matching a real UA only ever having one via its viewport.
fn measure_fixture_h(html: &str, available_width: f32, containing_height: Option<f32>) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure(
        &tree, &mut font_system, 0.0, 0.0, available_width, containing_height, &mut next_id, &mut cache, deadline,
        &mut incomplete, &mut Vec::new(), &Floats::default(), None,
    )
}

fn find_laid_by_margin_top(laid: &Laid, target: f32) -> Option<&Laid> {
    if laid.style.margin.top.px() == target {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_laid_by_margin_top(c, target) {
                return Some(found);
            }
        }
    }
    None
}

fn find_by_height(laid: &Laid, target_h: f32) -> Option<&Laid> {
    if laid.style.height.map(BoxLength::px) == Some(target_h) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_by_height(c, target_h) {
                return Some(found);
            }
        }
    }
    None
}

fn find_by_width(laid: &Laid, target_w: f32) -> Option<&Laid> {
    if laid.style.width.map(BoxLength::px) == Some(target_w) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_by_width(c, target_w) {
                return Some(found);
            }
        }
    }
    None
}

// ---------- v44: structural pseudo-classes (`:not()`, `:nth-child()`, `:first-child`) ----------
// These aren't `OurPseudoClass` variants at all — `:link`/`:visited` need custom
// state scratchy tracks itself, but `:not()`/`:nth-child()`/`:first-child` are
// built into the `selectors` crate's own matching, driven purely by the
// `SelectorsElement` methods (`prev_sibling_element`, `next_sibling_element`,
// `is_root`, ...) already implemented for `El`. Never actually verified before.

fn find_all_span_colors(b: &LayoutBox, out: &mut Vec<TextColor>) {
    if let BoxContent::Inline(items) = &b.content {
        for item in items {
            match item {
                InlineItem::Text(_, s) => out.push(s.color),
                // QW3: `<button>` is an inline-block (embedded box).
                InlineItem::Embedded(e) => find_all_span_colors(e, out),
                _ => {}
            }
        }
    } else if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            find_all_span_colors(c, out);
        }
    }
}

fn find_by_tag(h: &Handle, tag: &str) -> Option<Handle> {
    if element_tag(h) == tag {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_tag(c, tag))
}

fn q19_texts(b: &LayoutBox, out: &mut Vec<(String, TextColor)>) {
    match &b.content {
        BoxContent::Inline(items) => {
            for item in items {
                match item {
                    InlineItem::Text(t, s) => out.push((t.clone(), s.color)),
                    InlineItem::Embedded(e) => q19_texts(e, out),
                    _ => {}
                }
            }
        }
        BoxContent::Blocks(children) => {
            for c in children {
                q19_texts(c, out);
            }
        }
        _ => {}
    }
}

// ---------- v46: `<img>` rendering ----------

/// Builds a solid-color PNG in memory and returns it as a `data:` URI
/// — avoids hardcoding a base64 blob in test source; exercises the
/// real `image`-crate decode path the same way a real `<img src>`
/// would, just generated instead of fetched.
fn tiny_png_data_uri(width: u32, height: u32) -> String {
    use base64::Engine;
    let img = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |_, _| {
        image::Rgba([255u8, 0, 0, 255])
    }));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))
}

fn find_image_laid(laid: &Laid) -> Option<&Laid> {
    if matches!(laid.content, LaidContent::Image(_)) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_image_laid(c) {
                return Some(found);
            }
        }
    }
    // A default-`display: inline` `<img>` (the common case since
    // v48) lands inside a `MixedInline` run instead of its own
    // top-level `LaidContent::Image` — see `MixedPositioned`'s own
    // docs.
    if let LaidContent::MixedInline(items) = &laid.content {
        for item in items {
            // `Embedded` can now also hold a `display: inline-block`/
            // `inline-flex` element's own subtree, not just an
            // inline `<img>` — check this entry's own `content` so a
            // run mixing both kinds doesn't return whichever comes
            // first regardless of which one a test actually wants.
            if let MixedPositioned::Embedded(embedded_laid) = item
                && matches!(embedded_laid.content, LaidContent::Image(_))
            {
                return Some(embedded_laid);
            }
        }
    }
    None
}

// ---------- v48: real inline-level `<img>` flow ----------

fn find_mixed_inline(laid: &Laid) -> Option<&Laid> {
    if matches!(laid.content, LaidContent::MixedInline(_)) {
        return Some(laid);
    }
    if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
        for c in children {
            if let Some(found) = find_mixed_inline(c) {
                return Some(found);
            }
        }
    }
    None
}

fn mixed_items(laid: &Laid) -> &Vec<MixedPositioned> {
    let LaidContent::MixedInline(items) = &laid.content else { panic!("not a MixedInline Laid") };
    items
}

fn mixed_item_xy(item: &MixedPositioned) -> (f32, f32) {
    match item {
        MixedPositioned::Embedded(laid) => (laid.x, laid.y),
        MixedPositioned::Word { x, y, .. } => (*x, *y),
    }
}

// ---------- table layout: real CSS auto table layout (min/max content width per column) ----------

/// The first `LaidContent::TableRow`'s own cells, found by walking
/// through any number of wrapping `Blocks` boxes (a `<table>`'s own
/// content, or `<body>`/`<html>` above it) — same recursive-search
/// shape as this file's other `find_*` test helpers.
fn find_table_row_cells(laid: &Laid) -> Option<&[Laid]> {
    if let LaidContent::TableRow(cells) = &laid.content {
        return Some(cells);
    }
    if let LaidContent::Blocks(children) = &laid.content {
        for c in children {
            if let Some(found) = find_table_row_cells(c) {
                return Some(found);
            }
        }
    }
    None
}

fn collect_text_shape_ids(laid: &Laid, ids: &mut Vec<usize>) {
    match &laid.content {
        LaidContent::Text(id) => ids.push(*id),
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children {
                collect_text_shape_ids(c, ids);
            }
        }
        LaidContent::Image(_) | LaidContent::MixedInline(_) => {}
    }
}

// ---------- v52: positioning model (`position`/`z-index`) ----------

/// Like `measure_fixture`, but through `measure_document` — the real
/// top-level entry point, needed for anything that exercises `fixed`
/// or an orphaned `absolute` (both only ever resolved at the document
/// root; see `measure_document`'s own docs). Plain `measure_fixture`
/// calls `measure` directly, which would silently *drop* either case
/// (whatever bubbles out of its own disposable `pending` `Vec` is
/// simply never resolved) — the right behavior for `measure` itself
/// (it has no viewport to resolve them against), wrong for a test
/// that wants to see the fully-resolved tree.
fn measure_document_fixture(html: &str, width: f32, viewport_h: f32) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, width, viewport_h, &mut next_id, &mut cache, deadline, &mut incomplete)
}

// ---------- v56: real CSS2.1 Appendix E stacking-context semantics ----------

/// Shared plumbing for this section's tests: parse, build, measure at
/// a 200x200 viewport (`measure_document`, so `position: absolute`/
/// `fixed` resolve the same way they would in the real app), paint
/// into a real pixel buffer, and hand back just the buffer plus the
/// window size needed to index it. `<body style="margin:0">` in every
/// fixture below sidesteps `default.css`'s own `body { margin: 8px }`
/// — these tests sample hand-computed pixel coordinates against
/// in-flow boxes (unlike the pre-existing `z_index_controls_paint_
/// order_for_overlapping_absolute_boxes` test above, whose `position:
/// absolute; top/left: 0` boxes anchor to the *viewport*, immune to
/// body's own margin either way).
fn paint_fixture(html: &str) -> (Vec<u32>, u32, u32) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid =
        measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let (win_w, win_h) = (200u32, 200u32);
    let mut pixels = vec![0u32; (win_w * win_h) as usize];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
    (pixels, win_w, win_h)
}

fn px_at(pixels: &[u32], win_w: u32, x: u32, y: u32) -> u32 {
    pixels[(y * win_w + x) as usize] & 0x00FF_FFFF
}

// ---------- v57: flexbox (`display: flex`) ----------

/// The flex container's own direct `LaidContent::Blocks` children,
/// in document order — exactly the flex items `measure_flex`
/// positioned, found by locating the container itself (via its own
/// distinctive `width`, same `find_by_width` helper every other
/// section already uses) rather than a bespoke lookup.
fn flex_items(laid: &Laid, container_w: f32) -> Vec<&Laid> {
    let container = find_by_width(laid, container_w).expect("flex container not found");
    let LaidContent::Blocks(children) = &container.content else {
        panic!("expected the flex container's content to be Blocks (its own flex items)");
    };
    children.iter().collect()
}

fn measure_fixture_with_cache(html: &str, available_width: f32) -> (Laid, ShapeCache) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, available_width, None, &mut next_id, &mut cache, deadline,
        &mut incomplete, &mut Vec::new(), &Floats::default(), None,
    );
    (laid, cache)
}

/// Finds the pixel-space center of the first glyph whose own per-span
/// cursor (via `glyph.metadata`, the same lookup `hit_test_cursor`
/// itself does) matches `want` — lets a test drive `hit_test_cursor`
/// at a real, specific coordinate instead of guessing one.
fn find_glyph_center(laid: &Laid, cache: &ShapeCache, want: CursorStyle) -> Option<(f32, f32)> {
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            children.iter().find_map(|c| find_glyph_center(c, cache, want))
        }
        LaidContent::Text(id) => {
            let entry = cache.get(id)?;
            let guard = entry.lock().unwrap();
            let (buffer, _, _, _, _, cursors, _, _) = &*guard;
            let content_x = laid.x + laid.style.border_left() + laid.style.padding.left.px();
            let content_y = laid.y + laid.style.border_top() + laid.style.padding.top.px();
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    if cursors.get(glyph.metadata).copied() == Some(want) {
                        return Some((content_x + glyph.x + glyph.w / 2.0, content_y + run.line_top + run.line_height / 2.0));
                    }
                }
            }
            None
        }
        LaidContent::Image(_) | LaidContent::MixedInline(_) => None,
    }
}

// ---------- v64: `@media` (min-width/max-width, and/comma lists, print, viewport resize) ----------

/// Walks the tree for the first box whose own resolved style declares
/// a `background-color` — the simplest single signal to check whether
/// a `@media`-wrapped rule actually applied or not, reused by every
/// test below instead of a bespoke finder per test.
fn find_box_with_background(b: &LayoutBox) -> Option<TextColor> {
    if b.style.background_color.is_some() {
        return b.style.background_color;
    }
    if let BoxContent::Blocks(children) = &b.content {
        for c in children {
            if let Some(found) = find_box_with_background(c) {
                return Some(found);
            }
        }
    }
    None
}

// -- the real wire-socket protocol end to end, against a tiny
// in-process mock listener (same spirit as `wire`'s own test module,
// which does the mirror image of this: a real client against a real
// listener, rather than touching the real internet) --

fn unique_test_socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scratchy-test-{name}-{}-{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// A tiny single-shot mock `wire` listener: binds `path`, accepts
/// exactly one connection, writes `response` verbatim, then returns
/// (dropping the stream, closing the connection — the real `wire`
/// protocol's own "done" signal, same shape every other mock server
/// in this file already uses). Doesn't bother reading/validating the
/// request at all — unlike `fetch_raw_via_wire`'s own round-trip
/// test, these tests are about what `fetch_stylesheet_via_wire`/
/// `fetch_image_via_wire` do with a *response*, not about re-proving
/// the request framing that's already covered elsewhere.
fn spawn_mock_wire_once(path: &std::path::Path, response: &'static [u8]) {
    use std::os::unix::net::UnixListener;
    let listener = UnixListener::bind(path).expect("bind mock wire socket");
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let mut discard = [0u8; 4096];
        let _ = Read::read(&mut stream, &mut discard);
        let _ = stream.write_all(response);
    });
}

// -- the rendering side: does a cached live-fetched resource
// actually reach the page, not just the fetch functions above --

fn find_element_by_tag(handle: &Handle, tag: &str) -> Option<Handle> {
    if element_tag(handle) == tag {
        return Some(handle.clone());
    }
    handle.children.borrow().iter().find_map(|c| find_element_by_tag(c, tag))
}

// Q48: vertical margins collapse through an empty block (CSS2.1 8.3.1).
fn q48_y_of(html: &str, h: f32) -> f32 {
    let laid = measure_fixture(html, 900.0);
    find_by_height(&laid, h).expect("marker div not found").y
}

// ---------- Q33a: running scripts ----------

/// Serializes tests that flip the process-wide JS flag.
static JS_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_collecting(scripts: &[ScriptSource]) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_with(scripts, Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

/// QINC1i: the forced-layout special cases (QYTC1a, QYTC1d, HiddenSnap, ATTR_SNAP, the App-layout
/// IntersectionObserver path) are bypassed when forced layouts go through the engine (the
/// `*_incr_forced_tests` runs of the suites); the tests of those special cases return early there.
fn special_cases_bypassed() -> bool {
    crate::incr_engine::forced_on()
}

fn src(code: &str) -> ScriptSource {
    ScriptSource { name: "t".into(), code: code.into(), index: None, ..Default::default() }
}

// ---------- Q33b: DOM read API ----------

fn run_in_html(html: &str, code: &str) -> Vec<String> {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_in(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

// ---------- Q33t: timers ----------

/// Runs `code` and keeps the engine, then pumps its timers on a manual
/// clock (jumping straight to each next due time, so no real sleeping
/// and no dependence on machine load) until none are pending; returns
/// everything logged. Every registration made while the script runs
/// happens at t=0 ms, which makes due-time ordering fully deterministic.
fn run_with_timers(code: &str) -> Vec<String> {
    run_html_with_timers("", code)
}

/// `run_with_timers` against the document `html` (QPT1f: srcdoc iframes in the markup).
fn run_html_with_timers(html: &str, code: &str) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(html),
        &[src(code)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    // Bounded: an interval would otherwise never run dry.
    for _ in 0..10_000 {
        let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    let v = out.borrow().clone();
    v
}

// ---------- Q33c: DOM writes ----------

/// Runs `code` against `html`, returns the log and the body's markup
/// afterwards, read from the Rust tree (not through JS).
fn mutate(html: &str, code: &str) -> (Vec<String>, String) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let _engine = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let mut body = None;
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        if element_tag(&n) == "body" {
            body = Some(n.clone());
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    let mut s = String::new();
    for c in body.unwrap().children.borrow().iter() {
        let mut buf = Vec::new();
        let _ = html5ever::serialize::serialize(
            &mut buf,
            &markup5ever_rcdom::SerializableHandle::from(c.clone()),
            html5ever::serialize::SerializeOpts {
                traversal_scope: html5ever::serialize::TraversalScope::IncludeNode,
                ..Default::default()
            },
        );
        s.push_str(&String::from_utf8_lossy(&buf));
    }
    let log = out.borrow().clone();
    (log, s)
}

// ---------- Q33d: events ----------

/// Starts `code` against `html` and returns the engine, the document and
/// the shared log (so tests can dispatch native events afterwards).
fn events_engine(html: &str, code: &str) -> (JsEngine, Handle, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    (engine, doc, out)
}

fn by_id(doc: &Handle, id: &str) -> Handle {
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        if element_attr(&n, "id").as_deref() == Some(id) {
            return n;
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    panic!("no #{id}");
}

// ---------- QA13: document.write / writeln during page load ----------

fn qa13_run(html: &str) -> Vec<String> {
    let (_engine, out) = qb8_start(html);
    let v = out.borrow().clone();
    v
}

// ---------- QB8: document.currentScript, named eval frames ----------

fn qb8_start(html: &str) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    // `inline_scripts` reads the process-wide JS flag, which the noscript tests flip under this lock.
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(&doc, &inline_scripts(&doc), Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    (engine, out)
}

// ---------- QA2: U+00A0 is not collapsible white space ----------

fn qa2_body_h(inner: &str) -> f32 {
    let html = format!(r#"<html><body style="margin:0;font:12px sans-serif;line-height:1em">{inner}</body></html>"#);
    measure_fixture(&html, 400.0).h
}


// ---------- QF7: page JS network queue (no fetch/XHR globals yet) ----------

use crate::js_net::NetExecutor;

/// A canned executor: answers by URL substring, records every request it
/// receives (url, thread, read timeout) and can sleep first.
struct QfFake {
    calls: Mutex<Vec<(String, thread::ThreadId, std::time::Duration)>>,
    answers: Vec<(&'static str, Result<WireResponse, NetError>)>,
    delay_ms: u64,
    raw: Mutex<Vec<Vec<u8>>>,
}

impl QfFake {
    fn new(answers: Vec<(&'static str, Result<WireResponse, NetError>)>) -> Arc<Self> {
        Arc::new(QfFake { calls: Mutex::new(Vec::new()), answers, delay_ms: 0, raw: Mutex::new(Vec::new()) })
    }
    fn urls(&self) -> Vec<String> {
        self.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }
}

impl NetExecutor for QfFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        self.raw.lock().unwrap().push(job.request.clone());
        let line = String::from_utf8_lossy(&job.request).lines().next().unwrap_or("").to_string();
        let url = line.split(' ').nth(1).unwrap_or("").to_string();
        self.calls.lock().unwrap().push((url.clone(), thread::current().id(), job.limits.read_timeout));
        if self.delay_ms > 0 {
            thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        }
        for (pat, r) in &self.answers {
            if url.contains(pat) {
                return r.clone();
            }
        }
        Err(NetError::Io("no canned answer".into()))
    }
}

fn qf7_ok(body: &str, headers: &[(&str, &str)]) -> Result<WireResponse, NetError> {
    Ok(WireResponse {
        status: 200,
        status_text: "OK".into(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.as_bytes().to_vec(),
        error: None,
        final_url: None,
    })
}

/// Starts an engine on `page` with the fake as the executor (no scripts yet).
fn qf7_engine(page: &str, fake: &Arc<QfFake>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}")))).unwrap();
    (engine, out)
}

fn qf7_drive(engine: &JsEngine, fake: &Arc<QfFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

/// JS that sends one GET and logs `ok <status> <type> <body>` or `err <name> <msg>`.
fn qf7_go(url: &str, extra: &str) -> String {
    format!(
        "var h = __net_request({{method:'GET', url:'{url}', headers:[], body:null{extra}}});\
         h.promise.then(function(r){{ return r.text().then(function(t){{ console.log('ok', r.status, r.type, t); }}); }},\
         function(e){{ console.log('err', e.name, e.message); }});"
    )
}

// ---------- QF8: window.fetch over the QF7 queue (fake executor, no network) ----------

fn qf8_run(page: &str, fake: &Arc<QfFake>, code: &str) -> Vec<String> {
    let (engine, log) = qf7_engine(page, fake);
    engine.eval("t", code);
    qf7_drive(&engine, fake);
    let l = log.borrow().clone();
    l
}

// ---------- QF9: XMLHttpRequest over the QF7 queue (fake executor, no network) ----------

const QF9_TRACK: &str = "function track(x, tag) { tag = tag || ''; x.onreadystatechange = function () { console.log(tag + 'rs' + x.readyState); };\
  ['loadstart','progress','load','error','abort','timeout','loadend'].forEach(function (t) { x.addEventListener(t, function (e) { console.log(tag + t, e.loaded, e.total, e.lengthComputable); }); }); }\n";

fn qf9_run(page: &str, fake: &Arc<QfFake>, code: &str) -> Vec<String> {
    qf8_run(page, fake, &format!("{QF9_TRACK}{code}"))
}

// ---------- QF14b: the scratchy half of the cookie jar (fake wire, no network) ----------

/// A fake wire on a unix socket that records every request (head + body) and
/// answers by URL extension like a cookie-capable wire 3.
fn qf14b_fake_wire(path: &std::path::Path) -> Arc<Mutex<Vec<String>>> {
    use std::os::unix::net::UnixListener;
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("bind fake wire");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let s = s.clone();
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let line = text.lines().next().unwrap_or("").to_string();
                let (ct, body): (&str, &str) = if line.contains(".css") {
                    ("text/css", "p{margin:0}")
                } else if line.contains(".js") {
                    ("application/javascript", "var a = 1;")
                } else if line.contains(".png") {
                    ("image/png", "not really a png")
                } else {
                    ("text/html", "<html><body>x</body></html>")
                };
                s.lock().unwrap().push(text);
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nX-Wire-Version: 3\r\nX-Wire-Cookies: on\r\n\r\n{body}");
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    seen
}

fn qf14b_has(req: &str, header: &str) -> bool {
    req.to_ascii_lowercase().contains(&format!("\r\n{}\r\n", header.to_ascii_lowercase()))
}

fn qf14b_wait(seen: &Arc<Mutex<Vec<String>>>, n: usize) -> Vec<String> {
    for _ in 0..300 {
        if seen.lock().unwrap().len() >= n {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    seen.lock().unwrap().clone()
}

// ---------- QSD4: constructable stylesheets / adoptedStyleSheets ----------

/// Runs `code`, then builds the layout tree the way the App does (page CSS
/// plus the engine's published adopted CSS) and returns the first text colour.
fn qsd4_color(html: &str, code: &str) -> (Option<TextColor>, Vec<String>, String) {
    let (engine, doc, log) = events_engine(html, code);
    let adopted = engine.take_requests().adopted_css.unwrap_or_default();
    let css = crate::images::with_adopted_css("", &adopted).into_owned();
    let tree = build_layout_tree_with_resources(&doc, 800.0, None, &css, &HashMap::new());
    let l = log.borrow().clone();
    (find_first_span_color(&tree), l, adopted)
}

// ---------- QM1e: the media byte channel over a fake wire (no libmpv, no network) ----------

use crate::media_net::{MediaMode, MediaNetError, MediaSource, MediaStream, CHUNK_LARGE, CHUNK_SMALL};


#[derive(Default, Clone)]
struct Qm1eCfg {
    file: Vec<u8>,
    ignore_range: bool,
    /// Drop the second half of the body of the answer to request #1.
    truncate_once: bool,
    /// Report `Content-Range` starting this many bytes late.
    shift_start: u64,
    /// `X-Wire-Final-Url` per request index (0-based), else none.
    final_urls: Vec<(usize, String)>,
    acao: Option<String>,
    acac: bool,
    /// Accept requests and never answer.
    hold: bool,
}

struct Qm1eWire {
    path: std::path::PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
    cfg: Arc<Mutex<Qm1eCfg>>,
}

fn qm1e_wire(cfg: Qm1eCfg) -> Qm1eWire {
    use std::os::unix::net::UnixListener;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!("qm1e-{}-{}.sock", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind fake wire");
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let cfg = Arc::new(Mutex::new(cfg));
    let (s, c) = (seen.clone(), cfg.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (s, c) = (s.clone(), c.clone());
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let idx = {
                    let mut g = s.lock().unwrap();
                    g.push(text.clone());
                    g.len() - 1
                };
                let cfg = c.lock().unwrap().clone();
                if cfg.hold {
                    thread::sleep(std::time::Duration::from_secs(20));
                    return;
                }
                let hdr = |n: &str| {
                    text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()))
                };
                let mut extra = String::from("X-Wire-Version: 3\r\n");
                if let Some((_, u)) = cfg.final_urls.iter().find(|(i, _)| *i == idx) {
                    extra += &format!("X-Wire-Final-Url: {u}\r\n");
                }
                if let Some(a) = &cfg.acao {
                    extra += &format!("Access-Control-Allow-Origin: {a}\r\n");
                }
                if cfg.acac {
                    extra += "Access-Control-Allow-Credentials: true\r\n";
                }
                let max: Option<usize> = hdr("x-wire-max-bytes").and_then(|v| v.parse().ok());
                let total = cfg.file.len() as u64;
                let range = hdr("range").and_then(|r| {
                    let r = r.strip_prefix("bytes=")?.to_string();
                    let (a, b) = r.split_once('-')?;
                    Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))
                });
                let (head, body): (String, Vec<u8>) = match range {
                    Some((a, b)) if !cfg.ignore_range => {
                        if a >= total {
                            (format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\n{extra}"), vec![])
                        } else {
                            let b = b.min(total - 1);
                            let mut body = cfg.file[a as usize..=b as usize].to_vec();
                            if cfg.truncate_once && idx == 1 {
                                body.truncate(body.len() / 2);
                            }
                            (
                                format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{b}/{total}\r\n{extra}", a + cfg.shift_start),
                                body,
                            )
                        }
                    }
                    _ => match max {
                        Some(m) if cfg.file.len() > m => (
                            format!("HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nX-Wire-Error: too-large\r\n{extra}"),
                            b"wire: response too large".to_vec(),
                        ),
                        _ => (format!("HTTP/1.1 200 OK\r\n{extra}"), cfg.file.clone()),
                    },
                };
                let _ = stream.write_all(format!("{head}\r\n").as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    Qm1eWire { path, seen, cfg }
}

fn q30b_by_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| *v == *id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| q30b_by_id(c, id))
}

// ---- Q30c: checkbox/radio live state ----

fn q30c_by(doc: &Handle, id: &str) -> Handle {
    q30b_by_id(doc, id).unwrap_or_else(|| panic!("no #{id}"))
}

fn q30e_log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value("t", "window.L.join('|')").unwrap_or_default()
}

// ---------- QAN1d0: prefers-reduced-motion defaults to reduce ----------

static MOTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
mod qapi8_tests;
