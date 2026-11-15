//! M2: mesh roughing, material-removal simulation and gouge checks, end to end on the knob.

use fourcam::{gouge_check, load_mesh, plan, plan_with_mesh, sim, simulate_job, Job};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn job(name: &str) -> Job {
    let text = std::fs::read_to_string(root().join("examples/knob").join(name)).unwrap();
    Job::from_json(&text).unwrap()
}

/// Deviation over the knob body and boss, away from the front chamfer (which a flat or
/// ball tool cannot make; the V-bit flank chamfer does it) and the back end.
fn body_deviation(res: &fourcam::JobSim, program: usize) -> sim::Deviation {
    let _ = program;
    sim::deviation(&res.stock, &res.part, -21.5, -1.0, 0.02)
}

#[test]
fn mesh_rough_then_finish_matches_part() {
    let mut job = job("knob_finish.job.json");
    // rough + flat helical finish (the ball raster is covered by finish.rs)
    job.programs.retain(|p| p.name != "knob_4_finish_along");
    let mesh = load_mesh(&job, &root().join("examples/knob")).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();

    for g in gouge_check(&job, &tps, mesh.as_ref()).unwrap() {
        eprintln!(
            "{}: gouge check {} moves, deepest {:.4} mm",
            g.program, g.moves_checked, g.max_depth
        );
        assert!(g.moves_checked > 1000);
        assert!(g.max_depth < 0.01, "{g:?}");
    }

    let res = simulate_job(&job, &tps[..1], mesh.as_ref(), 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[0];
    let body = body_deviation(&res, 0);
    eprintln!("after rough: {dev:?}\n  body: {body:?}");
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0);
    // roughing leaves at least the allowance (0.15 mm) everywhere ...
    let min_left = min_left(&res, -21.5, -1.0);
    assert!(
        min_left > 0.15 - 0.02,
        "rough cut into the allowance: {min_left}"
    );
    // ... and at most one step (0.5 mm) plus allowance plus what the taper adds under a level
    assert!(body.max_left < 0.5 + 0.15 + 0.5, "{body:?}");

    let res = simulate_job(&job, &tps, mesh.as_ref(), 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[1];
    let body = body_deviation(&res, 1);
    eprintln!("after finish: {dev:?}\n  body: {body:?}");
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0);
    assert!(body.max_left < 0.06, "finish left {body:?}");
    // a flat tool on the 7.6 deg taper leaves a sawtooth up to stepover * slope (0.4 * 0.133)
    assert!(body.mean_left < 0.03, "finish left {body:?}");
}

fn min_left(res: &fourcam::JobSim, x_lo: f64, x_hi: f64) -> f64 {
    let g = res.stock.grid;
    let mut m = f64::INFINITY;
    for i in 0..g.nx {
        let x = g.x(i);
        if x < x_lo || x > x_hi {
            continue;
        }
        for j in 0..g.na {
            if res.part.at(i, j) > 0.0 {
                m = m.min(res.stock.at(i, j) - res.part.at(i, j));
            }
        }
    }
    m
}

#[test]
fn profile_program_simulates_cleanly() {
    // the G-code that was cut on the Z1: no rapids through stock, no gouges, and the
    // body comes out within the finishing scallop except at the wall feet and chamfer
    let job = job("knob.job.json");
    let tps = plan(&job).unwrap();
    let res = simulate_job(&job, &tps[..1], None, 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[0];
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0, "{dev:?}");
    // taper only (between the wall at -15 and the chamfer)
    let taper = sim::deviation(&res.stock, &res.part, -14.8, -1.0, 0.02);
    eprintln!("profile program, taper: {taper:?}");
    assert!(taper.max_left < 0.07, "{taper:?}");
    for g in gouge_check(&job, &tps, None).unwrap() {
        assert!(g.max_depth < 0.01, "{g:?}");
    }
}

#[test]
fn rough_never_cuts_below_allowance_surface() {
    let job = job("knob_finish.job.json");
    let mesh = load_mesh(&job, &root().join("examples/knob"))
        .unwrap()
        .unwrap();
    let mut rough_only = job.clone();
    rough_only.programs.truncate(1);
    // check against the part grown by the allowance: depth must stay below 0.15 + tol
    let tps = plan_with_mesh(&rough_only, Some(&mesh)).unwrap();
    let g = &gouge_check(&rough_only, &tps, Some(&mesh)).unwrap()[0];
    assert!(g.max_depth < 0.01);
    // A keeps turning one way within a level and levels alternate
    let mut a_last = 0.0;
    let mut dirs = Vec::new();
    for m in &tps[0].moves {
        if let fourcam::Move::Cut { a, .. } = m {
            let d = (a - a_last).signum();
            if d != 0.0 && dirs.last() != Some(&d) {
                dirs.push(d);
            }
            a_last = *a;
        }
    }
    assert!(
        dirs.len() >= 10,
        "levels should alternate direction: {dirs:?}"
    );
}
