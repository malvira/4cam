# Build the WebAssembly module and copy it with its JS wrapper to web/pkg/.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
rustup target add wasm32-unknown-unknown | Out-Null
cargo build -p fourcam-wasm --target wasm32-unknown-unknown --profile release-wasm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force web/pkg | Out-Null
Copy-Item target/wasm32-unknown-unknown/release-wasm/fourcam_wasm.wasm web/pkg/fourcam.wasm
Copy-Item crates/fourcam-wasm/js/fourcam.js web/pkg/fourcam.js
Write-Output ("web/pkg/fourcam.wasm ({0} bytes)" -f (Get-Item web/pkg/fourcam.wasm).Length)
