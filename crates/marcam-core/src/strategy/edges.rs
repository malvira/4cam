//! 3-axis edge work: chamfering (and deburring) the top edges of outlines and holes with a
//! V-bit, and thread milling holes or bosses with a thread mill on a helix.
//!
//! A chamfer of `width` with a V-bit of half angle α: the tip goes `width / tan α` below the
//! top plus a clearance (so the cone cuts, not the point), its axis `clearance · tan α` past
//! the edge into the waste; the cone then meets the top `width` from the edge (and a 90° bit
//! cuts as far down the wall).
//!
//! A thread: the tool's profile (its cutting radius at its tip) is taken out to the thread's
//! depth in radial passes, each a helix one pitch a turn: up while turning counter-clockwise
//! for a right-hand thread. Climb milling (spindle clockwise) turns counter-clockwise inside
//! a hole and clockwise round a boss; the helix's handedness then sets up or down.

use crate::det::Det;
use crate::job::{Chamfer, StockBox, ThreadMill};
use crate::toolpath::Builder;

const UP: [f64; 3] = [0.0, 0.0, 1.0];

fn tau() -> f64 {
    std::f64::consts::TAU
}

/// Chamfer the top edges of `op`'s outlines and holes with a V-bit of `half_angle` (radians).
pub fn chamfer(
    b: &mut Builder,
    half_angle: f64,
    stock: &StockBox,
    safe: f64,
    op: &Chamfer,
) -> Result<(), String> {
    if !(op.width > 0.0) {
        return Err("chamfer: the width must be more than 0".into());
    }
    if op.paths.is_empty() && op.holes.is_empty() {
        return Err("chamfer: give the outlines (paths) or the holes to chamfer".into());
    }
    let t = half_angle.dtan();
    if !(t > 1e-3 && t < 1e3) {
        return Err("chamfer: the V-bit's angle must be between 0 and 180 degrees".into());
    }
    let top = op.top.unwrap_or(stock.max[2]);
    let h = op.width / t + op.tip_clearance.max(0.0);
    let o = op.tip_clearance.max(0.0) * t;
    let z = top - h;
    let climb = op.direction != "conventional";
    let outside = op.side != "inside";
    b.comment("--- chamfer ---");
    for path in &op.paths {
        let mut p: Vec<[f64; 2]> = path.clone();
        if p.len() > 2 && p.first() == p.last() {
            p.pop();
        }
        if p.len() < 3 {
            return Err("chamfer: an outline needs three points or more".into());
        }
        // counter-clockwise, then the axis past the edge into the waste
        let area: f64 = (0..p.len())
            .map(|i| {
                let (a, c) = (p[i], p[(i + 1) % p.len()]);
                a[0] * c[1] - c[0] * a[1]
            })
            .sum();
        if area < 0.0 {
            p.reverse();
        }
        let mut lp = super::three_axis::offset_polygon(&p, if outside { o } else { -o });
        // climb: an outline clockwise, a pocket's edge counter-clockwise
        if outside == climb {
            lp.reverse();
        }
        let s = lp[0];
        b.rapid3(None, None, Some(safe));
        b.rapid3(Some(s[0]), Some(s[1]), None);
        b.rapid3(None, None, Some(top + 1.0));
        b.line3(s[0], s[1], z, UP, op.plunge_feed);
        for q in lp.iter().skip(1).chain(std::iter::once(&s)) {
            b.line3(q[0], q[1], z, UP, op.feed);
        }
        b.rapid3(None, None, Some(safe));
    }
    for hole in &op.holes {
        let [cx, cy, d] = *hole;
        let r = d / 2.0 - o;
        if r <= 0.0 {
            return Err(format!(
                "chamfer: the hole at ({cx}, {cy}) is too small ({d} mm) for this V-bit's clearance"
            ));
        }
        // climb inside a hole: counter-clockwise
        let s = if climb { 1.0 } else { -1.0 };
        let n = ((tau() * r / 0.2).ceil() as usize).clamp(24, 360);
        b.rapid3(None, None, Some(safe));
        b.rapid3(Some(cx + r), Some(cy), None);
        b.rapid3(None, None, Some(top + 1.0));
        b.line3(cx + r, cy, z, UP, op.plunge_feed);
        for k in 1..=n {
            let a = s * tau() * k as f64 / n as f64;
            b.line3(cx + r * a.dcos(), cy + r * a.dsin(), z, UP, op.feed);
        }
        b.rapid3(None, None, Some(safe));
    }
    Ok(())
}

/// Thread-mill `op`'s holes (or bosses) with a tool cutting at radius `tool_r` at its tip.
pub fn thread_mill(
    b: &mut Builder,
    tool_r: f64,
    stock: &StockBox,
    safe: f64,
    op: &ThreadMill,
) -> Result<(), String> {
    if !(op.pitch > 0.0 && op.diameter > 0.0 && op.depth > 0.0) {
        return Err("thread mill: the pitch, diameter and depth must be more than 0".into());
    }
    if op.points.is_empty() {
        return Err("thread mill: give the threads' centres (points)".into());
    }
    let right = match op.hand.as_str() {
        "right" => true,
        "left" => false,
        h => return Err(format!("thread mill: hand {h:?}: right or left")),
    };
    let climb = op.direction != "conventional";
    // ISO metric: 5/8 H inside a nut, 17/24 H on a bolt (H = 0.866 P)
    let h = op
        .thread_depth
        .unwrap_or(if op.internal { 0.5413 } else { 0.6134 } * op.pitch)
        .max(0.0);
    let half = op.diameter / 2.0;
    // the tool centre's radius at the final pass
    let r_final = if op.internal {
        half - tool_r
    } else {
        half - h + tool_r
    };
    if r_final <= 0.0 {
        return Err(format!(
            "thread mill: the tool (Ø{:.2}) is too big for a {} mm thread",
            2.0 * tool_r,
            op.diameter
        ));
    }
    // inside a hole the tool turns counter-clockwise to climb, round a boss clockwise
    let ccw = op.internal == climb;
    let up = ccw == right;
    let passes = op.passes.max(1);
    // radial passes: from the minor (inside) or the major (outside) diameter to the depth
    let radii: Vec<f64> = (1..=passes)
        .map(|k| {
            let f = k as f64 / passes as f64;
            if op.internal {
                r_final - h * (1.0 - f)
            } else {
                r_final + h * (1.0 - f)
            }
        })
        .chain(std::iter::repeat_n(r_final, op.spring_passes as usize))
        .collect();
    let top = op.top.unwrap_or(stock.max[2]);
    // the thread's length, and half a pitch past each end
    let (z_lo, z_hi) = (top - op.depth - 0.5 * op.pitch, top + 0.5 * op.pitch);
    let turns = ((z_hi - z_lo) / op.pitch).ceil().max(1.0);
    let seg = 36usize;
    let sign = if ccw { 1.0 } else { -1.0 };
    b.comment("--- thread milling ---");
    for &[cx, cy] in &op.points {
        for &r in &radii {
            if crate::budget::over() {
                return Ok(());
            }
            let (z0, z1) = if up { (z_lo, z_hi) } else { (z_hi, z_lo) };
            let at = |a: f64, z: f64| [cx + r * a.dcos(), cy + r * a.dsin(), z];
            b.rapid3(None, None, Some(safe));
            if op.internal {
                // into the hole at its centre, then out to the thread on a half circle
                b.rapid3(Some(cx), Some(cy), None);
                b.rapid3(None, None, Some(top + 1.0));
                b.line3(cx, cy, z0, UP, op.plunge_feed);
                for k in 1..=seg / 2 {
                    let t = std::f64::consts::PI * k as f64 / (seg / 2) as f64;
                    let a = std::f64::consts::PI + sign * t;
                    let p = [cx + 0.5 * r + 0.5 * r * a.dcos(), cy + 0.5 * r * a.dsin()];
                    b.line3(p[0], p[1], z0, UP, op.feed);
                }
            } else {
                // from outside the boss, straight in
                let out = r + 2.0 * tool_r + 1.0;
                b.rapid3(Some(cx + out), Some(cy), None);
                b.rapid3(None, None, Some(top + 1.0));
                b.line3(cx + out, cy, z0, UP, op.plunge_feed);
                b.line3(cx + r, cy, z0, UP, op.feed);
            }
            // the helix: a pitch a turn
            let n = (turns as usize) * seg;
            for k in 1..=n {
                let f = k as f64 / n as f64;
                let a = sign * tau() * turns * f;
                let p = at(a, z0 + (z1 - z0) * f);
                b.line3(p[0], p[1], p[2], UP, op.feed);
            }
            if op.internal {
                // back to the centre on a half circle
                for k in 1..=seg / 2 {
                    let t = std::f64::consts::PI * k as f64 / (seg / 2) as f64;
                    let a = sign * t;
                    let p = [cx + 0.5 * r + 0.5 * r * a.dcos(), cy + 0.5 * r * a.dsin()];
                    b.line3(p[0], p[1], z1, UP, op.feed);
                }
            } else {
                b.line3(cx + r + 2.0 * tool_r + 1.0, cy, z1, UP, op.feed);
            }
            b.rapid3(None, None, Some(safe));
        }
    }
    Ok(())
}
