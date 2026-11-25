// QM1k1k: the main (picture) stream is the video/* SourceBuffer, not index 0.
use crate::media_el::{FakeBackend, MediaBackend};
use crate::mse_store::*;

fn feed_of(types: &[&str]) -> std::sync::Arc<MseFeed> {
    let mut st = MseStore::from_env();
    for t in types {
        st.add_buffer(SourceBufferStore::fmp4().with_type(t));
    }
    MseFeed::new(st)
}

const V: &str = "video/mp4; codecs=\"avc1.640015\"";
const A: &str = "audio/mp4; codecs=\"mp4a.40.2\"";

#[test]
fn qm1k1k_track_roles_by_type() {
    assert_eq!(feed_of(&[V]).track_roles(), (0, None));
    assert_eq!(feed_of(&[A]).track_roles(), (0, None));
    assert_eq!(feed_of(&[V, A]).track_roles(), (0, Some(1)));
    assert_eq!(feed_of(&[A, V]).track_roles(), (1, Some(0)));
    // same kind: index order
    assert_eq!(feed_of(&[V, V]).track_roles(), (0, Some(1)));
    assert_eq!(feed_of(&[A, A]).track_roles(), (0, Some(1)));
    // uppercase and unknown types
    assert_eq!(feed_of(&["AUDIO/MP4", "Video/mp4"]).track_roles(), (1, Some(0)));
    assert_eq!(feed_of(&["", V]).track_roles(), (0, Some(1)));
}

#[test]
fn qm1k1k_fake_backend_loads_the_video_buffer_as_main() {
    for (types, main) in [(vec![A, V], 1), (vec![V, A], 0), (vec![A], 0), (vec![V], 0)] {
        let feed = feed_of(&types);
        let h = crate::media_el::fake_new();
        let mut f = FakeBackend(h.clone());
        f.load_mse(feed.clone(), 0.0).unwrap();
        let url = h.borrow().loaded.as_ref().unwrap().url.clone();
        assert!(url.starts_with(&format!("scratchy-mse://{}/", feed.id)) && url.ends_with(&format!("/{main}")), "{types:?} -> {url}");
    }
}

#[test]
fn qm1k1k_js_add_source_buffer_records_the_type() {
    let js = super::qm1k1d::attached();
    js.run("ms.addSourceBuffer('audio/mp4; codecs=\"mp4a.40.2\"'); ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"');");
    let feed = js.engine.media.as_ref().unwrap().borrow().mse.only().unwrap();
    assert_eq!(feed.track_roles(), (1, Some(0)));
}
