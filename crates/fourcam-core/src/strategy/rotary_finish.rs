//! Mesh finishing with a radial drop-cutter: rasters along the axis (constant A per pass) or a
//! helix around it.

use crate::dropcutter::DropCutter;
use crate::job::{FinishPattern, RotaryFinish};
use crate::toolpath::{Builder, Move};

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
    let cut = |b: &mut Builder, x: f64, a: f64| b.cut(x, cl(x, a), a, op.feed, op.plunge_feed);
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
