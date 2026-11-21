//! A boundary offset (Mar) grows or shrinks what facing faces and moves a 2D contour out or in,
//! beyond the tool's radius: the cuts' extents move by it; 0 (the default) changes nothing.

use marcam::job::Job;
use marcam::{plan_with_mesh, Move};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The plate (60 x 40 block, T1 3.175 mm flat) with only `op`.
fn plate_with(op: Value) -> Job {
    let dir = root().join("examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let (p, _) = marcam::project::Project::from_json_any(&text).unwrap();
    let mut v = serde_json::to_value(&p).unwrap();
    v["setups"][0]["operations"] = json!([op]);
    v["setups"][0]["outputs"] = json!([{"name": "o", "title": "o", "operations": ["o1"]}]);
    let libs = marcam::profiles::builtin_libraries();
    marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&dir).ok())
        .unwrap()
        .remove(0)
        .1
}

/// The cutting moves' points below `z` (x, y, z).
fn cuts(job: &Job, below: f64) -> Vec<[f64; 3]> {
    let tps = plan_with_mesh(job, None).unwrap();
    tps.iter()
        .flat_map(|t| &t.moves)
        .filter_map(|m| match m {
            Move::Line3 { x, y, z, .. } if *z < below => Some([*x, *y, *z]),
            _ => None,
        })
        .collect()
}

/// [min x, max x, min y, max y]
fn extents(p: &[[f64; 3]]) -> [f64; 4] {
    let f = |k: usize, max: bool| {
        p.iter().map(|q| q[k]).fold(
            if max {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            },
            |a, b| if max { a.max(b) } else { a.min(b) },
        )
    };
    [f(0, false), f(0, true), f(1, false), f(1, true)]
}

fn face(extra: Value) -> Value {
    let mut o = json!({"id": "o1", "type": "face", "tool": 1, "depth": 0.5, "step_down": 0.5, "stepover": 0.6, "feed": 600.0, "plunge_feed": 200.0});
    for (k, v) in extra.as_object().unwrap() {
        o[k] = v.clone();
    }
    o
}

#[test]
fn facing_the_whole_stock_grows_by_the_offset() {
    let a = extents(&cuts(&plate_with(face(json!({}))), -0.25));
    let b = extents(&cuts(
        &plate_with(face(json!({"boundary_offset": 3.0}))),
        -0.25,
    ));
    let c = extents(&cuts(
        &plate_with(face(json!({"boundary_offset": -2.0}))),
        -0.25,
    ));
    for k in 0..4 {
        let s = if k % 2 == 0 { -1.0 } else { 1.0 };
        assert!((b[k] - a[k] - 3.0 * s).abs() < 1e-9, "{a:?} {b:?}");
        assert!((c[k] - a[k] + 2.0 * s).abs() < 1e-9, "{a:?} {c:?}");
    }
}

#[test]
fn facing_a_picked_face_grows_by_the_offset_and_its_holes_shrink() {
    // a 40 x 20 face with a round hole of radius 4 in it
    let hole: Vec<[f64; 2]> = (0..48)
        .rev()
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 48.0;
            [30.0 + 4.0 * a.cos(), 20.0 + 4.0 * a.sin()]
        })
        .collect();
    let paths = json!([
        [[10.0, 10.0], [50.0, 10.0], [50.0, 30.0], [10.0, 30.0]],
        hole
    ]);
    let a = cuts(&plate_with(face(json!({"paths": paths}))), -0.25);
    let b = cuts(
        &plate_with(face(json!({"paths": paths, "boundary_offset": 2.0}))),
        -0.25,
    );
    let c = cuts(
        &plate_with(face(json!({"paths": paths, "boundary_offset": -1.5}))),
        -0.25,
    );
    let (ea, eb, ec) = (extents(&a), extents(&b), extents(&c));
    // (the loops come from a field sampled every 0.05 mm)
    for k in 0..4 {
        let s = if k % 2 == 0 { -1.0 } else { 1.0 };
        assert!((eb[k] - ea[k] - 2.0 * s).abs() < 0.08, "{ea:?} {eb:?}");
        assert!((ec[k] - ea[k] + 1.5 * s).abs() < 0.08, "{ea:?} {ec:?}");
    }
    // the hole: the nearest the tool comes to its centre, 2 mm nearer
    let near = |p: &[[f64; 3]]| {
        p.iter()
            .map(|q| (q[0] - 30.0).hypot(q[1] - 20.0))
            .fold(f64::INFINITY, f64::min)
    };
    assert!(
        (near(&a) - near(&b) - 2.0).abs() < 0.08,
        "{} {}",
        near(&a),
        near(&b)
    );
}

fn contour(side: &str, extra: Value) -> Value {
    let mut o = json!({"id": "o1", "type": "contour", "tool": 1, "path": [[10.0, 10.0], [50.0, 10.0], [50.0, 30.0], [10.0, 30.0]],
        "side": side, "depth": 2.0, "step_down": 1.0, "feed": 600.0, "plunge_feed": 200.0});
    for (k, v) in extra.as_object().unwrap() {
        o[k] = v.clone();
    }
    o
}

#[test]
fn a_contour_moves_out_or_in_by_the_offset_and_its_tabs_with_it() {
    let r = 3.175 / 2.0;
    for (side, base) in [("outside", r), ("inside", -r), ("on", 0.0)] {
        for g in [0.0, 1.5, -0.75] {
            let e = extents(&cuts(
                &plate_with(contour(side, json!({"boundary_offset": g}))),
                -0.25,
            ));
            let want = base + g;
            assert!(
                (e[0] - (10.0 - want)).abs() < 1e-6
                    && (e[1] - (50.0 + want)).abs() < 1e-6
                    && (e[2] - (10.0 - want)).abs() < 1e-6
                    && (e[3] - (30.0 + want)).abs() < 1e-6,
                "{side} {g}: {e:?}"
            );
        }
    }
    // two tabs: the passes over them (at the tab's top) on the moved contour
    let job = plate_with(contour(
        "outside",
        json!({"boundary_offset": 2.0, "tabs": 2, "tab_height": 1.0}),
    ));
    let tab_top = -2.0 + 1.0;
    let over: Vec<[f64; 3]> = cuts(&job, 10.0)
        .into_iter()
        .filter(|p| (p[2] - tab_top).abs() < 1e-9)
        .collect();
    assert!(!over.is_empty());
    let o = r + 2.0;
    assert!(
        over.iter().all(|p| (p[0] - (10.0 - o)).abs() < 1e-6
            || (p[0] - (50.0 + o)).abs() < 1e-6
            || (p[1] - (10.0 - o)).abs() < 1e-6
            || (p[1] - (30.0 + o)).abs() < 1e-6),
        "{over:?}"
    );
}
