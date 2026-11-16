//! The WebAssembly component side: SolidCraft's `craft:plugin@0.1.0` world
//! (`wit/plugin.wit`, copied unchanged from SolidCraft's `crates/craft-plugin/wit`).

use crate::{run_entry, Error, GRANT, MESH_COMMAND};

wit_bindgen::generate!({ world: "command-plugin", path: "wit" });

use craft::plugin::types::{FieldFailure, PluginError};
use craft::plugin::{commands, control, files, log};

struct Plugin;

fn to_plugin_error(e: Error) -> PluginError {
    match e {
        Error::Input { field, reason } => PluginError::InvalidInput(FieldFailure { field, reason }),
        // No other variant fits an engine refusal; `trap` carries the reason as text.
        Error::Engine(reason) => PluginError::Trap(format!("4cam: {reason}")),
    }
}

impl exports::craft::plugin::entry::Guest for Plugin {
    fn execute(entry: String, params: String) -> Result<String, PluginError> {
        let params: serde_json::Value = serde_json::from_str(&params).map_err(|e| {
            PluginError::InvalidInput(FieldFailure {
                field: "params".into(),
                reason: e.to_string(),
            })
        })?;
        // Check the inputs before asking SolidCraft for anything.
        if crate::entry_kind(&entry).is_none() {
            return Err(to_plugin_error(Error::Input {
                field: "entry".into(),
                reason: format!("no 4cam operation {entry:?}"),
            }));
        }
        let request = crate::mesh_request(&params).map_err(to_plugin_error)?;
        control::progress(0.05, "meshing the body");
        let mesh = commands::run(MESH_COMMAND, &request.to_string())?;
        let mesh: serde_json::Value = serde_json::from_str(&mesh)
            .map_err(|e| PluginError::Trap(format!("{MESH_COMMAND}: {e}")))?;
        if control::cancelled() {
            return Err(PluginError::Cancelled);
        }
        control::progress(0.2, "planning");
        let out = run_entry(&entry, &params, &mesh).map_err(to_plugin_error)?;
        control::progress(0.95, "writing files");
        for f in &out.files {
            files::write_file(GRANT, &f.path, &f.data)?;
        }
        log::log(
            log::Level::Info,
            &format!("4cam {entry}: wrote {} file(s)", out.files.len()),
        );
        Ok(out.result.to_string())
    }
}

export!(Plugin);
