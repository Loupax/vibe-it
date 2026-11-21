use super::*;
use crate::config::STATUS_BAR_H;

// QKEYS1: the status bar reserves its height; the page's viewport is the window minus the bar.

const WIN: (u32, u32) = (800, 600);

#[test]
fn qkeys1_page_viewport_is_the_window_minus_the_bar() {
    assert_eq!(page_viewport(WIN), (800, 600 - STATUS_BAR_H));
    assert_eq!(page_viewport((800, 10)), (800, 1), "a window shorter than the bar still has a 1 px page");
    assert_eq!(page_viewport((800, 0)), (800, 1));
}

fn walk<'a>(l: &'a Laid, out: &mut Vec<&'a Laid>) {
    out.push(l);
    if let LaidContent::Blocks(v) = &l.content {
        for c in v {
            walk(c, out);
        }
    }
    for p in &l.positioned {
        walk(p, out);
    }
}

#[test]
fn qkeys1_layout_viewport_height_and_fixed_bottom_box_sit_above_the_bar() {
    let page_h = page_viewport(WIN).1 as f32;
    let html = "<html><body style='margin:0'><div id=f style='position:fixed;bottom:0;left:0;height:10px;width:50px'></div>\
                <div id=v style='height:100vh'></div></body></html>";
    let laid = measure_document_fixture(html, WIN.0 as f32, page_h);
    let mut all = Vec::new();
    walk(&laid, &mut all);
    let fixed = all.iter().find(|l| l.w == 50.0 && l.h == 10.0).expect("the fixed box");
    assert_eq!(fixed.y + fixed.h, page_h, "its bottom is the page viewport's bottom");
    assert!(fixed.y + fixed.h <= WIN.1 as f32 - STATUS_BAR_H as f32, "so it is above the bar");
}

#[test]
fn qkeys1_inner_height_matches_the_page_viewport() {
    let (w, h) = page_viewport(WIN);
    crate::js::set_window_viewport(w, h);
    let doc = parse_html("<html><body></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let code = "var n = 0; addEventListener('resize', function () { n++; });\
                globalThis.snap = function () { console.log('s', innerWidth, innerHeight, visualViewport.height, document.documentElement.clientHeight, n); };\
                snap();";
    let eng = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    let want = format!("log:s {w} {h} {h} {h} 0");
    assert!(out.borrow().contains(&want), "{:?}", out.borrow());
    // A window resize publishes the new page viewport: one resize event.
    let (w2, h2) = page_viewport((800, 500));
    eng.set_viewport(w2, h2);
    eng.eval("t", "snap();");
    let want = format!("log:s {w2} {h2} {h2} {h2} 1");
    assert!(out.borrow().contains(&want), "{:?}", out.borrow());
    crate::js::set_window_viewport(0, 0);
}

#[test]
fn qkeys1_window_scrollbar_ends_above_the_bar() {
    let html = "<html><body style='margin:0'><div style='height:5000px'></div></body></html>";
    let (_, page_h) = page_viewport(WIN);
    let laid = Some(Rc::new(measure_document_fixture(html, WIN.0 as f32, page_h as f32)));
    let (rect, _content, client) = App::viewport_bar(&laid, page_viewport(WIN)).expect("a long page has a bar");
    assert_eq!(rect.1 + rect.3, page_h as f32, "the track ends where the page viewport ends");
    assert_eq!(client, page_h as f32);
}

#[test]
fn qkeys2_mode_name_is_the_key_mode() {
    assert_eq!(mode_name(KeyMode::Normal), "NORMAL");
    assert_eq!(mode_name(KeyMode::Input), "INPUT");
}

fn bar_pixels(bar: &crate::paint::StatusBar) -> Vec<u32> {
    let (w, h) = WIN;
    let mut font_system = new_font_system();
    let mut swash = SwashCache::new();
    let mut pixels = vec![0x00FF_FFFFu32; (w * h) as usize];
    crate::paint::paint_status_bar(bar, &mut font_system, &mut swash, &mut pixels, w, h);
    pixels
}

#[test]
fn qkeys1_bar_paints_only_the_bottom_strip_and_its_text_follows_the_mode() {
    let (w, h) = WIN;
    let bar = |mode, command| crate::paint::StatusBar { mode, url: "https://example.test/", message: "", command, prompt: ':' };
    let input = bar_pixels(&bar("INPUT", None));
    let normal = bar_pixels(&bar("NORMAL", None));
    let strip = ((h - STATUS_BAR_H) * w) as usize;
    assert!(input[..strip].iter().all(|&p| p == 0x00FF_FFFF), "nothing above the strip is touched");
    assert_ne!(px_at(&input, w, 799, h - 1), 0x00FF_FFFF, "the strip is filled");
    assert_ne!(input[strip..], normal[strip..], "the mode name is drawn: INPUT and NORMAL differ");
    // The message (D8) shows at the right.
    let with_msg = bar_pixels(&crate::paint::StatusBar { mode: "NORMAL", url: "https://example.test/", message: "Pattern not found: x", command: None, prompt: ':' });
    assert_ne!(normal[strip..], with_msg[strip..]);
    // The `:` line takes the strip.
    let mut cmd = TextEditBuffer::default();
    cmd.insert('q');
    let command = bar_pixels(&bar("NORMAL", Some(&cmd)));
    assert_ne!(normal[strip..], command[strip..]);
    assert!(command[..strip].iter().all(|&p| p == 0x00FF_FFFF));
}
