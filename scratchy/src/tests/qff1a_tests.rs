//! QFF1a: `@font-face` web fonts (src/webfonts.rs): parsing, woff decoding, fetching through a test
//! wire (CORS), loading into the font systems, layout with the face's metrics, removal on navigation,
//! and `document.fonts` / `FontFace` on real loads.
#[allow(unused_imports)]
use super::*;
use crate::webfonts::{self as wf, FaceStyle, FontSrc};

// ---------- a generated Ahem-like font: every ASCII glyph is a 1em box ----------

fn be16(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&(x as u16).to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// The tables of a TrueType font named `family`: units per em 1000, ascent 800, descent 200, glyph 1
/// (a 1000 x 1000 box, advance 1000) for U+0020..U+007E.
fn box_font_tables(family: &str) -> Vec<([u8; 4], Vec<u8>)> {
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, 1000);
    head.extend_from_slice(&[0; 16]);
    for x in [0, -200, 1000, 800] {
        be16(&mut head, x);
    }
    for x in [0, 8, 2, 0, 0] {
        be16(&mut head, x);
    }
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for x in [800, -200, 0, 1000, 0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0, 2] {
        be16(&mut hhea, x);
    }
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    for x in [2, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }
    let mut hmtx = Vec::new();
    for x in [1000, 0, 1000, 0] {
        be16(&mut hmtx, x);
    }
    let mut glyf = Vec::new();
    for x in [1, 0, -200, 1000, 800, 3, 0] {
        be16(&mut glyf, x);
    }
    glyf.extend_from_slice(&[1, 1, 1, 1]);
    for x in [0, 1000, 0, -1000, -200, 0, 1000, 0] {
        be16(&mut glyf, x);
    }
    let mut loca = Vec::new();
    for x in [0, 0, glyf.len() as i32 / 2] {
        be16(&mut loca, x);
    }
    // cmap: one format 4 subtable (3, 1); U+0020..U+007E -> glyph 1 through the glyph id array.
    let mut sub = Vec::new();
    for x in [4, 14 + 16 + 95 * 2, 0, 4, 4, 1, 0, 0x7E, 0xFFFF, 0, 0x20, 0xFFFF, 0, 1, 4, 0] {
        be16(&mut sub, x);
    }
    for _ in 0x20..=0x7E {
        be16(&mut sub, 1);
    }
    let mut cmap = Vec::new();
    for x in [0, 1, 3, 1] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 12);
    cmap.extend_from_slice(&sub);
    let utf16 = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
    let (fam, ps) = (utf16(family), utf16(&family.replace(' ', "")));
    let mut name = Vec::new();
    for x in [0, 2, 30] {
        be16(&mut name, x);
    }
    for (id, len, off) in [(1, fam.len(), 0), (6, ps.len(), fam.len())] {
        for x in [3, 1, 0x409, id, len as i32, off as i32] {
            be16(&mut name, x);
        }
    }
    name.extend_from_slice(&fam);
    name.extend_from_slice(&ps);
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, -100);
    be16(&mut post, 50);
    post.extend_from_slice(&[0; 20]);
    let mut t = vec![
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    t.sort_by_key(|x| x.0);
    t
}

fn sfnt(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let n = tables.len();
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for x in [n as i32, 128, 3, n as i32 * 16 - 128] {
        be16(&mut out, x);
    }
    let mut off = 12 + 16 * n;
    for (tag, data) in tables {
        out.extend_from_slice(tag);
        be32(&mut out, 0);
        be32(&mut out, off as u32);
        be32(&mut out, data.len() as u32);
        off += data.len().div_ceil(4) * 4;
    }
    for (_, data) in tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    out
}

/// The same tables as WOFF 1.0, every table zlib-compressed.
fn woff(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    use std::io::Write as _;
    let n = tables.len();
    let comp: Vec<Vec<u8>> = tables
        .iter()
        .map(|(_, d)| {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
            e.write_all(d).unwrap();
            let c = e.finish().unwrap();
            if c.len() < d.len() { c } else { d.clone() }
        })
        .collect();
    let mut body = Vec::new();
    let mut dir = Vec::new();
    let mut off = 44 + 20 * n;
    for ((tag, d), c) in tables.iter().zip(&comp) {
        dir.extend_from_slice(tag);
        be32(&mut dir, off as u32);
        be32(&mut dir, c.len() as u32);
        be32(&mut dir, d.len() as u32);
        be32(&mut dir, 0);
        body.extend_from_slice(c);
        body.resize(body.len().div_ceil(4) * 4, 0);
        off = 44 + 20 * n + body.len();
    }
    let total = 44 + dir.len() + body.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"wOFF");
    be32(&mut out, 0x0001_0000);
    be32(&mut out, total as u32);
    be16(&mut out, n as i32);
    be16(&mut out, 0);
    be32(&mut out, sfnt(tables).len() as u32);
    be16(&mut out, 1);
    be16(&mut out, 0);
    out.extend_from_slice(&[0; 20]);
    out.extend_from_slice(&dir);
    out.extend_from_slice(&body);
    out
}

// ---------- a test wire: routes by path, records the requests ----------

struct TestWire {
    path: std::path::PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Drop for TestWire {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Serves `routes` (path suffix -> extra headers, body); anything else is a 404.
fn test_wire(name: &str, routes: Vec<(&'static str, &'static str, Vec<u8>)>) -> TestWire {
    use std::os::unix::net::UnixListener;
    let path = std::env::temp_dir().join(format!("scratchy-qff1a-{name}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind test wire");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let routes = Arc::new(routes);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (s, routes) = (s.clone(), routes.clone());
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let target = text.split_whitespace().nth(1).unwrap_or("").to_string();
                s.lock().unwrap().push(text);
                let resp = match routes.iter().find(|(p, _, _)| target.ends_with(p)) {
                    Some((_, headers, body)) => {
                        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: font/ttf\r\nX-Wire-Version: 3\r\n{headers}\r\n").into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\n\r\nnope".to_vec(),
                };
                let _ = stream.write_all(&resp);
            });
        }
    });
    TestWire { path, seen }
}

// ---------- layout helpers ----------

/// Width of the widest box narrower than the viewport (the float holding the text).
fn float_width(laid: &Laid) -> f32 {
    fn collect(l: &Laid, out: &mut Vec<f32>) {
        if let LaidContent::Blocks(c) = &l.content {
            for k in c {
                out.push(k.w);
                collect(k, out);
            }
        }
    }
    let mut ws = Vec::new();
    collect(laid, &mut ws);
    ws.into_iter().filter(|w| *w < 700.0).fold(0.0, f32::max)
}

fn lay_out(doc: &Handle, fs: &mut FontSystem) -> Laid {
    let tree = build_layout_tree(doc, 800.0);
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    measure(&tree, fs, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None)
}

fn float_page(css: &str, family: &str) -> String {
    format!("<html><head><style>{css}</style></head><body style=\"margin:0\"><div style=\"float:left;font:20px/20px '{family}', monospace\">XXXX</div></body></html>")
}

// ---------- tests ----------

#[test]
fn qff1a_parses_font_face_rules_also_in_media_and_supports() {
    let css = "@font-face { font-family: 'Fancy  Sans'; src: local(\"Fancy Sans Bold\"), url(a.woff2) format('woff2'), url('b.woff') format(\"woff\"), url(c.ttf); font-weight: 100 900; font-style: italic; unicode-range: U+0-7F, U+4??; font-display: swap }\
               @media (min-width: 1px) { @font-face { font-family: M; src: url(m.ttf) } }\
               @supports (display: block) { @font-face { font-family: S; src: url(s.otf) format(opentype) } }\
               @supports (nonsense-prop: 1) { @font-face { font-family: Never; src: url(n.ttf) } }\
               @font-face { src: url(no-family.ttf) }\
               p { color: red }";
    let mut rules = Vec::new();
    let _ = parse_stylesheet_into(css, &mut rules);
    let faces = wf::take_parsed();
    assert_eq!(rules.len(), 1, "@font-face makes no style rule");
    let fams: Vec<&str> = faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(fams, ["Fancy Sans", "M", "S"]);
    let f = &faces[0];
    assert_eq!(
        f.srcs,
        [
            FontSrc::Local("Fancy Sans Bold".into()),
            FontSrc::Url { url: "a.woff2".into(), format: Some("'woff2'".into()) },
            FontSrc::Url { url: "b.woff".into(), format: Some("\"woff\"".into()) },
            FontSrc::Url { url: "c.ttf".into(), format: None },
        ]
    );
    assert_eq!(f.weight, Some((100, 900)));
    assert_eq!(f.style, FaceStyle::Italic);
    assert_eq!(f.ranges, [(0, 0x7F), (0x400, 0x4FF)]);
    assert!(f.media.is_empty());
    assert!(!faces[1].media.is_empty(), "the @media condition is kept");
    assert_eq!(f.key(), "fancy sans|local(Fancy Sans Bold)");
    assert_eq!(wf::parse_weight("bold"), Some(Some((700, 700))));
    assert_eq!(wf::parse_weight("auto"), Some(None));
}

#[test]
fn qff1a_source_formats_woff2_is_skipped() {
    assert!(wf::src_supported("x.ttf", None));
    assert!(wf::src_supported("x.woff?v=2", None));
    assert!(wf::src_supported("x", Some("'truetype'")));
    assert!(!wf::src_supported("x.woff2", None));
    assert!(!wf::src_supported("x.woff", Some("woff2")));
    assert!(!wf::src_supported("x.eot?#iefix", None));
    assert!(!wf::src_supported("x.svg#f", None));
    assert!(!wf::src_supported("data:font/woff2;base64,AAAA", None));
}

#[test]
fn qff1a_woff_decodes_to_the_same_font() {
    let tables = box_font_tables("Woff Box");
    let raw = sfnt(&tables);
    let w = woff(&tables);
    assert!(w.len() < raw.len(), "compressed");
    assert_eq!(wf::woff_to_sfnt(&w).as_deref(), Some(&raw[..]), "same sfnt as the uncompressed font");
    let face = wf::decode_face(w).expect("a face");
    assert_eq!(face.families[0].0, "Woff Box");
    assert!(wf::decode_face(b"wOF2 not supported".to_vec()).is_none());
    assert!(wf::decode_face(b"garbage!".to_vec()).is_none());
}

#[test]
fn qff1a_box_font_metrics_through_a_test_wire_and_removal_on_navigation() {
    let tables = box_font_tables("Test Box");
    let wire = test_wire("metrics", vec![("/box.woff", "", woff(&tables))]);
    // woff2 first (skipped without a request), then a 404, then the woff.
    let css = "@font-face { font-family: 'Test Box'; src: url(http://fonts.test/box.woff2) format('woff2'), url(http://fonts.test/missing.ttf), url(http://fonts.test/box.woff) format('woff') }";
    let doc = parse_html(&float_page(css, "Test Box"));
    let mut fs = new_font_system();
    let mut sync = wf::FontSync::default();
    wf::sync(&mut fs, &mut sync);
    let before = float_width(&lay_out(&doc, &mut fs));
    let jobs = wf::take_wanted();
    assert_eq!(jobs.len(), 1, "the face was asked for by layout");
    let page = Url::parse("http://fonts.test/page.html").unwrap();
    let job = &jobs[0];
    let face = wf::load_sources(Some(&wire.path), &page, &page, &page, &job.srcs, &HashMap::new());
    assert!(face.is_some(), "the woff source decoded");
    assert!(wf::finish_load(&job.key, job.page, face));
    assert_eq!(wf::state_of(&job.key), wf::LoadState::Loaded);
    let seen = wire.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| !r.contains("box.woff2")), "woff2 is never requested: {seen:?}");
    assert!(seen.iter().any(|r| r.contains("missing.ttf")) && seen.iter().any(|r| r.contains("/box.woff ")), "{seen:?}");
    assert!(wf::sync(&mut fs, &mut sync), "the face is added to the font system");
    let after = float_width(&lay_out(&doc, &mut fs));
    assert!((after - 80.0).abs() < 0.01, "four 1em boxes at 20px: {after} (fallback was {before})");
    let mangled = wf::loaded_family("Test Box").expect("mangled family");
    assert!(mangled.starts_with("test box ") && mangled != "Test Box", "{mangled}");
    // Navigation: the face goes from the registry and from the font system.
    wf::new_page();
    assert!(wf::sync(&mut fs, &mut sync));
    assert_eq!(wf::face_count(), 0);
    assert!(wf::loaded_family("Test Box").is_none());
    assert!(!fs.db().faces().any(|f| f.families.iter().any(|(n, _)| n == mangled)), "removed from the database");
    let doc2 = parse_html(&float_page("", "Test Box"));
    let again = float_width(&lay_out(&doc2, &mut fs));
    assert!((again - before).abs() < 0.01, "the next page lays out with the fallback again: {again} vs {before}");
}

#[test]
fn qff1a_font_systems_replaying_the_log_get_the_same_ids() {
    let tables = box_font_tables("Id Box");
    let bytes = sfnt(&tables);
    let mut a = new_font_system();
    let mut sa = wf::FontSync::default();
    // Page 1: two faces; `a` syncs; navigation; page 2: one face.
    let css = "@font-face { font-family: IdA; src: url(a.ttf) } @font-face { font-family: IdB; src: url(b.ttf) }";
    let mut rules = Vec::new();
    let _ = parse_stylesheet_into(css, &mut rules);
    let faces = Rc::new(wf::take_parsed());
    wf::publish_declared(&faces);
    for f in faces.iter() {
        assert!(wf::finish_load(&f.key(), wf::page(), wf::decode_face(bytes.clone())));
    }
    wf::sync(&mut a, &mut sa);
    wf::new_page();
    wf::sync(&mut a, &mut sa);
    let css2 = "@font-face { font-family: IdC; src: url(c.ttf) }";
    let _ = parse_stylesheet_into(css2, &mut rules);
    let faces2 = Rc::new(wf::take_parsed());
    wf::publish_declared(&faces2);
    assert!(wf::finish_load(&faces2[0].key(), wf::page(), wf::decode_face(bytes.clone())));
    wf::sync(&mut a, &mut sa);
    // `b` joins late and replays everything at once.
    let mut b = new_font_system();
    let mut sb = wf::FontSync::default();
    wf::sync(&mut b, &mut sb);
    let ids = |fs: &FontSystem| {
        let name = wf::loaded_family("IdC").unwrap();
        fs.db().faces().filter(|f| f.families[0].0 == name).map(|f| f.id).collect::<Vec<_>>()
    };
    assert_eq!(ids(&a).len(), 1);
    assert_eq!(ids(&a), ids(&b), "same id in both databases");
}

#[test]
fn qff1a_cross_origin_font_needs_cors() {
    let bytes = sfnt(&box_font_tables("Cors Box"));
    let wire = test_wire("cors", vec![("/open.ttf", "Access-Control-Allow-Origin: *\r\n", bytes.clone()), ("/closed.ttf", "", bytes.clone())]);
    let page = Url::parse("http://site.test/").unwrap();
    let open = Url::parse("http://cdn.test/open.ttf").unwrap();
    let closed = Url::parse("http://cdn.test/closed.ttf").unwrap();
    assert!(wf::fetch_font_bytes(&wire.path, &open, &page, &page).is_some());
    assert!(wf::fetch_font_bytes(&wire.path, &closed, &page, &page).is_none(), "no ACAO: blocked");
    let same = Url::parse("http://site.test/closed.ttf").unwrap();
    assert!(wf::fetch_font_bytes(&wire.path, &same, &page, &page).is_some(), "same origin needs no CORS header");
    let seen = wire.seen.lock().unwrap().clone();
    let cross = seen.iter().find(|r| r.contains("cdn.test/open.ttf")).unwrap();
    assert!(cross.contains("Origin: http://site.test\r\n"), "{cross}");
    let same_req = seen.iter().find(|r| r.contains("site.test/closed.ttf")).unwrap();
    assert!(!same_req.contains("Origin:"), "{same_req}");
}

#[test]
fn qff1a_local_source_uses_the_system_font() {
    let fs = new_font_system();
    let Some(sys) = fs.db().faces().find(|f| !f.families.is_empty()) else { return };
    let name = sys.families[0].0.clone();
    let face = wf::local_face(fs.db(), &name.to_uppercase()).expect("found by family, case-insensitively");
    assert_eq!(face.families[0].0, name);
    assert!(wf::local_face(fs.db(), "No Such Font Anywhere 12345").is_none());
}

#[test]
fn qff1a_best_face_by_style_then_weight_is_the_one_fetched() {
    let css = "@font-face { font-family: W; src: url(w400.ttf); font-weight: 400 }\
               @font-face { font-family: W; src: url(w700.ttf); font-weight: 700 }\
               @font-face { font-family: W; src: url(w400i.ttf); font-weight: 400; font-style: italic }\
               @font-face { font-family: W; src: url(cyr.ttf); font-weight: 400; unicode-range: U+0400-04FF }";
    let mut rules = Vec::new();
    let _ = parse_stylesheet_into(css, &mut rules);
    wf::publish_declared(&Rc::new(wf::take_parsed()));
    let _ = wf::take_wanted();
    let want = |w: u16, s: FontStyle| {
        let _ = wf::lookup("w", FontWeight(w), s);
        wf::take_wanted().into_iter().map(|j| j.key).collect::<Vec<_>>()
    };
    assert_eq!(want(400, FontStyle::Normal), ["w|w400.ttf"]);
    assert_eq!(want(400, FontStyle::Normal), Vec::<String>::new(), "asked once");
    assert_eq!(want(800, FontStyle::Normal), ["w|w700.ttf"]);
    assert_eq!(want(300, FontStyle::Italic), ["w|w400i.ttf"]);
}

#[test]
fn qff1a_script_font_face_loads_for_rendering_and_geometry() {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(sfnt(&box_font_tables("Js Box")));
    let html = format!(
        "<html><body style=\"margin:0\"><div id=s style=\"float:left;font:20px 'Js Box', monospace\">XXXX</div><script>\
        var bytes = Uint8Array.from(atob('{b64}'), function (c) {{ return c.charCodeAt(0); }});\
        var w0 = document.getElementById('s').getBoundingClientRect().width;\
        var ff = new FontFace('Js Box', bytes);\
        console.error('status0 ' + ff.status);\
        document.fonts.add(ff);\
        ff.loaded.then(function (f) {{\
          var w1 = document.getElementById('s').getBoundingClientRect().width;\
          console.error('loaded ' + f.status + ' ' + document.fonts.check('20px \"Js Box\"') + ' ' + (w0 !== 80) + ' ' + w1);\
          document.fonts.delete(ff);\
          console.error('after delete ' + document.getElementById('s').getBoundingClientRect().width);\
        }});\
        var bad = new FontFace('Bad', new Uint8Array([1, 2, 3, 4]));\
        bad.loaded.then(null, function (e) {{ console.error('bad ' + bad.status + ' ' + e.name); }});\
        </script></body></html>"
    );
    let report = js::js_report_for_html_timers(&html, None, "none.html", Some(500));
    assert!(report.contains("status0 loading"), "{report}");
    assert!(report.contains("loaded loaded true true 80"), "{report}");
    assert!(!report.contains("after delete 80"), "{report}");
    assert!(report.contains("bad error NetworkError"), "{report}");
}

#[test]
fn qff1a_document_fonts_follows_the_renderer_load() {
    let html = "<html><head><style>@font-face { font-family: Follow; src: url(http://x.test/f.ttf) }</style></head><body><script>\
        var f = [...document.fonts][0];\
        console.error('A ' + f.status + ' ' + document.fonts.status + ' ' + __scratchy_font_state('follow|http://x.test/f.ttf'));\
        </script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(100));
    assert!(report.contains("A unloaded loaded unloaded"), "{report}");
}

/// The App: a face that arrives restarts the engine and the next frame lays the text out with it;
/// a navigation removes it from the App's font system.
#[test]
fn qff1a_app_relayout_when_a_face_arrives() {
    let Ok(event_loop) = test_event_loop() else { return };
    let live = crate::incr_frame::set_live(true);
    let css = "@font-face { font-family: 'App Box'; src: url(app.ttf) }";
    let mut app = App::new_static(parse_html(&float_page(css, "App Box")), event_loop.create_proxy(), None);
    app.rebuild_layout(800, 600, false);
    let before = float_width(app.cached.as_ref().expect("laid"));
    let jobs = wf::take_wanted();
    assert_eq!(jobs.len(), 1, "layout asked for the face");
    let face = wf::decode_face(sfnt(&box_font_tables("App Box")));
    app.handle_font_fetched(app.nav_generation, &jobs[0].key, jobs[0].page, face);
    assert!(app.restyle_requested || app.layout_dirty, "a relayout is due");
    app.rebuild_layout(800, 600, false);
    let after = float_width(app.cached.as_ref().expect("laid"));
    assert!((after - 80.0).abs() < 0.01, "{after} (was {before})");
    let name = wf::loaded_family("App Box").expect("loaded");
    assert!(app.font_system.db().faces().any(|f| f.families[0].0 == name));
    app.replace_document(parse_html("<p>next</p>"), String::new());
    assert!(!app.font_system.db().faces().any(|f| f.families[0].0 == name), "gone after navigation");
    crate::incr_frame::set_live(live);
}

/// WPT's real Ahem through the same path, when a checkout is on this machine.
#[test]
fn qff1a_real_ahem_when_available() {
    let path = std::path::PathBuf::from("/home/loupax/src/wpt/fonts/Ahem.ttf");
    let Ok(bytes) = std::fs::read(&path) else { return };
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let wire = test_wire("ahem", vec![("/fonts/Ahem.ttf", "", leaked.to_vec())]);
    let doc = parse_html(&float_page("@font-face { font-family: 'WebAhem'; src: url(/fonts/Ahem.ttf) }", "WebAhem"));
    let mut fs = new_font_system();
    let mut sync = wf::FontSync::default();
    let _ = lay_out(&doc, &mut fs);
    let page = Url::parse("http://web-platform.test:8000/x.html").unwrap();
    for job in wf::take_wanted() {
        let face = wf::load_sources(Some(&wire.path), &page, &page, &page, &job.srcs, &HashMap::new());
        assert!(wf::finish_load(&job.key, job.page, face));
    }
    wf::sync(&mut fs, &mut sync);
    let w = float_width(&lay_out(&doc, &mut fs));
    assert!((w - 80.0).abs() < 0.01, "Ahem: {w}");
}
