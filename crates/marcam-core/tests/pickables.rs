//! Picking: the features recognised on parts, and what each operation type makes of them.
//! The bracket (tests/fixtures/picking/bracket.stl, see marcam-fixtures `prism::bracket`):
//! 60 x 40 x 12, top at Z0; through holes 5 mm at (40, 8), (52, 8), (40, 32), (52, 32); a
//! blind hole 6 mm at (46, 20) to -6; a pocket X8..28 Y8..32 to -5 with a round island 6 mm
//! at (18, 20) up to -2; a step X56..60 down to -3.

use marcam::pickables::{accepts, apply, pickables, ApplyContext, Frame, Pickable};
use marcam::Mesh;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bracket() -> Mesh {
    Mesh::load(&root().join("tests/fixtures/picking/bracket.stl")).unwrap()
}

fn of<'a>(p: &'a [Pickable], kind: &str) -> Vec<&'a Pickable> {
    p.iter().filter(|x| x.kind == kind).collect()
}

fn f(p: &Pickable, k: &str) -> f64 {
    p.data[k].as_f64().unwrap()
}

fn cx(tool: f64) -> ApplyContext {
    ApplyContext {
        stock_top: Some(0.0),
        tool_diameter: Some(tool),
        model: "part".into(),
    }
}

#[test]
fn the_brackets_holes() {
    let p = pickables(&bracket(), Frame::Xyz { index_a: 0.0 });
    let holes = of(&p, "hole");
    assert_eq!(
        holes.len(),
        5,
        "{:#?}",
        holes.iter().map(|h| &h.label).collect::<Vec<_>>()
    );
    let through: Vec<_> = holes.iter().filter(|h| h.data["through"] == true).collect();
    assert_eq!(through.len(), 4);
    for h in &through {
        assert!((f(h, "diameter") - 5.0).abs() < 0.01, "{}", h.label);
        assert!((f(h, "top") - 0.0).abs() < 1e-6 && (f(h, "bottom") + 12.0).abs() < 1e-6);
    }
    // all four alike: one "similar" key
    assert!(through.iter().all(|h| h.similar == through[0].similar));
    let blind = holes.iter().find(|h| h.data["through"] == false).unwrap();
    assert!((f(blind, "diameter") - 6.0).abs() < 0.01 && (f(blind, "depth") - 6.0).abs() < 1e-6);
    let c = &blind.data["center"];
    assert!(
        (c[0].as_f64().unwrap() - 46.0).abs() < 1e-6
            && (c[1].as_f64().unwrap() - 20.0).abs() < 1e-6
    );
    assert_eq!(blind.faces.len(), 2, "the wall and its floor");
    assert_ne!(blind.similar, through[0].similar);
}

#[test]
fn the_brackets_pocket_faces_and_loops() {
    let p = pickables(&bracket(), Frame::Xyz { index_a: 0.0 });
    let pockets = of(&p, "pocket");
    assert_eq!(
        pockets.len(),
        1,
        "{:#?}",
        pockets.iter().map(|x| &x.label).collect::<Vec<_>>()
    );
    let pk = pockets[0];
    assert!(
        (f(pk, "floor") + 5.0).abs() < 1e-6
            && (f(pk, "top")).abs() < 1e-6
            && (f(pk, "depth") - 5.0).abs() < 1e-6
    );
    assert_eq!(pk.data["islands"].as_array().unwrap().len(), 1);
    assert!(pk.label.contains("20.00 x 24.00"), "{}", pk.label);
    // flat faces: the top, the step, the pocket floor, the island's top (not the blind hole's
    // floor: that belongs to the hole)
    let mut z: Vec<f64> = of(&p, "flat_face").iter().map(|x| f(x, "z")).collect();
    z.sort_by(|a, b| a.total_cmp(b));
    assert_eq!(z, vec![-5.0, -3.0, -2.0, 0.0]);
    // edge loops: the top's outline and its 6 openings (4 holes, the blind hole, the pocket),
    // the step's, the pocket floor's outline and the island opening, the island top's, and
    // the bottom outline
    let loops = of(&p, "edge_loop");
    let outline = loops
        .iter()
        .find(|l| l.data["up"] == false)
        .expect("the bottom outline");
    assert_eq!(outline.data["side"], "outside");
    assert!(
        (f(outline, "length") - 200.0).abs() < 1e-6,
        "60 x 40 footprint: {}",
        outline.label
    );
    let top_openings = loops
        .iter()
        .filter(|l| f(l, "z") == 0.0 && l.data["side"] == "inside")
        .count();
    assert_eq!(top_openings, 6);
    // every region is covered by some pickable
    assert!(of(&p, "surface").iter().all(|s| s.faces.len() == 1));
}

#[test]
fn drilling_takes_holes_and_fills_points_depth_and_peck() {
    let p = pickables(&bracket(), Frame::Xyz { index_a: 0.0 });
    let holes = of(&p, "hole");
    let through: Vec<&Pickable> = holes
        .iter()
        .copied()
        .filter(|h| h.data["through"] == true)
        .collect();
    let (v, notes) = apply("drill", &through, &cx(5.0)).unwrap();
    assert_eq!(v["points"].as_array().unwrap().len(), 4);
    assert_eq!(v["top"], 0.0);
    assert_eq!(v["depth"], 12.5, "through: 0.5 past the bottom");
    assert_eq!(v["peck"], 5.0);
    assert!(notes.is_empty(), "{notes:?}");
    // a 6 mm hole with a 5 mm tool: use Bore instead
    let blind: Vec<&Pickable> = holes
        .iter()
        .copied()
        .filter(|h| h.data["through"] == false)
        .collect();
    let (v, notes) = apply("drill", &blind, &cx(5.0)).unwrap();
    assert_eq!(v["depth"], 6.0);
    assert!(notes.iter().any(|n| n.contains("Bore")), "{notes:?}");
    // a pocket is not a drilling target
    let err = apply("drill", &of(&p, "pocket"), &cx(5.0)).unwrap_err();
    assert!(err.contains("does not take a pocket"), "{err}");
}

#[test]
fn pocketing_contouring_and_facing() {
    let p = pickables(&bracket(), Frame::Xyz { index_a: 0.0 });
    let (v, _) = apply("pocket", &of(&p, "pocket"), &cx(3.175)).unwrap();
    assert_eq!(
        v["paths"].as_array().unwrap().len(),
        2,
        "outline and island"
    );
    assert_eq!(v["depth"], 5.0);
    assert_eq!(v["top"], 0.0);
    let outline = of(&p, "edge_loop")
        .into_iter()
        .filter(|l| l.data["up"] == false)
        .collect::<Vec<_>>();
    let (v, _) = apply("contour", &outline, &cx(3.175)).unwrap();
    assert_eq!(v["side"], "outside");
    assert_eq!(v["depth"], 12.0);
    let step = of(&p, "flat_face")
        .into_iter()
        .filter(|x| f(x, "z") == -3.0)
        .collect::<Vec<_>>();
    let (v, _) = apply("face", &step, &cx(6.0)).unwrap();
    assert_eq!(v["depth"], 3.0);
    // finishing takes surfaces and faces as face ids
    let (v, _) = apply("raster3d", &of(&p, "pocket"), &cx(3.0)).unwrap();
    assert!(
        v["faces"].as_array().unwrap().len() >= 5,
        "floor and walls: {v:?}"
    );
    assert!(accepts("engrave").is_empty());
}

#[test]
fn indexed_the_part_turned_half_round_shows_its_underside() {
    // turned 180 degrees about X: the bottom is now on top, the holes still go through
    let p = pickables(&bracket(), Frame::Xyz { index_a: 180.0 });
    assert_eq!(
        of(&p, "hole")
            .iter()
            .filter(|h| h.data["through"] == true)
            .count(),
        4
    );
    let top = of(&p, "flat_face")
        .iter()
        .map(|x| f(x, "z"))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (top - 12.0).abs() < 1e-6,
        "the old bottom (Z-12) is now the top at +12: {top}"
    );
    // the same face ids whatever the index
    let p0 = pickables(&bracket(), Frame::Xyz { index_a: 0.0 });
    let ids = |p: &[Pickable]| {
        let mut v: Vec<String> = p.iter().flat_map(|x| x.faces.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(ids(&p), ids(&p0));
}

#[test]
fn rotary_bands_on_the_knob() {
    let knob = Mesh::load(&root().join("examples/knob/knob.stl")).unwrap();
    let p = pickables(&knob, Frame::Rotary);
    let cyl = of(&p, "cylindrical_band");
    assert!(!cyl.is_empty(), "the boss is a cylinder");
    assert!(
        cyl.iter()
            .any(|c| (c.data["r"][1].as_f64().unwrap() - 5.0).abs() < 0.01),
        "{:?}",
        cyl.iter().map(|c| &c.label).collect::<Vec<_>>()
    );
    assert!(
        !of(&p, "revolved_band").is_empty(),
        "the taper, chamfer and shoulders"
    );
    let (v, _) = apply("rotary_finish", &cyl, &cx(3.175)).unwrap();
    assert_eq!(v["faces"].as_array().unwrap().len(), cyl.len());
    let (v, _) = apply("rotary_rough", &cyl[..1], &cx(3.175)).unwrap();
    assert!(v["x_range"].is_array());
    assert!(apply("drill", &cyl, &cx(3.175)).is_err());
}
