//! Lead-ins and lead-outs: a contour comes onto its path along a quarter arc from the waste
//! side (outside an outline, inside a hole), tangent to it, and leaves along another, instead
//! of plunging on the wall; a rectangular pocket ramps down to each level back and forth
//! instead of plunging. The G-code still verifies clean (descents fed within the plunge
//! feed).

use marcam::job::Operation;
use marcam::{limits_for, load_mesh, plan_with_mesh, post, verify_gcode, Job, Move, Toolpath};
use std::path::PathBuf;

fn plate() -> (Job, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok()).unwrap();
    (jobs.remove(0).1, dir)
}

fn plan(job: &Job, dir: &std::path::Path) -> Vec<Toolpath> {
    let mesh = load_mesh(job, dir).unwrap();
    plan_with_mesh(job, mesh.as_ref()).unwrap()
}

/// The moves of the operation of `kind` (from its marker to the next).
fn moves_of<'a>(tps: &'a [Toolpath], kind: &str) -> Vec<&'a Move> {
    let mut on = false;
    let mut out = Vec::new();
    for tp in tps {
        for m in &tp.moves {
            if let Move::Operation { kind: k, .. } = m {
                on = k == kind;
            }
            if on {
                out.push(m);
            }
        }
    }
    out
}

/// The points where the tool goes down (a feed move straight down, X and Y unchanged).
fn plunges(ms: &[&Move]) -> Vec<[f64; 3]> {
    let mut at: Option<[f64; 3]> = None;
    let mut out = Vec::new();
    for m in ms {
        let to = match **m {
            Move::Line3 { x, y, z, .. } => Some([x, y, z]),
            Move::Rapid3 { x, y, z } => {
                at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])])
            }
            _ => None,
        };
        if let (Some(a), Some(b), Move::Line3 { .. }) = (at, to, m) {
            if (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 && b[2] < a[2] - 1e-9 {
                out.push(b);
            }
        }
        if to.is_some() {
            at = to;
        }
    }
    out
}

fn verifies_clean(job: &Job, tps: &[Toolpath]) {
    let profile = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    for tp in tps {
        let issues = verify_gcode(&post(tp, &profile.post), &limits_for(tp, &profile.post));
        assert!(
            issues.is_empty(),
            "{}: {:?}",
            tp.name,
            issues.iter().map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }
    let _ = job;
}

#[test]
fn a_contour_comes_on_and_off_along_arcs_from_the_waste_side() {
    let (mut job, dir) = plate();
    // the plate's outline: outside the rectangle (2, 2)-(58, 38), a 3.175 tool
    for p in &mut job.programs {
        for o in &mut p.operations {
            if let Operation::Contour(c) = o {
                c.lead_in = 2.0;
                c.lead_out = 2.0;
            }
        }
    }
    let tps = plan(&job, &dir);
    let ms = moves_of(&tps, "contour");
    // every plunge is off the outline, outside it (the waste side), clear of the part by more
    // than the tool's radius plus the lead's
    let pl = plunges(&ms);
    assert!(!pl.is_empty());
    let r = 3.175 / 2.0;
    let outside_by = |p: [f64; 3]| {
        // distance outside the rectangle (negative inside)
        let dx = (2.0 - p[0]).max(p[0] - 58.0);
        let dy = (2.0 - p[1]).max(p[1] - 38.0);
        if dx > 0.0 && dy > 0.0 {
            dx.hypot(dy)
        } else {
            dx.max(dy)
        }
    };
    for p in &pl {
        assert!(
            outside_by(*p) > r + 1.0,
            "plunge at {p:?}: {} outside",
            outside_by(*p)
        );
    }
    // no move cuts into the part's outline (the tool's edge stays outside the rectangle)
    for m in &ms {
        if let Move::Line3 { x, y, z, .. } = **m {
            if z < -0.4 {
                assert!(
                    outside_by([x, y, z]) > r - 1e-6,
                    "into the part at {x} {y} {z}"
                );
            }
        }
    }
    verifies_clean(&job, &tps);
    // without leads: it plunges on the path (as before: at its first corner)
    let (job0, dir0) = plate();
    let pl0 = plunges(&moves_of(&plan(&job0, &dir0), "contour"));
    assert!(
        pl0.iter().all(|p| outside_by(*p) <= r * 2f64.sqrt() + 1e-6),
        "{pl0:?}"
    );
}

#[test]
fn a_hole_s_contour_leads_in_from_inside_the_hole() {
    let (mut job, dir) = plate();
    // the outline as a hole: inside the rectangle; the leads come from inside it
    for p in &mut job.programs {
        for o in &mut p.operations {
            if let Operation::Contour(c) = o {
                c.side = "inside".into();
                c.lead_in = 2.0;
                c.lead_out = 2.0;
            }
        }
    }
    let tps = plan(&job, &dir);
    let pl = plunges(&moves_of(&tps, "contour"));
    let inside_by = |p: [f64; 3]| {
        (p[0] - 2.0)
            .min(58.0 - p[0])
            .min(p[1] - 2.0)
            .min(38.0 - p[1])
    };
    let r = 3.175 / 2.0;
    for p in &pl {
        assert!(
            inside_by(*p) > r + 1.0,
            "plunge at {p:?}: {} inside",
            inside_by(*p)
        );
    }
    verifies_clean(&job, &tps);
}

#[test]
fn a_pocket_ramps_down_instead_of_plunging() {
    let (mut job, dir) = plate();
    let mut found = false;
    for p in &mut job.programs {
        for o in &mut p.operations {
            if let Operation::Pocket(pk) = o {
                if pk.paths.is_empty() && pk.regions.is_empty() {
                    pk.ramp = 3.0;
                    found = true;
                }
            }
        }
    }
    assert!(found, "the plate has a rectangular pocket");
    let tps = plan(&job, &dir);
    let ms = moves_of(&tps, "pocket");
    // nothing goes straight down into the material: the only plunges are to above the stock
    // (the ramps' starts), every descent below it at 3° or flatter
    let mut at: Option<[f64; 3]> = None;
    let mut steepest = 0.0f64;
    for m in &ms {
        let to = match **m {
            Move::Line3 { x, y, z, .. } => Some([x, y, z]),
            Move::Rapid3 { x, y, z } => {
                at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])])
            }
            _ => None,
        };
        if let (Some(a), Some(b), Move::Line3 { .. }) = (at, to, m) {
            let drop = a[2] - b[2];
            let across = (b[0] - a[0]).hypot(b[1] - a[1]);
            if drop > 1e-6 && b[2] < -0.5 - 1e-6 {
                steepest = steepest.max(drop.atan2(across).to_degrees());
            }
        }
        if to.is_some() {
            at = to;
        }
    }
    assert!(
        steepest <= 3.0 + 1e-6,
        "steepest descent into the pocket {steepest:.2}°"
    );
    verifies_clean(&job, &tps);
}

/// The descents into material of operation `kind`: (steepest angle below the horizontal,
/// vertical plunges below `below`).
fn descents(tps: &[Toolpath], kind: &str, below: f64) -> (f64, usize) {
    let ms = moves_of(tps, kind);
    let mut at: Option<[f64; 3]> = None;
    let (mut steepest, mut vertical) = (0.0f64, 0);
    for m in &ms {
        let to = match **m {
            Move::Line3 { x, y, z, .. } => Some([x, y, z]),
            Move::Rapid3 { x, y, z } => {
                at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])])
            }
            _ => None,
        };
        if let (Some(a), Some(b), Move::Line3 { .. }) = (at, to, m) {
            let drop = a[2] - b[2];
            let across = (b[0] - a[0]).hypot(b[1] - a[1]);
            if drop > 1e-6 && b[2] < below {
                if across < 1e-9 {
                    vertical += 1;
                } else {
                    steepest = steepest.max(drop.atan2(across).to_degrees());
                }
            }
        }
        if to.is_some() {
            at = to;
        }
    }
    (steepest, vertical)
}

#[test]
fn a_shaped_pocket_ramps_down_along_its_loops() {
    let (mut job, dir) = plate();
    for p in &mut job.programs {
        for o in &mut p.operations {
            if let Operation::Pocket(pk) = o {
                if pk.paths.is_empty() && pk.regions.is_empty() {
                    // the same rectangle as an outline: cleared by offset loops
                    let [x0, y0, x1, y1] = pk.rect;
                    pk.paths = vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]];
                    pk.ramp = 3.0;
                }
            }
        }
    }
    let tps = plan(&job, &dir);
    let (steepest, vertical) = descents(&tps, "pocket", -0.5 - 1e-6);
    assert_eq!(vertical, 0, "no plunge into the pocket");
    assert!(steepest <= 3.0 + 1e-6, "steepest {steepest:.2}°");
    verifies_clean(&job, &tps);
}

#[test]
fn a_3d_finish_comes_onto_each_pass_along_a_ramp() {
    let (mut job, dir) = plate();
    for p in &mut job.programs {
        for o in &mut p.operations {
            if let Operation::Raster3d(r) = o {
                r.lead_in = 0.5;
            }
        }
    }
    let tps = plan(&job, &dir);
    let ms = moves_of(&tps, "raster3d");
    // every vertical move down ends above the surface: a descent along the pass follows
    let mut at: Option<[f64; 3]> = None;
    let mut ramps = 0;
    for (i, m) in ms.iter().enumerate() {
        let to = match **m {
            Move::Line3 { x, y, z, .. } => Some([x, y, z]),
            Move::Rapid3 { x, y, z } => {
                at.map(|a| [x.unwrap_or(a[0]), y.unwrap_or(a[1]), z.unwrap_or(a[2])])
            }
            _ => None,
        };
        if let (Some(a), Some(b), Move::Line3 { .. }) = (at, to, m) {
            if (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 && b[2] < a[2] - 1e-9 {
                if let Some(Move::Line3 { z, .. }) = ms.get(i + 1).map(|m| (*m).clone()) {
                    assert!(
                        z < b[2] - 1e-3,
                        "a plunge at {b:?} straight onto the surface"
                    );
                    ramps += 1;
                }
            }
        }
        if to.is_some() {
            at = to;
        }
    }
    assert!(ramps > 0);
    // nothing into the part
    let mesh = load_mesh(&job, &dir).unwrap();
    let g = marcam::gouge_check(&job, &tps, mesh.as_ref()).unwrap();
    assert!(g.iter().all(|r| r.max_depth < 0.01), "{g:?}");
    verifies_clean(&job, &tps);
}

#[test]
fn a_rotary_finish_comes_onto_the_surface_turning() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/knob");
    let text = std::fs::read_to_string(dir.join("knob_finish.job.json")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let plan_with = |lead: f64| {
        let mut job = marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&dir).ok())
            .unwrap()
            .remove(0)
            .1;
        for p in &mut job.programs {
            for o in &mut p.operations {
                if let Operation::RotaryFinish(f) = o {
                    f.lead_in = lead;
                }
            }
        }
        let mesh = load_mesh(&job, &dir).unwrap();
        let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
        (job, tps)
    };
    let radial_plunges = |tps: &[Toolpath]| {
        let mut on = false;
        tps.iter()
            .flat_map(|tp| &tp.moves)
            .filter(|m| {
                if let Move::Operation { kind, .. } = m {
                    on = kind == "rotary_finish";
                }
                on && matches!(m, Move::Plunge { .. })
            })
            .count()
    };
    let (_, plain) = plan_with(0.0);
    let (job, led) = plan_with(0.3);
    assert!(
        radial_plunges(&plain) > 0,
        "the knob's finish has passes to come onto"
    );
    // the plunges stop above the surface, the turn takes it down: the part is not touched
    // any deeper than without
    let mesh = load_mesh(
        &job,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/knob"),
    )
    .unwrap();
    let g = marcam::gouge_check(&job, &led, mesh.as_ref()).unwrap();
    assert!(g.iter().all(|r| r.max_depth < 0.01), "{g:?}");
    // and the plunges end above where they did
    let ends = |tps: &[Toolpath]| -> Vec<f64> {
        let mut on = false;
        tps.iter()
            .flat_map(|tp| &tp.moves)
            .filter_map(|m| {
                if let Move::Operation { kind, .. } = m {
                    on = kind == "rotary_finish";
                }
                match m {
                    Move::Plunge { r, .. } if on => Some(*r),
                    _ => None,
                }
            })
            .collect()
    };
    let (e0, e1) = (ends(&plain), ends(&led));
    assert_eq!(e0.len(), e1.len());
    assert!(
        e1.iter().zip(&e0).all(|(a, b)| *a > *b + 0.1),
        "{e0:?} -> {e1:?}"
    );
}
