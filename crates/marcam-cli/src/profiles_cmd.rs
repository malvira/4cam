//! `marcam profiles ...`: the profile registry on disk (built-in + the user's profile folder),
//! importing from files or URLs, exporting.

use marcam::profiles::{
    self, builtin_libraries, builtin_machines, import, Imported, MachineProfile, ToolLibrary,
};
use std::path::{Path, PathBuf};

/// The user's profile folder: `$MARCAM_PROFILES`, else `%APPDATA%\marcam\profiles` on Windows,
/// else `$XDG_CONFIG_HOME/marcam/profiles` or `~/.config/marcam/profiles`. From before the
/// rename (4cam -> marcam), `$FOURCAM_PROFILES` and an existing `4cam` folder still count when
/// there is no `marcam` one.
pub fn user_dir() -> PathBuf {
    for var in ["MARCAM_PROFILES", "FOURCAM_PROFILES"] {
        if let Some(d) = std::env::var_os(var) {
            return PathBuf::from(d);
        }
    }
    let dir = default_dir("marcam");
    let legacy = default_dir("4cam");
    if !dir.exists() && legacy.exists() {
        return legacy;
    }
    dir
}

fn default_dir(app: &str) -> PathBuf {
    if let Some(d) = std::env::var_os("APPDATA") {
        return PathBuf::from(d).join(app).join("profiles");
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(d).join(app).join("profiles");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join(app).join("profiles")
}

/// Everything known: built-ins first, then the user's folder (which may override an id).
pub struct Registry {
    pub machines: Vec<(MachineProfile, String)>,
    pub libraries: Vec<(ToolLibrary, String)>,
    pub problems: Vec<String>,
}

impl Registry {
    pub fn load() -> Registry {
        let mut r = Registry {
            machines: builtin_machines()
                .into_iter()
                .map(|m| (m, "built-in".into()))
                .collect(),
            libraries: builtin_libraries()
                .into_iter()
                .map(|l| (l, "built-in".into()))
                .collect(),
            problems: Vec::new(),
        };
        let dir = user_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return r;
        };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let origin = p.display().to_string();
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            if name.ends_with(".machine.json") {
                match MachineProfile::from_json(&text) {
                    Ok(m) => {
                        r.machines.retain(|(x, _)| x.id != m.id);
                        r.machines.push((m, origin));
                    }
                    Err(e) => r.problems.push(format!("{origin}: {e}")),
                }
            } else if name.ends_with(".tools.json") {
                match ToolLibrary::from_json(&text) {
                    Ok(l) => {
                        r.libraries.retain(|(x, _)| x.id != l.id);
                        r.libraries.push((l, origin));
                    }
                    Err(e) => r.problems.push(format!("{origin}: {e}")),
                }
            }
        }
        r
    }

    pub fn libraries(&self) -> Vec<ToolLibrary> {
        self.libraries.iter().map(|(l, _)| l.clone()).collect()
    }

    pub fn machine(&self, id: &str) -> Option<&MachineProfile> {
        let id = profiles::canonical_machine_id(id);
        self.machines.iter().map(|(m, _)| m).find(|m| m.id == id)
    }
}

/// Read a file, or fetch an http(s) URL.
pub fn fetch(src: &str) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        let resp = ureq::get(src)
            .header("User-Agent", "marcam")
            .call()
            .map_err(|e| format!("{src}: {e}"))?;
        let mut body = resp.into_body();
        body.with_config()
            .limit(256 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("{src}: {e}"))
    } else {
        std::fs::read(src).map_err(|e| format!("{src}: {e}"))
    }
}

/// Machine from `--machine`: a registry id/alias, or a file/URL in any importable format.
pub fn resolve_machine(reg: &Registry, arg: &str) -> Result<MachineProfile, String> {
    if let Some(m) = reg.machine(arg) {
        return Ok(m.clone());
    }
    if Path::new(arg).exists() || arg.starts_with("http") {
        let bytes = fetch(arg)?;
        return match import(&bytes, arg)?.item {
            Imported::Machine(m) => Ok(m),
            Imported::Tools(_) => Err(format!("{arg} is a tool library, not a machine")),
        };
    }
    Err(format!(
        "unknown machine {arg:?}; known: {}",
        reg.machines
            .iter()
            .map(|(m, _)| m.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let (sub, rest) = args.split_first().ok_or(USAGE)?;
    let mut positional = Vec::new();
    let mut id_override = None;
    let mut out = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--id" => id_override = Some(it.next().ok_or("--id needs a value")?.clone()),
            "--out" => out = Some(it.next().ok_or("--out needs a value")?.clone()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n\n{USAGE}")),
            s => positional.push(s.to_string()),
        }
    }
    let reg = Registry::load();
    for p in &reg.problems {
        eprintln!("warning: {p}");
    }
    match sub.as_str() {
        "dir" => println!("{}", user_dir().display()),
        "list" => {
            println!("machines:");
            for (m, origin) in &reg.machines {
                println!("  {:<28} {}  [{origin}]", m.id, m.name);
            }
            println!("tool libraries:");
            for (l, origin) in &reg.libraries {
                println!(
                    "  {:<28} {} ({} tools)  [{origin}]",
                    l.id,
                    l.name,
                    l.tools.len()
                );
            }
            println!("user profile folder: {}", user_dir().display());
        }
        "show" | "export" => {
            let id = positional
                .first()
                .ok_or(format!("{sub}: which profile?\n\n{USAGE}"))?;
            let text = if let Some(m) = reg.machine(id) {
                serde_json::to_string_pretty(m)
            } else if let Some((l, _)) = reg.libraries.iter().find(|(l, _)| &l.id == id) {
                serde_json::to_string_pretty(l)
            } else {
                return Err(format!(
                    "no profile or tool library {id:?} (see `marcam profiles list`)"
                ));
            }
            .map_err(|e| e.to_string())?;
            match (&out, sub.as_str()) {
                (Some(path), _) => {
                    std::fs::write(path, text + "\n").map_err(|e| format!("{path}: {e}"))?;
                    println!("wrote {path}");
                }
                _ => println!("{text}"),
            }
        }
        "import" => {
            let src = positional
                .first()
                .ok_or(format!("import: which file or URL?\n\n{USAGE}"))?;
            let bytes = fetch(src)?;
            let imp = import(&bytes, src)?;
            for n in &imp.notes {
                println!("note: {n}");
            }
            let dir = match &out {
                Some(d) => PathBuf::from(d),
                None => user_dir(),
            };
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let (path, text, what) = match imp.item {
                Imported::Machine(mut m) => {
                    if let Some(id) = &id_override {
                        m.id = id.clone();
                    }
                    m.sources.insert(
                        0,
                        profiles::Source {
                            url: src.clone(),
                            note: "imported by marcam".into(),
                        },
                    );
                    m.validate()?;
                    let what = format!("machine {} ({})", m.id, m.name);
                    (
                        dir.join(format!("{}.machine.json", m.id)),
                        serde_json::to_string_pretty(&m),
                        what,
                    )
                }
                Imported::Tools(mut l) => {
                    if let Some(id) = &id_override {
                        l.id = id.clone();
                    }
                    l.sources.insert(
                        0,
                        profiles::Source {
                            url: src.clone(),
                            note: "imported by marcam".into(),
                        },
                    );
                    l.validate()?;
                    let what = format!("tool library {} ({} tools)", l.id, l.tools.len());
                    (
                        dir.join(format!("{}.tools.json", l.id)),
                        serde_json::to_string_pretty(&l),
                        what,
                    )
                }
            };
            let text = text.map_err(|e| e.to_string())?;
            std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
            println!("imported {what} -> {}", path.display());
        }
        other => return Err(format!("unknown profiles command {other:?}\n\n{USAGE}")),
    }
    Ok(())
}

pub const USAGE: &str = "\
marcam profiles list
    Built-in and imported machine profiles and tool libraries.
marcam profiles show <id>
    Print a machine profile or tool library as JSON.
marcam profiles import <file-or-url> [--id <new-id>] [--out <dir>]
    Read a marcam profile, a Fusion 360 tool library (.tools/.json) or a Fusion 360
    machine (.mch, or a .f3d that embeds one, as in Makera's Fusion-Profiles.zip)
    and save it as marcam JSON in the user profile folder.
marcam profiles export <id> [--out <file>]
    Write a profile or library as marcam JSON.
marcam profiles dir
    Print the user profile folder ($MARCAM_PROFILES overrides it).
";
