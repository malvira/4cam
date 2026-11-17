//! `4cam`: the command's name before the project was renamed to marcam. It runs the `marcam`
//! next to it with the same arguments.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let exe = std::env::current_exe()
        .map(|p| p.with_file_name(format!("marcam{}", std::env::consts::EXE_SUFFIX)));
    let status = exe.map_err(|e| e.to_string()).and_then(|exe| {
        Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .status()
            .map_err(|e| format!("{}: {e}", exe.display()))
    });
    match status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("4cam (now marcam): {e}");
            ExitCode::FAILURE
        }
    }
}
