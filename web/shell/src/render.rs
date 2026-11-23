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
                Ok(r) => r.damage.cloned(),
                Err(e) => {
                    eprintln!("[web] render: {e:?}");
                    None
                }
            }
        };
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
