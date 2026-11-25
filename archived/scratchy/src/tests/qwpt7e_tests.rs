// QWPT7e: hit-test leftovers: inline-level vertical margins, SVG path/stroke geometry, `use`, nested
// svg viewBoxes, the SVG pointer-events values (also from stylesheets), svg in scrolled/fixed boxes,
// elementsFromPoint through an svg.
#[allow(unused_imports)]
use super::*;

fn engine(html: &str) -> (JsEngine, Handle) {
    let doc = parse_html(html);
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let eng = crate::js::start_scripts_clocked(&doc, &[src("")], Rc::new(|_, _| {}), Some(clock)).unwrap();
    (eng, doc)
}

fn stack_at(eng: &JsEngine, x: f64, y: f64) -> String {
    eng.eval_value::<String>("t", &format!("document.elementsFromPoint({x}, {y}).map(function (e) {{ return e.id || e.localName; }}).join(',')")).unwrap()
}

fn top_at(eng: &JsEngine, x: f64, y: f64) -> String {
    eng.eval_value::<String>("t", &format!("(function () {{ var e = document.elementFromPoint({x}, {y}); return e ? (e.id || e.localName) : 'null'; }})()")).unwrap()
}

fn rect_of(eng: &JsEngine, id: &str) -> Vec<f64> {
    eng.eval_value::<Vec<f64>>("t", &format!("var b = document.getElementById('{id}').getBoundingClientRect(); [b.x, b.y, b.width, b.height]")).unwrap()
}

#[test]
fn inline_level_boxes_keep_their_vertical_margins() {
    for body in [
        "<svg id=x width=30 height=30 style='margin:100px'></svg>",
        "<span id=x style='display:inline-block;width:30px;height:30px;margin:100px'></span>",
        "<div style='height:500px'><svg id=x width=30 height=30 style='margin:100px'></svg></div>",
    ] {
        let (eng, _d) = engine(&format!("<html><body style='margin:0'>{body}</body></html>"));
        assert_eq!(rect_of(&eng, "x"), vec![100.0, 100.0, 30.0, 30.0], "{body}");
    }
}

const WPT_SVG: &str = "<html><head><style>html,body{margin:0;padding:0}#svg{margin:100px}rect{fill:rgba(180,0,0,0.2)}\
  #topRightSvg,#mg1,#mg2{pointer-events:bounding-box}#none{pointer-events:none}</style></head><body><div id=sandbox>\
  <svg id=svg width=300 height=300>\
  <rect id=r1 x=5 y=5 width=90 height=90 /><rect id=none x=10 y=10 width=80 height=80 /><rect id=r3 x=15 y=15 width=70 height=70 />\
  <svg id=topRightSvg x=205 y=5 width=90 height=90><rect id=tr1 x=5 y=5 width=80 height=80 /><rect id=tr2 x=10 y=10 width=70 height=70 /></svg>\
  <g id=mg1><g id=mg2><rect id=m1 x=105 y=105 width=90 height=90 /><rect id=m2 x=110 y=110 width=80 height=80 /></g></g>\
  <g id=bg1 transform='translate(300, 300)'><g id=bg2 transform='translate(-100, -100)'>\
  <rect id=b1 x=5 y=5 width=90 height=90 /><rect id=b2 x=110 y=110 width=80 height=80 transform='translate(-100, -100)' /></g></g>\
  </svg></div></body></html>";

#[test]
fn elements_from_point_lists_every_svg_shape_under_the_point() {
    let (eng, _d) = engine(WPT_SVG);
    // The rule with pointer-events: none (from the stylesheet) is skipped.
    assert_eq!(stack_at(&eng, 125.0, 125.0), "r3,r1,svg,sandbox,body,html");
    // A nested svg: its x/y offset, bounding-box pointer-events.
    assert_eq!(stack_at(&eng, 325.0, 125.0), "tr2,tr1,topRightSvg,svg,sandbox,body,html");
    assert_eq!(stack_at(&eng, 225.0, 225.0), "m2,m1,mg2,mg1,svg,sandbox,body,html");
    assert_eq!(stack_at(&eng, 325.0, 325.0), "b2,b1,bg2,bg1,svg,sandbox,body,html");
    assert_eq!(top_at(&eng, 125.0, 125.0), "r3");
    // Outside every shape the svg is the top.
    assert_eq!(stack_at(&eng, 398.0, 398.0), "svg,sandbox,body,html");
}

#[test]
fn nested_svg_viewbox_and_clip() {
    let (eng, _d) = engine(
        "<html><body style='margin:0'><svg id=s width=200 height=200>\
         <svg id=n x=50 y=50 width=100 height=100 viewBox='0 0 50 50'><rect id=r x=10 y=10 width=20 height=20 /><rect id=out x=45 y=45 width=100 height=100 /></svg></svg></body></html>",
    );
    // viewBox 50 -> 100: the rect (10..30) is at 50 + 20..60 = 70..110.
    assert_eq!(top_at(&eng, 90.0, 90.0), "r");
    assert_eq!(top_at(&eng, 65.0, 65.0), "s", "before the rect's corner");
    // The nested svg clips what leaves its viewport (150).
    assert_eq!(top_at(&eng, 148.0, 148.0), "out");
    assert_eq!(top_at(&eng, 170.0, 170.0), "s");
    assert_eq!(rect_of(&eng, "r"), vec![70.0, 70.0, 40.0, 40.0]);
}

#[test]
fn paths_are_hit_by_their_real_fill_and_stroke() {
    let (eng, _d) = engine(
        "<html><body style='margin:0'><svg id=s width=200 height=200>\
         <path id=tri d='M10 10 L90 10 L10 90 Z' />\
         <path id=ring fill-rule=evenodd d='M110 10 h80 v80 h-80 z M130 30 h40 v40 h-40 z' />\
         <path id=open fill=none stroke=black stroke-width=6 d='M10 150 L90 150' />\
         <path id=arc d='M110 150 a40 40 0 0 1 80 0 z' />\
         </svg></body></html>",
    );
    assert_eq!(top_at(&eng, 20.0, 20.0), "tri");
    assert_eq!(top_at(&eng, 80.0, 80.0), "s", "outside the triangle, inside its box");
    assert_eq!(top_at(&eng, 120.0, 20.0), "ring");
    assert_eq!(top_at(&eng, 150.0, 50.0), "s", "the even-odd hole");
    assert_eq!(top_at(&eng, 50.0, 152.0), "open", "on the stroke");
    assert_eq!(top_at(&eng, 50.0, 160.0), "s", "off the stroke of a fill:none path");
    assert_eq!(top_at(&eng, 150.0, 140.0), "arc", "inside the half disc");
    assert_eq!(top_at(&eng, 150.0, 175.0), "s", "below the chord");
}

#[test]
fn svg_pointer_events_values() {
    let (eng, _d) = engine(
        "<html><head></head><body style='margin:0'><svg id=s width=300 height=100>\
         <rect id=vp x=0 y=0 width=40 height=40 style='pointer-events:visiblePainted' fill=none />\
         <rect id=vf x=50 y=0 width=40 height=40 fill=none style='pointer-events:visibleFill' />\
         <rect id=vs x=100 y=0 width=40 height=40 stroke=black stroke-width=4 style='pointer-events:visibleStroke' />\
         <rect id=al x=150 y=0 width=40 height=40 fill=none style='pointer-events:all' />\
         <rect id=bb x=200 y=0 width=40 height=40 fill=none style='pointer-events:bounding-box' />\
         <rect id=st x=250 y=0 width=40 height=40 style='fill:none;stroke:black;stroke-width:10;pointer-events:stroke' />\
         </svg></body></html>",
    );
    assert_eq!(top_at(&eng, 20.0, 20.0), "s", "visiblePainted + fill:none, no stroke");
    assert_eq!(top_at(&eng, 70.0, 20.0), "vf", "visibleFill ignores fill:none");
    assert_eq!(top_at(&eng, 120.0, 20.0), "s", "visibleStroke: the inside is not stroke");
    assert_eq!(top_at(&eng, 100.0, 20.0), "vs", "visibleStroke: the stroke");
    assert_eq!(top_at(&eng, 170.0, 20.0), "al");
    assert_eq!(top_at(&eng, 220.0, 20.0), "bb");
    assert_eq!(top_at(&eng, 270.0, 20.0), "s", "stroke: the inside is not stroke");
    assert_eq!(top_at(&eng, 252.0, 20.0), "st");
    let cs = eng.eval_value::<String>("t", "getComputedStyle(document.getElementById('bb')).pointerEvents").unwrap();
    assert_eq!(cs, "bounding-box");
}

#[test]
fn use_is_hit_through_its_target_and_has_a_rect() {
    let (eng, _d) = engine(
        "<html><body style='margin:0'><svg id=s width=200 height=100>\
         <defs><rect id=t x=0 y=0 width=20 height=20 /></defs>\
         <use id=u href='#t' x=100 y=40 />\
         <rect id=t2 x=10 y=10 width=10 height=10 /></svg></body></html>",
    );
    assert_eq!(top_at(&eng, 110.0, 50.0), "u");
    assert_eq!(top_at(&eng, 150.0, 50.0), "s");
    assert_eq!(top_at(&eng, 15.0, 15.0), "t2", "the defs copy itself is never a target");
    assert_eq!(rect_of(&eng, "u"), vec![100.0, 40.0, 20.0, 20.0]);
}

#[test]
fn svg_in_a_fixed_or_scrolled_box_is_hit_at_the_right_point() {
    let (eng, _d) = engine(
        "<html><body style='margin:0;height:3000px'>\
         <div id=fx style='position:fixed;left:50px;top:50px'><svg width=100 height=100><rect id=fr x=10 y=10 width=30 height=30 /></svg></div>\
         <div id=sc style='position:absolute;left:300px;top:0;width:100px;height:100px;overflow:auto'>\
           <div style='height:400px'><svg width=100 height=100 style='margin-top:150px'><rect id=sr x=10 y=10 width=30 height=30 /></svg></div></div>\
         </body></html>",
    );
    assert_eq!(top_at(&eng, 70.0, 70.0), "fr");
    eng.eval_value::<f64>("t", "window.scrollTo(0, 500); 1").unwrap();
    // A fixed svg stays under the same viewport point after the window scrolls.
    assert_eq!(top_at(&eng, 70.0, 70.0), "fr");
    assert_eq!(top_at(&eng, 140.0, 140.0), "svg", "outside the rect, inside the svg");
    // An element scrolled by script: the svg inside it moves with it (its rect is at y = 160..190 unscrolled).
    eng.eval_value::<f64>("t", "window.scrollTo(0, 0); 1").unwrap();
    assert_eq!(top_at(&eng, 320.0, 70.0), "div", "the svg starts below the scrollport");
    eng.eval_value::<f64>("t", "document.getElementById('sc').scrollTop = 100; 1").unwrap();
    assert_eq!(top_at(&eng, 320.0, 70.0), "sr");
}
