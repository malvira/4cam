//! Geometry selection: face regions of a model (split at sharp edges, with ids that stay the
//! same when the triangles are reordered or the file is re-saved), and the selections an
//! operation can be limited to (whole model, faces, axial/angular ranges).
//!
//! Prototype semantics: a selection limits *where the tool goes* (a containment boundary in X
//! and A); the drop-cutter still checks against the whole model, so selecting faces never
//! gouges the faces next to them.

use crate::mesh::{cross, sub, Mesh, Triangle};
use serde::{Deserialize, Serialize};

/// A connected set of triangles bounded by sharp edges.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FaceRegion {
    pub id: String,
    pub triangles: usize,
    pub area: f64,
    /// Bounds in X (the rotary axis).
    pub x: [f64; 2],
    /// Angular extent (degrees, part angle) or None if it goes all the way round.
    pub a: Option<[f64; 2]>,
    /// Radius range.
    pub r: [f64; 2],
    /// "cylinder", "cone", "plane" (end face / shoulder), or "freeform".
    pub shape: String,
}

fn key(p: &[f64; 3]) -> (i64, i64, i64) {
    let q = |v: f64| (v * 1e4).round() as i64;
    (q(p[0]), q(p[1]), q(p[2]))
}

fn unit_normal(t: &Triangle) -> Option<[f64; 3]> {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (l > 1e-12).then(|| [n[0] / l, n[1] / l, n[2] / l])
}

fn area(t: &Triangle) -> f64 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

/// Split a mesh into regions at edges sharper than `crease_deg`. Returns the regions and, for
/// each triangle, its region index.
pub fn face_regions(mesh: &Mesh, crease_deg: f64) -> (Vec<FaceRegion>, Vec<usize>) {
    use std::collections::HashMap;
    let tris = &mesh.triangles;
    let normals: Vec<Option<[f64; 3]>> = tris.iter().map(unit_normal).collect();
    // edges keyed by welded vertex positions
    let mut edges: HashMap<((i64, i64, i64), (i64, i64, i64)), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (key(&t[e]), key(&t[(e + 1) % 3]));
            let k = if a < b { (a, b) } else { (b, a) };
            edges.entry(k).or_default().push(i);
        }
    }
    let cos_limit = crease_deg.to_radians().cos();
    // union-find over smooth edges
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for list in edges.values() {
        if list.len() != 2 {
            continue; // boundary or non-manifold: treat as sharp
        }
        let (a, b) = (list[0], list[1]);
        if let (Some(na), Some(nb)) = (normals[a], normals[b]) {
            if na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2] >= cos_limit {
                let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..tris.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut regions: Vec<(FaceRegion, Vec<usize>)> = groups
        .into_values()
        .map(|members| (describe(tris, &normals, &members), members))
        .collect();
    // deterministic order: by X then radius then id
    regions.sort_by(|a, b| {
        (a.0.x[0], a.0.r[0], &a.0.id)
            .partial_cmp(&(b.0.x[0], b.0.r[0], &b.0.id))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut of = vec![0; tris.len()];
    for (k, (_, members)) in regions.iter().enumerate() {
        for &i in members {
            of[i] = k;
        }
    }
    (regions.into_iter().map(|(r, _)| r).collect(), of)
}

fn describe(tris: &[Triangle], normals: &[Option<[f64; 3]>], members: &[usize]) -> FaceRegion {
    let (mut xlo, mut xhi, mut rlo, mut rhi) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, 0.0f64);
    let mut total = 0.0;
    let mut c = [0.0; 3];
    let mut angles = Vec::new();
    let mut nx_abs: f64 = 0.0;
    let mut n_min_x: f64 = 1.0;
    for &i in members {
        let t = &tris[i];
        let a = area(t);
        total += a;
        for p in t {
            xlo = xlo.min(p[0]);
            xhi = xhi.max(p[0]);
            let r = p[1].hypot(p[2]);
            rlo = rlo.min(r);
            rhi = rhi.max(r);
            if r > 1e-6 {
                angles.push(p[2].atan2(p[1]).to_degrees().rem_euclid(360.0));
            }
            for k in 0..3 {
                c[k] += a * p[k] / 3.0;
            }
        }
        if let Some(n) = normals[i] {
            nx_abs = nx_abs.max(n[0].abs());
            n_min_x = n_min_x.min(n[0].abs());
        }
    }
    // angular coverage: the complement of the largest gap between vertex angles
    angles.sort_by(|a, b| a.total_cmp(b));
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let a_range = if angles.len() < 2 {
        None
    } else {
        let mut gap = (
            angles[0] + 360.0 - angles[angles.len() - 1],
            angles.len() - 1,
        );
        for k in 1..angles.len() {
            let g = angles[k] - angles[k - 1];
            if g > gap.0 {
                gap = (g, k - 1);
            }
        }
        if gap.0 < 5.0 {
            None // goes all the way round
        } else {
            let start = angles[(gap.1 + 1) % angles.len()];
            Some([start, start + 360.0 - gap.0])
        }
    };
    let shape = if n_min_x > 0.999 {
        "plane"
    } else if nx_abs < 0.01 {
        "cylinder"
    } else if (nx_abs - n_min_x) < 0.01 {
        "cone"
    } else {
        "freeform"
    };
    // stable id: from size and position only (not triangle order or count)
    let q = |v: f64| (v * 100.0).round() as i64;
    let sig = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        q(total),
        q(c[0] / total.max(1e-12)),
        q(xlo),
        q(xhi),
        q(rlo),
        q(rhi),
        shape
    );
    let mut h: u64 = 0xcbf29ce484222325;
    for b in sig.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    FaceRegion {
        id: format!("f{:08x}", (h >> 32) as u32 ^ h as u32),
        triangles: members.len(),
        area: total,
        x: [xlo, xhi],
        a: a_range,
        r: [rlo, rhi],
        shape: shape.into(),
    }
}

/// What an operation is limited to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    /// Everything (the default).
    WholeModel,
    /// Face regions of a model, by id (see [`face_regions`]).
    Faces { model: String, ids: Vec<String> },
    /// Only where the tool tip is between these X positions.
    AxialRange { x: [f64; 2] },
    /// Only between these A angles (degrees).
    AngularRange { a: [f64; 2] },
}

/// Containment boundary for the tool: X and A ranges (None = unlimited).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Boundary {
    pub x: Option<[f64; 2]>,
    pub a: Option<[f64; 2]>,
}

fn intersect(a: Option<[f64; 2]>, b: [f64; 2]) -> Option<[f64; 2]> {
    let b = [b[0].min(b[1]), b[0].max(b[1])];
    Some(match a {
        None => b,
        Some(a) => [a[0].max(b[0]), a[1].min(b[1])],
    })
}

/// Combine selections into one boundary. Face selections use the selected regions' X and
/// angular extents, grown by `tool_radius` in X so the tool can reach their edges.
pub fn boundary(
    sel: &[Selection],
    regions_of: &dyn Fn(&str) -> Option<Vec<FaceRegion>>,
    tool_radius: f64,
) -> Result<Boundary, String> {
    let mut b = Boundary::default();
    for s in sel {
        match s {
            Selection::WholeModel => {}
            Selection::AxialRange { x } => b.x = intersect(b.x, *x),
            Selection::AngularRange { a } => b.a = intersect(b.a, *a),
            Selection::Faces { model, ids } => {
                let regions =
                    regions_of(model).ok_or(format!("model {model:?} has no faces to select"))?;
                let mut xr: Option<[f64; 2]> = None;
                let mut ar: Option<[f64; 2]> = None;
                let mut full_turn = false;
                for id in ids {
                    let r = regions
                        .iter()
                        .find(|r| &r.id == id)
                        .ok_or(format!("model {model:?} has no face {id:?}"))?;
                    xr = Some(xr.map_or(r.x, |x| [x[0].min(r.x[0]), x[1].max(r.x[1])]));
                    match r.a {
                        None => full_turn = true,
                        Some(a) => ar = Some(ar.map_or(a, |p| [p[0].min(a[0]), p[1].max(a[1])])),
                    }
                }
                if let Some(x) = xr {
                    b.x = intersect(b.x, [x[0] - tool_radius, x[1] + tool_radius]);
                }
                if let (false, Some(a)) = (full_turn, ar) {
                    b.a = intersect(b.a, a);
                }
            }
        }
    }
    if let Some([lo, hi]) = b.x {
        if lo > hi {
            return Err(format!(
                "the selections leave no X range ({lo:.3} > {hi:.3})"
            ));
        }
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Profile;

    fn knob() -> Mesh {
        let p: crate::job::Job =
            serde_json::from_str(include_str!("../../../examples/knob/knob.job.json")).unwrap();
        let prof: Profile = p.part.profile.unwrap();
        Mesh::revolve(&prof, 360)
    }

    #[test]
    fn knob_splits_into_its_faces() {
        let (regions, of) = face_regions(&knob(), 30.0);
        let shapes: Vec<(&str, f64, f64)> = regions
            .iter()
            .map(|r| (r.shape.as_str(), r.x[0], r.x[1]))
            .collect();
        // back cap, neck, step, boss, shoulder, taper, chamfer, front cap
        assert_eq!(regions.len(), 8, "{shapes:?}");
        assert_eq!(of.len(), knob().triangles.len());
        let taper = regions
            .iter()
            .find(|r| r.shape == "cone" && (r.x[0] + 15.0).abs() < 1e-6)
            .unwrap();
        assert!((taper.x[1] + 0.8).abs() < 1e-6 && taper.a.is_none());
        assert_eq!(
            regions.iter().filter(|r| r.shape == "plane").count(),
            4,
            "{shapes:?}"
        );
        assert_eq!(
            regions.iter().filter(|r| r.shape == "cylinder").count(),
            2,
            "{shapes:?}"
        );
    }

    #[test]
    fn ids_survive_reordering_and_stl_round_trip() {
        let m = knob();
        let (a, _) = face_regions(&m, 30.0);
        let mut shuffled = m.clone();
        shuffled.triangles.reverse();
        for t in shuffled.triangles.iter_mut() {
            t.rotate_left(1);
        }
        let (b, _) = face_regions(&shuffled, 30.0);
        let (c, _) = face_regions(&Mesh::from_bytes(&m.to_stl()).unwrap(), 30.0);
        let ids = |r: &[FaceRegion]| r.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(ids(&a), ids(&c));
        let mut uniq = ids(&a);
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), a.len());
    }

    #[test]
    fn boundaries_combine() {
        let (regions, _) = face_regions(&knob(), 30.0);
        let taper = regions
            .iter()
            .find(|r| r.shape == "cone" && (r.x[0] + 15.0).abs() < 1e-6)
            .unwrap();
        let get = |m: &str| (m == "profile").then(|| regions.clone());
        let b = boundary(
            &[Selection::Faces {
                model: "profile".into(),
                ids: vec![taper.id.clone()],
            }],
            &get,
            1.5,
        )
        .unwrap();
        assert_eq!(b.x, Some([-16.5, 0.7]));
        assert_eq!(b.a, None);
        let b = boundary(
            &[
                Selection::Faces {
                    model: "profile".into(),
                    ids: vec![taper.id.clone()],
                },
                Selection::AxialRange { x: [-10.0, 5.0] },
                Selection::AngularRange { a: [0.0, 90.0] },
            ],
            &get,
            1.5,
        )
        .unwrap();
        assert_eq!(b.x, Some([-10.0, 0.7]));
        assert_eq!(b.a, Some([0.0, 90.0]));
        assert!(boundary(
            &[Selection::Faces {
                model: "profile".into(),
                ids: vec!["nope".into()]
            }],
            &get,
            1.0
        )
        .is_err());
        assert!(boundary(
            &[
                Selection::AxialRange { x: [0.0, 1.0] },
                Selection::AxialRange { x: [2.0, 3.0] }
            ],
            &get,
            1.0
        )
        .is_err());
    }
}
