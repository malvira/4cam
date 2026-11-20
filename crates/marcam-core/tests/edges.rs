//! Chamfers and threads on the plate (3-axis): a 0.5 mm chamfer round its outline and its
//! holes with the 90° V-bit, the cone on the edge and the point clear of it; an M6 x 1 thread
//! in a hole with a 60° single-form thread mill, a pitch a turn, climbing up
//! counter-clockwise (right hand), posted as G3 helices, verifying clean.

use marcam::job::{Job, Operation};
use marcam::{limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Move};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The plate with only `op` (and a 60° thread mill as T5).
fn plate_with(op: Value) -> Job {
    let dir = root().join("examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let (p, _) = marcam::project::Project::from_json_any(&text).unwrap();
    let mut v = serde_json::to_value(&p).unwrap();
    v["tools"].as_array_mut().unwrap().push(json!({"number": 5, "description": "thread mill 60 deg", "kind": "vbit", "angle": 60.0, "diameter": 4.0}));
    v["setups"][0]["operations"] = json!([op]);
    v["setups"][0]["outputs"] = json!([{"name": "edges", "title": "edges", "operations": ["e1"]}]);
    let libs = marcam::profiles::builtin_libraries();
    marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&dir).ok())
        .unwrap()
        .remove(0)
        .1
}

fn cuts(job: &Job) -> (Vec<[f64; 3]>, Vec<marcam::Toolpath>) {
    let mesh = load_mesh(job, &root().join("examples/plate")).unwrap();
    let tps = plan_with_mesh(job, mesh.as_ref()).unwrap();
    let pts = tps
        .iter()
        .flat_map(|t| &t.moves)
        .filter_map(|m| match m {
            Move::Line3 { x, y, z, .. } => Some([*x, *y, *z]),
            _ => None,
        })
        .collect();
    (pts, tps)
}

fn clean(job: &Job, tps: &[marcam::Toolpath]) {
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    for tp in tps {
        let g = post(tp, &m.post);
        let issues = verify_gcode(&g, &limits_for(tp, &m.post));
        assert!(
            issues.is_empty(),
            "{:?}",
            issues.iter().map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }
    // a chamfer or a thread cuts past the part on purpose: no gouge reported
    let mesh = load_mesh(job, &root().join("examples/plate")).unwrap();
    let g = marcam::gouge_check(job, tps, mesh.as_ref()).unwrap();
    assert!(g.iter().all(|r| r.max_depth < 0.02), "{g:?}");
}

#[test]
fn a_chamfer_round_the_outline_and_the_holes() {
    let job = plate_with(
        json!({"id": "e1", "type": "chamfer", "tool": 4, "paths": [[[2.0, 2.0], [58.0, 2.0], [58.0, 38.0], [2.0, 38.0]]],
        "side": "outside", "holes": [[40.0, 6.0, 3.175]], "width": 0.5, "top": -0.5, "tip_clearance": 0.2, "feed": 300.0, "plunge_feed": 60.0}),
    );
    assert!(matches!(
        job.programs[0].operations[0],
        Operation::Chamfer(_)
    ));
    let (pts, tps) = cuts(&job);
    // 90°: the tip 0.5 + 0.2 below the top, its axis 0.2 out from the edge
    let z = -0.5 - 0.7;
    assert!(pts.iter().all(|p| (p[2] - z).abs() < 1e-9), "{pts:?}");
    let outline: Vec<&[f64; 3]> = pts.iter().filter(|p| p[0] < 30.0 || p[1] > 10.0).collect();
    assert!(
        outline
            .iter()
            .any(|p| (p[0] - 1.8).abs() < 1e-6 && (p[1] - 1.8).abs() < 1e-6),
        "{outline:?}"
    );
    // the outline climb-milled: clockwise
    let ring: Vec<[f64; 2]> = pts
        .iter()
        .filter(|p| (p[0] - 40.0).hypot(p[1] - 6.0) > 2.0)
        .map(|p| [p[0], p[1]])
        .collect();
    let area: f64 = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    assert!(area < 0.0, "{area}");
    // the hole: round it at its radius less 0.2
    let hole: Vec<f64> = pts
        .iter()
        .map(|p| (p[0] - 40.0).hypot(p[1] - 6.0))
        .filter(|r| *r < 2.0)
        .collect();
    assert!(!hole.is_empty() && hole.iter().all(|r| (r - (3.175 / 2.0 - 0.2)).abs() < 1e-6));
    clean(&job, &tps);
}

#[test]
fn an_m6_thread_in_a_hole() {
    let job = plate_with(
        json!({"id": "e1", "type": "thread_mill", "tool": 5, "points": [[30.0, 20.0]], "diameter": 6.0, "pitch": 1.0,
        "depth": 4.0, "top": -0.5, "internal": true, "hand": "right", "passes": 2, "thread_depth": 0.54, "feed": 200.0, "plunge_feed": 60.0}),
    );
    let (pts, tps) = cuts(&job);
    // the helices: radius 3 - 2 at the end, 0.27 less on the first pass
    let r: Vec<f64> = pts
        .iter()
        .map(|p| (p[0] - 30.0).hypot(p[1] - 20.0))
        .collect();
    let rmax = r.iter().cloned().fold(0.0, f64::max);
    assert!((rmax - 1.0).abs() < 1e-6, "{rmax}");
    assert!(r.iter().any(|x| (x - 0.73).abs() < 1e-6));
    // a pitch a turn, climbing: from 4.5 below the top to half a pitch above it
    let on: Vec<&[f64; 3]> = pts
        .iter()
        .filter(|p| ((p[0] - 30.0).hypot(p[1] - 20.0) - 1.0).abs() < 1e-6)
        .collect();
    let (lo, hi) = (
        on.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min),
        on.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(
        (lo - (-0.5 - 4.5)).abs() < 1e-6 && (hi - 0.0).abs() < 1e-6,
        "{lo} {hi}"
    );
    let first = on.iter().position(|p| p[2] > lo + 1e-9).unwrap();
    let (a, b) = (on[first - 1], on[first]);
    // counter-clockwise going up: right hand
    let turn = (a[0] - 30.0) * (b[1] - 20.0) - (a[1] - 20.0) * (b[0] - 30.0);
    assert!(turn > 0.0 && b[2] > a[2]);
    // a pitch a turn: up pitch / 2 pi per radian
    let ang = |p: &[f64; 3]| (p[1] - 20.0).atan2(p[0] - 30.0);
    let mut d = ang(b) - ang(a);
    if d < 0.0 {
        d += std::f64::consts::TAU;
    }
    assert!(
        ((b[2] - a[2]) / d - 1.0 / std::f64::consts::TAU).abs() < 1e-6,
        "{a:?} {b:?}"
    );
    // posted as G3 helices on the vise
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let g = post(&tps[0], &m.post);
    assert!(
        g.lines().filter(|l| l.starts_with("G3 ")).count() >= 2,
        "{g}"
    );
    clean(&job, &tps);
    // left hand: down counter-clockwise
    let lh = plate_with(
        json!({"id": "e1", "type": "thread_mill", "tool": 5, "points": [[30.0, 20.0]], "diameter": 6.0, "pitch": 1.0,
        "depth": 4.0, "top": -0.5, "hand": "left", "feed": 200.0, "plunge_feed": 60.0}),
    );
    let (lp, _) = cuts(&lh);
    let on: Vec<&[f64; 3]> = lp
        .iter()
        .filter(|p| ((p[0] - 30.0).hypot(p[1] - 20.0) - 1.0).abs() < 1e-6)
        .collect();
    let i = on.iter().position(|p| p[2] < on[0][2] - 1e-9).unwrap();
    assert!(on[i][2] < on[i - 1][2]);
}
