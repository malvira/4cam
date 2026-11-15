//! Mesh finishing with a radial drop-cutter: rasters along the axis (constant A per pass) or a
//! helix around it.

use crate::dropcutter::DropCutter;
use crate::job::{FinishPattern, RotaryFinish};
use crate::toolpath::{Builder, Move};

/// Emit a straight cut from `p0` to `p1` (`[x, r, a]`), split until the surface under the
/// move stays within `tol` below it (checked at the quarter points).
fn refine(
    b: &mut Builder,
    cl: &dyn Fn(f64, f64) -> f64,
    p0: [f64; 3],
    p1: [f64; 3],
    tol: f64,
    depth: u32,
    op: &RotaryFinish,
) {
    let at = |t: f64| {
        [
            p0[0] + (p1[0] - p0[0]) * t,
            p0[1] + (p1[1] - p0[1]) * t,
            p0[2] + (p1[2] - p0[2]) * t,
        ]
    };
    // highest surface point found above the straight move
    let mut top = f64::NEG_INFINITY;
    let mut rises = false;
    for t in [0.25, 0.5, 0.75] {
        let p = at(t);
        let need = cl(p[0], p[2]);
        top = top.max(need);
        rises |= need > p[1] + tol;
    }
    if rises && depth < 14 {
        let m = at(0.5);
        let mid = [m[0], cl(m[0], m[2]), m[2]];
        refine(b, cl, p0, mid, tol, depth + 1, op);
        refine(b, cl, mid, p1, tol, depth + 1, op);
        return;
    }
    if rises {
        // a wall (the surface jumps within a few microns): climb, cross, then descend
        let top = top.max(p0[1]).max(p1[1]);
        b.cut(p0[0], top, p0[2], op.feed, op.plunge_feed);
        b.cut(p1[0], top, p1[2], op.feed, op.plunge_feed);
    }
    b.cut(p1[0], p1[1], p1[2], op.feed, op.plunge_feed);
}

/// Plan a finishing pass. `r_max` is the part's largest radius (for `stepover_mm` in `along`).
/// Returns the lowest tip radius used.
pub fn rotary_finish(
    b: &mut Builder,
    dc: &DropCutter,
    r_max: f64,
    safe_r: f64,
    op: &RotaryFinish,
) -> Result<f64, String> {
    let floor = op.min_radius.max(0.0);
    let cl = |x: f64, a: f64| dc.radius_at(x, a, floor) + op.leave;
    let [x0, x1] = op.x_range;
    if op.sample_mm <= 0.0 || op.sample_deg <= 0.0 {
        return Err("sample_mm and sample_deg must be > 0".into());
    }
    let first = b.moves.len();
    let tol = op.tolerance.max(1e-4);
    // feed to (x, a), subdividing where the surface rises above the straight move
    let cut = |b: &mut Builder, x: f64, a: f64| {
        let (xs, rs, as_) = (b.x.unwrap_or(x), b.r.unwrap_or(0.0), b.a);
        refine(b, &cl, [xs, rs, as_], [x, cl(x, a), a], tol, 0, op);
    };
    match op.pattern {
        FinishPattern::Along => {
            let step = match (op.stepover_deg, op.stepover_mm) {
                (Some(d), _) => d,
                (None, Some(mm)) => (mm / r_max.max(1e-9)).to_degrees(),
                (None, None) => return Err("along needs stepover_deg or stepover_mm".into()),
            };
            if step <= 0.0 {
                return Err("stepover must be > 0".into());
            }
            let [a0, a1] = op.a_range;
            let span = a1 - a0;
            let angles: Vec<f64> = if span.abs() >= 360.0 - 1e-9 {
                let n = (360.0 / step).ceil().max(1.0) as usize;
                (0..n)
                    .map(|i| a0 + span.signum() * 360.0 * i as f64 / n as f64)
                    .collect()
            } else {
                let n = (span.abs() / step).ceil() as usize + 1;
                (0..n)
                    .map(|i| a0 + span * i as f64 / (n - 1).max(1) as f64)
                    .collect()
            };
            let m = ((x1 - x0).abs() / op.sample_mm).ceil().max(1.0) as usize;
            let xs: Vec<f64> = (0..=m)
                .map(|i| x0 + (x1 - x0) * i as f64 / m as f64)
                .collect();
            b.rapid_r(safe_r);
            b.rapid(Some(xs[0]), None, Some(angles[0]));
            b.plunge(cl(xs[0], angles[0]), op.plunge_feed);
            for (i, &a) in angles.iter().enumerate() {
                let forward = i % 2 == 0;
                let pass: Vec<f64> = if forward {
                    xs.clone()
                } else {
                    xs.iter().rev().copied().collect()
                };
                for &x in &pass {
                    cut(b, x, a);
                }
                if let Some(&next) = angles.get(i + 1) {
                    let x_end = *pass.last().unwrap();
                    let k = ((next - a).abs() / op.sample_deg).ceil().max(1.0) as usize;
                    for j in 1..=k {
                        cut(b, x_end, a + (next - a) * j as f64 / k as f64);
                    }
                }
            }
            b.rapid_r(safe_r);
        }
        FinishPattern::Around => {
            let step = op
                .stepover_mm
                .ok_or("around needs stepover_mm (X advance per turn)")?;
            if step <= 0.0 {
                return Err("stepover_mm must be > 0".into());
            }
            let a0 = op.a_range[0];
            let n_rev = (360.0 / op.sample_deg).ceil() as usize;
            let seg = 360.0 / n_rev as f64;
            b.rapid_r(safe_r);
            b.rapid(Some(x0), None, Some(a0));
            b.plunge(cl(x0, a0), op.plunge_feed);
            for _ in 0..n_rev {
                let a = b.a + seg;
                cut(b, x0, a);
            }
            let len = (x1 - x0).abs();
            if len > 1e-9 {
                let n = ((len / step * 360.0 / seg).ceil() as usize).max(1);
                let da = 360.0 * len / step / n as f64;
                for i in 1..=n {
                    let x = x0 + (x1 - x0) * i as f64 / n as f64;
                    let a = b.a + da;
                    cut(b, x, a);
                }
            }
            for _ in 0..n_rev {
                let a = b.a + seg;
                cut(b, x1, a);
            }
            b.rapid_r(safe_r);
        }
    }
    let r_min = b.moves[first..]
        .iter()
        .filter_map(|m| match m {
            Move::Cut { r, .. } | Move::Plunge { r, .. } => Some(*r),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    Ok(r_min)
}
