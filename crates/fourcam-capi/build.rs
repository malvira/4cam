// Hands the target triple to examples/cc_run.rs, which compiles the C example for it.
fn main() {
    println!(
        "cargo:rustc-env=FOURCAM_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
}
