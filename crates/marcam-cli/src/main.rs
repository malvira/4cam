//! `marcam`: the command line (the program is the library's `main`, so the STEP-enabled
//! `marcam-step` can run it too).

fn main() -> std::process::ExitCode {
    marcam_cli::main()
}
