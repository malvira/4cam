// Unit tests of web/project.js's pure parts: STL reading (binary, ASCII, errors), placement
// and stock sizing, and the project file format.   node web/tests/project.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { encodeStl, meshFileOf, parseStl, placePart, projectFile, readProjectFile } from "../project.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const knob = new Uint8Array(readFileSync(join(root, "examples", "knob", "knob.stl")));

// binary: the knob's 10080 triangles, and re-encoding gives the same triangles back
const { tris, ascii } = parseStl(knob);
assert.equal(ascii, false);
assert.equal(tris.length, 9 * 10080);
assert.deepEqual(parseStl(encodeStl(tris)).tris, tris);

// ASCII: the same triangles written as text
let text = "solid x\n";
for (let t = 0; t < 10080; t++) {
  text += "facet normal 0 0 0\nouter loop\n";
  for (let k = 0; k < 3; k++) text += `vertex ${tris[9 * t + 3 * k]} ${tris[9 * t + 3 * k + 1]} ${tris[9 * t + 3 * k + 2]}\n`;
  text += "endloop\nendfacet\n";
}
const a = parseStl(new TextEncoder().encode(text + "endsolid x\n"));
assert.equal(a.ascii, true);
assert.deepEqual(a.tris, tris);

// errors
assert.throws(() => parseStl(new TextEncoder().encode("hello")), /not an STL file/);
assert.throws(() => parseStl(new TextEncoder().encode("solid empty\nendsolid\n")), /no complete triangles/);

// rotary: front at X0, centred, bar R10.5 x 37 (the engine's rule)
const r = placePart(tris, { holding: "rotary", axis: "x" });
assert.deepEqual(r.stock, { shape: "cylinder", radius: 10.5, length: 37 });
assert.ok(Math.abs(r.translate[0]) < 1e-6 && r.axis === "x");
assert.deepEqual(r.size.map((v) => +v.toFixed(2)), [21.7, 20, 20]);
// the same part modelled along Z comes out the same
const z = new Float32Array(tris.length);
for (let i = 0; i < tris.length; i += 3) [z[i], z[i + 1], z[i + 2]] = [tris[i + 1], tris[i + 2], tris[i]];
const rz = placePart(z, { holding: "rotary", axis: "z" });
assert.deepEqual(rz.stock, r.stock);
assert.equal(rz.axis, "z");

// vise: Z up, 2 mm in from the corner, top 0.5 mm under Z0, block 2 mm under the part
const v = placePart(tris, { holding: "vise", up: "z" });
assert.equal(v.axis, "x");
assert.deepEqual(v.stock.max.map((x) => +x.toFixed(2)), [25.7, 24, 0]);
assert.equal(+v.stock.min[2].toFixed(2), -22.5);
assert.equal(placePart(tris, { holding: "vise", up: "y" }).axis, "z");

// the project file: one JSON, the mesh embedded, read back whole; plain projects/jobs too
const project = { schema_version: "0.2", name: "k", models: [{ id: "part", kind: "mesh", file: "knob.stl" }], tools: [], setups: [{ id: "s", part: ["part"], operations: [], outputs: [] }] };
const f = projectFile(project, knob, "knob.stl");
const back = readProjectFile(JSON.stringify(f));
assert.deepEqual(back.project, project);
assert.deepEqual(back.mesh, knob);
assert.equal(back.meshName, "knob.stl");
assert.equal(meshFileOf(project), "knob.stl");
assert.equal(readProjectFile(JSON.stringify(project)).mesh, null);
assert.equal(meshFileOf({ schema_version: "0.1", part: { mesh: { file: "a.stl" } } }), "a.stl");
assert.throws(() => readProjectFile("{ nope"), /not JSON/);
assert.throws(() => readProjectFile('{"hello": 1}'), /not a 4cam project/);
console.log("project.js OK");

// setup kinds: margins, explicit stock, the origin, and which operation types fit
{
  const { fitsKind, kindOf, STARTERS } = await import("../project.js");
  const r = placePart(tris, { kind: "indexed", axis: "x", margin: 2 });
  assert.equal(r.stock.radius, 12);
  const ex = placePart(tris, { kind: "rotary", axis: "x", explicit: { dia: 30, len: 60 }, origin: 1 });
  assert.deepEqual(ex.stock, { shape: "cylinder", radius: 15, length: 60 });
  assert.ok(Math.abs(ex.translate[0] - 1) < 1e-6, "front at X0 + 1");
  const box = placePart(tris, { kind: "vise", up: "z", explicit: { bx: 40, by: 30, bz: 25 }, origin: 1 });
  assert.deepEqual(box.stock, { shape: "box", min: [0, 0, -25], max: [40, 30, 0] });
  assert.equal(kindOf({ stock: { shape: "box" } }), "vise");
  assert.equal(kindOf({ kind: "indexed", stock: { shape: "cylinder" } }), "indexed");
  const t = (needs) => ({ machine_needs: needs });
  assert.ok(fitsKind(t(["rotary_a"]), "rotary") && !fitsKind(t(["xyz"]), "rotary") && fitsKind(t(["xyz"]), "rotary", false));
  assert.ok(fitsKind(t(["xyz"]), "indexed") && !fitsKind(t([]), "indexed"));
  assert.ok(fitsKind(t([]), "vise") && !fitsKind(t(["rotary_a"]), "vise"));
  assert.deepEqual(STARTERS.indexed.map((s) => s[2]), [0, 180, 0, 180]);
  console.log("setup kinds OK");
}
