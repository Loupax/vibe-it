//! C-FLICKER: buffer synchronisation for the page's GPU buffers (dmabufs, C6d).
//!
//! The engine commits a dmabuf as soon as it has *queued* its drawing on the GPU, not when the
//! GPU has finished it. Sampling it straight away shows a frame half drawn: the layers drawn
//! last (a page's fixed header, sidebar and overlays, drawn on top of the scrolled content)
//! are missing for that frame, so they flicker while the page scrolls. Two guards:
//!
//! - **Acquire**: a commit that attaches a dmabuf is held back (a smithay commit blocker) until
//!   the buffer is ready: the client's explicit acquire point (linux-drm-syncobj-v1, when the
//!   client uses it) or else the buffer's implicit fence (the dmabuf fd polls readable).
//! - **Release**: a buffer the shell has drawn from is released to the client (wl_buffer.release,
//!   or the explicit release point) only after the GPU has finished that draw, so the engine
//!   never draws into a buffer the shell is still reading (`Held`).
//!
//! `WEB_NO_BUFFER_SYNC=1` turns both off (A/B checks: the page's buffers are then sampled as
//! soon as they are committed and released at the next commit, as before C-FLICKER).

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::utils::Buffer;
use smithay::reexports::calloop::{Interest, LoopHandle};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::compositor::{add_blocker, add_pre_commit_hook, with_states, BufferAssignment, CompositorHandler, SurfaceAttributes};
use smithay::wayland::dmabuf::get_dmabuf;

use crate::state::State;

/// The env var that turns the guards off.
pub const DISABLE_ENV: &str = "WEB_NO_BUFFER_SYNC";

/// Install the acquire guard on a new surface (`CompositorHandler::new_surface`).
pub fn guard_surface(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _dh, surface| {
        if !state.buffer_sync {
            return;
        }
        let (dmabuf, acquire) = with_states(surface, |s| {
            let dmabuf = match s.cached_state.get::<SurfaceAttributes>().pending().buffer.as_ref() {
                Some(BufferAssignment::NewBuffer(b)) => get_dmabuf(b).cloned().ok(),
                _ => None,
            };
            (dmabuf, crate::syncobj::pending_acquire(s))
        });
        let Some(dmabuf) = dmabuf else {
            return;
        };
        if let Some(acquire) = acquire {
            if crate::syncobj::block_on(&state.loop_handle, surface, acquire) {
                return;
            }
        }
        block_on_implicit(&state.loop_handle, surface, &dmabuf);
    });
}

/// Hold the commit until the dmabuf's implicit fence has signalled (nothing to do when it
/// already has, or when the driver attaches no fence: the fd then polls ready at once).
fn block_on_implicit(handle: &LoopHandle<'static, State>, surface: &WlSurface, dmabuf: &Dmabuf) {
    let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) else {
        return;
    };
    let Some(client) = surface.client() else {
        return;
    };
    let res = handle.insert_source(source, move |_, _, state| {
        let dh = state.display_handle.clone();
        state.client_compositor_state(&client).blocker_cleared(state, &dh);
        Ok(())
    });
    if res.is_ok() {
        add_blocker(surface, blocker);
    }
}

/// Buffers the last renders drew from, each kept alive (not released to the client) until the
/// GPU has finished the render that used it (`B`: smithay's `Buffer`; generic for the tests).
pub struct Held<B = Buffer> {
    frames: Vec<(SyncPoint, Vec<B>)>,
}

impl<B> Default for Held<B> {
    fn default() -> Self {
        Held { frames: Vec::new() }
    }
}

impl<B> Held<B> {
    /// A render was submitted: keep `buffers` until `sync` is reached.
    pub fn push(&mut self, sync: SyncPoint, buffers: Vec<B>) {
        if buffers.is_empty() {
            return;
        }
        self.frames.push((sync, buffers));
    }

    /// Let go of the buffers of every render the GPU has finished (their release goes out when
    /// the surface no longer uses them either).
    pub fn collect(&mut self) {
        self.frames.retain(|(sync, _)| !sync.is_reached());
    }

    /// How many renders are still waiting.
    pub fn pending(&self) -> usize {
        self.frames.len()
    }
}
