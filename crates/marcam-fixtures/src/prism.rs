//! Prismatic (2.5D) test parts: flat faces at heights, vertical walls between them. Polygons
//! with holes are triangulated by ear clipping after bridging each hole to the outline.

use marcam::Mesh;

pub type P2 = [f64; 2];

/// Signed area (counter-clockwise positive).
pub fn area(p: &[P2]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// A circle of `n` points, counter-clockwise, starting at angle 0.
pub fn circle(c: P2, r: f64, n: usize) -> Vec<P2> {
    (0..n)
        .map(|k| {
            let t = k as f64 * std::f64::consts::TAU / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

fn cross(o: P2, a: P2, b: P2) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn inside_tri(p: P2, a: P2, b: P2, c: P2) -> bool {
    cross(a, b, p) >= -1e-12 && cross(b, c, p) >= -1e-12 && cross(c, a, p) >= -1e-12
}

fn segments_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

/// Triangulate `outer` (counter-clockwise) minus `holes` (any winding). Triangles are
/// counter-clockwise.
pub fn triangulate(outer: &[P2], holes: &[Vec<P2>]) -> Vec<[P2; 3]> {
    let mut poly: Vec<P2> = outer.to_vec();
    if area(&poly) < 0.0 {
        poly.reverse();
    }
    // holes clockwise, bridged in order of their rightmost point (largest first)
    let mut hs: Vec<Vec<P2>> = holes
        .iter()
        .map(|h| {
            let mut h = h.clone();
            if area(&h) > 0.0 {
                h.reverse();
            }
            h
        })
        .collect();
    hs.sort_by(|a, b| {
        let ma = a.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let mb = b.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        mb.total_cmp(&ma)
    });
    for h in &hs {
        // the hole's rightmost vertex, bridged to the nearest outline vertex it can see
        let hi = (0..h.len())
            .max_by(|&i, &j| h[i][0].total_cmp(&h[j][0]))
            .unwrap();
        let m = h[hi];
        let all: Vec<(P2, P2)> = (0..poly.len())
            .map(|i| (poly[i], poly[(i + 1) % poly.len()]))
            .chain(
                hs.iter()
                    .flat_map(|g| (0..g.len()).map(move |i| (g[i], g[(i + 1) % g.len()]))),
            )
            .collect();
        let mut best: Option<(f64, usize)> = None;
        for (i, &q) in poly.iter().enumerate() {
            if q[0] < m[0] - 1e-12 {
                continue;
            }
            let blocked = all
                .iter()
                .any(|&(a, b)| a != q && b != q && a != m && b != m && segments_cross(m, q, a, b));
            if blocked {
                continue;
            }
            let d = (q[0] - m[0]).hypot(q[1] - m[1]);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        let (_, oi) = best.expect("a hole with no visible outline vertex");
        let mut merged = poly[..=oi].to_vec();
        for k in 0..=h.len() {
            merged.push(h[(hi + k) % h.len()]);
        }
        merged.extend_from_slice(&poly[oi..]);
        poly = merged;
    }
    // ear clipping
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let mut out = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < 100_000 {
        guard += 1;
        let n = idx.len();
        let mut clipped = false;
        for k in 0..n {
            let (ia, ib, ic) = (idx[(k + n - 1) % n], idx[k], idx[(k + 1) % n]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            if cross(a, b, c) <= 1e-12 {
                continue;
            }
            let ear = idx.iter().all(|&j| {
                let p = poly[j];
                j == ia
                    || j == ib
                    || j == ic
                    || p == a
                    || p == b
                    || p == c
                    || !inside_tri(p, a, b, c)
            });
            if ear {
                out.push([a, b, c]);
                idx.remove(k);
                clipped = true;
                break;
            }
        }
        assert!(
            clipped,
            "triangulation stuck with {} vertices left",
            idx.len()
        );
    }
    if idx.len() == 3 {
        out.push([poly[idx[0]], poly[idx[1]], poly[idx[2]]]);
    }
    out
}

/// Builds a mesh from faces and walls.
#[derive(Default)]
pub struct Prism {
    pub tris: Vec<[[f64; 3]; 3]>,
}

impl Prism {
    /// A flat face at `z` (outline counter-clockwise from above), facing up or down.
    pub fn face(&mut self, z: f64, outer: &[P2], holes: &[Vec<P2>], up: bool) {
        for [a, b, c] in triangulate(outer, holes) {
            let t = [[a[0], a[1], z], [b[0], b[1], z], [c[0], c[1], z]];
            self.tris.push(if up { t } else { [t[0], t[2], t[1]] });
        }
    }

    /// Vertical wall along the closed loop `p` from `z0` up to `z1`. `outward`: the material
    /// is on the left of the loop's direction (a counter-clockwise outline, facing out); else
    /// on the right (a hole's wall, facing in).
    pub fn wall(&mut self, p: &[P2], z0: f64, z1: f64, outward: bool) {
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            self.quad(a, b, z0, z1, outward);
        }
    }

    /// One wall quad from `a` to `b`, `z0` to `z1`.
    pub fn quad(&mut self, a: P2, b: P2, z0: f64, z1: f64, outward: bool) {
        let (a0, b0, a1, b1) = (
            [a[0], a[1], z0],
            [b[0], b[1], z0],
            [a[0], a[1], z1],
            [b[0], b[1], z1],
        );
        if outward {
            self.tris.push([a0, b0, b1]);
            self.tris.push([a0, b1, a1]);
        } else {
            self.tris.push([a0, b1, b0]);
            self.tris.push([a0, a1, b1]);
        }
    }

    pub fn mesh(self) -> Mesh {
        Mesh {
            triangles: self.tris,
        }
    }
}

/// The picking bracket, 60 x 40 x 12 (top at Z0, X0 Y0 a corner):
/// - through holes 5 mm at (40, 8), (52, 8), (40, 32), (52, 32);
/// - a blind hole 6 mm at (46, 20), 6 deep (flat floor at -6);
/// - a pocket X8..28 Y8..32 with its floor at -5, and in it a round island 6 mm at (18, 20)
///   up to -2;
/// - a step at X56..60 down to -3, open at the end.
pub fn bracket() -> Mesh {
    const N: usize = 64;
    let mut m = Prism::default();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let through: Vec<Vec<P2>> = [[40.0, 8.0], [52.0, 8.0], [40.0, 32.0], [52.0, 32.0]]
        .iter()
        .map(|&c| circle(c, 2.5, N))
        .collect();
    let blind = circle([46.0, 20.0], 3.0, N);
    let pocket = rect(8.0, 8.0, 28.0, 32.0);
    let island = circle([18.0, 20.0], 3.0, N);
    // top Z0: X0..56, minus the pocket and the holes
    let mut top_holes = through.clone();
    top_holes.push(blind.clone());
    top_holes.push(pocket.clone());
    m.face(0.0, &rect(0.0, 0.0, 56.0, 40.0), &top_holes, true);
    // the step's floor and riser
    m.face(-3.0, &rect(56.0, 0.0, 60.0, 40.0), &[], true);
    m.quad([56.0, 0.0], [56.0, 40.0], -3.0, 0.0, true);
    // pocket floor, its walls, the island
    m.face(-5.0, &pocket, std::slice::from_ref(&island), true);
    m.wall(&pocket, -5.0, 0.0, false);
    m.face(-2.0, &island, &[], true);
    m.wall(&island, -5.0, -2.0, true);
    // holes
    for h in &through {
        m.wall(h, -12.0, 0.0, false);
    }
    m.wall(&blind, -6.0, 0.0, false);
    m.face(-6.0, &blind, &[], true);
    // bottom (down-facing), through holes open; the outline has a vertex at X56 on both
    // long sides, where the step's wall meets the full-height wall
    let outline = vec![
        [0.0, 0.0],
        [56.0, 0.0],
        [60.0, 0.0],
        [60.0, 40.0],
        [56.0, 40.0],
        [0.0, 40.0],
    ];
    m.face(-12.0, &outline, &through, false);
    // outside walls: full height to Z0 for X0..56, to -3 for the step (split at -3 so the
    // edges meet vertex to vertex)
    m.quad([0.0, 40.0], [0.0, 0.0], -12.0, -3.0, true);
    m.quad([0.0, 40.0], [0.0, 0.0], -3.0, 0.0, true);
    m.quad([0.0, 0.0], [56.0, 0.0], -12.0, -3.0, true);
    m.quad([0.0, 0.0], [56.0, 0.0], -3.0, 0.0, true);
    m.quad([56.0, 0.0], [60.0, 0.0], -12.0, -3.0, true);
    m.quad([60.0, 0.0], [60.0, 40.0], -12.0, -3.0, true);
    m.quad([60.0, 40.0], [56.0, 40.0], -12.0, -3.0, true);
    m.quad([56.0, 40.0], [0.0, 40.0], -12.0, -3.0, true);
    m.quad([56.0, 40.0], [0.0, 40.0], -3.0, 0.0, true);
    m.mesh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulation_covers_the_area() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]];
        let holes = vec![
            circle([3.0, 3.0], 1.0, 32),
            vec![[6.0, 2.0], [8.0, 2.0], [8.0, 4.0], [6.0, 4.0]],
        ];
        let tris = triangulate(&outer, &holes);
        let a: f64 = tris.iter().map(|t| area(t)).sum();
        let want = 60.0 - area(&holes[0]).abs() - 4.0;
        assert!((a - want).abs() < 1e-9, "{a} vs {want}");
        assert!(tris.iter().all(|t| area(t) > 0.0));
    }

    #[test]
    fn the_bracket_is_closed() {
        // every edge is used by exactly two triangles, in opposite directions
        use std::collections::HashMap;
        let m = bracket();
        let q = |p: [f64; 3]| {
            (
                (p[0] * 1e6).round() as i64,
                (p[1] * 1e6).round() as i64,
                (p[2] * 1e6).round() as i64,
            )
        };
        let mut edges: HashMap<_, i32> = HashMap::new();
        for t in &m.triangles {
            for k in 0..3 {
                *edges.entry((q(t[k]), q(t[(k + 1) % 3]))).or_default() += 1;
            }
        }
        let open: Vec<_> = edges
            .iter()
            .filter(|(&(a, b), &n)| n != 1 || edges.get(&(b, a)) != Some(&1))
            .collect();
        assert!(
            open.is_empty(),
            "{} open or doubled edges, e.g. {:?}",
            open.len(),
            open.first()
        );
        // volume: 60 x 40 x 12 minus the step, pocket, island, holes
        let v: f64 = m
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = *t;
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum();
        let circ = |r: f64| area(&circle([0.0, 0.0], r, 64));
        let want = 60.0 * 40.0 * 12.0 - 4.0 * 40.0 * 3.0 - 20.0 * 24.0 * 5.0 + circ(3.0) * 3.0
            - 4.0 * circ(2.5) * 12.0
            - circ(3.0) * 6.0;
        assert!((v - want).abs() < 1e-6, "volume {v} vs {want}");
    }
}
