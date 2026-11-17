//! Where an example's estimated machining time goes.
//!
//! `cargo run --release -p marcam-core --example time_breakdown -- examples/knob/knob.project.json ...`
//!
//! For each program it prints:
//! - floor: the time at the programmed feeds and the rapid rates, with no acceleration and no
//!   axis or controller limits;
//! - limits: the same with the axes' top feeds and the controller's feed cap;
//! - estimate: the full estimate (acceleration, junctions);
//! - the rapids, tool changes and dwells in it, and the moves' count and mean length.

use marcam::estimate::{estimate_gcode, Dynamics};
use marcam::{load_mesh, plan_with_mesh, post};
use std::path::Path;

fn main() {
    let libs = marcam::profiles::builtin_libraries();
    let mut paths: Vec<String> = std::env::args().skip(1).collect();
    let machine_id = match paths.iter().position(|a| a == "--machine") {
        Some(k) => {
            let m = paths.remove(k + 1);
            paths.remove(k);
            m
        }
        None => String::new(),
    };
    println!(
        "{:<44} {:>8} {:>8} {:>8} {:>6} | {:>7} {:>7} {:>6} {:>6} {:>7} {:>6}",
        "program",
        "floor",
        "limits",
        "estimate",
        "x",
        "cut",
        "rapid",
        "other",
        "moves",
        "mean mm",
        "F max"
    );
    for path in &paths {
        let text = std::fs::read_to_string(path).expect("read");
        let dir = Path::new(path).parent().unwrap().to_path_buf();
        let meshes = |src: &marcam::job::MeshSource| src.load(&dir).ok();
        let jobs = marcam::project::jobs_from_any_with(&text, &libs, &meshes).expect("project");
        for (setup, job) in &jobs {
            let id = if machine_id.is_empty() {
                job.machine
                    .clone()
                    .unwrap_or_else(|| "makera-z1-4axis".into())
            } else {
                machine_id.clone()
            };
            let profile = marcam::profiles::builtin_machine(&id).expect("machine");
            let mesh = load_mesh(job, &dir).expect("mesh");
            let tps = plan_with_mesh(job, mesh.as_ref()).expect("plan");
            for tp in &tps {
                let g = post(tp, &profile.post);
                let full = Dynamics::from_profile(&profile);
                let mut limits = full.clone();
                limits.accel = [f64::INFINITY; 4];
                let mut floor = limits.clone();
                floor.max_feed = [1e12; 4];
                floor.feed_cap = f64::INFINITY;
                let (ef, el, ee) = (
                    estimate_gcode(&g, &floor),
                    estimate_gcode(&g, &limits),
                    estimate_gcode(&g, &full),
                );
                let moves = g
                    .lines()
                    .filter(|l| l.starts_with("G1") || l.starts_with("G2") || l.starts_with("G3"))
                    .count();
                let fmax = g
                    .lines()
                    .filter_map(|l| {
                        l.split_whitespace()
                            .find_map(|w| w.strip_prefix('F')?.parse::<f64>().ok())
                    })
                    .fold(0.0f64, f64::max);
                let name = format!(
                    "{}{}",
                    if jobs.len() > 1 {
                        format!("{setup}/")
                    } else {
                        String::new()
                    },
                    tp.name
                );
                println!(
                    "{:<44} {:>8.1} {:>8.1} {:>8.1} {:>6.2} | {:>7.1} {:>7.1} {:>6.1} {:>6} {:>7.3} {:>6.0}",
                    name,
                    ef.total_s,
                    el.total_s,
                    ee.total_s,
                    ee.total_s / ef.total_s.max(1e-9),
                    ee.feed_s,
                    ee.rapid_s,
                    ee.other_s,
                    moves,
                    ee.feed_mm / moves.max(1) as f64,
                    fmax
                );
            }
        }
    }
}
