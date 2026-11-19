//! Seeded fuzzing of planning and the simulation, through the same API the web app calls.
//!
//! Each case is a random project: a rotary bar (a revolved profile or a mesh on the axis),
//! an indexed bar or a vise block (a mesh), with odd shapes (steps, needles, slivers,
//! degenerate and stray triangles, very small and very large parts) and every operation
//! type that fits, its defaults from the engine then pushed to extremes. It is planned and
//! posted, then played back at a random resolution preset (some operations hidden) and
//! sought to the end, the middle and the start. A case fails on a panic, a time over its
//! budget, memory over its budget, or a NaN in the G-code or the playback; engine errors
//! that say what is wrong are fine.
//!
//! The fixed set runs with `cargo test`. A longer run, and replaying one case:
//!
//!   MARCAM_FUZZ_CASES=500 cargo test -p marcam-wasm --test fuzz -- --nocapture
//!   MARCAM_FUZZ_SEED=1234 MARCAM_FUZZ_CASES=1 cargo test -p marcam-wasm --test fuzz -- --nocapture
//!
//! A failing case is written to target/fuzz/<seed>/ (the project and its part.stl) for a closer
//! look (MARCAM_FUZZ_SAVE=1: every case). MARCAM_FUZZ_EACH=1 runs each operation on its own;
//! MARCAM_FUZZ_NOSIM=1 plans only.

use marcam_wasm::api;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ memory: peak bytes held

struct Counting;
static HELD: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = HELD.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        HELD.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                let now = HELD.fetch_add(new - l.size(), Ordering::Relaxed) + new - l.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                HELD.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

// ------------------------------------------------------------------ random numbers

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    /// Log-uniform in [lo, hi].
    fn log(&mut self, lo: f64, hi: f64) -> f64 {
        (lo.ln() + (hi.ln() - lo.ln()) * self.f()).exp()
    }
    fn int(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.int(xs.len())]
    }
}

// ------------------------------------------------------------------ parts

type Tri = [[f32; 3]; 3];

fn stl(tris: &[Tri]) -> Vec<u8> {
    let mut b = vec![0u8; 80];
    b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for t in tris {
        b.extend_from_slice(&[0u8; 12]);
        for v in t {
            for c in v {
                b.extend_from_slice(&c.to_le_bytes());
            }
        }
        b.extend_from_slice(&[0u8; 2]);
    }
    b
}

/// A revolved profile: 1 to 12 points, steps, radii from 0 to past the stock, written either
/// way round.
fn profile(r: &mut Rng, len: f64, stock_r: f64) -> Vec<[f64; 2]> {
    let n = 1 + r.int(12);
    let mut xs: Vec<f64> = (0..n).map(|_| -r.range(0.0, len)).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut pts = Vec::new();
    for x in xs {
        let rad = if r.chance(0.08) {
            0.0
        } else if r.chance(0.05) {
            stock_r * r.range(1.0, 1.3)
        } else {
            stock_r * r.range(0.15, 1.0)
        };
        pts.push([x, rad]);
        if r.chance(0.2) {
            // a vertical step
            pts.push([x, stock_r * r.range(0.15, 1.0)]);
        }
    }
    if r.chance(0.3) {
        pts.reverse();
    }
    pts
}

/// A revolved solid about X from x0 to x1 (radii along it), `seg` facets round.
fn lathe(rs: &[(f64, f64)], seg: usize) -> Vec<Tri> {
    let mut out = Vec::new();
    let p = |x: f64, r: f64, k: usize| -> [f32; 3] {
        let a = k as f64 / seg as f64 * std::f64::consts::TAU;
        [x as f32, (r * a.cos()) as f32, (r * a.sin()) as f32]
    };
    for w in rs.windows(2) {
        let ((x0, r0), (x1, r1)) = (w[0], w[1]);
        for k in 0..seg {
            out.push([p(x0, r0, k), p(x1, r1, k), p(x1, r1, k + 1)]);
            out.push([p(x0, r0, k), p(x1, r1, k + 1), p(x0, r0, k + 1)]);
        }
    }
    // end caps
    if let (Some(&(xa, ra)), Some(&(xb, rb))) = (rs.first(), rs.last()) {
        for k in 0..seg {
            out.push([[xa as f32, 0.0, 0.0], p(xa, ra, k + 1), p(xa, ra, k)]);
            out.push([[xb as f32, 0.0, 0.0], p(xb, rb, k), p(xb, rb, k + 1)]);
        }
    }
    out
}

fn cuboid(lo: [f64; 3], hi: [f64; 3]) -> Vec<Tri> {
    let v = |i: usize| -> [f32; 3] {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] } as f32,
            if i & 2 == 0 { lo[1] } else { hi[1] } as f32,
            if i & 4 == 0 { lo[2] } else { hi[2] } as f32,
        ]
    };
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut out = Vec::new();
    for q in quads {
        out.push([v(q[0]), v(q[1]), v(q[2])]);
        out.push([v(q[0]), v(q[2]), v(q[3])]);
    }
    out
}

/// Odd things added to a mesh: zero-area triangles, duplicates, a stray sliver.
fn oddities(r: &mut Rng, tris: &mut Vec<Tri>, lo: [f64; 3], hi: [f64; 3]) {
    let at = |r: &mut Rng| -> [f32; 3] {
        [
            r.range(lo[0], hi[0]) as f32,
            r.range(lo[1], hi[1]) as f32,
            r.range(lo[2], hi[2]) as f32,
        ]
    };
    if r.chance(0.3) {
        for _ in 0..1 + r.int(20) {
            let a = at(r);
            tris.push([a, a, at(r)]);
        }
    }
    if r.chance(0.2) && !tris.is_empty() {
        let k = r.int(tris.len());
        let t = tris[k];
        tris.push(t);
    }
    if r.chance(0.2) {
        let a = at(r);
        let mut b = a;
        b[0] += 0.001;
        tris.push([a, b, at(r)]);
    }
}

// ------------------------------------------------------------------ projects

struct Case {
    doc: Value,
    mesh: Vec<u8>,
    res: &'static str,
    machine: &'static str,
    what: String,
}

fn tools(r: &mut Rng) -> Vec<Value> {
    let mut out = vec![
        json!({"number": 1, "description": "flat", "kind": "flat", "diameter": r.log(0.3, 12.0), "flute_length": r.log(2.0, 40.0)}),
        json!({"number": 2, "description": "ball", "kind": "ball", "diameter": r.log(0.3, 12.0), "flute_length": r.log(2.0, 40.0)}),
        json!({"number": 3, "description": "vbit", "kind": "vbit", "angle": r.range(10.0, 150.0), "diameter": r.log(0.5, 12.0), "flute_length": r.log(2.0, 30.0)}),
    ];
    let d = r.log(0.5, 10.0);
    out.push(json!({"number": 4, "description": "tapered", "kind": "tapered_ball", "tip_diameter": d * r.range(0.05, 1.0), "taper_angle": r.range(0.0, 30.0), "diameter": d, "flute_length": r.log(2.0, 30.0)}));
    out
}

fn tool_kind(t: &Value) -> &str {
    t["kind"].as_str().unwrap_or("")
}

/// Push numeric parameters to extremes (mostly within their limits; now and then past them).
fn perturb(r: &mut Rng, op: &mut Value, ty: &marcam::ops::OperationType) {
    for p in &ty.params {
        if !matches!(p.kind, "number" | "integer") || p.name == "tool" || !r.chance(0.35) {
            continue;
        }
        let Some(v) = op.get(p.name).and_then(Value::as_f64) else {
            continue;
        };
        let mut x = match r.int(6) {
            0 => p.min.unwrap_or(v * 0.01),
            1 => p.max.unwrap_or(v * 100.0),
            2 => v * r.log(0.01, 0.5),
            3 => v * r.log(2.0, 100.0),
            4 => p.min.map_or(v, |m| m + (v - m).abs() * 0.001),
            _ => v,
        };
        if r.chance(0.9) {
            if let Some(m) = p.min {
                x = x.max(m);
            }
            if let Some(m) = p.max {
                x = x.min(m);
            }
        }
        op[p.name] = if p.kind == "integer" {
            json!(x.round() as i64)
        } else {
            json!(x)
        };
    }
    for key in ["x_range", "a_range"] {
        if let Some([a, b]) = op
            .get(key)
            .and_then(|v| serde_json::from_value::<[f64; 2]>(v.clone()).ok())
        {
            if r.chance(0.3) {
                let (lo, hi) = match r.int(4) {
                    0 => (b, a),
                    1 => (a, a + (b - a) * 0.001),
                    2 => (a - 50.0, b + 50.0),
                    _ => (a + (b - a) * r.f(), b),
                };
                op[key] = json!([lo, hi]);
            }
        }
    }
}

fn make_case(seed: u64) -> Case {
    let mut r = Rng(seed);
    let kind = *r.pick(&["rotary", "rotary", "indexed", "vise"]);
    let res = *r.pick(&["coarse", "normal", "normal", "fine", "ultra"]);
    let machine = *r.pick(&["makera-carvera-4axis", "makera-z1-4axis", "linuxcnc-4axis"]);
    let tools = tools(&mut r);
    let mut mesh = Vec::new();
    let (models, stock, what) = if kind == "vise" {
        let (w, d, h) = (r.log(2.0, 300.0), r.log(2.0, 200.0), r.log(0.5, 60.0));
        let m = r.range(0.0, 3.0);
        let shape = r.int(4);
        let (lo, hi) = ([m, m, -h], [m + w, m + d, -h * r.range(0.0, 0.9)]);
        let mut tris = match shape {
            0 => cuboid(lo, hi),
            1 => {
                // a needle standing up
                let c = [m + w / 2.0, m + d / 2.0];
                cuboid(
                    [c[0] - 0.05, c[1] - 0.05, -h],
                    [c[0] + 0.05, c[1] + 0.05, -0.01],
                )
            }
            2 => {
                let mut t = cuboid(lo, hi);
                let c = [m + w * r.f(), m + d * r.f()];
                t.extend(cuboid(
                    [c[0] - 1.0, c[1] - 1.0, hi[2]],
                    [c[0] + 1.0, c[1] + 1.0, -h * 0.01],
                ));
                t
            }
            _ => {
                // a sliver plate
                cuboid([m, m, -h], [m + w, m + 0.02, -h + 0.02])
            }
        };
        oddities(&mut r, &mut tris, [m, m, -h], [m + w, m + d, 0.0]);
        mesh = stl(&tris);
        (
            json!([{"id": "part", "kind": "mesh", "file": "part.stl", "axis": "x", "translate": [0.0, 0.0, 0.0]}]),
            json!({"shape": "box", "min": [0.0, 0.0, -h], "max": [w + 2.0 * m, d + 2.0 * m, 0.0]}),
            format!("vise {w:.2}x{d:.2}x{h:.2} shape {shape}"),
        )
    } else {
        let stock_r = r.log(1.0, 60.0);
        let len = r.log(1.0, 250.0);
        let pts = profile(&mut r, len, stock_r);
        let stock =
            json!({"shape": "cylinder", "radius": stock_r, "length": len + r.range(0.0, 20.0)});
        if r.chance(0.5) || kind == "indexed" {
            let mut m = json!({"id": "part", "kind": "profile", "points": pts});
            if r.chance(0.2) {
                m["beyond_front"] = json!(stock_r * r.f());
            }
            (
                json!([m]),
                stock,
                format!(
                    "{kind} profile r{stock_r:.2} len {len:.2} {} points",
                    pts.len()
                ),
            )
        } else {
            let mut sorted = pts.clone();
            sorted.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
            let mut rs: Vec<(f64, f64)> = sorted.iter().map(|p| (p[0], p[1].max(0.01))).collect();
            if rs.len() == 1 {
                rs.push((rs[0].0 + 1.0, rs[0].1));
            }
            let seg = *r.pick(&[3usize, 8, 48, 200]);
            let mut tris = lathe(&rs, seg);
            let rr = stock_r * 1.2;
            oddities(&mut r, &mut tris, [-len, -rr, -rr], [0.0, rr, rr]);
            mesh = stl(&tris);
            (
                json!([{"id": "part", "kind": "mesh", "file": "part.stl", "axis": "x", "translate": [0.0, 0.0, 0.0]}]),
                stock,
                format!(
                    "{kind} mesh r{stock_r:.2} len {len:.2} {} triangles, {seg} round",
                    tris.len()
                ),
            )
        }
    };
    // flutes mostly long enough for the stock (else most cases end in that one error)
    let depth = stock["radius"].as_f64().unwrap_or_else(|| {
        stock["max"][2].as_f64().unwrap_or(0.0) - stock["min"][2].as_f64().unwrap_or(0.0)
    });
    let mut tools = tools;
    for t in tools.iter_mut() {
        if r.chance(0.85) {
            let f = t["flute_length"].as_f64().unwrap_or(10.0).max(depth + 2.0);
            t["flute_length"] = json!(f);
        }
    }
    let mesh_part = models[0]["kind"] == "mesh";
    let mut doc = json!({
        "schema_version": "0.2", "name": format!("fuzz {seed}"), "models": models, "tools": tools,
        "setups": [{"id": "s", "name": "s", "kind": kind, "machine": machine, "stock": stock, "clearance": r.log(0.5, 10.0),
                    "part": ["part"], "spindle_rpm": 12000, "operations": [], "outputs": [{"name": "fuzz", "title": "fuzz", "operations": []}]}],
    });
    // operations: each type that fits, a tool it takes, its defaults, pushed about
    let mut ops = Vec::new();
    let mut n = 0;
    for ty in marcam::ops::registry() {
        // (turning works from a revolved profile only)
        let profile_only = ty.geometry.len() == 1 && ty.geometry[0] == "profile";
        if !marcam::ops::fits_setup(&ty, kind) || (mesh_part && profile_only) || !r.chance(0.5) {
            continue;
        }
        let fit: Vec<&Value> = tools
            .iter()
            .filter(|t| ty.tool_shapes.contains(&tool_kind(t)))
            .collect();
        if fit.is_empty() {
            continue;
        }
        let tool = r.pick(&fit)["number"].as_u64().unwrap();
        let d = api::op_defaults(
            &doc.to_string(),
            &json!({"type": ty.kind, "setup": "s", "tool": tool}).to_string(),
            &mesh,
        );
        if d.get("error").is_some() {
            continue;
        }
        let mut op = d;
        perturb(&mut r, &mut op, &ty);
        n += 1;
        op["id"] = json!(format!("op{n}"));
        ops.push(op);
    }
    let ids: Vec<Value> = ops.iter().map(|o| o["id"].clone()).collect();
    doc["setups"][0]["operations"] = json!(ops);
    doc["setups"][0]["outputs"][0]["operations"] = json!(ids);
    let what = format!(
        "{what}; {res}; {} ops: {}",
        ops.len(),
        ops.iter()
            .map(|o| o["type"].as_str().unwrap_or("?").to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    Case {
        doc,
        mesh,
        res,
        machine,
        what,
    }
}

// ------------------------------------------------------------------ running a case

#[derive(Debug, Default)]
struct Outcome {
    planned: Option<String>,
    segments: usize,
    plan_ms: u128,
    sim_ms: u128,
    problems: Vec<String>,
}

/// Base64 (standard alphabet, padded) to bytes.
fn unb64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// Whether a base64 run of f32s holds a value that is not finite.
fn f32s_bad(s: &str) -> bool {
    unb64(s)
        .as_chunks::<4>()
        .0
        .iter()
        .any(|c| !f32::from_le_bytes(*c).is_finite())
}

fn run(case: &Case, seed: u64) -> Outcome {
    let mut out = Outcome::default();
    let doc = case.doc.to_string();
    let t0 = Instant::now();
    let planned = api::plan_and_post(&doc, case.machine, &case.mesh, "{}");
    out.plan_ms = t0.elapsed().as_millis();
    if let Some(e) = planned.get("error") {
        out.planned = Some(format!("error: {e}"));
        return out;
    }
    if planned["programs"].as_array().is_some_and(|p| {
        p.iter().any(|p| {
            p["gcode"]
                .as_str()
                .is_some_and(|g| g.contains("NaN") || g.contains("inf"))
        })
    }) {
        out.problems.push("NaN in the G-code".into());
    }
    out.planned = Some("ok".into());
    if env("MARCAM_FUZZ_NOSIM").is_some() {
        return out;
    }
    // the playback, some operations hidden
    let n_ops = case.doc["setups"][0]["operations"]
        .as_array()
        .map_or(0, Vec::len);
    let mut r = Rng(seed ^ 0x5151);
    let hide: Vec<usize> = (0..n_ops).filter(|_| r.chance(0.2)).collect();
    // the engine's estimate (seconds in a browser; natively about half), then the playback,
    // refused past 40 s as the app's confirmation would stop it
    let sim_opts = json!({"res": case.res, "hide": hide, "max_seconds": 40.0});
    let grid = api::sim_grid(
        &doc,
        &json!({"res": case.res, "hide": hide, "estimate": true}).to_string(),
        &case.mesh,
    );
    let estimate = grid["time"]["seconds"].as_f64();
    let t1 = Instant::now();
    let open = api::sim_open(&doc, &sim_opts.to_string(), &case.mesh);
    if let Some(e) = open.get("error") {
        out.planned = Some(format!("ok; playback: {e}"));
        out.sim_ms = t1.elapsed().as_millis();
        return out;
    }
    out.segments = open["segments"].as_u64().unwrap_or(0) as usize;
    if open["path"].as_str().is_some_and(f32s_bad) {
        out.problems.push("NaN in the playback path".into());
    }
    for at in [out.segments, out.segments / 2, 0, out.segments] {
        let s = api::sim_seek(at);
        if let Some(e) = s.get("error") {
            out.problems.push(format!("seek {at}: {e}"));
        }
        if ["r", "z"]
            .iter()
            .any(|k| s[*k].as_str().is_some_and(f32s_bad))
        {
            out.problems.push(format!("seek {at}: NaN in the stock"));
        }
    }
    api::sim_close();
    out.sim_ms = t1.elapsed().as_millis();
    // the whole job is simulated about 2.5 times over (to the end, the middle, the start, the
    // end again): far longer than the estimate says is a hang the estimate missed
    if let Some(est) = estimate {
        let native = est / 2.0 * 2.5;
        if out.sim_ms as f64 / 1000.0 > 4.0 * native + 10.0 {
            out.problems.push(format!(
                "playback {:.1} s against an estimate of {native:.1} s",
                out.sim_ms as f64 / 1000.0
            ));
        }
    } else {
        out.problems
            .push(format!("no simulation estimate: {}", grid));
    }
    out
}

fn env(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn save(seed: u64, case: &Case) {
    save_as(&seed.to_string(), case);
}

/// The case as a project file (and its part.stl) in target/fuzz/<name>/.
fn save_as(name: &str, case: &Case) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fuzz")
        .join(name);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(
        dir.join("fuzz.marcam.json"),
        serde_json::to_string_pretty(&case.doc).unwrap(),
    );
    if !case.mesh.is_empty() {
        let _ = std::fs::write(dir.join("part.stl"), &case.mesh);
    }
}

#[test]
fn fuzz_plan_and_simulate() {
    let base = env("MARCAM_FUZZ_SEED").unwrap_or(1);
    let cases = env("MARCAM_FUZZ_CASES").unwrap_or(24);
    // budgets: a case planned and played back in this time and memory (the browser has less)
    let time = Duration::from_secs(env("MARCAM_FUZZ_SECONDS").unwrap_or(240));
    let mem_mb = env("MARCAM_FUZZ_MB").unwrap_or(1500) as usize;
    let mut failed = Vec::new();
    // MARCAM_FUZZ_EACH=1: each case's operations one at a time (to find the one at fault)
    let each = env("MARCAM_FUZZ_EACH").is_some();
    let mut todo: Vec<(u64, Case)> = Vec::new();
    for seed in base..base + cases {
        let case = make_case(seed);
        if !each {
            todo.push((seed, case));
            continue;
        }
        let ops = case.doc["setups"][0]["operations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for op in ops {
            let mut doc = case.doc.clone();
            doc["setups"][0]["operations"] = json!([op.clone()]);
            doc["setups"][0]["outputs"][0]["operations"] = json!([op["id"].clone()]);
            let what = format!("{} -> only {} {}", case.what, op["id"], op);
            todo.push((
                seed,
                Case {
                    doc,
                    mesh: case.mesh.clone(),
                    res: case.res,
                    machine: case.machine,
                    what,
                },
            ));
        }
    }
    for (k, (seed, case)) in todo.into_iter().enumerate() {
        if env("MARCAM_FUZZ_SAVE").is_some() {
            save_as(&format!("{seed}-{k}"), &case);
        }
        let what = case.what.clone();
        let held0 = HELD.load(Ordering::Relaxed);
        PEAK.store(held0, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let doc = case.doc.clone();
        let mesh = case.mesh.clone();
        let (res, machine) = (case.res, case.machine);
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let c = Case {
                    doc,
                    mesh,
                    res,
                    machine,
                    what: String::new(),
                };
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&c, seed)));
                let _ = tx.send(r.map_err(|e| {
                    e.downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_default()
                }));
            })
            .unwrap();
        let t0 = Instant::now();
        let got = rx.recv_timeout(time);
        let peak_mb = PEAK.load(Ordering::Relaxed).saturating_sub(held0) >> 20;
        let mut problems = Vec::new();
        let line = match got {
            Err(_) => {
                problems.push(format!("over {} s (still running)", time.as_secs()));
                String::new()
            }
            Ok(Err(panic)) => {
                problems.push(format!("panic: {panic}"));
                String::new()
            }
            Ok(Ok(o)) => {
                problems.extend(o.problems.iter().cloned());
                format!(
                    "{} | plan {} ms, playback {} ms, {} segments",
                    o.planned.unwrap_or_default(),
                    o.plan_ms,
                    o.sim_ms,
                    o.segments
                )
            }
        };
        if peak_mb > mem_mb {
            problems.push(format!("peak memory {peak_mb} MB (budget {mem_mb})"));
        }
        let ok = problems.is_empty();
        println!(
            "{} seed {seed}: {what} | {line} | {peak_mb} MB, {:.1} s{}",
            if ok { "ok  " } else { "FAIL" },
            t0.elapsed().as_secs_f64(),
            if ok {
                String::new()
            } else {
                format!(" | {}", problems.join("; "))
            }
        );
        if !ok {
            save(seed, &case);
            failed.push(format!("seed {seed}: {}", problems.join("; ")));
            if problems.iter().any(|p| p.starts_with("over")) {
                // the case still runs on its thread: stop here
                break;
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} case(s) failed (replay: MARCAM_FUZZ_SEED=<seed> MARCAM_FUZZ_CASES=1; saved in target/fuzz):\n{}",
        failed.len(),
        failed.join("\n")
    );
}
