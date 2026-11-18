//! Replacing a project's model with a changed part (the engine's `replace` module): the
//! operations whose picked features are still there are derived again from them, those whose
//! features are gone are put in error and left out of the G-code, and the error is kept in
//! the saved project.

use crate::{grip, prism};
use marcam::pickables::{apply, pickables, ApplyContext, Frame, Pickable};
use marcam::replace::{inline_tool_diameter, replace_model};
use marcam::Mesh;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marcam-replace-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn project(file: &str, stock: Value, tools: Value) -> Value {
    json!({"schema_version": "0.2", "name": "replace",
        "models": [{"id": "part", "name": "part", "kind": "mesh", "file": file}],
        "tools": tools,
        "setups": [{"id": "vise", "kind": "vise", "machine": "makera-carvera-4axis", "stock": stock,
            "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false}, "part": ["part"], "spindle_rpm": 12000,
            "operations": [], "outputs": []}]})
}

/// An operation pointed at `picks` (as the web app makes it: the fields from the features,
/// their signatures kept).
fn op(id: &str, ty: &str, base: Value, picks: &[&Pickable], top: f64, d: f64) -> Value {
    let mut o = base;
    o["id"] = json!(id);
    o["type"] = json!(ty);
    if ["raster3d", "rough3d"].contains(&ty) {
        let ids: Vec<&String> = picks.iter().flat_map(|p| &p.faces).collect();
        o["geometry"] = json!([{"kind": "faces", "model": "part", "ids": ids}]);
    } else {
        let cx = ApplyContext {
            stock_top: Some(top),
            tool_diameter: Some(d),
            model: "part".into(),
            op: Some(o.clone()),
        };
        for (k, v) in apply(ty, picks, &cx).unwrap().0 {
            o[k] = v;
        }
    }
    o["picks"] = json!(picks
        .iter()
        .map(|p| p.signature.clone())
        .collect::<Vec<_>>());
    o
}

fn with_ops(mut p: Value, ops: Vec<Value>) -> Value {
    let ids: Vec<Value> = ops.iter().map(|o| o["id"].clone()).collect();
    p["setups"][0]["outputs"] = json!(ids.iter().map(|id| json!({"name": format!("f_{}", id.as_str().unwrap()), "title": "x", "operations": [id]})).collect::<Vec<_>>());
    p["setups"][0]["operations"] = Value::Array(ops);
    p
}

fn replace(p: &Value, dir: &Path, file: &str) -> (Value, Vec<marcam::replace::OpReport>) {
    replace_model(
        p,
        "part",
        file,
        &|src| src.load(dir).map_err(|e| e.to_string()),
        &inline_tool_diameter,
    )
    .unwrap()
}

/// The programs planned for project `p` (files in `dir`).
fn programs(p: &Value, dir: &Path) -> Vec<String> {
    let libs = marcam::profiles::builtin_libraries();
    let jobs =
        marcam::project::jobs_from_any_with(&p.to_string(), &libs, &|src| src.load(dir).ok())
            .unwrap();
    let job = &jobs[0].1;
    let mesh = marcam::load_mesh(job, dir).unwrap();
    marcam::plan_with_mesh(job, mesh.as_ref())
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

fn feats(m: &Mesh) -> Vec<Pickable> {
    pickables(
        m,
        Frame::Xyz {
            index_a: 0.0,
            sides: false,
        },
    )
}

const TOOLS: &str = r#"[{"number": 1, "description": "5 mm flat", "kind": "flat", "diameter": 5.0, "flute_length": 15},
    {"number": 2, "description": "3.175 mm flat", "kind": "flat", "diameter": 3.175, "flute_length": 12},
    {"number": 3, "description": "3 mm ball", "kind": "ball", "diameter": 3.0, "flute_length": 12}]"#;

#[test]
fn the_grip_block_with_an_oval_removed_and_back() {
    let dir = tmp("grip");
    std::fs::write(dir.join("a.stl"), grip::mesh().to_stl()).unwrap();
    let one = [grip::OVALS[0]];
    std::fs::write(dir.join("one_oval.stl"), grip::mesh_with(&one).to_stl()).unwrap();

    let fs = feats(&grip::mesh());
    let flat = |left: bool| {
        fs.iter()
            .find(|p| {
                p.kind == "flat_face"
                    && p.data["z"].as_f64().unwrap() < -1.4
                    && (p.data["cx"].as_f64().unwrap() < 30.0) == left
            })
            .unwrap()
    };
    let pocket_b = fs
        .iter()
        .find(|p| p.kind == "pocket" && p.data["outline"][0][0].as_f64().unwrap() > 30.0)
        .unwrap();
    let outline = fs.iter().find(|p| p.kind == "outline").unwrap();
    let base = json!({"tool": 2, "depth": 1.0, "step_down": 0.5, "stepover": 0.6, "feed": 800.0, "plunge_feed": 200.0});
    let r = 3.175;
    let p = with_ops(
        project(
            "a.stl",
            json!({"shape": "box", "min": [-5.0, -5.0, -8.5], "max": [65.0, 35.0, 0.5]}),
            serde_json::from_str(TOOLS).unwrap(),
        ),
        vec![
            op("op1", "face", base.clone(), &[flat(true)], 0.5, r),
            op(
                "op2",
                "pocket",
                json!({"tool": 2, "depth": 1.0, "step_down": 0.5, "stepover": 0.5, "feed": 800.0, "plunge_feed": 200.0}),
                &[pocket_b],
                0.5,
                r,
            ),
            op(
                "op3",
                "contour",
                json!({"tool": 2, "step_down": 1.5, "feed": 800.0, "plunge_feed": 200.0}),
                &[outline],
                0.5,
                r,
            ),
            // no picks: the whole model, planned whatever the model
            json!({"id": "op4", "type": "raster3d", "tool": 3, "x_range": [0.0, 60.0], "y_range": [0.0, 30.0], "stepover": 1.0, "sample": 0.5, "step_down": 2.0, "floor": -1.6, "feed": 800.0, "plunge_feed": 200.0}),
        ],
    );
    assert_eq!(programs(&p, &dir).len(), 4);

    // one oval gone: the pocket on it is in error, the rest stand
    let (q, rep) = replace(&p, &dir, "one_oval.stl");
    assert_eq!(q["models"][0]["file"], "one_oval.stl");
    let by = |id: &str| rep.iter().find(|x| x.op == id).unwrap();
    assert!(by("op1").ok && by("op3").ok, "{rep:?}");
    assert!(
        !by("op2").ok && by("op2").error.as_ref().unwrap().contains("Pocket"),
        "{rep:?}"
    );
    assert!(
        rep.iter().all(|x| x.op != "op4"),
        "an operation without picks is not resolved"
    );
    let ops = q["setups"][0]["operations"].as_array().unwrap();
    assert!(ops[1]["error"]
        .as_str()
        .unwrap()
        .contains("not on the new model"));
    assert!(ops[0].get("error").is_none());
    // planned without it; saved and read back, the error is still there
    let progs = programs(&q, &dir);
    assert_eq!(
        progs,
        vec!["f_op1", "f_op3", "f_op4"],
        "the pocket in error is left out"
    );
    let (back, _) = marcam::project::Project::from_json_any(&q.to_string()).unwrap();
    assert!(back.setups[0].operations[1].error.is_some() && !back.setups[0].operations[1].plans());
    let saved: Value = serde_json::from_str(&serde_json::to_string(&back).unwrap()).unwrap();
    assert!(saved["setups"][0]["operations"][1]["error"].is_string());

    // replaced back with the original: the pocket is found again, the error cleared
    let (q2, rep2) = replace(&q, &dir, "a.stl");
    assert!(rep2.iter().all(|x| x.ok), "{rep2:?}");
    assert!(q2["setups"][0]["operations"][1].get("error").is_none());
    assert_eq!(programs(&q2, &dir).len(), 4);
}

#[test]
fn the_bracket_with_a_hole_moved_and_its_pocket_a_little_longer() {
    let dir = tmp("bracket");
    std::fs::write(dir.join("a.stl"), prism::bracket().to_stl()).unwrap();
    let mut b = prism::Bracket::default();
    b.through[3] = [50.0, 30.0];
    b.pocket[2] = 28.03;
    std::fs::write(dir.join("b.stl"), prism::bracket_with(&b).to_stl()).unwrap();
    let fs = feats(&prism::bracket());
    let hole = |x: f64, y: f64| {
        fs.iter()
            .find(|p| {
                p.kind == "hole"
                    && (p.data["center"][0].as_f64().unwrap() - x).abs() < 1e-6
                    && (p.data["center"][1].as_f64().unwrap() - y).abs() < 1e-6
            })
            .unwrap()
    };
    let pocket = fs
        .iter()
        .find(|p| p.kind == "pocket" && p.data["floor"].as_f64().unwrap() < -4.9)
        .unwrap();
    let drill =
        json!({"tool": 1, "peck": 5.0, "retract": 1.0, "feed": 300.0, "plunge_feed": 100.0});
    let p = with_ops(
        project(
            "a.stl",
            json!({"shape": "box", "min": [0.0, 0.0, -12.0], "max": [60.0, 40.0, 0.0]}),
            serde_json::from_str(TOOLS).unwrap(),
        ),
        vec![
            op(
                "op1",
                "drill",
                drill.clone(),
                &[
                    hole(40.0, 8.0),
                    hole(52.0, 8.0),
                    hole(40.0, 32.0),
                    hole(52.0, 32.0),
                ],
                0.0,
                5.0,
            ),
            op("op2", "drill", drill, &[hole(40.0, 8.0)], 0.0, 5.0),
            op(
                "op3",
                "pocket",
                json!({"tool": 2, "depth": 1.0, "step_down": 1.0, "stepover": 0.5, "feed": 800.0, "plunge_feed": 200.0}),
                &[pocket],
                0.0,
                3.175,
            ),
        ],
    );
    let (q, rep) = replace(&p, &dir, "b.stl");
    let by = |id: &str| rep.iter().find(|x| x.op == id).unwrap();
    assert!(!by("op1").ok, "the four-hole drilling lost one: {rep:?}");
    assert!(
        by("op1").error.as_ref().unwrap().contains("Hole 5.00"),
        "{rep:?}"
    );
    assert!(by("op2").ok, "the hole that stayed: {rep:?}");
    assert!(
        by("op3").ok,
        "a pocket 0.03 mm longer is the same pocket: {rep:?}"
    );
    let pk = &q["setups"][0]["operations"][2];
    let x1 = pk["paths"][0]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q[0].as_f64().unwrap())
        .fold(f64::MIN, f64::max);
    assert!(
        (x1 - 28.03).abs() < 1e-6,
        "the pocket follows the new outline: {x1}"
    );
    assert_eq!(programs(&q, &dir), vec!["f_op2", "f_op3"]);
}

#[test]
fn ambiguous_and_missing_matches_say_so() {
    let fs = feats(&prism::bracket());
    // a signature of a 5 mm hole without its centre: four holes fit
    let h = fs
        .iter()
        .find(|p| p.kind == "hole" && p.data["through"] == true)
        .unwrap();
    let mut sig = h.signature.clone();
    sig["data"].as_object_mut().unwrap().remove("center");
    let e = marcam::replace::find(&sig, &fs).unwrap_err();
    assert!(
        e.contains("4 features") && e.contains("pick it again"),
        "{e}"
    );
    let mut gone = h.signature.clone();
    gone["data"]["diameter"] = json!(9.0);
    assert!(marcam::replace::find(&gone, &fs)
        .unwrap_err()
        .contains("not on the new model"));
    assert_eq!(marcam::replace::find(&h.signature, &fs).unwrap().id, h.id);
}
