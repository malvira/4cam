//! The GPU simulation against the CPU's on the knob (turning, knurl, chamfer) and the ring
//! (wrapped engraving). Skips, saying so, when there is no hardware GPU (e.g. CI).

use marcam::job::Job;
use marcam::sim::{self, RadialMap};
use marcam_gpu::{simulate_or_cpu, Backend, Gpu};
use std::path::PathBuf;

fn example(dir: &str, file: &str) -> Job {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(dir);
    let text = std::fs::read_to_string(d.join(file)).unwrap();
    let libs = marcam::profiles::builtin_libraries();
    marcam::project::jobs_from_any_with(&text, &libs, &|s| s.load(&d).ok())
        .unwrap()
        .remove(0)
        .1
}

/// GPU and CPU stocks after the whole job: largest height difference, and the share of cut
/// cells whose tags differ.
fn compare(gpu: &Gpu, job: &Job, dx: f64, na: usize) -> (f64, f64, usize) {
    let tps = marcam::plan(job).unwrap();
    let grid = marcam::sim_grid(job, None, dx, na).unwrap();
    let cutter = |n: u32| marcam::cutter_for(n, job.tool(n)?.shape).ok();
    let mut cpu = RadialMap::filled(grid, job.stock.radius);
    for tp in &tps {
        sim::simulate(&mut cpu, tp, &cutter, 0.1);
    }
    let mut g = RadialMap::filled(grid, job.stock.radius);
    let hits = gpu.simulate(&mut g, &tps, &cutter, 0.1).unwrap();
    let n = hits.len();
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
        ("knob", example("knob", "knob.marcam.toml")),
        ("ring", example("ring", "ring.marcam.toml")),
    ] {
        let (max, tags, n) = compare(&gpu, &job, 0.1, 240);
        eprintln!(
            "{name}: {n} rapids through stock, max height difference {max:.4} mm, {:.3}% tags differ",
            100.0 * tags
        );
        // 1 µm quantization (floor) plus f32 rounding of a ~12 mm radius
        assert!(max <= 0.003, "{name}: {max}");
        assert!(tags < 0.01, "{name}: {tags}");
    }
}

#[test]
fn indexed_jobs_fall_back_to_the_cpu() {
    let job = example("knob", "knob_indexed.marcam.toml");
    let tps = marcam::plan(&job).unwrap();
    let grid = marcam::sim_grid(&job, None, 0.2, 120).unwrap();
    let cutter = |n: u32| marcam::cutter_for(n, job.tool(n)?.shape).ok();
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

#[test]
fn gpu_block_matches_the_cpu_on_the_plate_and_plaque() {
    use marcam::sim3::{simulate3, Heightmap3};
    let Some(gpu) = Gpu::new() else {
        eprintln!("no hardware GPU: skipped");
        return;
    };
    for (dir, file) in [
        ("plate", "plate.marcam.toml"),
        ("plaque", "plaque.marcam.toml"),
    ] {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(dir);
        let job = example(dir, file);
        let mesh = marcam::load_mesh(&job, &d).unwrap();
        let tps = marcam::plan_with_mesh(&job, mesh.as_ref()).unwrap();
        let sb = job.stock_box.unwrap();
        let cell = marcam::sim::Resolution::preset("normal")
            .unwrap()
            .cell_for(&sb);
        let cutter = |n: u32| marcam::cutter_for(n, job.tool(n)?.shape).ok();
        let mut cpu = Heightmap3::new(&sb, cell);
        for tp in &tps {
            simulate3(&mut cpu, tp, &cutter, 0.1);
        }
        let mut g = Heightmap3::new(&sb, cell);
        let n = gpu
            .simulate_block(&mut g, &tps, &cutter, 0.1)
            .unwrap()
            .len();
        let max = cpu
            .z
            .iter()
            .zip(&g.z)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        let cut = cpu.op.iter().filter(|&&t| t != 0).count().max(1);
        let tags = cpu.op.iter().zip(&g.op).filter(|(a, b)| a != b).count() as f64 / cut as f64;
        eprintln!(
            "{dir}: {n} rapids through stock, max height difference {max:.4} mm, {:.3}% tags differ",
            100.0 * tags
        );
        assert!(max <= 0.002, "{dir}: {max}");
        assert!(tags < 0.01, "{dir}: {tags}");
    }
}

#[test]
fn gpu_reports_the_rapids_that_cut_like_the_cpu() {
    use marcam::toolpath::Builder;
    let Some(gpu) = Gpu::new() else {
        eprintln!("no hardware GPU: skipped");
        return;
    };
    // a cut, then a rapid sideways at depth (through stock), then a safe rapid
    let mut b = Builder::new();
    b.tool_change(1, 10000);
    b.operation(0, "test");
    b.rapid(Some(-10.0), Some(15.0), Some(0.0));
    b.plunge(9.0, 60.0);
    b.cut(-5.0, 9.0, 0.0, 300.0, 60.0);
    b.rapid(Some(-15.0), None, None);
    b.rapid_r(15.0);
    b.rapid(Some(-2.0), None, Some(90.0));
    let tp = marcam::Toolpath {
        name: "t".into(),
        title: "t".into(),
        tool: None,
        notes: vec![],
        reset_a: false,
        safe_r: 15.0,
        moves: b.moves,
        origin_comment: None,
        plunge_limit: None,
        ramp_angle: None,
    };
    let g = sim::Grid::new(-20.0, 0.0, 0.1, 240);
    let cutter = |_| Some(marcam::dropcutter::Cutter::Flat { radius: 1.5 });
    let mut cpu = RadialMap::filled(g, 12.0);
    let st = sim::simulate(&mut cpu, &tp, &cutter, 0.1);
    let mut gs = RadialMap::filled(g, 12.0);
    let hits = gpu
        .simulate(&mut gs, std::slice::from_ref(&tp), &cutter, 0.1)
        .unwrap();
    assert_eq!(st.rapid_hits.len(), 1);
    assert_eq!(hits, st.rapid_hits);
}
