use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::thread;

use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache};
use html5ever::driver::{parse_document, ParseOpts, Parser};
use html5ever::tendril::stream::Utf8LossyDecoder;
use html5ever::tendril::{ByteTendril, TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

enum Input {
    File(String),
    Stdin,
}

fn parse_args() -> Input {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.len() {
        0 => Input::Stdin,
        1 => Input::File(args[0].clone()),
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

fn new_parser() -> Utf8LossyDecoder<Parser<RcDom>> {
    parse_document(RcDom::default(), ParseOpts::default()).from_utf8()
}

/// Events sent from the background stdin-reading thread to wake the event loop.
enum AppEvent {
    Chunk(Vec<u8>),
    Eof,
}

struct App {
    text: String,
    /// Present only in streaming (stdin) mode; `None` once EOF has been
    /// processed, or always `None` in file mode.
    decoder: Option<Utf8LossyDecoder<Parser<RcDom>>>,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl App {
    fn new_static(text: String) -> Self {
        Self {
            text,
            decoder: None,
            window: None,
            surface: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn new_streaming() -> Self {
        Self {
            text: String::new(),
            decoder: Some(new_parser()),
            window: None,
            surface: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn refresh_text_from_decoder(&mut self) {
        let Some(decoder) = &self.decoder else { return };
        let mut raw_text = String::new();
        extract_text(&decoder.inner_sink.tokenizer.sink.sink.document, &mut raw_text);
        self.text = normalize_whitespace(&raw_text);
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

impl ApplicationHandler<AppEvent> for App {
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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Chunk(bytes) => {
                if let Some(decoder) = &mut self.decoder {
                    decoder.process(ByteTendril::from_slice(&bytes));
                }
                self.refresh_text_from_decoder();
            }
            AppEvent::Eof => {
                if let Some(decoder) = self.decoder.take() {
                    let dom = decoder.finish();
                    let mut raw_text = String::new();
                    extract_text(&dom.document, &mut raw_text);
                    self.text = normalize_whitespace(&raw_text);
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
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
    let input = parse_args();

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();

    let mut app = match input {
        Input::File(path) => {
            let html = fs::read_to_string(&path).unwrap_or_else(|e| {
                eprintln!("scratchy: {e}");
                std::process::exit(1);
            });
            let dom = new_parser().read_from(&mut html.as_bytes()).unwrap();
            let mut raw_text = String::new();
            extract_text(&dom.document, &mut raw_text);
            App::new_static(normalize_whitespace(&raw_text))
        }
        Input::Stdin => {
            let proxy = event_loop.create_proxy();
            thread::spawn(move || {
                let mut stdin = io::stdin();
                let mut buf = [0u8; 8192];
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) => {
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                        Ok(n) => {
                            if proxy.send_event(AppEvent::Chunk(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                    }
                }
            });
            App::new_streaming()
        }
    };

    event_loop.run_app(&mut app).unwrap();
}
