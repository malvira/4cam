//! Several part files given together (each setup its own part, Mar): a bundle, each model
//! taking its own file; one plain file as before.

use marcam::job::MeshSource;
use marcam::mesh::{bundle_file, BUNDLE_MAGIC};
use marcam::Mesh;
use std::path::PathBuf;

fn read(p: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(p),
    )
    .unwrap()
}

fn bundle(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = BUNDLE_MAGIC.to_vec();
    for (n, b) in files {
        out.extend((n.len() as u32).to_le_bytes());
        out.extend(n.as_bytes());
        out.extend((b.len() as u32).to_le_bytes());
        out.extend(*b);
    }
    out
}

fn source(file: &str) -> MeshSource {
    serde_json::from_value(serde_json::json!({ "file": file })).unwrap()
}

#[test]
fn each_model_takes_its_own_file_from_a_bundle() {
    let dome = read("examples/plate/dome.stl");
    let knob = read("examples/knob/knob.stl");
    let b = bundle(&[("dome.stl", &dome), ("parts/knob.stl", &knob)]);
    let want_dome = Mesh::from_bytes(&dome).unwrap();
    let want_knob = Mesh::from_bytes(&knob).unwrap();
    assert_eq!(
        source("dome.stl").mesh(&b).unwrap().triangles,
        want_dome.triangles
    );
    // (by its name, or its name without folders)
    assert_eq!(
        source("knob.stl").mesh(&b).unwrap().triangles,
        want_knob.triangles
    );
    // one plain file: as before, whatever the model names
    assert_eq!(
        source("other.stl").mesh(&dome).unwrap().triangles,
        want_dome.triangles
    );
    // a file not given: said so
    let e = source("missing.stl").mesh(&b).unwrap_err().to_string();
    assert!(e.contains("missing.stl") && e.contains("dome.stl"), "{e}");
    // nothing named: the first
    assert_eq!(Mesh::from_bytes(&b).unwrap().triangles, want_dome.triangles);
    assert_eq!(bundle_file(&dome, "x").unwrap().len(), dome.len());
    // cut short: refused
    assert!(Mesh::from_bytes(&b[..b.len() - 10]).is_err());
}
