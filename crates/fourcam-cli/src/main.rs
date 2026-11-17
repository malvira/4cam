//! `4cam` command-line interface.

use fourcam::{
    gouge_check, limits_for, load_mesh, plan_with_mesh, post, render, sim, simulate_job,
    verify_gcode, FeedMode, Job, Limits, Machine, Mesh,
};
use std::io::Write;

mod profiles_cmd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
4cam - 4th-axis rotary CAM engine

USAGE:
  4cam run <job.json> [--machine <id|file|url>] [--out <dir>]
      Plan the job, write one .nc file and one .png preview per program, verify
      each program and check it against the machine's limits. The machine is
      --machine, else the job's \"machine\", else makera-z1-4axis. Built in:
      makera-z1-4axis, makera-carvera-4axis, makera-carvera-air-4axis,
      linuxcnc-4axis (see `4cam profiles list`).
  4cam upgrade <job.json> [--out <project.json>]
      Convert a 0.1 job to a 0.2 project (models, tools, setups, operations).
      run/simulate/toolpath accept either version.
  4cam faces <job-or-project.json> [--model <id>] [--crease <deg>]
      Face regions of each model (split at sharp edges) with their stable ids,
      for an operation's geometry selection (kind faces, with model and ids).
  4cam wizard knurl <job-or-project.json> [--params <p.json>] [--out <file>]
      Add turning, a diamond knurl on the body face and a V-bit front chamfer,
      derived from the model's faces (defaults: the Z1 knob).
  4cam ops list | show <type> | check <job-or-project.json>
  4cam ops defaults <type> <job-or-project.json> [--setup id] [--tool n] [--machine id]
      A new operation with every parameter filled from the setup's tools,
      spindle, machine limits and part faces.
      The operation registry: types, parameters (units, defaults, limits),
      accepted tools and geometry; check validates a file's operations.
  4cam profiles list|show|import|export|dir ...
      Machine profiles and tool libraries (`4cam profiles help`).
  4cam estimate <file.nc> [--machine <id|file|url>]
      Estimate the machining time of any G-code file: feeds as the machine's
      controller reads them (degrees-as-length, linear-only, inverse time), rapids,
      acceleration and corner slow-downs, dwells and tool changes. `run` prints the
      same per program, per operation and in total.
  4cam verify <file.nc> [--plunge <mm/min>] [--safe <radius>]
      Check G-code for over-fast Z descents and low X/A rapids.
      Defaults: --plunge 60, --safe = highest Z reached by a G0 move.
  4cam toolpath <job.json>
      Print the machine-neutral toolpath IR as JSON.
  4cam simulate <job.json> [--out <dir>] [--sim-res coarse|normal|fine|ultra]
                [--dx <mm>] [--na <cells>] [--gpu]
      Simulate material removal of all programs in order on a radial heightmap
      (default fine: 0.05 mm x 720 cells around; coarse 0.2 x 180, normal
      0.1 x 240, ultra 0.025 x 1440), compare with the part, and write a
      <program>_stock.png per program. Fails on rapids through stock or gouges.
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
            // `name?` in `known`: a switch without a value
            if known.contains(&format!("{name}?").as_str()) {
                o.flags.push((name.to_string(), "on".into()));
                continue;
            }
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

    fn has(&self, name: &str) -> bool {
        self.flag(name).is_some()
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

/// Read a job (0.1) or project (0.2; its setups are planned one after another as one job
/// each), resolving tool references against the built-in and imported libraries.
fn load_job(path: &str) -> Result<Job, String> {
    let reg = profiles_cmd::Registry::load();
    let mut jobs = fourcam::project::jobs_from_any(&read(path)?, &reg.libraries())
        .map_err(|e| format!("{path}: {e}"))?;
    if jobs.len() > 1 {
        eprintln!(
            "note: {path} has {} setups; planning the first ({})",
            jobs.len(),
            jobs[0].0
        );
    }
    if jobs.is_empty() {
        return Err(format!("{path}: no setups"));
    }
    Ok(jobs.swap_remove(0).1)
}

fn cmd_wizard_knurl(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let (mut p, _) = fourcam::project::Project::from_json_any(&read(path)?)
        .map_err(|e| format!("{path}: {e}"))?;
    let w: fourcam::wizards::KnurlWizard = match o.flag("params") {
        Some(f) => serde_json::from_str(&read(f)?).map_err(|e| format!("{f}: {e}"))?,
        None => Default::default(),
    };
    let dir = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let models = p.models.clone();
    let meshes = |id: &str| match &models.iter().find(|m| m.id == id)?.kind {
        fourcam::project::ModelKind::Mesh(src) => src.load(&dir).ok(),
        _ => None,
    };
    let res = w.run(&mut p, &meshes)?;
    for n in &res.notes {
        eprintln!("note: {n}");
    }
    eprintln!(
        "added operations {} and outputs {}",
        res.operations.join(", "),
        res.outputs.join(", ")
    );
    let text = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?
        + "
";
    match o.flag("out") {
        Some(out) => std::fs::write(out, text).map_err(|e| format!("{out}: {e}"))?,
        None => print!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_faces(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let (p, _) = fourcam::project::Project::from_json_any(&read(path)?)
        .map_err(|e| format!("{path}: {e}"))?;
    let dir = Path::new(path).parent().unwrap_or(Path::new("."));
    let crease: f64 = o.flag("crease").map_or(Ok(30.0), |v| {
        v.parse().map_err(|_| format!("--crease: {v}"))
    })?;
    for m in &p.models {
        if o.flag("model").is_some_and(|want| want != m.id) {
            continue;
        }
        let mesh = match &m.kind {
            fourcam::project::ModelKind::Profile(prof) => Mesh::revolve(prof, 360),
            fourcam::project::ModelKind::Mesh(src) => src.load(dir).map_err(|e| e.to_string())?,
        };
        let (regions, _) = fourcam::geometry::face_regions(&mesh, crease);
        println!("model {} ({} faces):", m.id, regions.len());
        for r in &regions {
            let a = r.a.map_or("all round".to_string(), |a| {
                format!("A {:.1}..{:.1}", a[0], a[1])
            });
            println!(
                "  {}  {:<9} X {:>8.3}..{:<8.3} R {:.3}..{:.3}  {a}  area {:.1} mm2",
                r.id, r.shape, r.x[0], r.x[1], r.r[0], r.r[1], r.area
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_ops(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        None | Some("list") => {
            for t in fourcam::ops::registry() {
                println!(
                    "{:<16} {:<10} {} (tools: {}; geometry: {})",
                    t.kind,
                    t.group,
                    t.title,
                    t.tool_shapes.join("/"),
                    t.geometry.join("/")
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("show") => {
            let kind = args.get(1).ok_or("ops show <type>")?;
            let t = fourcam::ops::find(kind).ok_or(format!("unknown operation type {kind:?}"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("defaults") => {
            let kind = args.get(1).ok_or(
                "ops defaults <type> <job-or-project.json> [--setup id] [--tool n] [--machine id]",
            )?;
            let o = parse(&args[2..], &["setup", "tool", "machine"])?;
            let path = o.one_input()?;
            let (p, _) = fourcam::project::Project::from_json_any(&read(path)?)
                .map_err(|e| format!("{path}: {e}"))?;
            let reg = profiles_cmd::Registry::load();
            let machine = match o.flag("machine") {
                Some(m) => Some(profiles_cmd::resolve_machine(&reg, m)?),
                None => None,
            };
            let tool = o
                .flag("tool")
                .map(|t| t.parse::<u32>().map_err(|_| format!("--tool: {t}")))
                .transpose()?;
            let setup = o
                .flag("setup")
                .map(str::to_string)
                .or_else(|| p.setups.first().map(|s| s.id.clone()))
                .ok_or("no setups")?;
            let dir = Path::new(path)
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            let models = p.models.clone();
            let meshes = |id: &str| match &models.iter().find(|m| m.id == id)?.kind {
                fourcam::project::ModelKind::Mesh(src) => src.load(&dir).ok(),
                _ => None,
            };
            let v = p.op_defaults(
                kind,
                &setup,
                tool,
                &[],
                &reg.libraries(),
                machine.as_ref(),
                &meshes,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            let path = args.get(1).ok_or("ops check <job-or-project.json>")?;
            let (p, _) = fourcam::project::Project::from_json_any(&read(path)?)
                .map_err(|e| format!("{path}: {e}"))?;
            let errs = p.check_operations();
            for e in &errs {
                println!("{e}");
            }
            println!(
                "{path}: {} operation(s) checked, {} problem(s)",
                p.setups.iter().map(|s| s.operations.len()).sum::<usize>(),
                errs.len()
            );
            Ok(if errs.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Some(other) => Err(format!(
            "unknown ops command {other:?} (list, show <type>, check <file>)"
        )),
    }
}

fn cmd_upgrade(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let (p, upgraded) = fourcam::project::Project::from_json_any(&read(path)?)
        .map_err(|e| format!("{path}: {e}"))?;
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .trim_end_matches(".job");
    let mut p = p;
    if upgraded {
        p.name = name.to_string();
    }
    let text = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?
        + "
";
    match o.flag("out") {
        Some(out) => {
            std::fs::write(out, text).map_err(|e| format!("{out}: {e}"))?;
            println!(
                "wrote {out} (schema 0.2{})",
                if upgraded { ", upgraded from 0.1" } else { "" }
            );
        }
        None => print!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some((cmd, rest)) = args.split_first() else {
        print!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match cmd.as_str() {
        "run" => cmd_run(&parse(rest, &["machine", "out"])?),
        "profiles" => {
            if rest.first().is_some_and(|a| a == "help" || a == "--help") {
                print!("{}", profiles_cmd::USAGE);
                return Ok(ExitCode::SUCCESS);
            }
            profiles_cmd::cmd(rest)?;
            Ok(ExitCode::SUCCESS)
        }
        "verify" => cmd_verify(&parse(rest, &["plunge", "safe"])?),
        "estimate" => cmd_estimate(&parse(rest, &["machine"])?),
        "toolpath" => cmd_toolpath(&parse(rest, &[])?),
        "upgrade" => cmd_upgrade(&parse(rest, &["out"])?),
        "ops" => cmd_ops(rest),
        "faces" => cmd_faces(&parse(rest, &["model", "crease"])?),
        "wizard" => {
            let (name, more) = rest
                .split_first()
                .ok_or("wizard knurl <file> [--params p.json] [--out file]")?;
            if name != "knurl" {
                return Err(format!("unknown wizard {name:?} (available: knurl)"));
            }
            cmd_wizard_knurl(&parse(more, &["params", "out"])?)
        }
        "mesh-info" => cmd_mesh_info(&parse(rest, &[])?),
        "simulate" => cmd_simulate(&parse(rest, &["out", "dx", "na", "sim-res", "gpu?"])?),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn cmd_run(o: &Opts) -> Result<ExitCode, String> {
    let job_path = o.one_input()?;
    let reg = profiles_cmd::Registry::load();
    let dir = Path::new(job_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let meshes = |src: &fourcam::job::MeshSource| src.load(&dir).ok();
    let jobs = fourcam::project::jobs_from_any_with(&read(job_path)?, &reg.libraries(), &meshes)
        .map_err(|e| format!("{job_path}: {e}"))?;
    let mut bad = 0;
    let mut total = 0.0;
    for (setup, job) in &jobs {
        if jobs.len() > 1 {
            println!("== setup {setup}");
        }
        let (b, t) = run_setup(o, job_path, job, &reg)?;
        bad += b;
        total += t;
        if jobs.len() > 1 {
            println!("setup {setup}: {}", fourcam::estimate::format_duration(t));
        }
    }
    println!(
        "estimated machining time: {}",
        fourcam::estimate::format_duration(total)
    );
    Ok(if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Plan, post, verify and check one setup's job; returns the number of problems.
fn run_setup(
    o: &Opts,
    job_path: &str,
    job: &Job,
    reg: &profiles_cmd::Registry,
) -> Result<(usize, f64), String> {
    let job = job.clone();
    let mut seconds = 0.0;
    let wanted = o
        .flag("machine")
        .map(str::to_string)
        .or_else(|| job.machine.clone())
        .unwrap_or_else(|| "makera-z1-4axis".into());
    let profile = profiles_cmd::resolve_machine(reg, &wanted)?;
    println!("machine: {} ({})", profile.id, profile.name);
    let machine: Machine = profile.post.clone();
    let out = PathBuf::from(o.flag("out").unwrap_or("."));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (tps, mesh) = plan_job_with_mesh(&job, job_path)?;
    let gouges = gouge_check(&job, &tps, mesh.as_ref()).map_err(|e| e.to_string())?;
    let collisions = fourcam::collision_check(&job, &tps, &machine).map_err(|e| e.to_string())?;
    let mut bad = 0;
    for i in fourcam::profiles::check_limits(&job, &tps, &profile) {
        let at = i.program.map(|p| format!("{p}: ")).unwrap_or_default();
        println!("{}: {at}{}", i.severity, i.message);
        if i.severity == "error" {
            bad += 1;
        }
    }
    for tp in &tps {
        let text = post(tp, &machine);
        let path = out.join(format!("{}.nc", tp.name));
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        let part_profile = job.part.profile.as_ref().map(|p| p.points.as_slice());
        let png = out.join(format!("{}.png", tp.name));
        let image = match &job.stock_box {
            Some(sb) if render::is_xyz(tp) => render::preview_png3(tp, sb),
            _ => render::preview_png(tp, job.stock.radius, part_profile),
        };
        std::fs::write(&png, image).map_err(|e| format!("{}: {e}", png.display()))?;
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
        let time = fourcam::estimate::estimate_toolpath(tp, &profile);
        let e = &time.estimate;
        let fmt = fourcam::estimate::format_duration;
        println!(
            "  time: {} (cutting {}, rapids {}, tool changes/dwells {})",
            fmt(e.total_s),
            fmt(e.feed_s),
            fmt(e.rapid_s),
            fmt(e.other_s)
        );
        for (id, kind, s) in &time.ops {
            println!("    op {} {kind}: {}", id + 1, fmt(*s));
        }
        seconds += e.total_s;
        if let Some(c) = collisions.iter().find(|c| c.program == tp.name) {
            if c.count > 0 {
                println!(
                    "  HOLDER/SHANK COLLISION: {} move(s), deepest {:.3} mm",
                    c.count, c.max_depth
                );
                for k in c.collisions.iter().take(5) {
                    println!(
                        "    line {}: T{} {} {:.3} mm into the stock{} (tip X{:.3} Y{:.3} Z{:.3} A{:.3})",
                        k.line.unwrap_or(0),
                        k.tool,
                        k.contact,
                        k.depth,
                        if k.rapid { " on a rapid" } else { "" },
                        k.tip[0],
                        k.tip[1],
                        k.tip[2],
                        k.a
                    );
                }
                bad += 1;
            } else {
                println!("  holder/shank check: clear");
            }
        }
        if let Some(g) = gouges.iter().find(|g| g.program == tp.name) {
            if g.moves_checked > 0 {
                println!(
                    "  gouge check: {} cut moves, deepest {:.4} mm into the part",
                    g.moves_checked, g.max_depth
                );
                if g.max_depth > GOUGE_TOL {
                    let (x, a) = g.at.unwrap_or_default();
                    println!("  GOUGE at X{x:.3} A{a:.3}");
                    bad += 1;
                }
            }
        }
    }
    Ok((bad, seconds))
}

fn cmd_estimate(o: &Opts) -> Result<ExitCode, String> {
    let path = o.one_input()?;
    let text = read(path)?;
    let reg = profiles_cmd::Registry::load();
    let profile =
        profiles_cmd::resolve_machine(&reg, o.flag("machine").unwrap_or("makera-z1-4axis"))?;
    let e = fourcam::estimate::estimate_gcode(
        &text,
        &fourcam::estimate::Dynamics::from_profile(&profile),
    );
    let fmt = fourcam::estimate::format_duration;
    println!("machine: {} ({})", profile.id, profile.name);
    println!(
        "{path}: {} (cutting {} over {:.0} mm, rapids {} over {:.0} mm, tool changes/dwells {})",
        fmt(e.total_s),
        fmt(e.feed_s),
        e.feed_mm,
        fmt(e.rapid_s),
        e.rapid_mm,
        fmt(e.other_s)
    );
    for n in &e.notes {
        println!("  note: {n}");
    }
    Ok(ExitCode::SUCCESS)
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
        surface_radius_pad: 0.0,
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

/// Deepest cut into the part that `run` and `simulate` accept (mm).
const GOUGE_TOL: f64 = 0.02;

/// Plan a job, loading its mesh (if any) relative to the job file.
fn plan_job(job: &Job, job_path: &str) -> Result<Vec<fourcam::Toolpath>, String> {
    plan_job_with_mesh(job, job_path).map(|(t, _)| t)
}

fn plan_job_with_mesh(
    job: &Job,
    job_path: &str,
) -> Result<(Vec<fourcam::Toolpath>, Option<Mesh>), String> {
    let dir = Path::new(job_path).parent().unwrap_or(Path::new("."));
    let mesh = load_mesh(job, dir).map_err(|e| e.to_string())?;
    let tps = plan_with_mesh(job, mesh.as_ref()).map_err(|e| e.to_string())?;
    Ok((tps, mesh))
}

fn cmd_simulate(o: &Opts) -> Result<ExitCode, String> {
    let job_path = o.one_input()?;
    let job = load_job(job_path)?;
    let num = |name: &str, default: f64| -> Result<f64, String> {
        o.flag(name).map_or(Ok(default), |v| {
            v.parse()
                .map_err(|_| format!("--{name}: not a number: {v}"))
        })
    };
    // --sim-res coarse|normal|fine|ultra sets both; --dx / --na override (default: fine)
    let preset = match o.flag("sim-res") {
        Some(name) => sim::Resolution::preset(name).ok_or(format!(
            "--sim-res: {name:?} is not coarse, normal, fine or ultra"
        ))?,
        None => sim::Resolution::preset("fine").expect("built in"),
    };
    let dx = num("dx", preset.dx)?;
    let na = num("na", preset.na as f64)? as usize;
    let out = PathBuf::from(o.flag("out").unwrap_or("."));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (tps, mesh) = plan_job_with_mesh(&job, job_path)?;
    let res =
        simulate_job(&job, &tps, mesh.as_ref(), dx, na, GOUGE_TOL).map_err(|e| e.to_string())?;
    // replay to render the stock after each program
    let mut stock = sim::RadialMap::filled(res.stock.grid, job.stock.radius);
    let cutters = |t: u32| {
        job.tool(t)
            .and_then(|d| fourcam::cutter_for(t, d.shape).ok())
    };
    let mut bad = false;
    // --gpu (built with the `gpu` feature): the stock for the previews on the GPU
    #[cfg(feature = "gpu")]
    let gpu = o.has("gpu").then(fourcam_gpu::Gpu::new).flatten();
    #[cfg(not(feature = "gpu"))]
    if o.has("gpu") {
        return Err("this 4cam was built without GPU support (cargo feature `gpu`)".into());
    }
    for (((tp, prog), (name, st, dev)), coll) in tps
        .iter()
        .zip(&job.programs)
        .zip(&res.programs)
        .zip(&res.collisions)
    {
        #[cfg(feature = "gpu")]
        {
            let used = fourcam_gpu::simulate_or_cpu(
                gpu.as_ref(),
                &mut stock,
                std::slice::from_ref(tp),
                &cutters,
                (dx * 2.0).min(0.1),
            );
            if o.has("gpu") {
                println!("{name}: stock on {used:?}");
            }
        }
        #[cfg(not(feature = "gpu"))]
        sim::simulate(&mut stock, tp, &cutters, (dx * 2.0).min(0.1));
        // knurls and chamfers cut below the turned profile by design
        let shapes_part = prog.operations.iter().any(|o| {
            matches!(
                o,
                fourcam::job::Operation::HelicalKnurl(_) | fourcam::job::Operation::FlankChamfer(_)
            )
        });
        let png = out.join(format!("{name}_stock.png"));
        let title = format!("{name} - simulated stock (grey: part)");
        std::fs::write(
            &png,
            sim::stock_png(&title, &stock, &res.part.center, job.stock.radius),
        )
        .map_err(|e| format!("{}: {e}", png.display()))?;
        println!(
            "{name}: {} tool positions, {} cells cut, {} rapid(s) through stock",
            st.samples,
            st.cells_cut,
            st.rapid_hits.len()
        );
        println!(
            "  material left on the part: max {:.3} mm, mean {:.3} mm; deepest gouge {:.3} mm ({} cells over {GOUGE_TOL} mm)",
            dev.max_left, dev.mean_left, dev.max_gouge, dev.gouge_cells
        );
        if st.samples == 0
            && tp
                .moves
                .iter()
                .any(|m| matches!(m, fourcam::Move::Cut { .. }))
        {
            println!(
                "  (not simulated: its tools have no full shape, e.g. a V-bit without a diameter)"
            );
        }
        if shapes_part && dev.gouge_cells > 0 {
            println!(
                "  (knurl/chamfer: cuts below the turned profile on purpose, so these are not counted as gouges)"
            );
        }
        if coll.count > 0 {
            println!(
                "  HOLDER/SHANK COLLISION: {} move(s), deepest {:.3} mm (first at move {})",
                coll.count, coll.max_depth, coll.collisions[0].move_index
            );
        }
        println!("  preview: {}", png.display());
        bad |= !st.rapid_hits.is_empty() || (dev.gouge_cells > 0 && !shapes_part) || coll.count > 0;
    }
    Ok(if bad {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
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
