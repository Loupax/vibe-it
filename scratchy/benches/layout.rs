use criterion::{criterion_group, criterion_main, Criterion};
use scratchy::{build_layout_tree, measure, new_font_system, parse_html, ShapeCache};
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
        b.iter(|| build_layout_tree(black_box(&document)));
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
    let tree = build_layout_tree(&document);
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
                &mut next_id,
                &mut cache,
                deadline,
                &mut incomplete,
            )
        });
    });
}

criterion_group!(benches, bench_build_layout_tree, bench_measure);
criterion_main!(benches);
