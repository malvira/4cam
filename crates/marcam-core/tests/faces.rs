//! Rasters bounded by selected faces: the dome's cap on the plate (3-axis raster3d) and a
//! cone of the knob (rotary finish, both patterns). Simulated: the selected face is cut to
//! the part, nothing away from it is touched, and nothing is gouged.

use marcam::sim3::Heightmap3;
use marcam::{cutter_for, plan_with_mesh, Job};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A project with a face selection on operation `op` (and, if given, its other fields set).
fn with_faces(path: &str, op: &str, model: &str, ids: &[&str], set: Value) -> (Job, marcam::Mesh) {
    let text = std::fs::read_to_string(root().join(path)).unwrap();
    let dir = root().join(path).parent().unwrap().to_path_buf();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    for o in v["setups"][0]["operations"].as_array_mut().unwrap() {
        if o["id"] == op {
            o["geometry"] = json!([{"kind": "faces", "model": model, "ids": ids}]);
            for (k, x) in set.as_object().unwrap() {
                o[k] = x.clone();
            }
        }
    }
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&dir).ok())
            .unwrap();
    let job = jobs.remove(0).1;
    let mesh = marcam::load_mesh(&job, &dir).unwrap().unwrap();
    (job, mesh)
}

/// The dome: a cap of a 6.5 mm sphere centred at (46, 20, -7), 6 mm across at Z -4.5.
const CAP_R: f64 = 6.0;

#[test]
fn raster_on_the_dome_cap_only() {
    let (job, mesh) = with_faces(
        "examples/plate/plate.project.json",
        "op5",
        "dome",
        &["f049d19c2"],
        json!({}),
    );
    let tps = plan_with_mesh(&job, Some(&mesh)).unwrap();
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    for tp in &tps {
        let text = marcam::post(tp, &m.post);
        assert!(
            marcam::verify_gcode(&text, &marcam::limits_for(tp, &m.post)).is_empty(),
            "{}",
            tp.name
        );
    }
    let sb = job.stock_box.unwrap();
    let cut = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
    let mut hm = Heightmap3::new(&sb, 0.1);
    marcam::sim3::simulate3(&mut hm, &tps[0], &cut, 0.05);
    let before = hm.z.clone();
    let st = marcam::sim3::simulate3(&mut hm, &tps[1], &cut, 0.05);
    assert!(st.rapid_hits.is_empty(), "rapids through stock");
    let part = marcam::sim3::part_top(&hm, &mesh);
    let dev = marcam::sim3::deviation3(&hm, &part, 0.02);
    assert!(dev.max_gouge < 0.02, "gouge {}", dev.max_gouge);
    let r = 3.175 / 2.0; // T3, a ball
    let (mut on, mut off) = (0, 0);
    for j in 0..hm.ny {
        for i in 0..hm.nx {
            let (x, y) = hm.centre(i, j);
            let d = (x - 46.0).hypot(y - 20.0);
            let k = j * hm.nx + i;
            if d < CAP_R - 0.5 {
                // on the cap: down to it (within the scallop)
                let top = part[k].unwrap();
                assert!(
                    hm.z[k] - top < 0.05,
                    "left {} at ({x:.1}, {y:.1})",
                    hm.z[k] - top
                );
                on += 1;
            } else if d > CAP_R + 2.0 * r + 0.2 {
                // away from it: what the first program left, untouched. (A tool resting on
                // the cap's rim is on the cap; its footprint reaches a diameter beyond.)
                assert_eq!(hm.z[k], before[k], "cut at ({x:.1}, {y:.1})");
                off += 1;
            }
        }
    }
    assert!(on > 8000 && off > 100_000, "{on} {off}");
}

#[test]
fn rotary_finish_on_one_cone_of_the_knob() {
    // the knob's 814.9 mm2 cone, X -15..-0.8, radius 10 down to 8.107
    for (op, pattern) in [("op2", "along"), ("op3", "around")] {
        let (job, mesh) = with_faces(
            "examples/knob/knob_finish.project.json",
            op,
            "mesh",
            &["fbbf34e63"],
            json!({}),
        );
        let tps = plan_with_mesh(&job, Some(&mesh)).unwrap();
        // the programs before the finish, then the finish
        let k = tps
            .iter()
            .position(|tp| {
                job.programs
                    .iter()
                    .find(|p| p.name == tp.name)
                    .unwrap()
                    .operations
                    .iter()
                    .any(|o| matches!(o, marcam::job::Operation::RotaryFinish(f) if !f.faces.is_empty()))
            })
            .expect("a finish with faces");
        let grid = marcam::sim::Grid::new(-25.0, 3.0, 0.1, 360);
        let mut stock = marcam::sim::RadialMap::filled(grid, job.stock.radius);
        let cut = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
        for tp in &tps[..k] {
            marcam::sim::simulate(&mut stock, tp, &cut, 0.1);
        }
        let before = stock.r.clone();
        let st = marcam::sim::simulate(&mut stock, &tps[k], &cut, 0.1);
        assert!(st.rapid_hits.is_empty(), "{pattern}: rapids through stock");
        let part = marcam::sim::Part::from_mesh(grid, &mesh);
        let dev = marcam::sim::deviation(&stock, &part, -21.7, 0.0, 0.02);
        assert!(dev.max_gouge < 0.02, "{pattern}: gouge {}", dev.max_gouge);
        let r = 3.175 / 2.0;
        let (mut on, mut off) = (0, 0);
        for i in 0..grid.nx {
            let x = grid.x(i);
            for j in 0..grid.na {
                let c = i * grid.na + j;
                if x > -14.0 && x < -1.8 {
                    // on the cone: finished to it
                    let left = stock.r[c] as f64 - part.center.r[c] as f64;
                    assert!(left < 0.05, "{pattern}: left {left} at x {x:.2}");
                    on += 1;
                } else if x < -15.0 - 2.0 * r - 0.3 || x > -0.8 + 2.0 * r + 0.3 {
                    // (a tool resting on the cone's edge reaches a diameter past it)
                    assert_eq!(stock.r[c], before[c], "{pattern}: cut at x {x:.2}");
                    off += 1;
                }
            }
        }
        assert!(on > 40_000 && off > 20_000, "{pattern}: {on} {off}");
    }
}

#[test]
fn selections_that_do_not_apply_are_refused() {
    // raster3d takes faces only
    let text = std::fs::read_to_string(root().join("examples/plate/plate.project.json")).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    for o in v["setups"][0]["operations"].as_array_mut().unwrap() {
        if o["id"] == "op5" {
            o["geometry"] = json!([{"kind": "axial_range", "x": [40.0, 50.0]}]);
        }
    }
    let dir = root().join("examples/plate");
    let libs = marcam::profiles::builtin_libraries();
    let e = marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&dir).ok())
        .unwrap_err();
    assert!(e.contains("raster3d takes faces"), "{e}");
    // an unknown face is named
    let (job, mesh) = {
        let (j, m) = with_faces(
            "examples/plate/plate.project.json",
            "op5",
            "dome",
            &["f049d19c2"],
            json!({}),
        );
        (j, m)
    };
    let mut bad = job.clone();
    for p in &mut bad.programs {
        for o in &mut p.operations {
            if let marcam::job::Operation::Raster3d(r) = o {
                r.faces = vec!["fdeadbeef".into()];
            }
        }
    }
    let e = plan_with_mesh(&bad, Some(&mesh)).unwrap_err().to_string();
    assert!(e.contains("no face \"fdeadbeef\""), "{e}");
}
