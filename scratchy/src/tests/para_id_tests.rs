// QINC1b (R60 4.4): paragraph ids come from a counter that never restarts; the persistent
// `ShapeCache` is swept of ids no live `Laid` references.
use super::*;
use crate::layout::{collect_live_ids, sweep_shape_cache};
use std::collections::HashSet;

/// Lays out `html` with ids starting at `*next_id`, into `cache`; advances the counter.
fn layout_from(html: &str, next_id: &mut usize, cache: &mut ShapeCache) -> Laid {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    measure_document(&tree, &mut font_system, 400.0, 300.0, next_id, cache, deadline, &mut incomplete)
}

fn live(l: &Laid) -> HashSet<usize> {
    let mut s = HashSet::new();
    collect_live_ids(l, &mut s);
    s
}

fn text_of(cache: &ShapeCache, id: usize) -> String {
    let g = cache.get(&id).expect("live id has an entry").lock().unwrap();
    g.0.layout_runs().map(|r| r.text.to_string()).collect::<Vec<_>>().join("\n")
}

#[test]
fn two_layouts_never_share_an_id_for_different_text() {
    let mut cache = ShapeCache::new();
    let mut next = 0usize;
    let a = layout_from("<p>alpha one</p><p>beta two</p>", &mut next, &mut cache);
    let after_a = next;
    let ids_a = live(&a);
    assert!(!ids_a.is_empty());
    let b = layout_from("<p>gamma three</p><p>delta four</p><p>epsilon</p>", &mut next, &mut cache);
    let ids_b = live(&b);
    assert!(next > after_a);
    assert!(ids_a.is_disjoint(&ids_b), "{ids_a:?} vs {ids_b:?}");
    // Both trees still read their own text from the one cache.
    assert!(ids_a.iter().any(|&i| text_of(&cache, i).contains("alpha")));
    assert!(ids_b.iter().any(|&i| text_of(&cache, i).contains("gamma")));
}

#[test]
fn nonzero_base_still_shapes_every_paragraph() {
    let mut cache = ShapeCache::new();
    let mut next = 5000usize;
    let l = layout_from("<p>hello world</p><p>second <b>bold</b> para</p>", &mut next, &mut cache);
    let ids = live(&l);
    assert!(ids.iter().all(|&i| i >= 5000));
    for id in ids {
        let g = cache.get(&id).expect("entry").lock().unwrap();
        assert!(g.0.layout_runs().next().is_some(), "id {id} has no shaped runs");
    }
}

#[test]
fn sweep_keeps_every_live_id_and_drops_stale_ones() {
    let mut cache = ShapeCache::new();
    let mut next = 0usize;
    let _old = layout_from("<p>a</p><p>b</p><p>c</p><p>d</p><p>e</p><p>f</p>", &mut next, &mut cache);
    let new = layout_from("<p>x</p>", &mut next, &mut cache);
    let ids = live(&new);
    let total = cache.len();
    assert!(total > 2 * ids.len());
    let dropped = sweep_shape_cache(&mut cache, &ids);
    assert_eq!(dropped, total - ids.len());
    assert!(ids.iter().all(|i| cache.contains_key(i)));
    assert!(cache.keys().all(|i| ids.contains(i)));
}

#[test]
fn sweep_leaves_a_cache_within_twice_the_live_count() {
    let mut cache = ShapeCache::new();
    let mut next = 0usize;
    let a = layout_from("<p>a</p><p>b</p>", &mut next, &mut cache);
    let b = layout_from("<p>c</p><p>d</p>", &mut next, &mut cache);
    let before = cache.len();
    assert_eq!(sweep_shape_cache(&mut cache, &live(&b)), 0);
    assert_eq!(cache.len(), before);
    assert!(live(&a).iter().all(|i| cache.contains_key(i)));
}
