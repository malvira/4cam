#!/bin/sh
# Everything the project promises, in one go: format, lints, Rust tests, the WebAssembly
# package (tested under Node), the Python wheel (tested in a fresh venv) and the web app
# (headless Chrome/Edge; needs network for three.js).
set -e
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p fourcam-cli --features gpu --all-targets -- -D warnings
cargo test
# the GPU prototype against the CPU (skips, saying so, without a hardware GPU)
cargo test -p fourcam-gpu -- --nocapture
# the SolidCraft plugin: tests, the guest linted and packed, and the real host when
# SOLIDCRAFT_CLI names a solidcraft-cli (skipped otherwise, saying so)
cargo build -p fourcam-cli
cargo test -p fourcam-solidcraft
cargo clippy -p fourcam-solidcraft --lib --target wasm32-unknown-unknown -- -D warnings
sh scripts/build-solidcraft-plugin.sh
sh scripts/e2e-solidcraft.sh
# the C ABI: its library, the C example and (with a .NET SDK) the C# example
sh scripts/test-capi.sh
# the embedding examples in Rust, Python and Node (docs/embedding.md)
sh scripts/test-embed.sh
# the FreeCAD adapter, with the wheel, against a mocked FreeCAD API
sh scripts/test-freecad.sh
sh scripts/build-wasm.sh
node crates/fourcam-wasm/js/test.mjs
sh scripts/test-python.sh
node web/tests/meshes.test.mjs
node web/tests/project.test.mjs
node web/tests/e2e.mjs
echo "ALL CHECKS PASSED"
