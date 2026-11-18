// Project workflow for the web app: New (a setup dialog: name, machine, setup type, part,
// stock), Open (also drag-and-drop), Save / Save as (one .marcam.json file with the part mesh
// embedded), Import STL (binary or ASCII, mm or inch, through the same dialog), editing a
// setup (its type, machine, stock, origin, holding: converting it when the type changes),
// adding a setup, unsaved-changes tracking and warnings, and an undo for setup changes.

import { setupDialog } from "./setupdialog.js";

/** The file format Save writes: the project, and the mesh bytes it names, in one file. */
export const FILE_FORMAT = "marcam-project";
/** Project files saved before the rename (4cam -> marcam, `.4cam.json`) open too. */
export const LEGACY_FILE_FORMATS = ["4cam-project"];

/** The setup types, as the dialog and the tree name them. */
export const KINDS = {
  rotary: "Rotary 4th axis",
  indexed: "Indexed 3+1",
  vise: "3-axis vise",
};

const enc = new TextEncoder();
const dec = new TextDecoder();

// ------------------------------------------------------------------ STL

/** Triangles of an STL file (binary or ASCII), as 9 floats each. Throws on anything else. */
export function parseStl(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (b.length >= 84) {
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    const n = dv.getUint32(80, true);
    if (n > 0 && 84 + 50 * n === b.length) {
      const tris = new Float32Array(9 * n);
      for (let t = 0; t < n; t++) for (let k = 0; k < 9; k++) tris[9 * t + k] = dv.getFloat32(84 + 50 * t + 12 + 4 * k, true);
      return { tris, ascii: false };
    }
  }
  const head = dec.decode(b.subarray(0, Math.min(b.length, 512))).trimStart();
  if (head.startsWith("solid")) {
    const text = dec.decode(b);
    const vals = [];
    const re = /vertex\s+(\S+)\s+(\S+)\s+(\S+)/g;
    for (let m; (m = re.exec(text)); ) vals.push(+m[1], +m[2], +m[3]);
    if (vals.length && vals.length % 9 === 0 && vals.every(Number.isFinite)) return { tris: new Float32Array(vals), ascii: true };
    throw new Error("this ASCII STL has no complete triangles (facets of three vertices)");
  }
  throw new Error("not an STL file: neither a binary STL (its size does not match its triangle count) nor an ASCII one (it does not start with \"solid\")");
}

/** Binary STL bytes of triangles (9 floats each), with normals. */
export function encodeStl(tris, header = "marcam") {
  const n = tris.length / 9;
  const out = new Uint8Array(84 + 50 * n);
  out.set(enc.encode(header).subarray(0, 80));
  const dv = new DataView(out.buffer);
  dv.setUint32(80, n, true);
  for (let t = 0; t < n; t++) {
    const p = tris.subarray(9 * t, 9 * t + 9);
    const u = [p[3] - p[0], p[4] - p[1], p[5] - p[2]], v = [p[6] - p[0], p[7] - p[1], p[8] - p[2]];
    const nn = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    const len = Math.hypot(...nn) || 1;
    const o = 84 + 50 * t;
    for (let k = 0; k < 3; k++) dv.setFloat32(o + 4 * k, nn[k] / len, true);
    for (let k = 0; k < 9; k++) dv.setFloat32(o + 12 + 4 * k, p[k], true);
  }
  return out;
}

/** The engine's axis mapping (MeshSource.axis): the mesh axis that becomes X. */
export function mapAxis(p, axis) {
  if (axis === "y") return [p[1], p[2], p[0]];
  if (axis === "z") return [p[2], p[0], p[1]];
  return [p[0], p[1], p[2]];
}

const ceilHalf = (x) => Math.ceil(x * 2) / 2;
const r3 = (x) => Math.round(x * 1000) / 1000;

/** The engine's axis mapping that makes `up` (x, y or z) the vise's Z. */
export const UP_AXIS = { z: "x", y: "z", x: "y" };

/**
 * Where a part goes and the stock round it, for a setup `kind` (or the older `holding`).
 * - Bar ("rotary", "indexed"): `axis` (x, y or z) is the mesh axis turned onto X; the part is
 *   centred on it with its front (largest X) at X0 plus `origin`; bar stock of the largest
 *   radius + `margin` (default 0.5 mm, rounded up to 0.5 mm), the length + 15 mm to hold.
 * - Vise: `up` is the mesh axis that points up; the part sits `margin` (default 2 mm) in from
 *   the block's corner, its top `origin` (default 0.5 mm) below the block's top (Z0), the
 *   block 2 mm below it.
 * - `explicit`: { dia, len } for a bar, { bx, by, bz } for a block (the part centred on it).
 * Returns { axis, translate, stock, size } (size: the part's X, Y, Z extent after placing).
 */
export function placePart(tris, opts = {}) {
  const kind = opts.kind ?? (opts.holding === "vise" ? "vise" : "rotary");
  const vise = kind === "vise";
  const ax = vise ? UP_AXIS[opts.up ?? opts.axis ?? "z"] : opts.axis ?? "x";
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < tris.length; i += 3) {
    const q = mapAxis([tris[i], tris[i + 1], tris[i + 2]], ax);
    for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], q[k]);
      hi[k] = Math.max(hi[k], q[k]);
    }
  }
  const size = [0, 1, 2].map((k) => hi[k] - lo[k]);
  const ex = opts.explicit;
  if (vise) {
    const m = opts.margin ?? 2, top = opts.origin ?? 0.5;
    if (ex) {
      const translate = [(ex.bx - size[0]) / 2 - lo[0], (ex.by - size[1]) / 2 - lo[1], -top - hi[2]];
      return { axis: ax, translate, stock: { shape: "box", min: [0, 0, -ex.bz], max: [ex.bx, ex.by, 0] }, size };
    }
    const translate = [m - lo[0], m - lo[1], -top - hi[2]];
    const stock = { shape: "box", min: [0, 0, r3(-(size[2] + top + 2))], max: [r3(size[0] + 2 * m), r3(size[1] + 2 * m), 0] };
    return { axis: ax, translate, stock, size };
  }
  const cy = (lo[1] + hi[1]) / 2, cz = (lo[2] + hi[2]) / 2;
  let rmax = 0;
  for (let i = 0; i < tris.length; i += 3) {
    const q = mapAxis([tris[i], tris[i + 1], tris[i + 2]], ax);
    rmax = Math.max(rmax, Math.hypot(q[1] - cy, q[2] - cz));
  }
  const shift = opts.origin ?? 0;
  const translate = [-hi[0] + shift, -cy, -cz];
  const stock = ex
    ? { shape: "cylinder", radius: ex.dia / 2, length: ex.len }
    : { shape: "cylinder", radius: ceilHalf(rmax + (opts.margin ?? 0.5) - 1e-4), length: Math.ceil(size[0] + 15 + Math.max(0, shift)) };
  return { axis: ax, translate, stock, size };
}

/** The setup's kind: its own, else from its stock (a block: vise; a bar: rotary). */
export function kindOf(setup) {
  return setup?.kind ?? (setup?.stock?.shape === "box" ? "vise" : "rotary");
}

/**
 * Whether an operation type (from opTypes) fits a setup kind, as the engine's
 * ops::fits_setup: rotary takes what turns the part (and engrave/V-carve, wrapped), indexed the
 * 3-axis types (at fixed A angles), vise everything that needs no rotary axis. A bar without a
 * kind takes everything (older projects mix rotary and indexed operations).
 */
export function fitsKind(type, kind, explicit = true) {
  if (!explicit && kind === "rotary") return true;
  if (kind === "rotary") return !type.machine_needs.includes("xyz");
  if (kind === "indexed") return type.machine_needs.includes("xyz");
  if (kind === "vise") return !type.machine_needs.includes("rotary_a");
  return true;
}

/** The operations a new setup of each kind starts with: [type, tool, A, output]. */
export const STARTERS = {
  rotary: [["rotary_rough", 1, null, "rough"], ["rotary_finish", 2, null, "finish"]],
  indexed: [["rough3d", 1, 0, "rough"], ["rough3d", 1, 180, "rough"], ["raster3d", 2, 0, "finish"], ["raster3d", 2, 180, "finish"]],
  vise: [["rough3d", 1, null, "rough"], ["raster3d", 2, null, "finish"]],
};

// ------------------------------------------------------------------ files

function b64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
function unb64(s) {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** The saved file: the project and the mesh it names, embedded. */
export function projectFile(project, mesh, meshName, view) {
  const file = { format: FILE_FORMAT, version: 1, project };
  if (mesh) file.meshes = { [meshName]: b64(mesh) };
  // how it was shown (view only: which toolpaths were hidden)
  if (view?.hidden?.length) file.view = { hidden: view.hidden };
  return file;
}

/** Read a saved file, a 0.2 project or a 0.1 job: { project, mesh, meshName }. */
export function readProjectFile(text) {
  let obj;
  try {
    obj = JSON.parse(text);
  } catch (e) {
    throw new Error(`not a marcam project: the file is not JSON (${e.message})`);
  }
  if (obj && (obj.format === FILE_FORMAT || LEGACY_FILE_FORMATS.includes(obj.format))) {
    const [name, data] = Object.entries(obj.meshes || {})[0] || [];
    return { project: obj.project, mesh: data ? unb64(data) : null, meshName: name || "", view: obj.view || null };
  }
  if (obj && (obj.schema_version || obj.setups || obj.programs)) return { project: obj, mesh: null, meshName: "" };
  throw new Error("not a marcam project: no project, setups or programs in the file");
}

/** The mesh file a project names (its part's mesh model, or a 0.1 job's part.mesh). */
export function meshFileOf(project) {
  const m = project?.models?.find((x) => x.kind === "mesh" && (project.setups || []).some((s) => (s.part || []).includes(x.id)));
  return m?.file ?? project?.part?.mesh?.file ?? null;
}

const slug = (s) => (String(s).toLowerCase().replace(/\.[a-z0-9]+$/, "").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "part");

// ------------------------------------------------------------------ the workflow

export function setupProject(ctx) {
  const { $, engine, state, readJob, writeJob, setMesh, showError, setStatus, generate, refreshAll, clearResults, resetProject, machineArg, currentMachine } = ctx;
  const p = { name: "untitled", dirty: false, fileName: null, pending: null, undo: null };
  state.project = p;
  const dialog = setupDialog({ $, state, parseStl, placePart, kindOf, KINDS });

  function show() {
    $("projectName").textContent = p.name;
    $("projectDirty").classList.toggle("hidden", !p.dirty);
    $("projectDirty").title = p.dirty ? "unsaved changes" : "";
    document.title = `${p.dirty ? "* " : ""}${p.name} - marcam`;
  }
  /** Something in the project changed: unsaved, and (unless `stale: false`, a colour or a
   *  name) any results are out of date. Nothing is planned here (see auto-regenerate). */
  function changed({ stale = true } = {}) {
    p.dirty = true;
    show();
    if (stale) ctx.markStale();
  }
  function clean(name) {
    if (name) p.name = name;
    p.dirty = false;
    show();
  }
  /** Ask before throwing away unsaved changes; true to go on. */
  function mayDiscard(what) {
    return !p.dirty || window.confirm(`"${p.name}" has unsaved changes. ${what} anyway and lose them?`);
  }
  const engineMachine = () => (typeof machineArg() === "string" && machineArg() !== "custom" ? machineArg() : null);

  // ------------------------------------------------------------ undo (setup changes)
  /** Offer to undo a setup change: `job` is the project text from before it. */
  function keepUndo(label, job) {
    p.undo = { label, job, mesh: state.mesh, meshName: state.meshName };
    $("undoBar").innerHTML = `${label} <button type="button" id="undoSetup">Undo</button>`;
    $("undoBar").classList.remove("hidden");
  }
  function dropUndo() {
    p.undo = null;
    $("undoBar").classList.add("hidden");
  }
  async function undo() {
    const u = p.undo;
    if (!u) return;
    setStatus(`Undoing: ${u.label}`, "busy");
    dropUndo();
    writeJob(JSON.parse(u.job));
    setMesh(u.mesh, u.meshName);
    await refreshAll();
    setStatus(`Undone: ${u.label} Generate to see it again.`, "ready");
  }

  // ------------------------------------------------------------ new, open, save
  async function newProject() {
    if (!mayDiscard("Start a new project")) return false;
    const cfg = await dialog.open("new", { name: "untitled", machine: currentMachine()?.id, kind: p.pending?.kind ?? "rotary" });
    if (!cfg) return false;
    await resetProject("New project: import an STL (button or drop it here) or open a project to start.");
    setStatus("New project", "ready");
    if (cfg.machine) ctx.selectMachine(cfg.machine);
    const doc = { schema_version: "0.2", name: cfg.name || "untitled", models: [], tools: [], setups: [] };
    writeJob(doc, { load: true });
    p.fileName = null;
    p.pending = cfg;
    clean(doc.name);
    await refreshAll();
    if (cfg.file) await importParsed(cfg);
    return true;
  }

  async function openText(text, fileName) {
    const { project, mesh, meshName, view } = readProjectFile(text);
    const name = project.name || slug(fileName || "project");
    await resetProject("");
    writeJob(project, { load: true });
    const wanted = meshFileOf(project);
    setMesh(mesh, mesh ? `${meshName} (in the project)` : "");
    p.fileName = fileName || null;
    p.pending = null;
    clean(name);
    const sm = project.machine ?? project.setups?.[0]?.machine;
    if (sm) ctx.selectMachine(sm);
    if (wanted && !mesh) {
      clearResults("");
      await refreshAll();
      setStatus("Opened", "ready");
      showError(`This project uses the mesh "${wanted}", which is not in the file: import it (Import STL) to plan.`);
      return;
    }
    // what was hidden: the file's view, else this browser's memory of the project
    let hidden = view?.hidden;
    if (!hidden) {
      try {
        hidden = JSON.parse(localStorage.getItem(window.__marcam.hiddenStoreKey()) || "null");
      } catch {
        hidden = null;
      }
    }
    window.__marcam.setHidden(hidden || [], { persist: !!view });
    // opened, not planned: Generate when ready (it can take a while)
    clearResults(`Opened ${name}. Generate to plan its toolpaths.`, ["generate"]);
    await refreshAll();
    setStatus(`Opened ${name}`, "ready");
  }

  async function openFile(file) {
    if (!mayDiscard("Open another project")) return;
    showError("");
    setStatus(`Opening ${file.name}...`, "busy");
    try {
      await openText(await file.text(), file.name);
    } catch (e) {
      showError(`${file.name}: ${e.message}`);
      setStatus("Open failed", "error");
    }
  }

  function save(asNew) {
    showError("");
    let project;
    try {
      project = readJob();
    } catch (e) {
      return showError(e.message);
    }
    let name = p.name;
    if (asNew || !p.fileName) {
      const asked = window.prompt("Save the project as:", p.fileName || `${slug(p.name)}.marcam.json`);
      if (!asked) return;
      p.fileName = /\.json$/i.test(asked) ? asked : `${asked}.marcam.json`;
      name = p.fileName.replace(/(\.marcam)?\.json$/i, "").replace(/\.project$/i, "");
      if (project.schema_version === "0.2") project.name = name;
      writeJob(project, { load: true });
    }
    const file = projectFile(project, state.mesh, meshFileOf(project) || state.meshName.split(" ")[0] || "part.stl", { hidden: [...state.hidden] });
    const blob = new Blob([JSON.stringify(file, null, 1)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = p.fileName;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
    clean(name);
    setStatus(`Saved ${p.fileName}`, "ready");
  }

  // ------------------------------------------------------------ setups
  /** The tools starter operations use: T1 flat and T2 ball, added when missing. */
  function ensureTools(doc, stock) {
    const reach = stock.shape === "box" ? stock.max[2] - stock.min[2] : stock.radius;
    const flute = Math.max(12, Math.ceil(reach + 1));
    doc.tools = doc.tools || [];
    // the starter tools (marcam's own, by their description) follow the stock's depth, so the
    // holder clears it; tools the user defined are left alone (the holder check reports them)
    for (const t of doc.tools) {
      const m = /^3\.175 mm (flat|ball) end mill, \d+ mm flute$/.exec(t.description ?? "");
      if (m && t.kind === m[1] && t.flute_length !== flute) {
        t.flute_length = flute;
        t.description = `3.175 mm ${m[1]} end mill, ${flute} mm flute`;
      }
    }
    if (!doc.tools.some((t) => t.number === 1))
      doc.tools.push({ number: 1, description: `3.175 mm flat end mill, ${flute} mm flute`, kind: "flat", diameter: 3.175, flute_length: flute });
    if (!doc.tools.some((t) => t.number === 2))
      doc.tools.push({ number: 2, description: `3.175 mm ball end mill, ${flute} mm flute`, kind: "ball", diameter: 3.175, flute_length: flute });
  }

  const nextOpId = (doc) => {
    const used = new Set(doc.setups.flatMap((s) => s.operations.map((o) => o.id)));
    for (let k = 1; ; k++) if (!used.has(`op${k}`)) return `op${k}`;
  };

  /** A setup's starter operations (roughing, then finishing), every field from the engine. */
  async function addStarters(doc, s) {
    const base = `${slug(doc.name)}${doc.setups.length > 1 ? `_${slug(s.id)}` : ""}`;
    const outs = {};
    const added = [];
    for (const [type, tool, a] of STARTERS[kindOf(s)].map((x) => x.slice(0, 3))) {
      const what = STARTERS[kindOf(s)].find((x) => x[0] === type)[3];
      const { result } = await engine("opDefaults", doc, { type, setup: s.id, tool, machine: engineMachine() }, state.mesh);
      const op = { id: nextOpId(doc), ...result };
      if (a !== null) {
        op.index_a = a;
        op.name = `${type === "rough3d" ? "Rough" : "Finish"} at A${a}`;
      }
      s.operations.push(op);
      added.push(`${op.id} (${type}${a !== null ? ` at A${a}` : ""})`);
      if (!outs[what]) {
        outs[what] = { name: `${base}_${Object.keys(outs).length + 1}_${what}`, title: `${doc.name}: ${what}`, operations: [] };
        s.outputs.push(outs[what]);
      }
      outs[what].operations.push(op.id);
    }
    return added;
  }

  /** The mesh model a setup uses, its own: one shared with another setup is copied first. */
  function ownModel(doc, s) {
    const id = s.part.find((m) => doc.models.find((x) => x.id === m)?.kind === "mesh");
    if (!id) return null;
    const shared = doc.setups.some((o) => o !== s && o.part.includes(id));
    if (!shared) return doc.models.find((x) => x.id === id);
    const copy = { ...doc.models.find((x) => x.id === id), id: `${s.id}_part` };
    doc.models = doc.models.filter((x) => x.id !== copy.id).concat(copy);
    s.part = s.part.map((m) => (m === id ? copy.id : m));
    return copy;
  }

  /**
   * Apply a setup configuration from the dialog to setup `s`: kind, machine, holding, the
   * part placed again and the stock sized (when there is a mesh), the operations that do not
   * fit the kind removed and the rest given the engine's defaults for it again, starters added
   * when none are left. Returns { removed, added, recomputed }.
   */
  async function applySetup(doc, s, cfg, tris) {
    const kind = cfg.kind;
    s.kind = kind;
    if (cfg.machine) s.machine = cfg.machine;
    if (cfg.setupName) s.name = cfg.setupName;
    s.holding = { kind: kind === "vise" ? "vise" : "rotary_chuck", tailstock: kind !== "vise" && !!cfg.tailstock };
    const model = ownModel(doc, s);
    if (model && tris) {
      const place = placePart(tris, { kind, axis: cfg.axis, up: cfg.axis, margin: cfg.margin, explicit: cfg.explicit, origin: cfg.origin });
      model.axis = place.axis;
      model.translate = place.translate.map(r3);
      s.stock = place.stock;
      ensureTools(doc, place.stock);
    } else if (cfg.explicit) {
      s.stock = kind === "vise" ? { shape: "box", min: [0, 0, -cfg.explicit.bz], max: [cfg.explicit.bx, cfg.explicit.by, 0] } : { shape: "cylinder", radius: cfg.explicit.dia / 2, length: cfg.explicit.len };
    }
    // operations: keep what fits, with its defaults for the new part and kind
    const types = state.tree?.types?.length ? state.tree.types : (await engine("opTypes")).result;
    const removed = [], recomputed = [];
    const keep = [];
    const hasProfile = s.part.some((id) => doc.models.find((x) => x.id === id)?.kind === "profile");
    for (const op of s.operations) {
      const ty = types.find((t) => t.kind === op.type);
      if (!ty || !fitsKind(ty, kind)) {
        removed.push(`${op.id} (${op.type})`);
        continue;
      }
      // turning works from a revolved profile only
      if (!hasProfile && ty.geometry.length === 1 && ty.geometry[0] === "profile") {
        removed.push(`${op.id} (${op.type}: needs a revolved profile, and the part is a mesh)`);
        continue;
      }
      keep.push(op);
    }
    s.operations = keep;
    for (const [i, op] of [...s.operations.entries()]) {
      try {
        const { result } = await engine("opDefaults", doc, { type: op.type, setup: s.id, tool: op.tool, machine: engineMachine() }, state.mesh);
        const a = kind === "vise" ? 0 : op.index_a;
        s.operations[i] = { ...result, id: op.id, enabled: op.enabled, ...(op.name ? { name: op.name } : {}), ...(op.color ? { color: op.color } : {}), ...(a !== undefined && result.index_a !== undefined ? { index_a: a } : {}) };
        recomputed.push(op.id);
      } catch (e) {
        removed.push(`${op.id} (${op.type}: ${e.message})`);
        s.operations[i] = null;
      }
    }
    s.operations = s.operations.filter(Boolean);
    const ids = new Set(s.operations.map((o) => o.id));
    for (const o of s.outputs) o.operations = o.operations.filter((id) => ids.has(id));
    s.outputs = s.outputs.filter((o) => o.operations.length);
    const added = s.operations.length ? [] : await addStarters(doc, s);
    return { removed, added, recomputed };
  }

  /** The parsed STL of the dialog's file, in mm, or (no file) the project's mesh. */
  function trisOf(cfg) {
    if (cfg.tris) return cfg.units === "inch" ? cfg.tris.map((v) => v * 25.4) : cfg.tris;
    return state.mesh ? parseStl(state.mesh).tris : null;
  }

  /** A new part from the dialog (its file already parsed): into a project without a setup,
   *  one is made; otherwise the dialog's setup is converted round it. */
  async function importParsed(cfg) {
    setStatus(`Importing ${cfg.file.name}...`, "busy");
    let tris = trisOf(cfg);
    let doc;
    try {
      doc = readJob();
    } catch {
      doc = { schema_version: "0.2", name: "untitled", models: [], tools: [], setups: [] };
    }
    if (doc.schema_version !== "0.2") doc = (await engine("checkProject", doc)).result.project;
    const fresh = !doc.setups?.length || !doc.setups.some((s) => s.operations.length);
    if (state.mesh && meshFileOf(doc) && !fresh && !window.confirm(`Replace the part with ${cfg.file.name}? (OK: replace it; Cancel: add ${cfg.file.name} to the part)`)) {
      const old = parseStl(state.mesh).tris;
      const both = new Float32Array(old.length + tris.length);
      both.set(old);
      both.set(tris, old.length);
      tris = both;
    }
    const meshFile = `${slug(cfg.file.name)}.stl`;
    const bytes = encodeStl(tris, `marcam: ${cfg.file.name}`);
    setMesh(bytes, `${cfg.file.name} (${cfg.ascii ? "ASCII" : "binary"} STL${cfg.units === "inch" ? ", inch" : ""}, ${tris.length / 9} triangles)`);
    let report;
    if (fresh) {
      const name = doc.name && doc.name !== "untitled" ? doc.name : cfg.name && cfg.name !== "untitled" ? cfg.name : slug(cfg.file.name);
      doc = {
        schema_version: "0.2",
        name,
        models: [{ id: "part", name: cfg.file.name, kind: "mesh", file: meshFile, axis: "x", translate: [0, 0, 0] }],
        tools: [],
        setups: [{ id: "setup1", name: KINDS[cfg.kind], stock: { shape: "cylinder", radius: 1, length: 1 }, clearance: 3, part: ["part"], spindle_rpm: 12000, operations: [], outputs: [] }],
      };
      p.name = name;
      report = await applySetup(doc, doc.setups[0], cfg, tris);
    } else {
      // the new part for every mesh model; the dialog's setup converted, the others re-placed
      for (const m of doc.models) if (m.kind === "mesh") Object.assign(m, { name: cfg.file.name, file: meshFile });
      if (!doc.models.some((m) => m.kind === "mesh")) {
        doc.models = doc.models.filter((m) => m.kind !== "profile").concat({ id: "part", name: cfg.file.name, kind: "mesh", file: meshFile, axis: "x", translate: [0, 0, 0] });
        for (const s of doc.setups) s.part = ["part"];
      }
      const active = doc.setups.find((s) => s.id === state.tree?.setup) ?? doc.setups[0];
      report = await applySetup(doc, active, cfg, tris);
      for (const s of doc.setups.filter((x) => x !== active)) {
        await applySetup(doc, s, { kind: kindOf(s), machine: s.machine, axis: kindOf(s) === "vise" ? "z" : "x", tailstock: s.holding?.tailstock }, tris);
      }
    }
    // an import is a request for toolpaths: planned once
    await commitSetupChange(doc, report, null, true);
    return true;
  }

  /** Write a changed project and regenerate; say what the change removed or added. */
  async function commitSetupChange(doc, report, undoOf, planNow = false) {
    writeJob(doc);
    if (undoOf) keepUndo(undoOf.label, undoOf.job);
    const s0 = doc.setups.find((s) => s.id === state.tree?.setup) ?? doc.setups[0];
    const place = state.mesh && s0 ? placeInfo(doc, s0) : "";
    $("meshInfo").textContent = `${state.meshName}${place ? `: ${place}` : ""}`;
    const notes = [];
    if (report?.removed?.length) notes.push(`removed (they do not fit this setup): ${report.removed.join(", ")}`);
    if (report?.added?.length) notes.push(`added: ${report.added.join(", ")}`);
    if (planNow) {
      clearResults("");
      await refreshAll();
      await generate();
    } else {
      // the results (if any) stay, marked out of date, until Generate
      await refreshAll();
      if (!state.result) clearResults("The setup changed. Generate to plan its toolpaths.", ["generate"]);
      setStatus("Setup changed: Generate to plan it again", "ready");
    }
    if (notes.length) showError(`Setup changed: ${notes.join("; ")}`);
  }

  /** "21.70 x 20.00 x 20.00 mm, on the rotary axis ..." for a setup's placed part. */
  function placeInfo(doc, s) {
    const m = doc.models.find((x) => s.part.includes(x.id) && x.kind === "mesh");
    if (!m || !state.mesh) return "";
    const tris = parseStl(state.mesh).tris;
    const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < tris.length; i += 3) {
      const q = mapAxis([tris[i], tris[i + 1], tris[i + 2]], m.axis ?? "x");
      for (let k = 0; k < 3; k++) {
        lo[k] = Math.min(lo[k], q[k]);
        hi[k] = Math.max(hi[k], q[k]);
      }
    }
    const size = [0, 1, 2].map((k) => (hi[k] - lo[k]).toFixed(2)).join(" x ");
    const st = s.stock;
    const kind = kindOf(s);
    const where = kind === "vise" ? `in the vise (${KINDS.vise}), block ${r3(st.max[0] - st.min[0])} x ${r3(st.max[1] - st.min[1])} x ${r3(st.max[2] - st.min[2])} mm` : `on the rotary axis (${KINDS[kind]}, front at X0), bar R${st.radius} x ${st.length} mm`;
    return `${size} mm, ${where}`;
  }

  /** Import STL: the setup dialog (its type pre-selected from the current setup), then in. */
  async function importMesh(file) {
    showError("");
    let parsed;
    try {
      parsed = parseStl(new Uint8Array(await file.arrayBuffer()));
    } catch (e) {
      showError(`${file.name}: ${e.message}`);
      setStatus("Import failed", "error");
      return false;
    }
    let s = null;
    try {
      const doc = readJob();
      s = doc.setups?.find((x) => x.id === state.tree?.setup) ?? doc.setups?.[0] ?? null;
    } catch {}
    const preset = s
      ? { kind: kindOf(s), machine: s.machine ?? currentMachine()?.id, setupName: s.name, tailstock: s.holding?.tailstock }
      : { kind: p.pending?.kind ?? "rotary", machine: p.pending?.machine ?? currentMachine()?.id, margin: p.pending?.margin, explicit: p.pending?.explicit, units: p.pending?.units };
    const cfg = await dialog.open("import", { ...preset, file, parsed });
    if (!cfg) {
      setStatus("Import cancelled", "ready");
      return false;
    }
    if (cfg.machine) ctx.selectMachine(cfg.machine);
    await importParsed(cfg);
    return true;
  }

  /** Edit a setup: type, machine, stock, origin, holding. A new type converts the setup. */
  async function editSetup(id) {
    let doc;
    try {
      doc = readJob();
    } catch (e) {
      return showError(e.message);
    }
    if (doc.schema_version !== "0.2") doc = (await engine("checkProject", doc)).result.project;
    const s = doc.setups.find((x) => x.id === (id ?? state.tree?.setup)) ?? doc.setups[0];
    if (!s) return showError("No setup yet: import an STL first.");
    const m = doc.models.find((x) => s.part.includes(x.id) && x.kind === "mesh");
    const kind = kindOf(s);
    const axis = kind === "vise" ? Object.entries(UP_AXIS).find(([, v]) => v === (m?.axis ?? "x"))?.[0] ?? "z" : m?.axis ?? "x";
    const cfg = await dialog.open("edit", {
      kind, machine: s.machine ?? currentMachine()?.id, setupName: s.name, axis, tailstock: !!s.holding?.tailstock,
      stock: s.stock, hasMesh: !!(m && state.mesh), tris: m && state.mesh ? parseStl(state.mesh).tris : null,
    });
    if (!cfg) return false;
    setStatus(`Changing the setup to ${KINDS[cfg.kind]}...`, "busy");
    const before = $("job").value;
    const probe = structuredClone(doc);
    const ps = probe.setups.find((x) => x.id === s.id);
    const types = state.tree?.types?.length ? state.tree.types : (await engine("opTypes")).result;
    const going = ps.operations.filter((op) => !fitsKind(types.find((t) => t.kind === op.type) ?? { machine_needs: [] }, cfg.kind)).map((op) => `${op.id} (${op.type})`);
    if (going.length && !window.confirm(`Changing "${s.name || s.id}" to ${KINDS[cfg.kind]} removes ${going.join(", ")}: they do not fit that setup. Starter operations are added if none are left. Go on? (You can undo it.)`)) {
      setStatus("Setup unchanged", "ready");
      return false;
    }
    const report = await applySetup(doc, doc.setups.find((x) => x.id === s.id), cfg, m && state.mesh ? parseStl(state.mesh).tris : null);
    await commitSetupChange(doc, report, { label: `Setup "${s.name || s.id}" changed to ${KINDS[cfg.kind]}.`, job: before });
    if (cfg.machine) ctx.selectMachine(cfg.machine);
    return report;
  }

  /** Add a setup: the dialog's type, the same part, its own placement and starters. */
  async function addSetup() {
    let doc;
    try {
      doc = readJob();
    } catch (e) {
      return showError(e.message);
    }
    if (doc.schema_version !== "0.2") doc = (await engine("checkProject", doc)).result.project;
    const meshModel = doc.models.find((x) => x.kind === "mesh");
    if (!meshModel || !state.mesh) return showError("Import an STL first: a new setup needs the part as a mesh.");
    let n = doc.setups.length + 1;
    while (doc.setups.some((s) => s.id === `setup${n}`)) n++;
    const cfg = await dialog.open("add", { kind: "vise", machine: currentMachine()?.id, setupName: `Setup ${n}`, hasMesh: true, tris: parseStl(state.mesh).tris });
    if (!cfg) return false;
    setStatus(`Adding a ${KINDS[cfg.kind]} setup...`, "busy");
    const before = $("job").value;
    const model = { ...meshModel, id: `setup${n}_part` };
    doc.models.push(model);
    const s = { id: `setup${n}`, name: cfg.setupName || KINDS[cfg.kind], stock: { shape: "cylinder", radius: 1, length: 1 }, clearance: 3, part: [model.id], spindle_rpm: 12000, operations: [], outputs: [] };
    doc.setups.push(s);
    const report = await applySetup(doc, s, cfg, parseStl(state.mesh).tris);
    if (state.tree) state.tree.setup = s.id;
    await commitSetupChange(doc, report, { label: `Setup "${s.name}" added.`, job: before });
    if (cfg.machine) ctx.selectMachine(cfg.machine);
    return report;
  }

  // ------------------------------------------------------------ wiring
  $("projectNew").addEventListener("click", () => newProject());
  $("projectOpen").addEventListener("click", () => $("projectOpenFile").click());
  $("projectOpenFile").addEventListener("change", async () => {
    const f = $("projectOpenFile").files[0];
    $("projectOpenFile").value = "";
    if (f) await openFile(f);
  });
  $("projectSave").addEventListener("click", () => save(false));
  $("projectSaveAs").addEventListener("click", () => save(true));
  $("importStl").addEventListener("click", () => $("importStlFile").click());
  $("importStlFile").addEventListener("change", async () => {
    const f = $("importStlFile").files[0];
    $("importStlFile").value = "";
    if (f) await importMesh(f);
  });
  $("undoBar").addEventListener("click", (ev) => {
    if (ev.target.id === "undoSetup") undo();
  });
  // drop a mesh to import it, or a project to open it, anywhere on the page
  document.addEventListener("dragover", (ev) => {
    if (ev.dataTransfer?.types?.includes("Files")) {
      ev.preventDefault();
      document.body.classList.add("dropping");
    }
  });
  document.addEventListener("dragleave", (ev) => {
    if (!ev.relatedTarget) document.body.classList.remove("dropping");
  });
  document.addEventListener("drop", async (ev) => {
    const f = ev.dataTransfer?.files?.[0];
    document.body.classList.remove("dropping");
    if (!f) return;
    ev.preventDefault();
    if (/\.(stl)$/i.test(f.name)) await importMesh(f);
    else if (/\.json$/i.test(f.name)) await openFile(f);
    else showError(`${f.name}: drop an .stl to import it, or a .marcam.json / .project.json to open it`);
  });
  window.addEventListener("beforeunload", (ev) => {
    if (p.dirty) {
      ev.preventDefault();
      ev.returnValue = "";
    }
  });
  show();

  return { changed, clean, mayDiscard, newProject, openText, openFile, save, importMesh, editSetup, addSetup, undo, dropUndo, dialog, state: p };
}
