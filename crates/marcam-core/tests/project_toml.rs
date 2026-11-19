//! Project files in TOML: every example reads the same as TOML as it did as JSON, writes back
//! to the same model, and plans to the same G-code; a hand-written TOML with comments reads;
//! bundles keep their files.

use marcam::project::Project;
use marcam::project_file::{project_to_toml, read_bundle, to_json_text, write_bundle};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in std::fs::read_dir(root().join("examples"))
        .unwrap()
        .flatten()
    {
        for f in std::fs::read_dir(d.path()).unwrap().flatten() {
            let n = f.file_name().to_string_lossy().to_string();
            if n.ends_with(".marcam.toml") || n.ends_with(".job.json") {
                out.push(f.path());
            }
        }
    }
    out
}

/// The G-code of every setup of a project text (files beside `dir`).
fn gcode(text: &str, dir: &std::path::Path) -> Vec<String> {
    let libs = marcam::profiles::builtin_libraries();
    let jobs = marcam::project::jobs_from_any_with(text, &libs, &|src| src.load(dir).ok()).unwrap();
    let mut out = Vec::new();
    for (_, job) in jobs {
        let mesh = marcam::load_mesh(&job, dir).unwrap();
        let m =
            marcam::profiles::builtin_machine(job.machine.as_deref().unwrap_or("makera-z1-4axis"))
                .unwrap();
        for tp in marcam::plan_with_mesh(&job, mesh.as_ref()).unwrap() {
            out.push(marcam::post(&tp, &m.post));
        }
    }
    out
}

#[test]
fn every_example_round_trips_through_toml() {
    let all = examples();
    assert!(all.len() >= 9, "{all:?}");
    for path in all {
        let text = std::fs::read_to_string(&path).unwrap();
        let dir = path.parent().unwrap();
        let (p, _) = Project::from_json_any(&text).unwrap();
        // written as TOML and read back: the same model
        let toml = project_to_toml(&p).unwrap();
        assert!(!toml.trim_start_matches('#').trim_start().starts_with('{'));
        let (back, _) = Project::from_json_any(&toml).unwrap();
        assert_eq!(back, p, "{}: TOML round trip", path.display());
        // and as JSON (still read forever): the same
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            Project::from_json_any(&json).unwrap().0,
            p,
            "{}: JSON",
            path.display()
        );
        // the same G-code from the TOML as from the file
        assert_eq!(
            gcode(&toml, dir),
            gcode(&text, dir),
            "{}: G-code",
            path.display()
        );
        // no float noise in what is written
        assert!(!toml.contains("0000000000"), "{}: {toml}", path.display());
    }
}

#[test]
fn a_hand_written_toml_with_comments_reads() {
    let text = r#"
# my knob, by hand
schema_version = "0.2"
name = "hand"   # a comment after a value

[[models]]
id = "part"
kind = "profile"
points = [[-20.0, 10.0], [-10.0, 8.0], [0.0, 6.0]]   # X (back to front), radius

[[tools]]
number = 1
description = "3.175 flat"
kind = "flat"
diameter = 3.175
flute_length = 15

[[setups]]
id = "s"
kind = "rotary"
stock = { shape = "cylinder", radius = 11.0, length = 30.0 }
clearance = 3.0
part = ["part"]
spindle_rpm = 12000

[[setups.operations]]   # roughing
id = "op1"
type = "rotary_turning"
tool = 1
rough_depth = 0.4      # mm per pass
rough_step = 1.0
rough_feed = 300.0
finish_allowance = 0.15
finish_step = 0.4
finish_feed = 250.0
plunge_feed = 60.0

[[setups.outputs]]
name = "hand"
title = "by hand"
operations = ["op1"]
"#;
    let (p, _) = Project::from_json_any(text).unwrap();
    assert_eq!(p.name, "hand");
    assert_eq!(p.setups[0].operations.len(), 1);
    let g = gcode(text, &root());
    assert!(g[0].lines().count() > 50, "plans");
    // and a mistake says where
    let e = Project::from_json_any("schema_version = \"0.2\"\nname = \n").unwrap_err();
    assert!(e.contains("TOML") && e.contains("line 2"), "{e}");
}

#[test]
fn json_files_stay_json_and_bundles_keep_their_files() {
    assert_eq!(to_json_text("{\"a\": 1}").unwrap(), "{\"a\": 1}");
    let knob =
        std::fs::read_to_string(root().join("examples/knob/knob_finish.marcam.toml")).unwrap();
    let stl = std::fs::read(root().join("examples/knob/knob.stl")).unwrap();
    let z = write_bundle(&knob, &[("knob.stl".into(), stl.clone())]);
    let (p, files) = read_bundle(&z).unwrap();
    assert_eq!(p, knob);
    assert_eq!(files, vec![("knob.stl".to_string(), stl)]);
    assert!(
        z.len() < knob.len() + 600_000,
        "deflated: {} bytes",
        z.len()
    );
}
