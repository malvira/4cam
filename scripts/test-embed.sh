#!/bin/sh
# Run the embedding examples (examples/embed, docs/embedding.md) on the knob: Rust, Python
# (the wheel in a throwaway venv) and Node (the npm package packed locally). The C and C#
# examples run in scripts/test-capi.sh.
set -e
cd "$(dirname "$0")/.."
PY=${PYTHON:-python}
KNOB=examples/knob/knob.project.json
want() { echo "$1" | grep -q "$2" || { echo "missing \"$2\" in:"; echo "$1"; exit 1; }; }

OUT=$(cargo run -q -p marcam-embed-example -- $KNOB)
echo "$OUT"
want "$OUT" "setup1/knob_1_profile: 21787 lines"

WHL=$($PY scripts/build_wheel.py | tail -1)
$PY -m venv target/embed-venv
if [ -x target/embed-venv/Scripts/python.exe ]; then VPY=target/embed-venv/Scripts/python.exe; else VPY=target/embed-venv/bin/python; fi
$VPY -m pip install --no-index --force-reinstall --quiet "$WHL"
OUT=$($VPY -I examples/embed/python/plan.py $KNOB)
echo "$OUT"
want "$OUT" "knob_1_profile: 21787 lines, 0 issue(s)"

sh scripts/build-npm.sh >/dev/null
(cd examples/embed/node && npm install --no-save --offline --silent >/dev/null)
OUT=$(node examples/embed/node/plan.mjs $KNOB)
echo "$OUT"
want "$OUT" "knob_1_profile: 21787 lines, 0 issue(s)"
echo "EMBED EXAMPLES OK"
