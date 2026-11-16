#!/bin/sh
# End-to-end check of the SolidCraft plugin, headless, with solidcraft-cli.
#
#   SOLIDCRAFT_CLI=/path/to/solidcraft-cli scripts/e2e-solidcraft.sh [entry ...]
#
# Builds the plugin, then in one solidcraft-cli session: revolves a knob, installs the plugin
# with a write grant on a scratch folder, and runs each entry (default: rotary-rough,
# rotary-finish, helical-knurl) on the knob. Then runs `4cam run` on the project each entry
# wrote and requires the G-code to be byte-identical. SolidCraft's plugin folder is pointed
# at a scratch directory (LOCALAPPDATA / XDG_DATA_HOME / HOME), so nothing of the user's
# SolidCraft set-up is touched. Skips (exit 0) when SOLIDCRAFT_CLI is not set.
set -e
cd "$(dirname "$0")/.."
if [ -z "$SOLIDCRAFT_CLI" ]; then
  echo "SOLIDCRAFT_CLI not set: skipping the SolidCraft end-to-end check"
  exit 0
fi
ENTRIES=${*:-"rotary-rough rotary-finish helical-knurl"}

sh scripts/build-solidcraft-plugin.sh >/dev/null
cargo build -q --release -p fourcam-cli
FOURCAM=target/release/4cam
[ -x "$FOURCAM" ] || FOURCAM=target/release/4cam.exe

WORK=$(pwd)/target/solidcraft-e2e
rm -rf "$WORK"
mkdir -p "$WORK/data" "$WORK/out"
native() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi; }
OUT=$(native "$WORK/out")
ZIP=$(native "$(pwd)/target/solidcraft-plugin/fourcam.zip")
PKG=$(native "$(pwd)/target/solidcraft-plugin/fourcam")
export LOCALAPPDATA=$(native "$WORK/data") XDG_DATA_HOME="$WORK/data" HOME="$WORK/data"
case "$(uname -s)" in
  Darwin) INSTALLED="$WORK/data/Library/Application Support/SolidCraft/plugins/fourcam" ;;
  Linux) INSTALLED="$WORK/data/solidcraft/plugins/fourcam" ;;
  *) INSTALLED="$WORK/data/SolidCraft/plugins/fourcam" ;;
esac
INSTALLED=$(native "$INSTALLED")

# The approval hash covers plugin.json and plugin.wasm; inspection runs no plugin code.
HASH=$("$SOLIDCRAFT_CLI" exec plugin.inspect "{\"path\": \"$PKG\"}" | sed -n 's/.*"approval": *"\([0-9a-f]*\)".*/\1/p' | head -1)
[ -n "$HASH" ] || { echo "plugin.inspect gave no approval hash"; exit 1; }
echo "approval $HASH"

RUNS=""
for e in $ENTRIES; do
  RUNS="$RUNS,
 {\"command\": \"plugin.run\", \"params\": {\"path\": \"$INSTALLED\", \"approval\": \"$HASH\", \"entry\": \"$e\", \"params\": {\"bodies\": [\"Knob\"]}}}"
done
cat > "$WORK/script.json" <<EOF
{"commands": [
 {"command": "sketch.create", "params": {"plane": "XY"}},
 {"command": "sketch.line", "params": {"points": [[-21.7, 0], [-21.7, 3], [-18, 3], [-18, 5], [-15, 5], [-15, 10], [-0.8, 8.106666666666667], [0, 7.2], [0, 0]], "closed": true}},
 {"command": "sketch.finish", "params": {}},
 {"command": "solid.revolve", "params": {"axis": "x", "body_name": "Knob"}},
 {"command": "plugin.install", "params": {"path": "$ZIP", "approval": "$HASH", "approve": {"command_groups": [], "folders": [{"name": "output", "path": "$OUT", "read": false, "write": true}]}}}$RUNS
]}
EOF
"$SOLIDCRAFT_CLI" run "$(native "$WORK/script.json")" --results --json-out "$(native "$WORK/results.json")" > "$WORK/run.log" 2>&1 || {
  cat "$WORK/run.log"; echo "solidcraft-cli run failed"; exit 1; }

fail=0
for e in $ENTRIES; do
  kind=$(echo "$e" | tr - _)
  base="knob_$kind"
  if [ ! -f "$WORK/out/$base.nc" ]; then
    echo "FAIL $e: the plugin wrote no $base.nc"; fail=1; continue
  fi
  "$FOURCAM" run "$WORK/out/$base.project.json" --out "$WORK/cli-$kind" > "$WORK/cli-$kind.log"
  if cmp -s "$WORK/out/$base.nc" "$WORK/cli-$kind/$base.nc"; then
    echo "ok   $e: $(wc -l < "$WORK/out/$base.nc") lines, identical to 4cam run"
  else
    echo "FAIL $e: plugin G-code differs from 4cam run"; fail=1
  fi
done
[ $fail -eq 0 ] && echo "SOLIDCRAFT E2E PASSED"
exit $fail
