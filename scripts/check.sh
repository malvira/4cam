#!/bin/sh
# Everything the project promises, in one go: format, lints, Rust tests, the WebAssembly
# package (tested under Node), the Python wheel (tested in a fresh venv) and the web app
# (headless Chrome/Edge; needs network for three.js).
set -e
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
# STEP import builds SolidCraft's kernel: MARCAM_SKIP_STEP leaves it to scripts/check-step.sh
# (CI runs that as a job of its own)
if [ -n "$MARCAM_SKIP_STEP" ]; then
  cargo clippy --workspace --exclude marcam-step --all-targets -- -D warnings
else
  cargo clippy --workspace --all-targets -- -D warnings
fi
cargo clippy -p marcam-cli --features gpu --all-targets -- -D warnings
cargo test
# the GPU prototype against the CPU (skips, saying so, without a hardware GPU)
# one test at a time: several GPU devices opened at once have hung (a test burned 15 min of
# CPU in parallel, 5 s alone)
cargo test -p marcam-gpu -- --nocapture --test-threads=1
# the SolidCraft plugin: tests, the guest linted and packed, and the real host when
# SOLIDCRAFT_CLI names a solidcraft-cli (skipped otherwise, saying so)
cargo build -p marcam-cli
cargo test -p marcam-solidcraft
cargo clippy -p marcam-solidcraft --lib --target wasm32-unknown-unknown -- -D warnings
sh scripts/build-solidcraft-plugin.sh
sh scripts/e2e-solidcraft.sh
# the C ABI: its library, the C example and (with a .NET SDK) the C# example
sh scripts/test-capi.sh
# the embedding examples in Rust, Python and Node (docs/embedding.md)
sh scripts/test-embed.sh
# the FreeCAD adapter, with the wheel, against a mocked FreeCAD API
sh scripts/test-freecad.sh
sh scripts/build-wasm.sh
node crates/marcam-wasm/js/test.mjs
sh scripts/test-python.sh
node web/tests/meshes.test.mjs
node web/tests/project.test.mjs
node web/tests/e2e.mjs
# stateful UI tests: scripted scenarios and the seeded monkey (fixed seeds)
node web/tests/stress.mjs
# the simulation fuzz in the browser (fixed seeds; the engine-level one runs with cargo test)
node web/tests/simfuzz.mjs
# playback smoothness and a responsive page during background work (MARCAM_PERF_SLACK
# loosens the frame budget on slow machines)
node web/tests/perf.mjs --gate
if [ -z "$MARCAM_SKIP_STEP" ]; then sh scripts/check-step.sh; else echo "STEP checks skipped (MARCAM_SKIP_STEP)"; fi
echo "ALL CHECKS PASSED"
