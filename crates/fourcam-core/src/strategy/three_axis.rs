//! First 3-axis operations (tool along +Z, work held in a vise): facing, rectangular pocket,
//! drilling (pecking with an end mill), 2D contour and a 3D drop-cutter raster over a mesh.
//! Moves are `Rapid3` / `Line3` in the setup's work coordinates (Z up, Z0 usually the stock
//! top). Every pass starts and ends at the safe Z; rapids never move XY below it.

use crate::dropcutter::{drop_triangle, Cutter};
use crate::job::{Contour, Drill, Face, Pocket, Raster3d, StockBox};
use crate::mesh::Mesh;
use crate::toolpath::Builder;

const UP: [f64; 3] = [0.0, 0.0, 1.0];

/// Levels from `top` down to `bottom`, at most `step` apart, ending exactly at `bottom`.
pub fn levels(top: f64, bottom: f64, step: f64) -> Vec<f64> {
    if bottom >= top - 1e-9 {
        return vec![bottom];
    }
    let n = ((top - bottom) / step.max(1e-3)).ceil().max(1.0) as usize;
    (1..=n)
        .map(|k| top - (top - bottom) * k as f64 / n as f64)
        .collect()
}

/// Feed down to `z` at the plunge feed, from the safe height above (x, y).
fn enter(b: &mut Builder, safe: f64, x: f64, y: f64, z_start: f64, z: f64, plunge: f64) {
    b.rapid3(None, None, Some(safe));
    b.rapid3(Some(x), Some(y), None);
    b.rapid3(None, None, Some(z_start));
    b.line3(x, y, z, UP, plunge);
}

fn leave(b: &mut Builder, safe: f64) {
    b.rapid3(None, None, Some(safe));
}

/// Back-and-forth passes along X covering [x0, x1] x [y0, y1] (tool centre) at height z.
/// `area` is [x0, x1, y0, y1].
fn zigzag(b: &mut Builder, area: [f64; 4], z: f64, stepover: f64, feed: f64) {
    let [x0, x1, y0, y1] = area;
    let n = (((y1 - y0) / stepover.max(1e-3)).ceil() as usize).max(1);
    for k in 0..=n {
        let y = y0 + (y1 - y0) * k as f64 / n as f64;
        let (xa, xb) = if k % 2 == 0 { (x0, x1) } else { (x1, x0) };
        b.line3(xa, y, z, UP, feed);
        b.line3(xb, y, z, UP, feed);
    }
}

pub fn face(b: &mut Builder, d: f64, stock: &StockBox, safe: f64, op: &Face) {
    let r = d / 2.0;
    let top = stock.max[2];
    let bottom = top - op.depth;
    // overhang the stock by the tool radius so the edges are cleared
    let (x0, x1) = (stock.min[0] - r * 0.5, stock.max[0] + r * 0.5);
    let (y0, y1) = (stock.min[1] + r * 0.5, stock.max[1] - r * 0.5);
    b.comment("--- facing ---");
    for z in levels(top, bottom, op.step_down) {
        enter(b, safe, x0 - r - 1.0, y0, z + 1.0, z, op.plunge_feed);
        b.line3(x0, y0, z, UP, op.feed);
        zigzag(b, [x0, x1, y0, y1], z, op.stepover * d, op.feed);
        leave(b, safe);
    }
}

pub fn pocket(
    b: &mut Builder,
    d: f64,
    stock: &StockBox,
    safe: f64,
    op: &Pocket,
) -> Result<(), String> {
    let r = d / 2.0;
    let [px0, py0, px1, py1] = op.rect;
    let (x0, x1, y0, y1) = (
        px0.min(px1) + r,
        px0.max(px1) - r,
        py0.min(py1) + r,
        py0.max(py1) - r,
    );
    if x0 > x1 || y0 > y1 {
        return Err(format!(
            "pocket {:?} is narrower than the {d} mm tool",
            op.rect
        ));
    }
    let top = op.top.unwrap_or(stock.max[2]);
    b.comment("--- pocket ---");
    for z in levels(top, top - op.depth, op.step_down) {
        // ramp in along X at the plunge feed, clear, then one finishing lap of the walls
        enter(
            b,
            safe,
            x0,
            y0,
            z + op.step_down.max(0.5),
            z,
            op.plunge_feed,
        );
        zigzag(b, [x0, x1, y0, y1], z, op.stepover * d, op.feed);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)] {
            b.line3(x, y, z, UP, op.feed);
        }
        leave(b, safe);
    }
    Ok(())
}

pub fn drill(b: &mut Builder, stock: &StockBox, safe: f64, op: &Drill) {
    let top = op.top.unwrap_or(stock.max[2]);
    let bottom = top - op.depth;
    b.comment("--- drilling ---");
    for &[x, y] in &op.points {
        b.rapid3(None, None, Some(safe));
        b.rapid3(Some(x), Some(y), None);
        b.rapid3(None, None, Some(top + op.retract));
        // peck: feed down a peck, rapid back out, rapid down to just above the last depth
        let mut z = top;
        while z > bottom + 1e-9 {
            let next = (z - op.peck).max(bottom);
            b.line3(x, y, next, UP, op.feed);
            b.rapid3(None, None, Some(top + op.retract));
            if next > bottom + 1e-9 {
                b.rapid3(None, None, Some(next + 0.2));
            }
            z = next;
        }
        b.rapid3(None, None, Some(safe));
    }
}

/// Offset a closed polygon by `dist` (outward for a counter-clockwise polygon when positive),
/// joining edges with miters.
pub fn offset_polygon(path: &[[f64; 2]], dist: f64) -> Vec<[f64; 2]> {
    let n = path.len();
    let area: f64 = (0..n)
        .map(|i| {
            let (a, b) = (path[i], path[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0;
    let s = if area >= 0.0 { dist } else { -dist }; // outward for either winding
    let lines: Vec<([f64; 2], [f64; 2])> = (0..n)
        .map(|i| {
            let (a, b) = (path[i], path[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy).max(1e-12);
            let nrm = [dy / l * s, -dx / l * s]; // right-hand normal = outward for CCW
            ([a[0] + nrm[0], a[1] + nrm[1]], [dx, dy])
        })
        .collect();
    (0..n)
        .map(|i| {
            let (p1, d1) = lines[(i + n - 1) % n];
            let (p2, d2) = lines[i];
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            if den.abs() < 1e-12 {
                p2
            } else {
                let t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / den;
                [p1[0] + t * d1[0], p1[1] + t * d1[1]]
            }
        })
        .collect()
}

pub fn contour(
    b: &mut Builder,
    d: f64,
    stock: &StockBox,
    safe: f64,
    op: &Contour,
) -> Result<(), String> {
    if op.path.len() < 3 {
        return Err("contour needs a closed path of at least 3 points".into());
    }
    let r = d / 2.0;
    let path = match op.side.as_str() {
        "outside" => offset_polygon(&op.path, r),
        "inside" => offset_polygon(&op.path, -r),
        _ => op.path.clone(),
    };
    let top = op.top.unwrap_or(stock.max[2]);
    b.comment("--- contour ---");
    for z in levels(top, top - op.depth, op.step_down) {
        let [x, y] = path[0];
        enter(b, safe, x, y, z + op.step_down.max(0.5), z, op.plunge_feed);
        for p in path.iter().skip(1).chain(std::iter::once(&path[0])) {
            b.line3(p[0], p[1], z, UP, op.feed);
        }
        leave(b, safe);
    }
    Ok(())
}

/// Vertical drop-cutter over a mesh, with a simple XY bin index.
pub struct DropCutter3 {
    cutter: Cutter,
    tris: Vec<[[f64; 3]; 3]>,
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    bins: Vec<Vec<u32>>,
}

impl DropCutter3 {
    pub fn new(mesh: &Mesh, cutter: Cutter) -> DropCutter3 {
        let r = cutter.radius();
        let (lo, hi) = mesh.bounds().unwrap_or(([0.0; 3], [1.0; 3]));
        let cell = (2.0 * r).max(0.5);
        let (x0, y0) = (lo[0] - r, lo[1] - r);
        let nx = (((hi[0] + r - x0) / cell).ceil() as usize).max(1);
        let ny = (((hi[1] + r - y0) / cell).ceil() as usize).max(1);
        let mut bins = vec![Vec::new(); nx * ny];
        for (k, t) in mesh.triangles.iter().enumerate() {
            let (mut a, mut b) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in t {
                for c in 0..2 {
                    a[c] = a[c].min(p[c]);
                    b[c] = b[c].max(p[c]);
                }
            }
            let i0 = (((a[0] - r - x0) / cell).floor().max(0.0)) as usize;
            let i1 = ((((b[0] + r - x0) / cell).floor()) as usize).min(nx - 1);
            let j0 = (((a[1] - r - y0) / cell).floor().max(0.0)) as usize;
            let j1 = ((((b[1] + r - y0) / cell).floor()) as usize).min(ny - 1);
            for i in i0..=i1 {
                for j in j0..=j1 {
                    bins[j * nx + i].push(k as u32);
                }
            }
        }
        DropCutter3 {
            cutter,
            tris: mesh.triangles.clone(),
            x0,
            y0,
            cell,
            nx,
            ny,
            bins,
        }
    }

    /// Lowest tip Z at (x, y) that does not cut into the mesh (never below `floor`).
    pub fn z_at(&self, x: f64, y: f64, floor: f64) -> f64 {
        let (i, j) = ((x - self.x0) / self.cell, (y - self.y0) / self.cell);
        if i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.ny as f64 {
            return floor;
        }
        let mut best = floor;
        for &k in &self.bins[j as usize * self.nx + i as usize] {
            let t = &self.tris[k as usize];
            let lt = [
                [t[0][0] - x, t[0][1] - y, t[0][2]],
                [t[1][0] - x, t[1][1] - y, t[1][2]],
                [t[2][0] - x, t[2][1] - y, t[2][2]],
            ];
            if lt.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max) <= best {
                continue;
            }
            best = best.max(drop_triangle(&self.cutter, &lt));
        }
        best
    }
}

pub fn raster3d(
    b: &mut Builder,
    dc: &DropCutter3,
    stock: &StockBox,
    safe: f64,
    op: &Raster3d,
) -> Result<(), String> {
    let [x0, x1] = op.x_range;
    let [y0, y1] = op.y_range;
    let floor = op.floor;
    let d = 2.0 * dc.cutter.radius();
    let step = op.stepover.max(0.01);
    let n = (((y1 - y0).abs() / step).ceil() as usize).max(1);
    let m = (((x1 - x0).abs() / op.sample.max(0.01)).ceil() as usize).max(1);
    let surf = |x: f64, y: f64| dc.z_at(x, y, floor) + op.leave;
    let top = stock.max[2];
    let feeds = [op.feed, op.plunge_feed];
    b.comment("--- 3D raster ---");
    for level in levels(top, floor + op.leave, op.step_down.max(d * 0.05)) {
        let z_of = |x: f64, y: f64| surf(x, y).max(level);
        for k in 0..=n {
            let y = y0 + (y1 - y0) * k as f64 / n as f64;
            let xs: Vec<f64> = (0..=m)
                .map(|i| x0 + (x1 - x0) * i as f64 / m as f64)
                .collect();
            let row: Vec<f64> = if k % 2 == 0 {
                xs
            } else {
                xs.into_iter().rev().collect()
            };
            let z0 = z_of(row[0], y);
            if k == 0 {
                // one entry per level, from above the uncut stock: rows are linked at depth,
                // so the tool never drops beside material outside the raster area
                enter(b, safe, row[0], y, top + 1.0, z0, op.plunge_feed);
            } else {
                let p = b.p3.expect("inside a level");
                line_refined(b, &z_of, p, [row[0], y, z0], op.tolerance, 0, feeds);
            }
            for w in row.windows(2) {
                // split where the surface rises above the straight move
                line_refined(
                    b,
                    &z_of,
                    [w[0], y, z_of(w[0], y)],
                    [w[1], y, z_of(w[1], y)],
                    op.tolerance,
                    0,
                    feeds,
                );
            }
        }
        leave(b, safe);
    }
    Ok(())
}

fn line_refined(
    b: &mut Builder,
    z_of: &dyn Fn(f64, f64) -> f64,
    p0: [f64; 3],
    p1: [f64; 3],
    tol: f64,
    depth: u32,
    feed: [f64; 2],
) {
    if depth < 12 {
        let m = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
        let zm = z_of(m[0], m[1]);
        if zm > (p0[2] + p1[2]) / 2.0 + tol {
            let mid = [m[0], m[1], zm];
            line_refined(b, z_of, p0, mid, tol, depth + 1, feed);
            line_refined(b, z_of, mid, p1, tol, depth + 1, feed);
            return;
        }
    }
    b.line3(
        p1[0],
        p1[1],
        p1[2],
        UP,
        descent_feed(p0, p1, feed[0], feed[1]),
    );
}

/// Feed for a move from `p0` to `p1` so Z never falls faster than `plunge` mm/min, even
/// after the post rounds coordinates to 0.001 mm (whole mm/min, rounded down).
pub fn descent_feed(p0: [f64; 3], p1: [f64; 3], feed: f64, plunge: f64) -> f64 {
    const ROUND: f64 = 1e-3;
    let dz = p0[2] - p1[2];
    if dz <= -ROUND {
        return feed;
    }
    let len = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2) + dz * dz).sqrt();
    feed.min(
        (plunge * (len - ROUND).max(0.0) / (dz + ROUND))
            .floor()
            .max(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_end_at_the_bottom() {
        assert_eq!(levels(0.0, -1.0, 0.4), vec![-1.0 / 3.0, -2.0 / 3.0, -1.0]);
        assert_eq!(levels(0.0, 0.0, 0.4), vec![0.0]);
    }

    #[test]
    fn offsets_are_outward_for_both_windings() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let out = offset_polygon(&sq, 1.0);
        assert_eq!(out[0], [-1.0, -1.0]);
        assert_eq!(out[2], [11.0, 11.0]);
        let mut cw = sq;
        cw.reverse();
        let out = offset_polygon(&cw, 1.0);
        assert!(out.contains(&[-1.0, -1.0]) && out.contains(&[11.0, 11.0]));
        let inn = offset_polygon(&sq, -1.0);
        assert_eq!(inn[0], [1.0, 1.0]);
    }
}
