//! Kinematics-aware posting: a machine's axes (letters, directions, handedness, what turns)
//! decide the words; the toolpath's convention (tool motion in X/Y/Z, A turning the work
//! right-handed about +X) stays the same.

use fourcam::profiles::{builtin_machines, MachineProfile};
use fourcam::{plan, post, Job};

fn z1_with(edit: impl Fn(&mut serde_json::Value)) -> Result<MachineProfile, String> {
    let mut v =
        serde_json::to_value(fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap())
            .unwrap();
    edit(&mut v);
    MachineProfile::from_json(&v.to_string())
}

#[test]
fn built_in_machines_map_one_to_one() {
    for m in builtin_machines() {
        assert!(m.post.axes.is_identity(), "{}: {:?}", m.id, m.post.axes);
    }
}

#[test]
fn rotary_sign_and_letter_follow_the_machine() {
    let left = z1_with(|v| v["kinematics"]["rotary"][0]["handedness"] = "left".into()).unwrap();
    assert_eq!(left.post.axes.signs[3], -1.0);
    let minus_x =
        z1_with(|v| v["kinematics"]["rotary"][0]["axis"] = serde_json::json!([-1, 0, 0])).unwrap();
    assert_eq!(minus_x.post.axes.signs[3], -1.0);
    let head = z1_with(|v| v["kinematics"]["rotary"][0]["carries"] = "tool".into()).unwrap();
    assert_eq!(head.post.axes.signs[3], -1.0);
    // two flips cancel; another letter is used as given
    let b = z1_with(|v| {
        v["kinematics"]["rotary"][0]["handedness"] = "left".into();
        v["kinematics"]["rotary"][0]["axis"] = serde_json::json!([-1, 0, 0]);
        v["kinematics"]["rotary"][0]["letter"] = "B".into();
    })
    .unwrap();
    assert_eq!(
        (b.post.axes.letters[3].as_str(), b.post.axes.signs[3]),
        ("B", 1.0)
    );
    // a reversed Y; and a diagonal axis cannot be posted
    let y = z1_with(|v| v["kinematics"]["linear"][1]["direction"] = serde_json::json!([0, -1, 0]))
        .unwrap();
    assert_eq!(y.post.axes.signs[1], -1.0);
    let diag =
        z1_with(|v| v["kinematics"]["linear"][1]["direction"] = serde_json::json!([0, 1, 1]));
    assert!(diag.unwrap_err().contains("along X, Y or Z"));
}

#[test]
fn a_left_handed_rotary_gets_the_knob_with_every_a_negated() {
    let job = Job::from_json(include_str!("../../../examples/knob/knob.job.json")).unwrap();
    let tps = plan(&job).unwrap();
    let z1 = fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    let left = z1_with(|v| v["kinematics"]["rotary"][0]["handedness"] = "left".into()).unwrap();
    let flip = |w: &str| -> String {
        match w.strip_prefix('A') {
            Some(n) => {
                let v: f64 = n.parse().unwrap();
                format!("A{:.3}", -v)
            }
            None => w.to_string(),
        }
    };
    for tp in &tps {
        let (a, b) = (post(tp, &z1.post), post(tp, &left.post));
        assert_eq!(a.lines().count(), b.lines().count());
        let mut changed = 0;
        for (la, lb) in a.lines().zip(b.lines()) {
            // comments and templates (G92 A0) aside, the A words are negated
            let code = la.split('(').next().unwrap_or("");
            if code.starts_with("G92") || code.trim().is_empty() {
                continue;
            }
            let expect: Vec<String> = code.split_whitespace().map(flip).collect();
            let got: Vec<&str> = lb
                .split('(')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            assert_eq!(
                expect.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                got,
                "{la} | {lb}"
            );
            if la.contains(" A") {
                changed += 1;
            }
        }
        assert!(changed > 100, "{}: {changed} lines with A", tp.name);
    }
}

fn pose_program(moves: Vec<fourcam::Move>) -> fourcam::Toolpath {
    fourcam::Toolpath {
        name: "t".into(),
        title: "t".into(),
        tool: None,
        notes: vec![],
        reset_a: false,
        safe_r: 20.0,
        moves,
        origin_comment: None,
        plunge_limit: None,
    }
}

#[test]
fn general_moves_are_solved_for_the_machine() {
    use fourcam::Move;
    let z1 = fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    // the tool along the part's +Y: A90; the part point (0, 0, 10) is then at machine Y -10
    let tp = pose_program(vec![
        Move::Pose {
            tip: [0.0, 0.0, 10.0],
            axis: [0.0, 1.0, 0.0],
            feed: None,
        },
        // 10 mm along the part's X at F300: the controller's F is 300 (A does not move)
        Move::Pose {
            tip: [10.0, 0.0, 10.0],
            axis: [0.0, 1.0, 0.0],
            feed: Some(300.0),
        },
        // a direction leaning along X needs a fifth axis: the program stops, visibly
        Move::Pose {
            tip: [10.0, 0.0, 10.0],
            axis: [0.5, 0.0, 1.0],
            feed: None,
        },
    ]);
    let g = post(&tp, &z1.post);
    assert!(g.contains("G0 X0.000 Y-10.000 Z0.000 A90.000\n"), "{g}");
    assert!(g.contains("G1 X10.000 Y-10.000 Z0.000 F300\n"), "{g}");
    assert!(
        g.contains("second rotary axis") && g.contains("\nM0\n"),
        "{g}"
    );
    // a machine whose A turns the other way gets A-90 for the same pose
    let left = z1_with(|v| v["kinematics"]["rotary"][0]["handedness"] = "left".into()).unwrap();
    assert!(post(&tp, &left.post).contains("A-90.000"));
}

#[test]
fn the_indexed_knob_runs_on_a_left_handed_machine_with_a_negated() {
    let text = include_str!("../../../examples/knob/knob_indexed.project.json");
    let libs = fourcam::profiles::builtin_libraries();
    let job = fourcam::project::jobs_from_any_with(text, &libs, &|_| None)
        .unwrap()
        .remove(0)
        .1;
    let tps = plan(&job).unwrap();
    let z1 = fourcam::profiles::builtin_machine("makera-z1-4axis").unwrap();
    let left = z1_with(|v| v["kinematics"]["rotary"][0]["handedness"] = "left".into()).unwrap();
    let (a, b) = (post(&tps[2], &z1.post), post(&tps[2], &left.post));
    assert!(a.contains("G0 A90.000") && b.contains("G0 A-90.000"));
    for (la, lb) in a.lines().zip(b.lines()) {
        if !la.contains(" A") && !la.starts_with("G0 A") {
            assert_eq!(la, lb);
        }
    }
}
