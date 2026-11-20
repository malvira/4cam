//! The WebAssembly component side: SolidCraft's `craft:plugin@0.1.0` world
//! (`wit/plugin.wit`, copied unchanged from SolidCraft's `crates/craft-plugin/wit`).

use crate::{
    describe, operation_call, plan_project, run_entry, Error, Outcome, DESCRIBE, GRANT,
    MESH_COMMAND, PLAN_PROJECT, PROJECT_GRANT,
};

wit_bindgen::generate!({ world: "command-plugin", path: "wit" });

use craft::plugin::types::{FieldFailure, PluginError};
use craft::plugin::{commands, control, files, log};

struct Plugin;

fn to_plugin_error(e: Error) -> PluginError {
    match e {
        Error::Input { field, reason } => PluginError::InvalidInput(FieldFailure { field, reason }),
        // No other variant fits an engine refusal; `trap` carries the reason as text.
        Error::Engine(reason) => PluginError::Trap(format!("marcam: {reason}")),
    }
}

/// Write an entry's files to the output folder; its result as JSON.
fn finish(entry: &str, out: Outcome) -> Result<String, PluginError> {
    control::progress(0.95, "writing files");
    for f in &out.files {
        files::write_file(GRANT, &f.path, &f.data)?;
    }
    log::log(
        log::Level::Info,
        &format!("marcam {entry}: wrote {} file(s)", out.files.len()),
    );
    Ok(out.result.to_string())
}

impl exports::craft::plugin::entry::Guest for Plugin {
    fn execute(entry: String, params: String) -> Result<String, PluginError> {
        let params: serde_json::Value = serde_json::from_str(&params).map_err(|e| {
            PluginError::InvalidInput(FieldFailure {
                field: "params".into(),
                reason: e.to_string(),
            })
        })?;
        // the operations and their fields: nothing read or written
        if entry == DESCRIBE {
            return describe(&params)
                .map(|v| v.to_string())
                .map_err(to_plugin_error);
        }
        // a whole project from the project folder
        if entry == PLAN_PROJECT {
            control::progress(0.05, "reading the project");
            let read = |path: &str| {
                files::read_file(PROJECT_GRANT, path).map_err(|e| Error::Input {
                    field: "project".into(),
                    reason: format!("{path}: {e:?}"),
                })
            };
            let out = plan_project(&params, &read).map_err(to_plugin_error)?;
            return finish(&entry, out);
        }
        // Check the inputs before asking SolidCraft for anything.
        operation_call(&entry, &params).map_err(to_plugin_error)?;
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
        finish(&entry, out)
    }
}

export!(Plugin);
