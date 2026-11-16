//! Package the plugin for SolidCraft.
//!
//! ```text
//! pack <core.wasm> <out-dir>
//! ```
//!
//! Wraps the core module (built for wasm32-unknown-unknown, carrying wit-bindgen's component
//! type) as a WebAssembly component with no WASI adapter, writes `plugin.json` from the
//! operation registry, and zips both as `<out-dir>/fourcam.zip`. The unzipped package is
//! left in `<out-dir>/fourcam/` too.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn pack(core: &Path, out: &Path) -> Result<PathBuf, String> {
    let module = std::fs::read(core).map_err(|e| format!("{}: {e}", core.display()))?;
    let component = wit_component::ComponentEncoder::default()
        .module(&module)
        .map_err(|e| format!("not a plugin core module: {e:#}"))?
        .validate(true)
        .encode()
        .map_err(|e| format!("component encoding: {e:#}"))?;
    let manifest =
        serde_json::to_vec(&fourcam_solidcraft::manifest()).map_err(|e| e.to_string())?;
    if manifest.len() > 16 * 1024 {
        return Err(format!(
            "plugin.json is {} bytes; SolidCraft takes at most 16 KiB",
            manifest.len()
        ));
    }
    if component.len() > 16 * 1024 * 1024 {
        return Err(format!(
            "plugin.wasm is {} bytes; SolidCraft takes at most 16 MiB",
            component.len()
        ));
    }
    let dir = out.join(fourcam_solidcraft::PLUGIN_ID);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("plugin.json"), &manifest).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("plugin.wasm"), &component).map_err(|e| e.to_string())?;

    let zip_path = out.join(format!("{}.zip", fourcam_solidcraft::PLUGIN_ID));
    let file =
        std::fs::File::create(&zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in [("plugin.json", &manifest), ("plugin.wasm", &component)] {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    println!(
        "{} (plugin.json {} bytes, plugin.wasm {} bytes)",
        zip_path.display(),
        manifest.len(),
        component.len()
    );
    Ok(zip_path)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [core, out] = args.as_slice() else {
        eprintln!("usage: pack <core.wasm> <out-dir>");
        return ExitCode::from(2);
    };
    match pack(Path::new(core), Path::new(out)) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
