//! `4cam` command-line interface.

use fourcam::{limits_for, plan, post, verify_gcode, FeedMode, Job, Limits, Machine};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
4cam - 4th-axis rotary CAM engine

USAGE:
  4cam run <job.json> [--machine <machine.json>] [--out <dir>]
      Plan the job, write one .nc file per program and verify each one.
  4cam verify <file.nc> [--plunge <mm/min>] [--safe <radius>]
      Check G-code for over-fast Z descents and low X/A rapids.
      Defaults: --plunge 60, --safe = highest Z reached by a G0 move.
  4cam toolpath <job.json>
      Print the machine-neutral toolpath IR as JSON.
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
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn cmd_run(o: &Opts) -> Result<ExitCode, String> {
    let job = load_job(o.one_input()?)?;
    let machine = match o.flag("machine") {
        Some(p) => Machine::from_json(&read(p)?).map_err(|e| format!("{p}: {e}"))?,
        None => Machine::makera_z1(),
    };
    let out = PathBuf::from(o.flag("out").unwrap_or("."));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let tps = plan(&job).map_err(|e| e.to_string())?;
    let mut bad = 0;
    for tp in &tps {
        let text = post(tp, &machine);
        let path = out.join(format!("{}.nc", tp.name));
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
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
    let limits = Limits {
        plunge_feed,
        safe_r,
        feed_mode: FeedMode::DegreesAsLength,
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

fn cmd_toolpath(o: &Opts) -> Result<ExitCode, String> {
    let job = load_job(o.one_input()?)?;
    let tps = plan(&job).map_err(|e| e.to_string())?;
    let json = serde_json_string(&tps)?;
    // ignore a closed pipe (e.g. `| head`)
    let _ = writeln!(std::io::stdout(), "{json}");
    Ok(ExitCode::SUCCESS)
}

fn serde_json_string(tps: &[fourcam::Toolpath]) -> Result<String, String> {
    fourcam::toolpath::to_json(tps).map_err(|e| e.to_string())
}
