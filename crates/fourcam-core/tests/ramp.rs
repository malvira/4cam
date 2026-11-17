//! Ramp angle (Mar, 2026-11-17: "only slow steep descents"): a feed move descending more
//! gently than the operation's ramp angle (default 30 degrees) runs at the cutting feed;
//! steeper descents, and plunges, keep the tip falling no faster than the plunge feed.
//!
//! The original failure this rule guards (the knob, 2026-11-02): a pass dropping down a wall
//! at the cutting feed. Walls are steep, so they stay slowed, and the verifier still flags a
//! wall dropped at the cutting feed.

use fourcam::job::Operation;
use fourcam::toolpath::gentle_descent;
use fourcam::{limits_for, plan, post, verify_gcode, Job, Machine, Move};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn knob(ramp: Option<f64>) -> Job {
    let text = std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap();
    let mut job = Job::from_json(&text).unwrap();
    if let Some(r) = ramp {
        for p in &mut job.programs {
            for op in &mut p.operations {
                match op {
                    Operation::RotaryTurning(o) => o.ramp_angle = r,
                    Operation::HelicalKnurl(o) => o.ramp_angle = r,
                    Operation::FlankChamfer(o) => o.ramp_angle = r,
                    _ => {}
                }
            }
        }
    }
    job
}

/// (start, end) of each G1 that changes Z: X, Z (radius), A, F.
fn descents(g: &str) -> Vec<(usize, [f64; 3], [f64; 3], f64)> {
    let mut out = Vec::new();
    let (mut p, mut f) = ([0.0f64; 3], 0.0f64);
    for (i, line) in g.lines().enumerate() {
        let mut q = p;
        let mut g1 = false;
        for w in line.split_whitespace() {
            let (c, v) = w.split_at(1);
            let Ok(v) = v.parse::<f64>() else { continue };
            match c {
                "G" => g1 = v == 1.0,
                "X" => q[0] = v,
                "Z" => q[1] = v,
                "A" => q[2] = v,
                "F" => f = v,
                _ => {}
            }
        }
        if g1 && q[1] < p[1] - 1e-9 {
            out.push((i, p, q, f));
        }
        p = q;
    }
    out
}

/// Drop and travel along the surface of a rotary move (the arc at the mean radius).
fn slope(p: [f64; 3], q: [f64; 3]) -> (f64, f64) {
    let r = (0.5 * (p[1] + q[1])).max(0.5);
    let across = (q[0] - p[0]).hypot(r * (q[2] - p[2]).to_radians());
    (p[1] - q[1], across)
}

#[test]
fn the_rule() {
    let t30 = 30f64.to_radians().tan();
    assert!(gentle_descent(0.5, 1.0, t30)); // 26.6 deg
    assert!(!gentle_descent(0.6, 1.0, t30)); // 31 deg
    assert!(!gentle_descent(1.0, 0.0, t30), "a plunge is never gentle");
    assert!(
        !gentle_descent(0.01, 1.0, 0.0),
        "ramp 0: every descent is slowed"
    );
    // 3-axis: a gentle slope at the feed, a steep one slowed, a plunge at the plunge feed
    use fourcam::strategy::three_axis::descent_feed;
    assert_eq!(
        descent_feed([0.0, 0.0, 0.0], [1.0, 0.0, -0.3], 600.0, 60.0, t30),
        600.0
    );
    let steep = descent_feed([0.0, 0.0, 0.0], [1.0, 0.0, -2.0], 600.0, 60.0, t30);
    assert!(steep < 70.0, "{steep}");
    let ramp0 = descent_feed([0.0, 0.0, 0.0], [1.0, 0.0, -0.3], 600.0, 60.0, 0.0);
    assert!(ramp0 < 210.0, "ramp 0 keeps the old rule: {ramp0}");
}

#[test]
fn only_the_feeds_of_gentle_descents_change() {
    // the same moves as with every descent slowed (ramp 0): the geometry, hence the cut part
    // and the gouge check, are unchanged; only gentle descents get the cutting feed
    let (new, old) = (plan(&knob(None)).unwrap(), plan(&knob(Some(0.0))).unwrap());
    let t30 = 30f64.to_radians().tan();
    let mut faster = 0;
    for (a, b) in new.iter().zip(&old) {
        assert_eq!(a.moves.len(), b.moves.len());
        let mut at = (0.0, 0.0, 0.0);
        for (m, n) in a.moves.iter().zip(&b.moves) {
            match (m, n) {
                (
                    Move::Cut {
                        x,
                        r,
                        a: ang,
                        feed,
                        plunge_cap,
                    },
                    Move::Cut {
                        x: x2,
                        r: r2,
                        a: a2,
                        plunge_cap: cap2,
                        ..
                    },
                ) => {
                    assert_eq!((x, r, ang), (x2, r2, a2));
                    if plunge_cap != cap2 {
                        let (drop, across) = slope([at.0, at.1, at.2], [*x, *r, *ang]);
                        assert!(
                            gentle_descent(drop, across, t30),
                            "only gentle descents change"
                        );
                        assert_eq!(plunge_cap, feed);
                        faster += 1;
                    }
                    at = (*x, *r, *ang);
                }
                (m, n) => {
                    assert_eq!(m, n);
                    match m {
                        Move::Plunge { r, .. } => at.1 = *r,
                        Move::Rapid { x, r, a } => {
                            at = (x.unwrap_or(at.0), r.unwrap_or(at.1), a.unwrap_or(at.2))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    assert!(faster > 0);
}

#[test]
fn walls_stay_slowed_and_the_verifier_catches_a_wall_dropped_at_the_cutting_feed() {
    let job = knob(None);
    let m = Machine::makera_z1();
    let tp = plan(&job).unwrap().remove(0); // the profile: rough + finish, walls at the boss
    let g = post(&tp, &m);
    let limits = limits_for(&tp, &m);
    assert_eq!(limits.ramp_angle, 30.0);
    assert!(verify_gcode(&g, &limits).is_empty());
    let t30 = 30f64.to_radians().tan();
    // every descent steeper than the ramp falls no faster than the plunge feed
    let mut walls = 0;
    let mut wall_line = None;
    for (i, p, q, f) in descents(&g) {
        let (drop, across) = slope(p, q);
        if !gentle_descent(drop, across, t30) {
            // degrees as length: the move's F-length over F is its time
            let len = ((q[0] - p[0]).powi(2) + drop * drop + (q[2] - p[2]).powi(2)).sqrt();
            let rate = drop / (len / f);
            assert!(
                rate <= limits.plunge_feed * 1.02,
                "line {}: steep descent at {rate:.0} mm/min",
                i + 1
            );
            if drop > 1.0 {
                walls += 1;
                wall_line.get_or_insert(i);
            }
        }
    }
    assert!(walls > 0, "the knob's profile drops down walls");
    // 2026-11-02: the same wall at the cutting feed is flagged
    let i = wall_line.unwrap();
    let bad: Vec<String> = g
        .lines()
        .enumerate()
        .map(|(k, l)| {
            if k == i {
                let (head, _) = l.rsplit_once(" F").unwrap();
                format!("{head} F2500")
            } else {
                l.to_string()
            }
        })
        .collect();
    let issues = verify_gcode(&(bad.join("\n") + "\n"), &limits);
    assert!(
        issues.iter().any(|x| x.line == i + 1),
        "the wall dropped at the cutting feed is caught: {issues:?}"
    );
}

#[test]
fn ramp_zero_keeps_every_descent_slowed() {
    let job = knob(Some(0.0));
    let m = Machine::makera_z1();
    for tp in plan(&job).unwrap() {
        assert_eq!(tp.ramp_angle, None);
        let g = post(&tp, &m);
        let limits = limits_for(&tp, &m);
        assert_eq!(limits.ramp_angle, 0.0);
        assert!(verify_gcode(&g, &limits).is_empty(), "{}", tp.name);
        for (i, p, q, f) in descents(&g) {
            let (drop, _) = slope(p, q);
            let len = ((q[0] - p[0]).powi(2) + drop * drop + (q[2] - p[2]).powi(2)).sqrt();
            assert!(
                drop / (len / f) <= limits.plunge_feed * 1.02,
                "{} line {}",
                tp.name,
                i + 1
            );
        }
    }
}
