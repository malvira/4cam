//! End-to-end: the knob job must reproduce the G-code that was actually cut on the Makera Z1.

use fourcam::job::{Operation, ToolShape};
use fourcam::{limits_for, plan, post, verify_gcode, Error, Job, Machine};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn knob_job() -> Job {
    let text = std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap();
    Job::from_json(&text).unwrap()
}

/// Numbers may differ by one unit in the last printed digit (libm differences); all other text
/// must be identical.
fn same_within_last_digit(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (ta, tb): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    if ta.len() != tb.len() {
        return false;
    }
    ta.iter().zip(&tb).all(|(x, y)| {
        if x == y {
            return true;
        }
        let (xl, yl) = (&x[..1], &y[..1]);
        if xl != yl {
            return false;
        }
        let (xv, yv) = (&x[1..], &y[1..]);
        let dec = xv.split('.').nth(1).map_or(0, str::len);
        if dec != yv.split('.').nth(1).map_or(0, str::len) {
            return false;
        }
        match (xv.parse::<f64>(), yv.parse::<f64>()) {
            (Ok(p), Ok(q)) => (p - q).abs() <= 1.0001 * 10f64.powi(-(dec as i32)),
            _ => false,
        }
    })
}

#[test]
fn knob_matches_golden_gcode() {
    let job = knob_job();
    let machine = Machine::makera_z1();
    let tps = plan(&job).unwrap();
    assert_eq!(tps.len(), 2);
    let mut exact = 0;
    for tp in &tps {
        let ours = post(tp, &machine);
        let golden = std::fs::read_to_string(root().join(format!("tests/golden/{}.nc", tp.name)))
            .unwrap()
            .replace("\r\n", "\n");
        if ours == golden {
            exact += 1;
        }
        let (ol, gl): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), golden.lines().collect());
        assert_eq!(ol.len(), gl.len(), "{}: line count", tp.name);
        let mut near = 0;
        for (i, (o, g)) in ol.iter().zip(&gl).enumerate() {
            assert!(
                same_within_last_digit(o, g),
                "{} line {}:\n  ours:   {o}\n  golden: {g}",
                tp.name,
                i + 1
            );
            if o != g {
                near += 1;
            }
        }
        eprintln!(
            "{}: {} lines, {near} differ in the last digit",
            tp.name,
            ol.len()
        );

        let issues = verify_gcode(&ours, &limits_for(tp, &machine));
        assert!(issues.is_empty(), "{}: {issues:?}", tp.name);
    }
    eprintln!("{exact}/2 programs byte-identical");
}

#[test]
fn golden_files_pass_verifier() {
    let job = knob_job();
    let machine = Machine::makera_z1();
    for tp in plan(&job).unwrap() {
        let golden =
            std::fs::read_to_string(root().join(format!("tests/golden/{}.nc", tp.name))).unwrap();
        assert!(verify_gcode(&golden, &limits_for(&tp, &machine)).is_empty());
    }
}

#[test]
fn wrong_tool_shape_is_rejected() {
    let mut job = knob_job();
    if let Operation::RotaryTurning(o) = &mut job.programs[0].operations[0] {
        o.tool = 2;
    }
    assert!(matches!(
        plan(&job),
        Err(Error::WrongToolShape { tool: 2, .. })
    ));

    let mut job = knob_job();
    if let Operation::HelicalKnurl(o) = &mut job.programs[1].operations[0] {
        o.tool = 1;
    }
    assert!(matches!(
        plan(&job),
        Err(Error::WrongToolShape { tool: 1, .. })
    ));
}

#[test]
fn unknown_tool_is_rejected() {
    let mut job = knob_job();
    if let Operation::FlankChamfer(o) = &mut job.programs[1].operations[1] {
        o.tool = 7;
    }
    assert!(matches!(
        plan(&job),
        Err(Error::UnknownTool { tool: 7, .. })
    ));
}

#[test]
fn short_flute_is_rejected() {
    let mut job = knob_job();
    // knob needs 12.5 - 3 = 9.5 mm radial depth; a 10 mm flute leaves only 9 mm usable
    job.tools[0].flute_length = Some(10.0);
    assert!(matches!(
        plan(&job),
        Err(Error::FluteTooShort { tool: 1, .. })
    ));
    job.tools[0].flute_length = Some(10.6);
    assert!(plan(&job).is_ok());
    assert!(matches!(job.tools[0].shape, ToolShape::Flat { .. }));
}

#[test]
fn job_json_round_trips() {
    let job = knob_job();
    let text = serde_json::to_string(&job).unwrap();
    assert_eq!(Job::from_json(&text).unwrap(), job);
}

#[test]
fn knob_project_0_2_gives_the_same_gcode() {
    // the upgraded 0.2 project (examples/knob/knob.project.json) plans byte-identically
    let text = std::fs::read_to_string(root().join("examples/knob/knob.project.json")).unwrap();
    let jobs = fourcam::project::jobs_from_any(&text, &[]).unwrap();
    assert_eq!(jobs.len(), 1);
    let machine = Machine::makera_z1();
    for tp in plan(&jobs[0].1).unwrap() {
        let golden = std::fs::read_to_string(root().join(format!("tests/golden/{}.nc", tp.name)))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(post(&tp, &machine) == golden, "{} differs", tp.name);
    }
    // and upgrading the 0.1 job again gives the same project
    let job = std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap();
    let (p, _) = fourcam::project::Project::from_json_any(&job).unwrap();
    let (q, _) = fourcam::project::Project::from_json_any(&text).unwrap();
    assert_eq!(p, q);
}

#[test]
fn time_estimates_split_the_knob_by_operation() {
    let job =
        fourcam::Job::from_json(include_str!("../../../examples/knob/knob.job.json")).unwrap();
    let z1 = fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    let tps = fourcam::plan(&job).unwrap();
    for tp in &tps {
        let t = fourcam::estimate::estimate_toolpath(tp, &z1);
        let e = &t.estimate;
        // the operations and the preamble add up to the file
        let parts: f64 = t.ops.iter().map(|o| o.2).sum::<f64>() + t.overhead_s;
        assert!((parts - e.total_s).abs() < 1e-6, "{parts} vs {}", e.total_s);
        assert!((e.feed_s + e.rapid_s + e.other_s - e.total_s).abs() < 1e-9);
        // the G-code text alone gives the same total
        let text = fourcam::post(tp, &z1.post);
        let again = fourcam::estimate::estimate_gcode(
            &text,
            &fourcam::estimate::Dynamics::from_profile(&z1),
        );
        assert_eq!(again.total_s, e.total_s);
        // one manual tool change (60 s) and the 3 s spindle dwell
        assert!((e.other_s - 63.0).abs() < 1e-9, "{}", e.other_s);
    }
}
