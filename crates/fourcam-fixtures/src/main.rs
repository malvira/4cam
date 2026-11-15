//! Regenerates the part files the examples and tests use. Part generation is a test-fixture
//! concern, not an engine feature: the engine only plans against parts it is given.
//!
//!     cargo run -p fourcam-fixtures            (writes examples/knob/knob.stl)
//!     cargo run -p fourcam-fixtures -- --check (fails if the committed file differs)

use fourcam::{Job, Mesh};
use std::path::PathBuf;

/// The knob's revolved profile as a closed binary STL (720 facets around).
pub fn knob_stl(root: &std::path::Path) -> Vec<u8> {
    let text = std::fs::read_to_string(root.join("examples/knob/knob.job.json")).expect("knob job");
    let job = Job::from_json(&text).expect("knob job parses");
    Mesh::revolve(job.part.profile.as_ref().expect("profile"), 720).to_stl()
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("examples/knob/knob.stl");
    let bytes = knob_stl(&root);
    if std::env::args().any(|a| a == "--check") {
        let cur = std::fs::read(&out).unwrap_or_default();
        if cur != bytes {
            eprintln!(
                "{} is out of date: run `cargo run -p fourcam-fixtures`",
                out.display()
            );
            std::process::exit(1);
        }
        println!("{} is up to date", out.display());
    } else {
        std::fs::write(&out, bytes).expect("write knob.stl");
        println!("wrote {}", out.display());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn committed_knob_stl_matches_the_generator() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cur = std::fs::read(root.join("examples/knob/knob.stl")).unwrap();
        assert!(
            cur == super::knob_stl(&root),
            "examples/knob/knob.stl is out of date"
        );
    }
}
