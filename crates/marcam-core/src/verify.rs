//! G-code sanity checker for rotary programs (Z = radius from the rotation axis).
//!
//! Flags:
//! - feed moves whose Z falls faster than the plunge limit (with 2% slack for F rounding);
//! - rapids that move X or A while the tool is below the safe radius.

use crate::det::Det;
use crate::machine::FeedMode;
use std::fmt;

/// Limits the program is checked against.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Fastest allowed descent along Z (mm/min).
    pub plunge_feed: f64,
    /// Radius above which X/A rapids are allowed.
    pub safe_r: f64,
    pub feed_mode: FeedMode,
    /// `surface_speed` radius pad (mm), see [`crate::machine::Machine::surface_radius_pad`].
    pub surface_radius_pad: f64,
    /// Descents gentler than this (degrees) may run at any feed; steeper ones and pure
    /// plunges are held to `plunge_feed` (0: every descent is).
    pub ramp_angle: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IssueKind {
    /// Z descends at `rate` mm/min, faster than the plunge limit.
    FastDescent { rate: f64 },
    /// G0 moves X or A with the tool at radius `z`, below the safe radius.
    LowRapid { z: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    /// 1-based line number.
    pub line: usize,
    pub kind: IssueKind,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            IssueKind::FastDescent { rate } => write!(
                f,
                "line {}: Z descends at {rate:.1} mm/min (above plunge limit)",
                self.line
            ),
            IssueKind::LowRapid { z } => write!(
                f,
                "line {}: rapid X/A move at Z{z:.3} (below safe radius)",
                self.line
            ),
        }
    }
}

const SLACK: f64 = 1.02;

/// Split a line into (letter, value) words, ignoring comments.
pub(crate) fn words(line: &str) -> Vec<(char, f64)> {
    let mut code = String::new();
    let mut depth = 0;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ';' if depth == 0 => break,
            _ if depth == 0 => code.push(c),
            _ => {}
        }
    }
    let mut out = Vec::new();
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].to_ascii_uppercase();
        if c.is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_ascii_digit() || matches!(chars[j], '.' | '-' | '+'))
            {
                j += 1;
            }
            let s: String = chars[i + 1..j].iter().collect();
            if let Ok(v) = s.parse::<f64>() {
                out.push((c, v));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Check G-code text; returns the issues found (empty means clean).
pub fn verify(text: &str, limits: &Limits) -> Vec<Issue> {
    let mut issues = Vec::new();
    let (mut x, mut z, mut a) = (None::<f64>, None::<f64>, 0.0f64);
    let mut y: Option<f64> = None;
    let mut motion = None::<u32>;
    let mut feed = None::<f64>;
    for (idx, line) in text.lines().enumerate() {
        let w = words(line);
        let mut nx = None;
        let mut ny = None;
        let mut nz = None;
        let mut na = None;
        let (mut ni, mut nj) = (0.0f64, 0.0f64);
        let mut g92 = false;
        for &(c, v) in &w {
            match c {
                'G' => {
                    let g = v.round() as u32;
                    match g {
                        0..=3 => motion = Some(g),
                        92 => g92 = true,
                        _ => {}
                    }
                }
                'X' => nx = Some(v),
                'Y' => ny = Some(v),
                'Z' => nz = Some(v),
                'A' => na = Some(v),
                'I' => ni = v,
                'J' => nj = v,
                'F' => feed = Some(v),
                _ => {}
            }
        }
        if g92 {
            if let Some(v) = na {
                a = v;
            }
            continue;
        }
        if nx.is_none() && ny.is_none() && nz.is_none() && na.is_none() {
            continue;
        }
        let tx = nx.or(x);
        let tz = nz.or(z);
        let ta = na.unwrap_or(a);
        match motion {
            Some(0) => {
                let moves_xa = nx.is_some_and(|v| Some(v) != x)
                    || ny.is_some_and(|v| Some(v) != y)
                    || (ta - a).abs() > 1e-9;
                // the tool is at the lower of the start and end radius during the move
                let low = match (z, tz) {
                    (Some(z0), Some(z1)) => Some(z0.min(z1)),
                    (zz, None) | (None, zz) => zz,
                };
                if moves_xa {
                    if let Some(zl) = low {
                        if zl < limits.safe_r - 1e-6 {
                            issues.push(Issue {
                                line: idx + 1,
                                kind: IssueKind::LowRapid { z: zl },
                            });
                        }
                    }
                }
            }
            Some(m @ 1..=3) => {
                if let (Some(z0), Some(z1), Some(f)) = (z, tz, feed) {
                    let dz = z1 - z0;
                    if dz < 0.0 && f > 0.0 {
                        let mut dx = tx.zip(x).map_or(0.0, |(p, q)| p - q);
                        let (y0, mut y1) = (y.unwrap_or(0.0), ny.or(y).unwrap_or(0.0));
                        if m != 1 {
                            // an arc in XY (G17): its length, from the centre at I, J
                            let (x0, x1) = (x.unwrap_or(0.0), tx.unwrap_or(0.0));
                            let (cx, cy) = (x0 + ni, y0 + nj);
                            let r = ni.dhypot(nj);
                            let a0 = (y0 - cy).datan2(x0 - cx);
                            let a1 = (y1 - cy).datan2(x1 - cx);
                            let mut sweep = a1 - a0;
                            if m == 2 && sweep >= 0.0 {
                                sweep -= std::f64::consts::TAU;
                            } else if m == 3 && sweep <= 0.0 {
                                sweep += std::f64::consts::TAU;
                            }
                            dx = r * sweep.abs();
                            y1 = y0;
                        }
                        let radius = (y0.dhypot(z0) + y1.dhypot(z1)) / 2.0;
                        let len = crate::machine::feed_length(
                            limits.feed_mode,
                            limits.surface_radius_pad,
                            [dx, y1 - y0, dz, ta - a],
                            radius,
                        );
                        let time = len / f;
                        // the travel along the surface (rotary: the arc at the mean radius,
                        // as the planner measures it); printed coordinates are rounded, so
                        // a move within 2 microns of the ramp counts as on it
                        let arc = radius.max(0.5) * (ta - a).to_radians();
                        let across = (dx * dx + (y1 - y0).powi(2) + arc * arc).sqrt();
                        let ramp_tan = limits.ramp_angle.clamp(0.0, 89.0).to_radians().dtan();
                        let gentle = crate::toolpath::gentle_descent(
                            -dz,
                            across * 1.001 + 0.002 / ramp_tan.max(1e-9),
                            ramp_tan,
                        );
                        let rate = if time > 0.0 {
                            -dz / time
                        } else {
                            f64::INFINITY
                        };
                        if !gentle && rate > limits.plunge_feed * SLACK {
                            issues.push(Issue {
                                line: idx + 1,
                                kind: IssueKind::FastDescent { rate },
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        x = tx;
        y = ny.or(y);
        z = tz;
        a = ta;
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::det::Det;

    fn limits() -> Limits {
        Limits {
            plunge_feed: 60.0,
            safe_r: 15.5,
            feed_mode: FeedMode::DegreesAsLength,
            surface_radius_pad: 0.0,
            ramp_angle: 0.0,
        }
    }

    #[test]
    fn clean_program_passes() {
        let g = "G0 Z15.500\nG0 X-5.000 A10.000\nG1 Z10.000 F60\nG1 X-4.000 Z10.000 A20.000 F500\nG0 Z15.500\n";
        assert!(verify(g, &limits()).is_empty());
    }

    #[test]
    fn flags_fast_descent() {
        let g = "G0 Z15.500\nG0 X0.000\nG1 Z10.000 F300\n(slow ramp is fine)\nG1 X0 Z9.990 A6 F500\nG1 X0 Z8.000 A12 F500\n";
        let issues = verify(g, &limits());
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert_eq!(issues[0].line, 3);
        assert_eq!(issues[1].line, 6);
        assert!(
            matches!(issues[0].kind, IssueKind::FastDescent { rate } if (rate - 300.0).abs() < 1e-9)
        );
    }

    #[test]
    fn flags_low_rapid() {
        let g = "G0 Z15.500\nG0 X-3.000\nG1 Z9.000 F60\nG0 X-6.000\nG0 A90\nG0 Z15.5\nG0 X0 A0\n";
        let issues = verify(g, &limits());
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert_eq!(issues[0].line, 4);
        assert_eq!(issues[1].line, 5);
        assert!(matches!(issues[0].kind, IssueKind::LowRapid { z } if z == 9.0));
    }

    #[test]
    fn g92_resets_a_without_moving() {
        let g = "G0 Z5\nG92 A0\nG1 X0 Z5 A1 F100\n";
        assert!(verify(g, &limits()).is_empty());
    }
}
