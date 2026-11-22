#[allow(unused_imports)]
use super::*;

// ---------- QAPI2: navigator.webdriver / deviceMemory / connection / plugins / mimeTypes / userAgentData ----------

fn nav(code: &str) -> Vec<String> {
    run_collecting(&[src(code)])
}

#[test]
fn qapi2_constants() {
    let log = nav(
        "console.log(navigator.webdriver, navigator.deviceMemory, navigator.pdfViewerEnabled, navigator.hardwareConcurrency, 'webdriver' in navigator, 'userAgentData' in navigator);",
    );
    assert_eq!(log, ["log:false 8 false 1 true true"], "{log:?}");
}

#[test]
fn qapi2_connection_is_a_static_network_information() {
    let log = nav(
        "var c = navigator.connection; var n = 0; c.addEventListener('change', function () { n++; });\
         c.dispatchEvent(new Event('change'));\
         console.log(c.effectiveType, c.type, c.downlink, c.rtt, c.saveData, c.onchange, n, Object.prototype.toString.call(c), c instanceof EventTarget, c === navigator.connection);",
    );
    assert_eq!(log, ["log:4g unknown 10 50 false null 1 [object NetworkInformation] true true"], "{log:?}");
}

#[test]
fn qapi2_plugins_and_mime_types_are_empty() {
    let log = nav(
        "var p = navigator.plugins, m = navigator.mimeTypes;\
         console.log(p.length, m.length, p.item(0), p.namedItem('PDF Viewer'), m.item(0), typeof p.refresh, [...p].length, [...m].length,\
           p instanceof PluginArray, m instanceof MimeTypeArray, Object.prototype.toString.call(p));",
    );
    assert_eq!(log, ["log:0 0 null null null function 0 0 true true [object PluginArray]"], "{log:?}");
}

#[test]
fn qapi2_user_agent_data_follows_the_ua_string() {
    let log = nav(
        "var d = navigator.userAgentData; var m = /scratchy\\/(\\d+)/.exec(navigator.userAgent);\
         var b = d.brands.filter(function (x) { return x.brand === 'scratchy'; })[0];\
         console.log(d.mobile, d.platform, !!m, !!b && b.version === m[1], d.brands.length, Object.keys(d.toJSON()).join(), d === navigator.userAgentData);\
         d.getHighEntropyValues(['architecture', 'model', 'platformVersion', 'uaFullVersion']).then(function (v) {\
           console.log('he', Object.keys(v).join(), v.platform, v.mobile, v.brands.length);\
         });",
    );
    assert_eq!(log[0], "log:false Linux true true 2 brands,mobile,platform true", "{log:?}");
    assert_eq!(log[1], "log:he brands,mobile,platform Linux false 2", "{log:?}");
}

#[test]
fn qapi2_user_agent_data_parses_an_override() {
    let log = nav(
        "navigator.userAgent = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';\
         console.log(navigator.userAgentData.brands.map(function (b) { return b.brand + ' ' + b.version; }).join('; '));",
    );
    assert_eq!(log, ["log:Not;A=Brand 99; Chrome 120"], "{log:?}");
}

#[test]
fn qapi2_high_entropy_needs_an_argument_and_brands_are_fresh_copies() {
    let log = nav(
        "navigator.userAgentData.getHighEntropyValues().catch(function (e) { console.log(e.name); });\
         var b = navigator.userAgentData.brands; b[0].brand = 'x'; console.log(navigator.userAgentData.brands[0].brand);",
    );
    assert_eq!(log[0], "log:Not;A=Brand", "{log:?}");
    assert_eq!(log[1], "log:TypeError", "{log:?}");
}
