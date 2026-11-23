//! C6d: when the shell offers zwp_linux_dmabuf_v1 and when it stays on wl_shm (`dmabuf.rs`).

use std::path::PathBuf;

use smithay::backend::allocator::{Format, Fourcc, Modifier};

use crate::dmabuf::{decide, disabled_by, feedback, Offer, Renderer};

fn gpu() -> Renderer {
    Renderer {
        formats: 672,
        software: false,
        device: Ok((PathBuf::from("/dev/dri/renderD128"), 0xe280)),
    }
}

fn is_shm(o: &Offer, why: &str) -> bool {
    matches!(o, Offer::Shm(w) if w.contains(why))
}

#[test]
fn a_gpu_renderer_offers_dmabuf_on_its_device() {
    assert_eq!(
        decide(false, &gpu()),
        Offer::Dmabuf {
            device: PathBuf::from("/dev/dri/renderD128"),
            dev: 0xe280
        }
    );
}

#[test]
fn software_rendering_stays_on_shm() {
    // llvmpipe in the headless dwl nest: formats or not, there is no GPU to allocate on.
    let r = Renderer { software: true, ..gpu() };
    assert!(is_shm(&decide(false, &r), "software"));
}

#[test]
fn no_importable_formats_stays_on_shm() {
    let r = Renderer { formats: 0, ..gpu() };
    assert!(is_shm(&decide(false, &r), "imports no dmabufs"));
}

#[test]
fn no_drm_device_stays_on_shm() {
    let r = Renderer {
        device: Err("no EGL device: EGL_EXT_device_query missing".into()),
        ..gpu()
    };
    let o = decide(false, &r);
    assert!(is_shm(&o, "no DRM device"), "{o:?}");
    assert!(is_shm(&o, "EGL_EXT_device_query"), "the reason is kept: {o:?}");
}

#[test]
fn the_env_switch_wins() {
    assert!(is_shm(&decide(true, &gpu()), "WEB_NO_DMABUF"));
}

#[test]
fn env_values() {
    assert!(!disabled_by(None));
    assert!(!disabled_by(Some("")));
    assert!(!disabled_by(Some("0")));
    assert!(disabled_by(Some("1")));
    assert!(disabled_by(Some("yes")));
}

#[test]
fn feedback_builds_without_a_gpu() {
    let formats = [
        Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Abgr8888,
            modifier: Modifier::Invalid,
        },
    ];
    // The format table is a sealed memfd the clients map; building it needs no GPU.
    feedback(0xe280, formats).expect("feedback builds");
}
