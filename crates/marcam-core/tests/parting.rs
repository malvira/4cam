//! Parting (rotary): the knob cut off the bar behind it. The cut plans after the knob's own
//! programs without changing them, separates the part in the simulation (down to the sliver)
//! without touching it, takes its place from a picked edge or band, defaults to the part's
//! back, and a cut-off with operations after it is warned about.

use marcam::job::Operation;
use marcam::project::Project;
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn project() -> Project {
    let text =
        std::fs::read_to_string(root().join("examples/knob/knob_parting.marcam.toml")).unwrap();
    Project::from_json_any(&text).unwrap().0
}

fn job(p: &Project) -> marcam::job::Job {
    p.setup_job(&p.setups[0], &marcam::profiles::builtin_libraries())
        .unwrap()
}

#[test]
fn the_cut_off_separates_the_part_down_to_the_sliver_without_touching_it() {
    let j = job(&project());
    let tps = marcam::plan(&j).unwrap();
    assert_eq!(tps.len(), 3);
    let parting = &tps[2];
    assert!(
        parting.notes.iter().all(|n| !n.starts_with("(Warning")),
        "{:?}",
        parting.notes
    );
    // simulated: the groove behind the part is down to the 0.3 mm core, the part untouched
    let sim = marcam::simulate_job(&j, &tps, None, 0.1, 240, 0.02).unwrap();
    let g = sim.stock.grid;
    let row = |x: f64| ((x - g.x0) / g.dx).floor() as usize;
    let max_r = |i: usize| (0..g.na).map(|k| sim.stock.at(i, k)).fold(0.0, f64::max);
    let groove = max_r(row(-23.2));
    assert!(groove <= 0.3 + 0.02, "the groove's core: {groove}");
    let (_, st, dev) = &sim.programs[2];
    assert_eq!(dev.gouge_cells, 0, "the part is not cut into");
    assert!(st.rapid_hits.is_empty(), "no rapid through the stock");
    // the knob itself (its neck, behind the knurl) as turned
    let neck = max_r(row(-20.0));
    assert!((neck - 3.0).abs() < 0.2, "the neck: {neck}");
    // a time: the infeed slows as it goes in, so more than the straight depth at the feed
    let m = marcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    let t = marcam::estimate::estimate_toolpath(parting, &m);
    assert!(
        t.estimate.total_s > 12.2 / 0.3 * (360.0 / 360.0) * 60.0 * (0.3 / 150.0),
        "{}",
        t.estimate.total_s
    );
    // the G-code passes the verifier
    let g = marcam::post(parting, &m.post);
    assert!(marcam::verify_gcode(&g, &marcam::limits_for(parting, &m.post)).is_empty());
}

#[test]
fn operations_after_a_cut_off_are_warned_about() {
    let mut p = project();
    // the cut-off first, the knob after it
    let outs = &mut p.setups[0].outputs;
    let last = outs.pop().unwrap();
    outs.insert(0, last);
    let tps = marcam::plan(&job(&p)).unwrap();
    assert!(
        tps[0]
            .notes
            .iter()
            .any(|n| n.contains("3 operation(s) come after the cut-off at X-21.700")),
        "{:?}",
        tps[0].notes
    );
    // a groove (stopping at a radius) is not a cut-off: no warning
    let Operation::Parting(_) = &p.setups[0].operations[3].op else {
        panic!("op4 is the parting")
    };
    let mut v: Value = serde_json::to_value(&p).unwrap();
    v["setups"][0]["operations"][3]["stop_radius"] = json!(5.0);
    let p: Project = serde_json::from_value(v).unwrap();
    let tps = marcam::plan(&job(&p)).unwrap();
    assert!(tps[0].notes.iter().all(|n| !n.starts_with("(Warning")));
}

#[test]
fn it_defaults_to_the_parts_back_and_snaps_to_a_picked_edge_or_band() {
    let p = project();
    let libs = marcam::profiles::builtin_libraries();
    let d = p
        .op_defaults("parting", "setup1", Some(3), &[], &libs, None, &|_| None)
        .unwrap();
    assert_eq!(d["x"], json!(-21.7));
    assert!(d["depth_per_rev"].as_f64().unwrap() > 0.0 && d.get("stop_radius").is_some());
    // picked: the knob's round edges and bands, on the revolved profile
    let marcam::project::ModelKind::Profile(prof) = &p.models[0].kind else {
        panic!("the knob is a profile")
    };
    let mesh = marcam::Mesh::revolve(prof, 360);
    let picks = marcam::pickables::pickables(&mesh, marcam::pickables::Frame::Rotary);
    let cx = marcam::pickables::ApplyContext {
        stock_top: None,
        tool_diameter: Some(3.0),
        model: "profile".into(),
        op: None,
    };
    let edge = picks
        .iter()
        .find(|q| q.kind == "circular_edge" && (q.data["x"].as_f64().unwrap() + 15.0).abs() < 1e-6)
        .expect("the edge at X-15");
    let (v, _) = marcam::pickables::apply("parting", &[edge], &cx).unwrap();
    assert_eq!(v["x"], json!(-15.0));
    let band = picks
        .iter()
        .find(|q| q.kind == "cylindrical_band")
        .expect("a band");
    let (v, _) = marcam::pickables::apply("parting", &[band], &cx).unwrap();
    assert_eq!(v["x"], band.data["x"][0]);
    assert!(v["top"].as_f64().unwrap() > 0.0);
}
