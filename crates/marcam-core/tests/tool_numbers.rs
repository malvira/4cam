//! Tools without a T number yet (`unindexed`): planned as the next free number, with a
//! warning in the G-code; one tool under two numbers is warned about too.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gcode(project: &serde_json::Value) -> Vec<String> {
    let dir = root().join("examples/plate");
    let libs = marcam::profiles::builtin_libraries();
    let jobs = marcam::project::jobs_from_any_with(&project.to_string(), &libs, &|src| {
        src.load(&dir).ok()
    })
    .unwrap();
    let (_, job) = &jobs[0];
    let mesh = marcam::load_mesh(job, &dir).unwrap();
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    marcam::plan_with_mesh(job, mesh.as_ref())
        .unwrap()
        .iter()
        .map(|t| marcam::post(t, &m.post))
        .collect()
}

fn plate() -> serde_json::Value {
    let text = std::fs::read_to_string(root().join("examples/plate/plate.marcam.toml")).unwrap();
    serde_json::from_str(&marcam::project_file::to_json_text(&text).unwrap()).unwrap()
}

#[test]
fn an_unindexed_tool_gets_the_next_free_number_and_a_warning() {
    let mut p = plate();
    // the dome's ball (T3) has no number yet: T1 and T4 are taken, so it is written as T2
    p["unindexed"] = serde_json::json!([3]);
    let g = gcode(&p).join("\n");
    assert!(g.contains("T2"), "written as the next free number");
    assert!(!g.contains("M6 T3"), "not as its project number");
    assert!(
        g.contains("has no tool number in the project: written as T2"),
        "warned"
    );
    // indexed: as before, no warning
    let g0 = gcode(&plate()).join("\n");
    assert!(g0.contains("M6 T3") && !g0.contains("Warning"));
}

#[test]
fn one_tool_under_two_numbers_is_warned_about() {
    let mut p = plate();
    // T5: the same library tool as T1, used by the pocket
    p["tools"].as_array_mut().unwrap().push(serde_json::json!({"number": 5, "library": "makera-starter", "tool": "spiral-o-metal-3.175x12"}));
    let ops = p["setups"][0]["operations"].as_array_mut().unwrap();
    ops.iter_mut().find(|o| o["type"] == "pocket").unwrap()["tool"] = serde_json::json!(5);
    let g = gcode(&p).join("\n");
    assert!(g.contains("is both T1 and T5"), "{}", &g[..400]);
}
