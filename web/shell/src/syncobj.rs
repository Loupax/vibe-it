//! C-FLICKER: linux-drm-syncobj-v1 (explicit sync) for the page's dmabufs.
//!
//! With it the engine attaches every dmabuf with an acquire point (signalled when its GPU has
//! finished drawing the buffer) and a release point (which the shell signals when it is done
//! reading it). The shell holds a commit until its acquire point is signalled (`sync.rs`) and
//! signals the release point only after the GPU finished the last render that read the buffer
//! (`sync::Held`). It is the only reliable sync on drivers whose dmabufs carry no implicit
//! fences (NVIDIA's, the user's GPU), where the engine otherwise commits half-drawn frames.
//!
//! Offered together with zwp_linux_dmabuf_v1 (C6d), on the same DRM render node, when the
//! kernel driver supports syncobj eventfds. `WEB_NO_SYNCOBJ=1` turns it off (A/B checks).

use std::os::fd::OwnedFd;
use std::path::Path;

use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::DeviceFd;
use smithay::wayland::compositor::{add_blocker, CompositorHandler, SurfaceData};
use smithay::wayland::drm_syncobj::{supports_syncobj_eventfd, DrmSyncPoint, DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState};

use crate::state::State;

/// The env var that keeps explicit sync off.
pub const DISABLE_ENV: &str = "WEB_NO_SYNCOBJ";

/// Whether to offer explicit sync, and why not (logged at start).
pub fn decide(dmabuf_offered: bool, disabled: bool, eventfd: impl FnOnce() -> Result<bool, String>) -> Result<(), String> {
    if !dmabuf_offered {
        return Err("no dmabuf".into());
    }
    if disabled {
        return Err(format!("turned off ({DISABLE_ENV} or {})", crate::sync::DISABLE_ENV));
    }
    match eventfd() {
        Ok(true) => Ok(()),
        Ok(false) => Err("the DRM device has no syncobj eventfd".into()),
        Err(e) => Err(e),
    }
}

/// The global on `device` (the dmabuf feedback's render node) when `decide` allows it; the log
/// line says what happened either way.
pub fn setup(dh: &DisplayHandle, device: Option<&Path>, disabled: bool) -> (Option<DrmSyncobjState>, String) {
    let mut fd = None;
    let verdict = decide(device.is_some(), disabled, || {
        let path = device.unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let drm = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
        let ok = supports_syncobj_eventfd(&drm);
        fd = Some(drm);
        Ok(ok)
    });
    match (verdict, fd, device) {
        (Ok(()), Some(drm), Some(path)) => (
            Some(DrmSyncobjState::new::<State>(dh, drm)),
            format!("explicit sync: offered (linux-drm-syncobj-v1 on {})", path.display()),
        ),
        (Err(why), _, _) => (None, format!("explicit sync: not offered: {why}")),
        _ => (None, "explicit sync: not offered: no device".into()),
    }
}

/// The acquire point of a commit being made, if the client set one.
pub fn pending_acquire(states: &SurfaceData) -> Option<DrmSyncPoint> {
    states.cached_state.get::<DrmSyncobjCachedState>().pending().acquire_point.clone()
}

/// Hold the surface's commit until `point` is signalled; false when that cannot be arranged
/// (the caller then falls back to the implicit fence).
pub fn block_on(handle: &LoopHandle<'static, State>, surface: &WlSurface, point: DrmSyncPoint) -> bool {
    use smithay::backend::renderer::sync::Fence;
    if point.is_signaled() {
        return true;
    }
    let Ok((blocker, source)) = point.generate_blocker() else {
        return false;
    };
    let Some(client) = surface.client() else {
        return false;
    };
    let res = handle.insert_source(source, move |_, _, state| {
        let dh = state.display_handle.clone();
        state.client_compositor_state(&client).blocker_cleared(state, &dh);
        Ok(())
    });
    if res.is_err() {
        return false;
    }
    add_blocker(surface, blocker);
    true
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.syncobj.as_mut()
    }
}

smithay::delegate_drm_syncobj!(State);
