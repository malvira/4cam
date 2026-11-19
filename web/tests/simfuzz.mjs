// Seeded fuzzing of planning and the simulation in the browser (the smaller sibling of
// crates/marcam-wasm/tests/fuzz.rs): random revolved parts on a rotary or indexed setup, every
// operation type that fits with its defaults pushed to extremes, planned with Generate, then
// simulated at a random resolution; now and then the simulation is cancelled with the ring.
// Each case must end (planned or refused with a reason, simulated, refused as too slow or
// cancelled) within its time, with no page error and the app not left busy, and the engine
// must work after it.
//
//   node web/tests/simfuzz.mjs                  (the CI seeds)
//   node web/tests/simfuzz.mjs --cases 40 --seed 100 --long   (long simulations accepted too)
//
// Needs web/pkg (scripts/build-wasm.sh) and Chrome, Edge or Chromium.
import { sleep, startApp, waitFor } from "./harness.mjs";

const arg = (name, dflt) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : dflt);
const SEED = Number(arg("--seed", 1));
const CASES = Number(arg("--cases", 6));

function rng(seed) {
  let s = BigInt(seed) & 0xffffffffffffffffn;
  const next = () => {
    s = (s + 0x9e3779b97f4a7c15n) & 0xffffffffffffffffn;
    let z = s;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & 0xffffffffffffffffn;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & 0xffffffffffffffffn;
    return Number((z ^ (z >> 31n)) >> 11n) / 2 ** 53;
  };
  const r = {
    f: next,
    range: (a, b) => a + (b - a) * next(),
    log: (a, b) => Math.exp(Math.log(a) + (Math.log(b) - Math.log(a)) * next()),
    int: (n) => Math.floor(next() * n),
    chance: (p) => next() < p,
  };
  r.pick = (xs) => xs[r.int(xs.length)];
  return r;
}

/** A random project: a revolved profile on a rotary or indexed bar, four tools. */
function project(r, seed) {
  const stockR = r.log(2, 30);
  const len = r.log(3, 120);
  const n = 1 + r.int(9);
  const xs = Array.from({ length: n }, () => -r.range(0, len)).sort((a, b) => a - b);
  const pts = [];
  for (const x of xs) {
    pts.push([x, r.chance(0.08) ? 0 : stockR * r.range(0.2, 1)]);
    if (r.chance(0.2)) pts.push([x, stockR * r.range(0.2, 1)]);
  }
  if (r.chance(0.3)) pts.reverse();
  const kind = r.pick(["rotary", "rotary", "indexed"]);
  const d = r.log(0.5, 8);
  return {
    schema_version: "0.2", name: `simfuzz ${seed}`,
    models: [{ id: "part", kind: "profile", points: pts }],
    tools: [
      { number: 1, description: "flat", kind: "flat", diameter: r.log(0.5, 10), flute_length: r.log(3, 40) },
      { number: 2, description: "ball", kind: "ball", diameter: r.log(0.5, 10), flute_length: r.log(3, 40) },
      { number: 3, description: "vbit", kind: "vbit", angle: r.range(20, 120), diameter: r.log(1, 10), flute_length: r.log(3, 30) },
      { number: 4, description: "tapered", kind: "tapered_ball", tip_diameter: d * r.range(0.1, 1), taper_angle: r.range(0, 20), diameter: d, flute_length: r.log(3, 30) },
    ],
    setups: [{ id: "s", name: "s", kind, machine: "makera-carvera-4axis", stock: { shape: "cylinder", radius: stockR, length: len + r.range(0, 10) },
      clearance: r.log(1, 6), part: ["part"], spindle_rpm: 12000, operations: [], outputs: [{ name: "fuzz", title: "fuzz", operations: [] }] }],
  };
}

const app = await startApp();
const { js } = app;
const results = [];
let failed = false;
try {
  const types = await js("await window.__marcam.engine('opTypes').then((r) => r.result.map((t) => ({ kind: t.kind, shapes: t.tool_shapes, needs: t.machine_needs, params: t.params.filter((p) => p.kind === 'number').map((p) => ({ name: p.name, min: p.min ?? null, max: p.max ?? null })) })))");
  await js("const g = document.getElementById('simGpu'); g.checked = false; g.dispatchEvent(new Event('change')); true");
  // the app asks before simulations over 30 s: declined here, accepted with --long (minutes)
  if (!process.argv.includes("--long")) await js("const ask = window.confirm.bind(window); window.confirm = (m) => (/^Simulating at/.test(m) ? false : ask(m)); true");
  for (let c = 0; c < CASES; c++) {
    const seed = SEED + c;
    const r = rng(seed);
    const doc = project(r, seed);
    const kind = doc.setups[0].kind;
    const fits = (t) => (kind === "indexed" ? t.needs.includes("xyz") : !t.needs.includes("xyz"));
    // operations: the engine's defaults for each type that fits (a tool it takes), pushed about
    const ops = [];
    for (const t of types.filter(fits)) {
      if (!r.chance(0.5)) continue;
      const tools = doc.tools.filter((x) => t.shapes.includes(x.kind));
      if (!tools.length) continue;
      const tool = r.pick(tools).number;
      let op;
      try {
        op = await js(`await window.__marcam.engine('opDefaults', ${JSON.stringify(doc)}, ${JSON.stringify({ type: t.kind, setup: "s", tool })}).then((r) => r.result)`);
      } catch {
        continue;
      }
      for (const p of t.params) {
        if (typeof op[p.name] !== "number" || p.name === "tool" || !r.chance(0.3)) continue;
        let v = op[p.name] * r.pick([0.02, 0.2, 5, 50]);
        if (p.min !== null) v = Math.max(v, p.min);
        if (p.max !== null) v = Math.min(v, p.max);
        op[p.name] = v;
      }
      op.id = `op${ops.length + 1}`;
      ops.push(op);
    }
    doc.setups[0].operations = ops;
    doc.setups[0].outputs[0].operations = ops.map((o) => o.id);
    const res = r.pick(["coarse", "normal", "fine", "ultra"]);
    const cancel = r.chance(0.3);
    const what = `seed ${seed}: ${kind}, ${doc.models[0].points.length} points, ${ops.length} ops (${ops.map((o) => o.type).join(",")}), ${res}${cancel ? ", cancelled" : ""}`;
    const mark = app.events.length;
    const t0 = Date.now();
    let outcome = "";
    try {
      await js(`await window.__marcam.project.openText(${JSON.stringify(JSON.stringify({ format: "marcam-project", version: 1, project: doc }))}, "fuzz.marcam.json").then(() => true)`);
      await app.settle(200);
      await js("document.getElementById('generate').click(); true");
      await sleep(300);
      // planned, or refused with a reason; the planning budget bounds it
      await waitFor(() => js("['done', 'error', 'ready'].includes(document.body.dataset.state)"), "planning", 240000);
      const planned = await js("!!window.__marcam.state.result && document.body.dataset.state === 'done'");
      if (!planned) {
        outcome = `not planned: ${(await js("document.getElementById('error').textContent")).slice(0, 120)}`;
      } else {
        await js(`const s = document.getElementById('simRes'); s.value = '${res}'; s.dispatchEvent(new Event('change')); true`);
        await js("document.getElementById('simulate').click(); true");
        if (cancel) {
          await sleep(800);
          await js("document.getElementById('simRing') && !document.getElementById('simRing').classList.contains('hidden') ? (document.getElementById('simRing').click(), true) : false");
        }
        // simulated, refused as too slow, cancelled or failed with a reason; never stuck
        await waitFor(() => js("!window.__marcam.simLock.busy && document.body.dataset.state !== 'busy'"), "the simulation to end", 700000);
        outcome = (await js("document.getElementById('status').textContent")).slice(0, 80);
        const err = await js("document.getElementById('error').textContent");
        if (err) outcome += ` | ${err.slice(0, 120)}`;
      }
      // the engine answers afterwards
      await waitFor(() => js("await window.__marcam.engine('version').then(() => true)"), "the engine after the case", 30000);
      const errs = app.pageErrors(mark);
      const ok = errs.length === 0;
      results.push(ok);
      console.log(`${ok ? "PASS" : "FAIL"}  ${what} -> ${outcome} (${((Date.now() - t0) / 1000).toFixed(1)} s)${ok ? "" : ` | page errors: ${errs.join(" | ").slice(0, 300)}`}`);
    } catch (e) {
      results.push(false);
      console.log(`FAIL  ${what} -> ${e.message} (${((Date.now() - t0) / 1000).toFixed(1)} s)`);
      try { await app.shot(`simfuzz_${seed}`); } catch {}
      try { await js("window.__marcam.cancelEngine(); true"); } catch {}
    }
  }
} catch (e) {
  failed = true;
  console.log(`FAIL  ${e.message}`);
} finally {
  await app.close();
}
const bad = results.filter((x) => !x).length;
console.log(`\n${results.length - bad}/${results.length} simulation fuzz cases passed`);
process.exit(failed || bad ? 1 : 0);
