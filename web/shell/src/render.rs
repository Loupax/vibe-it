//! Drawing the shell window: the page and its dialogs and popups (smithay's Space), the
//! status bar (C3: `statusbar.rs` draws its pixels), and a cursor the page gave as a surface.

use std::time::Duration;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::utils::IsAlive;
use smithay::wayland::compositor::with_states;

use crate::state::State;

smithay::backend::renderer::element::render_elements! {
    pub ShellElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Bar=MemoryRenderBufferRenderElement<GlesRenderer>,
}

impl State {
    pub fn render(&mut self) {
        let mut elements: Vec<ShellElement> = Vec::new();
        let age = self.backend.buffer_age().unwrap_or(0);
        // C6b: the output's scale (the shell window's); the Space renders the page at it.
        let scale = self.layout.scale;
        let mut probe_line = None;
        let fb = self.backend.window_size();
        // C-FLICKER: buffers of renders the GPU has finished go back to the page.
        self.held.collect();
        let mut sync = None;
        let damage = {
            let Ok((renderer, mut framebuffer)) = self.backend.bind() else {
                return;
            };
            // Topmost first: the cursor, then the bar.
            if let CursorImageStatus::Surface(surface) = &self.cursor {
                if surface.alive() && !self.over_bar {
                    let hotspot = with_states(surface, |s| {
                        s.data_map
                            .get::<CursorImageSurfaceData>()
                            .map(|d| d.lock().unwrap().hotspot)
                            .unwrap_or_default()
                    });
                    let pos = (self.pointer_loc - hotspot.to_f64()).to_physical(scale).to_i32_round();
                    elements.extend(
                        render_elements_from_surface_tree(renderer, surface, pos, scale, 1.0, Kind::Cursor)
                            .into_iter()
                            .map(ShellElement::Surface),
                    );
                }
            }
            if let (Some(bar), Some((loc, px))) = (self.layout.bar(), self.layout.bar_physical()) {
                // C6b: painted at the framebuffer's resolution (`px` physical pixels) and drawn
                // 1:1 into the bar's logical strip: `src` is the whole buffer (buffer scale 1),
                // `size` the strip's logical size, which smithay scales back to `px`.
                self.bar.update(&self.keys.bar_view(), px.w as u32, px.h as u32, scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    loc.to_f64(),
                    &self.bar.buffer,
                    None,
                    Some(smithay::utils::Rectangle::from_size((px.w as f64, px.h as f64).into())),
                    Some(bar.size),
                    Kind::Unspecified,
                ) {
                    Ok(e) => elements.push(ShellElement::Bar(e)),
                    Err(e) => eprintln!("[web] status bar: {e:?}"),
                }
            }
            // C-PICK: the picker's list over the page, right above the bar.
            if let Some(e) = self.pick_overlay.element(renderer, &self.keys, &self.layout, &mut self.bar) {
                elements.push(ShellElement::Bar(e));
            }
            let tracker = self.damage_tracker.as_mut().unwrap();
            match smithay::desktop::space::render_output(
                &self.output,
                renderer,
                &mut framebuffer,
                1.0,
                age,
                [&self.space],
                &elements,
                tracker,
                [0.1, 0.1, 0.1, 1.0],
            ) {
                Ok(r) => {
                    // C-FLICKER: the probe reads back what this frame shows (checks only).
                    if let Some((rects, n)) = &mut self.probe {
                        *n += 1;
                        probe_line = Some(probe_frame(renderer, &mut framebuffer, rects, *n, scale, fb));
                    }
                    sync = Some(r.sync.clone());
                    r.damage.cloned()
                }
                Err(e) => {
                    eprintln!("[web] render: {e:?}");
                    None
                }
            }
        };
        if let Some(line) = probe_line {
            self.log(&line);
        }
        self.pacer.rendered(std::time::Instant::now());
        if let Some((since, n)) = &mut self.frame_stats {
            *n += 1;
            if since.elapsed() >= Duration::from_secs(5) {
                let line = format!("frames: {} renders in {:.1} s", n, since.elapsed().as_secs_f64());
                *since = std::time::Instant::now();
                *n = 0;
                self.log(&line);
            }
        }
        if let Some(damage) = damage {
            if let Err(e) = self.backend.submit(Some(&damage)) {
                self.log(&format!("submit: {e:?}"));
            }
        }
        // C-FLICKER: the page's dmabufs this render read stay ours until the GPU is done with it
        // (`sync.rs`): a new commit then does not release a buffer the GPU still samples.
        if let (Some(sync), true) = (sync, self.buffer_sync) {
            let buffers = self.drawn_dmabufs();
            self.held.push(sync, buffers);
            self.poll_held();
        }
        // C6h: presentation feedback: the page's `presented` events say when and at which refresh
        // the frame was shown; Chromium derives its BeginFrame interval from them (without
        // wp_presentation it assumes 60 Hz).
        {
            use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
            use smithay::wayland::presentation::Refresh;
            let mut feedback = smithay::desktop::utils::OutputPresentationFeedback::new(&self.output);
            let out = self.output.clone();
            for w in self.space.elements() {
                w.take_presentation_feedback(&mut feedback, |_, _| Some(out.clone()), |_, _| Kind::Vsync);
            }
            let refresh = Refresh::fixed(self.pacer.interval());
            feedback.presented(self.presentation_clock.now(), refresh, 0, Kind::Vsync);
        }
        // Frame callbacks: the page draws its next frame now.
        let time = self.start.elapsed();
        let output = self.output.clone();
        for w in self.space.elements() {
            w.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        if let CursorImageStatus::Surface(surface) = &self.cursor {
            send_frames_surface_tree(surface, &output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
    }
}

impl State {
    /// C-FLICKER: the dmabuf-backed buffers of every surface drawn (the page's window trees,
    /// popups included, and a cursor surface).
    fn drawn_dmabufs(&self) -> Vec<smithay::backend::renderer::utils::Buffer> {
        use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
        use smithay::wayland::compositor::{with_surface_tree_downward, TraversalAction};
        let mut out = Vec::new();
        let mut visit = |root: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface| {
            with_surface_tree_downward(
                root,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                // The tree walk holds the surface's lock: read its states as given (no with_states).
                |_, states, _| {
                    let data = states.data_map.get::<RendererSurfaceStateUserData>();
                    if let Some(b) = data.and_then(|d| d.lock().unwrap().buffer().cloned()) {
                        if smithay::wayland::dmabuf::get_dmabuf(&b).is_ok() {
                            out.push(b);
                        }
                    }
                },
                |_, _, _| true,
            );
        };
        for w in self.space.elements() {
            if let Some(t) = w.toplevel() {
                visit(t.wl_surface());
                for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(t.wl_surface()) {
                    visit(popup.wl_surface());
                }
            }
        }
        if let CursorImageStatus::Surface(surface) = &self.cursor {
            visit(surface);
        }
        out
    }

    /// C-FLICKER: while renders are held, look again every millisecond (a GPU frame is done
    /// in well under one outer frame), so the page gets its buffers back without waiting for
    /// the next render.
    fn poll_held(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if self.held_timer || self.held.pending() == 0 {
            return;
        }
        let armed = self
            .loop_handle
            .insert_source(Timer::from_duration(Duration::from_millis(1)), |_, _, state| {
                state.held.collect();
                if state.held.pending() == 0 {
                    state.held_timer = false;
                    TimeoutAction::Drop
                } else {
                    TimeoutAction::ToDuration(Duration::from_millis(1))
                }
            });
        self.held_timer = armed.is_ok();
    }
}

/// C-FLICKER: `probe.rs`'s rectangles of the frame just drawn, read back from the window's back
/// buffer inside a frame of its own (plain glReadPixels into memory: no surfaceless
/// make_current and no mapped pixel buffer around the swap, which NVIDIA's EGL fails with
/// EGLCreateSurface BadAlloc). Synchronous, so checks only.
fn probe_frame(
    renderer: &mut GlesRenderer,
    framebuffer: &mut smithay::backend::renderer::gles::GlesTarget<'_>,
    rects: &[crate::probe::Rect],
    n: u64,
    scale: f64,
    fb: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> String {
    use smithay::backend::renderer::gles::ffi;
    use smithay::backend::renderer::{Frame, Renderer};
    let mut line = format!("probe: frame {n}");
    let mut frame = match renderer.render(framebuffer, fb, smithay::utils::Transform::Normal) {
        Ok(f) => f,
        Err(e) => return format!("{line} | frame: {e:?}"),
    };
    for r in rects {
        line.push_str(" | ");
        let Some((x, y, w, h)) = crate::probe::to_gl(*r, scale, fb.w, fb.h) else {
            line.push_str("outside");
            continue;
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let read = frame.with_context(|gl| unsafe {
            gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
            gl.PixelStorei(ffi::PACK_ALIGNMENT, 4);
            gl.ReadPixels(x, y, w, h, ffi::RGBA, ffi::UNSIGNED_BYTE, px.as_mut_ptr().cast());
            gl.GetError()
        });
        match read {
            Ok(ffi::NO_ERROR) => {
                let (lo, hi) = crate::probe::min_max(&px);
                line.push_str(&crate::probe::entry(lo, hi));
            }
            other => line.push_str(&format!("read: {other:?}")),
        }
    }
    let _ = frame.finish();
    line
}
