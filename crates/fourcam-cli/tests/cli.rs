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

fn profiles_env(name: &str) -> (PathBuf, Command) {
    let dir = out_dir(name);
    let mut c = bin();
    c.env("FOURCAM_PROFILES", &dir);
    (dir, c)
}

#[test]
fn profiles_list_import_show_export() {
    let (dir, mut c) = profiles_env("profiles");
    let o = c.args(["profiles", "list"]).output().unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    for id in [
        "makera-z1-4axis",
        "makera-carvera-4axis",
        "makera-carvera-air-4axis",
        "linuxcnc-4axis",
        "makera-starter",
    ] {
        assert!(s.contains(id), "{s}");
    }
    // a Fusion 360 tool library (synthetic file in Fusion's format)
    let (_, mut c) = profiles_env("profiles");
    c.env("FOURCAM_PROFILES", &dir);
    let o = c
        .args([
            "profiles",
            "import",
            "tests/fixtures/fusion-tool-library.json",
            "--id",
            "fixture-tools",
        ])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{s}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        s.contains("imported tool library fixture-tools (3 tools)"),
        "{s}"
    );
    assert!(dir.join("fixture-tools.tools.json").exists());
    let mut c = bin();
    c.env("FOURCAM_PROFILES", &dir);
    let s = String::from_utf8_lossy(
        &c.args(["profiles", "show", "fixture-tools"])
            .output()
            .unwrap()
            .stdout,
    )
    .to_string();
    assert!(s.contains("\"angle\": 90.0"), "{s}");
    let mut c = bin();
    c.env("FOURCAM_PROFILES", &dir);
    let exported = dir.join("z1.json");
    let o = c
        .args(["profiles", "export", "makera-z1"])
        .arg("--out")
        .arg(&exported)
        .output()
        .unwrap();
    assert!(o.status.success());
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(fourcam::profiles::MachineProfile::from_json(&text).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_checks_machine_limits_and_uses_library_tools() {
    let (dir, mut c) = profiles_env("limits");
    // knob job on the Z1: the cut G-code asks for F5209 against a 4000 mm/min cap (warning)
    let o = c
        .args(["run", "examples/knob/knob.job.json", "--out"])
        .arg(&dir)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("machine: makera-z1-4axis"), "{s}");
    assert!(s.contains("feeds reach F5209"), "{s}");
    // tools from the built-in library, machine named in the job, a spindle the Air can't do
    let job = std::fs::read_to_string(root().join("examples/knob/knob.job.json")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&job).unwrap();
    v["machine"] = "makera-carvera-air-4axis".into();
    v["spindle_rpm"] = 15000.into();
    v["tools"] = serde_json::json!([
        {"number": 1, "library": "makera-starter", "tool": "spiral-o-metal-3.175x12"},
        {"number": 2, "library": "makera-starter", "tool": "vbit-90-3.175"}
    ]);
    let f = dir.join("air.job.json");
    std::fs::write(&f, v.to_string()).unwrap();
    let mut c = bin();
    c.env("FOURCAM_PROFILES", &dir);
    let o = c
        .arg("run")
        .arg(&f)
        .arg("--out")
        .arg(&dir)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(!o.status.success(), "15000 rpm must fail on the Air: {s}");
    assert!(s.contains("machine: makera-carvera-air-4axis"), "{s}");
    assert!(
        s.contains("error: spindle 15000 rpm is outside 0..13000 rpm"),
        "{s}"
    );
    let nc = std::fs::read_to_string(dir.join("knob_1_profile.nc")).unwrap();
    assert!(
        nc.contains("(T1 Spiral O Metal 3.175 x 12 mm (single flute))"),
        "{}",
        &nc[..300]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ops_registry_and_upgrade() {
    let o = bin().args(["ops", "list"]).output().unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    for k in [
        "rotary_turning",
        "rotary_rough",
        "rotary_finish",
        "helical_knurl",
        "flank_chamfer",
    ] {
        assert!(s.contains(k), "{s}");
    }
    let o = bin()
        .args(["ops", "show", "helical_knurl"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["tool_shapes"], serde_json::json!(["vbit"]));
    let o = bin()
        .args(["ops", "check", "examples/knob/knob.project.json"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    assert!(String::from_utf8_lossy(&o.stdout).contains("3 operation(s) checked, 0 problem(s)"));
    let dir = out_dir("upgrade");
    let f = dir.join("p.json");
    let o = bin()
        .args(["upgrade", "examples/knob/knob.job.json", "--out"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(o.status.success());
    let p: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    assert_eq!(p["schema_version"], "0.2");
    assert_eq!(
        p["setups"][0]["outputs"][1]["operations"],
        serde_json::json!(["op2", "op3"])
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn two_setup_project_runs_every_setup() {
    let out = out_dir("setups");
    let o = bin()
        .args(["run", "examples/knob/knob_two_setups.project.json", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{s}");
    assert!(
        s.contains("== setup setup1") && s.contains("== setup setup2"),
        "{s}"
    );
    for name in ["knob_1_profile", "knob_2_knurl"] {
        let ours = std::fs::read(out.join(format!("{name}.nc"))).unwrap();
        let golden = std::fs::read_to_string(root().join(format!("tests/golden/{name}.nc")))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(ours, golden.as_bytes(), "{name}");
    }
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn faces_list_and_face_selection_in_a_project() {
    let o = bin()
        .args([
            "faces",
            "examples/knob/knob_finish.project.json",
            "--model",
            "mesh",
        ])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(
        o.status.success() && s.contains("model mesh (8 faces)"),
        "{s}"
    );
    // pick the taper (the long cone) and finish only it
    let taper = s
        .lines()
        .find(|l| l.contains(" cone ") && l.contains("-15.000..-0.800"))
        .and_then(|l| l.split_whitespace().next())
        .unwrap()
        .to_string();
    let text =
        std::fs::read_to_string(root().join("examples/knob/knob_finish.project.json")).unwrap();
    let mut p: serde_json::Value = serde_json::from_str(&text).unwrap();
    let ops = p["setups"][0]["operations"].as_array_mut().unwrap();
    ops[1]["geometry"] = serde_json::json!([{"kind": "faces", "model": "mesh", "ids": [taper]}]);
    let dir = out_dir("faces");
    std::fs::copy(root().join("examples/knob/knob.stl"), dir.join("knob.stl")).unwrap();
    let f = dir.join("p.json");
    std::fs::write(&f, p.to_string()).unwrap();
    let o = bin()
        .arg("run")
        .arg(&f)
        .arg("--out")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let nc = std::fs::read_to_string(dir.join("knob_4_finish_along.nc")).unwrap();
    let xs: Vec<f64> = nc
        .lines()
        .filter(|l| l.starts_with("G1 X"))
        .filter_map(|l| l.split_whitespace().nth(1)?.strip_prefix('X')?.parse().ok())
        .collect();
    let lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    assert!(
        lo >= -16.588,
        "the finish should stay on the taper (+ tool radius): min X {lo}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn knurl_wizard_rebuilds_the_knob() {
    // strip the operations from the knob project, let the wizard add them back, run it
    let text = std::fs::read_to_string(root().join("examples/knob/knob.project.json")).unwrap();
    let mut p: serde_json::Value = serde_json::from_str(&text).unwrap();
    p["setups"][0]["operations"] = serde_json::json!([]);
    p["setups"][0]["outputs"] = serde_json::json!([]);
    let dir = out_dir("wizard");
    let empty = dir.join("empty.json");
    std::fs::write(&empty, p.to_string()).unwrap();
    let full = dir.join("full.json");
    let o = bin()
        .args(["wizard", "knurl"])
        .arg(&empty)
        .arg("--out")
        .arg(&full)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("added operations op1, op2, op3"));
    let o = bin()
        .arg("run")
        .arg(&full)
        .arg("--out")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    for name in ["knob_1_profile", "knob_2_knurl"] {
        let ours = std::fs::read(dir.join(format!("{name}.nc"))).unwrap();
        let golden = std::fs::read_to_string(root().join(format!("tests/golden/{name}.nc")))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(ours, golden.as_bytes(), "{name}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
