//! Hand-timed fixtures (tests/fixtures/timing): simple programs whose cutting time can be
//! worked out on paper from the operation's geometry and feeds. For each one:
//! - the estimate at the programmed feeds, with no acceleration or axis limits (the floor),
//!   matches the hand figure, from the posted G-code, in every feed mode;
//! - the full estimate (acceleration, junctions, axis limits), from the toolpath and from the
//!   G-code, is no more than a few percent above the floor.
//!
//! Tool: 4 mm flat (radius 2). Feeds 600 mm/min (turning: 240), plunges 60 mm/min. The
//! time of each line below is length / feed; plunges are listed separately.

use marcam::estimate::{estimate_gcode, estimate_toolpath, Dynamics};
use marcam::machine::FeedMode;
use marcam::profiles::MachineProfile;
use marcam::{load_mesh, plan_with_mesh, post, Toolpath};
use std::f64::consts::PI;
use std::path::Path;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/timing");

fn plan(name: &str) -> (Toolpath, MachineProfile) {
    let path = format!("{DIR}/{name}.project.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let dir = Path::new(DIR).to_path_buf();
    let meshes = |src: &marcam::job::MeshSource| src.load(&dir).ok();
    let libs = marcam::profiles::builtin_libraries();
    let (_, job) = marcam::project::jobs_from_any_with(&text, &libs, &meshes)
        .unwrap()
        .remove(0);
    let profile = marcam::profiles::builtin_machine(job.machine.as_deref().unwrap()).unwrap();
    let mesh = load_mesh(&job, &dir).unwrap();
    let mut tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    assert_eq!(tps.len(), 1);
    (tps.remove(0), profile)
}

/// No acceleration, no axis or controller limits: the programmed feeds only.
fn floor(d: &Dynamics) -> Dynamics {
    let mut f = d.clone();
    f.accel = [f64::INFINITY; 4];
    f.max_feed = [1e12; 4];
    f.feed_cap = f64::INFINITY;
    f
}

/// Check one fixture: `hand_s` is the hand-worked time of the feed moves (cuts + plunges).
fn check(name: &str, profile_edit: impl Fn(&mut MachineProfile), hand_s: f64, slack: f64) {
    let (tp, mut profile) = plan(name);
    profile_edit(&mut profile);
    let g = post(&tp, &profile.post);
    let dy = Dynamics::from_profile(&profile);
    let fl = estimate_gcode(&g, &floor(&dy));
    assert!(
        (fl.feed_s - hand_s).abs() / hand_s < 0.003,
        "{name} ({:?}): feed time at the programmed feeds {:.2} s, by hand {hand_s:.2} s",
        profile.post.feed_mode,
        fl.feed_s
    );
    // the floor of the whole program: these feeds, the rapids at their rates, the rest
    let mut no_accel = dy.clone();
    no_accel.accel = [f64::INFINITY; 4];
    let base = estimate_gcode(&g, &no_accel).total_s;
    assert!(base >= fl.total_s - 1e-6);
    let from_gcode = estimate_gcode(&g, &dy).total_s;
    let from_tp = estimate_toolpath(&tp, &profile).estimate.total_s;
    for (what, t) in [("G-code", from_gcode), ("toolpath", from_tp)] {
        let r = t / base;
        assert!(
            (1.0 - 1e-9..=1.0 + slack).contains(&r),
            "{name} ({:?}): estimate from the {what} {t:.2} s is {r:.3}x the no-acceleration {base:.2} s",
            profile.post.feed_mode
        );
    }
    println!(
        "{name:<14} {:>18} hand {hand_s:>7.1} s  floor {:>7.1} s  no-accel {base:>7.1} s  estimate {from_gcode:>7.1} s ({:.3}x)",
        format!("{:?}", profile.post.feed_mode),
        fl.feed_s,
        from_gcode / base
    );
}

const FEED: f64 = 600.0 / 60.0; // mm/s
const PLUNGE: f64 = 1.0; // 60 mm/min in mm/s

#[test]
fn facing_a_cube() {
    // 40 x 40 block, one 0.5 mm level, stepover 2 mm. The rows overhang the stock by half the
    // tool radius: X -1..41 (42 mm), Y 1..39 in ceil(38 / 2) = 19 steps -> 20 rows.
    // Lead-in from X-4 to X-1 (3 mm), 20 rows x 42 + 19 steps x 2 = 878 mm; 1 mm plunge.
    let hand = (3.0 + 20.0 * 42.0 + 19.0 * 2.0) / FEED + 1.0 / PLUNGE;
    check("facing", |_| {}, hand, 0.03);
}

#[test]
fn contouring_a_square() {
    // 30 x 30 square outside, offset by the radius: 34 x 34 (136 mm per lap); 2 levels of
    // 1 mm, each entered with a 1 mm plunge.
    let hand = 2.0 * (136.0 / FEED + 1.0 / PLUNGE);
    check("contour", |_| {}, hand, 0.03);
}

#[test]
fn rectangular_pocket() {
    // 30 x 20 pocket: tool centre X 7..33 (26), Y 7..23 (16); stepover 2 -> 8 steps, 9 rows.
    // Zigzag 9 x 26 + 8 x 2 = 250 mm; it ends at (33, 23); the wall lap goes back to (7, 7)
    // (the diagonal, hypot(26, 16)) and round: 26 + 16 + 26 + 16. Two 1 mm levels, a 1 mm
    // plunge each.
    let lap = 26f64.hypot(16.0) + 2.0 * (26.0 + 16.0);
    let hand = 2.0 * ((250.0 + lap) / FEED + 1.0 / PLUNGE);
    check("pocket", |_| {}, hand, 0.03);
}

#[test]
fn drill_pattern() {
    // 4 holes 3 mm deep, pecks of 1, from 1 mm above the top at 60 mm/min. First peck from
    // +1 to -1 (2 mm); the next ones restart 0.2 above the last depth: 1.2 + 1.2.
    let hand = 4.0 * (2.0 + 1.2 + 1.2) / PLUNGE;
    // genuinely acceleration-bound: every peck stops and reverses, and the rapids between are
    // 1 to 3 mm, too short to reach the rapid rate at 500 mm/s^2
    check("drilling", |_| {}, hand, 0.08);
}

#[test]
fn raster_over_a_flat_plate() {
    // 30 x 20 plate, top at Z0, raster X 0..30, Y 0..20 every 2 mm: 11 rows of 30 and 10
    // links of 2 (zigzag) = 350 mm at Z0, one pass (step-down beyond the depth), entered with
    // a 1 mm plunge.
    let hand = 350.0 / FEED + 1.0 / PLUNGE;
    check("raster_plate", |_| {}, hand, 0.03);
}

/// Turning a 10 mm bar to 8 mm (4 mm flat, 240 mm/min, 6 deg segments, 2 mm per rev).
/// - Roughing at radius 9, from X-38 (the profile's end plus the tool radius) to X13 (the
///   front plus the tool radius plus 1): a 2.5 mm plunge (13 -> 10.5), a ramp revolution from
///   10.5 to 9 (arc at the mean radius 9.75), one revolution, the 51 mm helix (25.5 revs of
///   hypot(2, 2 pi 9)), one revolution.
/// - The last level (radius 8) is the part: finishing cuts it, from X12.5 to X-37.98
///   (50.48 mm): a 4.5 mm plunge (13 -> 8.5), a ramp from 8.5 to 8 (mean 8.25), one
///   revolution, 25.24 revs of hypot(2, 2 pi 8), one revolution.
fn turning_hand() -> f64 {
    let rev = |r: f64| 2.0 * PI * r;
    let helix = |len: f64, r: f64| len / 2.0 * 2f64.hypot(rev(r));
    let rough = rev(9.75) + rev(9.0) + helix(51.0, 9.0) + rev(9.0);
    let finish = rev(8.25) + rev(8.0) + helix(12.5 + 37.98, 8.0) + rev(8.0);
    (rough + finish) / 4.0 + (2.5 + 4.5) / PLUNGE
}

#[test]
fn turning_a_cylinder_in_each_feed_mode() {
    let hand = turning_hand();
    // Z1: degrees as length
    check("turning", |_| {}, hand, 0.03);
    // linear only
    check(
        "turning",
        |p| p.post.feed_mode = FeedMode::LinearOnly,
        hand,
        0.03,
    );
    // inverse time (G93): the LinuxCNC post, whose preamble selects G93
    let linuxcnc = marcam::profiles::builtin_machine("linuxcnc-4axis").unwrap();
    assert_eq!(linuxcnc.post.feed_mode, FeedMode::InverseTime);
    check(
        "turning",
        |p| {
            p.post = linuxcnc.post.clone();
            p.kinematics = linuxcnc.kinematics.clone();
        },
        hand,
        0.03,
    );
    // surface speed as the Carvera firmware reads it: radius from Y/Z plus the 15 mm pad,
    // A at most 5 RPM (240 mm/min at radius 8 is 4.8 RPM: under it)
    let carvera = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    check(
        "turning",
        |p| {
            p.post.feed_mode = carvera.post.feed_mode;
            p.post.surface_radius_pad = carvera.post.surface_radius_pad;
            p.kinematics = carvera.kinematics.clone();
        },
        hand,
        0.03,
    );
}

#[test]
fn surface_speed_feeds() {
    use marcam::machine::feed_length;
    // a full turn at radius 10 with a 15 mm pad: the controller's circle is 2 pi 25
    let l = feed_length(FeedMode::SurfaceSpeed, 15.0, [0.0, 0.0, 0.0, 360.0], 10.0);
    assert!((l - 2.0 * PI * 25.0).abs() < 1e-9);
    // linear moves are plain mm, whatever the pad
    let l = feed_length(FeedMode::SurfaceSpeed, 15.0, [3.0, 4.0, 0.0, 0.0], 10.0);
    assert!((l - 5.0).abs() < 1e-12);
    // the other modes
    assert!(
        (feed_length(FeedMode::DegreesAsLength, 0.0, [3.0, 0.0, 0.0, 4.0], 9.0) - 5.0).abs()
            < 1e-12
    );
    assert_eq!(
        feed_length(FeedMode::LinearOnly, 0.0, [3.0, 0.0, 0.0, 40.0], 9.0),
        3.0
    );
    assert_eq!(
        feed_length(FeedMode::LinearOnly, 0.0, [0.0, 0.0, 0.0, 40.0], 9.0),
        40.0
    );

    // the Carvera post: a full turn at radius 9 at 240 mm/min of surface takes
    // 2 pi 9 / 240 min; F is written for the controller's circle (radius 9 + 15)
    let (tp, mut profile) = plan("turning");
    let carvera = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    profile.post = carvera.post.clone();
    profile.kinematics = carvera.kinematics.clone();
    let g = post(&tp, &profile.post);
    let line = g
        .lines()
        .find(|l| l.contains(" Z9.000 A") && l.starts_with("G1 X-38.000"))
        .expect("a revolution at radius 9");
    let f: f64 = line.rsplit('F').next().unwrap().parse().unwrap();
    assert!((f - 240.0 * 24.0 / 9.0).abs() <= 1.0, "{line}");
    // the verifier reads the same rule: the ramps descend within the plunge feed
    let issues = marcam::verify_gcode(&g, &marcam::limits_for(&tp, &profile.post));
    assert!(issues.is_empty(), "{issues:?}");
    // stock firmware's 5 RPM cap is the A axis's top rate in the estimate
    assert_eq!(Dynamics::from_profile(&carvera).max_feed[3], 1800.0);
}

/// Every example's estimate is between 1.0 and 1.6 times its floor (the programmed feeds,
/// rapids at their rates, tool changes and dwells, no acceleration): the planner adds only
/// what acceleration and junctions cost.
#[test]
fn examples_stay_near_their_floor() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let libs = marcam::profiles::builtin_libraries();
    for file in [
        "knob/knob.marcam.toml",
        "knob/knob_finish.marcam.toml",
        "knob/knob_indexed.marcam.toml",
        "plate/plate.marcam.toml",
        "ring/ring.marcam.toml",
        "plaque/plaque.marcam.toml",
    ] {
        let path = Path::new(root).join(file);
        let dir = path.parent().unwrap().to_path_buf();
        let meshes = |src: &marcam::job::MeshSource| src.load(&dir).ok();
        let text = std::fs::read_to_string(&path).unwrap();
        for (_, job) in marcam::project::jobs_from_any_with(&text, &libs, &meshes).unwrap() {
            let profile = marcam::profiles::builtin_machine(
                job.machine.as_deref().unwrap_or("makera-z1-4axis"),
            )
            .unwrap();
            let mesh = load_mesh(&job, &dir).unwrap();
            for tp in plan_with_mesh(&job, mesh.as_ref()).unwrap() {
                let g = post(&tp, &profile.post);
                let dy = Dynamics::from_profile(&profile);
                let mut no_accel = dy.clone();
                no_accel.accel = [f64::INFINITY; 4];
                let base = estimate_gcode(&g, &no_accel).total_s;
                let full = estimate_toolpath(&tp, &profile).estimate.total_s;
                let r = full / base;
                assert!(
                    (1.0 - 1e-9..=1.6).contains(&r),
                    "{file} {}: {full:.0} s is {r:.2}x its floor {base:.0} s",
                    tp.name
                );
            }
        }
    }
}
