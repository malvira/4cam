//! Part geometry.

use crate::job::Profile;

/// A profile revolved about the X (rotary) axis.
#[derive(Debug, Clone)]
pub struct RevolvedProfile {
    points: Vec<[f64; 2]>,
    stock_r: f64,
    beyond_front: f64,
}

impl RevolvedProfile {
    pub fn new(profile: &Profile, stock_r: f64) -> RevolvedProfile {
        let points = profile.points.clone();
        let beyond_front = profile
            .beyond_front
            .or_else(|| points.last().map(|p| p[1]))
            .unwrap_or(stock_r);
        RevolvedProfile {
            points,
            stock_r,
            beyond_front,
        }
    }

    pub fn x_min(&self) -> f64 {
        self.points.first().map_or(0.0, |p| p[0])
    }

    pub fn x_max(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p[0])
    }

    pub fn min_radius(&self) -> f64 {
        self.points
            .iter()
            .map(|p| p[1])
            .fold(self.stock_r, f64::min)
    }

    /// Finished radius at axial position `x`. Stock radius behind the first point.
    pub fn radius(&self, x: f64) -> f64 {
        let p = &self.points;
        if p.is_empty() || x < p[0][0] {
            return self.stock_r;
        }
        let last = p[p.len() - 1];
        if x > last[0] {
            return self.beyond_front;
        }
        // last point with px <= x: at a step (two points with the same x) the +x side wins
        let i = p.partition_point(|q| q[0] <= x) - 1;
        let [x0, r0] = p[i];
        if x == x0 || i + 1 == p.len() {
            return r0;
        }
        let [x1, r1] = p[i + 1];
        r0 + (r1 - r0) * (x - x0) / (x1 - x0)
    }

    /// Lowest tool-centre radius for a flat endmill at `x`: the highest profile point under
    /// the flat bottom, sampled at `offsets` (gouge-free).
    pub fn flat_drop(&self, x: f64, offsets: &[f64]) -> f64 {
        offsets
            .iter()
            .map(|o| self.radius(x + o))
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knob() -> RevolvedProfile {
        let p = Profile {
            points: vec![
                [-21.7, 3.0],
                [-18.0, 3.0],
                [-18.0, 5.0],
                [-15.0, 5.0],
                [-15.0, 10.0],
                [-0.8, 8.106666666666667],
                [0.0, 7.2],
            ],
            beyond_front: Some(7.2),
        };
        RevolvedProfile::new(&p, 12.5)
    }

    #[test]
    fn radius_steps_and_ends() {
        let g = knob();
        assert_eq!(g.radius(-30.0), 12.5);
        assert_eq!(g.radius(-21.7), 3.0);
        assert_eq!(g.radius(-18.0), 5.0);
        assert_eq!(g.radius(-15.0), 10.0);
        assert_eq!(g.radius(0.0), 7.2);
        assert_eq!(g.radius(1.0), 7.2);
        assert!((g.radius(-7.5) - 9.0).abs() < 1e-3);
    }

    #[test]
    fn flat_drop_sees_wall() {
        let g = knob();
        assert_eq!(g.flat_drop(-16.0, &[-1.0, 0.0, 1.0]), 10.0);
    }
}
