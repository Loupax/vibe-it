//! C6d: zwp_linux_dmabuf_v1, so the page can hand the shell GPU buffers instead of wl_shm.
//!
//! Without it Chromium's Ozone/Wayland has no way to share a GPU buffer with the shell: it
//! composites in software and every frame goes through shared memory (the shell then uploads
//! it to a texture). With it the engine's GPU process renders into dmabufs it allocates on the
//! shell's GPU and the shell imports them as textures, no CPU copy.
//!
//! The global is offered only when the shell's own renderer can use it: its EGL display
//! imports dmabufs (EGL_EXT_image_dma_buf_import: a non-empty format list) and sits on a real
//! DRM device (the feedback's main device, where the client allocates). A software renderer
//! (llvmpipe, e.g. in the headless dwl nest, which renders with pixman and offers no dmabuf
//! itself) has no device to allocate on, so the shell offers only wl_shm there, as before.
//! `WEB_NO_DMABUF=1` turns it off (A/B measurements, and an escape hatch for a driver that
//! misbehaves).

use std::path::PathBuf;

use smithay::backend::allocator::Format;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufState};

use crate::state::State;

/// The env var that keeps the shell on wl_shm.
pub const DISABLE_ENV: &str = "WEB_NO_DMABUF";

/// What the shell offers the page.
#[derive(Debug, PartialEq)]
pub enum Offer {
    /// wl_shm only, and why (logged at start).
    Shm(String),
    /// zwp_linux_dmabuf_v1 (v5, with feedback) for this DRM device.
    Dmabuf { device: PathBuf, dev: u64 },
}

/// The shell's renderer as far as dmabuf goes (gathered by `probe`, decided by `decide`).
#[derive(Debug, Clone)]
pub struct Renderer {
    /// How many (fourcc, modifier) pairs it imports.
    pub formats: usize,
    /// EGL_MESA_device_software: llvmpipe and friends.
    pub software: bool,
    /// Its DRM node (render node preferred) and that node's device number, or why there is none.
    pub device: Result<(PathBuf, u64), String>,
}

/// Whether to offer dmabuf, from the renderer's facts and the env switch.
pub fn decide(disabled: bool, r: &Renderer) -> Offer {
    if disabled {
        return Offer::Shm(format!("{DISABLE_ENV} is set"));
    }
    if r.software {
        return Offer::Shm("the shell renders in software (no GPU to share buffers on)".into());
    }
    if r.formats == 0 {
        return Offer::Shm("the shell's renderer imports no dmabufs".into());
    }
    match &r.device {
        Ok((device, dev)) => Offer::Dmabuf {
            device: device.clone(),
            dev: *dev,
        },
        Err(why) => Offer::Shm(format!("no DRM device for the shell's renderer ({why})")),
    }
}

/// `WEB_NO_DMABUF` set to anything but "" or "0".
pub fn disabled_by(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

/// The renderer's dmabuf facts.
pub fn probe(renderer: &GlesRenderer) -> Renderer {
    let formats = renderer.dmabuf_formats().iter().count();
    let (software, device) = match EGLDevice::device_for_display(renderer.egl_context().display()) {
        Ok(d) => {
            let path = d.render_device_path().or_else(|_| d.drm_device_path()).map_err(|e| e.to_string());
            let device = path.and_then(|p| {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(&p)
                    .map(|m| (p.clone(), m.rdev()))
                    .map_err(|e| format!("{}: {e}", p.display()))
            });
            (d.is_software(), device)
        }
        Err(e) => (false, Err(format!("no EGL device: {e}"))),
    };
    Renderer { formats, software, device }
}

/// The default feedback: one tranche, the renderer's formats on its device.
pub fn feedback(dev: u64, formats: impl IntoIterator<Item = Format>) -> std::io::Result<DmabufFeedback> {
    DmabufFeedbackBuilder::new(dev as libc::dev_t, formats).build()
}

/// The dmabuf global, when `decide` says so; the log line says what was offered either way.
pub struct Dmabuf {
    pub state: DmabufState,
    pub _global: Option<DmabufGlobal>,
}

pub fn setup(renderer: &GlesRenderer, dh: &DisplayHandle, env_value: Option<&str>) -> (Dmabuf, String) {
    let mut state = DmabufState::new();
    let r = probe(renderer);
    let line = match decide(disabled_by(env_value), &r) {
        Offer::Shm(why) => format!("dmabuf: not offered, wl_shm only: {why}"),
        Offer::Dmabuf { device, dev } => match feedback(dev, renderer.dmabuf_formats()) {
            Ok(fb) => {
                let global = state.create_global_with_default_feedback::<State>(dh, &fb);
                return (
                    Dmabuf {
                        state,
                        _global: Some(global),
                    },
                    format!("dmabuf: offered ({} formats on {})", r.formats, device.display()),
                );
            }
            Err(e) => format!("dmabuf: not offered, wl_shm only: feedback: {e}"),
        },
    };
    (Dmabuf { state, _global: None }, line)
}
