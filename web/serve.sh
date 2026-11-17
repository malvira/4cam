#!/bin/sh
# Build the engine and serve the web app at http://localhost:8000/web/
set -e
cd "$(dirname "$0")/.."
PORT=${1:-8000}
sh scripts/build-wasm.sh
echo "marcam web UI: http://localhost:$PORT/web/  (Ctrl+C to stop)"
exec python -m http.server "$PORT" --bind 127.0.0.1 --directory .
