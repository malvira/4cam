//! Toolpath smoothing (each operation's `smoothing` tolerance, default 0.005 mm finishing,
//! 0.01 roughing, 0 off): fewer moves, the path within the tolerance of the strategy's,
//! arcs as G2/G3 on a vise; checked by the verifier, the holder check, the gouge check and
//! the 3-axis simulation.

use marcam::job::{Job, Operation};
use marcam::sim3::Heightmap3;
use marcam::{
    collision_check, cutter_for, limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Move,
};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(rel: &str) -> (Job, PathBuf) {
    let path = root().join(rel);
    let dir = path.parent().unwrap().to_path_buf();
    let text = std::fs::read_to_string(&path).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    (jobs.remove(0).1, dir)
}

/// The job with every operation's smoothing set to `tol` (None: the defaults).
fn with_smoothing(mut job: Job, tol: Option<f64>) -> Job {
    for p in &mut job.programs {
        for op in &mut p.operations {
            let s = match op {
                Operation::RotaryTurning(o) => &mut o.smoothing,
                Operation::HelicalKnurl(o) => &mut o.smoothing,
                Operation::FlankChamfer(o) => &mut o.smoothing,
                Operation::RotaryFinish(o) => &mut o.smoothing,
                Operation::RotaryRough(o) => &mut o.smoothing,
                Operation::Raster3d(o) | Operation::Rough3d(o) => &mut o.smoothing,
                Operation::Engrave(o) => &mut o.smoothing,
                Operation::Vcarve(o) => &mut o.smoothing,
                Operation::Contour(o) => &mut o.smoothing,
                Operation::Pocket(o) => &mut o.smoothing,
                _ => continue,
            };
            *s = tol;
        }
    }
    job
}

#[test]
fn rotary_programs_smoothed_stay_on_the_part() {
    // the knob's mesh roughing and both finishes: far fewer moves, and the engine's gouge
    // check against the part (every move sampled along X, radius and A) as deep as unsmoothed
    let (job, dir) = load("examples/knob/knob_finish.job.json");
    let mesh = load_mesh(&job, &dir).unwrap();
    let raw_job = with_smoothing(job.clone(), Some(0.0));
    let raw = plan_with_mesh(&raw_job, mesh.as_ref()).unwrap();
    let smooth = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let g_raw = marcam::gouge_check(&raw_job, &raw, mesh.as_ref()).unwrap();
    let g = marcam::gouge_check(&job, &smooth, mesh.as_ref()).unwrap();
    for ((a, b), (ga, gb)) in raw.iter().zip(&smooth).zip(g_raw.iter().zip(&g)) {
        let cuts = |tp: &marcam::Toolpath| {
            tp.moves
                .iter()
                .filter(|m| matches!(m, Move::Cut { .. }))
                .count()
        };
        assert!(
            cuts(b) * 2 < cuts(a),
            "{}: {} -> {} cuts",
            a.name,
            cuts(a),
            cuts(b)
        );
        assert!(
            gb.max_depth <= ga.max_depth + 0.005 + 1e-9,
            "{}: gouge {} (unsmoothed {})",
            a.name,
            gb.max_depth,
            ga.max_depth
        );
        eprintln!(
            "{}: {} -> {} cuts; deepest {:.4} (unsmoothed {:.4})",
            a.name,
            cuts(a),
            cuts(b),
            gb.max_depth,
            ga.max_depth
        );
    }
}

#[test]
fn the_plate_smoothed_verifies_clears_the_holder_and_does_not_gouge() {
    let (job, dir) = load("examples/plate/plate.marcam.toml");
    let mesh = load_mesh(&job, &dir).unwrap();
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let raw = plan_with_mesh(&with_smoothing(job.clone(), Some(0.0)), mesh.as_ref()).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let lines = |tps: &[marcam::Toolpath]| {
        tps.iter()
            .map(|tp| post(tp, &m.post).lines().count())
            .sum::<usize>()
    };
    let (before, after) = (lines(&raw), lines(&tps));
    assert!(after * 10 < before * 9, "{before} -> {after} lines");
    for tp in &tps {
        let text = post(tp, &m.post);
        assert!(
            verify_gcode(&text, &limits_for(tp, &m.post)).is_empty(),
            "{}",
            tp.name
        );
    }
    assert!(collision_check(&job, &tps, &m.post)
        .unwrap()
        .iter()
        .all(|c| c.count == 0));
    // cut the block: nothing below the part
    let sb = job.stock_box.unwrap();
    let mut hm = Heightmap3::new(&sb, 0.1);
    let cutter = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
    for tp in &tps {
        let st = marcam::sim3::simulate3(&mut hm, tp, &cutter, 0.05);
        assert!(st.rapid_hits.is_empty());
    }
    let part = marcam::sim3::part_top(&hm, mesh.as_ref().unwrap());
    let d = marcam::sim3::deviation3(&hm, &part, 0.02);
    assert_eq!(d.gouge_cells, 0, "{d:?}");
    eprintln!("plate: {before} -> {after} lines; {d:?}");
}

#[test]
fn a_shaped_pocket_gets_arcs_only_when_smoothed() {
    let (job, _) = load("examples/plaque/plaque.marcam.toml");
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let arcs = |job: &Job| {
        plan_with_mesh(job, None).ok().map(|tps| {
            tps.iter()
                .map(|tp| {
                    post(tp, &m.post)
                        .lines()
                        .filter(|l| l.starts_with("G2 ") || l.starts_with("G3 "))
                        .count()
                })
                .sum::<usize>()
        })
    };
    // the plaque engraves text and outlines (flat, on a vise): arcs once smoothed
    if let (Some(off), Some(on)) = (
        arcs(&with_smoothing(job.clone(), Some(0.0))),
        arcs(&with_smoothing(job, Some(0.01))),
    ) {
        assert_eq!(off, 0);
        assert!(on > 0, "no arcs");
    }
    // an L-shaped pocket (shaped: offset loops with round corners)
    let pocket = |tol: f64| {
        let project = serde_json::json!({
            "schema_version": "0.2", "name": "p", "models": [],
            "tools": [{"number": 1, "description": "3.175 flat", "kind": "flat", "diameter": 3.175, "flute_length": 12}],
            "setups": [{"id": "s", "machine": "makera-carvera-4axis", "kind": "vise",
                "stock": {"shape": "box", "min": [0.0, 0.0, -10.0], "max": [60.0, 40.0, 0.0]},
                "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false}, "part": [], "spindle_rpm": 12000,
                "operations": [{"id": "op1", "type": "pocket", "tool": 1,
                    "paths": [[[5.0, 5.0], [55.0, 5.0], [55.0, 20.0], [30.0, 20.0], [30.0, 35.0], [5.0, 35.0], [5.0, 5.0]]],
                    "depth": 2.0, "step_down": 1.0, "stepover": 0.5, "feed": 300.0, "plunge_feed": 60.0, "smoothing": tol}],
                "outputs": [{"name": "p", "title": "p", "operations": ["op1"]}]}]
        });
        let libs = marcam::profiles::builtin_libraries();
        let job = marcam::project::jobs_from_any_with(&project.to_string(), &libs, &|_| None)
            .unwrap()
            .remove(0)
            .1;
        let tp = plan_with_mesh(&job, None).unwrap().remove(0);
        let text = post(&tp, &m.post);
        assert!(verify_gcode(&text, &limits_for(&tp, &m.post)).is_empty());
        (
            text.lines().count(),
            text.lines()
                .filter(|l| l.starts_with("G2 ") || l.starts_with("G3 "))
                .count(),
        )
    };
    let ((l0, a0), (l1, a1)) = (pocket(0.0), pocket(0.01));
    assert_eq!(a0, 0);
    assert!(
        a1 > 0 && l1 < l0,
        "{l0} lines, {a0} arcs -> {l1} lines, {a1} arcs"
    );
}

#[test]
fn off_is_the_strategy_as_it_was() {
    // the knob's goldens: smoothing explicitly 0 in the job (and the defaults would change it)
    let (job, _) = load("examples/knob/knob.job.json");
    for p in &job.programs {
        for op in &p.operations {
            assert_eq!(op.smoothing(), 0.0);
        }
    }
    let defaults = with_smoothing(job, None);
    assert!(defaults
        .programs
        .iter()
        .flat_map(|p| &p.operations)
        .all(|op| op.smoothing() > 0.0));
}
