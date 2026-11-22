//! QCISPLIT2: the `--area` / `--except` shard filter of `--wpt-th`.
use crate::wpt_th_run::selected_area;

#[test]
fn only_and_except_partition_the_areas() {
    let areas = ["dom/nodes", "html/canvas/element", "html/canvas/offscreen", "html/canvasx", "html/dom", "url"];
    let heavy = "html/canvas/*,dom/nodes";
    for a in areas {
        let in_heavy = selected_area(Some(heavy), None, a);
        let in_rest = selected_area(None, Some(heavy), a);
        assert_ne!(in_heavy, in_rest, "{a} must be in exactly one shard");
    }
    assert!(selected_area(Some(heavy), None, "html/canvas/element"));
    assert!(selected_area(Some(heavy), None, "dom/nodes"));
    // `prefix/*` needs the slash: html/canvasx and html/canvas itself are not under html/canvas/.
    assert!(!selected_area(Some(heavy), None, "html/canvasx"));
    assert!(!selected_area(Some("html/canvas/*"), None, "html/canvas"));
    // No filter = everything.
    assert!(areas.iter().all(|a| selected_area(None, None, a)));
}
