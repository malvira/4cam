//! A setup's work zero (Mar: "setups need a way to change the WCS"): a vise setup's
//! post.origin moves the program's X0 Y0 Z0. Nothing planned changes; the G-code's numbers are
//! written from there, and it still verifies clean.

use marcam::project::Project;
use marcam::{limits_for, plan_with_mesh, post, verify_gcode};
use serde_json::{json, Value};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plate")
}

fn plate(origin: Option<[f64; 3]>) -> marcam::Job {
    let text = std::fs::read_to_string(dir().join("plate.marcam.toml")).unwrap();
    let (p, _) = Project::from_json_any(&text).unwrap();
    let mut v: Value = serde_json::to_value(&p).unwrap();
    if let Some(o) = origin {
        v["setups"][0]["post"] = json!({ "origin": o });
    }
    let libs = marcam::profiles::builtin_libraries();
    let d = dir();
    marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&d).ok())
        .unwrap()
        .remove(0)
        .1
}

/// Every X, Y and Z word of the program, in order.
fn words(g: &str) -> Vec<(char, f64)> {
    g.lines()
        .filter(|l| !l.starts_with('(') && !l.starts_with(';'))
        .flat_map(|l| {
            l.split_whitespace()
                .filter_map(|w| {
                    let c = w.chars().next()?;
                    if "XYZ".contains(c) {
                        w[1..].parse::<f64>().ok().map(|v| (c, v))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_work_zero_moves_the_programs_numbers_not_the_cuts() {
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let run = |o: Option<[f64; 3]>| {
        let job = plate(o);
        let mesh = marcam::load_mesh(&job, &dir()).unwrap();
        let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
        tps.iter()
            .map(|tp| {
                let g = post(tp, &m.post);
                let issues = verify_gcode(&g, &limits_for(tp, &m.post));
                assert!(
                    issues.is_empty(),
                    "{o:?}: {:?}",
                    issues.iter().map(|i| i.to_string()).collect::<Vec<_>>()
                );
                (tp.moves.clone(), g)
            })
            .collect::<Vec<_>>()
    };
    let base = run(None);
    for o in [[30.0, 20.0, 0.0], [0.0, 0.0, -10.0], [60.0, 40.0, -10.0]] {
        let moved = run(Some(o));
        for ((m0, g0), (m1, g1)) in base.iter().zip(&moved) {
            // planned the same
            assert_eq!(m0, m1);
            // written from the new zero
            let (w0, w1) = (words(g0), words(g1));
            assert_eq!(w0.len(), w1.len());
            for (a, b) in w0.iter().zip(&w1) {
                let k = "XYZ".find(a.0).unwrap();
                assert_eq!(a.0, b.0);
                assert!((a.1 - o[k] - b.1).abs() < 0.0011, "{o:?}: {a:?} -> {b:?}");
            }
            assert!(g1.contains("Work zero"), "{g1}");
        }
    }
    // none: nothing written of it
    let j = plate(None);
    assert!(j.post.as_ref().is_none_or(|p| p.origin == [0.0; 3]));
}
