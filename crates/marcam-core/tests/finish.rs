//! Mesh finishing (M1): drop-cutter accuracy against the analytic knob profile, gouge checks on
//! generated toolpaths, mesh-file pipeline, and the LinuxCNC (G93) post.

use marcam::dropcutter::{Cutter, DropCutter};
use marcam::geom::RevolvedProfile;
use marcam::job::Profile;
use marcam::{limits_for, load_mesh, plan, plan_with_mesh, post, verify_gcode, Job, Machine};
use marcam::{Mesh, Move};
use std::f64::consts::PI;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn knob_job() -> Job {
    let text = std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap();
    Job::from_json(&text).unwrap()
}

fn knob_profile() -> Profile {
    knob_job().part.profile.unwrap()
}

/// Exact (non-mesh) tip radius for a radial tool over the revolved knob. For a surface of
/// revolution the highest point under any lateral offset is on the centre line, so this is a
/// 1D maximum over the axial footprint (sampled finely, plus every profile break point).
fn analytic_cl(geom: &RevolvedProfile, prof: &Profile, cutter: &Cutter, x: f64) -> f64 {
    let (x_min, x_max) = (geom.x_min(), geom.x_max());
    let rho = |u: f64| {
        if u < x_min || u > x_max {
            0.0
        } else {
            geom.radius(u)
        }
    };
    let r = cutter.radius();
    let n = (2.0 * r / 0.0005).ceil() as usize;
    let mut best = 0.0f64;
    let mut eval = |dx: f64| {
        if dx.abs() <= r {
            best = best.max(rho(x + dx) - cutter.height(dx.abs()));
        }
    };
    for i in 0..=n {
        eval(-r + 2.0 * r * i as f64 / n as f64);
    }
    for p in &prof.points {
        eval(p[0] - x);
    }
    best
}

fn cutters() -> Vec<Cutter> {
    vec![
        Cutter::Flat { radius: 1.5875 },
        Cutter::Ball { radius: 1.5875 },
        Cutter::Cone {
            radius: 3.0,
            half_angle: PI / 4.0,
        },
    ]
}

#[test]
fn drop_cutter_matches_analytic_profile() {
    let prof = knob_profile();
    let geom = RevolvedProfile::new(&prof, 12.5);
    let mesh = Mesh::revolve(&prof, 720);
    for cutter in cutters() {
        let dc = DropCutter::new(&mesh, cutter);
        let mut worst = 0.0f64;
        let mut x = -24.0;
        while x < 2.0 {
            let want = analytic_cl(&geom, &prof, &cutter, x);
            for a in [0.0, 37.3, 200.0, 333.0] {
                let got = dc.radius_at(x, a, 0.0);
                worst = worst.max((got - want).abs());
                assert!(
                    (got - want).abs() < 2e-3,
                    "{cutter:?} x={x:.3} a={a}: mesh {got:.5} analytic {want:.5}"
                );
            }
            x += 0.0731;
        }
        eprintln!("{cutter:?}: worst |mesh - analytic| = {worst:.2e} mm");
    }
}

fn finish_job(pattern: &str) -> Job {
    let mut job = knob_job();
    let op = match pattern {
        "along" => {
            r#"{"type": "rotary_finish", "tool": 9, "pattern": "along",
            "x_range": [-21.0, 0.6], "stepover_deg": 15.0, "sample_mm": 0.2,
            "feed": 400.0, "plunge_feed": 60.0}"#
        }
        _ => {
            r#"{"type": "rotary_finish", "tool": 9, "pattern": "around",
            "x_range": [0.5, -20.0], "stepover_mm": 1.0, "sample_deg": 6.0,
            "feed": 300.0, "plunge_feed": 60.0, "smoothing": 0.0}"#
        }
    };
    let extra: serde_json::Value = serde_json::from_str(&format!(
        r#"{{
        "tools": [{{"number": 9, "description": "3.175 mm ball", "kind": "ball", "diameter": 3.175, "flute_length": 12}}],
        "programs": [{{"name": "finish_{pattern}", "title": "finish {pattern}", "operations": [{op}]}}]
    }}"#
    ))
    .unwrap();
    job.tools = serde_json::from_value(extra["tools"].clone()).unwrap();
    job.programs = serde_json::from_value(extra["programs"].clone()).unwrap();
    job
}

/// Every point along every cut stays on or above the true part (within `tol`), and cut
/// end points sit on it (the path really follows the surface).
fn check_no_gouge(job: &Job, tp: &marcam::Toolpath, cutter: &Cutter) {
    let prof = job.part.profile.as_ref().unwrap();
    let geom = RevolvedProfile::new(prof, job.stock.radius);
    let (mut x, mut r, mut a) = (None::<f64>, None::<f64>, 0.0);
    let (mut worst_below, mut worst_above, mut cuts) = (0.0f64, 0.0f64, 0);
    for m in &tp.moves {
        match *m {
            Move::Rapid {
                x: nx,
                r: nr,
                a: na,
            } => {
                x = nx.or(x);
                r = nr.or(r);
                a = na.unwrap_or(a);
            }
            Move::Plunge { r: nr, .. } => r = Some(nr),
            Move::Cut {
                x: nx,
                r: nr,
                a: na,
                ..
            } => {
                let (x0, r0) = (x.unwrap(), r.unwrap());
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    let (xi, ri) = (x0 + (nx - x0) * t, r0 + (nr - r0) * t);
                    let need = analytic_cl(&geom, prof, cutter, xi);
                    worst_below = worst_below.max(need - ri);
                }
                // end points must sit on the surface, except the deliberate climb/descend
                // corners right at a wall (where the surface jumps)
                let here = analytic_cl(&geom, prof, cutter, nx);
                let near = |d: f64| (analytic_cl(&geom, prof, cutter, nx + d) - here).abs();
                if near(-0.002).max(near(0.002)) < 0.01 {
                    worst_above = worst_above.max(nr - here);
                }
                cuts += 1;
                x = Some(nx);
                r = Some(nr);
                a = na;
            }
            _ => {}
        }
    }
    let _ = a;
    eprintln!(
        "{}: {cuts} cuts, deepest dip below part {worst_below:.4} mm, highest end point above it {worst_above:.4} mm (away from walls)",
        tp.name
    );
    assert!(cuts > 100);
    assert!(worst_below < 0.01, "{}: gouges {worst_below} mm", tp.name);
    assert!(worst_above < 0.005, "{}: floats {worst_above} mm", tp.name);
}

#[test]
fn finish_toolpaths_follow_the_part_without_gouging() {
    let ball = Cutter::Ball { radius: 1.5875 };
    for pattern in ["along", "around"] {
        let job = finish_job(pattern);
        let tps = plan(&job).unwrap();
        assert_eq!(tps.len(), 1);
        check_no_gouge(&job, &tps[0], &ball);
        for machine in [Machine::makera_z1(), Machine::linuxcnc()] {
            let g = post(&tps[0], &machine);
            let issues = verify_gcode(&g, &limits_for(&tps[0], &machine));
            assert!(
                issues.is_empty(),
                "{} {}: {issues:?}",
                pattern,
                machine.name
            );
        }
    }
}

#[test]
fn along_rasters_hold_a_per_pass_and_around_is_one_helix() {
    // the strategy's own points (smoothing off): every pass at one A
    let mut job = finish_job("along");
    for p in &mut job.programs {
        for op in &mut p.operations {
            if let marcam::job::Operation::RotaryFinish(o) = op {
                o.smoothing = Some(0.0);
            }
        }
    }
    let tps = plan(&job).unwrap();
    let cuts: Vec<(f64, f64)> = tps[0]
        .moves
        .iter()
        .filter_map(|m| match m {
            Move::Cut { x, a, .. } => Some((*x, *a)),
            _ => None,
        })
        .collect();
    let mut angles: Vec<f64> = cuts.iter().map(|c| c.1).collect();
    angles.dedup();
    // 24 passes (15 deg) plus link points between them
    let passes = (0..24).map(|i| i as f64 * 15.0).collect::<Vec<_>>();
    for p in &passes {
        assert!(cuts.iter().filter(|c| (c.1 - p).abs() < 1e-9).count() > 100);
    }
    let tps = plan(&finish_job("around")).unwrap();
    let a: Vec<f64> = tps[0]
        .moves
        .iter()
        .filter_map(|m| match m {
            Move::Cut { a, .. } => Some(*a),
            _ => None,
        })
        .collect();
    assert!(
        a.windows(2).all(|w| w[1] > w[0]),
        "A must keep turning one way"
    );
    // 20.5 mm at 1 mm per turn, one turn at each end, and one clean-up turn at the foot of
    // each of the two walls (x = -18 and -15, approached from the -x side)
    let turns = (a.last().unwrap() - a[0]) / 360.0;
    assert!((turns - 24.5).abs() < 0.05, "{turns}");
    let feet: Vec<f64> = tps[0]
        .moves
        .windows(2)
        .filter_map(|w| match (&w[0], &w[1]) {
            (Move::Rapid { x: Some(x), .. }, Move::Plunge { .. }) => Some(*x),
            _ => None,
        })
        .skip(1)
        .collect();
    assert_eq!(feet.len(), 2, "{feet:?}");
    // ball tool: lowest point clears the wall corner when the tip is a radius away
    assert!((feet[0] - (-18.0 - 1.5875)).abs() < 1e-3, "{feet:?}");
    assert!((feet[1] - (-15.0 - 1.5875)).abs() < 1e-3, "{feet:?}");
}

#[test]
fn mesh_file_pipeline_matches_revolved_profile() {
    let dir = std::env::temp_dir().join(format!("marcam-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let job = finish_job("around");
    std::fs::write(
        dir.join("knob.stl"),
        Mesh::revolve(job.part.profile.as_ref().unwrap(), 720).to_stl(),
    )
    .unwrap();
    let mut with_mesh = job.clone();
    with_mesh.part.mesh = Some(marcam::job::MeshSource {
        file: "knob.stl".into(),
        axis: Default::default(),
        turn: Default::default(),
        translate: [0.0; 3],
        image: None,
    });
    assert!(plan(&with_mesh).is_err(), "mesh must be loaded first");
    let mesh = load_mesh(&with_mesh, &dir).unwrap().unwrap();
    let a = plan_with_mesh(&with_mesh, Some(&mesh)).unwrap();
    let b = plan(&job).unwrap();
    let pts = |tp: &marcam::Toolpath| -> Vec<(f64, f64, f64)> {
        tp.moves
            .iter()
            .filter_map(|m| match m {
                Move::Cut { x, r, a, .. } => Some((*x, *r, *a)),
                _ => None,
            })
            .collect()
    };
    let (pa, pb) = (pts(&a[0]), pts(&b[0]));
    assert_eq!(pa.len(), pb.len());
    for (p, q) in pa.iter().zip(&pb) {
        // STL stores f32 coordinates
        assert!((p.1 - q.1).abs() < 1e-4 && p.0 == q.0 && p.2 == q.2);
    }
    std::fs::remove_dir_all(&dir).ok();
}

fn words(line: &str) -> Vec<(char, f64)> {
    line.split_whitespace()
        .filter_map(|w| {
            let mut c = w.chars();
            let l = c.next()?;
            c.as_str().parse().ok().map(|v| (l, v))
        })
        .collect()
}

#[test]
fn linuxcnc_inverse_time_feeds() {
    let tps = plan(&knob_job()).unwrap();
    for tp in &tps {
        let z1 = post(tp, &Machine::makera_z1());
        let lc = post(tp, &Machine::linuxcnc());
        assert!(lc.contains("\nG93 "));
        assert!(verify_gcode(&lc, &limits_for(tp, &Machine::linuxcnc())).is_empty());
        let cuts: Vec<(f64, f64)> = tp
            .moves
            .iter()
            .filter_map(|m| match m {
                Move::Cut {
                    feed, plunge_cap, ..
                } => Some((*feed, *plunge_cap)),
                _ => None,
            })
            .collect();
        let z1_f: Vec<f64> = z1
            .lines()
            .filter(|l| l.starts_with("G1 X"))
            .map(|l| words(l).iter().find(|p| p.0 == 'F').unwrap().1)
            .collect();
        let lc_g1: Vec<&str> = lc.lines().filter(|l| l.starts_with("G1 X")).collect();
        assert_eq!(lc_g1.len(), cuts.len());
        assert_eq!(z1_f.len(), cuts.len());
        // walk the LinuxCNC file: every G1 F must be 1 / (minutes for the printed move at the
        // requested surface feed, never descending faster than the plunge feed)
        let (mut x, mut z, mut ang) = (0.0, 0.0, 0.0);
        let mut i = 0;
        for line in lc.lines() {
            if line.starts_with("G92") {
                ang = 0.0;
                continue;
            }
            let w = words(line);
            let get = |k: char| w.iter().find(|p| p.0 == k).map(|p| p.1);
            let (nx, nz, na) = (get('X'), get('Z'), get('A'));
            if line.starts_with("G1 X") {
                let (nx, nz, na) = (nx.unwrap(), nz.unwrap(), na.unwrap());
                let (feed, cap) = cuts[i];
                let rm = (0.5 * (z + nz)).max(0.5);
                let arc = rm * (na - ang).to_radians();
                let len = ((nx - x).powi(2) + (nz - z).powi(2) + arc * arc).sqrt();
                let mut dt = len / feed;
                if nz < z {
                    dt = dt.max((z - nz) / cap);
                }
                let f = get('F').unwrap();
                assert!(
                    (f - 1.0 / dt).abs() <= 5e-4 + 1e-9 / dt,
                    "{} move {i}: {line} want F{:.4}",
                    tp.name,
                    1.0 / dt
                );
                // cross-check with the Z1 post (degrees counted as mm, F = dist / dt)
                let dist = ((nx - x).powi(2) + (nz - z).powi(2) + (na - ang).powi(2)).sqrt();
                if dist > 0.05 {
                    let want = z1_f[i] / dist;
                    assert!(
                        (f - want).abs() <= 0.5 / dist + 0.01 * want,
                        "{} move {i}: G93 F{f} vs Z1-derived {want}",
                        tp.name
                    );
                }
                i += 1;
            }
            x = nx.unwrap_or(x);
            z = nz.unwrap_or(z);
            ang = na.unwrap_or(ang);
        }
        assert_eq!(i, cuts.len());
    }
}

#[test]
fn linuxcnc_finish_golden() {
    let job = finish_job("around");
    let tps = plan(&job).unwrap();
    let ours = post(&tps[0], &Machine::linuxcnc());
    let path = root().join("tests/golden/linuxcnc/knob_finish_around.nc");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &ours).unwrap();
    }
    let golden = std::fs::read_to_string(&path)
        .expect("golden missing: run with UPDATE_GOLDEN=1 to create it")
        .replace("\r\n", "\n");
    let (o, g): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), golden.lines().collect());
    assert_eq!(o.len(), g.len());
    for (i, (a, b)) in o.iter().zip(&g).enumerate() {
        if a == b {
            continue;
        }
        // allow one unit in the last printed digit (platform libm differences)
        let (wa, wb) = (words(a), words(b));
        assert_eq!(wa.len(), wb.len(), "line {}: {a} vs {b}", i + 1);
        for (p, q) in wa.iter().zip(&wb) {
            assert_eq!(p.0, q.0, "line {}", i + 1);
            assert!((p.1 - q.1).abs() <= 1.0001e-3, "line {}: {a} vs {b}", i + 1);
        }
    }
}
