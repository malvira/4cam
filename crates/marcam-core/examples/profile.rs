//! Where the time goes: plans, posts and simulates the examples and prints a timing per
//! stage (single-threaded, as the engine runs today). For docs/gpu.md.
//!
//!     cargo run --release -p marcam-core --example profile

use marcam::job::Job;
use marcam::sim::{self, Resolution};
use marcam::sim3::{self, Heightmap3};
use marcam::{cutter_for, gouge_check, limits_for, plan_with_mesh, post, verify_gcode};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(dir: &str, file: &str) -> (Job, Option<marcam::Mesh>, PathBuf) {
    let d = root().join("examples").join(dir);
    let text = std::fs::read_to_string(d.join(file)).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    let mut jobs = marcam::project::jobs_from_any_with(&text, &libs, &|s| s.load(&d).ok()).unwrap();
    let job = jobs.remove(0).1;
    let mesh = marcam::load_mesh(&job, &d).unwrap();
    (job, mesh, d)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn row(example: &str, stage: &str, v: f64, extra: &str) {
    println!("| {example} | {stage} | {v:.0} | {extra} |");
}

fn profile(name: &str, dir: &str, file: &str, machine: &str) {
    let t = Instant::now();
    let (job, mesh, _d) = load(dir, file);
    row(
        name,
        "load (mesh / image relief)",
        ms(t),
        &mesh.as_ref().map_or(String::new(), |m| {
            format!("{} triangles", m.triangles.len())
        }),
    );
    let profile = marcam::profiles::builtin_machine(machine).unwrap();
    let t = Instant::now();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    let moves: usize = tps.iter().map(|t| t.moves.len()).sum();
    row(
        name,
        "plan (toolpaths, drop-cutter)",
        ms(t),
        &format!("{moves} moves"),
    );
    let t = Instant::now();
    let texts: Vec<String> = tps.iter().map(|tp| post(tp, &profile.post)).collect();
    row(
        name,
        "post",
        ms(t),
        &format!(
            "{} lines",
            texts.iter().map(|s| s.lines().count()).sum::<usize>()
        ),
    );
    let t = Instant::now();
    for (tp, text) in tps.iter().zip(&texts) {
        verify_gcode(text, &limits_for(tp, &profile.post));
    }
    row(name, "verify", ms(t), "");
    let t = Instant::now();
    for tp in &tps {
        marcam::estimate::estimate_toolpath(tp, &profile);
    }
    row(name, "time estimate", ms(t), "");
    let t = Instant::now();
    let g = gouge_check(&job, &tps, mesh.as_ref()).unwrap_or_default();
    row(
        name,
        "gouge check",
        ms(t),
        &format!(
            "{} cut moves",
            g.iter().map(|g| g.moves_checked).sum::<usize>()
        ),
    );
    let cutter = |n: u32| cutter_for(n, job.tool(n)?.shape).ok();
    for res in ["normal", "fine"] {
        let r = Resolution::preset(res).unwrap();
        if let Some(sb) = job.stock_box {
            let t = Instant::now();
            let mut hm = Heightmap3::new(&sb, r.cell_for(&sb));
            let cells = hm.z.len();
            let mut samples = 0;
            for tp in &tps {
                let step = (hm.cell * 0.5).min(0.1);
                samples += sim3::simulate3(&mut hm, tp, &cutter, step).samples;
            }
            row(
                name,
                &format!("simulate 3-axis ({res})"),
                ms(t),
                &format!("{cells} cells, {samples} tool positions"),
            );
            if let Some(m) = mesh.as_ref() {
                let t = Instant::now();
                let part = sim3::part_top(&hm, m);
                row(
                    name,
                    &format!("part map + deviation ({res})"),
                    ms(t),
                    &format!("{} part cells", part.iter().flatten().count()),
                );
            }
        } else {
            let t = Instant::now();
            let res_j = marcam::simulate_job(&job, &tps, mesh.as_ref(), r.dx, r.na, 0.02).unwrap();
            let samples: usize = res_j.programs.iter().map(|p| p.1.samples).sum();
            row(
                name,
                &format!("simulate rotary + part map ({res})"),
                ms(t),
                &format!("{} cells, {samples} tool positions", res_j.stock.grid.len()),
            );
        }
    }
    // playback: open, play to the end (normal grid)
    let r = Resolution::preset("normal").unwrap();
    let t = Instant::now();
    if let Some(sb) = job.stock_box {
        let mut pb =
            sim3::Playback3::new(Heightmap3::new(&sb, r.cell_for(&sb)), &tps, &cutter, 0.1);
        let n = pb.len();
        pb.seek(n);
        row(
            name,
            "playback to the end (normal)",
            ms(t),
            &format!("{n} segments"),
        );
    } else if let Ok(g) = marcam::sim_grid(&job, mesh.as_ref(), r.dx, r.na) {
        let mut pb = sim::Playback::new(
            sim::RadialMap::filled(g, job.stock.radius),
            &tps,
            &cutter,
            0.1,
        );
        let n = pb.len();
        pb.seek(n);
        row(
            name,
            "playback to the end (normal)",
            ms(t),
            &format!("{n} segments"),
        );
    }
}

fn main() {
    let _ = Path::new(".");
    println!("| example | stage | ms | size |\n| --- | --- | ---: | --- |");
    profile("knob (Z1)", "knob", "knob.project.json", "makera-z1-4axis");
    profile(
        "knob mesh finish (Z1)",
        "knob",
        "knob_finish.job.json",
        "makera-z1-4axis",
    );
    profile(
        "plate (Carvera)",
        "plate",
        "plate.project.json",
        "makera-carvera-4axis",
    );
    profile(
        "plaque (Carvera)",
        "plaque",
        "plaque.project.json",
        "makera-carvera-4axis",
    );
    profile("ring (Z1)", "ring", "ring.project.json", "makera-z1-4axis");
}
