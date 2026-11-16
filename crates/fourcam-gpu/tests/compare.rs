//! The GPU simulation against the CPU's on the knob (turning, knurl, chamfer) and the ring
//! (wrapped engraving). Skips, saying so, when there is no hardware GPU (e.g. CI).

use fourcam::job::Job;
use fourcam::sim::{self, RadialMap};
use fourcam_gpu::{simulate_or_cpu, Backend, Gpu};
use std::path::PathBuf;

fn example(dir: &str, file: &str) -> Job {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(dir);
    let text = std::fs::read_to_string(d.join(file)).unwrap();
    let libs = fourcam::profiles::builtin_libraries();
    fourcam::project::jobs_from_any_with(&text, &libs, &|s| s.load(&d).ok())
        .unwrap()
        .remove(0)
        .1
}

/// GPU and CPU stocks after the whole job: largest height difference, and the share of cut
/// cells whose tags differ.
fn compare(gpu: &Gpu, job: &Job, dx: f64, na: usize) -> (f64, f64, usize) {
    let tps = fourcam::plan(job).unwrap();
    let grid = fourcam::sim_grid(job, None, dx, na).unwrap();
    let cutter = |n: u32| fourcam::cutter_for(n, job.tool(n)?.shape).ok();
    let mut cpu = RadialMap::filled(grid, job.stock.radius);
    for tp in &tps {
        sim::simulate(&mut cpu, tp, &cutter, 0.1);
    }
    let mut g = RadialMap::filled(grid, job.stock.radius);
    let n = gpu.simulate(&mut g, &tps, &cutter, 0.1).unwrap();
    let max = cpu
        .r
        .iter()
        .zip(&g.r)
        .map(|(a, b)| (a - b).abs() as f64)
        .fold(0.0, f64::max);
    let cut = cpu.op.iter().filter(|&&t| t != 0).count().max(1);
    let tags = cpu.op.iter().zip(&g.op).filter(|(a, b)| a != b).count();
    (max, tags as f64 / cut as f64, n)
}

#[test]
fn gpu_stock_matches_the_cpu_within_the_quantization() {
    let Some(gpu) = Gpu::new() else {
        eprintln!("no hardware GPU: skipped (the CPU simulation is used)");
        return;
    };
    eprintln!("GPU: {}", gpu.name);
    for (name, job) in [
        ("knob", example("knob", "knob.project.json")),
        ("ring", example("ring", "ring.project.json")),
    ] {
        let (max, tags, n) = compare(&gpu, &job, 0.1, 240);
        eprintln!(
            "{name}: {n} tool positions, max height difference {max:.4} mm, {:.3}% tags differ",
            100.0 * tags
        );
        // 1 µm quantization (floor) plus f32 rounding of a ~12 mm radius
        assert!(max <= 0.003, "{name}: {max}");
        assert!(tags < 0.01, "{name}: {tags}");
    }
}

#[test]
fn indexed_jobs_fall_back_to_the_cpu() {
    let job = example("knob", "knob_indexed.project.json");
    let tps = fourcam::plan(&job).unwrap();
    let grid = fourcam::sim_grid(&job, None, 0.2, 120).unwrap();
    let cutter = |n: u32| fourcam::cutter_for(n, job.tool(n)?.shape).ok();
    let mut stock = RadialMap::filled(grid, job.stock.radius);
    let gpu = Gpu::new();
    let used = simulate_or_cpu(gpu.as_ref(), &mut stock, &tps, &cutter, 0.1);
    assert!(matches!(used, Backend::Cpu(_)), "{used:?}");
    // and the CPU result is the plain simulation's
    let mut plain = RadialMap::filled(grid, job.stock.radius);
    for tp in &tps {
        sim::simulate(&mut plain, tp, &cutter, 0.1);
    }
    assert!(stock.r == plain.r);
}
