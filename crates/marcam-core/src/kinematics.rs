//! General moves: a tool tip and a tool direction in the part frame, and what they mean for
//! a machine with the toolpath convention (X, Y, Z the tool's motion relative to the work, A
//! turning the work right-handed about +X; the post's axis map then names and signs the
//! words for the real machine).
//!
//! A part point `p` sits at `Rx(A) p` in the machine frame. The spindle points along +Z, so
//! the A for a tool direction `t` is the one that turns `t` onto +Z: `atan2(t_y, t_z)`. A
//! direction with an X component needs a second rotary axis (5-axis) and is refused.

/// Part frame to machine frame with the rotary at `a_deg`: `Rx(A) p`.
pub fn to_machine(p: [f64; 3], a_deg: f64) -> [f64; 3] {
    let (s, c) = a_deg.to_radians().sin_cos();
    [p[0], c * p[1] - s * p[2], s * p[1] + c * p[2]]
}

/// Machine frame back to the part frame: `Rx(-A) p`.
pub fn to_part(p: [f64; 3], a_deg: f64) -> [f64; 3] {
    to_machine(p, -a_deg)
}

/// The A (degrees) that turns the part-frame tool direction `t` (tip towards spindle) onto
/// the spindle, the turn nearest `prev`.
pub fn solve_a(t: [f64; 3], prev: f64) -> Result<f64, String> {
    let n = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
    if n < 1e-12 {
        return Err("tool direction has no length".into());
    }
    if t[0].abs() / n > 1e-9 {
        return Err(format!(
            "tool direction ({:.4}, {:.4}, {:.4}) leans along X: that needs a second rotary axis",
            t[0] / n,
            t[1] / n,
            t[2] / n
        ));
    }
    let phi = t[1].atan2(t[2]).to_degrees();
    Ok(phi + 360.0 * ((prev - phi) / 360.0).round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turns_the_tool_direction_onto_the_spindle() {
        // straight down the part's +Z: A0; the part's +Y up: A90 (a turn of the work about X)
        assert_eq!(solve_a([0.0, 0.0, 1.0], 0.0).unwrap(), 0.0);
        let a = solve_a([0.0, 1.0, 0.0], 0.0).unwrap();
        assert!((a - 90.0).abs() < 1e-12);
        let t = to_machine([0.0, 1.0, 0.0], a);
        assert!(t[1].abs() < 1e-12 && (t[2] - 1.0).abs() < 1e-12, "{t:?}");
        // nearest turn to the previous A, and round trips
        assert!((solve_a([0.0, 1.0, 0.0], 700.0).unwrap() - 810.0).abs() < 1e-9);
        let p = [1.0, 2.0, 3.0];
        let back = to_part(to_machine(p, 37.0), 37.0);
        assert!((0..3).all(|k| (back[k] - p[k]).abs() < 1e-12));
        assert!(solve_a([0.5, 0.0, 1.0], 0.0)
            .unwrap_err()
            .contains("second rotary"));
    }
}
