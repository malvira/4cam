//! G-code options: a drill operation written as a canned cycle (G83 with pecks, G81 without)
//! on a machine that has them, as its moves on one that does not; the setup's work offset,
//! header and footer lines, line numbers and no comments. The verifier and the time estimate
//! read the cycles as the moves they make.

use marcam::job::{Job, PostOptions};
use marcam::post::post_lines;
use marcam::{limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Machine};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The plate with its drill operation canned (or not) and the setup's `post` options.
fn plate(canned: bool, post: Option<Value>) -> Job {
    let dir = root().join("examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let (p, _) = marcam::project::Project::from_json_any(&text).unwrap();
    let mut v = serde_json::to_value(&p).unwrap();
    for o in v["setups"][0]["operations"].as_array_mut().unwrap() {
        if o["type"] == "drill" {
            o["canned"] = json!(canned);
        }
    }
    if let Some(po) = post {
        v["setups"][0]["post"] = po;
    }
    let libs = marcam::profiles::builtin_libraries();
    marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|src| src.load(&dir).ok())
        .unwrap()
        .remove(0)
        .1
}

fn programs(job: &Job) -> Vec<marcam::Toolpath> {
    let mesh = load_mesh(job, &root().join("examples/plate")).unwrap();
    plan_with_mesh(job, mesh.as_ref()).unwrap()
}

#[test]
fn drilling_as_a_canned_cycle_where_the_machine_has_them() {
    let linuxcnc = Machine::linuxcnc();
    assert!(linuxcnc.canned_cycles);
    let carvera = Machine::builtin("makera-carvera-4axis").unwrap();
    assert!(!carvera.canned_cycles);
    let canned = programs(&plate(true, None));
    let plain = programs(&plate(false, None));
    // the moves are the same (the simulation and the checks use them)
    assert_eq!(canned[0].moves.len(), plain[0].moves.len());
    let tp = &canned[0];
    let g = post(tp, &linuxcnc);
    // the plate's 4 holes, 4 deep from -0.5 in 1.5 mm pecks, R 1 above the top
    let cycles: Vec<&str> = g.lines().filter(|l| l.contains("G83")).collect();
    assert_eq!(cycles.len(), 4, "{g}");
    assert_eq!(
        cycles[0],
        "G98 G83 X40.000 Y6.000 Z-4.500 R0.500 Q1.500 F60"
    );
    let i = g.lines().position(|l| l.contains("G83")).unwrap();
    let lines: Vec<&str> = g.lines().collect();
    // inverse-time feeds off around it (canned cycles take mm/min), G80 after
    assert_eq!(lines[i - 1], "G94");
    assert_eq!(lines[i + 4], "G80");
    assert_eq!(lines[i + 5], "G93");
    // far shorter than the moves, and as clean and as long to run
    let gp = post(&plain[0], &linuxcnc);
    assert!(g.lines().count() + 20 < gp.lines().count());
    let lim = limits_for(tp, &linuxcnc);
    assert!(
        verify_gcode(&g, &lim).is_empty(),
        "{:?}",
        verify_gcode(&g, &lim)
    );
    let prof = marcam::profiles::builtin_machine("linuxcnc-4axis").unwrap();
    let dy = marcam::estimate::Dynamics::from_profile(&prof);
    let (a, b) = (
        marcam::estimate::estimate_gcode(&g, &dy).total_s,
        marcam::estimate::estimate_gcode(&gp, &dy).total_s,
    );
    assert!((a - b).abs() < 0.1 * b, "{a:.1} s vs {b:.1} s");
    // the Carvera's post has none: the moves, said so
    let gc = post(tp, &carvera);
    assert!(!gc.contains("G83") && gc.contains("no canned cycles"));
    // no pecks (a peck as deep as the hole): G81
    let mut j = plate(true, None);
    for p in &mut j.programs {
        for o in &mut p.operations {
            if let marcam::job::Operation::Drill(d) = o {
                d.peck = 10.0;
            }
        }
    }
    let g81 = post(&programs(&j)[0], &linuxcnc);
    assert_eq!(g81.lines().filter(|l| l.contains("G81")).count(), 4);
    assert!(!g81.contains("Q"));
}

#[test]
fn the_setup_s_work_offset_header_footer_numbers_and_comments() {
    let opts = json!({"work_offset": "G55", "header": ["M8 (coolant on)", "(T{tool} first)"], "footer": ["M9"], "line_numbers": true, "no_comments": true});
    let job = plate(false, Some(opts.clone()));
    assert_eq!(
        job.post,
        Some(serde_json::from_value::<PostOptions>(opts).unwrap())
    );
    let tps = programs(&job);
    let tp = &tps[0];
    assert!(tp.post.is_some());
    let m = Machine::builtin("makera-carvera-4axis").unwrap();
    let (g, marks, at) = post_lines(tp, &m);
    let lines: Vec<&str> = g.lines().collect();
    // numbered, no comments (a comment-only header line goes)
    assert!(
        lines
            .iter()
            .enumerate()
            .all(|(i, l)| l.starts_with(&format!("N{} ", 10 * (i + 1)))),
        "{g}"
    );
    assert!(!g.contains('('));
    // the preamble, then the work offset and the header
    let body: Vec<&str> = lines.iter().map(|l| l.split_once(' ').unwrap().1).collect();
    let k = body.iter().position(|l| *l == "G55").unwrap();
    assert_eq!(body[k + 1], "M8");
    assert!(body[..k].contains(&"G17"));
    // the footer before the postamble
    let f = body.iter().rposition(|l| *l == "M9").unwrap();
    assert!(f > body.len() - 6 && f < body.len() - 1);
    // where each operation starts and each move's line still point into the text
    for (_, _, i) in &marks {
        assert!(*i < lines.len());
    }
    assert_eq!(at.len(), tp.moves.len());
    assert!(at.iter().all(|&i| i <= lines.len()));
    // and it verifies
    assert!(verify_gcode(&g, &limits_for(tp, &m)).is_empty());
    // a bad work offset is refused
    let dir = root().join("examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let (p, _) = marcam::project::Project::from_json_any(&text).unwrap();
    let mut v = serde_json::to_value(&p).unwrap();
    v["setups"][0]["post"] = json!({"work_offset": "G12"});
    let libs = marcam::profiles::builtin_libraries();
    let e = marcam::project::jobs_from_any_with(&v.to_string(), &libs, &|_| None).unwrap_err();
    assert!(e.contains("work offset"), "{e}");
}
