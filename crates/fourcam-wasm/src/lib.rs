//! WebAssembly build of 4cam. No wasm-bindgen: the module exports `fc_alloc`, `fc_free` and
//! `fc_call`, and `js/fourcam.js` wraps them. Every call takes up to three byte strings (UTF-8
//! JSON or raw mesh bytes) and returns a JSON document.
//!
//! The [`api`] functions are plain Rust so they are tested natively.

pub mod api {
    use fourcam::job::Job;
    use fourcam::profiles::{self, MachineProfile, ToolLibrary};
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

    /// Machine from a built-in id/alias, a 4cam machine profile, an older 4cam machine file,
    /// or a Fusion 360 machine definition (JSON). Empty: the job's `machine`, else the Z1.
    fn parse_machine(text: &str, job: Option<&Job>) -> Result<MachineProfile, String> {
        let t = text.trim();
        let t = if t.is_empty() {
            job.and_then(|j| j.machine.as_deref())
                .unwrap_or("makera-z1-4axis")
        } else {
            t
        };
        if let Some(m) = profiles::builtin_machine(t) {
            return Ok(m);
        }
        if !t.starts_with('{') {
            return Err(format!("unknown machine {t:?}"));
        }
        match profiles::import(t.as_bytes(), "machine.json")?.item {
            profiles::Imported::Machine(m) => Ok(m),
            profiles::Imported::Tools(_) => Err("that is a tool library, not a machine".into()),
        }
    }

    /// The job to plan from a 0.1 job or a 0.2 project (its first setup).
    fn job_any(text: &str, libs: &[ToolLibrary]) -> Result<Job, String> {
        let mut jobs = fourcam::project::jobs_from_any(text, libs)?;
        if jobs.is_empty() {
            return Err("the project has no setups".into());
        }
        Ok(jobs.swap_remove(0).1)
    }

    /// Built-in libraries plus `opts.libraries` (4cam tool library objects).
    fn libraries(opts: &Value) -> Result<Vec<ToolLibrary>, String> {
        let mut libs = profiles::builtin_libraries();
        for l in opts["libraries"].as_array().into_iter().flatten() {
            let lib = ToolLibrary::from_json(&l.to_string())?;
            libs.retain(|x| x.id != lib.id);
            libs.push(lib);
        }
        Ok(libs)
    }

    /// Built-in machine profiles and tool libraries.
    pub fn profile_list() -> Value {
        json!({
            "machines": profiles::builtin_machines(),
            "libraries": profiles::builtin_libraries(),
        })
    }

    /// Convert a profile file (4cam JSON, Fusion .tools/.json/.mch/.f3d, older machine file)
    /// into 4cam JSON: `{kind: "machine"|"tools", profile, notes}`.
    pub fn import_profile(bytes: &[u8], name: &str) -> Value {
        match profiles::import(bytes, name) {
            Ok(imp) => match imp.item {
                profiles::Imported::Machine(m) => {
                    json!({"kind": "machine", "profile": m, "notes": imp.notes})
                }
                profiles::Imported::Tools(l) => {
                    json!({"kind": "tools", "profile": l, "notes": imp.notes})
                }
            },
            Err(e) => err(e),
        }
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
        let opts_v: Value = serde_json::from_str(if opts.trim().is_empty() { "{}" } else { opts })
            .unwrap_or(Value::Null);
        let libs = match libraries(&opts_v) {
            Ok(l) => l,
            Err(e) => return err(e),
        };
        let job = match job_any(job_json, &libs) {
            Ok(j) => j,
            Err(e) => return err(e),
        };
        let profile = match parse_machine(machine_text, Some(&job)) {
            Ok(m) => m,
            Err(e) => return err(e),
        };
        let machine = profile.post.clone();
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
        let limits = profiles::check_limits(&job, &tps, &profile);
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
            "machine_id": profile.id,
            "limits": limits,
            "tools": job.tools,
            "stock": job.stock,
            "profile": job.part.profile,
            "mesh_bounds": mesh.as_ref().and_then(|m| m.bounds()),
            "programs": programs,
        })
    }

    /// Simulate all programs in order. `opts`: `{"dx": mm, "na": cells}` (default 0.1, 360).
    /// Returns the grid, the part map and the stock map after each program (radii, f32).
    pub fn simulate(job_json: &str, opts: &str, mesh: &[u8]) -> Value {
        let job = match job_any(job_json, &profiles::builtin_libraries()) {
            Ok(j) => j,
            Err(e) => return err(e),
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
        let job = match job_any(job_json, &profiles::builtin_libraries()) {
            Ok(j) => j,
            Err(e) => return err(e),
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

    // ------------------------------------------------------------ playback sessions

    struct Session {
        pb: sim::Playback,
    }

    thread_local! {
        static SESSION: std::cell::RefCell<Option<Session>> = const { std::cell::RefCell::new(None) };
    }

    /// Standard base64 of raw bytes (typed arrays travel this way: 4/3 the size of the data).
    pub fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for c in bytes.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                if k <= c.len() {
                    out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn f32s(v: &[f32]) -> String {
        b64(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }

    fn u16s(v: &[u16]) -> String {
        b64(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }

    /// Open a material-removal playback of the whole job. `opts`: `{"dx", "na", "libraries"}`.
    /// Returns the grid, the part map, the operations, the tools and every segment's end point
    /// (`path`: f32 [x, r, a, rapid, tool, op] per segment, base64).
    pub fn sim_open(job_json: &str, opts: &str, mesh: &[u8]) -> Value {
        let opts: Value = serde_json::from_str(if opts.trim().is_empty() { "{}" } else { opts })
            .unwrap_or(Value::Null);
        let libs = match libraries(&opts) {
            Ok(l) => l,
            Err(e) => return err(e),
        };
        let job = match job_any(job_json, &libs) {
            Ok(j) => j,
            Err(e) => return err(e),
        };
        let mesh = match parse_mesh(&job, mesh) {
            Ok(m) => m,
            Err(e) => return err(e),
        };
        let dx = opts["dx"].as_f64().unwrap_or(0.1).max(0.02);
        let na = (opts["na"].as_u64().unwrap_or(240) as usize).clamp(8, 2048);
        let tps = match plan_with_mesh(&job, mesh.as_ref()) {
            Ok(t) => t,
            Err(e) => return err(e),
        };
        // grid and part map exactly as the batch simulation builds them
        let res = match simulate_job(&job, &[], mesh.as_ref(), dx, na, 0.02) {
            Ok(r) => r,
            Err(e) => return err(e),
        };
        let grid = res.stock.grid;
        let cutters = |t: u32| {
            job.tool(t)
                .and_then(|d| fourcam::cutter_for(t, d.shape).ok())
        };
        let pb = sim::Playback::new(
            sim::RadialMap::filled(grid, job.stock.radius),
            &tps,
            &cutters,
            (dx * 2.0).min(0.1),
        );
        let mut path = Vec::with_capacity(pb.len() * 6);
        for sg in &pb.segments {
            path.extend_from_slice(&[
                sg.to[0] as f32,
                sg.to[1] as f32,
                sg.to[2] as f32,
                f32::from(u8::from(sg.rapid)),
                sg.tool.unwrap_or(0) as f32,
                sg.op as f32,
            ]);
        }
        let mut ops = Vec::new();
        for (p, tp) in tps.iter().enumerate() {
            for m in &tp.moves {
                if let fourcam::Move::Operation { id, kind } = m {
                    ops.push(json!({"id": id, "tag": id + 1, "kind": kind, "program": tp.name, "program_index": p}));
                }
            }
        }
        let first_seg_of_program: Vec<usize> = (0..tps.len())
            .map(|p| {
                pb.segments
                    .iter()
                    .position(|s| s.program as usize == p)
                    .unwrap_or(pb.len())
            })
            .collect();
        let out = json!({
            "grid": {"x0": grid.x0, "dx": grid.dx, "nx": grid.nx, "na": grid.na},
            "stock_radius": job.stock.radius,
            "part": f32s(&res.part.center.r),
            "segments": pb.len(),
            "program_starts": first_seg_of_program,
            "path": f32s(&path),
            "ops": ops,
            "tools": job.tools,
        });
        SESSION.with(|s| *s.borrow_mut() = Some(Session { pb }));
        out
    }

    /// Move the open playback to segment `n`; returns the rows that changed (radii and
    /// cut-by tags, base64) so the display can update in place.
    pub fn sim_seek(n: usize) -> Value {
        SESSION.with(|s| {
            let mut s = s.borrow_mut();
            let Some(sess) = s.as_mut() else {
                return err("no playback open (call sim_open)");
            };
            let rows = sess.pb.seek(n);
            let pos = sess.pb.position();
            match rows {
                None => json!({"pos": pos, "rows": null}),
                Some((i0, i1)) => {
                    let na = sess.pb.stock.grid.na;
                    let (a, b) = (i0 * na, (i1 + 1) * na);
                    json!({"pos": pos, "rows": [i0, i1],
                           "r": f32s(&sess.pb.stock.r[a..b]), "op": u16s(&sess.pb.stock.op[a..b])})
                }
            }
        })
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
            "profile_list" => profile_list(),
            "op_types" => serde_json::to_value(fourcam::ops::registry()).unwrap_or(Value::Null),
            "check_project" => match fourcam::project::Project::from_json_any(&s(a)) {
                Ok((p, upgraded)) => {
                    json!({"project": p, "upgraded": upgraded, "problems": p.check_operations()})
                }
                Err(e) => err(e),
            },
            "sim_open" => sim_open(&s(a), &s(b), c),
            "sim_seek" => sim_seek(s(a).trim().parse().unwrap_or(0)),
            "import_profile" => import_profile(a, &s(b)),
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
    fn profiles_import_limits_and_library_tools() {
        let l = api::profile_list();
        assert_eq!(l["machines"].as_array().unwrap().len(), 4);
        assert_eq!(l["libraries"][0]["id"], "makera-starter");
        let fixture = include_bytes!("../../../tests/fixtures/fusion-tool-library.json");
        let imp = api::import_profile(fixture, "My Tools.json");
        assert_eq!(imp["kind"], "tools", "{imp}");
        assert_eq!(imp["profile"]["tools"].as_array().unwrap().len(), 3);
        assert!(api::import_profile(b"nope", "x.json")
            .get("error")
            .is_some());
        // the knob on the Z1: limit warning for the 4000 mm/min cap
        let v = api::plan_and_post(api::EXAMPLE_JOB, "makera-z1-4axis", &[], "");
        assert_eq!(v["machine_id"], "makera-z1-4axis");
        let lim = v["limits"].as_array().unwrap();
        assert!(
            lim.iter()
                .any(|i| i["message"].as_str().unwrap().contains("F5209")),
            "{lim:?}"
        );
        // a job using an imported library tool, on the Carvera (alias)
        let mut job: serde_json::Value = serde_json::from_str(api::EXAMPLE_JOB).unwrap();
        job["tools"][0] = serde_json::json!({"number": 1, "library": "my-tools", "tool": "fixture-flat-3.175-12mm"});
        let opts = serde_json::json!({"libraries": [imp["profile"]]}).to_string();
        let v = api::plan_and_post(&job.to_string(), "carvera", &[], &opts);
        assert!(v.get("error").is_none(), "{v}");
        assert_eq!(v["machine_id"], "makera-carvera-4axis");
        assert!(v["programs"][0]["gcode"]
            .as_str()
            .unwrap()
            .contains("(T1 Fixture Flat 3.175*12mm)"));
        // machine given as a profile object
        let z1 = serde_json::to_string(&api::profile_list()["machines"][0]).unwrap();
        assert_eq!(
            api::plan_and_post(api::EXAMPLE_JOB, &z1, &[], "")["machine_id"],
            "makera-z1-4axis"
        );
    }

    #[test]
    fn playback_session_seeks_and_reports_rows() {
        assert_eq!(api::b64(b"Man"), "TWFu");
        assert_eq!(api::b64(b"Ma"), "TWE=");
        assert_eq!(api::b64(b"M"), "TQ==");
        assert!(api::sim_seek(5).get("error").is_some() || api::sim_seek(5)["pos"].is_number());
        let o = api::sim_open(api::EXAMPLE_JOB, r#"{"dx": 0.2, "na": 90}"#, &[]);
        assert!(o.get("error").is_none(), "{o}");
        let n = o["segments"].as_u64().unwrap() as usize;
        assert!(n > 30_000);
        assert_eq!(o["ops"].as_array().unwrap().len(), 3);
        assert_eq!(o["ops"][1]["kind"], "helical_knurl");
        assert_eq!(o["program_starts"][0], 0);
        let s = api::sim_seek(2000);
        assert_eq!(s["pos"], 2000);
        assert!(s["rows"].is_array(), "{s}");
        let s = api::sim_seek(10);
        assert_eq!(s["rows"][0], 0, "jumping back redraws all rows");
        let s = api::sim_seek(n + 100);
        assert_eq!(s["pos"].as_u64().unwrap() as usize, n);
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
