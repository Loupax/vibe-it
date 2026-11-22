//! QAPI4: `document.fonts` (FontFaceSet), `FontFace`, `FontFaceSetLoadEvent` (src/fonts_prelude.js).
#[allow(unused_imports)]
use super::*;

const PAGE: &str = "<html><head><style>\
    @font-face { font-family: 'Fancy Sans'; src: url(data:font/woff2;base64,AAAA) format('woff2'), local(Arial); font-weight: 700; }\
    @font-face { font-family: Other; src: local(Arial); }\
    p { color: red }</style></head><body></body></html>";

#[test]
fn qapi4_set_mirrors_font_face_rules() {
    let (log, _) = mutate(
        PAGE,
        "var f = document.fonts; var a = []; f.forEach(function (x) { a.push(x.family + ':' + x.status + ':' + x.weight); });\
         console.log(typeof f, f.size, f.status, a.join('|'), [...f].length, f.has(f.values().next().value), typeof FontFace, typeof FontFaceSet);\
         console.log(f.check('16px Fancy Sans'), f.check('bold 16px \"Fancy Sans\", serif'), f.check('12px Georgia'));\
         try { f.check('nonsense'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        ["log:object 2 loaded Fancy Sans:unloaded:700|Other:unloaded:normal 2 true function function", "log:false false true", "log:SyntaxError"],
        "{log:?}"
    );
}

#[test]
fn qapi4_ready_is_a_resolved_promise_and_faces_construct() {
    let (log, _) = mutate(
        "<html><body></body></html>",
        "var f = document.fonts; console.log(f.ready instanceof Promise, f.size, f.status);\
         var ff = new FontFace('Mine', 'url(data:font/woff2;base64,AAAA)', { weight: '300' });\
         console.log(ff.family, ff.status, ff.weight, ff.style, ff.display, ff.unicodeRange);\
         console.log(f.add(ff) === f, f.size, f.has(ff), f.check('1em Mine'), f.delete(ff), f.size, f.delete(ff));\
         var bad = new FontFace('Bad', 'garbage'); console.log(bad.status);\
         try { f.add(bad); } catch (e) { console.log(e.name); }\
         try { FontFace('x', 'y'); } catch (e) { console.log(e.name); }\
         var s = new FontFaceSet([ff]); console.log(s.size, s.status, s instanceof EventTarget);",
    );
    assert_eq!(
        log,
        [
            "log:true 0 loaded",
            "log:Mine unloaded 300 normal auto U+0-10FFFF",
            "log:true 1 true false true 0 false",
            "log:error",
            "log:InvalidModificationError",
            "log:TypeError",
            "log:1 loaded true",
        ],
        "{log:?}"
    );
}

#[test]
fn qapi4_load_events_and_ready() {
    let html = format!(
        "{}<script>\
        var f = document.fonts, ev = [];\
        f.onloading = function (e) {{ ev.push('loading:' + f.status); }};\
        f.addEventListener('loadingdone', function (e) {{ ev.push('done:' + e.fontfaces.length + ':' + f.status); }});\
        var before = f.ready;\
        f.load('16px \"Fancy Sans\"').then(function (list) {{\
          ev.push('load:' + list.length + ':' + list[0].status + ':' + f.check('16px \"Fancy Sans\"'));\
        }});\
        console.error('start ' + f.status + ' ' + f.load('1em Nothing').constructor.name);\
        f.ready.then(function (s) {{ console.error('R ' + (s === f) + ' ' + ev.join(',') + ' ' + f.status + ' ' + (before !== f.ready)); }});\
        </script>",
        PAGE.replace("</body></html>", "")
    );
    let report = js::js_report_for_html_timers(&html, None, "none.html", Some(500));
    assert!(report.contains("start loading Promise"), "{report}");
    assert!(report.contains("R true loading:loading"), "{report}");
    assert!(report.contains("done:1:loaded"), "{report}");
    assert!(report.contains("load:1:loaded:true"), "{report}");
}

#[test]
fn qapi4_failed_load_rejects_and_reports_error() {
    // No network here: a relative url cannot be fetched, so the face ends in 'error'.
    let html = "<html><head><style>@font-face { font-family: Gone; src: url(/nope.woff2); }</style></head><body><script>\
        var f = document.fonts; f.addEventListener('loadingerror', function (e) { console.error('E ' + e.fontfaces.length); });\
        f.load('1em Gone').then(function () { console.error('unexpected'); }, function (e) { console.error('rej ' + e.name + ' ' + [...f][0].status); });\
        </script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(3000));
    assert!(report.contains("rej NetworkError error"), "{report}");
    assert!(!report.contains("unexpected"), "{report}");
}
