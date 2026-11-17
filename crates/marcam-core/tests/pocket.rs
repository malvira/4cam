//! Pockets of any shape: an L-shaped outline with a round island, cleared by offset loops,
//! checked on the offsets themselves and on the block after a 3-axis simulation.

use marcam::engrave::Region;
use marcam::sim3::Heightmap3;
use marcam::strategy::three_axis::pocket_loops;
use marcam::{collision_check, cutter_for, limits_for, plan_with_mesh, post, verify_gcode, Job};
use serde_json::json;

const R: f64 = 3.175 / 2.0;

fn outline() -> Vec<Vec<[f64; 2]>> {
    let l = vec![
        [5.0, 5.0],
        [55.0, 5.0],
        [55.0, 20.0],
        [30.0, 20.0],
        [30.0, 35.0],
        [5.0, 35.0],
        [5.0, 5.0],
    ];
    let island: Vec<[f64; 2]> = (0..=48)
        .map(|k| {
            let t = k as f64 * std::f64::consts::TAU / 48.0;
            [15.0 + 4.0 * t.cos(), 15.0 + 4.0 * t.sin()]
        })
        .collect();
    vec![l, island]
}

/// Signed distance to the outline: inside the pocket (outline minus island) positive.
fn signed(region: &Region, p: [f64; 2]) -> f64 {
    let inside = region
        .spans(p[1])
        .iter()
        .any(|&(a, b)| p[0] >= a && p[0] < b);
    let d = region.distance(p, 1e9);
    if inside {
        d
    } else {
        -d
    }
}

fn job(paths: &[Vec<[f64; 2]>], stepover: f64) -> Job {
    let project = json!({
        "schema_version": "0.2",
        "name": "shaped",
        "models": [],
        "tools": [{"number": 1, "description": "3.175 flat", "kind": "flat", "diameter": 3.175, "flute_length": 12}],
        "setups": [{
            "id": "vise1", "machine": "makera-carvera-4axis",
            "stock": {"shape": "box", "min": [0.0, 0.0, -10.0], "max": [60.0, 40.0, 0.0]},
            "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false},
            "part": [], "spindle_rpm": 12000,
            "operations": [{
                "id": "op1", "type": "pocket", "tool": 1, "paths": paths,
                "depth": 3.0, "step_down": 1.0, "stepover": stepover,
                "feed": 300.0, "plunge_feed": 60.0
            }],
            "outputs": [{"name": "shaped", "title": "shaped pocket", "operations": ["op1"]}]
        }]
    });
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs =
        marcam::project::jobs_from_any_with(&project.to_string(), &libs, &|_| None).unwrap();
    jobs.remove(0).1
}

#[test]
fn offsets_sit_at_their_distance_and_finish_on_the_wall() {
    let paths = outline();
    let region = Region::new(&paths, 1.0);
    let step = 0.5 * 3.175;
    let loops = pocket_loops(&paths, R, step).unwrap();
    assert!(loops.len() > 5, "{}", loops.len());
    // every loop point is inside the pocket at least the tool radius from the walls
    for lp in &loops {
        assert!((lp[0][0] - lp[lp.len() - 1][0]).abs() < 1e-12, "closed");
        for &q in lp {
            assert!(
                signed(&region, q) > R - 0.01,
                "{q:?}: {}",
                signed(&region, q)
            );
        }
    }
    // the last loops are the finishing passes: exactly the tool radius from the walls,
    // one round the outline and one round the island
    let finish: Vec<_> = loops
        .iter()
        .filter(|lp| lp.iter().all(|&q| (signed(&region, q) - R).abs() < 0.01))
        .collect();
    assert_eq!(finish.len(), 2, "outline and island laps");
    // the island lap circles the island at 4 + R
    let round_island = finish
        .iter()
        .find(|lp| {
            lp.iter()
                .all(|q| ((q[0] - 15.0).hypot(q[1] - 15.0) - (4.0 + R)).abs() < 0.02)
        })
        .expect("a lap round the island");
    // climb milling: counter-clockwise round the outline, clockwise round the island
    let area = |lp: &Vec<[f64; 2]>| {
        lp.windows(2)
            .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
            .sum::<f64>()
            / 2.0
    };
    assert!(area(round_island) < 0.0);
    let outer = finish
        .iter()
        .find(|lp| !std::ptr::eq(**lp, *round_island))
        .unwrap();
    assert!(area(outer) > 0.0);
    // too narrow for the tool
    let slot = vec![vec![
        [0.0, 0.0],
        [20.0, 0.0],
        [20.0, 3.0],
        [0.0, 3.0],
        [0.0, 0.0],
    ]];
    assert!(pocket_loops(&slot, R, step).is_err());
}

#[test]
fn shaped_pocket_cuts_its_outline_and_spares_the_island() {
    let paths = outline();
    let region = Region::new(&paths, 1.0);
    for stepover in [0.4, 0.9] {
        let job = job(&paths, stepover);
        let tps = plan_with_mesh(&job, None).unwrap();
        let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
        let text = post(&tps[0], &m.post);
        assert!(verify_gcode(&text, &limits_for(&tps[0], &m.post)).is_empty());
        let coll = collision_check(&job, &tps, &m.post).unwrap();
        assert_eq!(coll[0].count, 0);

        let sb = job.stock_box.unwrap();
        let mut hm = Heightmap3::new(&sb, 0.1);
        let cutter = |n: u32| job.tool(n).and_then(|t| cutter_for(n, t.shape).ok());
        let st = marcam::sim3::simulate3(&mut hm, &tps[0], &cutter, 0.05);
        assert!(
            st.rapid_hits.is_empty(),
            "rapids through stock: {:?}",
            &st.rapid_hits[..3.min(st.rapid_hits.len())]
        );
        let (mut cut, mut spared) = (0, 0);
        for j in 0..hm.ny {
            for i in 0..hm.nx {
                let (x, y) = hm.centre(i, j);
                let z = hm.z[j * hm.nx + i];
                let s = signed(&region, [x, y]);
                if s > R + 0.1 {
                    // reachable by the tool centre: down to the floor
                    assert!(
                        (z + 3.0).abs() < 1e-6,
                        "stepover {stepover}: uncut at ({x:.2}, {y:.2}), z {z}"
                    );
                    cut += 1;
                } else if s < -0.1 {
                    // outside the outline, or on the island: untouched
                    assert!(
                        z.abs() < 1e-9,
                        "stepover {stepover}: gouge at ({x:.2}, {y:.2}), z {z}"
                    );
                    spared += 1;
                }
            }
        }
        assert!(cut > 50_000 && spared > 50_000, "{cut} {spared}");
    }
}

#[test]
fn a_paths_pocket_needs_no_rect_and_defaults_keep_the_rectangle() {
    let op = json!({"id": "op1", "type": "pocket", "tool": 1, "paths": outline(), "depth": 1.0,
                    "step_down": 1.0, "stepover": 0.5, "feed": 300.0, "plunge_feed": 60.0});
    assert!(
        marcam::ops::check(&op, None).is_empty(),
        "{:?}",
        marcam::ops::check(&op, None)
    );
    let t = marcam::ops::find("pocket").unwrap();
    let paths = t.params.iter().find(|p| p.name == "paths").unwrap();
    assert!(!paths.required);
    assert_eq!(paths.default, json!([]));
}
