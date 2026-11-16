//! Plan a 4cam project from Rust and print its programs.
//!
//!     cargo run -p fourcam-embed-example -- project.json
use fourcam::{load_mesh, plan_with_mesh, post, profiles, project};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: fourcam-embed-example project.json")?;
    let dir = std::path::Path::new(&path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let text = std::fs::read_to_string(&path)?;
    let libraries = profiles::builtin_libraries();
    // one plannable job per setup; mesh models are read next to the project
    for (setup, job) in project::jobs_from_any_with(&text, &libraries, &|src| src.load(dir).ok())? {
        let mesh = load_mesh(&job, dir)?;
        let id = job
            .machine
            .clone()
            .unwrap_or_else(|| "makera-z1-4axis".into());
        let machine = profiles::builtin_machine(&id).ok_or(format!("no machine {id:?}"))?;
        for tp in plan_with_mesh(&job, mesh.as_ref())? {
            let gcode = post(&tp, &machine.post);
            println!("{setup}/{}: {} lines", tp.name, gcode.lines().count());
        }
    }
    Ok(())
}
