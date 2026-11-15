//! G-code sanity checker for rotary programs (Z = radius from the rotation axis).
//!
//! Flags:
//! - feed moves whose Z falls faster than the plunge limit (with 2% slack for F rounding);
//! - rapids that move X or A while the tool is below the safe radius.

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
fn words(line: &str) -> Vec<(char, f64)> {
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
    let mut motion = None::<u32>;
    let mut feed = None::<f64>;
    for (idx, line) in text.lines().enumerate() {
        let w = words(line);
        let mut nx = None;
        let mut nz = None;
        let mut na = None;
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
                'Z' => nz = Some(v),
                'A' => na = Some(v),
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
        if nx.is_none() && nz.is_none() && na.is_none() {
            continue;
        }
        let tx = nx.or(x);
        let tz = nz.or(z);
        let ta = na.unwrap_or(a);
        match motion {
            Some(0) => {
                let moves_xa = nx.is_some_and(|v| Some(v) != x) || (ta - a).abs() > 1e-9;
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
            Some(1) => {
                if let (Some(z0), Some(z1), Some(f)) = (z, tz, feed) {
                    let dz = z1 - z0;
                    if dz < 0.0 && f > 0.0 {
                        let dx = tx.zip(x).map_or(0.0, |(p, q)| p - q);
                        let da = ta - a;
                        let time = match limits.feed_mode {
                            FeedMode::DegreesAsLength => (dx * dx + dz * dz + da * da).sqrt() / f,
                            FeedMode::LinearOnly => dx.hypot(dz) / f,
                            FeedMode::InverseTime => 1.0 / f,
                        };
                        let rate = if time > 0.0 {
                            -dz / time
                        } else {
                            f64::INFINITY
                        };
                        if rate > limits.plunge_feed * SLACK {
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
        z = tz;
        a = ta;
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            plunge_feed: 60.0,
            safe_r: 15.5,
            feed_mode: FeedMode::DegreesAsLength,
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
