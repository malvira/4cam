//! The oval-pockets grip block (after a part of Mar's): 60 x 30, top surface a textured grip
//! (bumps between Z-0.6 and Z0), two recessed oval flats at Z-1.5 with sloped sides, each with
//! a 4 mm through hole in its middle; bottom at Z-8, X0 Y0 a corner.
//!
//! Built as a height field on a 0.5 mm grid, closed by walls round its outline and its holes:
//! the flats are exactly flat (every grid point inside an oval is at Z-1.5), their edges follow
//! the grid, and the slopes and bumps are faceted as the grid falls.

use marcam::Mesh;

pub const SIZE: [f64; 2] = [60.0, 30.0];
pub const BOTTOM: f64 = -8.0;
pub const FLAT_Z: f64 = -1.5;
/// The ovals: centre and semi-axes (mm).
pub const OVALS: [([f64; 2], [f64; 2]); 2] = [([16.0, 15.0], [10.0, 7.0]), ([44.0, 15.0], [10.0, 7.0])];
pub const HOLE_R: f64 = 2.0;
const CELL: f64 = 0.5;
/// Width of the slope from a flat up to the grip (mm).
const SLOPE: f64 = 0.5;

/// The grip surface's height (between -0.6 and 0).
pub fn bump(x: f64, y: f64) -> f64 {
    -0.3 + 0.3 * (x * 1.3).sin() * (y * 1.1).cos()
}

/// How far outside an oval a point is (mm, about; <= 0 inside), the nearest oval.
fn outside(x: f64, y: f64) -> f64 {
    OVALS
        .iter()
        .map(|&([cx, cy], [a, b])| {
            let d = (((x - cx) / a).powi(2) + ((y - cy) / b).powi(2)).sqrt();
            (d - 1.0) * a.min(b)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Height of the top at (x, y).
pub fn height(x: f64, y: f64) -> f64 {
    let t = outside(x, y);
    if t <= 0.0 {
        FLAT_Z
    } else if t < SLOPE {
        // at least halfway up at the first grid point outside: a crisp edge to the flat
        FLAT_Z + (bump(x, y) - FLAT_Z) * (0.5 + 0.5 * t / SLOPE)
    } else {
        bump(x, y)
    }
}

/// Whether the grid cell (i, j) is material (not inside a hole).
fn solid(i: i64, j: i64) -> bool {
    let (nx, ny) = cells();
    if i < 0 || j < 0 || i >= nx || j >= ny {
        return false;
    }
    let (x, y) = ((i as f64 + 0.5) * CELL, (j as f64 + 0.5) * CELL);
    !OVALS
        .iter()
        .any(|&([cx, cy], _)| (x - cx).hypot(y - cy) < HOLE_R)
}

fn cells() -> (i64, i64) {
    ((SIZE[0] / CELL).round() as i64, (SIZE[1] / CELL).round() as i64)
}

pub fn mesh() -> Mesh {
    let (nx, ny) = cells();
    let top = |i: i64, j: i64| {
        let (x, y) = (i as f64 * CELL, j as f64 * CELL);
        [x, y, height(x, y)]
    };
    let bot = |i: i64, j: i64| [i as f64 * CELL, j as f64 * CELL, BOTTOM];
    let mut tris = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            if !solid(i, j) {
                continue;
            }
            let (a, b, c, d) = (top(i, j), top(i + 1, j), top(i + 1, j + 1), top(i, j + 1));
            // up: counter-clockwise from above
            tris.push([a, b, c]);
            tris.push([a, c, d]);
            let (a, b, c, d) = (bot(i, j), bot(i + 1, j), bot(i + 1, j + 1), bot(i, j + 1));
            tris.push([a, c, b]);
            tris.push([a, d, c]);
            // walls where the neighbour is empty (outside, or a hole): outward
            for (di, dj, p, q) in [
                (0, -1, (i, j), (i + 1, j)),
                (1, 0, (i + 1, j), (i + 1, j + 1)),
                (0, 1, (i + 1, j + 1), (i, j + 1)),
                (-1, 0, (i, j + 1), (i, j)),
            ] {
                if solid(i + di, j + dj) {
                    continue;
                }
                let (tp, tq, bp, bq) = (top(p.0, p.1), top(q.0, q.1), bot(p.0, p.1), bot(q.0, q.1));
                tris.push([bp, bq, tq]);
                tris.push([bp, tq, tp]);
            }
        }
    }
    Mesh { triangles: tris }
}

#[cfg(test)]
mod tests {
    #[test]
    fn closed_and_the_flats_are_flat() {
        let m = super::mesh();
        // closed: every directed edge has its opposite
        let k = |p: &[f64; 3]| p.map(|c| (c * 1e6).round() as i64);
        let mut edges = std::collections::HashMap::new();
        for t in &m.triangles {
            for e in 0..3 {
                *edges.entry((k(&t[e]), k(&t[(e + 1) % 3]))).or_insert(0i32) += 1;
            }
        }
        for ((a, b), n) in &edges {
            assert_eq!(edges.get(&(*b, *a)), Some(n), "open edge");
        }
        assert_eq!(super::height(16.0, 15.0), super::FLAT_Z);
        assert!(super::height(30.0, 15.0) > -0.61);
    }
}

/// Facing, a pocket and a cut-out pointed at the grip block's features (Mar's oval part): each
/// cuts only where it should.
#[cfg(test)]
mod picked {
    use marcam::pickables::{apply, pickables, ApplyContext, Frame, Pickable};
    use marcam::toolpath::Move;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    const R: f64 = 3.175 / 2.0;

    fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/picking")
    }

    /// Plan one operation `op` on the grip block's vise setup; its cutting points (x, y, z).
    fn cuts(op: Value) -> Vec<[f64; 3]> {
        let p: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oval_grip.project.json")).unwrap()).unwrap();
        cuts_in(p, &dir(), op).0
    }

    /// Plan `op` as the only operation of project `p`'s first setup (files in `dir`): its
    /// cutting points, sampled along each move, and the placed part.
    pub(super) fn cuts_in(mut p: Value, dir: &std::path::Path, op: Value) -> (Vec<[f64; 3]>, marcam::Mesh) {
        let s = &mut p["setups"][0];
        s["operations"] = json!([op]);
        s["outputs"] = json!([{"name": "grip", "title": "grip", "operations": ["op1"]}]);
        let libs = marcam::profiles::builtin_libraries();
        let jobs = marcam::project::jobs_from_any_with(&p.to_string(), &libs, &|src| src.load(dir).ok()).unwrap();
        let job = &jobs[0].1;
        let mesh = marcam::load_mesh(job, dir).unwrap();
        let tps = marcam::plan_with_mesh(job, mesh.as_ref()).unwrap();
        // each cutting move, sampled along its length (from the point before it)
        let mut all = Vec::new();
        let mut at: Option<[f64; 3]> = None;
        for m in &tps[0].moves {
            match *m {
                Move::Line3 { x, y, z, .. } => {
                    let b = [x, y, z];
                    let a = at.unwrap_or(b);
                    for k in 0..=8 {
                        let t = k as f64 / 8.0;
                        all.push([0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t));
                    }
                    at = Some(b);
                }
                Move::Rapid3 { x, y, z } => {
                    let a = at.unwrap_or([0.0; 3]);
                    at = Some([x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])]);
                }
                _ => {}
            }
        }
        (all, mesh.unwrap())
    }

    fn picks() -> Vec<Pickable> {
        pickables(&super::mesh(), Frame::Xyz { index_a: 0.0, sides: false })
    }

    fn applied(ty: &str, p: &Pickable) -> serde_json::Map<String, Value> {
        let cx = ApplyContext { stock_top: Some(0.5), tool_diameter: Some(2.0 * R), model: "part".into(), op: None };
        apply(ty, &[p], &cx).unwrap().0
    }

    /// The tool's disk at (x, y) lies on the oval flat (not on the slope, the grip or a hole).
    fn on_flat(x: f64, y: f64) -> bool {
        (0..24).all(|k| {
            let a = k as f64 * std::f64::consts::TAU / 24.0;
            let (px, py) = (x + 0.999 * R * a.cos(), y + 0.999 * R * a.sin());
            super::height(px, py) <= super::FLAT_Z + 1e-9
                && super::OVALS.iter().all(|&(c, _)| (px - c[0]).hypot(py - c[1]) >= super::HOLE_R - 1e-6)
        })
    }

    fn oval_face() -> Pickable {
        picks().into_iter().find(|p| p.kind == "flat_face" && p.data["z"].as_f64().unwrap() < -1.4).expect("an oval flat")
    }

    #[test]
    fn the_features_are_found() {
        let ps = picks();
        let n = |k: &str| ps.iter().filter(|p| p.kind == k).count();
        assert_eq!(ps.iter().filter(|p| p.kind == "flat_face" && (p.data["z"].as_f64().unwrap() + 1.5).abs() < 1e-6).count(), 2);
        assert_eq!(n("pocket"), 2, "recessed flats with sloped, textured sides are pockets");
        assert_eq!(n("outline"), 1);
        // (the holes are square on the grid: not round holes)
    }

    #[test]
    fn facing_a_picked_flat_stays_on_it() {
        let f = oval_face();
        let mut op = json!({"id": "op1", "type": "face", "tool": 2, "depth": 0.0, "step_down": 0.5, "stepover": 0.6, "feed": 800.0, "plunge_feed": 200.0});
        for (k, v) in applied("face", &f) {
            op[k] = v;
        }
        assert!((op["depth"].as_f64().unwrap() - 2.0).abs() < 1e-6, "{op}");
        let pts = cuts(op);
        assert!(pts.len() > 100);
        for p in &pts {
            assert!(p[2] >= super::FLAT_Z - 1e-6, "below the flat: {p:?}");
            assert!(on_flat(p[0], p[1]), "the tool leaves the flat at {p:?}");
        }
        // and it covers the flat: a point well inside it is passed over
        let c = super::OVALS[0].0;
        let near = |q: [f64; 2]| pts.iter().any(|p| (p[0] - q[0]).hypot(p[1] - q[1]) < R && (p[2] - super::FLAT_Z).abs() < 1e-6);
        assert!(near([c[0] - 6.0, c[1]]) || super::OVALS[1].0[0] > 0.0 && near([super::OVALS[1].0[0] - 6.0, c[1]]));
    }

    #[test]
    fn unbounded_facing_is_still_the_whole_top() {
        let pts = cuts(json!({"id": "op1", "type": "face", "tool": 2, "depth": 0.5, "step_down": 0.5, "stepover": 0.6, "feed": 800.0, "plunge_feed": 200.0}));
        let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
        assert!(lo < 0.0 && hi > 60.0, "{lo} {hi}");
    }

    #[test]
    fn a_pocket_on_a_picked_flat_stays_on_it() {
        let f = oval_face();
        let mut op = json!({"id": "op1", "type": "pocket", "tool": 2, "depth": 1.0, "step_down": 0.5, "stepover": 0.5, "feed": 800.0, "plunge_feed": 200.0});
        for (k, v) in applied("pocket", &f) {
            op[k] = v;
        }
        let pts = cuts(op);
        assert!(pts.len() > 100);
        for p in &pts {
            assert!(p[2] >= super::FLAT_Z - 1e-6 && on_flat(p[0], p[1]), "the pocket leaves the flat at {p:?}");
        }
    }

    #[test]
    fn the_outline_cuts_the_part_out() {
        let o = picks().into_iter().find(|p| p.kind == "outline").unwrap();
        // picked on a side wall: the outline is among that wall's features
        assert!(o.faces.len() > 2, "the bottom and the walls round it: {:?}", o.faces);
        let mut op = json!({"id": "op1", "type": "contour", "tool": 2, "step_down": 1.5, "feed": 800.0, "plunge_feed": 200.0});
        for (k, v) in applied("contour", &o) {
            op[k] = v;
        }
        assert_eq!(op["side"], "outside");
        let pts = cuts(op);
        let deepest = pts.iter().map(|p| p[2]).fold(f64::MAX, f64::min);
        assert!((deepest - (super::BOTTOM - marcam::pickables::BREAKTHROUGH)).abs() < 1e-6, "{deepest}");
        for p in &pts {
            // outside the 60 x 30 block by the tool's radius
            let dx = (-p[0]).max(p[0] - 60.0).max(0.0);
            let dy = (-p[1]).max(p[1] - 30.0).max(0.0);
            let d = if p[0] < 0.0 || p[0] > 60.0 || p[1] < 0.0 || p[1] > 30.0 { dx.hypot(dy) } else { -1.0 };
            assert!(d >= R - 1e-3, "the cut-out gouges the part at {p:?}");
        }
    }
}

/// Mar's own part (tests/local/oval_grip_real.stl, not in the repository: it is theirs), on a
/// vise moved by [2, 2, -3.699] as they set it up. Skipped when the file is not there.
#[cfg(test)]
mod real_part {
    use super::picked::cuts_in;
    use marcam::dropcutter::Cutter;
    use marcam::pickables::{apply, pickables, ApplyContext, Frame, Pickable};
    use marcam::strategy::three_axis::DropCutter3;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    const R: f64 = 3.175 / 2.0;

    fn dir() -> Option<PathBuf> {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/local");
        if d.join("oval_grip_real.stl").exists() {
            Some(d)
        } else {
            eprintln!("skipped: tests/local/oval_grip_real.stl is not here");
            None
        }
    }

    fn project() -> Value {
        json!({"schema_version": "0.2", "name": "real",
            "models": [{"id": "part", "name": "part", "kind": "mesh", "file": "oval_grip_real.stl", "translate": [2.0, 2.0, -3.699]}],
            "tools": [{"number": 2, "description": "3.175 mm flat", "kind": "flat", "diameter": 3.175, "flute_length": 12}],
            "setups": [{"id": "vise", "kind": "vise", "machine": "makera-carvera-4axis",
                "stock": {"shape": "box", "min": [0.0, 0.0, -4.0], "max": [105.85, 84.0, 0.0]},
                "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false}, "part": ["part"], "spindle_rpm": 12000,
                "operations": [], "outputs": []}]})
    }

    /// Plan `ty` pointed at `pick`; its cuts must not go into the part anywhere.
    fn plan(ty: &str, pick: &Pickable, base: Value) -> Vec<[f64; 3]> {
        let d = dir().unwrap();
        let cx = ApplyContext { stock_top: Some(0.0), tool_diameter: Some(2.0 * R), model: "part".into(), op: None };
        let mut op = base;
        for (k, v) in apply(ty, &[pick], &cx).unwrap().0 {
            op[k] = v;
        }
        let (pts, mesh) = cuts_in(project(), &d, op);
        assert!(pts.len() > 50, "{ty}: no cuts");
        // the offsets are found on a 0.05 mm grid (POCKET_FIELD_CELL): against a steep wall the
        // tool may be that much nearer than its radius
        let slack = marcam::strategy::three_axis::POCKET_FIELD_CELL + 0.01;
        let dc = DropCutter3::new(&mesh, Cutter::Flat { radius: R - slack });
        for p in &pts {
            let lowest = dc.z_at(p[0], p[1], f64::NEG_INFINITY);
            assert!(p[2] >= lowest - 0.005, "{ty} cuts into the part at {p:?} (the part is up to {lowest} there)");
        }
        pts
    }

    fn picks() -> Vec<Pickable> {
        let d = dir().unwrap();
        let libs = marcam::profiles::builtin_libraries();
        let mut p = project();
        p["setups"][0]["operations"] = json!([{"id": "op1", "type": "face", "tool": 2, "depth": 0.1, "step_down": 0.5, "stepover": 0.6, "feed": 800.0, "plunge_feed": 200.0}]);
        p["setups"][0]["outputs"] = json!([{"name": "x", "title": "x", "operations": ["op1"]}]);
        let jobs = marcam::project::jobs_from_any_with(&p.to_string(), &libs, &|src| src.load(&d).ok()).unwrap();
        let mesh = marcam::load_mesh(&jobs[0].1, &d).unwrap().unwrap();
        pickables(&mesh, Frame::Xyz { index_a: 0.0, sides: false })
    }

    #[test]
    fn facing_pocketing_and_cutting_out_marcs_part() {
        if dir().is_none() {
            return;
        }
        let ps = picks();
        let ovals: Vec<&Pickable> = ps.iter().filter(|p| p.kind == "flat_face" && (p.data["z"].as_f64().unwrap() + 1.5).abs() < 0.01 && p.data["area"].as_f64().unwrap() > 2800.0).collect();
        assert_eq!(ovals.len(), 2, "the two oval flats");
        let o = ovals[0];
        let outline: Vec<[f64; 2]> = serde_json::from_value(o.data["outline"].clone()).unwrap();
        let (lo, hi) = outline.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(a, b), q| ([a[0].min(q[0]), a[1].min(q[1])], [b[0].max(q[0]), b[1].max(q[1])]));
        let base = json!({"id": "op1", "tool": 2, "depth": 1.0, "step_down": 0.5, "stepover": 0.6, "feed": 800.0, "plunge_feed": 200.0});
        for ty in ["face", "pocket"] {
            let mut b = base.clone();
            b["type"] = json!(ty);
            let pts = plan(ty, o, b);
            for p in &pts {
                // on the oval only: within its outline's box (and past its open edges by the
                // tool's radius at most)
                assert!(p[0] >= lo[0] - R - 0.2 && p[0] <= hi[0] + R + 0.2 && p[1] >= lo[1] - R - 0.2 && p[1] <= hi[1] + R + 0.2, "{ty} leaves the oval at {p:?}");
                assert!(p[2] >= -1.5, "{ty} below the flat at {p:?}");
            }
            let floor = pts.iter().filter(|p| (p[2] + 1.499).abs() < 0.01).count();
            assert!(floor > 20, "{ty} reaches the flat");
        }
        let out = ps.iter().find(|p| p.kind == "outline").expect("the part's outline");
        let mut b = base.clone();
        b["type"] = json!("contour");
        b["step_down"] = json!(1.5);
        let pts = plan("contour", out, b);
        let deepest = pts.iter().map(|p| p[2]).fold(f64::MAX, f64::min);
        assert!((deepest - (-3.699 - marcam::pickables::BREAKTHROUGH)).abs() < 1e-3, "{deepest}");
    }
}
