// QMEM3j / QY53o3c: the shaping memo's size (compact entries, no per-line attribute B-tree) and
// its sweep policy (JS thread: last layout; shaping worker: last few builds; a cap above both).
use super::*;
use crate::layout::{memo_clear, memo_get, memo_put, memo_sweep, memo_track_begin, shape_memo_lookups, shape_spans, shaped_paragraph_bytes, shape_spans_legacy_defaults};

/// A page with what Wikipedia has: plain paragraphs, links and bold runs (several spans), inline
/// images and inline-blocks (the mixed-inline word path), `<pre>` with blank lines, a list, a table.
fn qmem3j_page() -> String {
    let img = tiny_png_data_uri(8, 8);
    let mut html = String::from("<html><body style='margin:4px'>");
    for i in 0..12 {
        html.push_str(&format!(
            "<p>Paragraph {i} of plain text that wraps across a few lines in a narrow box, with words.</p>\
<p>Run {i} with a <a href='#a{i}'>link</a>, some <b>bold words</b> and <i>italics</i> in it.</p>\
<p>Mixed {i} <img src='{img}' width=8 height=8> text<sup style='display:inline-block'>[{i}]</sup> after the image.</p>"
        ));
    }
    html.push_str("<pre>line one\n\nline three after a blank</pre><ul><li>one</li><li>two <b>bold</b></li></ul>");
    html.push_str("<table><tr><td>cell a</td><td>cell <i>b</i></td></tr></table></body></html>");
    html
}

/// Every box rect plus every glyph position of the laid tree, in walk order.
fn qmem3j_dump(laid: &Laid, cache: &ShapeCache, out: &mut Vec<String>) {
    out.push(format!("box {} {} {} {}", laid.x, laid.y, laid.w, laid.h));
    let glyphs = |id: usize, out: &mut Vec<String>| {
        if let Some(e) = cache.get(&id) {
            let g = e.lock().unwrap();
            out.push(format!("h {}", g.1));
            for run in g.0.layout_runs() {
                out.push(format!("run {} {} {} {}", run.line_top, run.line_y, run.line_height, run.line_w));
                for gl in run.glyphs {
                    out.push(format!("g {} {} {} {} {} {} {}", gl.glyph_id, gl.start, gl.end, gl.x, gl.y, gl.w, gl.metadata));
                }
            }
        }
    };
    match &laid.content {
        LaidContent::Blocks(v) | LaidContent::TableRow(v) => v.iter().for_each(|c| qmem3j_dump(c, cache, out)),
        LaidContent::Text(id) => glyphs(*id, out),
        LaidContent::MixedInline(items) => {
            for it in items {
                match it {
                    MixedPositioned::Word { id, x, y, w, h } => {
                        out.push(format!("word {x} {y} {w} {h}"));
                        glyphs(*id, out);
                    }
                    MixedPositioned::Embedded(l) => qmem3j_dump(l, cache, out),
                }
            }
        }
        LaidContent::Image(_) => {}
    }
    laid.positioned.iter().for_each(|c| qmem3j_dump(c, cache, out));
}

fn qmem3j_layout(doc: &Handle, fs: &mut FontSystem) -> (Vec<String>, ShapeCache) {
    let tree = build_layout_tree(doc, 360.0);
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let dl = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let laid = measure_document(&tree, fs, 360.0, 400.0, &mut next_id, &mut cache, dl, &mut inc);
    assert!(!inc, "fixture must lay out completely");
    let mut out = Vec::new();
    qmem3j_dump(&laid, &cache, &mut out);
    (out, cache)
}

#[test]
fn qmem3j_compacted_memo_lays_out_identically() {
    let doc = parse_html(&qmem3j_page());
    let mut fs = new_font_system();
    // Memo off: freshly shaped buffers, never compacted.
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    let (fresh, _) = qmem3j_layout(&doc, &mut fs);
    // Memo on: the first layout stores (compacts) every entry, the second is served from it.
    memo_clear();
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let (stored, _) = qmem3j_layout(&doc, &mut fs);
    let (h0, m0) = shape_memo_lookups();
    let (hit, cache) = qmem3j_layout(&doc, &mut fs);
    let (h1, m1) = shape_memo_lookups();
    let memo: Vec<_> = SHAPE_MEMO.with(|m| m.borrow().values().cloned().collect());
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
    memo_clear();
    assert!(fresh.iter().filter(|l| l.starts_with("g ")).count() > 500, "fixture shapes real text");
    assert_eq!(fresh, stored, "compacting a stored entry must not move anything");
    assert_eq!(fresh, hit, "a memo hit must lay out exactly like fresh shaping");
    assert_eq!(m1, m0, "the repeated layout of the same page misses nothing");
    assert!(h1 > h0, "and is served from the memo");
    assert!(!cache.is_empty() && memo.len() > 40, "{}", memo.len());
    // Compact: no capacity slack in what cosmic-text exposes, no attribute B-tree for one span.
    for e in &memo {
        let g = e.lock().unwrap();
        assert_eq!(g.0.lines.capacity(), g.0.lines.len());
        for l in &g.0.lines {
            let sh = l.shape_opt().expect("shaped");
            assert_eq!(sh.spans.capacity(), sh.spans.len());
            for sp in &sh.spans {
                assert_eq!(sp.words.capacity(), sp.words.len());
                assert!(sp.words.iter().all(|w| w.glyphs.capacity() == w.glyphs.len()));
            }
            let ll = l.layout_opt().expect("laid out");
            assert!(ll.iter().all(|x| x.glyphs.capacity() == x.glyphs.len()));
            if g.2.len() == 1 {
                assert_eq!(l.attrs_list().spans_iter().count(), 0, "a one-span line keeps its attrs as the default");
            }
        }
        assert!(shaped_paragraph_bytes(&g) > std::mem::size_of::<crate::layout::ShapedParagraph>());
    }
}

#[test]
fn qmem3j_first_span_defaults_shape_like_the_old_empty_defaults() {
    let mut fs = new_font_system();
    let span = |c: u32, w: u16| {
        let mut s = SpanStyle::from(ROOT_STYLE);
        s.color = TextColor(0xff00_0000 | c);
        s.font_weight = FontWeight(w);
        s
    };
    let cases: Vec<(Vec<InlineItem>, bool)> = vec![
        (vec![InlineItem::Text("one span of ordinary text that wraps in a narrow box".into(), span(1, 400))], false),
        (vec![InlineItem::Text("first ".into(), span(1, 400)), InlineItem::Text("bold".into(), span(2, 700)), InlineItem::Text(" tail".into(), span(3, 400))], false),
        (vec![InlineItem::Text("pre line\n\nafter a blank line".into(), span(1, 400))], true),
        (vec![InlineItem::Text(String::new(), span(1, 400))], false),
    ];
    for (items, pre) in cases {
        let a = shape_spans(&mut fs, &items, 120.0, pre, None);
        let b = shape_spans_legacy_defaults(&mut fs, &items, 120.0, pre, None);
        assert_eq!(a.1, b.1, "height");
        let runs = |p: &crate::layout::ShapedParagraph| {
            p.0.layout_runs()
                .map(|r| (r.line_top, r.line_height, r.line_w, r.glyphs.iter().map(|g| (g.glyph_id, g.start, g.end, g.x, g.w, g.metadata, g.color_opt, g.font_weight)).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        };
        assert_eq!(runs(&a), runs(&b));
    }
}

fn qmem3j_entry() -> Arc<Mutex<crate::layout::ShapedParagraph>> {
    let mut fs = new_font_system();
    Arc::new(Mutex::new(shape_spans(&mut fs, &[InlineItem::Text("x".into(), SpanStyle::from(ROOT_STYLE))], 100.0, false, None)))
}

#[test]
fn qy53o3c_worker_sweep_keeps_recent_builds_and_respects_the_cap() {
    SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    memo_clear();
    let e = qmem3j_entry();
    let has = |k: u64| SHAPE_MEMO.with(|m| m.borrow().contains_key(&k));
    // Build 1 stores 1 and 2; build 2 uses 1 and stores 3; build 3 uses 1 only; keep = 2 builds.
    memo_track_begin();
    memo_put(Some(1), &e);
    memo_put(Some(2), &e);
    memo_sweep(100, 2);
    memo_track_begin();
    assert!(memo_get(Some(1)).is_some());
    memo_put(Some(3), &e);
    memo_sweep(100, 2);
    assert!(has(1) && has(2) && has(3), "2 was used one build ago: kept");
    memo_track_begin();
    assert!(memo_get(Some(1)).is_some());
    memo_sweep(100, 2);
    assert!(has(1) && has(3) && !has(2), "2 is two builds old: dropped");
    // A build that never finishes is never swept (the next one starts tracking afresh).
    memo_track_begin();
    memo_put(Some(4), &e);
    memo_track_begin();
    assert!(memo_get(Some(1)).is_some() && memo_get(Some(3)).is_some());
    memo_sweep(100, 2);
    assert!(has(1) && has(3) && !has(4), "4 was stored by an abandoned build only");
    // Over the cap after the sweep: start over.
    memo_track_begin();
    for k in 10..20 {
        memo_put(Some(k), &e);
    }
    memo_sweep(5, 2);
    assert_eq!(SHAPE_MEMO.with(|m| m.borrow().len()), 0, "cap respected");
    // keep = 1 (the JS thread): exactly what this layout used.
    memo_track_begin();
    memo_put(Some(30), &e);
    memo_sweep(100, 1);
    memo_track_begin();
    memo_put(Some(31), &e);
    memo_sweep(100, 1);
    assert!(!has(30) && has(31));
    memo_clear();
    SHAPE_MEMO_ENABLED.with(|m| m.set(false));
}
