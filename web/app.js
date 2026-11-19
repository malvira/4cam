// marcam web UI: job editor -> engine (WebAssembly, in a worker) -> G-code, checks, previews.

import { blockSolid, cylinder, heightmapSolid, revolveProfile, stockObject, STOCK_LEFT } from "./meshes.js";
import { gpuDevice, simulateOnGpu } from "./gpu/gpusim.js";
import { setupMachining } from "./machining.js";
import { addViewCube } from "./viewcube.js";
import { splitter } from "./splitters.js";
import { setupTree } from "./tree.js";
import { setupProject } from "./project.js";

// Settings saved before the rename (4cam -> marcam) carry over once: "4cam.x" -> "marcam.x".
try {
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k && k.startsWith("4cam.") && localStorage.getItem("marcam." + k.slice(5)) === null) localStorage.setItem("marcam." + k.slice(5), localStorage.getItem(k));
  }
} catch {
  // storage blocked: nothing to carry over
}

const $ = (id) => document.getElementById(id);

// ---------------------------------------------------------------- engine (worker RPC)
const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
let nextId = 1;
const pending = new Map();
worker.onmessage = (ev) => {
  const { id, ok, result, error, ms } = ev.data;
  const p = pending.get(id);
  if (!p) return;
  pending.delete(id);
  ok ? p.resolve({ result, ms }) : p.reject(new Error(error));
};
worker.onerror = (ev) => setStatus(`Engine failed to start: ${ev.message || "see console"}`, "error");
/** Engine calls by command, since the page loaded (window.__marcam.calls). */
const calls = {};
function engine(cmd, ...args) {
  calls[cmd] = (calls[cmd] || 0) + 1;
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, cmd, args });
  });
}

// ---------------------------------------------------------------- state
const state = {
  mesh: null, // Uint8Array
  meshName: "",
  result: null, // planAndPost result
  sim: null, // simulate result
  tab: 0,
  three: null, // lazily loaded three.js view
};
window.__marcam = { state, engine, calls };

function setStatus(text, mode = "") {
  $("status").textContent = text;
  document.body.dataset.state = mode;
}

function showError(msg) {
  const el = $("error");
  el.textContent = msg || "";
  el.classList.toggle("hidden", !msg);
}

// ---------------------------------------------------------------- job editing
function readJob() {
  try {
    return JSON.parse($("job").value);
  } catch (e) {
    throw new Error(`Job JSON is not valid: ${e.message}`);
  }
}

/** Write the project into the editor. An edit (the default) marks the project unsaved and
 *  its results stale; `{ display: true }` (a colour, a name) only unsaved; `{ load: true }` is
 *  a project being opened or reset. */
function writeJob(job, opts = {}) {
  $("job").value = JSON.stringify(job, null, 2);
  if (!opts.load) window.__marcam.project?.changed({ stale: !opts.display });
}

/** The Generate button asks for attention when the results are out of date or missing. */
/** Whether the project has an operation to plan (one that is enabled). */
function hasEnabledOps(j) {
  // (an operation in error is not planned: its picked features are gone)
  // (nor one whose tool the project no longer has)
  return j.schema_version === "0.2" ? !!j.setups?.some((s) => s.operations?.some((o) => o.enabled !== false && !o.error && (j.tools || []).some((x) => x.number === o.tool))) : !!j.programs?.some((p) => p.operations?.length);
}

/** Whether the project has a part (a model, a mesh or a profile). */
function hasPart(j) {
  return j.schema_version === "0.2" ? !!j.models?.length : !!(j.part?.profile || j.part?.mesh);
}

function updateGenerateButton() {
  let ops = false;
  let reason = "";
  try {
    const j = readJob();
    ops = hasEnabledOps(j);
    if (!ops) reason = hasPart(j) ? "No operation to generate: add one in the project tree (or enable one)" : "Import an STL first";
  } catch {}
  $("generate").classList.toggle("attention", ops && (state.stale || !state.result));
  $("generate").title = !ops ? reason : state.stale ? "The project changed: generate the toolpaths again" : "";
  // nothing to plan: disabled (unless a generate or simulation holds it)
  if (document.body.dataset.state !== "busy") $("generate").disabled = !ops && !!state.registry;
}

// ---------------------------------------------------------------- auto-regenerate
// Off by default. When on, an edit regenerates 800 ms after the last one, never while a
// mouse button is held (a slider or a drag waits for its release), and a newer request
// supersedes one still running (its result is dropped).
const AUTO_DELAY = 800;
let autoTimer = null;
let autoPending = false;
let pointerHeld = false;
function autoOn() {
  return $("autoGen").checked;
}
function scheduleAuto() {
  if (!autoOn()) return;
  clearTimeout(autoTimer);
  if (pointerHeld) {
    autoPending = true;
    return;
  }
  autoPending = false;
  autoTimer = setTimeout(() => {
    autoTimer = null;
    generate();
  }, AUTO_DELAY);
}
window.addEventListener("pointerdown", () => {
  pointerHeld = true;
}, true);
const released = () => {
  pointerHeld = false;
  if (autoPending) scheduleAuto();
};
window.addEventListener("pointerup", released, true);
window.addEventListener("pointercancel", released, true);

/** The results no longer match the project: say so on them until the next Generate. */
function markStale() {
  let none = false;
  try {
    const j = readJob();
    none = !hasEnabledOps(j) && hasPart(j);
  } catch {}
  if (none) {
    // nothing left to plan: the toolpaths of what was there go; the part and stock stay
    if (state.result || !$("emptyState")) clearResults(NO_OPS, []);
    drawModel();
    updateGenerateButton();
    return;
  }
  drawModel();
  if (state.result && !state.stale) {
    state.stale = true;
    document.body.classList.add("stale");
    selectTab(state.tab);
  }
  updateGenerateButton();
  scheduleAuto();
}

const RESULT_GROUPS = ["part", "stock", "path", "sim", "tool"];

// ---------------------------------------------------------------- toolpath visibility
// View only: hidden operations' moves are left out of the 3D and side views. Nothing is
// planned or marked out of date, and the G-code is the same. Keys are "setup/operation";
// kept in the project file (its view) and in this browser.
state.hidden = new Set();
function hiddenStoreKey() {
  return `marcam.hidden:${window.__marcam?.project?.state?.name || "untitled"}`;
}
function setHidden(keys, opts = {}) {
  state.hidden = new Set(keys);
  if (opts.persist !== false) {
    try {
      localStorage.setItem(hiddenStoreKey(), JSON.stringify([...state.hidden]));
    } catch {
      // storage blocked: this session only
    }
  }
  const v = state.three;
  if (v && !v.failed) v.key = null;
  if (state.result) {
    updateView();
    drawSide();
  }
  window.__marcam.tree?.renderEyes?.();
  // the simulations leave hidden operations out: done again (nothing is planned again)
  window.__marcam.machining?.applyHidden?.();
  if (state.sim && opts.resimulate !== false) resimulate();
}

/** The simulated stock again (the eyes changed): the toolpaths stay, only what is cut. */
let resimTimer = null;
function resimulate() {
  clearTimeout(resimTimer);
  resimTimer = setTimeout(() => {
    if (document.body.dataset.state !== "busy" && state.result) simulate();
  }, 200);
}
/** Each preview point of program `p`: whether its operation is hidden (null: none is). */
function hiddenMask(p) {
  const ops = p.toolpath.ops || [];
  if (!state.hidden.size || !ops.length) return null;
  const setupId = state.tree?.setup;
  // a 0.1 job shown as a project: operations by planning order
  const s = state.tree?.doc?.setups?.find((x) => x.id === setupId);
  const planned = s ? s.outputs.flatMap((o) => o.operations).filter((id) => s.operations.find((op) => op.id === id)?.enabled !== false) : [];
  const n = p.toolpath.points.length / 4;
  const mask = new Uint8Array(n);
  ops.forEach((o, k) => {
    const id = o.id ?? planned[o.index];
    if (!state.hidden.has(`${setupId}/${id}`)) return;
    const end = k + 1 < ops.length ? ops[k + 1].start : n;
    // the move into the operation's first point belongs to it too
    mask.fill(1, o.start, end);
  });
  return mask;
}
const NO_OPS = "No operations: add one in the project tree (or use a detected feature in the setup), then Generate.";

/**
 * The one reset that New, Open and the examples go through: nothing of the previous project
 * survives. A generate or simulation still running is dropped when it returns; the auto
 * regenerate is cancelled; the machining playback closes and its meshes, GPU stock and the
 * worker's sessions are freed; face picking stops and its highlight is disposed; the tree,
 * the results (G-code, estimates, side view, toolpaths, part, stock, sim), the warnings, the
 * undo and the mesh are cleared; the camera refits on the next project.
 */
async function resetProject(msg, actions) {
  state.genRun = (state.genRun || 0) + 1;
  state.epoch = (state.epoch || 0) + 1;
  clearTimeout(autoTimer);
  autoTimer = null;
  autoPending = false;
  window.__marcam.machining?.reset();
  window.__marcam.tree?.reset();
  window.__marcam.project?.dropUndo();
  try {
    await engine("simClose");
  } catch {
    // an engine without sessions (not loaded yet): nothing to free
  }
  state.simGrid = null;
  state.hidden = new Set();
  setMesh(null, "");
  showError("");
  clearResults(msg, actions);
}

/** No results (a new or just opened project); `msg`, if given, says what to do. */
function clearResults(msg, actions = ["import", "open"]) {
  state.result = null;
  state.sim = null;
  state.job = null;
  state.stale = false;
  document.body.classList.remove("stale");
  $("tabs").innerHTML = "";
  const btn = { import: '<button type="button" data-empty="import" class="primary">Import STL...</button>', open: '<button type="button" data-empty="open">Open project...</button>', generate: '<button type="button" data-empty="generate" class="primary" id="emptyGenerate">Generate</button>' };
  $("summary").innerHTML = msg ? `<div class="empty-state" id="emptyState">${msg}<br>${actions.map((k) => btn[k]).join("")}</div>` : "";
  $("gcode").value = "";
  $("codeInfo").textContent = "";
  $("download").disabled = true;
  $("scrub").max = 0;
  $("scrub").value = 0;
  $("scrub").disabled = true;
  $("scrubLabel").textContent = "";
  const v = state.three;
  if (v && !v.failed) {
    for (const k of RESULT_GROUPS) clear(v.groups[k]);
    v.key = null;
    v.modelKey = null;
    v.simKey = null;
    v.fitted = null;
    v.renderer.render(v.scene, v.camera);
  }
  const ctx = $("side").getContext("2d");
  ctx.clearRect(0, 0, $("side").width, $("side").height);
  updateGenerateButton();
  drawModel();
}

/** Everything that shows the project, again: quick settings, tools, the tree, the grid. */
async function refreshAll() {
  renderQuick();
  renderTools();
  await window.__marcam.tree?.refresh();
  updateSimResInfo();
  updateGenerateButton();
  await drawModel();
}

/** Quick fields for the common parameters; they write straight into the JSON. */
function renderQuick() {
  const box = $("quick");
  box.innerHTML = "";
  let job;
  try {
    job = readJob();
  } catch {
    box.textContent = "(fix the JSON to edit settings here)";
    return;
  }
  const add = (group, label, path, step = "any") => {
    let obj = job;
    for (const k of path.slice(0, -1)) obj = obj?.[k];
    const key = path[path.length - 1];
    if (obj == null || typeof obj[key] !== "number") return;
    if (group && box.dataset.lastGroup !== group) {
      const g = document.createElement("div");
      g.className = "group";
      g.textContent = group;
      box.appendChild(g);
      box.dataset.lastGroup = group;
    }
    const id = `q_${path.join("_")}`;
    const l = document.createElement("label");
    l.htmlFor = id;
    l.textContent = label;
    const inp = document.createElement("input");
    inp.type = "number";
    inp.id = id;
    inp.step = step;
    inp.value = obj[key];
    inp.addEventListener("change", () => {
      const v = parseFloat(inp.value);
      if (!Number.isFinite(v)) return;
      const j = readJob();
      let o = j;
      for (const k of path.slice(0, -1)) o = o?.[k];
      // the project changed shape since this was drawn (a 0.1 job saved as 0.2 on its first
      // edit): the field is not there any more, so the panel is drawn again
      if (o == null || typeof o[key] !== "number") return renderQuick();
      o[key] = v;
      writeJob(j);
    });
    box.append(l, inp);
  };
  box.dataset.lastGroup = "";
  add("Stock", "radius (mm)", ["stock", "radius"]);
  add("Stock", "length (mm)", ["stock", "length"]);
  add("Stock", "clearance (mm)", ["clearance"]);
  add("Stock", "spindle (rpm)", ["spindle_rpm"], "1");
  (job.tools || []).forEach((t, i) => {
    const g = `T${t.number} ${t.kind}`;
    add(g, "diameter (mm)", ["tools", i, "diameter"]);
    add(g, "angle (deg)", ["tools", i, "angle"]);
  });
  (job.programs || []).forEach((p, i) =>
    (p.operations || []).forEach((op, k) => {
      const g = `${p.name}: ${op.type}`;
      for (const key of Object.keys(op)) {
        if (/feed|step|depth|allowance/.test(key) && typeof op[key] === "number") {
          add(g, key.replace(/_/g, " "), ["programs", i, "operations", k, key]);
        }
      }
    }),
  );
}

async function loadExample(name) {
  showError("");
  // busy until it is in: Generate meanwhile would plan the project being replaced
  if (state.registry) setStatus(`Loading the ${name} example...`, "busy");
  $("generate").disabled = $("simulate").disabled = true;
  try {
    await loadExampleInto(name);
  } finally {
    if (state.registry) $("generate").disabled = $("simulate").disabled = false;
  }
}

/** A project or job file's text (TOML or JSON) as the document object the page keeps. */
async function parseProjectText(text) {
  const t = text.replace(/^﻿/, "").trimStart();
  if (t.startsWith("{")) return JSON.parse(t);
  // TOML: the engine reads it (the same document model)
  return (await engine("checkProject", text)).result.project;
}
window.__marcam.parseProjectText = parseProjectText;

async function loadExampleInto(name) {
  await resetProject("");
  if (name === "knob") {
    const { result } = await engine("exampleJob");
    writeJob(result, { load: true });
    setMesh(null, "");
  } else {
    const [file, mesh] = {
      plate: ["plate/plate.marcam.toml", "plate/dome.stl"],
      knob_indexed: ["knob/knob_indexed.marcam.toml", null],
      plaque: ["plaque/plaque.marcam.toml", "plaque/plaque.png"],
      ring: ["ring/ring.marcam.toml", null],
    }[name] || ["knob/knob_finish.job.json", "knob/knob.stl"];
    const [job, stl] = await Promise.all([
      fetch(`../examples/${file}`).then((r) => {
        if (!r.ok) throw new Error(`examples/${file} not found (serve the repository root)`);
        return r.text().then(parseProjectText);
      }),
      mesh ? fetch(`../examples/${mesh}`).then((r) => (r.ok ? r.arrayBuffer() : null)) : null,
    ]);
    writeJob(job, { load: true });
    const meshName = mesh?.split("/")[1];
    setMesh(stl ? new Uint8Array(stl) : null, stl ? `${meshName} (example)` : "");
  }
  // a 0.1 job names its machine; a project names one per setup
  const loaded = readJob();
  const jm = loaded.machine ?? loaded.setups?.[0]?.machine;
  if (jm) renderMachines(jm);
  renderQuick();
  renderTools();
  if (window.__marcam.project) {
    window.__marcam.project.state.fileName = null;
    window.__marcam.project.clean(loaded.name || `example: ${name}`);
  }
  clearResults("");
  await window.__marcam.tree?.refresh();
  updateSimResInfo();
  if (state.registry) setStatus(`Loaded the ${name} example: Generate to plan it`, "ready");
}

/** The part's mesh (an STL, with face numbers when it came from a STEP); `source`: the file it
 *  was made from, which the project saves instead ({ name, bytes }: a STEP). */
function setMesh(bytes, name, source = null) {
  state.mesh = bytes;
  state.meshName = name;
  state.meshSource = source;
  $("meshInfo").textContent = bytes ? `${name}: ${(bytes.length / 1024).toFixed(0)} KiB` : "No mesh loaded";
}

/** The machine for the engine: a built-in id, an imported profile object, or custom JSON. */
function machineArg() {
  const v = $("machine").value;
  if (v === "custom") return $("machineJson").value;
  if (v.startsWith("user:")) return userMachines().find((m) => `user:${m.id}` === v) || null;
  return v;
}

function currentMachine() {
  const v = $("machine").value;
  if (v.startsWith("user:")) return userMachines().find((m) => `user:${m.id}` === v);
  return state.registry.machines.find((m) => m.id === v);
}

// ---------------------------------------------------------------- generate
/** "1 h 02 min", "12 min 05 s", "48 s" (as the engine's CLI prints them). */
export function fmtTime(sec) {
  const s = Math.max(0, Math.round(sec));
  if (s >= 3600) return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600 + 30) / 60)).padStart(2, "0")} min`;
  if (s >= 60) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
  return `${s} s`;
}
window.__marcam = Object.assign(window.__marcam || {}, { fmtTime });

async function generate() {
  showError("");
  let job;
  try {
    job = readJob();
  } catch (e) {
    showError(e.message);
    return;
  }
  if (!hasEnabledOps(job)) {
    showError(
      hasPart(job)
        ? "Nothing to generate: the project has no enabled operation. Add one in the project tree (or use a detected feature), or enable one."
        : "Nothing to generate yet: import an STL (it adds a starter roughing and finishing) or add an operation in the project tree.",
    );
    return;
  }
  if (job.part?.mesh && !state.mesh) {
    showError(`This job uses the mesh "${job.part.mesh.file}". Choose it under "Part mesh".`);
    return;
  }
  $("generate").disabled = $("simulate").disabled = true;
  setStatus("Generating...", "busy");
  const run = (state.genRun = (state.genRun || 0) + 1);
  try {
    const setupId = state.tree?.setup;
    const { result, ms } = await engine("planAndPost", job, machineArg(), state.mesh, { max_points: 150000, libraries: libraries(), setup: job.schema_version === "0.2" ? setupId : undefined });
    // an edit since then (auto-regenerate) has asked for a newer run: this one is dropped
    if (run !== state.genRun) return;
    state.result = result;
    state.sim = null;
    state.job = job;
    state.stale = false;
    document.body.classList.remove("stale");
    renderTabs();
    selectTab(0);
    const issues = result.programs.reduce((n, p) => n + p.issues.length, 0);
    // the tree shows each operation's and file's estimated time
    await window.__marcam.tree?.refresh();
    updateSimResInfo();
    // done only once everything is shown and the buttons work again
    $("generate").disabled = $("simulate").disabled = false;
    updateGenerateButton();
    // what was planned (operations unchanged since the last plan are reused)
    const pc = result.plan_cache;
    const what = pc && pc.reused ? ` (planned ${pc.planned} operation${pc.planned === 1 ? "" : "s"}, ${pc.reused} unchanged)` : "";
    setStatus(`Generated ${result.programs.length} program(s) in ${(ms / 1000).toFixed(1)} s${what}, ${issues} issue(s); estimated machining time ${fmtTime(result.time_total_s)}`, "done");
  } catch (e) {
    showError(e.message);
    setStatus("Failed", "error");
  } finally {
    $("generate").disabled = $("simulate").disabled = false;
  }
}

// ---------------------------------------------------------------- simulation resolution
/** The grid asked for: a preset, or custom cell sizes (rotary mm x degrees, 3-axis mm). */
function simOpts() {
  const v = $("simRes").value;
  // operations hidden in the tree are not simulated (their material stays)
  const hide = hiddenPlanned();
  if (v !== "custom") return { res: v, hide };
  const deg = Number($("simDeg").value) || 1.5;
  return { dx: Number($("simDx").value) || 0.1, na: Math.max(8, Math.round(360 / deg)), cell: Number($("simCell").value) || 0.25, hide };
}

/** The planning numbers of the operations hidden in the tree (those the engine planned: enabled,
 *  not in error, their tool in the project), in the setup the results are for. */
function hiddenPlanned() {
  if (!state.hidden?.size) return [];
  const j = state.job;
  const setupId = state.tree?.setup;
  const s = j?.schema_version === "0.2" ? j.setups?.find((x) => x.id === setupId) ?? j.setups?.[0] : null;
  if (!s) return [];
  const tools = new Set((j.tools || []).map((t) => t.number));
  const planned = s.outputs.flatMap((o) => o.operations).map((id) => s.operations.find((o) => o.id === id)).filter((o) => o && o.enabled !== false && !o.error && tools.has(o.tool));
  return planned.map((o, i) => (state.hidden.has(`${s.id}/${o.id}`) ? i : -1)).filter((i) => i >= 0);
}

function saveSimRes() {
  try {
    localStorage.setItem("marcam.simRes", JSON.stringify({ v: $("simRes").value, dx: $("simDx").value, deg: $("simDeg").value, cell: $("simCell").value, gpu: $("simGpu").checked }));
  } catch {}
}

/** The GPU to simulate on (null: the WASM CPU simulation) and why not, for the label. */
async function simGpu() {
  if (!$("simGpu").checked) return { gpu: null, why: "GPU off" };
  if (!navigator.gpu) return { gpu: null, why: "no WebGPU in this browser" };
  const gpu = await gpuDevice();
  return gpu ? { gpu, why: "" } : { gpu: null, why: "no GPU adapter" };
}

function showBackend(text) {
  state.simBackend = text;
  $("simBackend").textContent = text ? `last run: ${text}` : "";
}

function loadSimRes() {
  try {
    const s = JSON.parse(localStorage.getItem("marcam.simRes") || "null");
    if (s) {
      $("simRes").value = s.v;
      $("simDx").value = s.dx;
      $("simDeg").value = s.deg;
      $("simCell").value = s.cell;
      // (the GPU is not a saved setting: it is used when there is one; see ?gpu=off)
    }
  } catch {}
}

/** Show the grid the setting gives for this job: cells, memory, time against Normal. */
async function updateSimResInfo() {
  $("simResCustom").classList.toggle("hidden", $("simRes").value !== "custom");
  let job;
  try {
    job = readJob();
  } catch {
    return;
  }
  const base = { libraries: libraries(), setup: job.schema_version === "0.2" ? state.tree?.setup : undefined };
  try {
    const [{ result: g }, { result: n }] = await Promise.all([
      engine("simGrid", job, { ...base, ...simOpts() }, state.mesh),
      engine("simGrid", job, { ...base, res: "normal" }, state.mesh),
    ]);
    state.simGrid = g;
    const ratio = g.cells / Math.max(1, n.cells);
    const what = g.kind === "xyz" ? `${g.nx} x ${g.ny} cells of ${g.cell.toFixed(3)} mm` : `${g.nx} x ${g.na} cells (${g.dx} mm x ${(360 / g.na).toFixed(2)}\u00b0)`;
    const warn = g.too_large ? " - too fine (limit 40 million cells)" : g.cells > 5e6 ? " - very large: it may take minutes and a lot of memory" : "";
    $("simResInfo").textContent = `${what} = ${g.cells.toLocaleString("en")} cells, about ${Math.max(1, Math.round(g.engine_mb + g.display_mb))} MB, roughly ${ratio < 1.5 ? ratio.toFixed(1) : Math.round(ratio)}x the Normal time${warn}`;
    $("simResInfo").classList.toggle("error", !!warn);
  } catch {
    state.simGrid = null;
    $("simResInfo").textContent = "";
  }
}

/** Before a simulation: refuse grids over the limit, ask about very large ones. */
async function confirmSimRes() {
  await updateSimResInfo();
  const g = state.simGrid;
  if (!g) return true;
  if (g.too_large) {
    showError(`The simulation grid would have ${g.cells.toLocaleString("en")} cells (limit 40 million): choose a coarser setting.`);
    return false;
  }
  if (g.cells > 5e6) return window.confirm(`The simulation grid has ${g.cells.toLocaleString("en")} cells (about ${Math.round(g.engine_mb + g.display_mb)} MB). It may take minutes. Continue?`);
  return true;
}

for (const id of ["simRes", "simDx", "simDeg", "simCell", "simGpu"]) {
  $(id).addEventListener("change", () => {
    saveSimRes();
    updateSimResInfo();
  });
}
loadSimRes();

async function simulate() {
  if (!state.result) await generate();
  if (!state.result) return;
  // busy at once (the grid check below is asynchronous)
  setStatus("Checking the simulation grid...", "busy");
  if (!(await confirmSimRes())) {
    setStatus("Simulation not started", "done");
    return;
  }
  $("generate").disabled = $("simulate").disabled = true;
  setStatus("Simulating stock...", "busy");
  try {
    const t0 = performance.now();
    let result = null;
    const epoch = state.epoch;
    const { gpu, why } = await simGpu();
    let backend = `CPU (${why})`;
    if (gpu) {
      try {
        const { result: open } = await engine("gpuOpen", state.job, { ...simOpts(), libraries: libraries() }, state.mesh);
        // (gpuOpen plans through the same cache and leaves the hidden operations out)
        result = await simulateOnGpu(gpu, open, async (o, bytes) => (await engine("gpuStats", o, bytes)).result);
        backend = `GPU (${gpu.name})`;
      } catch (e) {
        // the engine refuses some jobs on the GPU (indexed 3+1 moves): the CPU does them
        backend = `CPU (${e.message})`;
      }
    }
    if (!result) result = (await engine("simulate", state.job, simOpts(), state.mesh)).result;
    const ms = performance.now() - t0;
    if (epoch !== state.epoch) return;
    showBackend(backend);
    result.stocks.forEach((st) => (st.hidden = hiddenPlanned().length));
    state.sim = result;
    selectTab(state.tab);
    const s = result.stocks[state.tab];
    setStatus(`Simulated in ${(ms / 1000).toFixed(1)} s`, "done");
    const hits = result.stocks.reduce((n, st) => n + (st.collisions?.count || 0), 0);
    if (s && (s.rapid_hits || s.gouge_cells || hits)) showError(`Simulation: ${s.rapid_hits} rapid(s) through stock, ${s.gouge_cells} gouged cell(s), ${hits} holder/shank collision(s)`);
  } catch (e) {
    showError(e.message);
    setStatus("Simulation failed", "error");
  } finally {
    $("generate").disabled = $("simulate").disabled = false;
  }
}

// ---------------------------------------------------------------- results
function collisionHtml(c) {
  if (!c) return "";
  if (!c.count) return `<div class="small" id="holderCheck">Holder/shank check: clear</div>`;
  const rows = c.collisions.slice(0, 20).map((k) =>
    `<li>line ${k.line ?? "?"}: T${k.tool} ${k.contact} ${k.depth.toFixed(3)} mm into the stock${k.rapid ? " (rapid)" : ""}</li>`).join("");
  return `<div class="issues bad" id="holderCheck">Holder/shank collision: ${c.count} move(s), deepest ${c.max_depth.toFixed(3)} mm<ul>${rows}</ul></div>`;
}

function renderTabs() {
  const tabs = $("tabs");
  tabs.innerHTML = "";
  state.result.programs.forEach((p, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "tab");
    b.dataset.index = i;
    const n = p.issues.length + (p.collisions?.count ? 1 : 0);
    b.innerHTML = `${p.name}<span class="badge ${n ? "bad" : "ok"}">${n ? `${n} issue${n > 1 ? "s" : ""}` : "OK"}</span>`;
    b.addEventListener("click", () => selectTab(i));
    tabs.appendChild(b);
  });
}

function selectTab(i) {
  state.tab = i;
  const p = state.result.programs[i];
  for (const b of $("tabs").children) b.setAttribute("aria-selected", String(+b.dataset.index === i));
  const g = p.gouge;
  const sim = state.sim?.stocks?.[i];
  const esc = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
  $("summary").innerHTML = `
    ${state.stale ? '<div class="stale-banner" id="staleBanner">Out of date: the project changed since these results were made. <button type="button" id="staleRegen">Generate again</button></div>' : ""}
    <div><strong>${esc(p.title)}</strong> &middot; <span id="lineCount">${p.lines.toLocaleString("en")}</span> lines
      &middot; ${state.result.machine}${p.tool ? ` &middot; T${p.tool[0]} ${esc(p.tool[1])}` : ""}</div>
    <div class="issues ${p.issues.length ? "bad" : "ok"}">${
      p.issues.length
        ? `${p.issues.length} verifier issue(s):<ul>${p.issues.slice(0, 200).map((s) => `<li>${esc(s)}</li>`).join("")}</ul>`
        : "Verifier: 0 issues (no fast plunges, no low rapids)"
    }</div>
    ${limitsHtml(p.name, esc)}
    ${p.time ? `<div class="small" id="timeEst">Estimated time: <strong>${fmtTime(p.time.total_s)}</strong> (cutting ${fmtTime(p.time.feed_s)}, rapids ${fmtTime(p.time.rapid_s)}, tool changes/dwells ${fmtTime(p.time.other_s)})${state.result.programs.length > 1 ? ` &middot; all files ${fmtTime(state.result.time_total_s)}` : ""}</div>` : ""}
    ${collisionHtml(p.collisions)}
    ${g && g.moves_checked ? `<div class="small">Gouge check: ${g.moves_checked.toLocaleString("en")} cut moves, deepest ${g.max_depth.toFixed(4)} mm into the part</div>` : ""}
    ${sim && !sim.samples ? `<div class="small muted">Not simulated: its tool has no full shape (a V-bit needs a diameter). The stock shown is from the programs before it.</div>` : ""}
    ${sim && sim.hidden ? `<div class="small muted" id="simHidden">${sim.hidden} operation${sim.hidden === 1 ? "" : "s"} hidden: not simulated (their material is left)</div>` : ""}
    ${sim ? `<div class="small">Simulated stock: material left max ${sim.max_left.toFixed(3)} mm, mean ${sim.mean_left.toFixed(3)} mm; deepest gouge ${sim.max_gouge.toFixed(3)} mm; ${sim.rapid_hits} rapid(s) through stock${sim.feature_cells ? `; ${sim.feature_cells.toLocaleString("en")} cells cut below the part on purpose (engraving, knurl, indexed features) not counted` : ""}</div>` : ""}`;
  $("gcode").value = p.gcode;
  $("codeInfo").textContent = `${p.name}.nc, ${(p.gcode.length / 1024).toFixed(0)} KiB`;
  $("download").disabled = false;
  const n = p.toolpath.points.length / 4;
  const sc = $("scrub");
  sc.max = String(n);
  sc.value = String(n);
  sc.disabled = n === 0;
  updateScrubLabel();
  drawSide();
  updateView();
}

function download() {
  const p = state.result?.programs[state.tab];
  if (!p) return;
  const url = URL.createObjectURL(new Blob([p.gcode], { type: "text/plain" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = `${p.name}.nc`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function updateScrubLabel() {
  const p = state.result?.programs[state.tab];
  if (!p) return;
  const n = +$("scrub").value;
  const pts = p.toolpath.points;
  const q = 4 * (n - 1);
  const at = n <= 0 ? "" : pts[q + 3] >= 2
    ? `X${pts[q].toFixed(2)} Y${pts[q + 1].toFixed(2)} Z${pts[q + 2].toFixed(2)}`
    : `X${pts[q].toFixed(2)} Z${pts[q + 1].toFixed(2)} A${pts[q + 2].toFixed(1)}`;
  const thin = p.toolpath.thinned ? ` (thinned from ${p.toolpath.total.toLocaleString("en")})` : "";
  $("scrubLabel").textContent = `${n.toLocaleString("en")} / ${(pts.length / 4).toLocaleString("en")}${thin} ${at}`;
}

// ---------------------------------------------------------------- 2D side view
function drawSide() {
  const cv = $("side");
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth, h = cv.clientHeight;
  cv.width = w * dpr;
  cv.height = h * dpr;
  const ctx = cv.getContext("2d");
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);
  const res = state.result;
  if (!res) return;
  const p = res.programs[state.tab];
  const pts = p.toolpath.points;
  const n = +$("scrub").value;
  const sb = res.stock_box;
  const hide = hiddenMask(p);
  if (sb) return drawSideXYZ(ctx, w, h, p, pts, n, sb, hide);
  let x0 = Infinity, x1 = -Infinity;
  for (let i = 0; i < pts.length; i += 4) if (pts[i + 3] % 2) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); }
  const prof = res.profile?.points || [];
  for (const [x] of prof) { x0 = Math.min(x0, x); x1 = Math.max(x1, x); }
  if (!Number.isFinite(x0)) { x0 = -1; x1 = 1; }
  const rTop = Math.max(p.safe_r, res.stock.radius) + 0.5;
  const pad = 30, sx = (x) => pad + ((x - x0) / (x1 - x0 || 1)) * (w - 2 * pad), sy = (r) => h - 18 - (r / rTop) * (h - 30);
  const css = getComputedStyle(document.body);
  ctx.font = "11px system-ui";
  ctx.fillStyle = css.getPropertyValue("--muted");
  ctx.fillText("side view: X vs radius (grey: part, line: stock)", 6, 12);
  if (prof.length) {
    ctx.fillStyle = "rgba(150,150,150,0.45)";
    ctx.beginPath();
    ctx.moveTo(sx(prof[0][0]), sy(0));
    for (const [x, r] of prof) ctx.lineTo(sx(x), sy(r));
    ctx.lineTo(sx(prof[prof.length - 1][0]), sy(0));
    ctx.fill();
  }
  ctx.strokeStyle = "rgba(120,120,120,0.8)";
  ctx.beginPath();
  ctx.moveTo(sx(x0), sy(res.stock.radius));
  ctx.lineTo(sx(x1), sy(res.stock.radius));
  ctx.stroke();
  ctx.lineWidth = 1;
  for (let i = 1; i < Math.min(n, pts.length / 4); i++) {
    const cut = pts[4 * i + 3];
    if (hide && hide[i]) continue;
    if (!cut && !$("showRapids").checked) continue;
    ctx.strokeStyle = cut ? "rgba(37,99,235,0.35)" : "rgba(220,38,38,0.35)";
    ctx.beginPath();
    ctx.moveTo(sx(pts[4 * (i - 1)]), sy(pts[4 * (i - 1) + 1]));
    ctx.lineTo(sx(pts[4 * i]), sy(pts[4 * i + 1]));
    ctx.stroke();
  }
}

/** Side view of a 3-axis program: X vs Z, with the stock block. */
function drawSideXYZ(ctx, w, h, p, pts, n, sb, hide) {
  const x0 = sb.min[0] - 3, x1 = sb.max[0] + 3;
  const z0 = sb.min[2] - 1, z1 = Math.max(p.safe_r, sb.max[2]) + 1;
  const pad = 30, sx = (x) => pad + ((x - x0) / (x1 - x0)) * (w - 2 * pad), sz = (z) => h - 18 - ((z - z0) / (z1 - z0)) * (h - 30);
  ctx.font = "11px system-ui";
  ctx.fillStyle = getComputedStyle(document.body).getPropertyValue("--muted");
  ctx.fillText("side view: X vs Z (box: stock)", 6, 12);
  ctx.fillStyle = "rgba(150,150,150,0.25)";
  ctx.fillRect(sx(sb.min[0]), sz(sb.max[2]), sx(sb.max[0]) - sx(sb.min[0]), sz(sb.min[2]) - sz(sb.max[2]));
  ctx.lineWidth = 1;
  for (let i = 1; i < Math.min(n, pts.length / 4); i++) {
    const cut = pts[4 * i + 3] % 2;
    if (hide && hide[i]) continue;
    if (!cut && !$("showRapids").checked) continue;
    ctx.strokeStyle = cut ? "rgba(37,99,235,0.35)" : "rgba(220,38,38,0.35)";
    ctx.beginPath();
    ctx.moveTo(sx(pts[4 * (i - 1)]), sz(pts[4 * (i - 1) + 2]));
    ctx.lineTo(sx(pts[4 * i]), sz(pts[4 * i + 2]));
    ctx.stroke();
  }
}

// ---------------------------------------------------------------- 3D view (three.js)
async function getThree() {
  if (state.three) return state.three;
  try {
    const THREE = await import("three");
    const { OrbitControls } = await import("three/addons/controls/OrbitControls.js");
    const host = $("view3d");
    const renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    host.appendChild(renderer.domElement);
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 2000);
    camera.up.set(0, 0, 1);
    const controls = new OrbitControls(camera, renderer.domElement);
    scene.add(new THREE.AmbientLight(0xffffff, 0.6));
    const sun = new THREE.DirectionalLight(0xffffff, 1.2);
    sun.position.set(20, -30, 40);
    scene.add(sun);
    const axes = new THREE.AxesHelper(5);
    scene.add(axes);
    const groups = { part: new THREE.Group(), stock: new THREE.Group(), path: new THREE.Group(), sim: new THREE.Group(), tool: new THREE.Group() };
    for (const g of Object.values(groups)) scene.add(g);
    const resize = () => {
      const w = host.clientWidth, h = host.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / Math.max(h, 1);
      camera.updateProjectionMatrix();
    };
    new ResizeObserver(resize).observe(host);
    resize();
    // the view cube (upper right): view only, it turns the camera
    const cube = addViewCube({ THREE, renderer, camera, controls }, host);
    const loop = () => {
      controls.update();
      renderer.render(scene, camera);
      cube.render();
      requestAnimationFrame(loop);
    };
    loop();
    state.three = { THREE, renderer, scene, camera, controls, groups, axes, fitted: null, cube };
    window.__marcam.viewCube = cube;
    $("viewNote").textContent = "Drag to orbit, scroll to zoom. Blue: cuts, red: rapids. The part turns, so the path is drawn around it (A rotates about X).";
  } catch (e) {
    state.three = { failed: true };
    $("viewNote").textContent = `3D preview unavailable (three.js could not be loaded from cdn.jsdelivr.net: ${e.message}). Everything else works.`;
  }
  return state.three;
}

function clear(group) {
  for (const c of [...group.children]) {
    group.remove(c);
    // (an object may be a group: the stock is its solid and its outline)
    c.traverse((o) => {
      o.geometry?.dispose();
      for (const mat of [].concat(o.material || [])) mat.dispose();
    });
  }
}

/** Polyline point i -> xyz: 3-axis points (kind 2/3) are already work XYZ. */
function pointXYZ(pts, i) {
  return pts[4 * i + 3] >= 2 ? [pts[4 * i], pts[4 * i + 1], pts[4 * i + 2]] : toXYZ(pts[4 * i], pts[4 * i + 1], pts[4 * i + 2]);
}

/** Machine (x, r, a) -> part frame xyz: the tool at A sits over part angle 90 - A. */
function toXYZ(x, r, aDeg) {
  const phi = ((90 - aDeg) * Math.PI) / 180;
  return [x, r * Math.cos(phi), r * Math.sin(phi)];
}

/**
 * The current setup's part and stock, from the project itself: shown with or without
 * results (a project with no operation, one just opened, one whose results were cleared).
 */
async function drawModel() {
  const v = await getThree();
  if (!v || v.failed) return;
  let doc;
  try {
    doc = readJob();
  } catch {
    return;
  }
  const { THREE, groups } = v;
  const v2 = doc.schema_version === "0.2";
  const setup = v2 ? doc.setups?.find((s) => s.id === state.tree?.setup) || doc.setups?.[0] : null;
  const ids = v2 ? (setup?.part?.length ? setup.part : doc.models?.map((m) => m.id) || []) : [];
  const models = v2 ? (doc.models || []).filter((m) => ids.includes(m.id)) : [];
  const key = JSON.stringify([setup?.id, setup?.stock, models, v2 ? null : [doc.stock, doc.part], state.mesh?.length || 0]);
  if (v.modelKey === key) return;
  v.modelKey = key;
  clear(groups.stock);
  clear(groups.part);
  // the part: a mesh model (as the engine places it) or a revolved profile
  const partMat = new THREE.MeshStandardMaterial({ color: 0xc8ccd2, metalness: 0.3, roughness: 0.5 });
  const mesh = v2 ? models.find((m) => m.kind === "mesh") : doc.part?.mesh;
  const prof = v2 ? models.find((m) => m.kind === "profile")?.points : doc.part?.profile?.points;
  let bounds = null;
  if (mesh && state.mesh) {
    try {
      const { result } = await engine("meshTriangles", doc, { max_triangles: 150000, setup: setup?.id }, state.mesh);
      if (v.modelKey !== key) return; // superseded meanwhile
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.Float32BufferAttribute(result.triangles, 3));
      g.computeVertexNormals();
      const meshMat = partMat.clone();
      meshMat.side = THREE.DoubleSide; // a user's mesh may have inconsistent winding
      groups.part.add(new THREE.Mesh(g, meshMat));
      bounds = result.bounds;
    } catch {
      // a mesh the engine cannot read: the stock alone
    }
  } else if (prof && prof.length > 1) {
    groups.part.add(new THREE.Mesh(toGeometry(THREE, revolveProfile(prof, 120)), partMat));
    bounds = [[Math.min(...prof.map((q) => q[0])), 0, 0], [Math.max(...prof.map((q) => q[0])), 0, 0]];
  }
  // the stock: the setup's block or bar (a 0.1 job's bar)
  // (a light tint with its edges drawn: see meshes.js stockObject)
  const st = v2 ? setup?.stock : doc.stock ? { shape: "cylinder", ...doc.stock } : null;
  const sb = st?.shape === "box" ? st : null;
  let fitKey = null;
  if (sb) {
    const size = [0, 1, 2].map((k) => sb.max[k] - sb.min[k]);
    const box = stockObject(THREE, new THREE.BoxGeometry(...size));
    box.position.set(...[0, 1, 2].map((k) => (sb.min[k] + sb.max[k]) / 2));
    groups.stock.add(box);
    fitKey = JSON.stringify(sb);
    v.bounds = { xmin: sb.min[0], xmax: sb.max[0], R: Math.max(sb.max[1] - sb.min[1], sb.max[2] - sb.min[2]) / 2 };
  } else if (st?.radius) {
    // the bar: from the part's front face back by its length
    const front = Math.max(0, bounds ? bounds[1][0] : 0);
    const xmin = front - (st.length || 30) - 1, xmax = front + 1;
    groups.stock.add(stockObject(THREE, toGeometry(THREE, cylinder(st.radius, xmin, xmax, 96))));
    v.bounds = { xmin, xmax, R: st.radius };
    fitKey = JSON.stringify([st.radius, Math.round(xmin), Math.round(xmax)]);
  }
  // frame the view when the stock changes; keep the person's orbit otherwise
  if (fitKey && v.fitted !== fitKey && v.fitted !== "picking") {
    if (sb) {
      const c = [0, 1, 2].map((k) => (sb.min[k] + sb.max[k]) / 2), span = Math.max(sb.max[0] - sb.min[0], sb.max[1] - sb.min[1]);
      v.camera.position.set(c[0] - span * 0.3, c[1] - span * 1.2, c[2] + span * 0.9);
      v.controls.target.set(...c);
    } else {
      const { xmin, xmax, R } = v.bounds;
      const span = xmax - xmin;
      v.camera.position.set((xmin + xmax) / 2 - span * 0.2, -span * 1.3, R * 2.2);
      v.controls.target.set((xmin + xmax) / 2, 0, 0);
    }
    v.controls.update();
    v.fitted = fitKey;
  }
  $("viewNote").textContent = sb
    ? "Drag to orbit, scroll to zoom. Blue: cuts, red: rapids. Vise setup: the block stays put and the tool moves in X, Y and Z (Z0 = top of the stock)."
    : "Drag to orbit, scroll to zoom. Blue: cuts, red: rapids. The part turns, so the path is drawn around it (A rotates about X).";
  groups.part.visible = $("showPart").checked && !(groups.sim.children.length && groups.sim.visible);
  groups.stock.visible = $("showStock").checked && !(groups.sim.children.length && groups.sim.visible);
  v.renderer.render(v.scene, v.camera);
}

async function updateView() {
  const v = await getThree();
  if (!v || v.failed || !state.result) return;
  const { THREE, groups } = v;
  const res = state.result;
  const p = res.programs[state.tab];
  const key = `${state.tab}:${res.programs.length}:${p.lines}:${[...state.hidden].join(",")}`;
  const hide = hiddenMask(p);

  if (v.key !== key) {
    v.key = key;
    // toolpath as two line-segment sets with prefix counts for the scrubber
    clear(groups.path);
    const pts = p.toolpath.points;
    const n = pts.length / 4;
    const cut = [], rap = [];
    const cutUpTo = new Uint32Array(n + 1), rapUpTo = new Uint32Array(n + 1);
    for (let i = 1; i < n; i++) {
      if (hide && hide[i]) {
        cutUpTo[i + 1] = cut.length / 6;
        rapUpTo[i + 1] = rap.length / 6;
        continue;
      }
      const a = pointXYZ(pts, i - 1);
      const b = pointXYZ(pts, i);
      (pts[4 * i + 3] % 2 ? cut : rap).push(...a, ...b);
      cutUpTo[i + 1] = cut.length / 6;
      rapUpTo[i + 1] = rap.length / 6;
    }
    const lines = (arr, color, opacity) => {
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.Float32BufferAttribute(arr, 3));
      return new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color, transparent: true, opacity }));
    };
    v.cutLines = lines(cut, 0x2563eb, 0.85);
    v.rapLines = lines(rap, 0xdc2626, 0.5);
    v.cutUpTo = cutUpTo;
    v.rapUpTo = rapUpTo;
    groups.path.add(v.cutLines, v.rapLines);

    // the part and the stock come from the project (drawModel), results or not
    v.simKey = null;
    await drawModel();
  }

  // simulated stock surface for this program
  const simKey = state.sim ? `${key}:sim` : null;
  if (v.simKey !== simKey) {
    v.simKey = simKey;
    clear(groups.sim);
    const s = state.sim?.stocks?.[state.tab];
    if (s) groups.sim.add(simMesh(THREE, state.sim, s));
  }
  groups.sim.visible = $("showSim").checked;
  groups.part.visible = $("showPart").checked && !(groups.sim.children.length && groups.sim.visible);
  groups.stock.visible = $("showStock").checked && !(groups.sim.children.length && groups.sim.visible);
  v.rapLines.visible = $("showRapids").checked;
  if (state.machining?.open) for (const k of ["part", "stock", "sim", "path"]) groups[k].visible = false;

  // scrubber
  const k = +$("scrub").value;
  v.cutLines.geometry.setDrawRange(0, 2 * v.cutUpTo[Math.min(k, v.cutUpTo.length - 1)]);
  v.rapLines.geometry.setDrawRange(0, 2 * v.rapUpTo[Math.min(k, v.rapUpTo.length - 1)]);
  clear(groups.tool);
  const pts = p.toolpath.points;
  if (k > 0) {
    const [x, y, z] = pointXYZ(pts, k - 1);
    const tip = new THREE.Mesh(new THREE.SphereGeometry(0.4, 16, 12), new THREE.MeshBasicMaterial({ color: 0xf59e0b }));
    tip.position.set(x, y, z);
    groups.tool.add(tip);
  }
}

function ramp(t) {
  t = Math.max(0, Math.min(1, t));
  const stops = [[40, 60, 200], [30, 190, 210], [240, 210, 40], [210, 40, 40]];
  const f = t * 3, i = Math.min(2, Math.floor(f)), u = f - i;
  return stops[i].map((c, k) => (c * (1 - u) + stops[i + 1][k] * u) / 255);
}

/** Indexed geometry from a web/meshes.js mesh, with normals (and colours if present). */
function toGeometry(THREE, m) {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(m.positions, 3));
  if (m.colors) g.setAttribute("color", new THREE.BufferAttribute(m.colors, 3));
  g.setIndex(new THREE.BufferAttribute(m.indices, 1));
  g.computeVertexNormals();
  return g;
}

/** Simulated stock -> closed solid (a bar around X, or a 3-axis block), coloured by material
 * left over the part (grey where there is no part). */
function simMesh(THREE, sim, s) {
  const r = sim.kind === "xyz" ? s.z : s.r;
  const has = (k) => (sim.kind === "xyz" ? sim.part[k] != null : sim.part[k] > 0);
  // where there is no part: remaining stock, tinted (not the part's grey)
  const color = (k) => (has(k) ? ramp((r[k] - sim.part[k]) / 0.5) : STOCK_LEFT);
  const m = sim.kind === "xyz" ? blockSolid(sim.grid, r, sim.stock_box.min[2], color) : heightmapSolid(sim.grid, r, color);
  return new THREE.Mesh(toGeometry(THREE, m), new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.7 }));
}

// ---------------------------------------------------------------- wiring
$("loadExample").addEventListener("click", () => {
  if (!window.__marcam.project.mayDiscard("Load the example")) return;
  loadExample($("example").value).catch((e) => {
    showError(e.message);
    setStatus("Loading failed", "error");
  });
});
$("job").addEventListener("input", () => window.__marcam.project.changed());
$("summary").addEventListener("click", (ev) => {
  if (ev.target.id === "staleRegen") generate();
  if (ev.target.dataset?.empty === "import") $("importStl").click();
  if (ev.target.dataset?.empty === "open") $("projectOpen").click();
  if (ev.target.dataset?.empty === "generate") generate();
});
$("generate").addEventListener("click", generate);
$("simulate").addEventListener("click", simulate);
$("download").addEventListener("click", download);
$("job").addEventListener("change", renderQuick);
$("machine").addEventListener("change", () => {
  const custom = $("machine").value === "custom";
  $("machineJson").classList.toggle("hidden", !custom);
  if (custom && !$("machineJson").value) {
    const z1 = state.registry.machines.find((m) => m.id === "makera-z1-4axis");
    $("machineJson").value = JSON.stringify(z1, null, 2);
  }
  store("marcam.machineSel", $("machine").value);
  renderMachineInfo();
  // the project remembers a built-in machine on its setup
  try {
    const doc = readJob();
    const id = $("machine").value;
    const s = doc.setups?.find((x) => x.id === state.tree?.setup) ?? doc.setups?.[0];
    if (s && state.registry.machines.some((m) => m.id === id) && s.machine !== id) {
      s.machine = id;
      writeJob(doc);
      window.__marcam.tree?.refresh();
    } else window.__marcam.project?.changed();
  } catch {
    window.__marcam.project?.changed();
  }
});
$("mesh").addEventListener("change", async () => {
  const f = $("mesh").files[0];
  if (f) {
    const bytes = new Uint8Array(await f.arrayBuffer());
    try {
      const { isStep } = await import("./step.js");
      if (isStep(bytes, f.name)) {
        const { readPart } = await import("./project.js");
        const part = await readPart(bytes, f.name);
        setMesh(part.step.stl, f.name, { name: f.name, bytes });
      } else setMesh(bytes, f.name);
    } catch (e) {
      return showError(`${f.name}: ${e.message}`);
    }
    window.__marcam.project.changed();
  }
});
$("scrub").addEventListener("input", () => {
  updateScrubLabel();
  drawSide();
  updateView();
});
for (const id of ["showRapids", "showStock", "showPart", "showSim"]) $(id).addEventListener("change", () => { drawSide(); updateView(); });
window.addEventListener("resize", drawSide);

(async () => {
  try {
    const { result } = await engine("version");
    $("version").textContent = `v${result.version}`;
    state.registry = (await engine("profileList")).result;
    initLibraries();
    renderMachines(load("marcam.machineSel", "makera-z1-4axis"));
    renderLibraries();
    await loadExample("knob");
    setMesh(null, "");
    $("generate").disabled = $("simulate").disabled = false;
    setStatus("Ready", "ready");
  } catch (e) {
    setStatus("Engine failed to load", "error");
    showError(`${e.message}\nBuild it first: scripts/build-wasm.sh (or .ps1)`);
  }
})();

// ---------------------------------------------------------------- machines and tool libraries
function load(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    return v == null ? fallback : JSON.parse(v);
  } catch {
    return fallback;
  }
}
function store(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {}
}
const userMachines = () => load("marcam.machines", []);
const libraries = () => load("marcam.libraries", []);

/** First run: "My tools" starts as a copy of the Makera starter library. */
function initLibraries() {
  if (libraries().length) return;
  const starter = state.registry.libraries.find((l) => l.id === "makera-starter");
  store("marcam.libraries", [{ ...structuredClone(starter), id: "my-tools", name: "My tools (from Makera starter)" }]);
}

function renderMachines(selected) {
  const sel = $("machine");
  sel.innerHTML = "";
  const opt = (value, text) => {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    sel.appendChild(o);
  };
  for (const m of state.registry.machines) opt(m.id, m.name);
  for (const m of userMachines()) opt(`user:${m.id}`, `${m.name} (imported)`);
  opt("custom", "Custom JSON...");
  const alias = { "makera-z1": "makera-z1-4axis", z1: "makera-z1-4axis", carvera: "makera-carvera-4axis", "carvera-air": "makera-carvera-air-4axis", linuxcnc: "linuxcnc-4axis" };
  const want = alias[selected] || selected;
  const values = [...sel.options].map((o) => o.value);
  sel.value = values.includes(want) ? want : values.includes(`user:${want}`) ? `user:${want}` : "makera-z1-4axis";
  $("machineJson").classList.toggle("hidden", sel.value !== "custom");
  renderMachineInfo();
}

function renderMachineInfo() {
  const m = currentMachine();
  if (!m) {
    $("machineInfo").textContent = "";
    return;
  }
  const tr = (l) => {
    const a = m.kinematics.linear.find((x) => x.letter === l);
    return a && a.min != null && a.max != null ? `${l} ${a.max - a.min} mm` : null;
  };
  const parts = [
    [tr("X"), tr("Y"), tr("Z")].filter(Boolean).join(", "),
    m.spindle ? `spindle ${m.spindle.min_rpm}-${m.spindle.max_rpm} rpm` : "",
    m.max_feed ? `feed cap ${m.max_feed} mm/min` : "",
    m.rotary_envelope ? `4th axis ${m.rotary_envelope.max_diameter} x ${m.rotary_envelope.max_length} mm` : "",
    m.tool_changer ? (m.tool_changer.kind === "atc" ? `ATC ${m.tool_changer.slots ?? ""} slots` : "manual tool change") : "",
    `feeds: ${m.post.feed_mode.replace(/_/g, " ")}`,
  ].filter(Boolean);
  $("machineInfo").textContent = parts.join(" · ");
}

function limitsHtml(program, esc) {
  const lim = (state.result.limits || []).filter((i) => !i.program || i.program === program);
  if (!lim.length) return `<div class="small" id="limits">Machine limits: OK for ${esc(state.result.machine)}</div>`;
  const bad = lim.some((i) => i.severity === "error");
  return `<div id="limits" class="limits ${bad ? "bad" : ""}">Machine limits (${esc(state.result.machine)}):<ul>${lim
    .map((i) => `<li>${i.severity}: ${esc(i.message)}</li>`)
    .join("")}</ul></div>`;
}

let editing = null; // index of the tool being edited, or -1 for a new one

function currentLibrary() {
  const libs = libraries();
  return libs.find((l) => l.id === $("library").value) || libs[0];
}

function saveLibrary(lib) {
  store("marcam.libraries", libraries().map((l) => (l.id === lib.id ? lib : l)));
}

function renderLibraries(selected) {
  const sel = $("library");
  const keep = selected || sel.value || load("marcam.libSel", "my-tools");
  sel.innerHTML = "";
  for (const l of libraries()) {
    const o = document.createElement("option");
    o.value = l.id;
    o.textContent = `${l.name} (${l.tools.length})`;
    sel.appendChild(o);
  }
  if ([...sel.options].some((o) => o.value === keep)) sel.value = keep;
  renderTools();
}

const shapeText = (s) =>
  s.kind === "vbit" ? `V ${s.angle}° Ø${s.diameter}`
  : s.kind === "bull" ? `bull Ø${s.diameter} r${s.corner_radius}`
  : s.kind === "tapered_ball" ? `tapered ball tip Ø${s.tip_diameter} ${s.taper_angle}°/side Ø${s.diameter}`
  : `${s.kind} Ø${s.diameter}`;

function renderTools() {
  const lib = currentLibrary();
  const tbl = $("toolTable");
  tbl.innerHTML = "<tr><th>Tool</th><th>Shape</th><th>#</th><th></th></tr>";
  if (!lib) return;
  let jobTools = [];
  try {
    jobTools = (readJob().tools || []).map((t) => t.number);
  } catch {}
  lib.tools.forEach((t, i) => {
    const tr = document.createElement("tr");
    tr.dataset.tool = t.id;
    const esc = (x) => String(x).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
    const use = `<select data-use="${i}" aria-label="Use ${esc(t.name)} as"><option value="">use as...</option>${jobTools
      .map((n) => `<option value="${n}">T${n}</option>`)
      .join("")}<option value="new">new T</option></select>`;
    tr.innerHTML = `<td>${esc(t.name)}</td><td>${esc(shapeText(t.shape))}</td><td>${t.number ?? ""}</td>
      <td>${use} <button type="button" data-edit="${i}">Edit</button> <button type="button" data-del="${i}" aria-label="Remove ${esc(t.name)}">✕</button></td>`;
    tbl.appendChild(tr);
  });
}

/** Library tool -> job tool (the shapes the strategies use). */
function toJobTool(t, number) {
  const s = t.shape;
  const base = { number, description: t.name };
  if (t.flute_length) base.flute_length = t.flute_length;
  if (t.shank_diameter) base.shank_diameter = t.shank_diameter;
  if (t.stickout) base.stickout = t.stickout;
  if (s.kind === "flat" || s.kind === "bull") return { ...base, kind: "flat", diameter: s.diameter };
  if (s.kind === "ball") return { ...base, kind: "ball", diameter: s.diameter };
  if (s.kind === "vbit") return { ...base, kind: "vbit", angle: s.angle, diameter: s.diameter };
  if (s.kind === "tapered_ball")
    return { ...base, kind: "tapered_ball", tip_diameter: s.tip_diameter, taper_angle: s.taper_angle, diameter: s.diameter };
  throw new Error(`${t.name}: ${s.kind} tools are not used by the strategies yet`);
}

function useTool(i, target) {
  showError("");
  const t = currentLibrary().tools[i];
  try {
    const job = readJob();
    job.tools = job.tools || [];
    const number = target === "new" ? Math.max(0, ...job.tools.map((x) => x.number)) + 1 : +target;
    const jt = toJobTool(t, number);
    const k = job.tools.findIndex((x) => x.number === number);
    if (k >= 0) job.tools[k] = jt;
    else job.tools.push(jt);
    writeJob(job);
    renderQuick();
    renderTools();
    $("importInfo").textContent = `T${number} is now ${t.name}`;
  } catch (e) {
    showError(e.message);
  }
}

function openForm(i) {
  editing = i;
  const t = i >= 0 ? currentLibrary().tools[i] : { name: "", shape: { kind: "flat", diameter: 3.175 } };
  $("tfName").value = t.name;
  $("tfKind").value = ["flat", "ball", "bull", "vbit", "tapered_ball", "drill"].includes(t.shape.kind) ? t.shape.kind : "ball";
  $("tfTip").value = t.shape.tip_diameter ?? "";
  $("tfTaper").value = t.shape.taper_angle ?? "";
  $("tfDiameter").value = t.shape.diameter;
  $("tfAngle").value = t.shape.angle ?? "";
  $("tfCorner").value = t.shape.corner_radius ?? "";
  $("tfFlute").value = t.flute_length ?? "";
  $("tfNumber").value = t.number ?? "";
  $("toolForm").classList.remove("hidden");
  $("tfName").focus();
}

function saveForm(ev) {
  ev.preventDefault();
  showError("");
  const lib = currentLibrary();
  const num = (id) => ($(id).value === "" ? undefined : +$(id).value);
  const kind = $("tfKind").value;
  const shape = { kind, diameter: num("tfDiameter") };
  if (kind === "vbit") shape.angle = num("tfAngle") ?? 90;
  if (kind === "bull") shape.corner_radius = num("tfCorner") ?? 0;
  if (kind === "drill") shape.point_angle = 118;
  if (kind === "tapered_ball") {
    shape.tip_diameter = num("tfTip") ?? shape.diameter;
    shape.taper_angle = num("tfTaper") ?? 0;
    if (!(shape.tip_diameter > 0 && shape.tip_diameter <= shape.diameter && shape.taper_angle >= 0 && shape.taper_angle < 90))
      return showError("Tapered ball: 0 < tip Ø <= diameter, 0 <= taper < 90°");
  }
  if (!(shape.diameter > 0)) return showError("Diameter must be > 0");
  if (kind === "vbit" && !(shape.angle > 0 && shape.angle < 180)) return showError("V-bit angle must be between 0 and 180");
  const old = editing >= 0 ? lib.tools[editing] : {};
  const name = $("tfName").value.trim() || "tool";
  const tool = { ...old, id: old.id || `${name.toLowerCase().replace(/[^a-z0-9.]+/g, "-")}-${Date.now() % 100000}`, name, shape };
  const flute = num("tfFlute");
  const number = num("tfNumber");
  if (flute) tool.flute_length = flute;
  else delete tool.flute_length;
  if (number) tool.number = number;
  else delete tool.number;
  if (editing >= 0) lib.tools[editing] = tool;
  else lib.tools.push(tool);
  saveLibrary(lib);
  $("toolForm").classList.add("hidden");
  renderLibraries(lib.id);
}

async function importBytes(bytes, name) {
  showError("");
  const { result } = await engine("importProfile", bytes, name);
  const n = result.notes || [];
  const notes = n.length ? ` (${n.length} note${n.length > 1 ? "s" : ""}: ${n.slice(0, 3).join("; ")})` : "";
  if (result.kind === "tools") {
    const libs = libraries().filter((l) => l.id !== result.profile.id);
    libs.push(result.profile);
    store("marcam.libraries", libs);
    renderLibraries(result.profile.id);
    store("marcam.libSel", result.profile.id);
    $("importInfo").textContent = `Imported tool library "${result.profile.name}": ${result.profile.tools.length} tools${notes}`;
  } else {
    const ms = userMachines().filter((m) => m.id !== result.profile.id);
    ms.push(result.profile);
    store("marcam.machines", ms);
    renderMachines(result.profile.id);
    store("marcam.machineSel", $("machine").value);
    $("importInfo").textContent = `Imported machine "${result.profile.name}"${notes}`;
  }
}

function downloadJson(obj, file) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(obj, null, 2) + "\n"], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = file;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

$("library").addEventListener("change", () => {
  store("marcam.libSel", $("library").value);
  renderTools();
});
$("toolTable").addEventListener("click", (ev) => {
  const b = ev.target.closest("button");
  if (!b) return;
  if (b.dataset.edit) openForm(+b.dataset.edit);
  if (b.dataset.del) {
    const lib = currentLibrary();
    lib.tools.splice(+b.dataset.del, 1);
    saveLibrary(lib);
    renderLibraries(lib.id);
  }
});
$("toolTable").addEventListener("change", (ev) => {
  const s = ev.target.closest("select[data-use]");
  if (s && s.value) useTool(+s.dataset.use, s.value);
});
$("addTool").addEventListener("click", () => openForm(-1));
$("toolForm").addEventListener("submit", saveForm);
$("tfCancel").addEventListener("click", () => $("toolForm").classList.add("hidden"));
$("exportLibrary").addEventListener("click", () => {
  const l = currentLibrary();
  if (l) downloadJson(l, `${l.id}.tools.json`);
});
$("importFile").addEventListener("change", async () => {
  const f = $("importFile").files[0];
  if (!f) return;
  try {
    await importBytes(new Uint8Array(await f.arrayBuffer()), f.name);
  } catch (e) {
    showError(e.message);
  }
});
$("importUrlBtn").addEventListener("click", async () => {
  const url = $("importUrl").value.trim();
  if (!url) return;
  try {
    const r = await fetch(url);
    if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
    await importBytes(new Uint8Array(await r.arrayBuffer()), decodeURIComponent(url.split("/").pop().split("?")[0]));
  } catch (e) {
    showError(`Import failed: ${e.message}`);
  }
});
$("job").addEventListener("change", renderTools);

// ---------------------------------------------------------------- resizable panes
window.__marcam.splitters = {
  left: splitter({ bar: $("splitLeft"), target: $("left"), axis: "x", min: 300, max: () => window.innerWidth - 520, id: "left" }),
  code: splitter({ bar: $("splitCode"), target: $("codeWrap"), axis: "x", after: true, min: 200, max: () => $("views").clientWidth - 288, id: "code" }),
  side: splitter({ bar: $("split3d"), target: $("side"), axis: "y", after: true, min: 80, max: () => Math.max(80, $("viewWrap").clientHeight - 260), id: "side" }),
  tools: splitter({ bar: $("splitTools"), target: $("toolTableWrap"), axis: "y", min: 80, max: 900, id: "tools" }),
};
// the 2D side view redraws at its new size (the 3D view has its own ResizeObserver)
new ResizeObserver(() => drawSide()).observe($("side"));

// ---------------------------------------------------------------- project tree (0.2)
window.__marcam.tree = setupTree({
  engine, $, state, readJob, writeJob, showError, getThree,
  machineArg: () => (typeof machineArg() === "string" && machineArg() !== "custom" ? machineArg() : null),
  onChange: () => { renderQuick(); renderTools(); },
});
$("job").addEventListener("change", () => window.__marcam.tree.refresh());

// ---------------------------------------------------------------- auto-regenerate setting
try {
  $("autoGen").checked = localStorage.getItem("marcam.autoGen") === "1";
} catch {}
$("autoGen").addEventListener("change", () => {
  try {
    localStorage.setItem("marcam.autoGen", $("autoGen").checked ? "1" : "0");
  } catch {}
  if ($("autoGen").checked && state.stale) scheduleAuto();
});
window.__marcam.autoDelay = AUTO_DELAY;
window.__marcam.setHidden = setHidden;
window.__marcam.hiddenStoreKey = hiddenStoreKey;
// what Generate plans with (the tests plan the saved project again with it)
window.__marcam.planArgs = () => ({ machine: machineArg(), libraries: libraries() });

// ---------------------------------------------------------------- project workflow
window.__marcam.project = setupProject({
  $, engine, state, readJob, writeJob, setMesh, showError, setStatus, generate, refreshAll, clearResults, markStale, resetProject,
  machineArg, currentMachine, selectMachine: (id) => renderMachines(id),
});

// ---------------------------------------------------------------- machining simulation
window.__marcam.machining = setupMachining({ engine, getThree, $, state, readJob, setStatus, showError, libraries, simOpts, confirmSimRes, simGpu, showBackend });
window.__marcam.updateSimResInfo = updateSimResInfo;

// ---------------------------------------------------------------- view hooks (used by the e2e test)
/** Camera presets: "iso", "front" (looking down the axis from +X), "back" (from -X). `show`
 *  picks layers: { path, part, stock, sim }. Renders immediately. */
window.__marcam.setView = async (name, show = {}) => {
  const v = await getThree();
  if (!v || v.failed || !v.bounds) return false;
  const { xmin, xmax, R } = v.bounds;
  const cx = (xmin + xmax) / 2, span = xmax - xmin;
  if (name === "front") v.camera.position.set(xmax + 6 * R, 0, 0);
  else if (name === "back") v.camera.position.set(xmin - 6 * R, 0, 0);
  else v.camera.position.set(cx - span * 0.2, -span * 1.3, R * 2.2);
  v.controls.target.set(cx, 0, 0);
  v.controls.update();
  for (const [k, on] of Object.entries(show)) if (v.groups[k]) v.groups[k].visible = on;
  if (show.path === false) v.groups.tool.visible = false;
  v.axes.visible = show.axes ?? true;
  v.renderer.render(v.scene, v.camera);
  return true;
};

/** "r,g,b" at a fraction (fx, fy) of the 3D canvas, after a fresh render. */
window.__marcam.pixelAt = (fx, fy) => {
  const v = state.three;
  v.renderer.render(v.scene, v.camera);
  const gl = v.renderer.getContext();
  const w = gl.drawingBufferWidth, h = gl.drawingBufferHeight;
  const buf = new Uint8Array(4);
  gl.readPixels(Math.floor(fx * w), Math.floor((1 - fy) * h), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, buf);
  return `${buf[0]},${buf[1]},${buf[2]}`;
};

// ---------------------------------------------------------------- developer switches
// ?gpu=off: simulate on the CPU (the GPU is used automatically otherwise); ?json or
// Ctrl+Shift+J: show the project as text
{
  const q = new URLSearchParams(location.search);
  if (q.get("gpu") === "off") $("simGpu").checked = false;
  if (q.has("json")) $("jsonPanel").classList.remove("hidden");
  document.addEventListener("keydown", (ev) => {
    if (ev.ctrlKey && ev.shiftKey && (ev.key === "J" || ev.key === "j")) {
      ev.preventDefault();
      $("jsonPanel").classList.toggle("hidden");
    }
  });
}

// ---------------------------------------------------------------- the side view (optional)
// Off by default (the view cube does most of what it did), remembered in this browser. View
// only: nothing is planned.
{
  const apply = (on) => {
    $("side").classList.toggle("hidden", !on);
    $("split3d").classList.toggle("hidden", !on);
    if (on) drawSide();
  };
  let on = false;
  try {
    on = localStorage.getItem("marcam.sideView") === "1";
  } catch {}
  $("showSide").checked = on;
  apply(on);
  $("showSide").addEventListener("change", () => {
    try {
      localStorage.setItem("marcam.sideView", $("showSide").checked ? "1" : "0");
    } catch {}
    apply($("showSide").checked);
  });
}

// ---------------------------------------------------------------- the menus
// File: what the toolbar's buttons did; a menu closes once something in it is chosen.
$("menuImport").addEventListener("click", () => $("importStl").click());
$("menuReplace").addEventListener("click", () => $("replaceModelFile").click());
$("menuDownload").addEventListener("click", () => $("download").click());
for (const d of document.querySelectorAll("details.menu")) {
  d.addEventListener("click", (ev) => {
    if (ev.target.closest("button")) d.open = false;
  });
}
document.addEventListener("click", (ev) => {
  for (const d of document.querySelectorAll("details.menu[open]")) if (!d.contains(ev.target)) d.open = false;
});
