# Build the WebAssembly module and copy it with its JS wrapper to web/pkg/.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
rustup target add wasm32-unknown-unknown | Out-Null
cargo build -p marcam-wasm --target wasm32-unknown-unknown --profile release-wasm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force web/pkg | Out-Null
Copy-Item target/wasm32-unknown-unknown/release-wasm/marcam_wasm.wasm web/pkg/marcam.wasm
Copy-Item crates/marcam-wasm/js/marcam.js web/pkg/marcam.js
Write-Output ("web/pkg/marcam.wasm ({0} bytes)" -f (Get-Item web/pkg/marcam.wasm).Length)
