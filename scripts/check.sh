#!/bin/sh
# Everything the project promises, in one go: format, lints, Rust tests, the WebAssembly
# package (tested under Node), the Python wheel (tested in a fresh venv) and the web app
# (headless Chrome/Edge; needs network for three.js).
set -e
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
sh scripts/build-wasm.sh
node crates/fourcam-wasm/js/test.mjs
sh scripts/test-python.sh
node web/tests/e2e.mjs
echo "ALL CHECKS PASSED"
