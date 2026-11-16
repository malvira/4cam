#!/bin/sh
# Prepare every package without publishing anything (see docs/packaging.md):
#  - crates.io: package and verify-build the five crates, uploads aborted (--dry-run);
#  - PyPI: the abi3 wheel and the sdist with maturin (a local venv, target/pkg-venv), the
#    wheel tested in a fresh venv and the sdist rebuilt from source;
#  - npm: the WebAssembly package, smoke-tested from Node, packed and `npm publish --dry-run`.
# Artifacts land in target/pkg/.
set -e
cd "$(dirname "$0")/.."
PY=${PYTHON:-python}
echo "== crates.io (dry run)"
cargo publish --dry-run --workspace --allow-dirty 2>&1 | grep -E "Packaged|aborting upload"

echo "== PyPI (wheel + sdist, not uploaded)"
if [ ! -d target/pkg-venv ]; then $PY -m venv target/pkg-venv; fi
if [ -x target/pkg-venv/Scripts/python.exe ]; then BIN=target/pkg-venv/Scripts; else BIN=target/pkg-venv/bin; fi
VPY=$(ls $BIN/python* | head -1)
$VPY -m pip install --quiet "maturin>=1.9,<2"
rm -rf target/pkg/python
(cd crates/fourcam-py && ../../$BIN/maturin build --release --out ../../target/pkg/python >/dev/null && ../../$BIN/maturin sdist --out ../../target/pkg/python >/dev/null)
rm -rf target/pkg-test && $PY -m venv target/pkg-test
if [ -x target/pkg-test/Scripts/python.exe ]; then TPY=target/pkg-test/Scripts/python.exe; else TPY=target/pkg-test/bin/python; fi
$TPY -m pip install --quiet --no-index target/pkg/python/*.whl
$TPY -I crates/fourcam-py/tests/test_fourcam.py | tail -1
PATH="$(pwd)/$BIN:$PATH" $VPY -m pip wheel --quiet --no-build-isolation --no-deps target/pkg/python/*.tar.gz -w target/pkg/python/from-sdist
ls target/pkg/python target/pkg/python/from-sdist

echo "== npm (packed, publish --dry-run)"
sh scripts/build-npm.sh
echo "PACKAGES READY (nothing published)"
