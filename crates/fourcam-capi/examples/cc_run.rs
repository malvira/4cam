//! Compile a C program against the 4cam C library with the platform's C compiler (found the
//! way build scripts find it: MSVC, gcc or clang), next to a copy of the library.
//!
//! ```text
//! cc_run <program.c> <dir with the built library> <out dir>
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [src, lib_dir, out] = args.as_slice() else {
        eprintln!("usage: cc_run <program.c> <library dir> <out dir>");
        return ExitCode::from(2);
    };
    let (src, lib_dir, out) = (Path::new(src), Path::new(lib_dir), PathBuf::from(out));
    std::fs::create_dir_all(&out).expect("out dir");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let target = env!("FOURCAM_TARGET");
    let compiler = cc::Build::new()
        .target(target)
        .host(target)
        .opt_level(0)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .include(&include)
        .get_compiler();
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("prog");
    let mut cmd: Command = compiler.to_command();
    let exe;
    if compiler.is_like_msvc() {
        exe = out.join(format!("{stem}.exe"));
        // the program finds fourcam_c.dll next to itself
        std::fs::copy(lib_dir.join("fourcam_c.dll"), out.join("fourcam_c.dll")).expect("copy dll");
        cmd.arg(src)
            .arg(format!("/Fe{}", exe.display()))
            .arg(format!("/Fo{}\\", out.display()))
            .arg(lib_dir.join("fourcam_c.dll.lib"));
    } else {
        exe = out.join(stem);
        cmd.arg(src)
            .arg("-o")
            .arg(&exe)
            .arg(format!("-L{}", lib_dir.display()))
            .arg("-lfourcam_c")
            .arg(format!(
                "-Wl,-rpath,{}",
                std::fs::canonicalize(lib_dir).expect("lib dir").display()
            ));
    }
    let status = cmd.status().expect("run the C compiler");
    if !status.success() {
        eprintln!("C compiler failed: {status}");
        return ExitCode::FAILURE;
    }
    println!("{}", exe.display());
    ExitCode::SUCCESS
}
