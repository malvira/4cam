//! M2: mesh roughing, material-removal simulation and gouge checks, end to end on the knob.

use marcam::{gouge_check, load_mesh, plan, plan_with_mesh, sim, simulate_job, Job};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn job(name: &str) -> Job {
    let text = std::fs::read_to_string(root().join("examples/knob").join(name)).unwrap();
    Job::from_json(&text).unwrap()
}

/// Deviation over the knob body and boss, away from the front chamfer (which a flat or
/// ball tool cannot make; the V-bit flank chamfer does it) and the back end.
fn body_deviation(res: &marcam::JobSim, program: usize) -> sim::Deviation {
    let _ = program;
    sim::deviation(&res.stock, &res.part, -21.5, -1.0, 0.02)
}

#[test]
fn mesh_rough_then_finish_matches_part() {
    let mut job = job("knob_finish.job.json");
    // rough + flat helical finish (the ball raster is covered by finish.rs)
    job.programs.retain(|p| p.name != "knob_4_finish_along");
    let mesh = load_mesh(&job, &root().join("examples/knob")).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();

    for g in gouge_check(&job, &tps, mesh.as_ref()).unwrap() {
        eprintln!(
            "{}: gouge check {} moves, deepest {:.4} mm",
            g.program, g.moves_checked, g.max_depth
        );
        assert!(g.moves_checked > 1000);
        assert!(g.max_depth < 0.01, "{g:?}");
    }

    let res = simulate_job(&job, &tps[..1], mesh.as_ref(), 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[0];
    let body = body_deviation(&res, 0);
    eprintln!("after rough: {dev:?}\n  body: {body:?}");
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0);
    // roughing leaves at least the allowance (0.15 mm) everywhere ...
    let min_left = min_left(&res, -21.5, -1.0);
    assert!(
        min_left > 0.15 - 0.02,
        "rough cut into the allowance: {min_left}"
    );
    // ... and at most one step (0.5 mm) plus allowance plus what the taper adds under a level
    assert!(body.max_left < 0.5 + 0.15 + 0.5, "{body:?}");

    let res = simulate_job(&job, &tps, mesh.as_ref(), 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[1];
    let body = body_deviation(&res, 1);
    eprintln!("after finish: {dev:?}\n  body: {body:?}");
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0);
    assert!(body.max_left < 0.06, "finish left {body:?}");
    // a flat tool on the 7.6 deg taper leaves a sawtooth up to stepover * slope (0.4 * 0.133)
    assert!(body.mean_left < 0.03, "finish left {body:?}");
}

fn min_left(res: &marcam::JobSim, x_lo: f64, x_hi: f64) -> f64 {
    let g = res.stock.grid;
    let mut m = f64::INFINITY;
    for i in 0..g.nx {
        let x = g.x(i);
        if x < x_lo || x > x_hi {
            continue;
        }
        for j in 0..g.na {
            if res.part.hi.at(i, j) > 0.0 {
                m = m.min(res.stock.at(i, j) - res.part.hi.at(i, j));
            }
        }
    }
    m
}

#[test]
fn profile_program_simulates_cleanly() {
    // the G-code that was cut on the Z1: no rapids through stock, no gouges, and the
    // body comes out within the finishing scallop except at the wall feet and chamfer
    let job = job("knob.job.json");
    let tps = plan(&job).unwrap();
    let res = simulate_job(&job, &tps[..1], None, 0.05, 720, 0.02).unwrap();
    let (_, st, dev) = &res.programs[0];
    assert!(st.rapid_hits.is_empty());
    assert_eq!(dev.gouge_cells, 0, "{dev:?}");
    // taper only (between the wall at -15 and the chamfer)
    let taper = sim::deviation(&res.stock, &res.part, -14.8, -1.0, 0.02);
    eprintln!("profile program, taper: {taper:?}");
    assert!(taper.max_left < 0.07, "{taper:?}");
    for g in gouge_check(&job, &tps, None).unwrap() {
        assert!(g.max_depth < 0.01, "{g:?}");
    }
}

#[test]
fn rough_never_cuts_below_allowance_surface() {
    let job = job("knob_finish.job.json");
    let mesh = load_mesh(&job, &root().join("examples/knob"))
        .unwrap()
        .unwrap();
    let mut rough_only = job.clone();
    rough_only.programs.truncate(1);
    // check against the part grown by the allowance: depth must stay below 0.15 + tol
    let tps = plan_with_mesh(&rough_only, Some(&mesh)).unwrap();
    let g = &gouge_check(&rough_only, &tps, Some(&mesh)).unwrap()[0];
    assert!(g.max_depth < 0.01);
    // A keeps turning one way within a level and levels alternate
    let mut a_last = 0.0;
    let mut dirs = Vec::new();
    for m in &tps[0].moves {
        if let marcam::Move::Cut { a, .. } = m {
            let d = (a - a_last).signum();
            if d != 0.0 && dirs.last() != Some(&d) {
                dirs.push(d);
            }
            a_last = *a;
        }
    }
    assert!(
        dirs.len() >= 10,
        "levels should alternate direction: {dirs:?}"
    );
}

/// Knob body radius (knob_rotary.py body_R) and the designed knurl depth (knurl_depth).
fn body_r(x: f64) -> f64 {
    8.0 + 2.0 * (-x) / 15.0
}

fn knurl_depth(x: f64) -> f64 {
    let p_perp = 2.0 * std::f64::consts::PI * body_r(x) / 30.0 * 30f64.to_radians().cos();
    0.95 * p_perp / (2.0 * 45f64.to_radians().tan())
}

/// Helix angle swept by a groove from its start at x = -15.6 (degrees), as in the program.
fn theta_deg(x: f64) -> f64 {
    let t = 30f64.to_radians().tan() * (0.6 / 10.0 + 7.5 * (10.0 / body_r(x)).ln());
    t.to_degrees()
}

#[test]
fn knurl_program_cuts_the_designed_diamonds() {
    // the cut G-code: profile, then knurl + V-bit chamfer, on one simulated bar
    let job = job("knob.job.json");
    let tps = plan(&job).unwrap();
    let grid = {
        let res = simulate_job(&job, &tps[..1], None, 0.05, 720, 0.02).unwrap();
        res.stock.grid
    };
    let cutters = |t: u32| {
        job.tool(t)
            .and_then(|d| marcam::cutter_for(t, d.shape).ok())
    };
    let mut stock = sim::RadialMap::filled(grid, job.stock.radius);
    let st1 = sim::simulate(&mut stock, &tps[0], &cutters, 0.1);
    let after_profile = stock.clone();
    let st2 = sim::simulate(&mut stock, &tps[1], &cutters, 0.1);
    assert!(
        st1.rapid_hits.is_empty() && st2.rapid_hits.is_empty(),
        "{st1:?} {st2:?}"
    );
    assert!(st2.samples > 10_000, "the knurl program was not simulated");
    let g = grid;
    let col =
        |phi_deg: f64| ((phi_deg.rem_euclid(360.0) / 360.0 * g.na as f64) as usize).min(g.na - 1);
    let row = |x: f64| (((x - g.x0) / g.dx) as usize).min(g.nx - 1);

    // 1. nothing behind the knurl's reach (tool tip runs out to -15.6, V-bit radius 1.5875)
    let mut touched_behind = 0;
    for i in 0..g.nx {
        if g.x(i) < -15.6 - 1.6 {
            for j in 0..g.na {
                if (after_profile.at(i, j) - stock.at(i, j)).abs() > 1e-6 {
                    touched_behind += 1;
                }
            }
        }
    }
    assert_eq!(touched_behind, 0, "the knurl program cut the boss or neck");

    // 2. thirty grooves per family, at the designed depth, where the helix says
    let mut worst = 0.0f64;
    let mut checked = 0;
    let mut x = -14.0;
    while x <= -1.5 {
        let i = row(x);
        let xc = g.x(i);
        let want = body_r(xc) - knurl_depth(xc);
        for fam in [1.0, -1.0] {
            for k in 0..30 {
                let a = k as f64 * 12.0 + fam * theta_deg(xc);
                let phi = 90.0 - a; // tool at A sits over part angle 90 - A
                let j = col(phi) as i64;
                // groove bottom: lowest cell within +-1 deg of the predicted centre
                let bottom = (-2..=2)
                    .map(|d| stock.at(i, (j + d).rem_euclid(g.na as i64) as usize))
                    .fold(f64::INFINITY, f64::min);
                worst = worst.max((bottom - want).abs());
                checked += 1;
            }
        }
        x += 0.5;
    }
    eprintln!(
        "knurl: {checked} groove bottoms checked, worst |bottom - designed| = {worst:.3} mm \
         (designed depth {:.3} mm at x=-14 .. {:.3} mm at x=-1.5)",
        knurl_depth(-14.0),
        knurl_depth(-1.5)
    );
    assert!(worst < 0.05, "groove bottoms off by {worst} mm");
    assert!(knurl_depth(-14.0) < 0.86 && knurl_depth(-1.5) > 0.69);

    // 3. never deeper than the designed groove anywhere on the body (no over-cut), and the
    //    diamond tops survive: fill 0.95 leaves small flats at the body radius. Tops lie on
    //    rows about 1.6 mm apart along X, so look for them in 2 mm windows.
    let mut deepest_excess = 0.0f64;
    for i in 0..g.nx {
        let xc = g.x(i);
        if !(-14.5..=-1.0).contains(&xc) {
            continue;
        }
        let floor = body_r(xc) - knurl_depth(xc);
        for j in 0..g.na {
            deepest_excess = deepest_excess.max(floor - stock.at(i, j));
        }
    }
    let mut lowest_top = f64::INFINITY;
    let mut x0 = -14.5;
    while x0 + 2.0 <= -1.0 {
        let top = (row(x0)..row(x0 + 2.0))
            .map(|i| {
                (0..g.na)
                    .map(|j| stock.at(i, j) - body_r(g.x(i)))
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .fold(f64::NEG_INFINITY, f64::max);
        lowest_top = lowest_top.min(top);
        x0 += 1.0;
    }
    eprintln!(
        "knurl: deepest cut below the designed groove floor {deepest_excess:.3} mm;          in every 2 mm of body the highest diamond top is {lowest_top:+.3} mm from the body radius"
    );
    assert!(deepest_excess < 0.03, "over-cut {deepest_excess} mm");
    assert!(
        lowest_top > -0.1,
        "diamond tops cut down by {lowest_top} mm"
    );

    // 4. diamonds: in every 12 degree window around the body there is a top and a bottom
    let i = row(-7.5);
    for w in 0..30 {
        let cells: Vec<f64> = (0..g.na / 30)
            .map(|d| stock.at(i, w * g.na / 30 + d))
            .collect();
        let (lo, hi) = cells
            .iter()
            .fold((f64::INFINITY, 0.0f64), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(
            hi - lo > 0.5 * knurl_depth(-7.5),
            "window {w}: relief {} mm",
            hi - lo
        );
    }

    // 5. front chamfer by the V-bit flank: a true 45 deg through (0, 7.2)
    let mut chamfer_worst = 0.0f64;
    let mut x = -0.6;
    while x <= -0.05 {
        let i = row(x);
        let top = (0..g.na).map(|j| stock.at(i, j)).fold(0.0, f64::max);
        chamfer_worst = chamfer_worst.max((top - (7.2 - g.x(i))).abs());
        x += 0.05;
    }
    eprintln!("chamfer: worst |surface - (7.2 - x)| = {chamfer_worst:.3} mm over x = -0.6..0");
    assert!(chamfer_worst < 0.05, "{chamfer_worst}");
}

#[test]
fn playback_matches_batch_and_tags_operations() {
    // the cut knob G-code on a coarse grid, both programs on one bar
    let job = job("knob.job.json");
    let tps = plan(&job).unwrap();
    let grid = simulate_job(&job, &tps[..1], None, 0.1, 180, 0.02)
        .unwrap()
        .stock
        .grid;
    let cutters = |t: u32| {
        job.tool(t)
            .and_then(|d| marcam::cutter_for(t, d.shape).ok())
    };
    let step = 0.2;

    // batch: whole programs
    let mut batch = sim::RadialMap::filled(grid, job.stock.radius);
    for tp in &tps {
        sim::simulate(&mut batch, tp, &cutters, step);
    }

    // playback: forwards, back to the middle (restores a snapshot), then to the end
    let mut pb = sim::Playback::new(
        sim::RadialMap::filled(grid, job.stock.radius),
        &tps,
        &cutters,
        step,
    );
    let n = pb.len();
    assert!(n > 30_000, "{n} segments");
    let mid = n / 2;
    pb.seek(mid);
    let at_mid = pb.stock.clone();
    let dirty = pb.seek(n).unwrap();
    assert!(dirty.0 <= dirty.1);
    assert_eq!(
        pb.stock.r, batch.r,
        "playback to the end differs from the batch simulation"
    );
    assert_eq!(pb.stock.op, batch.op);
    let back = pb.seek(mid);
    assert_eq!(
        back,
        Some((0, grid.nx - 1)),
        "jumping back redraws everything"
    );
    assert_eq!(
        pb.stock.r, at_mid.r,
        "jumping back to the middle differs from going forward"
    );
    assert_eq!(pb.stock.op, at_mid.op);
    pb.seek(n);
    assert_eq!(pb.stock.r, batch.r);

    // operation ids: 0 = rotary_turning, 1 = helical_knurl, 2 = flank_chamfer (tags = id + 1)
    let ops: Vec<(u32, String)> = tps
        .iter()
        .flat_map(|tp| tp.moves.iter())
        .filter_map(|m| match m {
            marcam::Move::Operation { id, kind } => Some((*id, kind.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        ops,
        vec![
            (0, "rotary_turning".into()),
            (1, "helical_knurl".into()),
            (2, "flank_chamfer".into())
        ]
    );
    let g = batch.grid;
    let col = |x: f64| ((x - g.x0) / g.dx) as usize;
    let tags_at = |x: f64| -> std::collections::BTreeSet<u16> {
        let i = col(x);
        (0..g.na).map(|j| batch.op[i * g.na + j]).collect()
    };
    // boss and neck: only the turning cut them
    assert_eq!(tags_at(-16.5), [1].into_iter().collect());
    assert_eq!(tags_at(-20.0), [1].into_iter().collect());
    // knurled body: the knurl cut every cell around (its V also skims the turning's
    // finishing scallop off the diamond tops)
    let body = tags_at(-7.5);
    assert_eq!(body, [2].into_iter().collect(), "{body:?}");
    // front chamfer region (tip at x=0.5): the V-bit chamfer pass
    assert!(tags_at(0.3).contains(&3), "{:?}", tags_at(0.3));
    // stock behind everything is untouched
    assert_eq!(tags_at(g.x0 + 0.05), [0].into_iter().collect());
}
