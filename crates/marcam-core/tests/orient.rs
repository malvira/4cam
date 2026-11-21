//! A part's orientation in its setup (Mar): `rotate`, degrees about X, then Y, then Z after
//! the axis mapping. Quarter turns are exact; none leaves the part as it was.

use marcam::job::Turn;
use marcam::mesh::Axis;
use marcam::Mesh;
use std::path::PathBuf;

fn dome() -> Mesh {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plate/dome.stl");
    Mesh::from_bytes(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
fn quarter_turns_are_exact_and_none_changes_nothing() {
    let m = dome();
    let none = m
        .clone()
        .oriented_rotated(Axis::Z, [0.0; 3], Turn::None, [1.0, 2.0, 3.0]);
    assert_eq!(
        none.triangles,
        m.clone()
            .oriented_turned(Axis::Z, Turn::None, [1.0, 2.0, 3.0])
            .triangles
    );
    let base = m.clone().oriented(Axis::Z, [0.0; 3]);
    let (lo, hi) = base.bounds().unwrap();
    // a quarter turn about Z: X becomes Y, Y becomes -X, exactly
    let q = m
        .clone()
        .oriented_rotated(Axis::Z, [0.0, 0.0, 90.0], Turn::None, [0.0; 3]);
    for (a, b) in base.triangles.iter().zip(&q.triangles) {
        for (p, r) in a.iter().zip(b) {
            assert_eq!(*r, [-p[1], p[0], p[2]]);
        }
    }
    let (qlo, qhi) = q.bounds().unwrap();
    assert_eq!(
        (qlo[0], qhi[0], qlo[1], qhi[1]),
        (-hi[1], -lo[1], lo[0], hi[0])
    );
    // two quarter turns about X: the same as turned over about X
    let x2 = m
        .clone()
        .oriented_rotated(Axis::Z, [180.0, 0.0, 0.0], Turn::None, [0.0; 3]);
    let t = m.clone().oriented_turned(Axis::Z, Turn::X180, [0.0; 3]);
    assert_eq!(x2.triangles, t.triangles);
    // any angle: about Z by 30 degrees, the part's size round Z unchanged
    let a = m.oriented_rotated(Axis::Z, [0.0, 0.0, 30.0], Turn::None, [0.0; 3]);
    let r0 = base
        .triangles
        .iter()
        .flatten()
        .map(|p| p[0].hypot(p[1]))
        .fold(0.0, f64::max);
    let r1 = a
        .triangles
        .iter()
        .flatten()
        .map(|p| p[0].hypot(p[1]))
        .fold(0.0, f64::max);
    assert!((r0 - r1).abs() < 1e-9);
}
