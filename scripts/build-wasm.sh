#!/bin/sh
# Build the WebAssembly module and copy it with its JS wrapper to web/pkg/.
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p marcam-wasm --target wasm32-unknown-unknown --profile release-wasm
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release-wasm/marcam_wasm.wasm web/pkg/marcam.wasm
cp crates/marcam-wasm/js/marcam.js web/pkg/marcam.js
echo "web/pkg/marcam.wasm ($(wc -c < web/pkg/marcam.wasm) bytes)"
