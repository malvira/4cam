// 4cam web UI: job editor -> engine (WebAssembly, in a worker) -> G-code, checks, previews.

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
function engine(cmd, ...args) {
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
window.__4cam = { state, engine };

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

function writeJob(job) {
  $("job").value = JSON.stringify(job, null, 2);
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
      for (const k of path.slice(0, -1)) o = o[k];
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
  if (name === "knob") {
    const { result } = await engine("exampleJob");
    writeJob(result);
    setMesh(null, "");
  } else {
    const [job, stl] = await Promise.all([
      fetch("../examples/knob/knob_finish.job.json").then((r) => {
        if (!r.ok) throw new Error("examples/knob/knob_finish.job.json not found (serve the repository root)");
        return r.json();
      }),
      fetch("../examples/knob/knob.stl").then((r) => (r.ok ? r.arrayBuffer() : null)),
    ]);
    writeJob(job);
    setMesh(stl ? new Uint8Array(stl) : null, stl ? "knob.stl (example)" : "");
  }
  renderQuick();
}

function setMesh(bytes, name) {
  state.mesh = bytes;
  state.meshName = name;
  $("meshInfo").textContent = bytes ? `${name}: ${(bytes.length / 1024).toFixed(0)} KiB` : "No mesh loaded";
}

function machineArg() {
  const v = $("machine").value;
  if (v !== "custom") return v;
  return $("machineJson").value;
}

// ---------------------------------------------------------------- generate
async function generate() {
  showError("");
  let job;
  try {
    job = readJob();
  } catch (e) {
    showError(e.message);
    return;
  }
  if (job.part?.mesh && !state.mesh) {
    showError(`This job uses the mesh "${job.part.mesh.file}". Choose it under "Part mesh".`);
    return;
  }
  $("generate").disabled = $("simulate").disabled = true;
  setStatus("Generating...", "busy");
  try {
    const { result, ms } = await engine("planAndPost", job, machineArg(), state.mesh, { max_points: 150000 });
    state.result = result;
    state.sim = null;
    state.job = job;
    renderTabs();
    selectTab(0);
    const issues = result.programs.reduce((n, p) => n + p.issues.length, 0);
    setStatus(`Generated ${result.programs.length} program(s) in ${(ms / 1000).toFixed(1)} s, ${issues} issue(s)`, "done");
  } catch (e) {
    showError(e.message);
    setStatus("Failed", "error");
  } finally {
    $("generate").disabled = $("simulate").disabled = false;
  }
}

async function simulate() {
  if (!state.result) await generate();
  if (!state.result) return;
  $("generate").disabled = $("simulate").disabled = true;
  setStatus("Simulating stock...", "busy");
  try {
    const { result, ms } = await engine("simulate", state.job, { dx: 0.1, na: 240 }, state.mesh);
    state.sim = result;
    selectTab(state.tab);
    const s = result.stocks[state.tab];
    setStatus(`Simulated in ${(ms / 1000).toFixed(1)} s`, "done");
    if (s && (s.rapid_hits || s.gouge_cells)) showError(`Simulation: ${s.rapid_hits} rapid(s) through stock, ${s.gouge_cells} gouged cell(s)`);
  } catch (e) {
    showError(e.message);
    setStatus("Simulation failed", "error");
  } finally {
    $("generate").disabled = $("simulate").disabled = false;
  }
}

// ---------------------------------------------------------------- results
function renderTabs() {
  const tabs = $("tabs");
  tabs.innerHTML = "";
  state.result.programs.forEach((p, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "tab");
    b.dataset.index = i;
    const n = p.issues.length;
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
    <div><strong>${esc(p.title)}</strong> &middot; <span id="lineCount">${p.lines.toLocaleString("en")}</span> lines
      &middot; ${state.result.machine}${p.tool ? ` &middot; T${p.tool[0]} ${esc(p.tool[1])}` : ""}</div>
    <div class="issues ${p.issues.length ? "bad" : "ok"}">${
      p.issues.length
        ? `${p.issues.length} verifier issue(s):<ul>${p.issues.slice(0, 200).map((s) => `<li>${esc(s)}</li>`).join("")}</ul>`
        : "Verifier: 0 issues (no fast plunges, no low rapids)"
    }</div>
    ${g && g.moves_checked ? `<div class="small">Gouge check: ${g.moves_checked.toLocaleString("en")} cut moves, deepest ${g.max_depth.toFixed(4)} mm into the part</div>` : ""}
    ${sim && !sim.samples ? `<div class="small muted">Not simulated: its tool has no full shape (a V-bit needs a diameter). The stock shown is from the programs before it.</div>` : ""}
    ${sim ? `<div class="small">Simulated stock: material left max ${sim.max_left.toFixed(3)} mm, mean ${sim.mean_left.toFixed(3)} mm; deepest gouge ${sim.max_gouge.toFixed(3)} mm; ${sim.rapid_hits} rapid(s) through stock</div>` : ""}`;
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
  const at = n > 0 ? `X${pts[4 * (n - 1)].toFixed(2)} Z${pts[4 * (n - 1) + 1].toFixed(2)} A${pts[4 * (n - 1) + 2].toFixed(1)}` : "";
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
  let x0 = Infinity, x1 = -Infinity;
  for (let i = 0; i < pts.length; i += 4) if (pts[i + 3]) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); }
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
    if (!cut && !$("showRapids").checked) continue;
    ctx.strokeStyle = cut ? "rgba(37,99,235,0.35)" : "rgba(220,38,38,0.35)";
    ctx.beginPath();
    ctx.moveTo(sx(pts[4 * (i - 1)]), sy(pts[4 * (i - 1) + 1]));
    ctx.lineTo(sx(pts[4 * i]), sy(pts[4 * i + 1]));
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
    scene.add(new THREE.AxesHelper(5));
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
    const loop = () => {
      controls.update();
      renderer.render(scene, camera);
      requestAnimationFrame(loop);
    };
    loop();
    state.three = { THREE, renderer, scene, camera, controls, groups, fitted: null };
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
    c.geometry?.dispose();
    c.material?.dispose();
  }
}

/** Machine (x, r, a) -> part frame xyz: the tool at A sits over part angle 90 - A. */
function toXYZ(x, r, aDeg) {
  const phi = ((90 - aDeg) * Math.PI) / 180;
  return [x, r * Math.cos(phi), r * Math.sin(phi)];
}

async function updateView() {
  const v = await getThree();
  if (!v || v.failed || !state.result) return;
  const { THREE, groups } = v;
  const res = state.result;
  const p = res.programs[state.tab];
  const key = `${state.tab}:${res.programs.length}:${p.lines}`;

  if (v.key !== key) {
    v.key = key;
    // toolpath as two line-segment sets with prefix counts for the scrubber
    clear(groups.path);
    const pts = p.toolpath.points;
    const n = pts.length / 4;
    const cut = [], rap = [];
    const cutUpTo = new Uint32Array(n + 1), rapUpTo = new Uint32Array(n + 1);
    for (let i = 1; i < n; i++) {
      const a = toXYZ(pts[4 * (i - 1)], pts[4 * (i - 1) + 1], pts[4 * (i - 1) + 2]);
      const b = toXYZ(pts[4 * i], pts[4 * i + 1], pts[4 * i + 2]);
      (pts[4 * i + 3] ? cut : rap).push(...a, ...b);
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

    // stock cylinder and part
    clear(groups.stock);
    clear(groups.part);
    const prof = res.profile?.points;
    let xs = [];
    for (let i = 0; i < pts.length; i += 4) if (pts[i + 3]) xs.push(pts[i]);
    if (prof) xs.push(...prof.map((q) => q[0]));
    if (res.mesh_bounds) xs.push(res.mesh_bounds[0][0], res.mesh_bounds[1][0]);
    const xmin = Math.min(...xs) - 3, xmax = Math.max(...xs) + 1;
    const R = res.stock.radius;
    const cyl = new THREE.CylinderGeometry(R, R, xmax - xmin, 96, 1, true);
    cyl.rotateZ(-Math.PI / 2);
    cyl.translate((xmin + xmax) / 2, 0, 0);
    groups.stock.add(new THREE.Mesh(cyl, new THREE.MeshStandardMaterial({ color: 0x9ca3af, transparent: true, opacity: 0.12, side: THREE.DoubleSide, depthWrite: false })));
    const partMat = new THREE.MeshStandardMaterial({ color: 0xc8ccd2, metalness: 0.3, roughness: 0.5, side: THREE.DoubleSide, transparent: true, opacity: 0.85 });
    if (state.mesh && res.mesh_bounds) {
      const { result } = await engine("meshTriangles", state.job, { max_triangles: 150000 }, state.mesh);
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.Float32BufferAttribute(result.triangles, 3));
      g.computeVertexNormals();
      groups.part.add(new THREE.Mesh(g, partMat));
    } else if (prof && prof.length > 1) {
      // LatheGeometry revolves (radius, y) about Y; turn Y into X
      const lathe = new THREE.LatheGeometry(prof.map(([x, r]) => new THREE.Vector2(r, x)), 120);
      lathe.rotateZ(-Math.PI / 2);
      groups.part.add(new THREE.Mesh(lathe, partMat));
    }
    if (!v.fitted) {
      const span = xmax - xmin;
      v.camera.position.set((xmin + xmax) / 2 - span * 0.2, -span * 1.3, R * 2.2);
      v.controls.target.set((xmin + xmax) / 2, 0, 0);
      v.fitted = true;
    }
    v.simKey = null;
  }

  // simulated stock surface for this program
  const simKey = state.sim ? `${key}:sim` : null;
  if (v.simKey !== simKey) {
    v.simKey = simKey;
    clear(groups.sim);
    const s = state.sim?.stocks?.[state.tab];
    if (s) groups.sim.add(simMesh(THREE, state.sim, s.r));
  }
  groups.sim.visible = $("showSim").checked;
  groups.part.visible = $("showPart").checked && !(groups.sim.children.length && groups.sim.visible);
  groups.stock.visible = $("showStock").checked && !(groups.sim.children.length && groups.sim.visible);
  v.rapLines.visible = $("showRapids").checked;

  // scrubber
  const k = +$("scrub").value;
  v.cutLines.geometry.setDrawRange(0, 2 * v.cutUpTo[Math.min(k, v.cutUpTo.length - 1)]);
  v.rapLines.geometry.setDrawRange(0, 2 * v.rapUpTo[Math.min(k, v.rapUpTo.length - 1)]);
  clear(groups.tool);
  const pts = p.toolpath.points;
  if (k > 0) {
    const [x, y, z] = toXYZ(pts[4 * (k - 1)], pts[4 * (k - 1) + 1], pts[4 * (k - 1) + 2]);
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

/** Heightmap -> surface around X, coloured by material left over the part (0..0.5 mm). */
function simMesh(THREE, sim, r) {
  const { x0, dx, nx, na } = sim.grid;
  const pos = new Float32Array(nx * na * 3), col = new Float32Array(nx * na * 3);
  for (let i = 0; i < nx; i++) {
    const x = x0 + (i + 0.5) * dx;
    for (let j = 0; j < na; j++) {
      const phi = ((j + 0.5) * 2 * Math.PI) / na;
      const k = i * na + j, rr = r[k], part = sim.part[k];
      pos.set([x, rr * Math.cos(phi), rr * Math.sin(phi)], 3 * k);
      col.set(part > 0 ? ramp((rr - part) / 0.5) : [0.75, 0.75, 0.75], 3 * k);
    }
  }
  const idx = [];
  for (let i = 0; i + 1 < nx; i++)
    for (let j = 0; j < na; j++) {
      const a = i * na + j, b = i * na + ((j + 1) % na), c = (i + 1) * na + j, d = (i + 1) * na + ((j + 1) % na);
      idx.push(a, b, c, b, d, c);
    }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  g.setAttribute("color", new THREE.BufferAttribute(col, 3));
  g.setIndex(idx);
  g.computeVertexNormals();
  return new THREE.Mesh(g, new THREE.MeshStandardMaterial({ vertexColors: true, side: THREE.DoubleSide, roughness: 0.7 }));
}

// ---------------------------------------------------------------- wiring
$("loadExample").addEventListener("click", () => loadExample($("example").value).catch((e) => showError(e.message)));
$("generate").addEventListener("click", generate);
$("simulate").addEventListener("click", simulate);
$("download").addEventListener("click", download);
$("job").addEventListener("change", renderQuick);
$("machine").addEventListener("change", async () => {
  const custom = $("machine").value === "custom";
  $("machineJson").classList.toggle("hidden", !custom);
  if (custom && !$("machineJson").value) {
    const { result } = await engine("machine", "makera-z1");
    $("machineJson").value = JSON.stringify(result, null, 2);
  }
});
$("mesh").addEventListener("change", async () => {
  const f = $("mesh").files[0];
  if (f) setMesh(new Uint8Array(await f.arrayBuffer()), f.name);
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
    await loadExample("knob");
    setMesh(null, "");
    $("generate").disabled = $("simulate").disabled = false;
    setStatus("Ready", "ready");
  } catch (e) {
    setStatus("Engine failed to load", "error");
    showError(`${e.message}\nBuild it first: scripts/build-wasm.sh (or .ps1)`);
  }
})();
