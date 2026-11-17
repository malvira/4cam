//! Pickables: the features of a part an operation can be pointed at, recognised from its
//! mesh. One list serves every operation (holes for drilling, pockets for pocketing, edge
//! loops for contours, faces and surfaces for finishing, bands for rotary work); each
//! operation type accepts some kinds and fills its parameters from them ([`accepts`],
//! [`apply`]).
//!
//! Recognition works on the face regions of [`crate::geometry::face_regions`] (the same ids
//! the `faces` selections use), in the setup's frame:
//! - vise and indexed 3+1 (Z up, the part turned to the operation's A index): flat faces,
//!   holes (concave vertical cylinders, through or blind), pockets (up-facing floors walled
//!   all round, with islands), the edge loops of flat faces, and every other region as a
//!   surface;
//! - rotary (the part's axis is X): cylindrical bands (round all the way, constant radius),
//!   revolved bands (cones, shoulders, other surfaces of revolution) and surfaces (the rest).

use crate::geometry::{face_regions, FaceRegion};
use crate::mesh::{cross, sub, Mesh, Triangle};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The frame the part is recognised in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// Z up (vise; indexed 3+1 with the part turned to `index_a` degrees about X).
    Xyz { index_a: f64 },
    /// The rotary axis along X.
    Rotary,
}

/// A feature to pick.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pickable {
    /// Stable within a part: the kind and the face region(s) it comes from.
    pub id: String,
    /// "hole", "pocket", "flat_face", "edge_loop", "surface", "cylindrical_band",
    /// "revolved_band".
    pub kind: String,
    /// What a person reads: "Hole 5.00 mm through, 12.00 deep".
    pub label: String,
    /// The face regions it covers (what lights up).
    pub faces: Vec<String>,
    /// Equal for features alike enough to pick together ("select all similar").
    pub similar: String,
    /// Kind-specific numbers (mm, degrees), see the recognisers.
    pub data: Value,
}

const FLAT: f64 = 0.999; // |nz| of a flat face
const WALL: f64 = 0.02; // |nz| of a vertical wall
const Q: f64 = 1e4; // vertex welding (0.1 micron)

type Key = (i64, i64, i64);
fn key(p: &[f64; 3]) -> Key {
    (
        (p[0] * Q).round() as i64,
        (p[1] * Q).round() as i64,
        (p[2] * Q).round() as i64,
    )
}

fn unit_normal(t: &Triangle) -> [f64; 3] {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-300);
    [n[0] / l, n[1] / l, n[2] / l]
}

fn tri_area(t: &Triangle) -> f64 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// One region's triangles and numbers.
struct Region {
    info: FaceRegion,
    tris: Vec<usize>,
    area: f64,
    nz: [f64; 2],
    z: [f64; 2],
}

/// Recognise the pickable features of `mesh` (in the setup's frame).
pub fn pickables(mesh: &Mesh, frame: Frame) -> Vec<Pickable> {
    let (infos, of) = face_regions(mesh, 30.0);
    match frame {
        Frame::Rotary => rotary(&infos),
        Frame::Xyz { index_a } => {
            let turned;
            let m = if index_a != 0.0 {
                turned = mesh.rotated_x(index_a);
                &turned
            } else {
                mesh
            };
            xyz(m, infos, &of)
        }
    }
}

fn rotary(infos: &[FaceRegion]) -> Vec<Pickable> {
    infos
        .iter()
        .map(|r| {
            let full = r.a.is_none();
            let (kind, label) = if full && r.shape == "cylinder" {
                (
                    "cylindrical_band",
                    format!(
                        "Cylinder {:.2} mm, X{:.2}..{:.2}",
                        2.0 * r.r[1],
                        r.x[0],
                        r.x[1]
                    ),
                )
            } else if full {
                (
                    "revolved_band",
                    format!(
                        "{} X{:.2}..{:.2}, radius {:.2}..{:.2}",
                        match r.shape.as_str() {
                            "cone" => "Cone",
                            "plane" => "Shoulder",
                            _ => "Revolved surface",
                        },
                        r.x[0],
                        r.x[1],
                        r.r[0],
                        r.r[1]
                    ),
                )
            } else {
                (
                    "surface",
                    format!("Surface X{:.2}..{:.2}, {:.1} mm2", r.x[0], r.x[1], r.area),
                )
            };
            let similar = match kind {
                "cylindrical_band" => format!("cylindrical_band:{}", r2(r.r[1])),
                "revolved_band" => {
                    format!("revolved_band:{}:{}:{}", r.shape, r2(r.r[0]), r2(r.r[1]))
                }
                _ => format!("surface:{}", r.id),
            };
            Pickable {
                id: format!("{kind}:{}", r.id),
                kind: kind.into(),
                label,
                faces: vec![r.id.clone()],
                similar,
                data: json!({"x": r.x, "r": r.r, "a": r.a, "shape": r.shape, "area": r.area}),
            }
        })
        .collect()
}

/// Closed loops of the edges in `edges` (directed, as the triangles run): their vertices.
fn loops(edges: &[(Key, Key)]) -> Vec<Vec<Key>> {
    let mut next: HashMap<Key, Vec<Key>> = HashMap::new();
    for &(a, b) in edges {
        next.entry(a).or_default().push(b);
    }
    let mut used: HashMap<(Key, Key), bool> = HashMap::new();
    let mut out = Vec::new();
    for &(a0, b0) in edges {
        if used.contains_key(&(a0, b0)) {
            continue;
        }
        let mut lp = vec![a0];
        let (mut a, mut b) = (a0, b0);
        used.insert((a, b), true);
        let mut guard = 0;
        while b != a0 && guard < 1_000_000 {
            guard += 1;
            lp.push(b);
            let Some(c) = next
                .get(&b)
                .and_then(|v| v.iter().find(|c| !used.contains_key(&(b, **c))))
            else {
                break;
            };
            a = b;
            b = *c;
            used.insert((a, b), true);
        }
        if b == a0 && lp.len() >= 3 {
            out.push(lp);
        }
    }
    out
}

fn signed_area(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f64>()
        / 2.0
}

fn length(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| (p[(i + 1) % n][0] - p[i][0]).hypot(p[(i + 1) % n][1] - p[i][1]))
        .sum()
}

/// Least-squares circle through XY points: (centre, radius, largest misfit).
fn fit_circle(pts: &[[f64; 2]]) -> Option<([f64; 2], f64, f64)> {
    if pts.len() < 6 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = (
        pts.iter().map(|p| p[0]).sum::<f64>() / n,
        pts.iter().map(|p| p[1]).sum::<f64>() / n,
    );
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in pts {
        let (u, v) = (p[0] - mx, p[1] - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let (b1, b2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let uc = (b1 * svv - b2 * suv) / det;
    let vc = (suu * b2 - suv * b1) / det;
    let c = [mx + uc, my + vc];
    let r = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    let misfit = pts
        .iter()
        .map(|p| ((p[0] - c[0]).hypot(p[1] - c[1]) - r).abs())
        .fold(0.0, f64::max);
    Some((c, r, misfit))
}

/// Largest gap between the points' angles about `c` (degrees).
fn largest_gap(pts: &[[f64; 2]], c: [f64; 2]) -> f64 {
    let mut a: Vec<f64> = pts
        .iter()
        .map(|p| {
            (p[1] - c[1])
                .atan2(p[0] - c[0])
                .to_degrees()
                .rem_euclid(360.0)
        })
        .collect();
    a.sort_by(|x, y| x.total_cmp(y));
    a.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    if a.len() < 2 {
        return 360.0;
    }
    let mut g = a[0] + 360.0 - a[a.len() - 1];
    for k in 1..a.len() {
        g = g.max(a[k] - a[k - 1]);
    }
    g
}

fn xyz(mesh: &Mesh, infos: Vec<FaceRegion>, of: &[usize]) -> Vec<Pickable> {
    let tris = &mesh.triangles;
    let mut regions: Vec<Region> = infos
        .into_iter()
        .map(|info| Region {
            info,
            tris: Vec::new(),
            area: 0.0,
            nz: [f64::INFINITY, f64::NEG_INFINITY],
            z: [f64::INFINITY, f64::NEG_INFINITY],
        })
        .collect();
    let mut pos: HashMap<Key, [f64; 3]> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        let r = &mut regions[of[i]];
        r.tris.push(i);
        let a = tri_area(t);
        r.area += a;
        if a > 1e-12 {
            let n = unit_normal(t);
            r.nz = [r.nz[0].min(n[2]), r.nz[1].max(n[2])];
        }
        for p in t {
            r.z = [r.z[0].min(p[2]), r.z[1].max(p[2])];
            pos.insert(key(p), *p);
        }
    }
    let zmin = mesh.bounds().map_or(0.0, |(lo, _)| lo[2]);
    // directed edges -> triangle, to find each boundary edge's neighbour across it
    let mut edge_tri: HashMap<(Key, Key), usize> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for k in 0..3 {
            edge_tri.insert((key(&t[k]), key(&t[(k + 1) % 3])), i);
        }
    }
    let is_up = |r: &Region| r.nz[0] > FLAT;
    let is_down = |r: &Region| r.nz[1] < -FLAT;
    let is_wall = |r: &Region| r.nz[0] > -WALL && r.nz[1] < WALL;
    let mut out = Vec::new();

    // holes: concave vertical cylinders
    let mut hole_regions: Vec<(usize, [f64; 2], f64)> = Vec::new();
    for (k, r) in regions.iter().enumerate() {
        if !is_wall(r) || r.area < 1e-6 {
            continue;
        }
        let pts: Vec<[f64; 2]> = r
            .tris
            .iter()
            .flat_map(|&i| tris[i].iter().map(|p| [p[0], p[1]]))
            .collect();
        let Some((c, rad, misfit)) = fit_circle(&pts) else {
            continue;
        };
        if misfit > (0.01 * rad).max(0.02) || largest_gap(&pts, c) > 30.0 {
            continue;
        }
        // concave: the walls face the centre
        let inward = r
            .tris
            .iter()
            .map(|&i| {
                let t = &tris[i];
                let n = unit_normal(t);
                let m = [
                    (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                    (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                ];
                let d = [c[0] - m[0], c[1] - m[1]];
                let l = d[0].hypot(d[1]).max(1e-12);
                tri_area(t) * (n[0] * d[0] + n[1] * d[1]) / l
            })
            .sum::<f64>()
            / r.area;
        if inward < 0.9 {
            continue;
        }
        hole_regions.push((k, c, rad));
    }
    let mut floor_of_hole: HashMap<usize, usize> = HashMap::new();
    for &(k, c, rad) in &hole_regions {
        let r = &regions[k];
        let (top, bottom) = (r.z[1], r.z[0]);
        // a flat floor at the bottom inside the circle: blind
        let floor = regions.iter().position(|f| {
            is_up(f)
                && (f.z[0] - bottom).abs() < 1e-3
                && f.tris.iter().all(|&i| {
                    tris[i]
                        .iter()
                        .all(|p| (p[0] - c[0]).hypot(p[1] - c[1]) <= rad * 1.01 + 1e-3)
                })
        });
        // closed at the top by a down-facing face: it opens away from the spindle
        let capped = regions.iter().any(|f| {
            is_down(f)
                && (f.z[1] - top).abs() < 1e-3
                && f.tris.iter().all(|&i| {
                    tris[i]
                        .iter()
                        .all(|p| (p[0] - c[0]).hypot(p[1] - c[1]) <= rad * 1.01 + 1e-3)
                })
        });
        if capped {
            continue;
        }
        let through = floor.is_none() && (bottom - zmin).abs() < 1e-3;
        if let Some(f) = floor {
            floor_of_hole.insert(f, k);
        }
        let depth = top - bottom;
        let d = 2.0 * rad;
        let mut faces = vec![r.info.id.clone()];
        if let Some(f) = floor {
            faces.push(regions[f].info.id.clone());
        }
        out.push(Pickable {
            id: format!("hole:{}", r.info.id),
            kind: "hole".into(),
            label: format!(
                "Hole {d:.2} mm {}, {depth:.2} deep",
                if through { "through" } else { "blind" }
            ),
            faces,
            similar: format!("hole:{}:{}:{}", r2(d), through, r2(depth)),
            data: json!({
                "center": [round_um(c[0]), round_um(c[1])], "diameter": round_um(d), "top": top, "bottom": bottom,
                "depth": depth, "through": through
            }),
        });
    }
    let hole_set: std::collections::HashSet<usize> = hole_regions.iter().map(|h| h.0).collect();

    // flat faces, their edge loops, and pockets
    for (k, r) in regions.iter().enumerate() {
        let up = is_up(r);
        if !(up || is_down(r)) {
            continue;
        }
        let z = if up { r.z[1] } else { r.z[0] };
        let mine: std::collections::HashSet<usize> = r.tris.iter().copied().collect();
        // boundary edges: the triangle across belongs to another region
        let mut bedges = Vec::new();
        let mut walls_up = true;
        let mut wall_regions: Vec<usize> = Vec::new();
        // the top of the wall across each boundary edge, by the edge's start vertex
        let mut wall_top: HashMap<Key, f64> = HashMap::new();
        for &i in &r.tris {
            let t = &tris[i];
            for e in 0..3 {
                let (a, b) = (key(&t[e]), key(&t[(e + 1) % 3]));
                let across = edge_tri.get(&(b, a)).copied();
                if across.is_some_and(|j| mine.contains(&j)) {
                    continue;
                }
                bedges.push((a, b));
                match across {
                    Some(j) => {
                        let wr = of[j];
                        let tz = tris[j]
                            .iter()
                            .map(|p| p[2])
                            .fold(f64::NEG_INFINITY, f64::max);
                        if !(is_wall(&regions[wr]) && tz > z + 1e-3) {
                            walls_up = false;
                        } else {
                            wall_top.insert(a, regions[wr].z[1]);
                            if !wall_regions.contains(&wr) {
                                wall_regions.push(wr);
                            }
                        }
                    }
                    None => walls_up = false,
                }
            }
        }
        let key_loops = loops(&bedges);
        let xy = |l: &Vec<Key>| -> Vec<[f64; 2]> {
            l.iter()
                .map(|k| {
                    let p = pos[k];
                    [p[0], p[1]]
                })
                .collect()
        };
        let mut order: Vec<usize> = (0..key_loops.len()).collect();
        order.sort_by(|&a, &b| {
            signed_area(&xy(&key_loops[b]))
                .abs()
                .total_cmp(&signed_area(&xy(&key_loops[a])).abs())
        });
        // a pocket's rim: the lowest top of the walls round its outline (not its islands')
        let rim = order.first().map_or(f64::INFINITY, |&o| {
            key_loops[o]
                .iter()
                .filter_map(|k| wall_top.get(k).copied())
                .fold(f64::INFINITY, f64::min)
        });
        let mut lps: Vec<Vec<[f64; 2]>> = order.iter().map(|&o| xy(&key_loops[o])).collect();
        // outer loop first (largest area); for an up face, outer loops run counter-clockwise
        if !up {
            for l in &mut lps {
                l.reverse();
            }
        }
        let id = &r.info.id;
        if up && !floor_of_hole.contains_key(&k) {
            out.push(Pickable {
                id: format!("flat_face:{id}"),
                kind: "flat_face".into(),
                label: format!("Flat face at Z{z:.2}, {:.1} mm2", r.area),
                faces: vec![id.clone()],
                similar: format!("flat_face:{}", r2(z)),
                data: json!({"z": z, "area": r.area, "loops": lps.len()}),
            });
            if walls_up && !lps.is_empty() && !wall_regions.iter().any(|w| hole_set.contains(w)) {
                let outline = lps[0].clone();
                let islands: Vec<Vec<[f64; 2]>> = lps[1..].to_vec();
                let depth = rim - z;
                let (lo, hi) = bbox(&outline);
                let mut faces = vec![id.clone()];
                faces.extend(wall_regions.iter().map(|&w| regions[w].info.id.clone()));
                out.push(Pickable {
                    id: format!("pocket:{id}"),
                    kind: "pocket".into(),
                    label: format!(
                        "Pocket {:.2} x {:.2}, {depth:.2} deep{}",
                        hi[0] - lo[0],
                        hi[1] - lo[1],
                        match islands.len() {
                            0 => String::new(),
                            1 => ", 1 island".into(),
                            n => format!(", {n} islands"),
                        }
                    ),
                    faces,
                    similar: format!(
                        "pocket:{}:{}:{}:{}",
                        r2(hi[0] - lo[0]),
                        r2(hi[1] - lo[1]),
                        r2(depth),
                        islands.len()
                    ),
                    data: json!({
                        "floor": z, "top": rim, "depth": depth, "outline": close(&outline),
                        "islands": islands.iter().map(|l| close(l)).collect::<Vec<_>>()
                    }),
                });
            }
        }
        if floor_of_hole.contains_key(&k) {
            continue;
        }
        // edge loops: the outline of each flat face, and the openings in up faces
        for (n, l) in lps.iter().enumerate() {
            if !up && n > 0 {
                continue; // the bottom's holes are the holes' loops
            }
            let outer = n == 0;
            let side = if outer { "outside" } else { "inside" };
            let len = length(l);
            out.push(Pickable {
                id: format!("edge_loop:{id}:{n}"),
                kind: "edge_loop".into(),
                label: format!(
                    "Edge loop at Z{z:.2} ({}, {len:.1} mm)",
                    if outer { "outline" } else { "opening" }
                ),
                faces: vec![id.clone()],
                similar: format!("edge_loop:{}:{}:{}", r2(z), side, r2(len)),
                data: json!({"z": z, "path": close(l), "side": side, "length": len, "up": up}),
            });
        }
    }
    // every other region: a surface
    for r in &regions {
        let used = out.iter().any(|p| p.faces.contains(&r.info.id));
        if used || r.area < 1e-6 {
            continue;
        }
        out.push(Pickable {
            id: format!("surface:{}", r.info.id),
            kind: "surface".into(),
            label: format!("Surface Z{:.2}..{:.2}, {:.1} mm2", r.z[0], r.z[1], r.area),
            faces: vec![r.info.id.clone()],
            similar: format!("surface:{}", r.info.id),
            data: json!({"z": r.z, "area": r.area}),
        });
    }
    out.sort_by(|a, b| (kind_order(&a.kind), &a.id).cmp(&(kind_order(&b.kind), &b.id)));
    out
}

fn kind_order(k: &str) -> usize {
    ["hole", "pocket", "flat_face", "edge_loop", "surface"]
        .iter()
        .position(|x| *x == k)
        .unwrap_or(9)
}

fn bbox(p: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for q in p {
        for k in 0..2 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    (lo, hi)
}

/// The loop with its first point repeated at the end (as the operations take paths).
fn close(l: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut v: Vec<[f64; 2]> = l.iter().map(|p| [round_um(p[0]), round_um(p[1])]).collect();
    if let Some(&f) = v.first() {
        v.push(f);
    }
    v
}

fn round_um(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// A setup's part, as a mesh in the setup's frame, and the frame to recognise it in: the
/// model asked for, else the setup's mesh model, else its first model (a profile is
/// revolved). Vise and indexed setups are Z up (indexed: turned to `index_a`), rotary ones
/// along X. `mesh_of` reads a mesh model's file. Returns the setup id too.
pub fn setup_part(
    p: &crate::project::Project,
    setup: Option<&str>,
    model: Option<&str>,
    index_a: f64,
    mesh_of: &dyn Fn(&crate::job::MeshSource) -> Result<Mesh, String>,
) -> Result<(String, Mesh, Frame), String> {
    use crate::project::{ModelKind, StockDef};
    let s = match setup {
        Some(id) => p
            .setups
            .iter()
            .find(|s| s.id == id)
            .ok_or(format!("no setup {id:?}"))?,
        None => p.setups.first().ok_or("the project has no setups")?,
    };
    let is_mesh = |id: &String| {
        p.models
            .iter()
            .any(|m| &m.id == id && matches!(m.kind, ModelKind::Mesh(_)))
    };
    let model_id = match model {
        Some(m) => m.to_string(),
        None => s
            .part
            .iter()
            .find(|id| is_mesh(id))
            .or(s.part.first())
            .cloned()
            .ok_or("the setup has no part model")?,
    };
    let m = p
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or(format!("no model {model_id:?}"))?;
    let part = match &m.kind {
        ModelKind::Profile(prof) => Mesh::revolve(prof, 360),
        ModelKind::Mesh(src) => mesh_of(src)?,
    };
    let block = matches!(s.stock, StockDef::Box { .. });
    let kind = s.kind.clone().unwrap_or_else(|| {
        if block {
            "vise".into()
        } else {
            "rotary".into()
        }
    });
    let frame = if kind == "rotary" {
        Frame::Rotary
    } else {
        Frame::Xyz { index_a }
    };
    Ok((s.id.clone(), part, frame))
}

/// The top of a setup's stock in the recognition frame (block: its top; bar: its radius).
pub fn stock_top(p: &crate::project::Project, setup: &str) -> Option<f64> {
    use crate::project::StockDef;
    let s = p.setups.iter().find(|s| s.id == setup)?;
    Some(match s.stock {
        StockDef::Box { max, .. } => max[2],
        StockDef::Cylinder { radius, .. } | StockDef::FromPrevious { radius, .. } => radius,
    })
}

// ------------------------------------------------------------------ operations

/// The kinds an operation type takes, in order of preference (empty: picking does not apply).
pub fn accepts(op_type: &str) -> &'static [&'static str] {
    match op_type {
        "drill" => &["hole"],
        "pocket" => &["pocket"],
        "contour" => &["edge_loop"],
        "face" => &["flat_face"],
        "raster3d" | "rough3d" => &["surface", "flat_face", "pocket", "hole"],
        "rotary_finish" => &["cylindrical_band", "revolved_band", "surface"],
        "rotary_rough" => &["cylindrical_band", "revolved_band", "surface"],
        _ => &[],
    }
}

/// What a pick needs to know about the setup.
#[derive(Debug, Clone)]
pub struct ApplyContext {
    /// Top of the stock (Z, 3-axis).
    pub stock_top: Option<f64>,
    /// Diameter of the operation's tool (mm).
    pub tool_diameter: Option<f64>,
    /// The part model the face ids belong to.
    pub model: String,
}

/// The parameters `picks` set on an operation of type `op_type`, and notes for the person
/// (a suggestion, a mismatch). Errors when a pick is of a kind the type does not take.
pub fn apply(
    op_type: &str,
    picks: &[&Pickable],
    cx: &ApplyContext,
) -> Result<(Map<String, Value>, Vec<String>), String> {
    let ok = accepts(op_type);
    if picks.is_empty() {
        return Err("nothing picked".into());
    }
    for p in picks {
        if !ok.contains(&p.kind.as_str()) {
            return Err(format!(
                "{op_type} does not take a {} (it takes {})",
                p.kind.replace('_', " "),
                if ok.is_empty() {
                    "no picks".to_string()
                } else {
                    ok.join(", ").replace('_', " ")
                }
            ));
        }
    }
    let f = |p: &Pickable, k: &str| p.data[k].as_f64().unwrap_or(0.0);
    let mut v = Map::new();
    let mut notes = Vec::new();
    match op_type {
        "drill" => {
            let points: Vec<Value> = picks.iter().map(|p| p.data["center"].clone()).collect();
            let top = picks
                .iter()
                .map(|p| f(p, "top"))
                .fold(f64::NEG_INFINITY, f64::max);
            // through holes: 0.5 mm past the bottom
            let depth = picks
                .iter()
                .map(|p| top - f(p, "bottom") + if p.data["through"] == true { 0.5 } else { 0.0 })
                .fold(0.0, f64::max);
            let d = picks.iter().map(|p| f(p, "diameter")).fold(0.0, f64::max);
            v.insert("points".into(), Value::Array(points));
            v.insert("top".into(), json!(round_um(top)));
            v.insert("depth".into(), json!(round_um(depth)));
            // a peck of about one diameter, at least 0.5 mm
            v.insert("peck".into(), json!(round_um(d.max(0.5))));
            if let Some(td) = cx.tool_diameter {
                let wrong: Vec<f64> = picks
                    .iter()
                    .map(|p| f(p, "diameter"))
                    .filter(|hd| (hd - td).abs() > 0.05)
                    .collect();
                if !wrong.is_empty() {
                    notes.push(format!(
                        "the {td:.2} mm tool drills {td:.2} mm holes; {} {} (use Bore instead)",
                        if wrong.len() == 1 {
                            "this hole is"
                        } else {
                            "these holes are"
                        },
                        wrong
                            .iter()
                            .map(|d| format!("{d:.2} mm"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if picks.iter().any(|p| (f(p, "top") - top).abs() > 1e-3) {
                notes.push(
                    "the holes start at different heights: all are drilled from the highest".into(),
                );
            }
        }
        "pocket" => {
            let p = picks[0];
            if picks.len() > 1 {
                notes.push("a pocket operation clears one pocket: the first one picked".into());
            }
            let mut paths = vec![p.data["outline"].clone()];
            if let Some(is) = p.data["islands"].as_array() {
                paths.extend(is.iter().cloned());
            }
            v.insert("paths".into(), Value::Array(paths));
            v.insert("top".into(), json!(round_um(f(p, "top"))));
            v.insert("depth".into(), json!(round_um(f(p, "depth"))));
        }
        "contour" => {
            let p = picks[0];
            if picks.len() > 1 {
                notes.push("a contour follows one loop: the first one picked".into());
            }
            let top = cx.stock_top.unwrap_or(f(p, "z"));
            v.insert("path".into(), p.data["path"].clone());
            v.insert("side".into(), p.data["side"].clone());
            v.insert("top".into(), json!(round_um(top)));
            v.insert("depth".into(), json!(round_um((top - f(p, "z")).max(0.01))));
        }
        "face" => {
            let p = picks[0];
            let top = cx.stock_top.unwrap_or(f(p, "z"));
            v.insert("depth".into(), json!(round_um((top - f(p, "z")).max(0.01))));
        }
        _ => {
            // finishing: the faces, and for rotary roughing the X span they cover
            let mut ids: Vec<String> = Vec::new();
            for p in picks {
                for id in &p.faces {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
            }
            if op_type == "rotary_rough" {
                let lo = picks
                    .iter()
                    .map(|p| p.data["x"][0].as_f64().unwrap_or(0.0))
                    .fold(f64::INFINITY, f64::min);
                let hi = picks
                    .iter()
                    .map(|p| p.data["x"][1].as_f64().unwrap_or(0.0))
                    .fold(f64::NEG_INFINITY, f64::max);
                v.insert("x_range".into(), json!([round_um(lo), round_um(hi)]));
            } else {
                v.insert("faces".into(), json!(ids));
                let _ = &cx.model;
            }
        }
    }
    Ok((v, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_circle_fits_exactly() {
        let pts: Vec<[f64; 2]> = (0..32)
            .map(|k| {
                let t = k as f64 * std::f64::consts::TAU / 32.0;
                [3.0 + 2.5 * t.cos(), -1.0 + 2.5 * t.sin()]
            })
            .collect();
        let (c, r, misfit) = fit_circle(&pts).unwrap();
        assert!(
            (c[0] - 3.0).abs() < 1e-9
                && (c[1] + 1.0).abs() < 1e-9
                && (r - 2.5).abs() < 1e-9
                && misfit < 1e-9
        );
        assert!(largest_gap(&pts, c) < 12.0);
    }
}
