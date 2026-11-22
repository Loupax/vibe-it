//! QAPI8: the missing-API report's css `declaration` rows only name declarations scratchy really drops.
#[allow(unused_imports)]
use super::*;
use crate::missing_api;

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        missing_api::set_enabled(false);
        missing_api::reset();
    }
}

/// Names the report calls unsupported declarations for `css` (parsed as one rule body).
fn reported(css: &str) -> Vec<String> {
    missing_api::reset();
    missing_api::set_enabled(true);
    crate::css::parse_stylesheet(&format!("p {{ {css} }}"));
    let json = missing_api::css_json();
    missing_api::set_enabled(false);
    let mut out = Vec::new();
    for part in json.split("{\"kind\":\"declaration\",\"name\":\"").skip(1) {
        out.push(part.split('"').next().unwrap().to_string());
    }
    missing_api::reset();
    out
}

#[test]
fn qapi8_initial_values_are_not_reported() {
    let _g = Guard;
    let r = reported("height:auto;width:auto;left:auto;right:auto;min-height:auto;max-width:none;outline:none;top:auto !important;z-index:auto");
    assert!(r.is_empty(), "{r:?}");
}

#[test]
fn qapi8_properties_handled_elsewhere_are_not_reported() {
    let _g = Guard;
    let r = reported("--x:1;transition:top .5s ease-in-out;-webkit-transition:opacity 1s;animation:spin 1s;-webkit-animation-name:spin;transition-duration:40ms");
    assert!(r.is_empty(), "{r:?}");
}

#[test]
fn qapi8_really_unparsed_declarations_are_still_reported() {
    let _g = Guard;
    let r = reported(
        "scroll-margin-top:4px;touch-action:pan-y;user-select:none;\
         color-scheme:only dark;pointer-events:none;frobnicate:1",
    );
    for n in [
        "scroll-margin-top", "touch-action", "user-select",
        "color-scheme", "pointer-events", "frobnicate",
    ] {
        assert!(r.iter().any(|x| x == n), "{n} missing from {r:?}");
    }
}

#[test]
fn qapi8_a_non_initial_value_of_a_filtered_property_that_fails_is_reported() {
    let _g = Guard;
    let r = reported("height:banana;outline-style:wobbly");
    assert!(r.iter().any(|x| x == "height"), "{r:?}");
}
