#!/bin/sh
# Build the browser's STEP module (SolidCraft's kernel, crates/marcam-step) into
# web/pkg/marcam_step.wasm. It is fetched only when a STEP file is opened. SolidCraft comes from
# its GitHub repository at the revision pinned in crates/marcam-step/Cargo.toml.
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p marcam-step --lib --no-default-features --target wasm32-unknown-unknown --profile release-step-wasm
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release-step-wasm/marcam_step.wasm web/pkg/marcam_step.wasm
echo "web/pkg/marcam_step.wasm ($(wc -c < web/pkg/marcam_step.wasm) bytes)"
