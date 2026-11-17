#!/bin/sh
# Build the wheel, install it into a throwaway venv (target/py-venv, no network) and test it.
set -e
cd "$(dirname "$0")/.."
PY=${PYTHON:-python}
WHL=$($PY scripts/build_wheel.py | tail -1)
$PY -m venv target/py-venv
if [ -x target/py-venv/Scripts/python.exe ]; then VPY=target/py-venv/Scripts/python.exe; else VPY=target/py-venv/bin/python; fi
$VPY -m pip install --no-index --force-reinstall --quiet "$WHL"
$VPY -I crates/marcam-py/tests/test_marcam.py
