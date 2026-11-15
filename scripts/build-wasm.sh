#!/bin/sh
# Build the WebAssembly module and copy it with its JS wrapper to web/pkg/.
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p fourcam-wasm --target wasm32-unknown-unknown --profile release-wasm
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release-wasm/fourcam_wasm.wasm web/pkg/fourcam.wasm
cp crates/fourcam-wasm/js/fourcam.js web/pkg/fourcam.js
echo "web/pkg/fourcam.wasm ($(wc -c < web/pkg/fourcam.wasm) bytes)"
