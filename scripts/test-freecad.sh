#!/bin/sh
# The FreeCAD adapter (adapters/freecad) with the fourcam wheel, against a mocked FreeCAD API:
# FreeCAD is not needed (see adapters/freecad/README.md). Every plan's G-code must equal
# `4cam run` on the project the adapter wrote.
set -e
cd "$(dirname "$0")/.."
PY=${PYTHON:-python}
cargo build -q --release -p fourcam-cli
# (Git Bash's -x finds 4cam.exe for "4cam"; Python needs the real name)
if [ -f target/release/4cam.exe ]; then CLI=target/release/4cam.exe; else CLI=target/release/4cam; fi
WHL=$($PY scripts/build_wheel.py | tail -1)
$PY -m venv target/freecad-venv
if [ -x target/freecad-venv/Scripts/python.exe ]; then VPY=target/freecad-venv/Scripts/python.exe; else VPY=target/freecad-venv/bin/python; fi
$VPY -m pip install --no-index --force-reinstall --quiet "$WHL"
$VPY -I adapters/freecad/tests/test_freecad.py "$CLI"
