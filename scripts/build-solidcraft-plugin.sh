#!/bin/sh
# Build the SolidCraft plugin package: target/solidcraft-plugin/fourcam.zip (and the
# unzipped target/solidcraft-plugin/fourcam/). See docs/adapters/solidcraft.md.
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p fourcam-solidcraft --lib --target wasm32-unknown-unknown --profile release-wasm
cargo run -q -p fourcam-solidcraft --bin pack -- \
  target/wasm32-unknown-unknown/release-wasm/fourcam_solidcraft.wasm target/solidcraft-plugin
