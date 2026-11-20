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
        if crate::budget::over() {
            break;
        }
        let y = y0 + (y1 - y0) * k as f64 / n as f64;
        let (xa, xb) = if k % 2 == 0 { (x0, x1) } else { (x1, x0) };
        b.line3(xa, y, z, UP, feed);
        b.line3(xb, y, z, UP, feed);
    }
}

pub fn face(b: &mut Builder, d: f64, stock: &StockBox, safe: f64, op: &Face) -> Result<(), String> {
    if !op.paths.is_empty() {
        // bounded by a face: cleared like a shaped pocket from the stock top down
        let p = Pocket {
            tool: op.tool,
            index_a: op.index_a,
            rect: [0.0; 4],
            paths: op.paths.clone(),
            depth: op.depth,
            top: None,
            step_down: op.step_down,
            stepover: op.stepover,
            feed: op.feed,
            plunge_feed: op.plunge_feed,
            smoothing: None,
            regions: Vec::new(),
        };
        b.comment("--- facing (bounded by a face) ---");
        return path_pocket(b, d, stock, safe, &p);
    }
    let r = d / 2.0;
    let top = stock.max[2];
    let bottom = top - op.depth;
    // overhang the stock by the tool radius so the edges are cleared
    let (x0, x1) = (stock.min[0] - r * 0.5, stock.max[0] + r * 0.5);
    let (y0, y1) = (stock.min[1] + r * 0.5, stock.max[1] - r * 0.5);
    b.comment("--- facing ---");
    for z in levels(top, bottom, op.step_down) {
        if crate::budget::over() {
            break;
        }
        enter(b, safe, x0 - r - 1.0, y0, z + 1.0, z, op.plunge_feed);
        b.line3(x0, y0, z, UP, op.feed);
        zigzag(b, [x0, x1, y0, y1], z, op.stepover * d, op.feed);
        leave(b, safe);
    }
    Ok(())
}

pub fn pocket(
    b: &mut Builder,
    d: f64,
    stock: &StockBox,
    safe: f64,
    op: &Pocket,
) -> Result<(), String> {
    if !op.paths.is_empty() {
        return path_pocket(b, d, stock, safe, op);
    }
    let r = d / 2.0;
    let [px0, py0, px1, py1] = op.rect;
    let (x0, x1, y0, y1) = (
        px0.min(px1) + r,
        px0.max(px1) - r,
        py0.min(py1) + r,
        py0.max(py1) - r,
    );
    if op.rect == [0.0; 4] {
        return Err("a pocket needs a rect or closed paths".into());
    }
    if x0 > x1 || y0 > y1 {
        return Err(format!(
            "pocket {:?} is narrower than the {d} mm tool",
            op.rect
        ));
    }
    let top = op.top.unwrap_or(stock.max[2]);
    b.comment("--- pocket ---");
    for z in levels(top, top - op.depth, op.step_down) {
        if crate::budget::over() {
            break;
        }
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

/// Largest spacing between a shaped pocket's offset loops, in tool diameters (see
/// [`path_pocket`]): a larger stepover is cut at this.
pub const MAX_LOOP_SPACING: f64 = 0.7;
/// Grid spacing of the distance field a shaped pocket is offset on (mm).
pub const POCKET_FIELD_CELL: f64 = 0.05;
/// Offset loops are simplified to within this (mm).
pub const POCKET_LOOP_TOL: f64 = 0.002;

/// The tool-centre loops that clear a shaped pocket at one depth, innermost first: offsets of
/// the outline (and round its islands) at the tool radius, then every `step` deeper in. The
/// last loop of each region is the finishing pass along the walls.
pub fn pocket_loops(
    paths: &[Vec<[f64; 2]>],
    r: f64,
    step: f64,
) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let field = crate::offset::DistanceField::new(paths, POCKET_FIELD_CELL, f64::INFINITY);
    let deepest = field.max();
    if deepest <= r + 1e-6 {
        return Err(format!(
            "the pocket is nowhere wide enough for the {} mm tool",
            2.0 * r
        ));
    }
    let mut levels = Vec::new();
    let mut l = r;
    while l < deepest {
        levels.push(l);
        l += step;
    }
    // the middle (up to `deepest`) must be within reach of the last loop
    if let Some(&last) = levels.last() {
        if deepest - last > 0.9 * r {
            levels.push(deepest - 0.5 * r);
        }
    }
    let mut loops: Vec<(f64, Vec<[f64; 2]>)> = Vec::new();
    for &level in &levels {
        if crate::budget::over() {
            break;
        }
        for lp in field.loops(level) {
            let lp = crate::offset::simplify(&lp, POCKET_LOOP_TOL);
            if lp.len() >= 4 {
                loops.push((level, lp));
            }
        }
    }
    // Ridges between two walls (or a wall and an island) can be less deep than the next
    // level yet out of reach of the last: there, add loops halfway between the levels, kept
    // only where they pass near what is left. Spacing at most 0.7 diameters makes one round
    // enough; up to three are tried.
    for _ in 0..3 {
        let refs: Vec<&[[f64; 2]]> = loops.iter().map(|(_, l)| l.as_slice()).collect();
        let left = field.uncovered(&refs, r);
        if left.is_empty() {
            break;
        }
        let mut have: Vec<f64> = loops.iter().map(|(l, _)| *l).collect();
        have.sort_by(f64::total_cmp);
        have.dedup();
        let mut added = Vec::new();
        for (k, &lo) in have.iter().enumerate() {
            let hi = have.get(k + 1).copied().unwrap_or(deepest);
            if !left.iter().any(|&(_, d)| d > lo && d <= hi) {
                continue;
            }
            let mid = 0.5 * (lo + hi);
            for lp in field.loops(mid) {
                let near = left.iter().any(|&(p, _)| {
                    lp.iter()
                        .any(|q| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) <= 4.0 * r * r)
                });
                if near {
                    let lp = crate::offset::simplify(&lp, POCKET_LOOP_TOL);
                    if lp.len() >= 4 {
                        added.push((mid, lp));
                    }
                }
            }
        }
        if added.is_empty() {
            break;
        }
        loops.extend(added);
    }
    // innermost first; the walls' laps last
    loops.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(loops.into_iter().map(|(_, l)| l).collect())
}

/// A pocket of any shape: at each depth, the offset loops from the middle out, each entered
/// from above at the plunge feed and left to the safe height.
fn path_pocket(
    b: &mut Builder,
    d: f64,
    stock: &StockBox,
    safe: f64,
    op: &Pocket,
) -> Result<(), String> {
    let r = d / 2.0;
    // Offset loops keep their corners sharp, so along a corner's diagonal they are √2 times
    // the spacing apart: more than 0.7 diameters would leave material there.
    let loops = pocket_loops(
        &op.paths,
        r,
        (op.stepover * d).clamp(0.01, MAX_LOOP_SPACING * d),
    )?;
    let top = op.top.unwrap_or(stock.max[2]);
    b.comment("--- pocket (outline) ---");
    let mut at = [f64::NAN; 2];
    for z in levels(top, top - op.depth, op.step_down) {
        if crate::budget::over() {
            break;
        }
        for lp in &loops {
            // start where the last loop ended, as near as the loop allows
            let n = lp.len() - 1;
            let s = (0..n)
                .min_by(|&i, &j| {
                    let dd = |k: usize| (lp[k][0] - at[0]).powi(2) + (lp[k][1] - at[1]).powi(2);
                    dd(i).total_cmp(&dd(j))
                })
                .filter(|_| at[0].is_finite())
                .unwrap_or(0);
            let start = lp[s];
            enter(
                b,
                safe,
                start[0],
                start[1],
                z + op.step_down.max(0.5),
                z,
                op.plunge_feed,
            );
            for k in 1..=n {
                let q = lp[(s + k) % n];
                b.line3(q[0], q[1], z, UP, op.feed);
            }
            leave(b, safe);
            at = start;
        }
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
    // a closed path may repeat its first point at the end (picked loops do): offset without
    // it, or the zero-length edge there sends the tool back onto the outline
    let mut src = op.path.clone();
    while src.len() > 3 && {
        let (a, b) = (src[0], src[src.len() - 1]);
        (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9
    } {
        src.pop();
    }
    // (the outline as written: where tabs placed by hand are measured along)
    let src0 = src.clone();
    let mut path = match op.side.as_str() {
        "outside" => offset_polygon(&src, r),
        "inside" => offset_polygon(&src, -r),
        _ => src,
    };
    // Climb milling, spindle clockwise, keeps the material on the tool's right: round an
    // outline (outside, or on it) counter-clockwise, round a hole clockwise. Conventional:
    // the other way.
    let area: f64 = (0..path.len())
        .map(|i| {
            let (a, b) = (path[i], path[(i + 1) % path.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let climb = op.direction.as_deref() != Some("conventional");
    let want_ccw = (op.side != "inside") == climb;
    if (area > 0.0) != want_ccw {
        path.reverse();
    }
    let top = op.top.unwrap_or(stock.max[2]);
    let bottom = top - op.depth;
    // tabs: the last levels go over them, at the tab's top, for the tab's length plus the tool
    let tab_top = bottom + op.tab_height;
    let mut tabs = tab_spans(&path, op.tabs, op.tab_width + d);
    tabs.extend(placed_spans(&src0, &path, &op.tab_at, op.tab_width + d));
    let mut zs = levels(top, bottom, op.step_down);
    if !tabs.is_empty() && tab_top < top && !zs.iter().any(|z| (z - tab_top).abs() < 1e-6) {
        // a level at the tabs' top, so the cut above them is a whole one
        zs.push(tab_top);
        zs.sort_by(|a, b| b.total_cmp(a));
    }
    b.comment(if tabs.is_empty() {
        "--- contour ---"
    } else {
        "--- contour (with tabs) ---"
    });
    for z in zs {
        if crate::budget::over() {
            break;
        }
        let [x, y] = path[0];
        enter(b, safe, x, y, z + op.step_down.max(0.5), z, op.plunge_feed);
        if z >= tab_top - 1e-9 || tabs.is_empty() {
            for p in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                b.line3(p[0], p[1], z, UP, op.feed);
            }
        } else {
            contour_with_tabs(b, &path, &tabs, z, tab_top, op);
        }
        leave(b, safe);
    }
    Ok(())
}

/// Where tabs go along a closed path: `n` spans of `len` (arc length from the path's first
/// point), evenly spaced, each centred half a spacing in (none when the path is too short).
fn tab_spans(path: &[[f64; 2]], n: u32, len: f64) -> Vec<(f64, f64)> {
    if n == 0 || path.len() < 2 {
        return Vec::new();
    }
    let total: f64 = (0..path.len())
        .map(|i| {
            let (a, b) = (path[i], path[(i + 1) % path.len()]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .sum();
    let gap = total / n as f64;
    if len >= gap * 0.8 {
        return Vec::new();
    }
    (0..n)
        .map(|k| {
            let c = gap * (k as f64 + 0.5);
            (c - len / 2.0, c + len / 2.0)
        })
        .collect()
}

/// Arc length along a closed path to its point nearest `q`.
fn arc_of(path: &[[f64; 2]], q: [f64; 2]) -> f64 {
    let n = path.len();
    let (mut best, mut at, mut s) = (f64::INFINITY, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (path[i], path[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = [a[0] + dx * t, a[1] + dy * t];
        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
        if d < best {
            best = d;
            at = s + t * l2.sqrt();
        }
        s += l2.sqrt();
    }
    at
}

/// The point a fraction `f` of the way round a closed path from its first point.
fn point_at(path: &[[f64; 2]], f: f64) -> [f64; 2] {
    let n = path.len();
    let len = |i: usize| {
        let (a, b) = (path[i], path[(i + 1) % n]);
        (b[0] - a[0]).hypot(b[1] - a[1])
    };
    let total: f64 = (0..n).map(len).sum();
    let mut want = f.rem_euclid(1.0) * total;
    for i in 0..n {
        let l = len(i);
        if want <= l || i + 1 == n {
            let (a, b) = (path[i], path[(i + 1) % n]);
            let t = if l > 0.0 { (want / l).min(1.0) } else { 0.0 };
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        want -= l;
    }
    path[0]
}

/// Tabs placed by hand (`at`: fractions of the outline `src` as written) as spans of `len`
/// along the tool's `path`: each where the path passes nearest its point on the outline. A
/// span across the path's start is split in two.
fn placed_spans(src: &[[f64; 2]], path: &[[f64; 2]], at: &[f64], len: f64) -> Vec<(f64, f64)> {
    if src.len() < 2 || path.len() < 2 {
        return Vec::new();
    }
    let total: f64 = (0..path.len())
        .map(|i| {
            let (a, b) = (path[i], path[(i + 1) % path.len()]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .sum();
    let mut out = Vec::new();
    for &f in at {
        let c = arc_of(path, point_at(src, f));
        let (a, e) = (c - len / 2.0, c + len / 2.0);
        if a < 0.0 {
            out.push((a + total, total));
            out.push((0.0, e));
        } else if e > total {
            out.push((a, total));
            out.push((0.0, e - total));
        } else {
            out.push((a, e));
        }
    }
    out
}

/// One pass round `path` at `z`, rising to `tab_top` over each tab span (straight up and
/// down at the span's ends).
fn contour_with_tabs(
    b: &mut Builder,
    path: &[[f64; 2]],
    tabs: &[(f64, f64)],
    z: f64,
    tab_top: f64,
    op: &Contour,
) {
    let n = path.len();
    let mut s = 0.0;
    let on_tab = |s: f64| tabs.iter().any(|&(a, e)| s > a + 1e-9 && s < e - 1e-9);
    for i in 0..n {
        let (a, c) = (path[i], path[(i + 1) % n]);
        let l = (c[0] - a[0]).hypot(c[1] - a[1]);
        if l < 1e-12 {
            continue;
        }
        // the segment's breakpoints: its tab starts and ends
        let mut cuts: Vec<f64> = tabs
            .iter()
            .flat_map(|&(t0, t1)| [t0, t1])
            .filter(|&t| t > s && t < s + l)
            .collect();
        cuts.push(s + l);
        let mut at = s;
        for t in cuts {
            let mid = 0.5 * (at + t);
            let zz = if on_tab(mid) { tab_top } else { z };
            let f = (t - s) / l;
            let p = [a[0] + (c[0] - a[0]) * f, a[1] + (c[1] - a[1]) * f];
            // up or down at a tab's end, where the height changes: straight
            let fa = (at - s) / l;
            let pa = [a[0] + (c[0] - a[0]) * fa, a[1] + (c[1] - a[1]) * fa];
            b.line3(
                pa[0],
                pa[1],
                zz,
                UP,
                if zz > z { op.feed } else { op.plunge_feed },
            );
            b.line3(p[0], p[1], zz, UP, op.feed);
            at = t;
        }
        s += l;
    }
    // back down at the start, closed
    b.line3(path[0][0], path[0][1], z, UP, op.plunge_feed);
}

/// Vertical drop-cutter over a mesh, with a simple XY bin index.
pub struct DropCutter3 {
    cutter: Cutter,
    tris: Vec<[[f64; 3]; 3]>,
    /// XY box of each triangle: [x_min, y_min, x_max, y_max].
    boxes: Vec<[f64; 4]>,
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
        // bins of half the radius, each listing the triangles within the radius of it: a
        // query looks at one bin, so small bins mean few candidates
        let cell = (0.5 * r).max(0.25);
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
        // highest triangles first: a query stops at the first one that cannot beat its best
        let top = |k: u32| {
            let t = &mesh.triangles[k as usize];
            t[0][2].max(t[1][2]).max(t[2][2])
        };
        for b in &mut bins {
            b.sort_by(|&p, &q| top(q).total_cmp(&top(p)));
        }
        let boxes = mesh
            .triangles
            .iter()
            .map(|t| {
                [
                    t[0][0].min(t[1][0]).min(t[2][0]),
                    t[0][1].min(t[1][1]).min(t[2][1]),
                    t[0][0].max(t[1][0]).max(t[2][0]),
                    t[0][1].max(t[1][1]).max(t[2][1]),
                ]
            })
            .collect();
        DropCutter3 {
            cutter,
            tris: mesh.triangles.clone(),
            boxes,
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
        // (past the planning budget: the operation is failing, see crate::budget)
        if crate::budget::over() || i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.ny as f64
        {
            return floor;
        }
        let mut best = floor;
        let r = self.cutter.radius();
        let bin = &self.bins[j as usize * self.nx + i as usize];
        // the work: the triangles looked at (at most the bin)
        crate::budget::spend(crate::budget::PER_QUERY + bin.len() as u64);
        for &k in bin {
            let bb = &self.boxes[k as usize];
            if bb[0] > x + r || bb[2] < x - r || bb[1] > y + r || bb[3] < y - r {
                continue;
            }
            let t = &self.tris[k as usize];
            let lt = [
                [t[0][0] - x, t[0][1] - y, t[0][2]],
                [t[1][0] - x, t[1][1] - y, t[1][2]],
                [t[2][0] - x, t[2][1] - y, t[2][2]],
            ];
            if lt.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max) <= best {
                // sorted by top: no later triangle can be higher
                break;
            }
            best = best.max(drop_triangle(&self.cutter, &lt));
        }
        best
    }
}

/// Stepover for a raster: from the scallop target when one is set and the tool has a
/// curved or pointed tip ([`Cutter::scallop_width`]: ball 2 sqrt(h (2R - h)), V-bit
/// 2 h tan(half angle), tapered ball by its ball and then its cone), else the operation's
/// stepover.
pub fn raster_stepover(op: &Raster3d, c: &Cutter) -> f64 {
    c.scallop_width(op.scallop).unwrap_or(op.stepover).max(0.01)
}

/// Clip the line t*u + v*w (u, w unit, perpendicular) to the rectangle; the t range.
fn clip_line(u: [f64; 2], w: [f64; 2], v: f64, x: [f64; 2], y: [f64; 2]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (k, lim) in [(0, x), (1, y)] {
        let p = v * w[k];
        if u[k].abs() < 1e-12 {
            if p < lim[0].min(lim[1]) - 1e-9 || p > lim[0].max(lim[1]) + 1e-9 {
                return None;
            }
        } else {
            let (a, b) = ((lim[0] - p) / u[k], (lim[1] - p) / u[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    (t1 >= t0 - 1e-9).then_some((t0, t1.max(t0)))
}

/// Parallel passes over the part mesh at `angle`, zigzag or one-way, in step-down levels from
/// the top of the stock to the part (+ `leave`). Zigzag rows are linked at depth along the
/// surface; one-way passes lift to the safe height and re-enter from above the stock.
pub fn raster3d(
    b: &mut Builder,
    dc: &DropCutter3,
    stock: &StockBox,
    safe: f64,
    op: &Raster3d,
) -> Result<(), String> {
    raster3d_on(b, dc, None, None, stock, safe, op)
}

/// Material thinner than this (mm) over what a pass leaves is not cut by rest roughing: the
/// finish takes it.
pub const REST_MIN: f64 = 0.05;
/// How far above the material left a rest roughing comes down to, or passes over it (mm).
pub const REST_CLEAR: f64 = 0.3;

/// What the operations before a rest roughing left of the block, as the highest top under a
/// tool of `radius` at each cell (the block's heightmap after them, widened by the tool): a
/// pass at Z cuts material at (x, y) only where this is above Z.
#[derive(Debug, Clone)]
pub struct RestMap {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    pub top: Vec<f64>,
}

impl RestMap {
    /// From the block left (`hm`) and the rest tool's radius.
    pub fn new(hm: &crate::sim3::Heightmap3, radius: f64) -> RestMap {
        let (nx, ny, cell) = (hm.nx, hm.ny, hm.cell);
        let k = (radius / cell).ceil() as isize;
        // the disc's cells, as offsets (its centre cell included)
        let disc: Vec<(isize, isize)> = (-k..=k)
            .flat_map(|dj| (-k..=k).map(move |di| (di, dj)))
            .filter(|&(di, dj)| {
                let (dx, dy) = (
                    (di.unsigned_abs() as f64 - 0.5).max(0.0) * cell,
                    (dj.unsigned_abs() as f64 - 0.5).max(0.0) * cell,
                );
                dx * dx + dy * dy <= radius * radius
            })
            .collect();
        // a row's maximum over each run of cells first, then the disc's rows (separable
        // per row width): kept simple, the disc's offsets directly
        let mut top = vec![f64::NEG_INFINITY; nx * ny];
        for j in 0..ny as isize {
            for i in 0..nx as isize {
                let mut m = f64::NEG_INFINITY;
                for &(di, dj) in &disc {
                    let (a, c) = (i + di, j + dj);
                    if a < 0 || c < 0 || a >= nx as isize || c >= ny as isize {
                        continue;
                    }
                    m = m.max(hm.z[c as usize * nx + a as usize]);
                }
                top[j as usize * nx + i as usize] = m;
            }
        }
        RestMap {
            x0: hm.x0,
            y0: hm.y0,
            cell,
            nx,
            ny,
            top,
        }
    }

    /// [`RestMap::at`] at a point.
    pub fn at_point(&self, p: [f64; 2]) -> f64 {
        self.at(p[0], p[1])
    }

    /// The highest material under the tool centred at (x, y) (beside the block: its edge).
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let fi = (x - self.x0) / self.cell;
        let fj = (y - self.y0) / self.cell;
        if fi < 0.0 || fj < 0.0 || fi >= self.nx as f64 || fj >= self.ny as f64 {
            // (beside the block the tool's edge still reaches it: the nearest cell)
            let i = (fi.max(0.0) as usize).min(self.nx - 1);
            let j = (fj.max(0.0) as usize).min(self.ny - 1);
            return self.top[j * self.nx + i];
        }
        self.top[fj as usize * self.nx + fi as usize]
    }
}

/// [`raster3d`] limited to where the tool rests on the faces in `sel` (a drop-cutter on just
/// those faces, following the whole part through `dc`): it lifts to the safe height between
/// runs, and links rows at depth only when the link stays on the faces.
pub fn raster3d_on(
    b: &mut Builder,
    dc: &DropCutter3,
    sel: Option<&DropCutter3>,
    rest: Option<&RestMap>,
    stock: &StockBox,
    safe: f64,
    op: &Raster3d,
) -> Result<(), String> {
    let [x0, x1] = op.x_range;
    let [y0, y1] = op.y_range;
    let floor = op.floor;
    let d = 2.0 * dc.cutter.radius();
    let step = raster_stepover(op, &dc.cutter);
    let one_way = match op.pattern.as_str() {
        "zigzag" => false,
        "one_way" => true,
        p => return Err(format!("pattern {p:?}: use \"zigzag\" or \"one_way\"")),
    };
    // one-way climb (M3 spindle): the uncut side (+w, the next passes) on the tool's right,
    // so the passes run along -u
    let backwards = match op.direction.as_str() {
        "climb" => true,
        "conventional" => false,
        p => {
            return Err(format!(
                "direction {p:?}: use \"climb\" or \"conventional\""
            ))
        }
    };
    let (sa, ca) = op.angle.to_radians().sin_cos();
    let (u, w) = ([ca, sa], [-sa, ca]);
    let corners = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]];
    let ws: Vec<f64> = corners.iter().map(|c| c[0] * w[0] + c[1] * w[1]).collect();
    let (w0, w1) = (
        ws.iter().cloned().fold(f64::INFINITY, f64::min),
        ws.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    );
    // angle 0: the rows run from y0 to y1, as given
    let (w0, w1) = if op.angle == 0.0 { (y0, y1) } else { (w0, w1) };
    let n = (((w1 - w0).abs() / step).ceil() as usize).max(1);
    // the surface under each point, once for all the levels (the same values every time)
    let cache = std::cell::RefCell::new(std::collections::HashMap::<(u64, u64), f64>::new());
    let surf = |x: f64, y: f64| {
        let key = (x.to_bits(), y.to_bits());
        if let Some(&z) = cache.borrow().get(&key) {
            return z;
        }
        let z = dc.z_at(x, y, floor) + op.leave;
        // (past the budget nothing more is kept: the operation is failing)
        if !crate::budget::over() {
            cache.borrow_mut().insert(key, z);
        }
        z
    };
    let top = stock.max[2];
    let feeds = [op.feed, op.plunge_feed];
    b.comment("--- 3D raster ---");
    for level in levels(top, floor + op.leave, op.step_down.max(d * 0.05)) {
        if crate::budget::over() {
            break;
        }
        // a level that holds the tool up nowhere is the surface pass itself: every deeper
        // level would cut exactly the same path again, so it is the last
        let held = std::cell::Cell::new(false);
        let z_of = |x: f64, y: f64| {
            let s = surf(x, y);
            if level > s {
                held.set(true);
            }
            s.max(level)
        };
        let mut first = true;
        // face selection: at the safe height until the first run on the faces
        let mut lifted = true;
        // rest roughing: the tool left down where the material ended (see masked_row)
        let mut air: Option<[f64; 2]> = None;
        let mut emitted = 0usize;
        for k in 0..=n {
            if crate::budget::over() {
                break;
            }
            let wk = w0 + (w1 - w0) * k as f64 / n as f64;
            let row: Vec<[f64; 2]> = if op.angle == 0.0 {
                let m = (((x1 - x0).abs() / op.sample.max(0.01)).ceil() as usize).max(1);
                (0..=m)
                    .map(|i| [x0 + (x1 - x0) * i as f64 / m as f64, wk])
                    .collect()
            } else {
                let Some((t0, t1)) = clip_line(u, w, wk, [x0, x1], [y0, y1]) else {
                    continue;
                };
                let m = (((t1 - t0) / op.sample.max(0.01)).ceil() as usize).max(1);
                (0..=m)
                    .map(|i| {
                        let t = t0 + (t1 - t0) * i as f64 / m as f64;
                        [t * u[0] + wk * w[0], t * u[1] + wk * w[1]]
                    })
                    .collect()
            };
            let reverse = if one_way { backwards } else { emitted % 2 == 1 };
            emitted += 1;
            let row: Vec<[f64; 2]> = if reverse {
                row.into_iter().rev().collect()
            } else {
                row
            };
            if sel.is_some() || rest.is_some() {
                // on the selected faces, and (rest roughing) where material is left above
                // the pass
                let inside = |p: [f64; 2]| {
                    sel.is_none_or(|s| on_sel(dc, s, p[0], p[1]))
                        && rest.is_none_or(|r| r.at(p[0], p[1]) > z_of(p[0], p[1]) + REST_MIN)
                };
                masked_row(
                    b,
                    &inside,
                    &z_of,
                    &row,
                    safe,
                    top,
                    op,
                    &mut lifted,
                    rest,
                    &mut air,
                );
                continue;
            }
            let [sx, sy] = row[0];
            let z0 = z_of(sx, sy);
            if first {
                // one entry per level (per pass one-way), from above the uncut stock, so the
                // tool never drops beside material outside the raster area
                enter(b, safe, sx, sy, top + 1.0, z0, op.plunge_feed);
                first = false;
            } else if one_way {
                leave(b, safe);
                enter(b, safe, sx, sy, top + 1.0, z0, op.plunge_feed);
            } else {
                let p = b.p3.expect("inside a level");
                line_refined(b, &z_of, p, [sx, sy, z0], op.tolerance, 0, feeds);
            }
            for q in row.windows(2) {
                // split where the surface rises above the straight move
                line_refined(
                    b,
                    &z_of,
                    [q[0][0], q[0][1], z_of(q[0][0], q[0][1])],
                    [q[1][0], q[1][1], z_of(q[1][0], q[1][1])],
                    op.tolerance,
                    0,
                    feeds,
                );
            }
        }
        leave(b, safe);
        if !held.get() {
            break;
        }
    }
    Ok(())
}

/// Where the tool rests on the selected faces (see [`super::rotary_finish::on_faces`]).
fn on_sel(dc: &DropCutter3, sel: &DropCutter3, x: f64, y: f64) -> bool {
    super::rotary_finish::on_faces(
        dc.z_at(x, y, f64::NEG_INFINITY),
        sel.z_at(x, y, f64::NEG_INFINITY),
        f64::NEG_INFINITY,
    )
}

/// About how fast the machine rapids (mm/min), for choosing between a link at depth and a
/// lift: a guess on the slow side.
const RAPID_GUESS: f64 = 3000.0;
/// And what each rapid costs besides its length (minutes: about 0.15 s of speeding up and
/// slowing down).
const RAPID_OVERHEAD: f64 = 0.0025;

/// One raster row on selected faces, or (rest roughing) where material is left: cut its runs
/// there, finding their edges to 1 um, lifting between runs; a row's first run is reached at
/// depth from the last one when the straight link stays on the faces. With `rest`, the tool
/// comes down from just above the material left (not the block's top), and crosses a gap at
/// depth when that is quicker than lifting (nothing is left there to cut).
#[allow(clippy::too_many_arguments)]
fn masked_row(
    b: &mut Builder,
    inside: &dyn Fn([f64; 2]) -> bool,
    z_of: &dyn Fn(f64, f64) -> f64,
    row: &[[f64; 2]],
    safe: f64,
    top: f64,
    op: &Raster3d,
    lifted: &mut bool,
    rest: Option<&RestMap>,
    air: &mut Option<[f64; 2]>,
) {
    let feeds = [op.feed, op.plunge_feed];
    let lerp =
        |p: [f64; 2], q: [f64; 2], t: f64| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
    let dist = |p: [f64; 2], q: [f64; 2]| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
    let at = |p: [f64; 2]| [p[0], p[1], z_of(p[0], p[1])];
    // where a descent starts: above the block, or just above what is left there
    let start = |p: [f64; 2], z: f64| {
        rest.map_or(top + 1.0, |r| {
            (r.at(p[0], p[1]) + REST_CLEAR).min(top + 1.0)
        })
        .max(z)
    };
    let mut prev: Option<[f64; 2]> = None;
    for &q in row {
        let here = inside(q);
        match (*lifted, here) {
            (false, true) => {
                let from = b.p3.expect("cutting");
                // from the last row's end: at depth only if the link stays on the faces (rest
                // roughing: always, as a full pass would)
                let link_ok = prev.is_some() || rest.is_some() || {
                    let a = [from[0], from[1]];
                    let n = ((dist(a, q) / op.sample.max(0.01)).ceil() as usize).max(1);
                    (1..n).all(|k| inside(lerp(a, q, k as f64 / n as f64)))
                };
                if link_ok {
                    line_refined(b, z_of, from, at(q), op.tolerance, 0, feeds);
                } else {
                    leave(b, safe);
                    let z = z_of(q[0], q[1]);
                    enter(b, safe, q[0], q[1], start(q, z), z, op.plunge_feed);
                }
            }
            (true, false) => {}
            (false, false) => {
                // leaving: cut to the edge, then up (rest roughing: maybe not)
                if let Some(p) = prev {
                    let e = super::rotary_finish::boundary(p, q, true, &lerp, inside, dist(p, q));
                    let from = b.p3.expect("cutting");
                    line_refined(b, z_of, from, at(e), op.tolerance, 0, feeds);
                }
                if rest.is_some() {
                    *air = b.p3.map(|p| [p[0], p[1]]);
                } else {
                    leave(b, safe);
                }
                *lifted = true;
            }
            (true, true) => {
                // onto the faces (or the material): at their edge
                let e = prev.map_or(q, |p| {
                    super::rotary_finish::boundary(p, q, false, &lerp, inside, dist(p, q))
                });
                let z = z_of(e[0], e[1]);
                let from = b.p3;
                match (air.take(), from, rest) {
                    (Some(a), Some(f), Some(r)) => {
                        // across the gap at depth, or over it just above what is left along
                        // the way: whichever is quicker
                        let s = start(e, z);
                        let n = ((dist(a, e) / r.cell).ceil() as usize).max(1);
                        let hop = (0..=n)
                            .map(|k| r.at_point(lerp(a, e, k as f64 / n as f64)))
                            .fold(f[2].max(s), f64::max)
                            + REST_CLEAR;
                        let hop = hop.min(safe);
                        let lift = ((hop - f[2]) + dist(a, e) + (hop - s)) / RAPID_GUESS
                            + 3.0 * RAPID_OVERHEAD
                            + (s - z) / op.plunge_feed.max(1e-6);
                        if dist(a, e) / op.feed.max(1e-6) <= lift {
                            line_refined(b, z_of, f, at(e), op.tolerance, 0, feeds);
                        } else {
                            b.rapid3(None, None, Some(hop));
                            b.rapid3(Some(e[0]), Some(e[1]), None);
                            b.rapid3(None, None, Some(s));
                            b.line3(e[0], e[1], z, UP, op.plunge_feed);
                        }
                    }
                    _ => enter(b, safe, e[0], e[1], start(e, z), z, op.plunge_feed),
                }
                *lifted = false;
                if dist(e, q) > 1e-9 {
                    line_refined(b, z_of, at(e), at(q), op.tolerance, 0, feeds);
                }
            }
        }
        prev = Some(q);
    }
    // one-way passes always come back from above
    if op.pattern == "one_way" && !*lifted {
        leave(b, safe);
        *lifted = true;
    }
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
        descent_feed(p0, p1, feed[0], feed[1], b.ramp_tan),
    );
}

/// Feed for a move from `p0` to `p1`: a descent no steeper than the ramp (`ramp_tan` =
/// tan(ramp angle)) runs at `feed`; a steeper one so Z never falls faster than `plunge`
/// mm/min, even after the post rounds coordinates to 0.001 mm (whole mm/min, rounded down).
pub fn descent_feed(p0: [f64; 3], p1: [f64; 3], feed: f64, plunge: f64, ramp_tan: f64) -> f64 {
    const ROUND: f64 = 1e-3;
    let dz = p0[2] - p1[2];
    if dz <= -ROUND {
        return feed;
    }
    let across = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
    // the post's rounding may make a move up to 2 microns steeper than planned
    if crate::toolpath::gentle_descent(dz + 2.0 * ROUND, across, ramp_tan) {
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
