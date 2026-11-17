#!/bin/sh
# Build the SolidCraft plugin package: target/solidcraft-plugin/marcam.zip (and the
# unzipped target/solidcraft-plugin/marcam/). See docs/adapters/solidcraft.md.
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p marcam-solidcraft --lib --target wasm32-unknown-unknown --profile release-wasm
cargo run -q -p marcam-solidcraft --bin pack -- \
  target/wasm32-unknown-unknown/release-wasm/marcam_solidcraft.wasm target/solidcraft-plugin
