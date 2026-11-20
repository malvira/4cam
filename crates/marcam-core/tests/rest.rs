//! Rest roughing: a 3D roughing with `rest` cuts only where the operations before it in the
//! setup left material (simulated), so a smaller tool after a larger one goes where the
//! larger could not reach, not over the whole part again.

use marcam::sim3::{simulate3, Heightmap3};
use marcam::{load_mesh, plan_with_mesh, plan_with_mesh_cached, Job, Move, PlanCache, Toolpath};
use serde_json::{json, Value};
use std::path::Path;

const PICKING: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/picking");

/// The oval grip block (a textured grip between two recessed ovals): `ops` in one file.
fn project(ops: Value) -> Value {
    let mut doc: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(PICKING).join("oval_grip.project.json")).unwrap(),
    )
    .unwrap();
    let ids: Vec<Value> = ops
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].clone())
        .collect();
    doc["setups"][0]["operations"] = ops;
    doc["setups"][0]["outputs"] = json!([{"name": "grip", "title": "grip", "operations": ids}]);
    doc
}

/// A 3D roughing of the whole block with tool `tool`.
fn rough(id: &str, tool: u32, rest: bool) -> Value {
    let mut o = json!({"id": id, "type": "rough3d", "tool": tool, "x_range": [-5.0, 65.0], "y_range": [-5.0, 35.0],
        "stepover": 1.2, "sample": 0.4, "step_down": 1.5, "floor": -8.5, "leave": 0.25, "feed": 600.0, "plunge_feed": 100.0});
    if rest {
        o["rest"] = json!(true);
    }
    o
}

fn job(doc: &Value) -> Job {
    let libs = marcam::profiles::builtin_libraries();
    let dir = Path::new(PICKING).to_path_buf();
    let meshes = |src: &marcam::job::MeshSource| src.load(&dir).ok();
    marcam::project::jobs_from_any_with(&doc.to_string(), &libs, &meshes)
        .unwrap()
        .remove(0)
        .1
}

fn plan(doc: &Value) -> Result<Vec<Toolpath>, String> {
    let job = job(doc);
    let mesh = load_mesh(&job, Path::new(PICKING)).unwrap();
    plan_with_mesh(&job, mesh.as_ref()).map_err(|e| e.to_string())
}

/// How far operation `op` cuts (mm along its feed moves).
fn cuts(tp: &Toolpath, op: u32) -> f64 {
    let mut on = false;
    let mut at: Option<[f64; 3]> = None;
    let mut len = 0.0;
    for m in &tp.moves {
        if let Move::Operation { id, .. } = m {
            on = *id == op;
        }
        let to = match *m {
            Move::Line3 { x, y, z, .. } => Some([x, y, z]),
            Move::Rapid3 { x, y, z } => {
                at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])])
            }
            _ => None,
        };
        if let (true, Some(a), Some(b), Move::Line3 { .. }) = (on, at, to, m) {
            len += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        }
        if to.is_some() {
            at = to;
        }
    }
    len
}

/// Operation `op`'s machining time (s) on the setup's machine (the Carvera).
fn op_time(tp: &Toolpath, op: u32) -> f64 {
    let m = marcam::profiles::builtin_machines()
        .into_iter()
        .find(|m| m.id == "makera-carvera-4axis")
        .unwrap();
    marcam::estimate::estimate_toolpath(tp, &m)
        .ops
        .iter()
        .find(|o| o.0 == op)
        .unwrap()
        .2
}

/// The block after the toolpath, at 0.1 mm cells.
fn cut_block(doc: &Value, tp: &Toolpath) -> Heightmap3 {
    let job = job(doc);
    let mut hm = Heightmap3::new(job.stock_box.as_ref().unwrap(), 0.1);
    let cutter = |t: u32| {
        job.tool(t)
            .and_then(|d| marcam::cutter_for(t, d.shape).ok())
    };
    simulate3(&mut hm, tp, &cutter, 0.1);
    hm
}

#[test]
fn rest_roughing_cuts_only_what_the_larger_tool_left() {
    // the 5 mm roughing, then the 3.175 mm one over everything, or as rest roughing
    let full = project(json!([rough("op1", 1, false), rough("op2", 2, false)]));
    let rest = project(json!([rough("op1", 1, false), rough("op2", 2, true)]));
    let tf = plan(&full).unwrap().remove(0);
    let tr = plan(&rest).unwrap().remove(0);
    // the first operation is the same; the rest one cuts far less, in less time (the grip's
    // texture leaves the smaller tool something almost all over it: about two thirds; a part
    // whose rest is a few corners saves far more)
    assert_eq!(cuts(&tf, 0), cuts(&tr, 0));
    let (all_len, left_len) = (cuts(&tf, 1), cuts(&tr, 1));
    assert!(left_len > 0.0, "the smaller tool has something to cut");
    assert!(
        left_len < 0.6 * all_len,
        "rest roughing cuts {left_len:.0} mm, the full pass {all_len:.0} mm"
    );
    let (all, left) = (op_time(&tf, 1), op_time(&tr, 1));
    assert!(
        left < 0.7 * all,
        "rest roughing takes {left:.0} s, the full pass {all:.0} s"
    );
    // and leaves the block as the full pass does, away from the part's walls (there the full
    // pass's moves down off the part's edge, straight between two samples, take a little of
    // the stock left beside the wall; the rest pass does not)
    let (a, b) = (cut_block(&full, &tf), cut_block(&rest, &tr));
    let j0 = job(&full);
    let mesh = load_mesh(&j0, Path::new(PICKING)).unwrap().unwrap();
    let part = marcam::sim3::part_top(&a, &mesh);
    let near_wall = |i: usize, j: usize| {
        let k = 5isize; // 0.5 mm
        (-k..=k).any(|dj| {
            (-k..=k).any(|di| {
                let (ii, jj) = (i as isize + di, j as isize + dj);
                ii >= 0
                    && jj >= 0
                    && (ii as usize) < a.nx
                    && (jj as usize) < a.ny
                    && part[jj as usize * a.nx + ii as usize].is_some()
                        != part[j * a.nx + i].is_some()
            })
        })
    };
    let mut worst = (0.0f64, 0.0, 0.0);
    for j in 0..a.ny {
        for i in 0..a.nx {
            let k = j * a.nx + i;
            if b.z[k] - a.z[k] > worst.0 && !near_wall(i, j) {
                let (x, y) = a.centre(i, j);
                worst = (b.z[k] - a.z[k], x, y);
            }
        }
    }
    assert!(
        worst.0 < 0.15,
        "rest roughing left {:.3} mm more than the full pass at ({:.2}, {:.2})",
        worst.0,
        worst.1,
        worst.2
    );
}

#[test]
fn rest_roughing_first_in_the_setup_cuts_the_whole_block() {
    // nothing before it: the whole block is there to cut, as without rest
    let plain = plan(&project(json!([rough("op1", 2, false)])))
        .unwrap()
        .remove(0);
    let rest = plan(&project(json!([rough("op1", 2, true)])))
        .unwrap()
        .remove(0);
    let (p, r) = (cuts(&plain, 0), cuts(&rest, 0));
    assert!(
        (r - p).abs() <= 0.02 * p,
        "{r:.0} mm with rest, {p:.0} mm without"
    );
}

#[test]
fn rest_is_for_3d_roughing_on_a_vise() {
    let mut finish = rough("op2", 3, true);
    finish["type"] = json!("raster3d");
    let e = plan(&project(json!([rough("op1", 1, false), finish]))).unwrap_err();
    assert!(e.contains("rest is for 3D roughing"), "{e}");
}

#[test]
fn rest_roughing_is_planned_again_when_what_came_before_changes() {
    let mut doc = project(json!([rough("op1", 1, false), rough("op2", 2, true)]));
    let mut cache = PlanCache::new();
    let run = |doc: &Value, cache: &mut PlanCache| {
        let job = job(doc);
        let mesh = load_mesh(&job, Path::new(PICKING)).unwrap();
        let tps = plan_with_mesh_cached(&job, mesh.as_ref(), cache).unwrap();
        (tps, cache.take_counts())
    };
    let (first, _) = run(&doc, &mut cache);
    let (again, counts) = run(&doc, &mut cache);
    assert_eq!(counts, (2, 0), "nothing changed: both reused");
    assert_eq!(first[0].moves, again[0].moves);
    // a deeper first roughing (less left): the rest one is planned again too
    doc["setups"][0]["operations"][0]["leave"] = json!(0.1);
    let (_, counts) = run(&doc, &mut cache);
    assert_eq!(
        counts,
        (0, 2),
        "the first changed, and the rest one with it"
    );
}
