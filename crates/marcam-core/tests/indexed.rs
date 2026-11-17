//! Indexed 3+1: 3-axis operations on a rotary setup, with the part turned to a fixed A.
//! The knob example adds a D-flat on the shaft (A0) and a cross hole in the body (A90)
//! after turning; the turning and knurl programs stay byte-identical to the goldens.

use marcam::job::{Operation, ToolShape};
use marcam::ops::{defaults, registry, DefaultsContext};
use marcam::profiles::{builtin_machine, check_limits};
use marcam::{limits_for, plan, post, simulate_job, verify_gcode, Job};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn knob_indexed() -> Job {
    let text =
        std::fs::read_to_string(root().join("examples/knob/knob_indexed.project.json")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs = marcam::project::jobs_from_any_with(&text, &libs, &|_| None).unwrap();
    jobs.remove(0).1
}

#[test]
fn indexed_programs_post_verify_and_keep_the_goldens() {
    let job = knob_indexed();
    let tps = plan(&job).unwrap();
    assert_eq!(tps.len(), 3);
    let z1 = builtin_machine("makera-z1-4axis").unwrap();
    for (tp, golden) in tps.iter().zip(["knob_1_profile", "knob_2_knurl"]) {
        let want = std::fs::read_to_string(root().join(format!("tests/golden/{golden}.nc")))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(post(tp, &z1.post) == want, "{golden} changed");
    }
    let text = post(&tps[2], &z1.post);
    assert!(text.contains("G0 A0.000\n") && text.contains("G0 A90.000\n"));
    assert!(
        text.contains("G1 X-20.113 Y-1.413 Z2.500 F60"),
        "D-flat plunge"
    );
    let issues = verify_gcode(&text, &limits_for(&tps[2], &z1.post));
    assert!(issues.is_empty(), "{issues:?}");
    let errors: Vec<_> = check_limits(&job, &tps, &z1)
        .into_iter()
        .filter(|i| i.severity == "error")
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn simulation_shows_the_flat_and_the_hole_at_their_angles() {
    let job = knob_indexed();
    let tps = plan(&job).unwrap();
    let res = simulate_job(&job, &tps, None, 0.1, 360, 0.02).unwrap();
    for ((name, st, _), tp) in res.programs.iter().zip(&tps) {
        for &i in st.rapid_hits.iter().take(4) {
            eprintln!(
                "{name}: {:?} after {:?}",
                tp.moves[i],
                &tp.moves[i.saturating_sub(2)..i]
            );
        }
        assert!(st.rapid_hits.is_empty(), "{name}: rapids through stock");
    }
    // the flat and the hole cut below the turned profile on purpose: reported as feature
    // cells, not gouges (the knurl program's grooves likewise)
    let (_, _, dev) = &res.programs[2];
    assert_eq!(dev.gouge_cells, 0, "{dev:?}");
    assert!(dev.feature_cells > 100, "{dev:?}");
    let g = res.stock.grid;
    let r_at = |x: f64, phi_deg: f64| {
        let i = ((x - g.x0) / g.dx) as usize;
        let j = (phi_deg / 360.0 * g.na as f64) as usize % g.na;
        res.stock.r[i * g.na + j] as f64
    };
    // D-flat at A0: the tool came down over part angle 90 to Z2.5; the far side stays turned
    assert!(
        (r_at(-20.05, 90.0) - 2.5).abs() < 0.01,
        "{}",
        r_at(-20.05, 90.0)
    );
    assert!(
        (r_at(-20.05, 270.0) - 3.0).abs() < 0.01,
        "{}",
        r_at(-20.05, 270.0)
    );
    // a flat chord: 1 mm off the top, the surface is at 2.5 / cos(angle)
    let off = (1.0f64 / 2.5).atan().to_degrees();
    let want = 2.5 / off.to_radians().cos();
    assert!(
        (r_at(-20.05, 90.0 + off) - want).abs() < 0.03,
        "{} vs {want}",
        r_at(-20.05, 90.0 + off)
    );
    // cross hole at A90: over part angle 0, down to radius 6 (3 below Z9); the body
    // elsewhere keeps its knurled surface (over 8)
    assert!(
        (r_at(-8.05, 0.5) - 6.0).abs() < 0.01,
        "{}",
        r_at(-8.05, 0.5)
    );
    assert!(r_at(-8.05, 180.0) > 8.0 && r_at(-8.05, 90.0) > 8.0);
    // and the hole is as wide as the 3.175 mm endmill: 1.4 mm to the side it is still cut
    assert!(r_at(-8.05 + 1.4, 0.5) < 6.01 && r_at(-8.05 + 1.7, 0.5) > 8.0);
}

#[test]
fn every_3_axis_type_defaults_and_plans_on_a_rotary_setup() {
    let mut job = knob_indexed();
    job.tools.push(marcam::job::Tool {
        flute_length: Some(12.0),
        ..marcam::job::Tool::new(3, "ball 3.175", ToolShape::Ball { diameter: 3.175 })
    });
    let z1 = builtin_machine("makera-z1-4axis").unwrap();
    for t in registry()
        .iter()
        .filter(|t| t.machine_needs.contains(&"xyz"))
    {
        let cx = DefaultsContext {
            job: &job,
            regions: None,
            machine: Some(&z1),
            tool: None,
            boundary: None,
            part_xy: None,
            part_z: None,
        };
        let mut v = defaults(t.kind, &cx).unwrap_or_else(|e| panic!("{}: {e}", t.kind));
        assert_eq!(v["index_a"], 0.0, "{}", t.kind);
        v["index_a"] = 45.0.into();
        let op: Operation = serde_json::from_value(v).unwrap();
        let mut one = job.clone();
        one.programs.truncate(1);
        one.programs[0].operations.push(op);
        let tps = plan(&one).unwrap_or_else(|e| panic!("{}: {e}", t.kind));
        let text = post(&tps[0], &z1.post);
        assert!(text.contains("G0 A45.000"), "{}", t.kind);
        let issues = verify_gcode(&text, &limits_for(&tps[0], &z1.post));
        assert!(
            issues.is_empty(),
            "{}: {:?}",
            t.kind,
            &issues[..issues.len().min(3)]
        );
    }
}

#[test]
fn a_vise_setup_rejects_an_index_angle() {
    let text = std::fs::read_to_string(root().join("examples/plate/plate.project.json")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["setups"][0]["operations"][0]["index_a"] = 90.0.into();
    let libs = marcam::profiles::builtin_libraries();
    let jobs = marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|_| None).unwrap();
    let e = plan(&jobs[0].1).unwrap_err().to_string();
    assert!(e.contains("index_a 90 on a vise setup"), "{e}");
}
