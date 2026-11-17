# Build the engine and serve the web app at http://localhost:8000/web/
param([int]$Port = 8000)
$ErrorActionPreference = "Stop"
$root = Join-Path $PSScriptRoot ".."
& (Join-Path $root "scripts/build-wasm.ps1")
Write-Output "marcam web UI: http://localhost:$Port/web/  (Ctrl+C to stop)"
python -m http.server $Port --bind 127.0.0.1 --directory $root
