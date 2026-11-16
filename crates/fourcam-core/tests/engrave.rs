//! Engraving and V-carving, flat on a vise and wrapped round the rotary axis: depth and
//! width checked in the simulated stock.

use fourcam::job::Job;
use fourcam::sim3::{simulate3, Heightmap3};
use fourcam::{cutter_for, limits_for, plan, post, simulate_job, verify_gcode};
use serde_json::{json, Value};

fn project(stock: Value, ops: Value, machine: &str) -> Job {
    let n = ops.as_array().unwrap().len();
    let ids: Vec<String> = (0..n).map(|k| format!("op{}", k + 1)).collect();
    let mut ops = ops;
    for (k, o) in ops.as_array_mut().unwrap().iter_mut().enumerate() {
        o["id"] = json!(ids[k]);
    }
    let doc = json!({
        "schema_version": "0.2", "name": "engrave test",
        "models": [{"id": "bar", "kind": "profile", "points": [[-30.0, 12.0], [0.0, 12.0]]}],
        "tools": [
            {"number": 2, "description": "V-bit 90", "kind": "vbit", "angle": 90.0, "diameter": 2.4},
            {"number": 3, "description": "ball 1", "kind": "ball", "diameter": 1.0, "flute_length": 4.0}],
        "setups": [{"id": "s", "name": "s", "machine": machine, "stock": stock, "clearance": 3.0,
                    "part": ["bar"], "spindle_rpm": 12000, "operations": ops,
                    "outputs": [{"name": "out", "title": "out", "operations": ids}]}]
    });
    let libs = fourcam::profiles::builtin_libraries();
    let mut jobs =
        fourcam::project::jobs_from_any_with(&doc.to_string(), &libs, &|_| None).unwrap();
    jobs.remove(0).1
}

fn clean(job: &Job, machine: &str) -> Vec<fourcam::Toolpath> {
    let tps = plan(job).unwrap();
    let m = fourcam::profiles::builtin_machine(machine).unwrap().post;
    for tp in &tps {
        let text = post(tp, &m);
        let issues = verify_gcode(&text, &limits_for(tp, &m));
        assert!(issues.is_empty(), "{:?}", &issues[..issues.len().min(3)]);
    }
    tps
}

fn vise(ops: Value) -> (Job, Heightmap3) {
    let job = project(
        json!({"shape": "box", "min": [0.0, 0.0, -5.0], "max": [30.0, 20.0, 0.0]}),
        ops,
        "makera-carvera-4axis",
    );
    let tps = clean(&job, "makera-carvera-4axis");
    let mut hm = Heightmap3::new(job.stock_box.as_ref().unwrap(), 0.05);
    let cutter = |n: u32| cutter_for(n, job.tool(n)?.shape).ok();
    let st = simulate3(&mut hm, &tps[0], &cutter, 0.02);
    assert!(st.rapid_hits.is_empty());
    (job, hm)
}

#[test]
fn engraved_line_is_a_v_groove_of_the_right_depth_and_width_in_two_passes() {
    let (job, hm) = vise(
        json!([{"type": "engrave", "tool": 2, "paths": [[[2.0, 5.0], [12.0, 5.0]]],
        "depth": 0.4, "step_down": 0.2, "feed": 300.0, "plunge_feed": 60.0}]),
    );
    // V-bit 90: the groove is 0.8 wide at the top and its walls slope at 45 degrees
    for (dy, want) in [
        (0.0, -0.4),
        (0.15, -0.25),
        (0.3, -0.1),
        (0.45, 0.0),
        (-0.3, -0.1),
    ] {
        let z = hm.at(7.0, 5.0 + dy + 1e-9);
        assert!((z - want).abs() < 0.03, "at y {dy}: Z{z:.3}, want {want}");
    }
    // the ends stop at the path's ends (plus the cone's reach at depth)
    assert!(hm.at(1.5, 5.0) > -1e-9 && hm.at(12.6, 5.0) > -1e-9);
    let tps = plan(&job).unwrap();
    let mut depths: Vec<i64> = tps[0]
        .moves
        .iter()
        .filter_map(|m| match *m {
            fourcam::Move::Line3 { z, .. } => Some((z * 1000.0).round() as i64),
            _ => None,
        })
        .collect();
    depths.sort();
    depths.dedup();
    assert_eq!(depths, vec![-400, -200], "two passes");
}

#[test]
fn engraved_text_lands_where_its_box_says() {
    let (_, hm) = vise(
        json!([{"type": "engrave", "tool": 3, "text": "4CAM", "text_height": 6.0,
        "text_at": [5.0, 7.0], "depth": 0.2, "step_down": 0.2, "feed": 300.0, "plunge_feed": 60.0}]),
    );
    // cells cut deeper than 0.1: their box is the text's (19 mm x 6 mm) widened by the ball's
    // width at that depth (0.45 mm each side)
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for j in 0..hm.ny {
        for i in 0..hm.nx {
            if hm.z[j * hm.nx + i] < -0.1 {
                let (x, y) = hm.centre(i, j);
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
    }
    let w = (0.5f64.powi(2) - 0.4f64.powi(2)).sqrt();
    for (got, want) in [
        (lo[0], 5.0 - w),
        (hi[0], 24.0 + w),
        (lo[1], 7.0 - w),
        (hi[1], 13.0 + w),
    ] {
        assert!((got - want).abs() < 0.08, "{lo:?} {hi:?} (want {want})");
    }
    // the middle bar of the A is cut, the inside of the O-less C is not
    assert!(hm.at(5.0 + 10.0 + 2.0, 7.0 + 2.0) < -0.15);
    assert!(hm.at(5.0 + 5.0 + 2.0, 7.0 + 3.0) > -1e-9);
}

#[test]
fn v_carve_depth_follows_the_distance_to_the_outline() {
    // a 10 x 2 bar: centre 1 deep (half width / tan 45), walls through the outline
    let (_, hm) = vise(json!([{"type": "vcarve", "tool": 2,
        "paths": [[[2.0, 4.0], [12.0, 4.0], [12.0, 6.0], [2.0, 6.0], [2.0, 4.0]]],
        "max_depth": 3.0, "stepover": 0.05, "sample": 0.05, "feed": 300.0, "plunge_feed": 60.0}]));
    for (y, want) in [
        (5.0, -1.0),
        (4.5, -0.5),
        (5.75, -0.25),
        (3.9, 0.0),
        (6.1, 0.0),
    ] {
        let z = hm.at(7.0, y + 1e-9);
        assert!((z - want).abs() < 0.05, "at y {y}: Z{z:.3}, want {want}");
    }
    // a shallow limit makes a flat floor
    let (_, hm) = vise(json!([{"type": "vcarve", "tool": 2,
        "paths": [[[2.0, 4.0], [12.0, 4.0], [12.0, 6.0], [2.0, 6.0], [2.0, 4.0]]],
        "max_depth": 0.6, "stepover": 0.05, "sample": 0.05, "feed": 300.0, "plunge_feed": 60.0}]));
    assert!((hm.at(7.0, 5.0) + 0.6).abs() < 0.03 && (hm.at(7.0, 4.3) + 0.3).abs() < 0.05);
    assert!(hm.at(7.0, 3.95) > -1e-9, "nothing cut outside the outline");
}

fn rotary(ops: Value) -> fourcam::JobSim {
    let job = project(
        json!({"shape": "cylinder", "radius": 12.0, "length": 40.0}),
        ops,
        "makera-z1-4axis",
    );
    let tps = clean(&job, "makera-z1-4axis");
    // the grid as the batch simulation builds it, then cut with samples 0.01 mm apart: a
    // sharp V tip moving round the bar must pass over every 0.25 degree cell centre
    let mut res = simulate_job(&job, &[], None, 0.05, 1440, 0.02).unwrap();
    let cutter = |n: u32| cutter_for(n, job.tool(n)?.shape).ok();
    for tp in &tps {
        let st = fourcam::sim::simulate(&mut res.stock, tp, &cutter, 0.01);
        assert!(st.rapid_hits.is_empty());
    }
    res
}

fn r_at(res: &fourcam::JobSim, x: f64, phi_deg: f64) -> f64 {
    let g = res.stock.grid;
    let i = ((x - g.x0) / g.dx) as usize;
    let j = (phi_deg.rem_euclid(360.0) / 360.0 * g.na as f64) as usize % g.na;
    res.stock.r[i * g.na + j] as f64
}

#[test]
fn wrapped_engraving_runs_round_the_bar() {
    // a path along Y (arc length) at X -9.975 (a cell centre of the 0.05 mm grid): round
    // the bar, centred on the top at A0
    let res = rotary(json!([{"type": "engrave", "tool": 2,
        "paths": [[[-9.975, -5.0], [-9.975, 5.0]]],
        "depth": 0.3, "step_down": 0.3, "feed": 300.0, "plunge_feed": 60.0}]));
    let arc = (4.0f64 / 12.0).to_degrees(); // 4 mm of arc on R12
    for phi in [90.0, 90.0 - arc, 90.0 + arc] {
        let r = r_at(&res, -9.975, phi);
        assert!((r - 11.7).abs() < 0.005, "phi {phi}: {r}");
    }
    assert!((r_at(&res, -9.975, 270.0) - 12.0).abs() < 1e-6);
    // V-groove walls at 45 degrees: 0.2 to the side it is 0.1 deep
    let r = r_at(&res, -10.175, 90.0);
    assert!((r - 11.9).abs() < 0.005, "{r}");
}

#[test]
fn wrapped_text_and_v_carve() {
    let res = rotary(json!([
        {"type": "vcarve", "tool": 2,
         "paths": [[[-25.0, -1.0], [-15.0, -1.0], [-15.0, 1.0], [-25.0, 1.0], [-25.0, -1.0]]],
         "max_depth": 2.0, "stepover": 0.05, "sample": 0.05, "feed": 300.0, "plunge_feed": 60.0},
        {"type": "engrave", "tool": 2, "text": "HI", "text_height": 4.0, "text_at": [-7.975, -2.0],
         "text_rotate": 0.0, "depth": 0.2, "step_down": 0.2, "feed": 300.0, "plunge_feed": 60.0}]));
    // V-carve: 1 deep along the middle of the 2 mm band, nothing past the outline
    let r = r_at(&res, -20.0, 90.0);
    assert!((r - 11.0).abs() < 0.04, "{r}");
    let edge = (1.1f64 / 12.0).to_degrees();
    assert!((r_at(&res, -20.0, 90.0 + edge) - 12.0).abs() < 1e-6);
    // the H's left stroke at X -7.975 from arc -2 to +2
    let r = r_at(&res, -7.975, 90.0);
    assert!((r - 11.8).abs() < 0.005, "{r}");
    // inside the H, above its crossbar (arc +1, i.e. 4.8 degrees before the top)
    let above = 90.0 - (1.0f64 / 12.0).to_degrees();
    let r = r_at(&res, -6.64, above);
    assert!((r - 12.0).abs() < 1e-6, "{r}");
}

fn example(name: &str) -> (Job, std::path::PathBuf) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    let text = std::fs::read_to_string(dir.join(format!("{name}.project.json"))).unwrap();
    let libs = fourcam::profiles::builtin_libraries();
    let mut jobs =
        fourcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    (jobs.remove(0).1, dir)
}

#[test]
fn plaque_title_and_star_on_the_vise() {
    let (job, dir) = example("plaque");
    let mesh = fourcam::load_mesh(&job, &dir).unwrap();
    let tps = fourcam::plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let carvera = fourcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let text = post(&tps[2], &carvera.post);
    assert!(verify_gcode(&text, &limits_for(&tps[2], &carvera.post)).is_empty());
    // the border strip above the relief, at 0.05 mm cells
    let strip = fourcam::job::StockBox {
        min: [0.0, 65.0, -15.0],
        max: [90.0, 90.0, 0.0],
    };
    let mut hm = Heightmap3::new(&strip, 0.05);
    let cutter = |n: u32| cutter_for(n, job.tool(n)?.shape).ok();
    let st = simulate3(&mut hm, &tps[2], &cutter, 0.02);
    assert!(st.rapid_hits.is_empty());
    // star: wide in the middle, so it has a flat floor at the 1.5 mm limit, with ridges
    // between the 0.1 mm rows of at most 0.05 (half the stepover / tan 45); nothing past
    // its outer points
    let z = hm.at(78.0, 78.0);
    assert!((-1.5 - 1e-6..=-1.45).contains(&z), "{z}");
    assert!(hm.at(78.0, 86.2) > -1e-9 && hm.at(70.0, 78.0) > -1e-9);
    // the L of RELIEF (third letter): its upright at X 8 + 2 x 5 x 5/6, 0.4 deep
    let x = 8.0 + 2.0 * 5.0 * 5.0 / 6.0;
    let deepest = (0..8)
        .map(|k| hm.at(x - 0.075 + 0.025 * k as f64, 74.0))
        .fold(f64::INFINITY, f64::min);
    assert!((deepest + 0.4).abs() < 0.03, "{deepest}");
}

#[test]
fn ring_grooves_text_and_diamonds_wrapped_on_the_z1() {
    let (job, _) = example("ring");
    let tps = clean(&job, "makera-z1-4axis");
    let z1 = fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    let errors: Vec<_> = fourcam::profiles::check_limits(&job, &tps, &z1)
        .into_iter()
        .filter(|i| i.severity == "error")
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let res = simulate_job(&job, &tps, None, 0.1, 720, 0.02).unwrap();
    assert!(res.programs[0].1.rapid_hits.is_empty());
    // grooves all the way round at X -40 and -5
    for phi in [0.0, 90.0, 180.0, 270.0, 45.0] {
        for x in [-40.0, -5.0] {
            let r = r_at(&res, x, phi);
            // 0.25 deep at the cell (0.05 off the line), up to 0.025 less where the batch
            // simulation's 0.1 mm samples pass a cell beside the sharp tip
            assert!(
                (11.75 - 1e-4..11.78).contains(&r),
                "groove at X{x}, phi {phi}: {r}"
            );
        }
    }
    // diamond 5 of 9 is centred on the top: flat floor at the 1.4 limit (the cell is 0.05
    // of arc beside a row, so it sees up to the 0.05 ridge between rows); midway to the
    // next diamond the bar is untouched
    let r = r_at(&res, -31.0, 90.0);
    assert!((10.6 - 1e-4..10.655).contains(&r), "{r}");
    let gap = 90.0 - (24.0 * std::f64::consts::PI / 9.0 / 2.0 / 12.0).to_degrees();
    assert!(
        (r_at(&res, -31.0, gap) - 12.0).abs() < 1e-6,
        "{}",
        r_at(&res, -31.0, gap)
    );
    // the text runs round the bar between X -20 and -15: the R at the top is cut
    let cut = (0..50)
        .map(|k| r_at(&res, -19.95 + 0.1 * k as f64, 90.0))
        .fold(f64::INFINITY, f64::min);
    assert!(cut < 11.76, "{cut}");
}
