#!/bin/sh
# Assemble the npm package (target/pkg/npm: the wrapper, the wasm, types, README, LICENSE),
# smoke-test it from Node and pack it. Nothing is published: `npm publish --dry-run` only
# reports what would be uploaded.
set -e
cd "$(dirname "$0")/.."
sh scripts/build-wasm.sh >/dev/null
OUT=target/pkg/npm
rm -rf "$OUT"
mkdir -p "$OUT"
cp crates/fourcam-wasm/npm/package.json crates/fourcam-wasm/npm/index.js crates/fourcam-wasm/npm/index.d.ts crates/fourcam-wasm/npm/README.md "$OUT/"
cp crates/fourcam-wasm/js/fourcam.js web/pkg/fourcam.wasm LICENSE "$OUT/"
node --input-type=module -e "
import { init } from './$OUT/index.js';
const cam = await init();
const v = cam.version();
const r = cam.planAndPost(cam.exampleJob());
if (v.name !== '4cam' || r.error || r.programs.length !== 2) { console.error(v, r.error); process.exit(1); }
console.log('npm package works:', v.version, r.programs.map(p => p.name + ' ' + p.lines).join(', '));
"
(cd "$OUT" && npm pack --pack-destination .. --silent >/dev/null && npm publish --dry-run 2>&1 | grep -E "notice (name|version|filename|package size|unpacked size|total files)|\+ fourcam" || true)
ls target/pkg/fourcam-*.tgz
