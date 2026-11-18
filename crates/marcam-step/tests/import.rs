//! STEP import (tests/fixtures/step, made with SolidCraft's kernel): sizes and volumes, faces
//! as the engine's face regions, stable face ids, several bodies, errors, and a project whose
//! part is a STEP file planned through the engine.

use marcam::geometry::face_regions;
use marcam::Mesh;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(root().join("tests/fixtures/step").join(name)).unwrap()
}

fn bounds(m: &Mesh) -> ([f64; 3], [f64; 3]) {
    m.bounds().unwrap()
}

/// Enclosed volume (closed, outward triangles).
fn volume(m: &Mesh) -> f64 {
    m.triangles
        .iter()
        .map(|[a, b, c]| {
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

#[test]
fn a_box_with_a_hole() {
    let p = marcam_step::import(&fixture("box_with_hole.step"), 0.01).unwrap();
    assert_eq!((p.bodies, p.faces), (1, 7), "6 sides and the hole's wall");
    let (lo, hi) = bounds(&p.mesh);
    for (k, (a, b)) in [(0.0, 40.0), (0.0, 30.0), (-10.0, 0.0)].iter().enumerate() {
        assert!(
            (lo[k] - a).abs() < 1e-6 && (hi[k] - b).abs() < 1e-6,
            "{lo:?} {hi:?}"
        );
    }
    // 40 x 30 x 10 less an 8 mm hole 10 deep, within the chord's facets
    let want = 40.0 * 30.0 * 10.0 - std::f64::consts::PI * 16.0 * 10.0;
    let v = volume(&p.mesh);
    assert!((v - want).abs() / want < 1e-3, "{v} vs {want}");
    // the engine's regions are the B-rep faces, and the hole is found as a hole
    let (regions, _) = face_regions(&p.mesh, 30.0);
    assert_eq!(regions.len(), 7);
    assert!(regions.iter().all(|r| r.id.starts_with('s')));
    let picks = marcam::pickables::pickables(
        &p.mesh,
        marcam::pickables::Frame::Xyz {
            index_a: 0.0,
            sides: false,
        },
    );
    let holes: Vec<_> = picks.iter().filter(|x| x.kind == "hole").collect();
    assert_eq!(holes.len(), 1);
    assert!((holes[0].data["diameter"].as_f64().unwrap() - 8.0).abs() < 0.02);
    assert_eq!(holes[0].data["through"], true);
}

#[test]
fn the_revolved_knob_matches_its_profile() {
    let p = marcam_step::import(&fixture("knob.step"), 0.005).unwrap();
    let job = marcam::Job::from_json(
        &std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap(),
    )
    .unwrap();
    let prof = job.part.profile.unwrap();
    assert_eq!(
        p.faces,
        prof.points.len() - 1 + 2,
        "a face per profile segment, and the two ends"
    );
    let (lo, hi) = bounds(&p.mesh);
    let (x0, x1) = (
        prof.points.first().unwrap()[0].min(prof.points.last().unwrap()[0]),
        prof.points.first().unwrap()[0].max(prof.points.last().unwrap()[0]),
    );
    let rmax = prof.points.iter().map(|q| q[1]).fold(0.0, f64::max);
    assert!(
        (lo[0] - x0).abs() < 1e-6 && (hi[0] - x1).abs() < 1e-6,
        "{lo:?} {hi:?}"
    );
    assert!((hi[1] - rmax).abs() < 0.01 && (hi[2] - rmax).abs() < 0.01);
    // the same solid as the profile revolved finely
    let fine = Mesh::revolve(&prof, 2880);
    assert!(
        (volume(&p.mesh) - volume(&fine)).abs() / volume(&fine) < 0.002,
        "{} vs {}",
        volume(&p.mesh),
        volume(&fine)
    );
}

#[test]
fn several_bodies_come_in_together() {
    let p = marcam_step::import(&fixture("two_bodies.step"), 0.01).unwrap();
    assert_eq!(p.bodies, 2);
    assert_eq!(p.faces, 6 + 3, "a block and a cylinder");
    assert_eq!(p.body_names, vec!["block".to_string(), "puck".to_string()]);
    let want = 20.0 * 20.0 * 10.0 + std::f64::consts::PI * 36.0 * 10.0;
    assert!(
        (volume(&p.mesh) - want).abs() / want < 1e-3,
        "{} vs {want}",
        volume(&p.mesh)
    );
}

#[test]
fn face_ids_are_the_same_every_time_and_through_stl() {
    let bytes = fixture("box_with_hole.step");
    let a = marcam_step::import(&bytes, 0.01).unwrap();
    let b = marcam_step::import(&bytes, 0.01).unwrap();
    assert_eq!(a.mesh.faces, b.mesh.faces);
    let ids = |m: &Mesh| {
        face_regions(m, 30.0)
            .0
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&a.mesh), ids(&b.mesh));
    // through the STL the web app hands the engine: the same faces
    let stl = marcam_step::to_stl(&bytes, 0.01).unwrap();
    let back = Mesh::from_bytes(&stl).unwrap();
    assert_eq!(back.faces, a.mesh.faces);
    assert_eq!(ids(&back), ids(&a.mesh));
    // a finer chord: more triangles, the same faces
    let fine = marcam_step::import(&bytes, 0.002).unwrap();
    assert!(fine.mesh.triangles.len() > a.mesh.triangles.len());
    assert_eq!(ids(&fine.mesh), ids(&a.mesh));
}

#[test]
fn clear_errors() {
    let e = marcam_step::import(b"solid not a step file", 0.01).unwrap_err();
    assert!(e.contains("not a STEP file"), "{e}");
    let e = marcam_step::import(
        b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n",
        0.01,
    )
    .unwrap_err();
    assert!(
        e.contains("nothing to machine") || e.contains("STEP"),
        "{e}"
    );
    // without the reader installed the engine says what it needs; with it, it reads STEP
    let bytes = fixture("box_with_hole.step");
    marcam_step::register();
    let m = Mesh::from_bytes(&bytes).unwrap();
    assert_eq!(face_regions(&m, 30.0).0.len(), 7);
}

#[test]
fn a_project_whose_part_is_a_step_file_plans() {
    marcam_step::register();
    let dir = root().join("tests/fixtures/step");
    let project = serde_json::json!({
        "schema_version": "0.2", "name": "box", "models": [{"id": "part", "name": "box", "kind": "mesh", "file": "box_with_hole.step"}],
        "tools": [{"number": 1, "description": "3.175 flat", "kind": "flat", "diameter": 3.175, "flute_length": 15},
                  {"number": 2, "description": "3 ball", "kind": "ball", "diameter": 3.0, "flute_length": 15}],
        "setups": [{"id": "s", "kind": "vise", "machine": "makera-carvera-4axis",
            "stock": {"shape": "box", "min": [0.0, 0.0, -10.0], "max": [40.0, 30.0, 1.0]},
            "clearance": 3.0, "holding": {"kind": "vise", "tailstock": false}, "part": ["part"], "spindle_rpm": 12000,
            "operations": [{"id": "op1", "type": "bore", "tool": 1, "points": [[20.0, 15.0]], "diameter": 8.0, "depth": 11.5, "top": 1.0, "feed": 300.0, "plunge_feed": 60.0},
                           {"id": "op2", "type": "raster3d", "tool": 2, "x_range": [0.0, 40.0], "y_range": [0.0, 30.0], "stepover": 1.0, "sample": 0.5,
                            "step_down": 2.0, "floor": -1.0, "feed": 400.0, "plunge_feed": 60.0, "faces": []}],
            "outputs": [{"name": "box", "title": "box", "operations": ["op1", "op2"]}]}]
    });
    let libs = marcam::profiles::builtin_libraries();
    let jobs = marcam::project::jobs_from_any_with(&project.to_string(), &libs, &|src| {
        src.load(&dir).ok()
    })
    .unwrap();
    let job = &jobs[0].1;
    let mesh = marcam::load_mesh(job, &dir)
        .unwrap()
        .expect("the STEP part");
    assert_eq!(face_regions(&mesh, 30.0).0.len(), 7);
    let tps = marcam::plan_with_mesh(job, Some(&mesh)).unwrap();
    let m = marcam::profiles::builtin_machine("makera-carvera-4axis").unwrap();
    let text = marcam::post(&tps[0], &m.post);
    assert!(marcam::verify_gcode(&text, &marcam::limits_for(&tps[0], &m.post)).is_empty());
    assert!(text.lines().count() > 100);
}
