#!/bin/sh
# The FreeCAD adapter (adapters/freecad) with the marcam wheel, against a mocked FreeCAD API:
# FreeCAD is not needed (see adapters/freecad/README.md). Every plan's G-code must equal
# `marcam run` on the project the adapter wrote.
set -e
cd "$(dirname "$0")/.."
PY=${PYTHON:-python}
cargo build -q --release -p marcam-cli
# (Git Bash's -x finds marcam.exe for "marcam"; Python needs the real name)
if [ -f target/release/marcam.exe ]; then CLI=target/release/marcam.exe; else CLI=target/release/marcam; fi
WHL=$($PY scripts/build_wheel.py | tail -1)
$PY -m venv --clear target/freecad-venv
if [ -x target/freecad-venv/Scripts/python.exe ]; then VPY=target/freecad-venv/Scripts/python.exe; else VPY=target/freecad-venv/bin/python; fi
$VPY -m pip install --no-index --force-reinstall --quiet "$WHL"
$VPY -I adapters/freecad/tests/test_freecad.py "$CLI"
