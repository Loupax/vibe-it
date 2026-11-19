use criterion::{criterion_group, Criterion};
use scratchy::{build_layout_tree, measure, new_font_system, parse_html, Floats, ShapeCache};
use std::hint::black_box;

/// A synthetic-but-realistic fixture (scratchy has no network access, no
/// bundler tool to pull in a real page yet — see NOTES.md), big enough to
/// give both the tree-builder and the shaper real work: enough paragraphs
/// with mixed inline elements to exercise text extraction/the cascade, a
/// list and a table to exercise the box-model/table-layout paths too.
fn synthetic_html(paragraphs: usize) -> String {
    let mut s = String::from("<!doctype html><html><body>\n");
    for i in 0..paragraphs {
        s.push_str(&format!(
            "<p>Paragraph {i} with <b>bold</b>, <i>italic</i>, and a <a href=\"#\">link</a> \
             to pad out the text a bit so shaping has real work to do across a realistic \
             line length, not just a couple of words.</p>\n"
        ));
    }
    s.push_str("<ul>\n");
    for i in 0..20 {
        s.push_str(&format!("<li>List item {i}</li>\n"));
    }
    s.push_str("</ul>\n<table>\n");
    for r in 0..10 {
        s.push_str("<tr>");
        for c in 0..5 {
            s.push_str(&format!("<td>row {r}, col {c}</td>"));
        }
        s.push_str("</tr>\n");
    }
    s.push_str("</table>\n</body></html>");
    s
}

/// DOM -> CSS cascade -> box tree, no shaping — isolates the cost that was
/// at the center of the box-tree-flattening bug fixed during v1-v5 (a
/// nested block absorbed into one giant text run on a real Wikipedia page).
fn bench_build_layout_tree(c: &mut Criterion) {
    let html = synthetic_html(200);
    let document = parse_html(&html);
    c.bench_function("build_layout_tree, 200 paragraphs + list + table", |b| {
        b.iter(|| build_layout_tree(black_box(&document), 1920.0));
    });
}

/// A minimal real (not just a literal data: string typed from memory)
/// base64-encoded PNG, built via the `image` crate exactly like the
/// unit tests' own `tiny_png_data_uri` — this crate target already
/// depends on `image`/`base64` directly (see Cargo.toml), so reusing
/// that approach here needs no new dependency.
fn tiny_png_data_uri(width: u32, height: u32) -> String {
    use base64::Engine;
    let img = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |_, _| image::Rgba([255u8, 0, 0, 255])));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Same shape as `synthetic_html`, but every paragraph also carries one
/// small inline `<img>` mixed into its own running text — sparse icons
/// inside flowing prose, the real-world pattern (Wikipedia's own inline
/// flag/lock/edit-badge icons) that routes a paragraph through `measure_
/// mixed_inline`'s own word-wrap-around-an-image path instead of the
/// plain whole-paragraph one `synthetic_html`'s own fixture exercises.
/// Exists specifically to catch a regression back to that function's own
/// old one-`cosmic_text::Buffer`-per-word shaping, measured (`src/lib.rs`'s
/// own `#[ignore]`d `mixed_inline_overhead_diagnostic`/`shape_spans_per_
/// call_overhead_diagnostic`) at 15-17x (end to end) to 135-280x (shape_
/// spans calls alone) slower than shaping each chunk of text between
/// images once instead.
fn synthetic_html_with_inline_icons(paragraphs: usize) -> String {
    let src = tiny_png_data_uri(4, 4);
    let mut s = String::from("<!doctype html><html><body>\n");
    for i in 0..paragraphs {
        s.push_str(&format!(
            "<p>Paragraph {i} opens with a run of plain words before the page's \
             own inline icon shows up right here <img src=\"{src}\" style=\"width: \
             12px; height: 12px\"> and then the sentence keeps going afterward with \
             more ordinary words, padding this chunk out to a realistic length on \
             both sides of the icon so shaping has real work to do.</p>\n"
        ));
    }
    s.push_str("</body></html>");
    s
}

/// Same shaping work as `bench_measure`, but through a page shaped like
/// `synthetic_html_with_inline_icons` instead — see its own docs for
/// why this specific shape (an inline `<img>` sitting in otherwise
/// ordinary running text) is the one that matters here.
fn bench_measure_mixed_inline(c: &mut Criterion) {
    let html = synthetic_html_with_inline_icons(200);
    let document = parse_html(&html);
    // Wide enough that each paragraph's own icon-straddled text chunks
    // comfortably fit on one line — the common real-world case this
    // benchmark targets, not the rare chunk-itself-wraps fallback
    // `measure_mixed_inline`'s own docs separately call out.
    let tree = build_layout_tree(&document, 2400.0);
    let mut font_system = new_font_system();
    c.bench_function("measure, cold shape cache, 200 paragraphs each with one inline icon", |b| {
        b.iter(|| {
            let mut cache = ShapeCache::new();
            let mut next_id = 0usize;
            let mut incomplete = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            measure(
                black_box(&tree),
                &mut font_system,
                0.0,
                0.0,
                2400.0,
                None,
                &mut next_id,
                &mut cache,
                deadline,
                &mut incomplete,
                &mut Vec::new(),
                &Floats::default(),
                None,
            )
        });
    });
}

/// Full layout: box tree -> shaped text + resolved geometry. A fresh,
/// empty `ShapeCache` every iteration — real usage caches shaped
/// paragraphs across frames (see `measure`'s own docs), so a *reused*
/// cache here would benchmark almost nothing after the first iteration.
/// This is the cost that was once multiple seconds to tens of seconds on
/// a real large page before the resumable, time-budgeted shaping pass
/// (see PROMPTS.md/NOTES.md) — a generous 60s deadline here means this
/// always benchmarks a *complete* layout pass, never a partial one.
fn bench_measure(c: &mut Criterion) {
    let html = synthetic_html(200);
    let document = parse_html(&html);
    let tree = build_layout_tree(&document, 1920.0);
    let mut font_system = new_font_system();
    c.bench_function("measure, cold shape cache, 200 paragraphs + list + table", |b| {
        b.iter(|| {
            let mut cache = ShapeCache::new();
            let mut next_id = 0usize;
            let mut incomplete = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            measure(
                black_box(&tree),
                &mut font_system,
                0.0,
                0.0,
                1024.0,
                None,
                &mut next_id,
                &mut cache,
                deadline,
                &mut incomplete,
                &mut Vec::new(),
                &Floats::default(),
                None,
            )
        });
    });
}

/// A real saved page (R3.4): set `SCRATCHY_BENCH_PAGE=/path/to/page.html`
/// (e.g. a Wikipedia article saved through `wire`/`snag`) to track the
/// restyle cost the hover/relayout path pays. Skipped when unset — the
/// page itself is too large to commit.
fn bench_real_page(c: &mut Criterion) {
    let Ok(path) = std::env::var("SCRATCHY_BENCH_PAGE") else { return };
    let html = std::fs::read_to_string(&path).expect("SCRATCHY_BENCH_PAGE unreadable");
    let document = parse_html(&html);
    let mut group = c.benchmark_group("real_page");
    group.sample_size(10);
    group.bench_function("build_layout_tree_sized", |b| {
        b.iter(|| black_box(scratchy::build_layout_tree_sized(black_box(&document), 1052.0, 1050.0)));
    });
    group.finish();
}

/// QOBS10: the QY53 "big-css" fixture (generated here, never vendored): 20 000
/// class rules (`.c<i> .d<i> { margin-left: 1px }`), 200 `::before` rules, and
/// 1 000 nested divs with 3 classes each. `custom_props` adds 200 custom
/// properties on `:root` plus an inline override on every 10th div (QY53i);
/// `long_text` appends 100 paragraphs of ~2 000 characters (QP4).
fn big_css_html(custom_props: bool, long_text: bool) -> String {
    let mut s = String::from("<!doctype html><html><head><style>\n");
    for i in 0..20_000 {
        s.push_str(&format!(".c{i} .d{i} {{ margin-left: 1px }}\n"));
    }
    for i in 0..200 {
        s.push_str(&format!(".e{i}::before {{ content: \"x\" }}\n"));
    }
    if custom_props {
        s.push_str(":root {");
        for i in 0..200 {
            s.push_str(&format!(" --p{i}: {i}px;"));
        }
        s.push_str(" }\ndiv { padding-left: var(--p1, 0px); margin-top: var(--p2, 0px) }\n");
    }
    s.push_str("</style></head><body>\n");
    for i in 0..1000 {
        if custom_props && i % 10 == 0 {
            s.push_str(&format!("<div id=\"n{i}\" class=\"c{i} d{i} e{}\" style=\"--p{}: 7px\">", i % 200, i % 200));
        } else {
            s.push_str(&format!("<div id=\"n{i}\" class=\"c{i} d{i} e{}\">", i % 200));
        }
    }
    s.push_str("leaf");
    for _ in 0..1000 {
        s.push_str("</div>");
    }
    if long_text {
        for p in 0..100 {
            s.push_str(&format!("<p>Paragraph {p}: "));
            for w in 0..300 {
                s.push_str(&format!("word{} ", (p * 31 + w * 7) % 997));
            }
            s.push_str("</p>\n");
        }
    }
    s.push_str("</body></html>");
    s
}

fn find_by_id(h: &markup5ever_rcdom::Handle, id: &str) -> Option<markup5ever_rcdom::Handle> {
    if let markup5ever_rcdom::NodeData::Element { attrs, .. } = &h.data
        && attrs.borrow().iter().any(|a| a.name.local.as_ref() == "id" && &*a.value == id)
    {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_id(c, id))
}

fn find_tag(h: &markup5ever_rcdom::Handle, tag: &str) -> Option<markup5ever_rcdom::Handle> {
    if let markup5ever_rcdom::NodeData::Element { name, .. } = &h.data
        && name.local.as_ref() == tag
    {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_tag(c, tag))
}

/// One forced layout as the page script's `write attribute; read offsetWidth`
/// pays it: write one attribute (`data-w`), then
/// `build_layout_tree_sized` (= `build_layout_tree_with_resources` with no
/// extra CSS) + `measure_document`. With `style_append`, a small `<style>`
/// element with different text each iteration is appended to `<head>` before
/// the build (and removed after), so the stylesheet source changes and the
/// rule cache misses: that measures the stylesheet re-parse cost (QY53f1).
fn bench_big_css_variant(c: &mut Criterion, name: &str, custom_props: bool, long_text: bool, style_append: bool) {
    let document = parse_html(&big_css_html(custom_props, long_text));
    let target = find_by_id(&document, "n999").expect("innermost div");
    let head = find_tag(&document, "head").expect("head");
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut tick = 0u64;
    let mut group = c.benchmark_group("forced_layout");
    group.sample_size(10);
    group.bench_function(name, |b| {
        b.iter(|| {
            tick += 1;
            if let markup5ever_rcdom::NodeData::Element { attrs, .. } = &target.data {
                let mut attrs = attrs.borrow_mut();
                let v: html5ever::tendril::StrTendril = tick.to_string().into();
                match attrs.iter_mut().find(|a| a.name.local.as_ref() == "data-w") {
                    Some(a) => a.value = v,
                    None => attrs.push(html5ever::Attribute { name: html5ever::QualName::new(None, html5ever::ns!(), html5ever::LocalName::from("data-w")), value: v }),
                }
            }
            let extra = style_append.then(|| {
                let frag = parse_html(&format!("<style>.s{tick} {{ color: red }}</style>"));
                let style = find_tag(&frag, "style").unwrap();
                head.children.borrow_mut().push(style.clone());
                style
            });
            let tree = scratchy::build_layout_tree_sized(black_box(&document), 1052.0, 1050.0);
            if extra.is_some() {
                head.children.borrow_mut().pop();
            }
            let mut next_id = 0usize;
            let mut incomplete = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
            black_box(scratchy::measure_document(&tree, &mut font_system, 1052.0, 1050.0, &mut next_id, &mut cache, deadline, &mut incomplete))
        });
    });
    group.finish();
}

fn bench_big_css(c: &mut Criterion) {
    bench_big_css_variant(c, "big_css", false, false, false);
    bench_big_css_variant(c, "big_css, 200 custom props + overrides", true, false, false);
    bench_big_css_variant(c, "big_css, 100 long text paragraphs", false, true, false);
    bench_big_css_variant(c, "big_css, small <style> appended (stylesheet re-parse)", false, false, true);
}

criterion_group!(benches, bench_build_layout_tree, bench_measure, bench_measure_mixed_inline, bench_real_page, bench_big_css);

/// The big-css fixture nests 1 000 divs, which overflows the default 8 MB
/// main-thread stack in style/layout recursion until QSTK1 merges, so the
/// whole bench body runs on a thread with a 256 MB stack (same rule/element
/// counts as the QY53 fixture, no shallower structure).
fn main() {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            benches();
            Criterion::default().configure_from_args().final_summary();
        })
        .unwrap()
        .join()
        .unwrap();
}
