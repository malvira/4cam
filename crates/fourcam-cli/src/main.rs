//! `4cam` command-line interface.

use fourcam::{
    limits_for, load_mesh, plan_with_mesh, post, render, verify_gcode, FeedMode, Job, Limits,
    Machine, Mesh,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
4cam - 4th-axis rotary CAM engine

USAGE:
  4cam run <job.json> [--machine <name|machine.json>] [--out <dir>]
      Plan the job, write one .nc file and one .png preview per program, and
      verify each program. Built-in machines: makera-z1 (default), linuxcnc.
  4cam verify <file.nc> [--plunge <mm/min>] [--safe <radius>]
      Check G-code for over-fast Z descents and low X/A rapids.
      Defaults: --plunge 60, --safe = highest Z reached by a G0 move.
  4cam toolpath <job.json>
      Print the machine-neutral toolpath IR as JSON.
  4cam revolve <job.json> <out.stl> [--segments <n>]
      Revolve the job's part.profile into a closed binary STL (default 720 facets).
  4cam mesh-info <file.stl|file.3mf>
      Print triangle count, bounding box and largest radius about X.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

struct Opts {
    positional: Vec<String>,
    flags: Vec<(String, String)>,
}

fn parse(args: &[String], known: &[&str]) -> Result<Opts, String> {
    let mut o = Opts {
        positional: Vec::new(),
        flags: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            if !known.contains(&name) {
                return Err(format!("unknown option --{name}\n\n{USAGE}"));
            }
            let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
            o.flags.push((name.to_string(), v.clone()));
        } else {
            o.positional.push(a.clone());
        }
    }
    Ok(o)
}

impl Opts {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn one_input(&self) -> Result<&str, String> {
        match self.positional.as_slice() {
            [p] => Ok(p),
            _ => Err(format!("expected exactly one input file\n\n{USAGE}")),
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn load_job(path: &str) -> Result<Job, String> {
    Job::from_json(&read(path)?).map_err(|e| format!("{path}: {e}"))
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some((cmd, rest)) = args.split_first() else {
        print!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match cmd.as_str() {
        "run" => cmd_run(&parse(rest, &["machine", "out"])?),
        "verify" => cmd_verify(&parse(rest, &["plunge", "safe"])?),
        "toolpath" => cmd_toolpath(&parse(rest, &[])?),
        "revolve" => cmd_revolve(&parse(rest, &["segments"])?),
        "mesh-info" => cmd_mesh_info(&parse(rest, &[])?),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn cmd_run(o: &Opts) -> Result<ExitCode, String> {
    let job_path = o.one_input()?;
    let job = load_job(job_path)?;
    let machine = match o.flag("machine") {
        Some(p) => match Machine::builtin(p) {
            Some(m) => m,
            None => Machine::from_json(&read(p)?).map_err(|e| format!("{p}: {e}"))?,
        },
        None => Machine::makera_z1(),
    };
    let out = PathBuf::from(o.flag("out").unwrap_or("."));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let tps = plan_job(&job, job_path)?;
    let mut bad = 0;
    for tp in &tps {
        let text = post(tp, &machine);
        let path = out.join(format!("{}.nc", tp.name));
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        let profile = job.part.profile.as_ref().map(|p| p.points.as_slice());
        let png = out.join(format!("{}.png", tp.name));
        std::fs::write(&png, render::preview_png(tp, job.stock.radius, profile))
            .map_err(|e| format!("{}: {e}", png.display()))?;
        let issues = verify_gcode(&text, &limits_for(tp, &machine));
        println!(
            "{}: {} lines, {} issue(s)",
            path.display(),
            text.lines().count(),
            issues.len()
        );
        for i in &issues {
            println!("  {i}");
        }
        bad += issues.len();
    }
    Ok(if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn highest_rapid_z(text: &str) -> Option<f64> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("G0 ") || l.trim_start().starts_with("G00 "))
        .flat_map(|l| l.split_whitespace())
        .filter_map(|w| w.strip_prefix('Z').and_then(|v| v.parse::<f64>().ok()))
        .reduce(f64::max)
}

fn cmd_verify(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let text = read(path)?;
    let num = |name: &str| -> Result<Option<f64>, String> {
        o.flag(name)
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| format!("--{name}: not a number: {v}"))
            })
            .transpose()
    };
    let plunge_feed = num("plunge")?.unwrap_or(60.0);
    let safe_r = match num("safe")? {
        Some(s) => s,
        None => highest_rapid_z(&text).ok_or("no G0 Z move found; pass --safe")?,
    };
    let inverse = text.lines().any(|l| {
        let code = l.split('(').next().unwrap_or("");
        code.split_whitespace().any(|w| w == "G93")
    });
    let limits = Limits {
        plunge_feed,
        safe_r,
        feed_mode: if inverse {
            FeedMode::InverseTime
        } else {
            FeedMode::DegreesAsLength
        },
    };
    let issues = verify_gcode(&text, &limits);
    println!(
        "{}: {} issue(s) (plunge limit {plunge_feed} mm/min, safe Z {safe_r})",
        Path::new(path).display(),
        issues.len()
    );
    for i in &issues {
        println!("  {i}");
    }
    Ok(if issues.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Plan a job, loading its mesh (if any) relative to the job file.
fn plan_job(job: &Job, job_path: &str) -> Result<Vec<fourcam::Toolpath>, String> {
    let dir = Path::new(job_path).parent().unwrap_or(Path::new("."));
    let mesh = load_mesh(job, dir).map_err(|e| e.to_string())?;
    plan_with_mesh(job, mesh.as_ref()).map_err(|e| e.to_string())
}

fn cmd_revolve(o: &Opts) -> Result<ExitCode, String> {
    let [job_path, out] = o.positional.as_slice() else {
        return Err(format!(
            "expected <job.json> <out.stl>

{USAGE}"
        ));
    };
    let job = load_job(job_path)?;
    let profile = job
        .part
        .profile
        .as_ref()
        .ok_or("the job has no part.profile")?;
    let segments = match o.flag("segments") {
        Some(v) => v
            .parse()
            .map_err(|_| format!("--segments: not a count: {v}"))?,
        None => fourcam::PROFILE_SEGMENTS,
    };
    let mesh = Mesh::revolve(profile, segments);
    std::fs::write(out, mesh.to_stl()).map_err(|e| format!("{out}: {e}"))?;
    println!("{out}: {} triangles", mesh.triangles.len());
    Ok(ExitCode::SUCCESS)
}

fn cmd_mesh_info(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let mesh = Mesh::load(Path::new(path)).map_err(|e| e.to_string())?;
    println!("{path}: {} triangles", mesh.triangles.len());
    if let Some((lo, hi)) = mesh.bounds() {
        println!(
            "  bounds X {:.3}..{:.3}  Y {:.3}..{:.3}  Z {:.3}..{:.3}",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
        );
        println!("  largest radius about X: {:.3}", mesh.max_radius());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_toolpath(o: &Opts) -> Result<ExitCode, String> {
    let job_path = o.one_input()?;
    let job = load_job(job_path)?;
    let tps = plan_job(&job, job_path)?;
    let json = serde_json_string(&tps)?;
    // ignore a closed pipe (e.g. `| head`)
    let _ = writeln!(std::io::stdout(), "{json}");
    Ok(ExitCode::SUCCESS)
}

fn serde_json_string(tps: &[fourcam::Toolpath]) -> Result<String, String> {
    fourcam::toolpath::to_json(tps).map_err(|e| e.to_string())
}
