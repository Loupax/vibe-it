//! QWPTFLK4: html/webappapis/atob/base64.any.html gave 286 subtests on main and 380 in one CI run.
//! Not timing: the page's `atob() setup` test fetches `../../../fetch/data-urls/resources/base64.json`,
//! which is outside the sparse checkout unless `fetch/data-urls` (or `fetch`) is listed in
//! tools/wpt-dirs.txt. These tests pin that dependency.

fn dirs() -> Vec<String> {
    include_str!("../../tools/wpt-dirs.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// A path is inside the sparse checkout when one of the (cone) dirs is a prefix of it.
fn covered(path: &str) -> bool {
    dirs().iter().any(|d| path == d || path.starts_with(&format!("{d}/")))
}

#[test]
fn atob_json_dependency_is_in_the_sparse_checkout() {
    assert!(covered("fetch/data-urls/resources/base64.json"), "tools/wpt-dirs.txt must cover fetch/data-urls");
}

#[test]
fn atob_page_is_in_the_subset_and_not_flaky() {
    let subset = include_str!("../../wpt-th/subset.txt");
    assert!(subset.lines().any(|l| l.trim() == "html/webappapis/atob/base64.any.html"));
    let flaky = include_str!("../../wpt-th/flaky.txt");
    assert!(!flaky.lines().any(|l| l.starts_with("html/webappapis/atob/")));
}
