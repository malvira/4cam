//! End-to-end: run the `4cam` binary on the bundled examples.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_4cam"));
    c.current_dir(root());
    c
}

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fourcam-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn run_knob_reproduces_goldens_and_writes_previews() {
    let out = out_dir("knob");
    let o = bin()
        .args(["run", "examples/knob/knob.job.json", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout.matches("0 issue(s)").count(), 2, "{stdout}");
    for name in ["knob_1_profile", "knob_2_knurl"] {
        let ours = std::fs::read(out.join(format!("{name}.nc"))).unwrap();
        let golden = std::fs::read_to_string(root().join(format!("tests/golden/{name}.nc")))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(
            ours,
            golden.as_bytes(),
            "{name} differs from the cut G-code"
        );
        let png = std::fs::read(out.join(format!("{name}.png"))).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
    }
    // the written files pass `4cam verify` on their own
    let o = bin()
        .arg("verify")
        .arg(out.join("knob_2_knurl.nc"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn run_on_linuxcnc_and_verify_detects_g93() {
    let out = out_dir("lcnc");
    let o = bin()
        .args([
            "run",
            "examples/knob/knob.job.json",
            "--machine",
            "linuxcnc",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let o = bin()
        .arg("verify")
        .arg(out.join("knob_1_profile.nc"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn verify_flags_a_bad_file() {
    let out = out_dir("bad");
    let f = out.join("bad.nc");
    std::fs::write(&f, "G0 Z15.5\nG0 X0\nG1 Z5 F500\nG0 X-5\n").unwrap();
    let o = bin().arg("verify").arg(&f).output().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(!o.status.success());
    assert!(stdout.contains("2 issue(s)"), "{stdout}");
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn revolve_and_mesh_info() {
    let out = out_dir("mesh");
    let stl = out.join("knob.stl");
    let o = bin()
        .args(["revolve", "examples/knob/knob.job.json"])
        .arg(&stl)
        .args(["--segments", "90"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let o = bin().arg("mesh-info").arg(&stl).output().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("1260 triangles"), "{stdout}");
    assert!(
        stdout.contains("largest radius about X: 10.000"),
        "{stdout}"
    );
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn bad_job_is_reported() {
    let out = out_dir("badjob");
    let f = out.join("job.json");
    let text = std::fs::read_to_string(root().join("examples/knob/knob.job.json"))
        .unwrap()
        .replace("\"tool\": 2", "\"tool\": 5");
    std::fs::write(&f, text).unwrap();
    let o = bin()
        .arg("run")
        .arg(&f)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("T5"));
    std::fs::remove_dir_all(&out).ok();
}
