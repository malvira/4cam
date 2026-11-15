//! WebAssembly build of 4cam. No wasm-bindgen: the module exports `fc_alloc`, `fc_free` and
//! `fc_call`, and `js/fourcam.js` wraps them. Every call takes up to three byte strings (UTF-8
//! JSON or raw mesh bytes) and returns a JSON document.
//!
//! The [`api`] functions are plain Rust so they are tested natively.

pub mod api {
    use fourcam::job::Job;
    use fourcam::{
        gouge_check, limits_for, plan_with_mesh, post, render, sim, simulate_job, verify_gcode,
        Machine, Mesh, Toolpath,
    };
    use serde_json::{json, Value};

    pub const EXAMPLE_JOB: &str = include_str!("../../../examples/knob/knob.job.json");

    fn err(msg: impl std::fmt::Display) -> Value {
        json!({ "error": msg.to_string() })
    }

    pub fn version() -> Value {
        json!({ "name": "4cam", "version": env!("CARGO_PKG_VERSION"), "schema_version": fourcam::job::SCHEMA_VERSION })
    }

    /// Built-in machine by name, or the default (Makera Z1) for an empty name.
    pub fn machine(name: &str) -> Value {
        let name = if name.is_empty() { "makera-z1" } else { name };
        match Machine::builtin(name) {
            Some(m) => serde_json::to_value(m).unwrap_or(Value::Null),
            None => err(format!(
                "unknown machine {name:?} (built in: makera-z1, linuxcnc)"
            )),
        }
    }

    fn parse_machine(text: &str) -> Result<Machine, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(Machine::makera_z1());
        }
        if let Some(m) = Machine::builtin(t) {
            return Ok(m);
        }
        Machine::from_json(t).map_err(|e| format!("machine: {e}"))
    }

    /// Mesh bytes (STL or 3MF), placed on the axis as the job's `part.mesh` says.
    fn parse_mesh(job: &Job, bytes: &[u8]) -> Result<Option<Mesh>, String> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let m = Mesh::from_bytes(bytes).map_err(|e| e.to_string())?;
        Ok(Some(match &job.part.mesh {
            Some(src) => m.oriented(src.axis, src.translate),
            None => m,
        }))
    }

    /// Tool-tip polyline, flattened `[x, r, a, kind, ...]` (kind 0 rapid, 1 feed), thinned
    /// to about `max_points` points (rapids and direction changes between rapid and feed are
    /// always kept).
    fn polyline(tp: &Toolpath, max_points: usize) -> (Vec<f32>, usize, bool) {
        let pts = render::polyline(tp);
        let total = pts.len();
        let stride = if max_points > 0 && total > max_points {
            total.div_ceil(max_points)
        } else {
            1
        };
        let mut out = Vec::with_capacity(4 * (total / stride + 16));
        for (i, p) in pts.iter().enumerate() {
            let keep = stride == 1
                || i % stride == 0
                || i + 1 == total
                || !p.3
                || pts.get(i + 1).is_some_and(|q| !q.3);
            if keep {
                out.extend_from_slice(&[
                    p.0 as f32,
                    p.1 as f32,
                    p.2 as f32,
                    f32::from(u8::from(p.3)),
                ]);
            }
        }
        (out, total, stride > 1)
    }

    /// Plan, post, verify and gouge-check a job. `opts`: `{"max_points": n}` (default 200000).
    pub fn plan_and_post(job_json: &str, machine_text: &str, mesh: &[u8], opts: &str) -> Value {
        let job = match Job::from_json(job_json) {
            Ok(j) => j,
            Err(e) => return err(format!("job: {e}")),
        };
        let machine = match parse_machine(machine_text) {
            Ok(m) => m,
            Err(e) => return err(e),
        };
        let mesh = match parse_mesh(&job, mesh) {
            Ok(m) => m,
            Err(e) => return err(e),
        };
        if job.part.mesh.is_some() && mesh.is_none() {
            return err(format!(
                "this job uses the mesh file {:?}: load it too",
                job.part
                    .mesh
                    .as_ref()
                    .map(|m| m.file.as_str())
                    .unwrap_or("")
            ));
        }
        let opts: Value = serde_json::from_str(if opts.trim().is_empty() { "{}" } else { opts })
            .unwrap_or(Value::Null);
        let max_points = opts["max_points"].as_u64().unwrap_or(200_000) as usize;
        let tps = match plan_with_mesh(&job, mesh.as_ref()) {
            Ok(t) => t,
            Err(e) => return err(e),
        };
        let gouges = gouge_check(&job, &tps, mesh.as_ref()).unwrap_or_default();
        let programs: Vec<Value> = tps
            .iter()
            .map(|tp| {
                let gcode = post(tp, &machine);
                let issues: Vec<String> = verify_gcode(&gcode, &limits_for(tp, &machine))
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                let g = gouges.iter().find(|g| g.program == tp.name);
                let (points, total, thinned) = polyline(tp, max_points);
                json!({
                    "name": tp.name,
                    "title": tp.title,
                    "tool": tp.tool,
                    "lines": gcode.lines().count(),
                    "gcode": gcode,
                    "issues": issues,
                    "gouge": g.map(|g| json!({"moves_checked": g.moves_checked, "max_depth": g.max_depth, "at": g.at})),
                    "safe_r": tp.safe_r,
                    "toolpath": { "points": points, "total": total, "thinned": thinned },
                })
            })
            .collect();
        json!({
            "machine": machine.name,
            "stock": job.stock,
            "profile": job.part.profile,
            "mesh_bounds": mesh.as_ref().and_then(|m| m.bounds()),
            "programs": programs,
        })
    }

    /// Simulate all programs in order. `opts`: `{"dx": mm, "na": cells}` (default 0.1, 360).
    /// Returns the grid, the part map and the stock map after each program (radii, f32).
    pub fn simulate(job_json: &str, opts: &str, mesh: &[u8]) -> Value {
        let job = match Job::from_json(job_json) {
            Ok(j) => j,
            Err(e) => return err(format!("job: {e}")),
        };
        let mesh = match parse_mesh(&job, mesh) {
            Ok(m) => m,
            Err(e) => return err(e),
        };
        let opts: Value = serde_json::from_str(if opts.trim().is_empty() { "{}" } else { opts })
            .unwrap_or(Value::Null);
        let dx = opts["dx"].as_f64().unwrap_or(0.1).max(0.01);
        let na = (opts["na"].as_u64().unwrap_or(360) as usize).clamp(8, 4096);
        let tps = match plan_with_mesh(&job, mesh.as_ref()) {
            Ok(t) => t,
            Err(e) => return err(e),
        };
        let res = match simulate_job(&job, &tps, mesh.as_ref(), dx, na, 0.02) {
            Ok(r) => r,
            Err(e) => return err(e),
        };
        // replay to capture the stock after each program
        let mut stock = sim::RadialMap::filled(res.stock.grid, job.stock.radius);
        let cutters = |t: u32| {
            job.tool(t)
                .and_then(|d| fourcam::cutter_for(t, d.shape).ok())
        };
        let mut stocks = Vec::new();
        for (tp, (name, st, dev)) in tps.iter().zip(&res.programs) {
            sim::simulate(&mut stock, tp, &cutters, (dx * 2.0).min(0.1));
            stocks.push(json!({
                "name": name,
                "samples": st.samples,
                "cells_cut": st.cells_cut,
                "rapid_hits": st.rapid_hits.len(),
                "max_left": dev.max_left,
                "mean_left": dev.mean_left,
                "max_gouge": dev.max_gouge,
                "gouge_cells": dev.gouge_cells,
                "r": stock.r,
            }));
        }
        let g = res.stock.grid;
        json!({
            "grid": { "x0": g.x0, "dx": g.dx, "nx": g.nx, "na": g.na },
            "stock_radius": job.stock.radius,
            "part": res.part.center.r,
            "stocks": stocks,
        })
    }

    /// Oriented mesh triangles (flattened xyz, f32) for display, thinned to about
    /// `max_triangles` (`opts`, default 200000).
    pub fn mesh_triangles(job_json: &str, opts: &str, mesh: &[u8]) -> Value {
        let job = match Job::from_json(job_json) {
            Ok(j) => j,
            Err(e) => return err(format!("job: {e}")),
        };
        let m = match parse_mesh(&job, mesh) {
            Ok(Some(m)) => m,
            Ok(None) => return err("no mesh given"),
            Err(e) => return err(e),
        };
        let opts: Value = serde_json::from_str(if opts.trim().is_empty() { "{}" } else { opts })
            .unwrap_or(Value::Null);
        let max = opts["max_triangles"].as_u64().unwrap_or(200_000).max(1) as usize;
        let stride = m.triangles.len().div_ceil(max).max(1);
        let tri: Vec<f32> = m
            .triangles
            .iter()
            .step_by(stride)
            .flat_map(|t| t.iter().flat_map(|p| p.iter().map(|&c| c as f32)))
            .collect();
        json!({ "triangles": tri, "total": m.triangles.len(), "thinned": stride > 1, "bounds": m.bounds() })
    }

    /// Dispatch by name (what `fc_call` uses).
    pub fn call(name: &str, a: &[u8], b: &[u8], c: &[u8]) -> Value {
        let s = |v: &[u8]| String::from_utf8_lossy(v).into_owned();
        match name {
            "version" => version(),
            "example_job" => serde_json::from_str(EXAMPLE_JOB).unwrap_or(Value::Null),
            "machine" => machine(&s(a)),
            "plan_and_post" => plan_and_post(&s(a), &s(b), c, ""),
            "plan_and_post_opts" => {
                // a = job, b = {"machine": ..., "max_points": ...}, c = mesh
                let o: Value = serde_json::from_slice(b).unwrap_or(Value::Null);
                let m = match &o["machine"] {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                plan_and_post(&s(a), &m, c, &s(b))
            }
            "simulate" => simulate(&s(a), &s(b), c),
            "mesh_triangles" => mesh_triangles(&s(a), &s(b), c),
            other => err(format!("unknown function {other:?}")),
        }
    }
}

/// Allocate `len` bytes for the host to write into.
#[no_mangle]
pub extern "C" fn fc_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Free memory from [`fc_alloc`] or a result of [`fc_call`] (`len` = its full length).
///
/// # Safety
/// `ptr` and `len` must come from `fc_alloc(len)` or be a result pointer with its length.
#[no_mangle]
pub unsafe extern "C" fn fc_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
    }
}

unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Call `name` with three byte-string arguments. Returns a pointer to a 4-byte little-endian
/// length followed by that many bytes of JSON; free it with `fc_free(ptr, 4 + length)`.
///
/// # Safety
/// Each pointer/length pair must describe readable memory (or be null/0).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn fc_call(
    name_ptr: *const u8,
    name_len: usize,
    a_ptr: *const u8,
    a_len: usize,
    b_ptr: *const u8,
    b_len: usize,
    c_ptr: *const u8,
    c_len: usize,
) -> *mut u8 {
    let name = String::from_utf8_lossy(slice(name_ptr, name_len)).into_owned();
    let out = api::call(
        &name,
        slice(a_ptr, a_len),
        slice(b_ptr, b_len),
        slice(c_ptr, c_len),
    )
    .to_string()
    .into_bytes();
    let mut buf = Vec::with_capacity(4 + out.len());
    buf.extend_from_slice(&(out.len() as u32).to_le_bytes());
    buf.extend_from_slice(&out);
    let mut buf = buf.into_boxed_slice();
    let p = buf.as_mut_ptr();
    std::mem::forget(buf);
    p
}

#[cfg(test)]
mod tests {
    use super::api;

    #[test]
    fn example_runs_through_the_json_api() {
        let v = api::plan_and_post(api::EXAMPLE_JOB, "", &[], r#"{"max_points": 5000}"#);
        assert!(v.get("error").is_none(), "{v}");
        let progs = v["programs"].as_array().unwrap();
        assert_eq!(progs.len(), 2);
        assert_eq!(progs[0]["lines"], 21787);
        assert_eq!(progs[1]["lines"], 15146);
        for p in progs {
            assert_eq!(p["issues"].as_array().unwrap().len(), 0);
            let pts = p["toolpath"]["points"].as_array().unwrap();
            assert!(pts.len() % 4 == 0 && pts.len() / 4 < 9000, "{}", pts.len());
            assert_eq!(p["toolpath"]["thinned"], true);
        }
        assert!(progs[0]["gouge"]["max_depth"].as_f64().unwrap() < 0.01);
        assert!(progs[0]["gcode"]
            .as_str()
            .unwrap()
            .starts_with("(Knob radial profile"));
    }

    #[test]
    fn machines_errors_and_dispatch() {
        assert_eq!(api::machine("linuxcnc")["feed_mode"], "inverse_time");
        assert!(api::machine("nope").get("error").is_some());
        let v = api::plan_and_post(api::EXAMPLE_JOB, "linuxcnc", &[], "");
        assert!(v["programs"][0]["gcode"].as_str().unwrap().contains("G93"));
        assert!(api::plan_and_post("{", "", &[], "").get("error").is_some());
        let bad = api::EXAMPLE_JOB.replace("\"tool\": 1", "\"tool\": 7");
        let v = api::plan_and_post(&bad, "", &[], "");
        assert!(v["error"].as_str().unwrap().contains("T7"));
        assert!(api::call("nope", &[], &[], &[]).get("error").is_some());
        assert_eq!(api::call("version", &[], &[], &[])["name"], "4cam");
    }

    #[test]
    fn mesh_job_needs_mesh_and_simulates() {
        let job = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/knob/knob_finish.job.json"
        ))
        .unwrap();
        let stl = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/knob/knob.stl"
        ))
        .unwrap();
        assert!(api::plan_and_post(&job, "", &[], "").get("error").is_some());
        let tri = api::mesh_triangles(&job, r#"{"max_triangles": 1000}"#, &stl);
        assert_eq!(tri["total"], 10080);
        assert!(tri["triangles"].as_array().unwrap().len() <= 1000 * 9);
        // profile-only job simulates fine at a coarse grid
        let s = api::simulate(api::EXAMPLE_JOB, r#"{"dx": 0.2, "na": 90}"#, &[]);
        assert!(s.get("error").is_none(), "{}", s["error"]);
        let st = s["stocks"].as_array().unwrap();
        assert_eq!(st.len(), 2);
        assert_eq!(st[0]["rapid_hits"], 0);
        assert_eq!(st[0]["gouge_cells"], 0);
        let n = s["grid"]["nx"].as_u64().unwrap() * s["grid"]["na"].as_u64().unwrap();
        assert_eq!(st[0]["r"].as_array().unwrap().len() as u64, n);
    }
}
