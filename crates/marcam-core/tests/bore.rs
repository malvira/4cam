//! Boring: holes bigger than the end mill, interpolated (helical entry, roughing rings,
//! finish, spring passes), as G2/G3 arcs or straight segments; checked on the toolpath, the
//! G-code (arcs, direction, verifier, time) and a 3-axis simulation of the block.

use marcam::sim3::Heightmap3;
use marcam::{cutter_for, estimate, limits_for, plan_with_mesh, post, verify_gcode, Job, Move};
use serde_json::{json, Value};

const R_TOOL: f64 = 3.175 / 2.0;

/// A 40 x 30 x 10 block (top Z0), a 3.175 mm flat, boring `op` (merged over the defaults
/// below), on a vise or (`indexed`) on a 4th axis.
fn job(op: Value, indexed: bool) -> Job {
    let mut o = json!({
        "id": "op1", "type": "bore", "tool": 1, "points": [[10.0, 15.0], [30.0, 15.0]],
        "diameter": 8.0, "depth": 10.5, "helix_pitch": 0.5, "stepover": 0.4,
        "finish_allowance": 0.1, "spring_passes": 1, "direction": "climb", "arcs": true,
        "tolerance": 0.002, "feed": 300.0, "plunge_feed": 60.0
    });
    for (k, v) in op.as_object().unwrap() {
        o[k] = v.clone();
    }
    let setup = if indexed {
        json!({"id": "s", "kind": "indexed", "machine": "makera-z1-4axis",
               "stock": {"shape": "cylinder", "radius": 12.0, "length": 40.0},
               "clearance": 3.0, "holding": {"kind": "rotary_chuck", "tailstock": false},
               "part": [], "spindle_rpm": 12000, "operations": [o],
               "outputs": [{"name": "bore", "title": "bore", "operations": ["op1"]}]})
    } else {
        json!({"id": "s", "kind": "vise", "machine": "makera-carvera-4axis",
               "stock": {"shape": "box", "min": [0.0, 0.0, -10.0], "max": [40.0, 30.0, 0.0]},
               "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false},
               "part": [], "spindle_rpm": 12000, "operations": [o],
               "outputs": [{"name": "bore", "title": "bore", "operations": ["op1"]}]})
    };
    let project = json!({
        "schema_version": "0.2", "name": "bore", "models": [],
        "tools": [{"number": 1, "description": "3.175 flat", "kind": "flat", "diameter": 3.175, "flute_length": 15}],
        "setups": [setup]
    });
    let libs = marcam::profiles::builtin_libraries();
    marcam::project::jobs_from_any_with(&project.to_string(), &libs, &|_| None)
        .unwrap()
        .remove(0)
        .1
}

fn machine(indexed: bool) -> marcam::profiles::MachineProfile {
    marcam::profiles::builtin_machine(if indexed {
        "makera-z1-4axis"
    } else {
        "makera-carvera-4axis"
    })
    .unwrap()
}

#[test]
fn the_tool_stays_inside_the_holes_and_finishes_at_the_size() {
    let job = job(json!({}), false);
    let tp = plan_with_mesh(&job, None).unwrap().remove(0);
    let finish = 4.0 - R_TOOL;
    let mut at_size = 0;
    for m in &tp.moves {
        if let Move::Line3 { x, y, z, .. } = m {
            let r = [[10.0, 15.0], [30.0, 15.0]]
                .iter()
                .map(|c: &[f64; 2]| (x - c[0]).hypot(y - c[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(
                r <= finish + 1e-9,
                "({x:.3}, {y:.3}) is {r:.4} from the centre, past {finish:.4}"
            );
            if (r - finish).abs() < 1e-9 && (*z + 10.5).abs() < 1e-9 {
                at_size += 1;
            }
        }
    }
    assert!(
        at_size > 0,
        "the finish runs round at the size, at the bottom"
    );
}

#[test]
fn arcs_climb_conventional_and_segments() {
    let m = machine(false);
    let text = |op: Value| post(&plan_with_mesh(&job(op, false), None).unwrap()[0], &m.post);
    let climb = text(json!({}));
    let g = |t: &str, w: &str| t.lines().filter(|l| l.starts_with(w)).count();
    assert!(
        g(&climb, "G3 ") > 0 && g(&climb, "G2 ") == 0,
        "climb: counter-clockwise inside the hole"
    );
    let conv = text(json!({"direction": "conventional"}));
    assert!(g(&conv, "G2 ") > 0 && g(&conv, "G3 ") == 0);
    let lines = text(json!({"arcs": false}));
    assert!(g(&lines, "G2 ") + g(&lines, "G3 ") == 0);
    assert!(
        climb.lines().count() * 5 < lines.lines().count(),
        "{} vs {} lines",
        climb.lines().count(),
        lines.lines().count()
    );
    // both verify clean, and take the same time (the arcs replace segments within 2 microns)
    let tp = plan_with_mesh(&job(json!({}), false), None)
        .unwrap()
        .remove(0);
    let lim = limits_for(&tp, &m.post);
    assert!(
        verify_gcode(&climb, &lim).is_empty(),
        "{:?}",
        verify_gcode(&climb, &lim)
    );
    assert!(verify_gcode(&lines, &lim).is_empty());
    let dy = estimate::Dynamics::from_profile(&m);
    let (ta, tl) = (
        estimate::estimate_gcode(&climb, &dy),
        estimate::estimate_gcode(&lines, &dy),
    );
    let floor = |e: &estimate::Estimate| e.feed_mm;
    assert!(
        (floor(&ta) - floor(&tl)).abs() / floor(&tl) < 0.001,
        "{} vs {} mm",
        ta.feed_mm,
        tl.feed_mm
    );
    // an arc's end is on the circle and its centre is the hole's
    let arc = climb.lines().find(|l| l.starts_with("G3 ")).unwrap();
    let w = |c: char| {
        arc.split_whitespace()
            .find_map(|t| t.strip_prefix(c)?.parse::<f64>().ok())
            .unwrap()
    };
    let (x, y) = (w('X'), w('Y'));
    let r = (x - 10.0).hypot(y - 15.0).min((x - 30.0).hypot(y - 15.0));
    assert!(r <= 4.0 - R_TOOL + 0.002, "{arc}");
}

#[test]
fn the_simulated_block_has_the_holes_and_nothing_else_cut() {
    let job = job(json!({}), false);
    let tp = plan_with_mesh(&job, None).unwrap().remove(0);
    let sb = job.stock_box.unwrap();
    let mut hm = Heightmap3::new(&sb, 0.1);
    let cutter = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
    let st = marcam::sim3::simulate3(&mut hm, &tp, &cutter, 0.05);
    assert!(st.rapid_hits.is_empty());
    let (mut inside, mut outside) = (0, 0);
    for j in 0..hm.ny {
        for i in 0..hm.nx {
            let (x, y) = hm.centre(i, j);
            let z = hm.z[j * hm.nx + i];
            let r = (x - 10.0).hypot(y - 15.0).min((x - 30.0).hypot(y - 15.0));
            if r < 4.0 - 0.1 {
                assert!(
                    z <= -10.0 + 1e-9,
                    "uncut in a hole at ({x:.2}, {y:.2}): z {z}"
                );
                inside += 1;
            } else if r > 4.0 + 0.1 {
                assert!(
                    z.abs() < 1e-9,
                    "cut outside the holes at ({x:.2}, {y:.2}): z {z}"
                );
                outside += 1;
            }
        }
    }
    assert!(inside > 0 && outside > 0);
}

#[test]
fn errors_blind_holes_and_a_radial_hole_on_a_fourth_axis() {
    // a hole the tool cannot bore
    let e = plan_with_mesh(&job(json!({"diameter": 3.0}), false), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("drill it instead"), "{e}");
    // blind: the bottom at the depth
    let tp = plan_with_mesh(&job(json!({"depth": 4.0}), false), None)
        .unwrap()
        .remove(0);
    let low = tp
        .moves
        .iter()
        .filter_map(|m| {
            if let Move::Line3 { z, .. } = m {
                Some(*z)
            } else {
                None
            }
        })
        .fold(f64::INFINITY, f64::min);
    assert!((low + 4.0).abs() < 1e-9);
    // indexed (a radial hole, turned to A90): general moves, written as straight segments
    let job = job(
        json!({"index_a": 90.0, "points": [[-20.0, 0.0]], "depth": 5.0, "top": 12.0}),
        true,
    );
    let tps = plan_with_mesh(&job, None).unwrap();
    let m = machine(true);
    let text = post(&tps[0], &m.post);
    assert!(text
        .lines()
        .all(|l| !l.starts_with("G2 ") && !l.starts_with("G3 ")));
    assert!(text.contains("A90"), "turned to A90");
    assert!(verify_gcode(&text, &limits_for(&tps[0], &m.post)).is_empty());
}

#[test]
fn bore_takes_holes_from_picking() {
    use marcam::pickables::{apply, pickables, ApplyContext, Frame};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bracket = marcam::Mesh::load(&root.join("tests/fixtures/picking/bracket.stl")).unwrap();
    let p = pickables(
        &bracket,
        Frame::Xyz {
            index_a: 0.0,
            sides: false,
        },
    );
    let blind: Vec<_> = p
        .iter()
        .filter(|x| x.kind == "hole" && x.data["through"] == false)
        .collect();
    let cx = ApplyContext {
        stock_top: Some(0.0),
        tool_diameter: Some(3.175),
        model: "part".into(),
        op: None,
    };
    let (v, notes) = apply("bore", &blind, &cx).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(v["diameter"], 6.0);
    assert_eq!(v["depth"], 6.0);
    assert_eq!(v["points"], json!([[46.0, 20.0]]));
    // a tool as big as the hole
    let big = ApplyContext {
        tool_diameter: Some(6.0),
        ..cx
    };
    let (_, notes) = apply("bore", &blind, &big).unwrap();
    assert!(notes.iter().any(|n| n.contains("too big")), "{notes:?}");
}
