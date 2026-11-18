//! `marcam-step`: the `marcam` command line, reading STEP (.step/.stp) parts too. It is not in
//! the published `marcam` (SolidCraft's kernel comes from git): build it from this repository,
//! `cargo install --path crates/marcam-step`.

fn main() -> std::process::ExitCode {
    marcam_step::register();
    marcam_cli::main()
}
