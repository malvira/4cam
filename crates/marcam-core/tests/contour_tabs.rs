//! 2D contours: climb or conventional milling (an outside contour climb-cut runs
//! counter-clockwise, spindle clockwise), tabs evenly round the outline or placed by hand
//! (a fraction of the way round the outline as written, the same place whichever way the
//! tool goes), a level at the tabs' top, and the machine's preferred direction for new ones.

use marcam::job::{Job, Operation};
use marcam::Move;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The plate with only its outline contour (op4), changed by `f`.
fn plate(f: impl Fn(&mut marcam::job::Contour)) -> Job {
    let dir = root().join("examples/plate");
    let text = std::fs::read_to_string(dir.join("plate.marcam.toml")).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut job = marcam::project::jobs_from_any_with(&text, &libs, &|s| s.load(&dir).ok())
        .unwrap()
        .remove(0)
        .1;
    for p in &mut job.programs {
        p.operations.retain(|o| matches!(o, Operation::Contour(_)));
        for o in &mut p.operations {
            if let Operation::Contour(c) = o {
                f(c);
            }
        }
    }
    job.programs.retain(|p| !p.operations.is_empty());
    job
}

/// The contour's cutting moves (x, y, z).
fn cuts(job: &Job) -> Vec<[f64; 3]> {
    let dir = root().join("examples/plate");
    let mesh = marcam::load_mesh(job, &dir).unwrap();
    let tps = marcam::plan_with_mesh(job, mesh.as_ref()).unwrap();
    tps.iter()
        .flat_map(|t| t.moves.iter())
        .filter_map(|m| match m {
            Move::Line3 { x, y, z, .. } => Some([*x, *y, *z]),
            _ => None,
        })
        .collect()
}

/// Signed area of the moves at one level (positive: counter-clockwise).
fn area_at(moves: &[[f64; 3]], z: f64) -> f64 {
    let pts: Vec<[f64; 3]> = moves
        .iter()
        .copied()
        .filter(|p| (p[2] - z).abs() < 1e-6)
        .collect();
    (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

#[test]
fn climb_runs_an_outside_contour_counter_clockwise_conventional_the_other_way() {
    let bottom = -2.5;
    let climb = cuts(&plate(|_| {}));
    let conv = cuts(&plate(|c| c.direction = Some("conventional".into())));
    assert!(area_at(&climb, bottom) > 0.0, "climb: counter-clockwise");
    assert!(area_at(&conv, bottom) < 0.0, "conventional: clockwise");
}

#[test]
fn tabs_leave_material_round_the_outline() {
    let job = plate(|c| {
        c.tabs = 4;
        c.tab_width = 5.0;
        c.tab_height = 1.0;
    });
    let m = cuts(&job);
    let (bottom, tab_top) = (-2.5, -1.5);
    // a level at the tabs' top, cut all round
    assert!(m.iter().any(|p| (p[2] - tab_top).abs() < 1e-6));
    // the last level: at the bottom except over the four tabs, where it rises to their top
    let low = m.iter().filter(|p| (p[2] - bottom).abs() < 1e-6).count();
    let up = m.iter().filter(|p| (p[2] - tab_top).abs() < 1e-6).count();
    assert!(
        low > 0 && up >= 8,
        "{low} at the bottom, {up} at the tabs' top"
    );
    // none: no move at the tabs' top below the level that is a whole pass
    let plain = cuts(&plate(|_| {}));
    assert!(!plain.iter().any(|p| (p[2] - tab_top).abs() < 1e-6) || plain.len() < m.len());
}

/// Where the tabs end: the XY where the last pass steps between the bottom and the tabs' top.
fn tab_points(m: &[[f64; 3]], tab_top: f64) -> Vec<[f64; 2]> {
    let (bottom, eps) = (-2.5, 1e-6);
    m.windows(2)
        .filter(|w| {
            ((w[0][2] - bottom).abs() < eps && (w[1][2] - tab_top).abs() < eps)
                || ((w[0][2] - tab_top).abs() < eps && (w[1][2] - bottom).abs() < eps)
        })
        .map(|w| [w[0][0], w[0][1]])
        .collect()
}

#[test]
fn a_tab_placed_by_hand_stays_put_whichever_way_the_tool_goes() {
    // a quarter of the way round the outline as written ([2,2] -> [58,2] -> [58,38] ...):
    // 46 mm along its 184, on its first side at (48, 2): the tool's path just outside it
    let tab = |dir: &str| {
        let m = cuts(&plate(|c| {
            c.tab_at = vec![0.25];
            c.tab_width = 4.0;
            c.direction = Some(dir.into());
        }));
        tab_points(&m, -1.5)
    };
    for dir in ["climb", "conventional"] {
        let pts = tab(dir);
        assert!(!pts.is_empty(), "{dir}: a tab");
        // (the pass's start, where it steps down, is among them too)
        let centred = pts.windows(2).any(|w| {
            ((w[0][0] + w[1][0]) / 2.0 - 48.0).abs() < 0.5 && w[0][1] < 2.0 && w[1][1] < 2.0
        });
        assert!(centred, "{dir}: the tab's ends {pts:?}");
    }
}

#[test]
fn a_new_contour_takes_the_machines_preferred_direction() {
    let mut m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let text = std::fs::read_to_string(root().join("examples/plate/plate.marcam.toml")).unwrap();
    let (p, _) = marcam::project::Project::from_json_any(&text).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let d = |m: &marcam::profiles::MachineProfile| {
        p.op_defaults("contour", "vise1", Some(1), &[], &libs, Some(m), &|_| None)
            .unwrap()["direction"]
            .clone()
    };
    assert_eq!(d(&m), serde_json::json!("climb"));
    m.cut_direction = Some("conventional".into());
    assert_eq!(d(&m), serde_json::json!("conventional"));
}
