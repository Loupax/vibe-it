//! Drawing the shell window: the page and its dialogs and popups (smithay's Space), the
//! status bar strip (a plain colour until C3), and a cursor the page gave as a surface.

use std::time::Duration;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportAll, ImportMem};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::utils::IsAlive;
use smithay::wayland::compositor::with_states;

use crate::state::State;

smithay::backend::renderer::element::render_elements! {
    pub ShellElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Solid=SolidColorRenderElement,
}

impl State {
    pub fn render(&mut self) {
        let mut elements: Vec<ShellElement<GlesRenderer>> = Vec::new();
        let age = self.backend.buffer_age().unwrap_or(0);
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
                    let pos = (self.pointer_loc - hotspot.to_f64()).to_physical(1.0).to_i32_round();
                    elements.extend(
                        render_elements_from_surface_tree(renderer, surface, pos, 1.0, 1.0, Kind::Cursor)
                            .into_iter()
                            .map(ShellElement::Surface),
                    );
                }
            }
            if let Some(bar) = self.layout.bar() {
                self.bar.resize(bar.size);
                elements.push(ShellElement::Solid(SolidColorRenderElement::from_buffer(
                    &self.bar,
                    bar.loc.to_physical(1),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )));
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
        if let Some(damage) = damage {
            if let Err(e) = self.backend.submit(Some(&damage)) {
                self.log(&format!("submit: {e:?}"));
            }
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
