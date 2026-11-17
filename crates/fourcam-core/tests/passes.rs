//! Level-based passes (raster3d, rough3d): a level that holds the tool up nowhere is the
//! surface pass itself, and deeper levels would repeat it, so they are not cut. Mar saw an
//! un-roughed parallel finish cut the same surface over and over.

use fourcam::{load_mesh, plan_with_mesh, Job, Move, Toolpath};
use serde_json::json;
use std::path::Path;

const TIMING: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/timing");

/// A raster3d over the timing fixture's flat plate (30 x 20, top at Z0), from a block whose
/// top is at `top`, down to `floor`, `step_down` per level.
fn raster(top: f64, floor: f64, step_down: f64) -> Toolpath {
    let project = json!({
        "schema_version": "0.2", "name": "passes",
        "models": [{"id": "plate", "name": "plate", "kind": "mesh", "file": "plate.stl"}],
        "tools": [{"number": 1, "description": "4 mm flat", "kind": "flat", "diameter": 4.0, "flute_length": 15}],
        "setups": [{
            "id": "vise", "machine": "makera-carvera-4axis",
            "stock": {"shape": "box", "min": [0.0, 0.0, -5.0], "max": [30.0, 20.0, top]},
            "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false},
            "part": ["plate"], "spindle_rpm": 12000,
            "operations": [{
                "id": "op1", "type": "raster3d", "tool": 1, "x_range": [0.0, 30.0], "y_range": [0.0, 20.0],
                "stepover": 2.0, "sample": 1.0, "step_down": step_down, "floor": floor, "scallop": 0.0,
                "feed": 600.0, "plunge_feed": 60.0
            }],
            "outputs": [{"name": "passes", "title": "passes", "operations": ["op1"]}]
        }]
    });
    plan_project(&project.to_string(), Path::new(TIMING)).remove(0)
}

fn jobs(text: &str, dir: &Path) -> Vec<Job> {
    let libs = fourcam::profiles::builtin_libraries();
    let d = dir.to_path_buf();
    let meshes = |src: &fourcam::job::MeshSource| src.load(&d).ok();
    fourcam::project::jobs_from_any_with(text, &libs, &meshes)
        .unwrap()
        .into_iter()
        .map(|(_, j)| j)
        .collect()
}

fn plan_project(text: &str, dir: &Path) -> Vec<Toolpath> {
    jobs(text, dir)
        .iter()
        .flat_map(|job| {
            let mesh = load_mesh(job, dir).unwrap();
            plan_with_mesh(job, mesh.as_ref()).unwrap()
        })
        .collect()
}

/// The cutting moves of each pass: a pass starts at a vertical plunge (3-axis entry) or a
/// radial plunge (rotary). Points rounded to a micron.
fn passes(tp: &Toolpath) -> Vec<Vec<[i64; 4]>> {
    let q = |v: f64| (v * 1000.0).round() as i64;
    let mut out: Vec<Vec<[i64; 4]>> = Vec::new();
    let mut at: Option<(f64, f64)> = None;
    for m in &tp.moves {
        match m {
            Move::Rapid3 { x, y, .. } => {
                if let (Some(x), Some(y)) = (x, y) {
                    at = Some((*x, *y));
                }
            }
            Move::Line3 { x, y, z, .. } => {
                let vertical =
                    at.is_some_and(|(px, py)| (px - x).abs() < 1e-9 && (py - y).abs() < 1e-9);
                if out.is_empty() || (vertical && out.last().is_some_and(|p| !p.is_empty())) {
                    out.push(Vec::new());
                }
                out.last_mut().unwrap().push([q(*x), q(*y), q(*z), 0]);
                at = Some((*x, *y));
            }
            Move::Plunge { .. } => out.push(Vec::new()),
            Move::Cut { x, r, a, .. } => {
                if out.is_empty() {
                    out.push(Vec::new());
                }
                out.last_mut().unwrap().push([q(*x), 0, q(*r), q(*a)]);
            }
            _ => {}
        }
    }
    out.retain(|p| p.len() > 1);
    out
}

/// Levels: the raster's passes, by their entry plunges.
fn level_count(tp: &Toolpath) -> usize {
    passes(tp).len()
}

#[test]
fn a_step_down_beyond_the_depth_is_one_pass() {
    // the block's top is the plate's top: one level, whatever the step
    for step in [10.0, 5.0, 1.0, 0.4] {
        assert_eq!(level_count(&raster(0.0, -5.0, step)), 1, "step {step}");
    }
    // 5 mm of block above the plate, step-down 5 or more: one pass at the surface
    assert_eq!(level_count(&raster(5.0, 0.0, 5.0)), 1);
    assert_eq!(level_count(&raster(5.0, 0.0, 8.0)), 1);
}

#[test]
fn depth_over_step_of_two_and_a_half_is_three_passes() {
    // 5 mm above the surface in steps of 2: levels at 3.33, 1.67 and 0 (the surface)
    assert_eq!(level_count(&raster(5.0, 0.0, 2.0)), 3);
    // and a floor below the surface adds nothing: the level at the surface holds the tool up
    // nowhere and is the last
    let deep = raster(5.0, -5.0, 2.0);
    let n = level_count(&deep);
    assert!(n <= 3, "{n} passes");
    let p = passes(&deep);
    assert_ne!(p[n - 1], p[n - 2], "the last two passes differ");
}

#[test]
fn no_pass_repeats_the_one_before_it() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples"));
    for file in [
        "knob/knob.project.json",
        "knob/knob_finish.project.json",
        "knob/knob_indexed.project.json",
        "plate/plate.project.json",
        "ring/ring.project.json",
        "plaque/plaque.project.json",
    ] {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        for tp in plan_project(&text, path.parent().unwrap()) {
            let p = passes(&tp);
            for k in 1..p.len() {
                assert_ne!(
                    p[k],
                    p[k - 1],
                    "{file} {}: pass {k} repeats pass {}",
                    tp.name,
                    k - 1
                );
            }
        }
    }
}

/// Mar's case: STL imported into a vise setup, the starter finishing raster3d with no
/// roughing before it. Its engine defaults take the whole depth in one level; and even at a
/// fine step-down, no level below the part is cut twice.
#[test]
fn an_unroughed_parallel_finish_cuts_the_surface_once() {
    let plate = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/plate"));
    let text = std::fs::read_to_string(plate.join("plate.project.json")).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    // only the dome finish, none of the other operations
    let project = fourcam::project::Project::from_json_any(&doc.to_string())
        .unwrap()
        .0;
    let ops = doc["setups"][0]["operations"].as_array().unwrap().clone();
    let mut dome = ops.into_iter().find(|o| o["type"] == "raster3d").unwrap();
    let defaults = project
        .op_defaults(
            "raster3d",
            "vise1",
            Some(dome["tool"].as_u64().unwrap() as u32),
            &[],
            &fourcam::profiles::builtin_libraries(),
            None,
            &|_| None,
        )
        .unwrap();
    let setup = &mut doc["setups"][0];
    let top = setup["stock"]["max"][2].as_f64().unwrap();
    let floor = defaults["floor"].as_f64().unwrap();
    assert!(
        (defaults["step_down"].as_f64().unwrap() - (top - floor)).abs() < 0.011,
        "an un-roughed raster3d defaults to one full-depth level: {defaults}"
    );
    // a fine step-down: the passes stop at the first level below the whole surface
    dome["step_down"] = json!(0.4);
    setup["operations"] = json!([dome]);
    setup["outputs"] =
        json!([{"name": "dome", "title": "dome", "operations": [dome["id"].clone()]}]);
    let tps = plan_project(&doc.to_string(), plate);
    let p = passes(&tps[0]);
    for k in 1..p.len() {
        assert_ne!(p[k], p[k - 1], "pass {k} repeats");
    }
    // the block's top is 0 and the dome's area bottoms out at the plate (-0.5 after facing
    // is not in this job: the raster's own lowest point decides)
    let lowest = p.last().unwrap().iter().map(|q| q[2]).min().unwrap() as f64 / 1000.0;
    let expect = ((top - lowest) / 0.4).ceil() as usize + 1;
    assert!(
        p.len() <= expect,
        "{} passes for {:.2} mm of depth at 0.4",
        p.len(),
        top - lowest
    );
}
