// Node smoke test of the WebAssembly build: node crates/fourcam-wasm/js/test.mjs <fourcam.wasm>
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import assert from "node:assert/strict";
import { load } from "./fourcam.js";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", "..");
const wasmPath = process.argv[2] ?? join(root, "web", "pkg", "fourcam.wasm");
const cam = await load(readFileSync(wasmPath));

const v = cam.version();
assert.equal(v.name, "4cam");

const job = cam.exampleJob();
let t = Date.now();
const res = cam.planAndPost(job, "makera-z1", null, { max_points: 50000 });
assert.ok(!res.error, res.error);
assert.deepEqual(res.programs.map((p) => p.lines), [21787, 15146]);
for (const p of res.programs) assert.equal(p.issues.length, 0, p.issues.join("\n"));
// byte-identical to the G-code cut on the Z1
for (const p of res.programs) {
  const golden = readFileSync(join(root, "tests", "golden", `${p.name}.nc`), "utf8").replace(/\r\n/g, "\n");
  assert.equal(p.gcode, golden, `${p.name} differs from the golden file`);
}
console.log(`plan+post knob: ${Date.now() - t} ms, lines ${res.programs.map((p) => p.lines)}`);

const lc = cam.planAndPost(job, "linuxcnc");
assert.ok(lc.programs[0].gcode.includes("G93"));

const bad = cam.planAndPost("{", "");
assert.ok(bad.error);

// mesh job: needs the STL, then plans, finishes and simulates
const meshJob = JSON.parse(readFileSync(join(root, "examples", "knob", "knob_finish.job.json"), "utf8"));
const stl = readFileSync(join(root, "examples", "knob", "knob.stl"));
assert.ok(cam.planAndPost(meshJob, "").error);
t = Date.now();
const mres = cam.planAndPost(meshJob, "", new Uint8Array(stl), { max_points: 20000 });
assert.ok(!mres.error, mres.error);
for (const p of mres.programs) {
  assert.equal(p.issues.length, 0);
  assert.ok(p.gouge.max_depth < 0.01, `${p.name} gouges ${p.gouge.max_depth}`);
}
console.log(`plan+post mesh job: ${Date.now() - t} ms, programs ${mres.programs.map((p) => p.name)}`);

const tri = cam.meshTriangles(meshJob, { max_triangles: 5000 }, new Uint8Array(stl));
assert.equal(tri.total, 10080);

t = Date.now();
const sim = cam.simulate(job, { dx: 0.1, na: 180 });
assert.ok(!sim.error, sim.error);
assert.equal(sim.stocks[0].rapid_hits, 0);
assert.equal(sim.stocks[0].gouge_cells, 0);
assert.equal(sim.stocks[0].r.length, sim.grid.nx * sim.grid.na);
console.log(`simulate knob: ${Date.now() - t} ms, max left ${sim.stocks[0].max_left.toFixed(3)} mm`);
console.log("wasm OK");
