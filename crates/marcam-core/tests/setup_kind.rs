//! A setup's kind (rotary, indexed 3+1, vise): which operation types fit it, and the project
//! check's messages when an operation or the stock does not.

use marcam::ops::{find, fits_setup};
use marcam::project::Project;

#[test]
fn which_operations_fit_each_kind() {
    let fits = |op: &str, kind: &str| fits_setup(&find(op).unwrap(), kind);
    // rotary: what turns the part, and engraving wrapped round it
    assert!(
        fits("rotary_finish", "rotary")
            && fits("helical_knurl", "rotary")
            && fits("engrave", "rotary")
    );
    assert!(!fits("pocket", "rotary") && !fits("raster3d", "rotary"));
    // indexed 3+1: the 3-axis types at fixed angles
    assert!(fits("pocket", "indexed") && fits("raster3d", "indexed") && fits("drill", "indexed"));
    assert!(!fits("rotary_finish", "indexed") && !fits("engrave", "indexed"));
    // vise: everything without the rotary axis
    assert!(fits("pocket", "vise") && fits("engrave", "vise") && fits("vcarve", "vise"));
    assert!(!fits("rotary_rough", "vise") && !fits("helical_knurl", "vise"));
}

#[test]
fn the_project_check_names_what_does_not_fit() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/knob/knob_indexed.project.json"
    ))
    .unwrap();
    let (mut p, _) = Project::from_json_any(&text).unwrap();
    // no kind: the knob mixes rotary and indexed operations, as before
    assert!(
        p.check_operations().is_empty(),
        "{:?}",
        p.check_operations()
    );
    p.setups[0].kind = Some("indexed".into());
    let problems = p.check_operations();
    assert!(
        problems
            .iter()
            .any(|e| e.contains("rotary_turning does not fit a indexed setup")),
        "{problems:?}"
    );
    p.setups[0].kind = Some("vise".into());
    assert!(p
        .check_operations()
        .iter()
        .any(|e| e.contains("a vise setup needs box stock")));
    p.setups[0].kind = Some("lathe".into());
    assert!(p
        .check_operations()
        .iter()
        .any(|e| e.contains("use \"rotary\", \"indexed\" or \"vise\"")));
    // the kind survives a round trip through the document
    p.setups[0].kind = Some("rotary".into());
    let again = serde_json::to_string(&p).unwrap();
    assert!(again.contains("\"kind\":\"rotary\""));
}
