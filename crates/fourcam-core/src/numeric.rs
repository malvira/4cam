//! Numeric helpers that reproduce NumPy's results bit for bit, so generated toolpaths match the
//! reference Python implementation exactly.

use std::f64::consts::PI;

/// `numpy.arange(start, stop, step)` for floats.
pub fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let len = ((stop - start) / step).ceil();
    if len.is_nan() || len <= 0.0 {
        return Vec::new();
    }
    let len = len as usize;
    let delta = (start + step) - start;
    (0..len)
        .map(|i| match i {
            0 => start,
            1 => start + step,
            _ => start + i as f64 * delta,
        })
        .collect()
}

/// `numpy.linspace(start, stop, num)` (endpoint included).
pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    match num {
        0 => Vec::new(),
        1 => vec![start],
        _ => {
            let step = (stop - start) / (num - 1) as f64;
            let mut v: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
            v[num - 1] = stop;
            v
        }
    }
}

/// `numpy.interp(x, xp, fp)` for a scalar `x`; `xp` must be ascending.
pub fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    assert!(n > 0 && n == fp.len(), "interp: bad table");
    if x < xp[0] {
        return fp[0];
    }
    if x >= xp[n - 1] {
        return fp[n - 1];
    }
    // largest j with xp[j] <= x
    let j = xp.partition_point(|&v| v <= x) - 1;
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    let r = slope * (x - xp[j]) + fp[j];
    if r.is_nan() {
        slope * (x - xp[j + 1]) + fp[j + 1]
    } else {
        r
    }
}

/// `numpy.round(x, decimals)`: scale, round half to even, unscale.
pub fn round_half_even(x: f64, decimals: i32) -> f64 {
    let s = 10f64.powi(decimals);
    (x * s).round_ties_even() / s
}

/// `numpy.radians`.
pub fn rad(deg: f64) -> f64 {
    deg * (PI / 180.0)
}

/// `numpy.degrees`.
pub fn deg(rad: f64) -> f64 {
    rad * (180.0 / PI)
}

/// Python's builtin `max(a, b)`: the first argument wins ties.
pub fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arange_len_and_values() {
        let v = arange(-1.5875, 1.5875 + 1e-9, 0.01);
        assert_eq!(v.len(), 318);
        assert_eq!(v[0], -1.5875);
        assert_eq!(arange(0.0, 1.0, 0.25), vec![0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn linspace_endpoint() {
        let v = linspace(-15.6, 0.6, 120);
        assert_eq!(v.len(), 120);
        assert_eq!(v[119], 0.6);
        assert_eq!(v[0], -15.6);
    }

    #[test]
    fn interp_edges() {
        let xp = [0.0, 1.0, 2.0];
        let fp = [0.0, 10.0, 0.0];
        assert_eq!(interp(-1.0, &xp, &fp), 0.0);
        assert_eq!(interp(0.5, &xp, &fp), 5.0);
        assert_eq!(interp(1.0, &xp, &fp), 10.0);
        assert_eq!(interp(2.0, &xp, &fp), 0.0);
        assert_eq!(interp(3.0, &xp, &fp), 0.0);
    }

    #[test]
    fn rounding() {
        assert_eq!(round_half_even(2.5, 0), 2.0);
        assert_eq!(round_half_even(3.5, 0), 4.0);
        assert_eq!(round_half_even(-0.02, 4), -0.02);
    }
}
