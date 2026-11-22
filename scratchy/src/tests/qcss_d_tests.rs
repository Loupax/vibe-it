//! QCSS-D: filter, backdrop-filter, text-shadow, mix-blend-mode, clip-path (parse, computed
//! style, pixels on tiny boxes, style_diff class).
use super::*;
use crate::incr_diff::{style_diff, StyleDiff};

const RED: u32 = 0xFF0000;
const BLUE: u32 = 0x0000FF;

fn page(body: &str) -> (Vec<u32>, u32) {
    let (p, w, _) = paint_fixture(&format!("<html><body style='margin:0'>{body}</body></html>"));
    (p, w)
}

fn chan(c: u32) -> (i32, i32, i32) {
    (((c >> 16) & 255) as i32, ((c >> 8) & 255) as i32, (c & 255) as i32)
}

fn near(c: u32, r: i32, g: i32, b: i32, tol: i32) -> bool {
    let (cr, cg, cb) = chan(c);
    (cr - r).abs() <= tol && (cg - g).abs() <= tol && (cb - b).abs() <= tol
}

fn box_with(style: &str) -> String {
    format!("<div style='position:absolute;left:20px;top:20px;width:20px;height:20px;background:#f00;{style}'></div>")
}

fn style_of(style: &str) -> ComputedStyle {
    let doc = parse_html(&format!("<body><div style='width:123px;{style}'>x</div></body>"));
    let tree = build_layout_tree(&doc, 400.0);
    fn find(b: &LayoutBox) -> Option<ComputedStyle> {
        if b.style.width == Some(BoxLength::Px(123.0)) {
            return Some(b.style);
        }
        match &b.content {
            BoxContent::Blocks(c) => c.iter().find_map(find),
            _ => None,
        }
    }
    find(&tree).expect("the marked div")
}

#[test]
fn computed_style_serialises_each_property() {
    let log = run_in_html(
        "<div id=a style='filter: blur(2px) brightness(150%) hue-rotate(90deg) drop-shadow(1px 2px 3px red); \
         backdrop-filter: grayscale(.5); text-shadow: 1px 2px 3px #00f, 0 0 2px; mix-blend-mode: multiply; \
         clip-path: inset(10px 20% round 4px)'>x</div><div id=b></div>\
         <div id=c style='clip-path: circle(50% at 10px 25%)'></div>\
         <div id=d style='clip-path: polygon(evenodd, 0 0, 100% 0, 50% 100%)'></div>",
        "const a = getComputedStyle(document.getElementById('a')); \
         console.log(a.filter); console.log(a.backdropFilter); console.log(a.textShadow); \
         console.log(a.mixBlendMode); console.log(a.clipPath); \
         const b = getComputedStyle(document.getElementById('b')); \
         console.log([b.filter, b.backdropFilter, b.textShadow, b.mixBlendMode, b.clipPath].join('|')); \
         console.log(getComputedStyle(document.getElementById('c')).clipPath); \
         console.log(getComputedStyle(document.getElementById('d')).clipPath);",
    );
    assert_eq!(log[0], "log:blur(2px) brightness(1.5) hue-rotate(90deg) drop-shadow(rgb(255, 0, 0) 1px 2px 3px)", "{log:?}");
    assert_eq!(log[1], "log:grayscale(0.5)", "{log:?}");
    assert_eq!(log[2], "log:rgb(0, 0, 255) 1px 2px 3px, rgb(0, 0, 0) 0px 0px 2px", "{log:?}");
    assert_eq!(log[3], "log:multiply", "{log:?}");
    assert_eq!(log[4], "log:inset(10px 20% round 4px)", "{log:?}");
    assert_eq!(log[5], "log:none|none|none|normal|none", "{log:?}");
    assert_eq!(log[6], "log:circle(50% at 10px 25%)", "{log:?}");
    assert_eq!(log[7], "log:polygon(evenodd, 0px 0px, 100% 0px, 50% 100%)", "{log:?}");
}

#[test]
fn invalid_values_are_dropped_and_none_wins_the_cascade() {
    let s = style_of("filter: blur(2px); filter: bogus(3)");
    assert!(s.filter.is_some(), "an invalid value keeps the earlier one");
    let s = style_of("filter: url(#x)");
    assert!(s.filter.is_none(), "url() filters are out of scope");
    let s = style_of("filter: blur(-1px)");
    assert!(s.filter.is_none());
    let s = style_of("filter: blur(2px); filter: none");
    assert!(s.filter.is_none());
    let s = style_of("clip-path: inset(50%)");
    assert!(s.clip_path.is_some());
    let s = style_of("mix-blend-mode: wat");
    assert_eq!(s.mix_blend_mode, BlendMode::Normal);
}

#[test]
fn text_shadow_inherits_and_filter_does_not() {
    let doc = parse_html("<body><div style='text-shadow: 1px 1px red; filter: blur(1px)'><p id=p>x</p></div></body>");
    let tree = build_layout_tree(&doc, 400.0);
    fn collect(b: &LayoutBox, depth: usize, out: &mut Vec<(usize, ComputedStyle)>) {
        out.push((depth, b.style));
        if let BoxContent::Blocks(c) = &b.content {
            for x in c {
                collect(x, depth + 1, out);
            }
        }
    }
    let mut v = Vec::new();
    collect(&tree, 0, &mut v);
    let deepest = v.iter().max_by_key(|(d, _)| *d).unwrap().1;
    assert!(deepest.text_shadow.is_some(), "text-shadow is inherited");
    assert!(deepest.filter.is_none(), "filter is not");
}

#[test]
fn changes_are_paint_class_not_relayout() {
    let a = style_of("color: red");
    for css in ["filter: blur(3px)", "backdrop-filter: blur(3px)", "text-shadow: 1px 1px red", "mix-blend-mode: screen", "clip-path: circle(40%)"] {
        let b = style_of(&format!("color: red; {css}"));
        assert_eq!(style_diff(&a, &b), StyleDiff::Paint, "{css}");
    }
}

#[test]
fn filter_color_functions_change_pixels() {
    let (p, w) = page(&box_with("filter: invert(1)"));
    assert!(near(px_at(&p, w, 30, 30), 0, 255, 255, 2), "{:06x}", px_at(&p, w, 30, 30));
    assert_eq!(px_at(&p, w, 10, 10), 0, "outside stays untouched");
    let (p, w) = page(&box_with("filter: grayscale(1)"));
    assert!(near(px_at(&p, w, 30, 30), 54, 54, 54, 2), "{:06x}", px_at(&p, w, 30, 30));
    let (p, w) = page(&box_with("filter: brightness(0.5)"));
    assert!(near(px_at(&p, w, 30, 30), 128, 0, 0, 2));
    let (p, w) = page(&box_with("filter: opacity(0.5)"));
    assert!(near(px_at(&p, w, 30, 30), 128, 0, 0, 2));
    let (p, w) = page(&box_with("filter: sepia(1)"));
    let (r, g, b) = chan(px_at(&p, w, 30, 30));
    assert!(r > g && g > b && b > 0, "sepia is brown: {r} {g} {b}");
    let (p, w) = page(&box_with("filter: hue-rotate(120deg)"));
    let (r, g, _) = chan(px_at(&p, w, 30, 30));
    assert!(g > r, "red rotated 120deg leans green: {r} {g}");
    let (p, w) = page(&box_with("filter: saturate(0)"));
    let (r, g, b) = chan(px_at(&p, w, 30, 30));
    assert!((r - g).abs() <= 2 && (g - b).abs() <= 2);
    let (p, w) = page(&box_with("filter: contrast(0)"));
    assert!(near(px_at(&p, w, 30, 30), 128, 128, 128, 2));
}

#[test]
fn filter_blur_spreads_past_the_box_and_softens_the_edge() {
    let (p, w) = page(&box_with("filter: blur(3px)"));
    let outside = px_at(&p, w, 17, 30);
    assert!(chan(outside).0 > 0, "halo outside the left edge: {outside:06x}");
    let edge_in = chan(px_at(&p, w, 21, 30)).0;
    assert!(edge_in < 250, "inner edge is softened: {edge_in}");
    assert!(chan(px_at(&p, w, 30, 30)).0 > 200, "centre keeps most of the colour");
    assert_eq!(px_at(&p, w, 3, 3), 0, "far away untouched");
    // The cap: a huge radius stays bounded and still paints something faint.
    let (p, w) = page(&box_with("filter: blur(5000px)"));
    assert!(chan(px_at(&p, w, 30, 30)).0 < 120, "heavily blurred box is faint");
}

#[test]
fn filter_drop_shadow_paints_offset_copy() {
    let (p, w) = page(&box_with("background:#00f;filter: drop-shadow(15px 15px 0 #0f0)"));
    assert_eq!(px_at(&p, w, 30, 30), BLUE);
    assert!(near(px_at(&p, w, 50, 50), 0, 255, 0, 2), "shadow past the box corner: {:06x}", px_at(&p, w, 50, 50));
    assert_eq!(px_at(&p, w, 50, 10), 0);
}

#[test]
fn clip_path_basic_shapes() {
    let (p, w) = page(&box_with("clip-path: inset(50%)"));
    assert_eq!(px_at(&p, w, 30, 30), 0, "inset(50%) hides the box (visually-hidden idiom)");
    let (p, w) = page(&box_with("clip-path: inset(5px)"));
    assert_eq!(px_at(&p, w, 22, 22), 0);
    assert_eq!(px_at(&p, w, 30, 30), RED);
    let (p, w) = page(&box_with("clip-path: circle(50%)"));
    assert_eq!(px_at(&p, w, 30, 30), RED);
    assert_eq!(px_at(&p, w, 21, 21), 0, "corner is outside the circle");
    let (p, w) = page(&box_with("clip-path: ellipse(50% 50%)"));
    assert_eq!(px_at(&p, w, 30, 30), RED);
    assert_eq!(px_at(&p, w, 21, 21), 0);
    let (p, w) = page(&box_with("clip-path: polygon(0 0, 100% 0, 0 100%)"));
    assert_eq!(px_at(&p, w, 23, 23), RED, "inside the triangle");
    assert_eq!(px_at(&p, w, 37, 37), 0, "outside the triangle");
    let (p, w) = page(&box_with("clip-path: none"));
    assert_eq!(px_at(&p, w, 37, 37), RED);
}

#[test]
fn clip_path_also_clips_descendants() {
    let html = box_with("clip-path: inset(0 50% 0 0)").replace("></div>", "><div style='width:20px;height:20px;background:#00f'></div></div>");
    let (p, w) = page(&html);
    assert_eq!(px_at(&p, w, 25, 30), BLUE);
    assert_eq!(px_at(&p, w, 35, 30), 0);
}

#[test]
fn mix_blend_mode_separable_modes() {
    let under = "<div style='position:absolute;left:20px;top:20px;width:20px;height:20px;background:#00f'></div>";
    let over = |m: &str| format!("<div style='position:absolute;left:20px;top:20px;width:20px;height:20px;background:#f00;mix-blend-mode:{m}'></div>");
    let (p, w) = page(&format!("{under}{}", over("multiply")));
    assert_eq!(px_at(&p, w, 30, 30), 0, "red x blue = black");
    let (p, w) = page(&format!("{under}{}", over("screen")));
    assert_eq!(px_at(&p, w, 30, 30), 0xFF00FF);
    let (p, w) = page(&format!("{under}{}", over("difference")));
    assert_eq!(px_at(&p, w, 30, 30), 0xFF00FF);
    let (p, w) = page(&format!("{under}{}", over("normal")));
    assert_eq!(px_at(&p, w, 30, 30), RED);
    let (p, w) = page(&format!("{under}{}", over("darken")));
    assert_eq!(px_at(&p, w, 30, 30), 0);
}

#[test]
fn backdrop_filter_filters_what_is_behind() {
    let stripes = "<div style='position:absolute;left:0;top:0;width:100px;height:50px;background:#f00'></div>\
                   <div style='position:absolute;left:100px;top:0;width:100px;height:50px;background:#00f'></div>";
    let bar = "<div style='position:absolute;left:10px;top:10px;width:150px;height:20px;backdrop-filter: invert(1)'></div>";
    let (p, w) = page(&format!("{stripes}{bar}"));
    assert!(near(px_at(&p, w, 20, 20), 0, 255, 255, 2), "red behind the bar is inverted: {:06x}", px_at(&p, w, 20, 20));
    assert!(near(px_at(&p, w, 150, 20), 255, 255, 0, 2));
    assert_eq!(px_at(&p, w, 20, 40), RED, "outside the bar untouched");
    let bar = "<div style='position:absolute;left:10px;top:10px;width:150px;height:20px;backdrop-filter: blur(4px)'></div>";
    let (p, w) = page(&format!("{stripes}{bar}"));
    let mid = px_at(&p, w, 100, 20);
    let (r, _, b) = chan(mid);
    assert!(r > 30 && b > 30 && r < 230 && b < 230, "the stripe boundary is blurred under the bar: {mid:06x}");
    assert_eq!(px_at(&p, w, 99, 40), RED, "below the bar the edge is sharp");
    assert_eq!(px_at(&p, w, 100, 40), BLUE);
}

fn count_red(p: &[u32]) -> usize {
    p.iter().filter(|c| near(**c, 255, 0, 0, 40)).count()
}

#[test]
fn text_shadow_paints_under_transparent_text_and_blurs() {
    let text = |css: &str| format!("<div style='position:absolute;left:10px;top:10px;font-size:40px;color:transparent;{css}'>MMM</div>");
    let (none, _) = page(&text(""));
    assert_eq!(count_red(&none), 0);
    let (hard, _) = page(&text("text-shadow: 3px 3px 0 #f00"));
    assert!(count_red(&hard) > 50, "a hard shadow draws: {}", count_red(&hard));
    let (soft, _) = page(&text("text-shadow: 3px 3px 6px #f00"));
    let touched = soft.iter().filter(|c| **c != 0).count();
    let touched_hard = hard.iter().filter(|c| **c != 0).count();
    assert!(touched > touched_hard, "blur spreads the shadow: {touched} vs {touched_hard}");
    let (two, _) = page(&text("text-shadow: 3px 3px 0 #f00, -3px -3px 0 #00f"));
    assert!(two.iter().any(|c| near(*c, 0, 0, 255, 40)), "second shadow paints too");
    let (inh, _) = page(&format!("<div style='text-shadow: 3px 3px 0 #f00'>{}</div>", text("")));
    assert!(count_red(&inh) > 50, "inherited by descendants");
}

#[test]
fn effects_respect_ancestor_overflow_clip() {
    let (p, w) = page(
        "<div style='position:absolute;left:0;top:0;width:30px;height:100px;overflow:hidden'>\
         <div style='position:absolute;left:20px;top:20px;width:40px;height:20px;background:#f00;filter:invert(1)'></div></div>",
    );
    assert!(near(px_at(&p, w, 25, 30), 0, 255, 255, 2));
    assert_eq!(px_at(&p, w, 40, 30), 0, "clipped by the ancestor");
}
