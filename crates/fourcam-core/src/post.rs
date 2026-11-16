//! Post-processor: toolpath IR -> G-code text for a [`Machine`].

use crate::machine::{FeedMode, Machine};
use crate::toolpath::{true_length, Move, Toolpath};

struct State {
    x: Option<f64>,
    z: Option<f64>,
    a: f64,
    y: Option<f64>,
}

fn fill(template: &str, tool: u32, rpm: u32, safe: &str) -> String {
    template
        .replace("{tool}", &tool.to_string())
        .replace("{rpm}", &rpm.to_string())
        .replace("{safe}", safe)
}

/// Convert one program's toolpath to G-code. Lines end with `\n`.
pub fn post(tp: &Toolpath, machine: &Machine) -> String {
    post_marked(tp, machine).0
}

/// [`post`], plus where each operation starts: (operation id, kind, index of its first
/// line). The text is the same.
pub fn post_marked(tp: &Toolpath, machine: &Machine) -> (String, Vec<(u32, String, usize)>) {
    let (text, marks, _) = post_lines(tp, machine);
    (text, marks)
}

/// [`post_marked`], plus each move's G-code line: the 1-based number of the last line it
/// wrote (the line before it if it wrote none). The text is the same.
pub fn post_lines(
    tp: &Toolpath,
    machine: &Machine,
) -> (String, Vec<(u32, String, usize)>, Vec<usize>) {
    let mut marks = Vec::new();
    let mut before = Vec::with_capacity(tp.moves.len() + 1);
    let d = machine.decimals;
    let num = |v: f64| format!("{v:.d$}");
    // axis words as this machine names and signs them (identity for XYZ + A about +X)
    let word = |k: usize, v: f64| machine.axes.word(k, v, d);
    let safe = num(tp.safe_r);
    let mut out: Vec<String> = Vec::new();

    out.push(format!("({})", tp.title));
    if let Some((n, desc)) = &tp.tool {
        out.push(format!("(T{n} {desc})"));
    }
    if let Some(c) = tp
        .origin_comment
        .as_ref()
        .or(machine.origin_comment.as_ref())
    {
        out.push(format!("({c})"));
    }
    out.extend(machine.preamble.iter().cloned());
    for note in &tp.notes {
        out.push(format!("({note})"));
    }
    if tp.reset_a {
        if let Some(r) = &machine.reset_rotary {
            out.push(r.clone());
        }
    }

    let mut s = State {
        x: None,
        z: None,
        a: 0.0,
        y: None,
    };
    for mv in &tp.moves {
        before.push(out.len());
        match mv {
            Move::Comment { text } => out.push(format!("({text})")),
            Move::Operation { id, kind } => marks.push((*id, kind.clone(), out.len())),
            Move::Pose { tip, axis, feed } => {
                let a = match crate::kinematics::solve_a(*axis, s.a) {
                    Ok(a) => a,
                    Err(e) => {
                        // planning checks this; a hand-made toolpath gets a visible stop
                        out.push(format!("(4cam: {e})"));
                        out.push("M0".into());
                        continue;
                    }
                };
                let m = crate::kinematics::to_machine(*tip, a);
                // a value counts as changed when it really moved: the part-frame round trip
                // leaves ~1e-12 noise that must not flip a printed digit into a sideways rapid
                let changed =
                    |old: Option<f64>, new: f64| old.is_none_or(|o| (o - new).abs() > 1e-6);
                match feed {
                    None => {
                        let mut w = String::from("G0");
                        for (k, v, old) in [
                            (0, m[0], s.x),
                            (1, m[1], s.y),
                            (2, m[2], s.z),
                            (3, a, Some(s.a)),
                        ] {
                            if changed(old, v) {
                                w += &format!(" {}", word(k, v));
                            }
                        }
                        if w != "G0" {
                            out.push(w);
                        }
                    }
                    Some(feed) => {
                        // the feed is the tip's speed along the part; F as this controller reads it
                        let prev = [
                            s.x.unwrap_or(m[0]),
                            s.y.unwrap_or(m[1]),
                            s.z.unwrap_or(m[2]),
                        ];
                        let p0 = crate::kinematics::to_part(prev, s.a);
                        let len = ((tip[0] - p0[0]).powi(2)
                            + (tip[1] - p0[1]).powi(2)
                            + (tip[2] - p0[2]).powi(2))
                        .sqrt();
                        if len < 1e-6 && (a - s.a).abs() < 1e-9 {
                            continue;
                        }
                        let time = len.max(1e-9) / feed;
                        let (dx, dy, dz, da) =
                            (m[0] - prev[0], m[1] - prev[1], m[2] - prev[2], a - s.a);
                        let lin = (dx * dx + dy * dy + dz * dz).sqrt();
                        let fw = match machine.feed_mode {
                            FeedMode::InverseTime => format!("{:.3}", 1.0 / time),
                            FeedMode::DegreesAsLength => {
                                format!("{:.0}", (lin * lin + da * da).sqrt() / time)
                            }
                            FeedMode::LinearOnly => {
                                format!("{:.0}", if lin > 1e-9 { lin } else { da.abs() } / time)
                            }
                        };
                        let aw = if changed(Some(s.a), a) {
                            format!(" {}", word(3, a))
                        } else {
                            String::new()
                        };
                        out.push(format!(
                            "G1 {} {} {}{aw} F{fw}",
                            word(0, m[0]),
                            word(1, m[1]),
                            word(2, m[2])
                        ));
                    }
                }
                s.x = Some(m[0]);
                s.y = Some(m[1]);
                s.z = Some(m[2]);
                s.a = a;
            }
            Move::Rapid3 { x, y, z } => {
                let mut w = String::from("G0");
                for (k, v, st) in [(0, x, &mut s.x), (1, y, &mut s.y), (2, z, &mut s.z)] {
                    if let Some(v) = v {
                        w += &format!(" {}", word(k, *v));
                        *st = Some(*v);
                    }
                }
                out.push(w);
            }
            Move::Line3 { x, y, z, feed, .. } => {
                let p0 = [s.x.unwrap_or(*x), s.y.unwrap_or(*y), s.z.unwrap_or(*z)];
                let len = ((x - p0[0]).powi(2) + (y - p0[1]).powi(2) + (z - p0[2]).powi(2)).sqrt();
                if len < 1e-6 {
                    continue;
                }
                let fw = if machine.feed_mode == FeedMode::InverseTime {
                    format!("{:.3}", feed / len)
                } else {
                    format!("{feed:.0}")
                };
                out.push(format!(
                    "G1 {} {} {} F{fw}",
                    word(0, *x),
                    word(1, *y),
                    word(2, *z)
                ));
                s.x = Some(*x);
                s.y = Some(*y);
                s.z = Some(*z);
            }
            Move::ToolChange { tool, rpm } => {
                out.push(fill(&machine.tool_change, *tool, *rpm, &safe));
                out.push(fill(&machine.spindle_on, *tool, *rpm, &safe));
                if let Some(dw) = &machine.dwell {
                    out.push(dw.clone());
                }
            }
            Move::Rapid { x, r, a } => {
                let mut w = String::from("G0");
                if let Some(z) = r {
                    w += &format!(" {}", word(2, *z));
                    s.z = Some(*z);
                }
                if let Some(x) = x {
                    w += &format!(" {}", word(0, *x));
                    s.x = Some(*x);
                }
                if let Some(a) = a {
                    w += &format!(" {}", word(3, *a));
                    s.a = *a;
                }
                out.push(w);
            }
            Move::Plunge { r, feed } => {
                if machine.feed_mode == FeedMode::InverseTime {
                    // G93: F = 1 / minutes for this move
                    let dz = s.z.map_or(0.0, |z| (r - z).abs()).max(1e-6);
                    out.push(format!("G1 {} F{:.3}", word(2, *r), feed / dz));
                } else {
                    out.push(format!("G1 {} F{feed:.0}", word(2, *r)));
                }
                s.z = Some(*r);
            }
            Move::Cut {
                x,
                r,
                a,
                feed,
                plunge_cap,
            } => {
                let (x0, z0) = (s.x.unwrap_or(*x), s.z.unwrap_or(*r));
                // G93 feeds are per move: time the move the controller will actually run,
                // i.e. between the printed (rounded) coordinates
                let q = |v: f64| {
                    if machine.feed_mode == FeedMode::InverseTime {
                        // exactly the printed value
                        num(v).parse::<f64>().unwrap_or(v)
                    } else {
                        v
                    }
                };
                let (dx, dz, da) = (q(*x) - q(x0), q(*r) - q(z0), q(*a) - q(s.a));
                let len = true_length(dx, dz, da, q(z0), q(*r));
                if len < 1e-6 {
                    continue;
                }
                let mut dt = len / feed;
                if dz < 0.0 {
                    // never descend faster than the plunge feed
                    dt = crate::numeric::py_max(dt, -dz / plunge_cap);
                }
                let f = match machine.feed_mode {
                    FeedMode::DegreesAsLength => (dx * dx + dz * dz + da * da).sqrt() / dt,
                    FeedMode::LinearOnly => {
                        let lin = dx.hypot(dz);
                        if lin > 1e-6 {
                            lin / dt
                        } else {
                            *feed
                        }
                    }
                    FeedMode::InverseTime => 1.0 / dt,
                };
                let mut fw = if machine.feed_mode == FeedMode::InverseTime {
                    format!("{f:.3}")
                } else {
                    format!("{f:.0}")
                };
                // on very short moves the printed (rounded) coordinates can descend faster than
                // the exact ones: slow such a move so the program as written stays within the
                // plunge cap (and its 2% allowance for F rounding)
                let p = |v: f64| num(v).parse::<f64>().unwrap_or(v);
                let (px, pz, pa) = (p(*x) - p(x0), p(*r) - p(z0), p(*a) - p(s.a));
                if pz < 0.0 {
                    let span = match machine.feed_mode {
                        FeedMode::DegreesAsLength => (px * px + pz * pz + pa * pa).sqrt(),
                        FeedMode::LinearOnly => px.hypot(pz),
                        FeedMode::InverseTime => 1.0,
                    };
                    let fv: f64 = fw.parse().unwrap_or(f);
                    if fv > 0.0 && -pz * fv / span > plunge_cap * 1.02 {
                        let slow = span * plunge_cap / -pz;
                        fw = if machine.feed_mode == FeedMode::InverseTime {
                            format!("{:.3}", (slow * 1000.0).floor() / 1000.0)
                        } else {
                            format!("{:.0}", slow.floor().max(1.0))
                        };
                    }
                }
                out.push(format!(
                    "G1 {} {} {} F{fw}",
                    word(0, *x),
                    word(2, *r),
                    word(3, *a)
                ));
                s.x = Some(*x);
                s.z = Some(*r);
                s.a = *a;
            }
        }
    }
    before.push(out.len());
    let tool = tp.tool.as_ref().map_or(0, |t| t.0);
    for line in &machine.postamble {
        out.push(fill(line, tool, 0, &safe));
    }
    let mut text = out.join("\n");
    text.push('\n');
    (text, marks, before[1..].to_vec())
}
