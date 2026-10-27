use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::Rc;

use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache};
use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

fn read_input() -> io::Result<String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.len() {
        0 => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
        1 => fs::read_to_string(&args[0]),
        _ => {
            eprintln!("usage: scratchy [file]");
            std::process::exit(1);
        }
    }
}

fn extract_text(handle: &Handle, out: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => {
            out.push_str(&contents.borrow());
            out.push(' ');
        }
        NodeData::Element { name, .. } => {
            let tag = name.local.as_ref();
            if tag == "script" || tag == "style" {
                return;
            }
        }
        _ => {}
    }
    for child in handle.children.borrow().iter() {
        extract_text(child, out);
    }
}

fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct App {
    text: String,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl App {
    fn new(text: String) -> Self {
        Self {
            text,
            window: None,
            surface: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        surface
            .resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            )
            .unwrap();

        let metrics = Metrics::new(16.0, 20.0);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(width as f32), Some(height as f32));
        buffer.set_text(&self.text, &Attrs::new(), Shaping::Advanced, None);

        let mut pixels = surface.buffer_mut().unwrap();
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
        }

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            TextColor::rgb(0, 0, 0),
            |x, y, w, h, color| {
                if color.a() == 0 {
                    return;
                }
                for dy in 0..h as i32 {
                    for dx in 0..w as i32 {
                        let px = x + dx;
                        let py = y + dy;
                        if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
                            continue;
                        }
                        let idx = (py as u32 * width + px as u32) as usize;
                        let a = color.a() as u32;
                        let inv = 255 - a;
                        let r = (color.r() as u32 * a + 255 * inv) / 255;
                        let g = (color.g() as u32 * a + 255 * inv) / 255;
                        let b = (color.b() as u32 * a + 255 * inv) / 255;
                        pixels[idx] = (r << 16) | (g << 8) | b;
                    }
                }
            },
        );

        pixels.present().unwrap();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("scratchy")
            .with_inner_size(LogicalSize::new(1024.0, 768.0));
        let window = Rc::new(event_loop.create_window(window_attrs).unwrap());
        let context = Context::new(window.clone()).unwrap();
        let surface = Surface::new(&context, window.clone()).unwrap();

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let html = match read_input() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("scratchy: {e}");
            std::process::exit(1);
        }
    };

    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut raw_text = String::new();
    extract_text(&dom.document, &mut raw_text);
    let text = normalize_whitespace(&raw_text);

    let event_loop = EventLoop::new().unwrap();
    let mut app = App::new(text);
    event_loop.run_app(&mut app).unwrap();
}
