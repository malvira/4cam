#!/bin/sh
# End-to-end check of the SolidCraft plugin, headless, with solidcraft-cli.
#
#   SOLIDCRAFT_CLI=/path/to/solidcraft-cli scripts/e2e-solidcraft.sh [operation ...]
#
# Builds the plugin, then in one solidcraft-cli session: revolves a knob, installs the plugin
# with a write grant on a scratch folder (and a read grant on it for plan-project), and runs
# `plan` for each operation (default: all of them) on the knob, the four rotary operations'
# own entries, `describe`, and `plan-project` on a project `plan` wrote. Then runs
# `marcam run` on each project written and requires the G-code to be byte-identical (and the
# own entries' and plan-project's G-code to be plan's). SolidCraft's plugin folder is pointed
# at a scratch directory (LOCALAPPDATA / XDG_DATA_HOME / HOME), so nothing of the user's
# SolidCraft set-up is touched. Skips (exit 0) when SOLIDCRAFT_CLI is not set.
set -e
cd "$(dirname "$0")/.."
if [ -z "$SOLIDCRAFT_CLI" ]; then
  echo "SOLIDCRAFT_CLI not set: skipping the SolidCraft end-to-end check"
  exit 0
fi
OPS=${*:-"rotary_rough rotary_finish helical_knurl flank_chamfer parting face pocket drill bore contour raster3d waterline rough3d engrave vcarve"}
ALIASES="rotary_rough rotary_finish helical_knurl flank_chamfer"

sh scripts/build-solidcraft-plugin.sh >/dev/null
cargo build -q --release -p marcam-cli
MARCAM=target/release/marcam
[ -x "$MARCAM" ] || MARCAM=target/release/marcam.exe

WORK=$(pwd)/target/solidcraft-e2e
rm -rf "$WORK"
mkdir -p "$WORK/data" "$WORK/out"
native() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi; }
OUT=$(native "$WORK/out")
ZIP=$(native "$(pwd)/target/solidcraft-plugin/marcam.zip")
PKG=$(native "$(pwd)/target/solidcraft-plugin/marcam")
export LOCALAPPDATA=$(native "$WORK/data") XDG_DATA_HOME="$WORK/data" HOME="$WORK/data"
case "$(uname -s)" in
  Darwin) INSTALLED="$WORK/data/Library/Application Support/SolidCraft/plugins/marcam" ;;
  Linux) INSTALLED="$WORK/data/solidcraft/plugins/marcam" ;;
  *) INSTALLED="$WORK/data/SolidCraft/plugins/marcam" ;;
esac
INSTALLED=$(native "$INSTALLED")

# The approval hash covers plugin.json and plugin.wasm; inspection runs no plugin code.
HASH=$("$SOLIDCRAFT_CLI" exec plugin.inspect "{\"path\": \"$PKG\"}" | sed -n 's/.*"approval": *"\([0-9a-f]*\)".*/\1/p' | head -1)
[ -n "$HASH" ] || { echo "plugin.inspect gave no approval hash"; exit 1; }
echo "approval $HASH"

run() { # entry, params
  RUNS="$RUNS,
 {\"command\": \"plugin.run\", \"params\": {\"path\": \"$INSTALLED\", \"approval\": \"$HASH\", \"entry\": \"$1\", \"params\": $2}}"
}
RUNS=""
for k in $OPS; do run plan "{\"bodies\": [\"Knob\"], \"operation\": \"$k\"}"; done
for k in $ALIASES; do run "$(echo "$k" | tr _ -)" "{\"bodies\": [\"Knob\"], \"name\": \"own_$k\"}"; done
run describe "{\"operation\": \"pocket\"}"
run plan-project "{\"project\": \"knob_pocket.marcam.toml\"}"
cat > "$WORK/script.json" <<EOF
{"commands": [
 {"command": "sketch.create", "params": {"plane": "XY"}},
 {"command": "sketch.line", "params": {"points": [[-21.7, 0], [-21.7, 3], [-18, 3], [-18, 5], [-15, 5], [-15, 10], [-0.8, 8.106666666666667], [0, 7.2], [0, 0]], "closed": true}},
 {"command": "sketch.finish", "params": {}},
 {"command": "solid.revolve", "params": {"axis": "x", "body_name": "Knob"}},
 {"command": "plugin.install", "params": {"path": "$ZIP", "approval": "$HASH", "approve": {"command_groups": [], "folders": [{"name": "output", "path": "$OUT", "read": false, "write": true}, {"name": "project", "path": "$OUT", "read": true, "write": false}]}}}$RUNS
]}
EOF
"$SOLIDCRAFT_CLI" run "$(native "$WORK/script.json")" --results --json-out "$(native "$WORK/results.json")" > "$WORK/run.log" 2>&1 || {
  cat "$WORK/run.log"; echo "solidcraft-cli run failed"; exit 1; }

# a program over SolidCraft's 1 MiB file-write limit comes gzipped
unz() { if [ -f "$WORK/out/$1.nc.gz" ]; then gzip -dc "$WORK/out/$1.nc.gz" > "$WORK/out/$1.nc"; echo " (written gzipped)"; fi; }

fail=0
for k in $OPS; do
  base="knob_$k"
  note=$(unz "$base")
  if [ ! -f "$WORK/out/$base.nc" ]; then
    echo "FAIL plan $k: the plugin wrote no $base.nc"; fail=1; continue
  fi
  "$MARCAM" run "$WORK/out/$base.marcam.toml" --out "$WORK/cli-$k" > "$WORK/cli-$k.log"
  if cmp -s "$WORK/out/$base.nc" "$WORK/cli-$k/$base.nc"; then
    echo "ok   plan $k: $(wc -l < "$WORK/out/$base.nc") lines$note, identical to marcam run"
  else
    echo "FAIL plan $k: plugin G-code differs from marcam run"; fail=1
  fi
done
for k in $ALIASES; do
  unz "own_$k" >/dev/null
  case " $OPS " in *" $k "*) ;; *) continue ;; esac
  if cmp -s "$WORK/out/own_$k.nc" "$WORK/out/knob_$k.nc"; then
    echo "ok   $(echo "$k" | tr _ -): identical to plan's"
  else
    echo "FAIL $(echo "$k" | tr _ -): its G-code differs from plan's"; fail=1
  fi
done
if grep -q '"operation": *"pocket"' "$WORK/results.json" && grep -q '"fields"' "$WORK/results.json"; then
  echo "ok   describe: the pocket's fields"
else
  echo "FAIL describe: no fields in the results"; fail=1
fi
if grep -q '"project": *"knob_pocket.marcam.toml"' "$WORK/results.json"; then
  echo "ok   plan-project: planned knob_pocket.marcam.toml from the project folder"
else
  echo "FAIL plan-project: no result"; fail=1
fi
[ $fail -eq 0 ] && echo "SOLIDCRAFT E2E PASSED"
exit $fail
