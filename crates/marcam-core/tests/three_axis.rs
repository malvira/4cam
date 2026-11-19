//! 3-axis operations on the plate example (Carvera, vise, box stock): plan, post, verify,
//! then cut the block as a Z heightmap and check what is left.

use marcam::dropcutter::Cutter;
use marcam::job::StockBox;
use marcam::sim3::Heightmap3;
use marcam::{cutter_for, limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Job};
use std::path::PathBuf;

fn plate() -> (Job, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    assert_eq!(jobs.len(), 1);
    (jobs.remove(0).1, dir)
}

/// The dome fixture: sphere radius 6.5 centred at (46, 20, -7), cut off at Z -4.5.
fn dome_z(x: f64, y: f64) -> Option<f64> {
    let r = (x - 46.0).hypot(y - 20.0);
    (r < 6.0).then(|| -7.0 + (6.5f64 * 6.5 - r * r).sqrt())
}

fn simulate() -> (Heightmap3, Vec<Vec<usize>>) {
    let (job, dir) = plate();
    let mesh = load_mesh(&job, &dir).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let stock: StockBox = job.stock_box.expect("box stock");
    let mut hm = Heightmap3::new(&stock, 0.25);
    let cutter = |n: u32| -> Option<Cutter> {
        let t = job.tools.iter().find(|t| t.number == n)?;
        cutter_for(n, t.shape).ok()
    };
    let hits = tps
        .iter()
        .map(|tp| {
            let h = hm.cut(tp, &cutter, 0.1);
            for &i in h.iter().take(3) {
                eprintln!(
                    "{}: {:?} after {:?}",
                    tp.name,
                    tp.moves[i],
                    &tp.moves[i.saturating_sub(3)..i]
                );
            }
            h
        })
        .collect();
    (hm, hits)
}

#[test]
fn plate_programs_post_and_verify_clean() {
    let (job, dir) = plate();
    let mesh = load_mesh(&job, &dir).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    assert_eq!(tps.len(), 2);
    let profile = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    // a vise job: no rotary envelope or A-axis complaints, and it fits the travels
    let limits = marcam::profiles::check_limits(&job, &tps, &profile);
    assert!(limits.is_empty(), "{limits:?}");
    let machine = profile.post;
    for tp in &tps {
        let text = post(tp, &machine);
        assert!(text.contains("(Zero: the setup's work coordinates, Z up"));
        assert!(!text.contains(" A"), "a 3-axis program must not move A");
        let limits = limits_for(tp, &machine);
        assert_eq!(limits.plunge_feed, 60.0);
        let issues = verify_gcode(&text, &limits);
        assert!(
            issues.is_empty(),
            "{}: {:?}",
            tp.name,
            &issues[..issues.len().min(5)]
        );
    }
}

#[test]
fn plate_cuts_what_each_operation_promises() {
    let (hm, hits) = simulate();
    assert!(
        hits.iter().all(|h| h.is_empty()),
        "rapids through material: {hits:?}"
    );
    let near = |x: f64, y: f64, z: f64, tol: f64, what: &str| {
        let got = hm.at(x, y);
        assert!(
            (got - z).abs() <= tol,
            "{what} at ({x}, {y}): Z{got:.3}, expected Z{z:.3}"
        );
    };
    // facing: the whole top comes down 0.5
    near(32.0, 20.0, -0.5, 1e-6, "faced top");
    near(57.0, 37.0, -0.5, 1e-6, "faced corner");
    // pocket [5, 5, 25, 35], 3 deep below the faced top, walls on the rectangle
    near(15.0, 20.0, -3.5, 1e-6, "pocket floor");
    near(5.125, 20.0, -3.5, 1e-6, "pocket floor at the wall");
    near(4.875, 20.0, -0.5, 1e-6, "material outside the pocket wall");
    near(25.125, 20.0, -0.5, 1e-6, "material outside the pocket wall");
    // holes: 4 deep, as wide as the 3.175 mm endmill
    for (x, y) in [(40.0, 6.0), (54.0, 6.0), (40.0, 34.0), (54.0, 34.0)] {
        near(x, y, -4.5, 1e-6, "hole bottom");
        near(x + 1.4, y, -4.5, 1e-6, "hole bottom near the wall");
        near(x + 1.9, y, -0.5, 1e-6, "material beside the hole");
    }
    // bore: a 6 mm hole through at (30, 31), bored with the 3.175 mm endmill
    near(
        30.0,
        31.0,
        -10.5,
        1e-6,
        "bore through the block (0.5 past the bottom)",
    );
    near(32.9, 31.0, -10.5, 1e-6, "bore near its wall");
    near(33.2, 31.0, -0.5, 1e-6, "material beside the bore");
    // outline: 2 deep outside the 56 x 36 rectangle, nothing inside it
    near(1.0, 20.0, -2.5, 1e-6, "outside the outline");
    near(30.0, 0.5, -2.5, 1e-6, "outside the outline");
    near(2.375, 20.0, -0.5, 1e-6, "inside the outline");
    // dome: never below the surface, within a small scallop above it where the ball reaches;
    // floor around it
    let mut worst = (0.0f64, f64::INFINITY);
    for j in 0..hm.ny {
        for i in 0..hm.nx {
            let (x, y) = hm.centre(i, j);
            if let Some(z) = dome_z(x, y) {
                let d = hm.at(x, y) - z;
                worst.1 = worst.1.min(d);
                // the ball leaves a fillet where the dome meets the floor (contact at r 5.66)
                if (x - 46.0).hypot(y - 20.0) < 5.5 {
                    worst.0 = worst.0.max(d);
                }
            }
        }
    }
    assert!(worst.1 > -0.02, "dome gouged by {:.3}", -worst.1);
    assert!(worst.0 < 0.25, "dome left {:.3} high", worst.0);
    near(38.0, 12.0, -4.5, 0.01, "raster floor beside the dome");
    near(46.0, 20.0, -0.5, 0.02, "dome top");
    // the engine's own comparison with the part mesh agrees: no gouged cells over the dome
    let (job, dir) = plate();
    let mesh = load_mesh(&job, &dir).unwrap().unwrap();
    let part = marcam::sim3::part_top(&hm, &mesh);
    let d = marcam::sim3::deviation3(&hm, &part, 0.02);
    assert!(d.cells > 1500 && d.gouge_cells == 0, "{d:?}");
}
