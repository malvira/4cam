//! The C ABI from Rust: the committed header matches the code, and every function answers
//! with JSON (results, errors for bad input, NULLs tolerated) and frees cleanly.

use marcam_c::*;
use serde_json::Value;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Take ownership of a returned string: parse it and free it.
fn take(p: *mut c_char) -> Value {
    assert!(!p.is_null());
    // SAFETY: returned by the library, valid until freed below.
    let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
    // SAFETY: freed once.
    unsafe { marcam_free(p) };
    serde_json::from_str(&s).unwrap()
}

#[test]
fn header_is_up_to_date() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(dir.join("cbindgen.toml")).unwrap();
    let mut out = Vec::new();
    cbindgen::Builder::new()
        .with_crate(&dir)
        .with_config(config)
        .generate()
        .unwrap()
        .write(&mut out);
    let want = String::from_utf8(out).unwrap().replace("\r\n", "\n");
    let have = std::fs::read_to_string(dir.join("include/marcam.h"))
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if have != want {
        if std::env::var_os("MARCAM_BLESS").is_some() {
            std::fs::write(dir.join("include/marcam.h"), &want).unwrap();
        } else {
            panic!("include/marcam.h is out of date: run with MARCAM_BLESS=1 to regenerate");
        }
    }
    for f in [
        "marcam_free",
        "marcam_version",
        "marcam_abi_version",
        "marcam_plan_and_post",
        "marcam_simulate",
        "marcam_estimate_gcode",
        "marcam_op_types",
        "marcam_op_defaults",
        "marcam_check_project",
        "marcam_call",
    ] {
        assert!(
            want.contains(&format!("{f}(")),
            "{f} missing from the header"
        );
    }
}

#[test]
fn version_and_registry() {
    let v = take(marcam_version());
    assert_eq!(v["name"], "marcam");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(marcam_abi_version(), 1);
    let types = take(marcam_op_types());
    assert!(types
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["kind"] == "rotary_finish"));
}

#[test]
fn plan_post_estimate_simulate_the_knob() {
    let project = std::fs::read_to_string(root().join("examples/knob/knob.project.json")).unwrap();
    let p = CString::new(project.clone()).unwrap();
    // SAFETY: valid strings, no mesh.
    let r = take(unsafe {
        marcam_plan_and_post(
            p.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    });
    assert!(r.get("error").is_none(), "{r}");
    let progs = r["programs"].as_array().unwrap();
    // the same G-code as the CLI's golden files
    let golden = std::fs::read_to_string(root().join("tests/golden/knob_1_profile.nc")).unwrap();
    assert_eq!(
        progs[0]["gcode"].as_str().unwrap().replace("\r\n", "\n"),
        golden.replace("\r\n", "\n")
    );
    let g = CString::new(progs[0]["gcode"].as_str().unwrap()).unwrap();
    // SAFETY: valid strings.
    let e = take(unsafe { marcam_estimate_gcode(g.as_ptr(), std::ptr::null()) });
    assert!(e["total_s"].as_f64().unwrap() > 60.0, "{e}");
    let o = CString::new(r#"{"res": "coarse"}"#).unwrap();
    // SAFETY: valid strings, no mesh.
    let s = take(unsafe { marcam_simulate(p.as_ptr(), o.as_ptr(), std::ptr::null(), 0) });
    assert_eq!(
        s["stocks"].as_array().unwrap().len(),
        2,
        "{}",
        s.get("error").unwrap_or(&Value::Null)
    );
    // defaults and checks
    let req = CString::new(r#"{"type": "rotary_rough", "setup": "setup1"}"#).unwrap();
    // SAFETY: valid strings, no mesh.
    let d = take(unsafe { marcam_op_defaults(p.as_ptr(), req.as_ptr(), std::ptr::null(), 0) });
    assert_eq!(d["type"], "rotary_rough", "{d}");
    assert!(d["step_down"].as_f64().unwrap() > 0.0);
    // SAFETY: valid string.
    let c = take(unsafe { marcam_check_project(p.as_ptr()) });
    assert_eq!(c["upgraded"], false);
    // the generic entry point
    let name = CString::new("profile_list").unwrap();
    // SAFETY: valid name, no arguments.
    let l = take(unsafe {
        marcam_call(
            name.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
        )
    });
    assert!(l["machines"].as_array().unwrap().len() >= 4, "{l}");
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    let junk = CString::new("not json").unwrap();
    // SAFETY: valid strings / NULLs.
    unsafe {
        assert!(take(marcam_plan_and_post(
            junk.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null()
        ))["error"]
            .is_string());
        assert!(take(marcam_plan_and_post(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null()
        ))["error"]
            .is_string());
        assert!(take(marcam_simulate(
            junk.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0
        ))["error"]
            .is_string());
        // picking: the bracket's holes, and a drill filled from them
        let fx = root().join("tests/fixtures/picking");
        let project =
            CString::new(std::fs::read_to_string(fx.join("bracket.project.json")).unwrap())
                .unwrap();
        let stl = std::fs::read(fx.join("bracket.stl")).unwrap();
        let empty = CString::new("{}").unwrap();
        let pk = take(marcam_pickables(
            project.as_ptr(),
            empty.as_ptr(),
            stl.as_ptr(),
            stl.len(),
        ));
        let ids: Vec<String> = pk["pickables"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["kind"] == "hole" && x["data"]["through"] == true)
            .map(|x| x["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids.len(), 4);
        let req =
            CString::new(serde_json::json!({"type": "drill", "ids": ids, "tool": 1}).to_string())
                .unwrap();
        let filled = take(marcam_apply_picks(
            project.as_ptr(),
            req.as_ptr(),
            stl.as_ptr(),
            stl.len(),
        ));
        assert_eq!(filled["params"]["depth"], 12.5, "{filled}");
        let unknown = CString::new("no_such_function").unwrap();
        assert!(take(marcam_call(
            unknown.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0
        ))["error"]
            .is_string());
        marcam_free(std::ptr::null_mut());
    }
}
