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
// profiles: built-in list, Fusion tool library import, machine limits
const reg = cam.profileList();
assert.deepEqual(reg.machines.map((m) => m.id), ["makera-z1-4axis", "makera-carvera-4axis", "makera-carvera-air-4axis", "linuxcnc-4axis"]);
const imp = cam.importProfile(readFileSync(join(root, "tests", "fixtures", "fusion-tool-library.json")), "Fixture Tools.json");
assert.equal(imp.kind, "tools");
assert.equal(imp.profile.tools.length, 3);
const lim = cam.planAndPost(job, "carvera-air");
assert.equal(lim.machine_id, "makera-carvera-air-4axis");
assert.ok(lim.limits.some((i) => i.message.includes("F5209")), JSON.stringify(lim.limits));
console.log(`profiles: ${reg.machines.length} machines, ${reg.libraries[0].tools.length} starter tools, limits ${lim.limits.length}`);
// operation registry and 0.2 projects
const types = cam.opTypes();
assert.deepEqual(types.map((t) => t.kind).sort(), ["contour", "drill", "engrave", "face", "flank_chamfer", "helical_knurl", "pocket", "raster3d", "rotary_finish", "rotary_rough", "rotary_turning", "rough3d", "vcarve"]);
const chk = cam.checkProject(JSON.parse(readFileSync(join(root, "examples", "knob", "knob.job.json"), "utf8")));
assert.equal(chk.upgraded, true);
assert.deepEqual(chk.problems, []);
const proj = JSON.parse(readFileSync(join(root, "examples", "knob", "knob.project.json"), "utf8"));
const pr = cam.planAndPost(proj, "makera-z1");
assert.deepEqual(pr.programs.map((p) => p.lines), [21787, 15146]);
console.log(`registry: ${types.length} operation types; 0.2 project plans like the 0.1 job`);
// knurl wizard on an empty setup gives the knob program back
const empty = JSON.parse(JSON.stringify(proj));
empty.setups[0].operations = [];
empty.setups[0].outputs = [];
const wz = cam.wizardKnurl(empty, {});
assert.ok(!wz.error, wz.error);
assert.deepEqual(wz.added.operations, ["op1", "op2", "op3"]);
const wr = cam.planAndPost(wz.project, "makera-z1");
for (const p of wr.programs) {
  const golden = readFileSync(join(root, "tests", "golden", `${p.name}.nc`), "utf8").replace(/\r\n/g, "\n");
  assert.equal(p.gcode, golden, `${p.name} from the wizard differs`);
}
const fr = cam.faceRegions(proj, "profile");
assert.equal(fr.regions.length, 8);
console.log("knurl wizard reproduces the cut G-code; 8 face regions");
// 3-axis plate on the Carvera: XYZ polylines (kinds 2/3), box stock, clean programs
const plate = JSON.parse(readFileSync(join(root, "examples", "plate", "plate.project.json"), "utf8"));
const dome = new Uint8Array(readFileSync(join(root, "examples", "plate", "dome.stl")));
const pl = cam.planAndPost(plate, "", dome, { max_points: 20000 });
assert.ok(!pl.error, pl.error);
assert.equal(pl.machine_id, "makera-carvera-4axis");
assert.deepEqual(pl.stock_box, { min: [0, 0, -10], max: [60, 40, 0] });
assert.deepEqual(pl.programs.map((p) => p.issues.length), [0, 0]);
for (const p of pl.programs) {
  const kinds = new Set();
  for (let i = 3; i < p.toolpath.points.length; i += 4) kinds.add(p.toolpath.points[i]);
  assert.deepEqual([...kinds].sort(), [2, 3]);
}
// 3-axis simulation: the block after each program, compared with the dome
const psim = cam.simulate(plate, {}, dome);
assert.ok(!psim.error, psim.error);
assert.equal(psim.kind, "xyz");
assert.deepEqual([psim.grid.nx, psim.grid.ny, psim.grid.cell], [240, 160, 0.25]);
assert.deepEqual(psim.stocks.map((s) => s.rapid_hits), [0, 0]);
assert.equal(psim.stocks[1].gouge_cells, 0);
const top = (s, x, y) => s.z[Math.floor(y / 0.25) * 240 + Math.floor(x / 0.25)];
assert.ok(Math.abs(top(psim.stocks[0], 15, 20) + 3.5) < 1e-5, "pocket floor");
assert.ok(Math.abs(top(psim.stocks[0], 40, 6) + 4.5) < 1e-5, "hole bottom");
// playback reaches the batch result, with every cut cell tagged by its operation
const po = cam.simOpen(plate, {}, dome);
assert.ok(!po.error, po.error);
assert.equal(po.kind, "xyz");
assert.deepEqual(po.ops.map((o) => o.kind), ["face", "pocket", "drill", "contour", "raster3d"]);
const end = cam.simSeek(po.segments);
assert.deepEqual(end.rows, [0, 159]);
const zEnd = new Float32Array(Uint8Array.from(atob(end.z), (c) => c.charCodeAt(0)).buffer);
assert.deepEqual([...zEnd], psim.stocks[1].z);
const tags = new Uint16Array(Uint8Array.from(atob(end.op), (c) => c.charCodeAt(0)).buffer);
assert.equal(tags[Math.floor(20 / 0.25) * 240 + Math.floor(15 / 0.25)], 2, "pocket floor cut by op 2");
console.log(`3-axis simulation: ${po.segments} moves, ${po.ops.length} operations, dome max left ${psim.stocks[1].max_left.toFixed(3)} mm`);
console.log(`3-axis plate: ${pl.programs.map((p) => `${p.name} ${p.lines} lines`).join(", ")}`);
// indexed 3+1 on the knob: plans clean, playback path carries Y and A for every segment
const kidx = JSON.parse(readFileSync(join(root, "examples", "knob", "knob_indexed.project.json"), "utf8"));
const ki = cam.planAndPost(kidx, "makera-z1");
assert.ok(!ki.error, ki.error);
assert.deepEqual(ki.programs.map((p) => p.issues.length), [0, 0, 0]);
assert.ok(ki.programs[2].gcode.includes("G0 A90.000"));
const kio = cam.simOpen(kidx, { na: 120 });
assert.ok(!kio.error, kio.error);
const kpath = new Float32Array(Uint8Array.from(atob(kio.path), (c) => c.charCodeAt(0)).buffer);
assert.equal(kpath.length, 7 * kio.segments);
const ys = new Set();
for (let i = 0; i < kio.segments; i++) if (kpath[7 * i + 1] !== 0) ys.add(kpath[7 * i + 3]);
assert.deepEqual([...ys].sort((a, b) => a - b), [0], "Y moves only in the A0 D-flat (the hole is on the centreline)");
console.log(`indexed 3+1: ${ki.programs[2].lines} lines, playback ${kio.segments} moves`);
// SVG import: a 40 x 20 user-unit rectangle placed at (5, 5) and scaled to 10 mm tall (20 x 10 mm)
const sv = cam.svgPaths('<svg xmlns="http://www.w3.org/2000/svg"><rect x="0" y="0" width="40" height="20"/></svg>', { at: [5, 5], height: 10 });
assert.ok(!sv.error, sv.error);
assert.equal(sv.count, 1);
assert.deepEqual(sv.paths[0].map((q) => q.join(",")).sort(), ["5,15", "25,15", "25,5", "5,5", "5,15"].sort());
assert.ok(cam.svgPaths("not svg", {}).error);
// wrapped engraving on the Z1 (ring example)
const ring = JSON.parse(readFileSync(join(root, "examples", "ring", "ring.project.json"), "utf8"));
const rg = cam.planAndPost(ring, "makera-z1");
assert.ok(!rg.error, rg.error);
assert.deepEqual(rg.programs.map((p) => p.issues.length), [0]);
console.log(`engraving: ring ${rg.programs[0].lines} lines; SVG rectangle -> ${sv.paths[0].length} points`);
console.log("wasm OK");
