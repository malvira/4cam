//! Tapered ball end mills: the tool's shape, the drop-cutter against it, the scallop
//! stepover checked by cutting two passes into a simulated block, and whole jobs (the plate's
//! dome raster and the knob's rotary finish) simulated without gouges.

use fourcam::dropcutter::{drop_triangle, Cutter};
use fourcam::job::{StockBox, Tool, ToolShape};
use fourcam::sim3::Heightmap3;
use fourcam::{cutter_for, plan_with_mesh, simulate_job, Job};
use std::path::PathBuf;

/// Tip 1 mm, sides 5° from the axis, 3.175 mm across.
fn tb() -> Cutter {
    cutter_for(
        1,
        ToolShape::TaperedBall {
            tip_diameter: 1.0,
            taper_angle: 5.0,
            diameter: 3.175,
        },
    )
    .unwrap()
}

#[test]
fn shape_is_a_ball_blending_into_a_cone() {
    let c = tb();
    let (rb, a) = (0.5f64, 5f64.to_radians());
    let dt = rb * a.cos();
    // the ball near the tip, the cone beyond the tangent point, continuous and smooth there
    assert!((c.height(0.2) - (rb - (rb * rb - 0.04f64).sqrt())).abs() < 1e-12);
    let eps = 1e-7;
    let (h0, h1) = (c.height(dt - eps), c.height(dt + eps));
    assert!((h1 - h0).abs() < 1e-5);
    let slope = |d: f64| (c.height(d + 1e-6) - c.height(d - 1e-6)) / 2e-6;
    assert!((slope(dt - 1e-4) - slope(dt + 1e-4)).abs() < 0.05 * slope(dt + 1e-4));
    assert!((slope(1.2) - 1.0 / a.tan()).abs() < 1e-3);
    assert_eq!(c.radius(), 3.175 / 2.0);
    // no taper: a ball the tip's size
    let ball = cutter_for(
        1,
        ToolShape::TaperedBall {
            tip_diameter: 2.0,
            taper_angle: 0.0,
            diameter: 3.0,
        },
    )
    .unwrap();
    assert_eq!(ball, Cutter::Ball { radius: 1.0 });
    for bad in [
        (0.0, 5.0, 3.0),
        (1.0, -1.0, 3.0),
        (1.0, 90.0, 3.0),
        (2.0, 5.0, 1.0),
    ] {
        let s = ToolShape::TaperedBall {
            tip_diameter: bad.0,
            taper_angle: bad.1,
            diameter: bad.2,
        };
        assert!(cutter_for(1, s).is_err(), "{bad:?}");
    }
}

/// The drop-cutter's answer against a plane equals a brute-force search over the tool's
/// surface: shallow planes touch the ball, steep ones the cone's rim.
#[test]
fn drop_cutter_matches_brute_force_on_planes() {
    let c = tb();
    for deg in [0.0f64, 10.0, 30.0, 60.0, 80.0, 86.0, 88.0] {
        let k = deg.to_radians().tan();
        // a large triangle in the plane z = k x
        let tri = [
            [-50.0, -50.0, -50.0 * k],
            [50.0, -50.0, 50.0 * k],
            [0.0, 60.0, 0.0],
        ];
        let got = drop_triangle(&c, &tri);
        // brute force: the highest the tool can sit is the lowest clearance over its footprint
        let mut want = f64::NEG_INFINITY;
        let n = 4000;
        for i in 0..=n {
            let d = c.radius() * i as f64 / n as f64;
            for x in [d, -d] {
                want = want.max(k * x - c.height(d));
            }
        }
        assert!(
            (got - want).abs() < 1e-4,
            "{deg} deg: drop {got} vs brute force {want}"
        );
    }
}

/// Two passes `scallop_width(h)` apart, cut into a simulated block, leave a cusp `h` high,
/// on the ball and up the cone.
#[test]
fn scallop_stepover_leaves_the_asked_cusp() {
    let c = tb();
    for h in [0.005, 0.02, 0.2] {
        let w = c.scallop_width(h).unwrap();
        let sb = StockBox {
            min: [-5.0, -5.0, -5.0],
            max: [5.0, 5.0, 0.0],
        };
        let mut hm = Heightmap3::new(&sb, 0.002);
        for y in [-w / 2.0, w / 2.0] {
            let mut x = -3.0;
            while x <= 3.0 {
                hm.stamp(&c, x, y, -1.0, 1);
                x += 0.01;
            }
        }
        // the cusp: highest point between the passes, at x = 0
        let mut cusp = f64::NEG_INFINITY;
        let mut y = -w / 2.0;
        while y <= w / 2.0 {
            cusp = cusp.max(hm.at(0.0, y));
            y += 0.001;
        }
        let height = cusp + 1.0;
        assert!(
            (height - h).abs() < 0.003 + 0.02 * h,
            "h {h}: width {w}, cusp {height}"
        );
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(path: &str) -> (Job, PathBuf) {
    let text = std::fs::read_to_string(root().join(path)).unwrap();
    let dir = root().join(path).parent().unwrap().to_path_buf();
    let libs = fourcam::profiles::builtin_libraries();
    let mut jobs =
        fourcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    (jobs.remove(0).1, dir)
}

fn tapered(number: u32) -> Tool {
    Tool {
        flute_length: Some(12.0),
        ..Tool::new(
            number,
            "tapered ball 1 mm tip, 5 deg",
            ToolShape::TaperedBall {
                tip_diameter: 1.0,
                taper_angle: 5.0,
                diameter: 3.175,
            },
        )
    }
}

#[test]
fn dome_raster_with_a_tapered_ball_does_not_gouge() {
    let (mut job, dir) = load("examples/plate/plate.project.json");
    // the dome's raster (T3, a ball) cut with the tapered ball instead
    let t3 = job.tools.iter_mut().find(|t| t.number == 3).unwrap();
    *t3 = tapered(3);
    let mesh = fourcam::load_mesh(&job, &dir).unwrap().unwrap();
    let tps = plan_with_mesh(&job, Some(&mesh)).unwrap();
    let sb = job.stock_box.unwrap();
    let mut hm = Heightmap3::new(&sb, 0.1);
    let cut = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
    for tp in &tps {
        let st = fourcam::sim3::simulate3(&mut hm, tp, &cut, 0.05);
        assert!(
            st.rapid_hits.is_empty(),
            "{}: rapids through stock",
            tp.name
        );
    }
    let part = fourcam::sim3::part_top(&hm, &mesh);
    let dev = fourcam::sim3::deviation3(&hm, &part, 0.02);
    assert!(dev.max_gouge < 0.02, "deepest gouge {}", dev.max_gouge);
    let m = fourcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    for tp in &tps {
        let text = fourcam::post(tp, &m.post);
        assert!(fourcam::verify_gcode(&text, &fourcam::limits_for(tp, &m.post)).is_empty());
    }
    // the GPU has no tapered shape: those programs stay on the CPU
    assert!(fourcam::gpu_input::block(&hm, &tps, &cut, 0.05).is_err());
}

#[test]
fn knob_rotary_finish_with_a_tapered_ball_does_not_gouge() {
    let (mut job, dir) = load("examples/knob/knob_finish.project.json");
    let t3 = job.tools.iter_mut().find(|t| t.number == 3).unwrap();
    *t3 = tapered(3);
    let mesh = fourcam::load_mesh(&job, &dir).unwrap().unwrap();
    let tps = plan_with_mesh(&job, Some(&mesh)).unwrap();
    let res = simulate_job(&job, &tps, Some(&mesh), 0.1, 360, 0.02).unwrap();
    for (name, st, dev) in &res.programs {
        assert!(st.rapid_hits.is_empty(), "{name}: rapids through stock");
        assert!(
            dev.max_gouge < 0.02,
            "{name}: deepest gouge {}",
            dev.max_gouge
        );
    }
}
