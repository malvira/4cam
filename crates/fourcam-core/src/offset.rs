//! Offsets of a 2D region (closed polylines, even-odd: an outline and its islands) by
//! distance, as closed loops: the iso-lines of the signed distance to the outline, sampled on
//! a grid and traced with marching squares. Used to clear pockets of any shape.

use crate::engrave::{Path2, Region};
use std::collections::HashMap;

/// A grid edge: (i, j, 0) from (i, j) to (i+1, j); (i, j, 1) from (i, j) to (i, j+1).
type Edge = (usize, usize, u8);

/// A region's signed distance (inside positive) sampled on a grid.
pub struct DistanceField {
    pub x0: f64,
    pub y0: f64,
    pub h: f64,
    pub nx: usize,
    pub ny: usize,
    pub d: Vec<f64>,
}

impl DistanceField {
    /// Sample `paths` (even-odd) every `h` mm, distances capped at `cap`.
    pub fn new(paths: &[Path2], h: f64, cap: f64) -> DistanceField {
        let region = Region::new(paths, cap);
        let pad = 2.0 * h;
        let (x0, y0) = (region.lo[0] - pad, region.lo[1] - pad);
        let nx = (((region.hi[0] + pad - x0) / h).ceil() as usize + 1).max(2);
        let ny = (((region.hi[1] + pad - y0) / h).ceil() as usize + 1).max(2);
        let mut d = vec![0.0; nx * ny];
        for j in 0..ny {
            let y = y0 + j as f64 * h;
            let spans = region.spans(y);
            for i in 0..nx {
                let x = x0 + i as f64 * h;
                let inside = spans.iter().any(|&(a, b)| x >= a && x < b);
                let dist = region.distance([x, y], cap);
                d[j * nx + i] = if inside { dist } else { -dist };
            }
        }
        DistanceField {
            x0,
            y0,
            h,
            nx,
            ny,
            d,
        }
    }

    /// Polylines (open strokes) swept by a round pen of radius `r`: `r` minus the distance to
    /// the nearest stroke, sampled every `h` mm. Its zero loops are the outlines of the inked
    /// shape (outer ones counter-clockwise, round holes clockwise).
    pub fn from_strokes(paths: &[Path2], r: f64, h: f64) -> DistanceField {
        let refs: Vec<&[[f64; 2]]> = paths.iter().map(|p| p.as_slice()).collect();
        let reach = r + 3.0 * h;
        let index = SegIndex::new(&refs, reach);
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for q in paths.iter().flatten() {
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        if !lo[0].is_finite() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let (x0, y0) = (lo[0] - reach, lo[1] - reach);
        let nx = (((hi[0] + reach - x0) / h).ceil() as usize + 1).max(2);
        let ny = (((hi[1] + reach - y0) / h).ceil() as usize + 1).max(2);
        let mut d = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let p = [x0 + i as f64 * h, y0 + j as f64 * h];
                d[j * nx + i] = r - index.nearest(p);
            }
        }
        DistanceField {
            x0,
            y0,
            h,
            nx,
            ny,
            d,
        }
    }

    pub fn max(&self) -> f64 {
        self.d.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }

    fn at(&self, i: usize, j: usize) -> f64 {
        self.d[j * self.nx + i]
    }

    fn point(&self, i: usize, j: usize) -> [f64; 2] {
        [self.x0 + i as f64 * self.h, self.y0 + j as f64 * self.h]
    }

    /// Closed loops where the distance equals `level` (> 0), each with the deeper part on its
    /// left: counter-clockwise round an outline, clockwise round an island. A tool running
    /// them in order climb-mills the remaining wall.
    pub fn loops(&self, level: f64) -> Vec<Path2> {
        // edges are keyed by (i, j, dir): dir 0 = horizontal edge from (i, j) to (i+1, j),
        // dir 1 = vertical edge from (i, j) to (i, j+1)
        let crossing = |i: usize, j: usize, dir: u8| -> [f64; 2] {
            let (i2, j2) = if dir == 0 { (i + 1, j) } else { (i, j + 1) };
            let (a, b) = (self.at(i, j) - level, self.at(i2, j2) - level);
            let t = if (a - b).abs() > 1e-300 {
                a / (a - b)
            } else {
                0.5
            };
            let (p, q) = (self.point(i, j), self.point(i2, j2));
            [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
        };
        // directed segments: from edge -> to edge, oriented with the inside on the left
        let mut next: HashMap<Edge, Edge> = HashMap::new();
        for j in 0..self.ny - 1 {
            for i in 0..self.nx - 1 {
                let v = [
                    self.at(i, j) > level,         // bottom-left
                    self.at(i + 1, j) > level,     // bottom-right
                    self.at(i + 1, j + 1) > level, // top-right
                    self.at(i, j + 1) > level,     // top-left
                ];
                let bottom = (i, j, 0u8);
                let right = (i + 1, j, 1u8);
                let top = (i, j + 1, 0u8);
                let left = (i, j, 1u8);
                let code = v[0] as u8 | (v[1] as u8) << 1 | (v[2] as u8) << 2 | (v[3] as u8) << 3;
                let centre = 0.25
                    * (self.at(i, j)
                        + self.at(i + 1, j)
                        + self.at(i + 1, j + 1)
                        + self.at(i, j + 1))
                    > level;
                // each directed segment has the deeper side on its right; loops are reversed
                // when traced
                let segs: &[(Edge, Edge)] = match code {
                    0 | 15 => &[],
                    1 => &[(left, bottom)],
                    2 => &[(bottom, right)],
                    3 => &[(left, right)],
                    4 => &[(right, top)],
                    5 => {
                        if centre {
                            &[(left, top), (right, bottom)]
                        } else {
                            &[(left, bottom), (right, top)]
                        }
                    }
                    6 => &[(bottom, top)],
                    7 => &[(left, top)],
                    8 => &[(top, left)],
                    9 => &[(top, bottom)],
                    10 => {
                        if centre {
                            &[(bottom, left), (top, right)]
                        } else {
                            &[(bottom, right), (top, left)]
                        }
                    }
                    11 => &[(top, right)],
                    12 => &[(right, left)],
                    13 => &[(right, bottom)],
                    14 => &[(bottom, left)],
                    _ => &[],
                };
                for &(a, b) in segs {
                    next.insert(a, b);
                }
            }
        }
        let mut out = Vec::new();
        let mut keys: Vec<_> = next.keys().copied().collect();
        keys.sort();
        for start in keys {
            if !next.contains_key(&start) {
                continue;
            }
            let mut loop_pts = Vec::new();
            let mut e = start;
            while let Some(n) = next.remove(&e) {
                loop_pts.push(crossing(e.0, e.1, e.2));
                e = n;
                if e == start {
                    break;
                }
            }
            if loop_pts.len() >= 3 {
                loop_pts.push(loop_pts[0]);
                loop_pts.reverse();
                out.push(loop_pts);
            }
        }
        out
    }
}

/// Polyline segments binned on a grid, for "how far is the nearest one" within a reach.
pub struct SegIndex {
    segs: Vec<[[f64; 2]; 2]>,
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    bins: Vec<Vec<u32>>,
}

impl SegIndex {
    /// The segments of `paths`, binned `cell` mm (queries reach at most `cell`).
    pub fn new(paths: &[&[[f64; 2]]], cell: f64) -> SegIndex {
        let segs: Vec<[[f64; 2]; 2]> = paths
            .iter()
            .flat_map(|p| {
                let mut s: Vec<[[f64; 2]; 2]> = p.windows(2).map(|w| [w[0], w[1]]).collect();
                if p.len() == 1 {
                    s.push([p[0], p[0]]); // a dot
                }
                s
            })
            .collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for s in &segs {
            for q in s {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        if segs.is_empty() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let cell = cell.max(1e-3);
        let nx = (((hi[0] - lo[0]) / cell).floor() as usize + 1).max(1);
        let ny = (((hi[1] - lo[1]) / cell).floor() as usize + 1).max(1);
        let mut bins = vec![Vec::new(); nx * ny];
        for (k, s) in segs.iter().enumerate() {
            let i0 = ((s[0][0].min(s[1][0]) - lo[0]) / cell).floor() as usize;
            let i1 = (((s[0][0].max(s[1][0]) - lo[0]) / cell).floor() as usize).min(nx - 1);
            let j0 = ((s[0][1].min(s[1][1]) - lo[1]) / cell).floor() as usize;
            let j1 = (((s[0][1].max(s[1][1]) - lo[1]) / cell).floor() as usize).min(ny - 1);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    bins[j * nx + i].push(k as u32);
                }
            }
        }
        SegIndex {
            segs,
            x0: lo[0],
            y0: lo[1],
            cell,
            nx,
            ny,
            bins,
        }
    }

    /// Distance from `p` to the nearest segment, or `cell` if none is that close.
    pub fn nearest(&self, p: [f64; 2]) -> f64 {
        let fi = ((p[0] - self.x0) / self.cell).floor();
        let fj = ((p[1] - self.y0) / self.cell).floor();
        let mut best = self.cell;
        for dj in -1i64..=1 {
            for di in -1i64..=1 {
                let (i, j) = (fi as i64 + di, fj as i64 + dj);
                if i < 0 || j < 0 || i >= self.nx as i64 || j >= self.ny as i64 {
                    continue;
                }
                for &k in &self.bins[j as usize * self.nx + i as usize] {
                    let [a, b] = self.segs[k as usize];
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = dx * dx + dy * dy;
                    let t = if l2 > 0.0 {
                        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let (ex, ey) = (p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
                    best = best.min((ex * ex + ey * ey).sqrt());
                }
            }
        }
        best
    }
}

impl DistanceField {
    /// Grid points at least `r` inside that no loop passes within `r` of: what a tool of
    /// radius `r` following `loops` would leave. Returns (point, its distance).
    pub fn uncovered(&self, loops: &[&[[f64; 2]]], r: f64) -> Vec<([f64; 2], f64)> {
        let index = SegIndex::new(loops, r * 1.01);
        let mut out = Vec::new();
        for j in 0..self.ny {
            for i in 0..self.nx {
                let d = self.at(i, j);
                if d >= r {
                    let p = self.point(i, j);
                    if index.nearest(p) > r {
                        out.push((p, d));
                    }
                }
            }
        }
        out
    }
}

/// Douglas-Peucker on a closed loop (first point repeated at the end).
pub fn simplify(p: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    fn rec(p: &[[f64; 2]], tol: f64, keep: &mut [bool], a: usize, b: usize) {
        if b <= a + 1 {
            return;
        }
        let (pa, pb) = (p[a], p[b]);
        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let mut best = (0.0, a);
        for (k, q) in p.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-12 {
                ((q[0] - pa[0]).powi(2) + (q[1] - pa[1]).powi(2)).sqrt()
            } else {
                ((q[0] - pa[0]) * dy - (q[1] - pa[1]) * dx).abs() / len
            };
            if d > best.0 {
                best = (d, k);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            rec(p, tol, keep, a, best.1);
            rec(p, tol, keep, best.1, b);
        }
    }
    if p.len() < 4 {
        return p.to_vec();
    }
    let mut keep = vec![false; p.len()];
    let n = p.len() - 1;
    // split the loop at its first point and the point farthest from it
    let far = (1..n)
        .max_by(|&i, &j| {
            let d = |k: usize| (p[k][0] - p[0][0]).powi(2) + (p[k][1] - p[0][1]).powi(2);
            d(i).total_cmp(&d(j))
        })
        .unwrap_or(1);
    keep[0] = true;
    keep[far] = true;
    keep[n] = true;
    rec(p, tol, &mut keep, 0, far);
    rec(p, tol, &mut keep, far, n);
    p.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(q, _)| *q)
        .collect()
}
