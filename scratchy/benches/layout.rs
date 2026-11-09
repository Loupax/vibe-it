use criterion::{criterion_group, criterion_main, Criterion};
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

criterion_group!(benches, bench_build_layout_tree, bench_measure, bench_measure_mixed_inline);
criterion_main!(benches);
