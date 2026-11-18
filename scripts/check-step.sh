#!/bin/sh
# STEP import (SolidCraft's kernel, fetched by cargo from its GitHub repository at the pinned
# revision): lints, the engine tests, the CLI with its `step` feature, the browser's STEP module
# and the web app importing STEP files. scripts/check.sh runs this unless MARCAM_SKIP_STEP is
# set (CI runs it as a job of its own).
set -e
cd "$(dirname "$0")/.."
cargo clippy -p marcam-step --all-targets -- -D warnings
cargo clippy -p marcam-step --no-default-features --lib --target wasm32-unknown-unknown -- -D warnings
cargo test -p marcam-step

sh scripts/build-wasm.sh
sh scripts/build-step-wasm.sh
node web/tests/step.mjs
echo "STEP CHECKS PASSED"
