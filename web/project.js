// Project workflow for the web app: New, Open (also drag-and-drop), Save / Save as (one
// .4cam.json file with the part mesh embedded), Import STL (binary or ASCII, mm or inch, axis
// and holding chosen), unsaved-changes tracking and warnings. Importing a mesh places it
// (rotary: its axis along X, front at X0, centred; vise: Z up, on the block's corner), sizes
// the stock round it, fills a starter roughing + finishing from the engine's defaults (or
// recomputes the existing operations' defaults) and regenerates everything.

/** The file format Save writes: the project, and the mesh bytes it names, in one file. */
export const FILE_FORMAT = "4cam-project";

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
export function encodeStl(tris, header = "4cam") {
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
function mapAxis(p, axis) {
  if (axis === "y") return [p[1], p[2], p[0]];
  if (axis === "z") return [p[2], p[0], p[1]];
  return [p[0], p[1], p[2]];
}

const ceilHalf = (x) => Math.ceil(x * 2) / 2;
const r3 = (x) => Math.round(x * 1000) / 1000;

/**
 * Where a part goes and the stock round it. `holding` "rotary": `axis` (x, y or z) is the
 * mesh axis turned onto X; the part is centred on it with its front (largest X) at X0; bar
 * stock of the largest radius + 0.5 mm (rounded up to 0.5 mm), the length + 15 mm to hold.
 * "vise": `up` (x, y or z) is the mesh axis that points up; the part sits 2 mm in from the
 * block's corner with its top 0.5 mm below the block's top (Z0), the block 2 mm below it.
 * Returns { axis, translate, stock, size } (size: the part's X, Y, Z extent after placing).
 */
export function placePart(tris, { holding = "rotary", axis = "x", up = "z" } = {}) {
  const ax = holding === "vise" ? { z: "x", y: "z", x: "y" }[up] : axis;
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < tris.length; i += 3) {
    const q = mapAxis([tris[i], tris[i + 1], tris[i + 2]], ax);
    for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], q[k]);
      hi[k] = Math.max(hi[k], q[k]);
    }
  }
  const size = [0, 1, 2].map((k) => hi[k] - lo[k]);
  if (holding === "vise") {
    const m = 2;
    const translate = [m - lo[0], m - lo[1], -0.5 - hi[2]];
    const stock = { shape: "box", min: [0, 0, r3(-(size[2] + 0.5 + 2))], max: [r3(size[0] + 2 * m), r3(size[1] + 2 * m), 0] };
    return { axis: ax, translate, stock, size };
  }
  const cy = (lo[1] + hi[1]) / 2, cz = (lo[2] + hi[2]) / 2;
  let rmax = 0;
  for (let i = 0; i < tris.length; i += 3) {
    const q = mapAxis([tris[i], tris[i + 1], tris[i + 2]], ax);
    rmax = Math.max(rmax, Math.hypot(q[1] - cy, q[2] - cz));
  }
  const translate = [-hi[0], -cy, -cz];
  const stock = { shape: "cylinder", radius: ceilHalf(rmax + 0.5 - 1e-4), length: Math.ceil(size[0] + 15) };
  return { axis: ax, translate, stock, size };
}

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
export function projectFile(project, mesh, meshName) {
  const file = { format: FILE_FORMAT, version: 1, project };
  if (mesh) file.meshes = { [meshName]: b64(mesh) };
  return file;
}

/** Read a saved file, a 0.2 project or a 0.1 job: { project, mesh, meshName }. */
export function readProjectFile(text) {
  let obj;
  try {
    obj = JSON.parse(text);
  } catch (e) {
    throw new Error(`not a 4cam project: the file is not JSON (${e.message})`);
  }
  if (obj && obj.format === FILE_FORMAT) {
    const [name, data] = Object.entries(obj.meshes || {})[0] || [];
    return { project: obj.project, mesh: data ? unb64(data) : null, meshName: name || "" };
  }
  if (obj && (obj.schema_version || obj.setups || obj.programs)) return { project: obj, mesh: null, meshName: "" };
  throw new Error("not a 4cam project: no project, setups or programs in the file");
}

/** The mesh file a project names (its part's mesh model, or a 0.1 job's part.mesh). */
export function meshFileOf(project) {
  const m = project?.models?.find((x) => x.kind === "mesh" && (project.setups || []).some((s) => (s.part || []).includes(x.id)));
  return m?.file ?? project?.part?.mesh?.file ?? null;
}

const slug = (s) => (String(s).toLowerCase().replace(/\.[a-z0-9]+$/, "").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "part");

// ------------------------------------------------------------------ the workflow

export function setupProject(ctx) {
  const { $, engine, state, readJob, writeJob, setMesh, showError, setStatus, generate, refreshAll, clearResults, machineArg, currentMachine } = ctx;
  const p = { name: "untitled", dirty: false, fileName: null };
  state.project = p;

  function show() {
    $("projectName").textContent = p.name;
    $("projectDirty").classList.toggle("hidden", !p.dirty);
    $("projectDirty").title = p.dirty ? "unsaved changes" : "";
    document.title = `${p.dirty ? "* " : ""}${p.name} - 4cam`;
  }
  /** Something in the project changed: unsaved, and any results are out of date. */
  function changed() {
    p.dirty = true;
    show();
    ctx.markStale();
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

  async function newProject() {
    if (!mayDiscard("Start a new project")) return false;
    showError("");
    setStatus("New project", "ready");
    const doc = { schema_version: "0.2", name: "untitled", models: [], tools: [], setups: [] };
    writeJob(doc, { load: true });
    setMesh(null, "");
    p.fileName = null;
    clean("untitled");
    clearResults("New project: import an STL (button or drop it here) or open a project to start.");
    await refreshAll();
    return true;
  }

  async function openText(text, fileName) {
    const { project, mesh, meshName } = readProjectFile(text);
    const name = project.name || slug(fileName || "project");
    writeJob(project, { load: true });
    const wanted = meshFileOf(project);
    setMesh(mesh, mesh ? `${meshName} (in the project)` : "");
    p.fileName = fileName || null;
    clean(name);
    const sm = project.machine ?? project.setups?.[0]?.machine;
    if (sm) ctx.selectMachine(sm);
    clearResults("");
    await refreshAll();
    if (wanted && !mesh) {
      setStatus("Opened", "ready");
      showError(`This project uses the mesh "${wanted}", which is not in the file: import it (Import STL) to plan.`);
      return;
    }
    await generate();
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
      const asked = window.prompt("Save the project as:", p.fileName || `${slug(p.name)}.4cam.json`);
      if (!asked) return;
      p.fileName = /\.json$/i.test(asked) ? asked : `${asked}.4cam.json`;
      name = p.fileName.replace(/(\.4cam)?\.json$/i, "").replace(/\.project$/i, "");
      if (project.schema_version === "0.2") project.name = name;
      writeJob(project, { load: true });
    }
    const file = projectFile(project, state.mesh, meshFileOf(project) || state.meshName.split(" ")[0] || "part.stl");
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

  /** Operations for a new part: roughing then finishing, every field from the engine. */
  async function starterOps(doc, setupId, holding) {
    const plan = holding === "vise" ? [["rough3d", 1, "rough"], ["raster3d", 2, "finish"]] : [["rotary_rough", 1, "rough"], ["rotary_finish", 2, "finish"]];
    const s = doc.setups.find((x) => x.id === setupId);
    const base = slug(doc.name);
    for (const [k, [type, tool, what]] of plan.entries()) {
      const { result } = await engine("opDefaults", doc, { type, setup: setupId, tool, machine: typeof machineArg() === "string" ? machineArg() : null }, state.mesh);
      const id = `op${k + 1}`;
      s.operations.push({ id, ...result });
      s.outputs.push({ name: `${base}_${k + 1}_${what}`, title: `${doc.name}: ${what}`, operations: [id] });
    }
  }

  /** The tools a new part starts with: a flat and a ball end mill, long enough to reach. */
  function starterTools(stock) {
    const reach = stock.shape === "box" ? stock.max[2] - stock.min[2] : stock.radius;
    const flute = Math.max(12, Math.ceil(reach + 1));
    return [
      { number: 1, description: `3.175 mm flat end mill, ${flute} mm flute`, kind: "flat", diameter: 3.175, flute_length: flute },
      { number: 2, description: `3.175 mm ball end mill, ${flute} mm flute`, kind: "ball", diameter: 3.175, flute_length: flute },
    ];
  }

  /**
   * Import a mesh file. Into a project without a part it makes one (setup, stock, tools and a
   * starter roughing + finishing); into one with a mesh it asks: replace it (OK) or add the
   * new triangles to it (Cancel). Either way the part is placed again, the stock re-sized,
   * the operations' defaults recomputed, and everything regenerated.
   */
  async function importMesh(file, opts = readImportOptions()) {
    showError("");
    setStatus(`Importing ${file.name}...`, "busy");
    let tris, ascii;
    try {
      ({ tris, ascii } = parseStl(new Uint8Array(await file.arrayBuffer())));
    } catch (e) {
      showError(`${file.name}: ${e.message}`);
      setStatus("Import failed", "error");
      return false;
    }
    if (opts.units === "inch") tris = tris.map((v) => v * 25.4);
    let doc;
    try {
      doc = readJob();
    } catch {
      doc = { schema_version: "0.2", name: "untitled", models: [], tools: [], setups: [] };
    }
    // a 0.1 job is upgraded first; the editor then holds the 0.2 project
    if (doc.schema_version !== "0.2") doc = (await engine("checkProject", doc)).result.project;
    const fresh = !doc.setups?.length || !doc.setups.some((s) => s.operations.length);
    const hadMesh = !!state.mesh && !!meshFileOf(doc);
    if (hadMesh && !fresh && !window.confirm(`Replace the part with ${file.name}? (OK: replace it; Cancel: add ${file.name} to the part)`)) {
      const old = parseStl(state.mesh).tris;
      const both = new Float32Array(old.length + tris.length);
      both.set(old);
      both.set(tris, old.length);
      tris = both;
    }
    const meshFile = `${slug(file.name)}.stl`;
    const bytes = encodeStl(tris, `4cam: ${file.name}`);
    const holding = fresh ? opts.holding : doc.setups[0].stock?.shape === "box" ? "vise" : "rotary";
    const place = placePart(tris, { holding, axis: opts.axis, up: opts.up });
    const model = { id: "part", name: file.name, kind: "mesh", file: meshFile, axis: place.axis, translate: place.translate.map(r3) };
    if (fresh) {
      const name = doc.name && doc.name !== "untitled" ? doc.name : slug(file.name);
      const machine = currentMachine();
      doc = {
        schema_version: "0.2",
        name,
        models: [model],
        tools: starterTools(place.stock),
        setups: [{
          id: "setup1",
          name: holding === "vise" ? "Vise, top side" : "Rotary setup",
          ...(machine?.id ? { machine: machine.id } : {}),
          stock: place.stock,
          clearance: 3,
          holding: { kind: holding === "vise" ? "vise" : "rotary_chuck", tailstock: false },
          part: ["part"],
          spindle_rpm: 12000,
          operations: [],
          outputs: [],
        }],
      };
      p.name = name;
    } else {
      // the new part replaces the setups' parts; profiles go (the mesh is the part now)
      doc.models = [...doc.models.filter((m) => m.id !== "part" && m.kind !== "mesh" && m.kind !== "profile"), model];
      for (const s of doc.setups) {
        s.part = ["part"];
        if ((s.stock?.shape === "box") === (holding === "vise")) s.stock = place.stock;
      }
    }
    setMesh(bytes, `${file.name} (${ascii ? "ASCII" : "binary"} STL${opts.units === "inch" ? ", inch" : ""}, ${tris.length / 9} triangles): ` +
      `${place.size.map((v) => v.toFixed(2)).join(" x ")} mm, ` +
      (holding === "vise" ? `on the vise, block ${place.stock.max[0]} x ${place.stock.max[1]} x ${-place.stock.min[2]} mm` : `on the rotary axis (front at X0), bar R${place.stock.radius} x ${place.stock.length} mm`));
    if (fresh) {
      await starterOps(doc, "setup1", holding);
    } else {
      // every operation's defaults again, for the new part; picked faces no longer apply
      const dropped = [];
      for (const s of doc.setups) {
        for (const [i, op] of [...s.operations.entries()]) {
          try {
            const { result } = await engine("opDefaults", doc, { type: op.type, setup: s.id, tool: op.tool, machine: typeof machineArg() === "string" ? machineArg() : null }, state.mesh);
            s.operations[i] = { ...result, id: op.id, enabled: op.enabled, ...(op.name ? { name: op.name } : {}), ...(op.color ? { color: op.color } : {}) };
          } catch (e) {
            dropped.push(`${op.id} (${op.type}: ${e.message})`);
            s.operations[i] = null;
          }
        }
        s.operations = s.operations.filter(Boolean);
        const ids = new Set(s.operations.map((o) => o.id));
        for (const o of s.outputs) o.operations = o.operations.filter((id) => ids.has(id));
      }
      if (dropped.length) showError(`Removed what no longer fits the new part: ${dropped.join("; ")}`);
    }
    writeJob(doc);
    changed();
    clearResults("");
    await refreshAll();
    await generate();
    return true;
  }

  function readImportOptions() {
    return { units: $("importUnits").value, axis: $("importAxis").value, up: $("importAxis").value, holding: $("importHolding").value };
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
  $("importHolding").addEventListener("change", () => {
    const vise = $("importHolding").value === "vise";
    $("importAxisLabel").textContent = vise ? "up" : "axis";
    $("importAxis").value = vise ? "z" : "x";
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
    else showError(`${f.name}: drop an .stl to import it, or a .4cam.json / .project.json to open it`);
  });
  window.addEventListener("beforeunload", (ev) => {
    if (p.dirty) {
      ev.preventDefault();
      ev.returnValue = "";
    }
  });
  show();

  return { changed, clean, mayDiscard, newProject, openText, openFile, save, importMesh, state: p };
}
