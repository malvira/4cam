//! Adaptive clearing: a pocket and a 3D roughing cut at a steady load, the stepover's width
//! of new material along every move at depth (simulated), where a zigzag's first pass at
//! each level is a full-width slot; they come in down a helix or from what is open, clear
//! what they should, gouge nothing and post clean G-code.

use marcam::sim3::{segments3, Heightmap3};
use marcam::{limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Job, Move, Toolpath};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const PICKING: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/picking");

fn plate() -> (Job, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    (jobs.remove(0).1, dir)
}

/// The width (mm) of material each move at depth of operation `kind` cuts: the volume it
/// removes over its length and the level's depth `layer` (simulated on `hm`, the moves before
/// it too), with the move's length.
fn loads(job: &Job, tp: &Toolpath, hm: &mut Heightmap3, kind: &str, layer: f64) -> Vec<(f64, f64)> {
    let cutter = |t: u32| {
        job.tool(t)
            .and_then(|d| marcam::cutter_for(t, d.shape).ok())
    };
    let mut on = vec![false; tp.moves.len()];
    let mut cur = false;
    for (k, m) in tp.moves.iter().enumerate() {
        if let Move::Operation { kind: k2, .. } = m {
            cur = k2 == kind;
        }
        on[k] = cur;
    }
    let cell2 = hm.cell * hm.cell;
    let mut out = Vec::new();
    for seg in segments3(std::slice::from_ref(tp)) {
        let c = seg.tool.and_then(cutter);
        let Some(from) = seg.from else {
            marcam::sim3::apply_segment3(hm, &seg, c.as_ref(), 0.05);
            continue;
        };
        let len = (seg.to[0] - from[0]).hypot(seg.to[1] - from[1]);
        // level moves of the operation (not the helices' descents)
        let measured =
            !seg.rapid && on[seg.move_index] && (seg.to[2] - from[2]).abs() < 1e-9 && len > 0.2;
        // the cells it can reach
        let r = c.map_or(0.0, |c| c.radius()) + hm.cell;
        let (i0, i1) = (
            (((from[0].min(seg.to[0]) - r - hm.x0) / hm.cell)
                .floor()
                .max(0.0) as usize)
                .min(hm.nx),
            (((from[0].max(seg.to[0]) + r - hm.x0) / hm.cell)
                .ceil()
                .max(0.0) as usize)
                .min(hm.nx),
        );
        let (j0, j1) = (
            (((from[1].min(seg.to[1]) - r - hm.y0) / hm.cell)
                .floor()
                .max(0.0) as usize)
                .min(hm.ny),
            (((from[1].max(seg.to[1]) + r - hm.y0) / hm.cell)
                .ceil()
                .max(0.0) as usize)
                .min(hm.ny),
        );
        let sum = |hm: &Heightmap3| -> f64 {
            (j0..j1)
                .map(|j| (i0..i1).map(|i| hm.z[j * hm.nx + i]).sum::<f64>())
                .sum()
        };
        let before = if measured { sum(hm) } else { 0.0 };
        marcam::sim3::apply_segment3(hm, &seg, c.as_ref(), 0.05);
        if measured {
            let w = (before - sum(hm)) * cell2 / (len * layer);
            out.push((w, len));
        }
    }
    out
}

/// The share of the length (0..1) cutting more than `w` mm wide.
fn over(l: &[(f64, f64)], w: f64) -> f64 {
    let all: f64 = l.iter().map(|x| x.1).sum();
    l.iter().filter(|x| x.0 > w).map(|x| x.1).sum::<f64>() / all
}

fn clean(job: &Job, tps: &[Toolpath]) {
    let profile = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    for tp in tps {
        let issues = verify_gcode(&post(tp, &profile.post), &limits_for(tp, &profile.post));
        assert!(
            issues.is_empty(),
            "{}: {:?}",
            tp.name,
            issues.iter().map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }
    let _ = job;
}

#[test]
fn an_adaptive_pocket_cuts_at_a_steady_load_from_a_helix() {
    // the plate's 20 x 30 pocket, 3 deep, with the 3.175 mm flat: 15% of it (0.48 mm) at
    // 1.5 mm a level
    let (mut job, dir) = plate();
    let mut zig = job.clone();
    for (j, adaptive) in [(&mut job, true), (&mut zig, false)] {
        for p in &mut j.programs {
            for o in &mut p.operations {
                if let marcam::job::Operation::Pocket(pk) = o {
                    pk.adaptive = adaptive;
                    pk.stepover = if adaptive { 0.15 } else { 0.5 };
                    pk.step_down = 1.5;
                    pk.ramp = 3.0;
                }
            }
        }
    }
    let mesh = load_mesh(&job, &dir).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let tp = tps
        .iter()
        .find(|t| {
            t.moves
                .iter()
                .any(|m| matches!(m, Move::Operation { kind, .. } if kind == "pocket"))
        })
        .unwrap();
    assert!(
        tp.notes.iter().all(|w| !w.contains("plunge")),
        "{:?}",
        tp.notes
    );
    // inside the pocket (the tool's radius in from its walls), down to its depth
    let r = 3.175 / 2.0;
    let mut on = false;
    let mut steepest: f64 = 0.0;
    let mut at: Option<[f64; 3]> = None;
    for m in &tp.moves {
        if let Move::Operation { kind, .. } = m {
            on = kind == "pocket";
        }
        if let (true, Move::Line3 { x, y, z, .. }) = (on, m) {
            assert!(
                *x >= 5.0 + r - 1e-3
                    && *x <= 25.0 - r + 1e-3
                    && *y >= 5.0 + r - 1e-3
                    && *y <= 35.0 - r + 1e-3,
                "outside the pocket at ({x:.3}, {y:.3})"
            );
            assert!(*z >= -3.5 - 1e-9, "below the pocket at {z}");
            if let Some(a) = at {
                let (dz, dxy) = (a[2] - z, (x - a[0]).hypot(y - a[1]));
                if dz > 1e-6 && dxy > 1e-6 {
                    steepest = steepest.max(dz / dxy);
                }
                // (no move straight down into the material: the helices ramp at 3 degrees)
            }
        }
        if let Move::Line3 { x, y, z, .. } = m {
            at = Some([*x, *y, *z]);
        }
        if let Move::Rapid3 { x, y, z } = m {
            at = at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])]);
        }
    }
    assert!(
        steepest < 3.5f64.to_radians().tan(),
        "descends at {:.1} deg",
        steepest.atan().to_degrees()
    );
    // the load: the stepover's width, not the tool's (a zigzag's first pass is a slot)
    let sb = job.stock_box.unwrap();
    let mut hm = Heightmap3::new(&sb, 0.05);
    let l = loads(&job, tp, &mut hm, "pocket", 1.5);
    let ae = 0.15 * 3.175;
    assert!(
        over(&l, 2.0 * ae) < 0.03,
        "{:.1}% of it over twice the stepover",
        100.0 * over(&l, 2.0 * ae)
    );
    let mut hz = Heightmap3::new(&sb, 0.05);
    let ztps = plan_with_mesh(&zig, mesh.as_ref()).unwrap();
    let ztp = ztps
        .iter()
        .find(|t| {
            t.moves
                .iter()
                .any(|m| matches!(m, Move::Operation { kind, .. } if kind == "pocket"))
        })
        .unwrap();
    let lz = loads(&zig, ztp, &mut hz, "pocket", 1.5);
    assert!(
        over(&lz, 2.0 * ae) > 0.3,
        "the zigzag: {:.1}%",
        100.0 * over(&lz, 2.0 * ae)
    );
    // cleared to the floor everywhere but the corners' fillets
    let mut worst = (f64::NEG_INFINITY, 0.0, 0.0);
    for j in 0..hm.ny {
        for i in 0..hm.nx {
            let (x, y) = hm.centre(i, j);
            let inside = x > 5.1 && x < 24.9 && y > 5.1 && y < 34.9;
            let corner = (x < 5.0 + r || x > 25.0 - r) && (y < 5.0 + r || y > 35.0 - r);
            if inside && !corner && hm.z[j * hm.nx + i] > worst.0 {
                worst = (hm.z[j * hm.nx + i], x, y);
            }
        }
    }
    assert!(worst.0 < -3.5 + 0.02, "left at {worst:?}");
    clean(&job, &tps);
}

/// The oval grip block (a textured grip between two recessed ovals), roughed with the 5 mm
/// flat (T1).
fn grip(pattern: &str) -> Value {
    let mut doc: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(PICKING).join("oval_grip.project.json")).unwrap(),
    )
    .unwrap();
    let (stepover, step_down) = if pattern == "adaptive" {
        (0.8, 3.0)
    } else {
        (1.2, 1.5)
    };
    doc["setups"][0]["operations"] = json!([{"id": "op1", "type": "rough3d", "tool": 1, "x_range": [-5.0, 65.0], "y_range": [-5.0, 35.0],
        "stepover": stepover, "sample": 0.4, "step_down": step_down, "floor": -8.5, "leave": 0.25, "feed": 600.0, "plunge_feed": 100.0,
        "pattern": pattern, "ramp_angle": 3.0}]);
    doc["setups"][0]["outputs"] = json!([{"name": "grip", "title": "grip", "operations": ["op1"]}]);
    doc
}

fn grip_job(doc: &Value) -> Job {
    let libs = marcam::profiles::builtin_libraries();
    let dir = Path::new(PICKING).to_path_buf();
    let meshes = |src: &marcam::job::MeshSource| src.load(&dir).ok();
    marcam::project::jobs_from_any_with(&doc.to_string(), &libs, &meshes)
        .unwrap()
        .remove(0)
        .1
}

#[test]
fn adaptive_roughing_clears_down_to_the_part_at_a_steady_load() {
    let doc = grip("adaptive");
    let job = grip_job(&doc);
    let mesh = load_mesh(&job, Path::new(PICKING)).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let tp = &tps[0];
    // not into the part
    let g = marcam::gouge_check(&job, &tps, mesh.as_ref()).unwrap();
    assert!(g.iter().all(|r| r.max_depth < 0.02), "{g:?}");
    clean(&job, &tps);
    // at a steady load
    let sb = job.stock_box.unwrap();
    let mut hm = Heightmap3::new(&sb, 0.1);
    let l = loads(&job, tp, &mut hm, "rough3d", 3.0);
    let ae = 0.8;
    assert!(
        over(&l, 2.0 * ae) < 0.05,
        "{:.1}% of it over twice the stepover",
        100.0 * over(&l, 2.0 * ae)
    );
    // down to the part: at most a level's terrace (and what is left for the finish) above it
    let part = marcam::sim3::part_top(&hm, mesh.as_ref().unwrap());
    let dev = marcam::sim3::deviation3(&hm, &part, 0.02);
    assert!(dev.max_gouge < 0.02, "{dev:?}");
    assert!(dev.mean_left < 0.25 + 1.0, "{dev:?}");
}
