// Watertightness of the 3D view's display meshes: node web/tests/meshes.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { blockSolid, checkClosed, cylinder, heightmapSolid, revolveProfile } from "../meshes.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const job = JSON.parse(readFileSync(join(root, "examples", "knob", "knob.job.json"), "utf8"));
const prof = job.part.profile.points;

/** Volume of the revolved profile (frustums), and the inscribed-polygon factor. */
function frustums(points) {
  let v = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    const [x0, r0] = points[i], [x1, r1] = points[i + 1];
    v += (Math.PI * (x1 - x0) * (r0 * r0 + r0 * r1 + r1 * r1)) / 3;
  }
  return v;
}
const polygonFactor = (n) => (n * Math.sin((2 * Math.PI) / n)) / (2 * Math.PI);

let n = 0;
function ok(name, cond, detail) {
  assert.ok(cond, `${name}: ${detail}`);
  console.log(`PASS  ${name}  (${detail})`);
  n++;
}

// knob part (revolved profile with steps at -18 and -15)
for (const seg of [24, 120]) {
  const m = revolveProfile(prof, seg);
  const c = checkClosed(m);
  const want = frustums(prof) * polygonFactor(seg);
  ok(`knob part, ${seg} segments: closed`, c.closed, `${c.triangles} triangles, ${c.badEdges} open/misoriented edges`);
  ok(`knob part, ${seg} segments: outward, right volume`, Math.abs(c.volume - want) / want < 1e-4, `volume ${c.volume.toFixed(3)} vs ${want.toFixed(3)} mm^3`);
}

// stock cylinder
{
  const c = checkClosed(cylinder(12.5, -24.7, 1, 96));
  const want = Math.PI * 12.5 * 12.5 * 25.7 * polygonFactor(96);
  ok("stock cylinder: closed", c.closed, `${c.badEdges} bad edges`);
  ok("stock cylinder: volume", Math.abs(c.volume - want) / want < 1e-4, `${c.volume.toFixed(1)} vs ${want.toFixed(1)}`);
}

// simulated stock heightmap: a stepped bar with a groove, with colours
{
  const grid = { x0: -10, dx: 0.5, nx: 20, na: 36 };
  const r = new Float32Array(grid.nx * grid.na);
  for (let i = 0; i < grid.nx; i++) for (let j = 0; j < grid.na; j++) r[i * grid.na + j] = i < 10 ? 4 : 6 - (j % 6 === 0 ? 0.5 : 0);
  const m = heightmapSolid(grid, r, () => [1, 0, 0]);
  const c = checkClosed(m);
  ok("simulated stock heightmap: closed", c.closed, `${c.triangles} triangles, ${c.badEdges} bad edges`);
  ok("simulated stock heightmap: outward (positive volume)", c.volume > 0, `${c.volume.toFixed(1)} mm^3`);
  ok("simulated stock heightmap: one colour per vertex", m.colors.length === m.positions.length, `${m.colors.length}`);
}

// 3-axis block heightmap: a pocket and a hole cut into a 30 x 20 x 10 block
{
  const grid = { x0: 0, y0: 0, cell: 0.5, nx: 60, ny: 40 };
  const z = new Float32Array(grid.nx * grid.ny).fill(0);
  for (let j = 10; j < 30; j++) for (let i = 5; i < 25; i++) z[j * grid.nx + i] = -3;
  z[20 * grid.nx + 45] = -6;
  const m = blockSolid(grid, z, -10, () => [0, 1, 0]);
  const c = checkClosed(m);
  ok("block heightmap: closed", c.closed, `${c.triangles} triangles, ${c.badEdges} bad edges`);
  // volume: the block over the cell-centre footprint (29.5 x 19.5) minus the cuts; on this
  // triangulation each lowered interior vertex removes depth x cell^2
  const want = 29.5 * 19.5 * 10 - 3 * 400 * 0.25 - 6 * 0.25;
  ok("block heightmap: outward, with the cuts' volume removed", Math.abs(c.volume - want) < 1e-3, `${c.volume.toFixed(1)} vs ${want.toFixed(1)} mm^3`);
  ok("block heightmap: vertex k is cell k", m.positions[3 * (20 * grid.nx + 45) + 2] === -6 && m.colors.length === m.positions.length, "");
}

// the checker itself catches open and flipped shells
{
  const open = revolveProfile(prof, 24);
  const cut = { positions: open.positions, indices: open.indices.slice(0, open.indices.length - 24 * 3) };
  ok("checker flags a missing end cap", !checkClosed(cut).closed, `${checkClosed(cut).badEdges} bad edges`);
  const flipped = { positions: open.positions, indices: open.indices.slice() };
  [flipped.indices[0], flipped.indices[1]] = [flipped.indices[1], flipped.indices[0]];
  ok("checker flags a flipped triangle", !checkClosed(flipped).closed, `${checkClosed(flipped).badEdges} bad edges`);
}
console.log(`\n${n} mesh checks passed`);
