//! The `marcam-step` command: the marcam command line reading STEP parts.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_marcam-step"));
    c.current_dir(root());
    c
}

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marcam-step-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `marcam-step`: a project whose part is a STEP file is planned, and its features
/// are the STEP's faces (the hole found as a hole, on face "s..").
#[test]
fn a_step_part_is_read_with_its_faces() {
    let out = out_dir("step");
    let o = bin()
        .args(["run", "tests/fixtures/step/box.project.json", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(stdout.contains("0 issue(s)"), "{stdout}");
    assert!(out.join("box.nc").exists());
    let o = bin()
        .args(["pickables", "tests/fixtures/step/box.project.json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(stdout.contains("hole:s"), "{stdout}");
    assert!(stdout.contains("Hole 8.00 mm through"), "{stdout}");
}
