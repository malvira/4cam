//! GPU against CPU (one thread, all threads) for the rotary stock simulation.
//!
//!     cargo run --release -p fourcam-gpu --example bench

use fourcam::sim::{self, RadialMap, Resolution};
use fourcam_gpu::Gpu;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let gpu = Gpu::new();
    println!(
        "GPU: {}",
        gpu.as_ref().map_or("none".to_string(), |g| g.name.clone())
    );
    println!("| example | grid | cells | CPU 1 thread ms | CPU all threads ms | GPU ms |\n| --- | --- | ---: | ---: | ---: | ---: |");
    for (name, dir, file) in [
        ("knob", "knob", "knob.project.json"),
        ("ring", "ring", "ring.project.json"),
    ] {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(dir);
        let text = std::fs::read_to_string(d.join(file)).unwrap();
        let libs = fourcam::profiles::builtin_libraries();
        let job = fourcam::project::jobs_from_any_with(&text, &libs, &|_| None)
            .unwrap()
            .remove(0)
            .1;
        let tps = fourcam::plan(&job).unwrap();
        let cutter = |n: u32| fourcam::cutter_for(n, job.tool(n)?.shape).ok();
        for res in ["normal", "fine", "ultra"] {
            let r = Resolution::preset(res).unwrap();
            let grid = fourcam::sim_grid(&job, None, r.dx, r.na).unwrap();
            let step = (r.dx * 2.0).min(0.1);
            let time = |threads: usize| {
                let mut s = RadialMap::filled(grid, job.stock.radius);
                let t = Instant::now();
                for tp in &tps {
                    sim::simulate_threads(&mut s, tp, &cutter, step, threads);
                }
                t.elapsed().as_secs_f64() * 1000.0
            };
            let one = if res == "ultra" { f64::NAN } else { time(1) };
            let all = time(sim::sim_threads(grid.nx));
            let g = gpu.as_ref().map_or(f64::NAN, |g| {
                let mut s = RadialMap::filled(grid, job.stock.radius);
                let t = Instant::now();
                g.simulate(&mut s, &tps, &cutter, step).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            });
            println!(
                "| {name} | {res} | {} | {one:.0} | {all:.0} | {g:.0} |",
                grid.len()
            );
        }
    }
}
