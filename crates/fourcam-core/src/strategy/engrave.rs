//! Engraving (follow paths and single-line text at a depth) and V-carving (depth from the
//! V-bit angle and the distance to the outline), on a vise (X/Y/Z) or wrapped round the
//! rotary axis (path X along the axis, path Y as arc length round the surface radius `top`,
//! centred on the top of the bar at A0).
//!
//! V-carving is exact for a V-bit: the tip at depth d(p) / tan(half angle), d the distance to
//! the outline, never cuts below that surface anywhere, because d changes by at most the
//! distance moved (it is 1-Lipschitz), while the cone's sides rise by distance / tan.

use crate::engrave::{text_paths, Path2, Region};
use crate::job::{Engrave, Vcarve};
use crate::toolpath::Builder;

const UP: [f64; 3] = [0.0, 0.0, 1.0];

/// Where the cut goes: on the vise (Z up) or wrapped round the rotary axis.
#[derive(Debug, Clone, Copy)]
pub enum Surface {
    /// Flat top at Z `top`.
    Flat { top: f64 },
    /// Cylinder of radius `top` round X.
    Wrapped { top: f64 },
}

/// The operation's polylines: its paths plus its text.
pub fn engrave_paths(op: &Engrave) -> Result<Vec<Path2>, String> {
    let mut out: Vec<Path2> = op.paths.iter().filter(|p| p.len() >= 2).cloned().collect();
    if !op.text.trim().is_empty() {
        out.extend(text_paths(
            &op.text,
            op.text_height,
            op.text_at,
            op.text_rotate,
        )?);
    }
    if out.is_empty() {
        return Err("engrave needs text or paths".into());
    }
    Ok(out)
}

/// A point of the path at depth `d` below the surface: tool tip (x, y, z) on the vise, or
/// (x, radius, A degrees) wrapped.
fn place(s: Surface, p: [f64; 2], d: f64) -> [f64; 3] {
    match s {
        Surface::Flat { top } => [p[0], p[1], top - d],
        Surface::Wrapped { top } => [p[0], top - d, (p[1] / top).to_degrees()],
    }
}

fn go_to(b: &mut Builder, s: Surface, safe: f64, at: [f64; 3], plunge: f64) {
    match s {
        Surface::Flat { top } => {
            b.rapid3(None, None, Some(safe));
            b.rapid3(Some(at[0]), Some(at[1]), None);
            b.rapid3(None, None, Some(top + 1.0));
            b.line3(at[0], at[1], at[2], UP, plunge);
        }
        Surface::Wrapped { top } => {
            b.rapid_r(safe);
            b.rapid(Some(at[0]), None, Some(at[2]));
            b.rapid_r(top + 1.0);
            b.plunge(at[1], plunge);
        }
    }
}

fn cut(b: &mut Builder, s: Surface, at: [f64; 3], feed: f64, plunge: f64) {
    match s {
        Surface::Flat { .. } => {
            // down a V's side no faster than the plunge feed
            let f = b.p3.map_or(feed, |p| {
                super::three_axis::descent_feed(p, at, feed, plunge)
            });
            b.line3(at[0], at[1], at[2], UP, f)
        }
        Surface::Wrapped { .. } => b.cut(at[0], at[1], at[2], feed, plunge),
    }
}

fn lift(b: &mut Builder, s: Surface, safe: f64) {
    match s {
        Surface::Flat { .. } => b.rapid3(None, None, Some(safe)),
        Surface::Wrapped { .. } => b.rapid_r(safe),
    }
}

/// Follow every path at each depth level down to `depth`.
pub fn engrave(b: &mut Builder, s: Surface, safe: f64, op: &Engrave) -> Result<(), String> {
    if op.depth <= 0.0 {
        return Err("engrave depth must be > 0".into());
    }
    let paths = engrave_paths(op)?;
    let levels = super::three_axis::levels(0.0, -op.depth, op.step_down.max(0.01));
    b.comment("--- engrave ---");
    for path in &paths {
        for &l in &levels {
            let d = -l;
            go_to(b, s, safe, place(s, path[0], d), op.plunge_feed);
            for q in &path[1..] {
                // long straight segments stay straight on the vise; wrapped they become
                // helices on the surface, exactly the straight line in the unrolled view
                cut(b, s, place(s, *q, d), op.feed, op.plunge_feed);
            }
            lift(b, s, safe);
        }
    }
    Ok(())
}

/// V-carve the closed paths with a V-bit of `half_angle` (radians): rows `stepover` apart
/// across the region, points `sample` apart along them, each at depth
/// min(distance to the outline / tan(half angle), max_depth).
pub fn vcarve(
    b: &mut Builder,
    s: Surface,
    safe: f64,
    half_angle: f64,
    op: &Vcarve,
) -> Result<(), String> {
    let tan = half_angle.tan();
    if tan.is_nan() || tan <= 0.0 {
        return Err("vcarve needs a V-bit with an angle".into());
    }
    let reach = op.max_depth * tan;
    let region = Region::new(&op.paths, reach);
    if region.is_empty() {
        return Err("vcarve needs closed paths (3 or more points each)".into());
    }
    let depth_at = |p: [f64; 2]| (region.distance(p, reach) / tan).min(op.max_depth);
    let step = op.stepover.max(0.005);
    let n = (((region.hi[1] - region.lo[1]) / step).ceil() as usize).max(1);
    b.comment("--- V-carve ---");
    for k in 0..=n {
        let y = region.lo[1] + (region.hi[1] - region.lo[1]) * k as f64 / n as f64;
        let mut spans = region.spans(y);
        if k % 2 == 1 {
            spans.reverse();
        }
        for (x0, x1) in spans {
            let (x0, x1) = if k % 2 == 1 { (x1, x0) } else { (x0, x1) };
            let m = (((x1 - x0).abs() / op.sample.max(0.005)).ceil() as usize).max(1);
            let pts: Vec<[f64; 2]> = (0..=m)
                .map(|i| [x0 + (x1 - x0) * i as f64 / m as f64, y])
                .collect();
            go_to(
                b,
                s,
                safe,
                place(s, pts[0], depth_at(pts[0])),
                op.plunge_feed,
            );
            for w in pts.windows(2) {
                refined(b, s, &depth_at, w[0], w[1], 0, op);
            }
            lift(b, s, safe);
        }
    }
    Ok(())
}

/// Cut from `a` to `p` (already at `a`), splitting where the straight move would go deeper
/// than the target between the points.
fn refined(
    b: &mut Builder,
    s: Surface,
    depth_at: &dyn Fn([f64; 2]) -> f64,
    a: [f64; 2],
    p: [f64; 2],
    level: u32,
    op: &Vcarve,
) {
    let mid = [(a[0] + p[0]) / 2.0, (a[1] + p[1]) / 2.0];
    if level < 10 && depth_at(mid) < (depth_at(a) + depth_at(p)) / 2.0 - op.tolerance {
        refined(b, s, depth_at, a, mid, level + 1, op);
        refined(b, s, depth_at, mid, p, level + 1, op);
        return;
    }
    cut(b, s, place(s, p, depth_at(p)), op.feed, op.plunge_feed);
}
