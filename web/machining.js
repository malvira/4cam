// Solid machining simulation: the stock as a closed solid that the tool cuts as the job plays.
// The engine (WebAssembly, in the worker) keeps the stock heightmap and the cut-by-operation
// tags; this module plays it back, updates the GPU buffers in place for the rows that changed,
// and colours each cut surface by the operation that cut it. Rotary setups play on a bar
// (rows = X cells, A turns the work); 3-axis (vise) setups on a block (rows = Y cells, the
// tool moves in XYZ).

import { blockSolid, heightmapSolid, stockObject, STOCK_LEFT } from "./meshes.js";
import { GpuStock } from "./gpu/gpusim.js";
import { colorNum, colorRgb } from "./colors.js";

const PALETTE = ["#3b82f6", "#f59e0b", "#10b981", "#ef4444", "#a855f7", "#14b8a6", "#e11d48", "#84cc16"];
// stock not cut yet: the stock's tint (distinct from cuts and from the part)
// (View > Colours: the uncut stock's colour)
const uncut = () => colorRgb("uncut") || STOCK_LEFT;

const b64ToBytes = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const f32 = (s) => new Float32Array(b64ToBytes(s).buffer);
const u16 = (s) => new Uint16Array(b64ToBytes(s).buffer);
const hexToRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);

// The 3D display shows the engine grid every k cells (each display cell the deepest cut of
// its block) when the grid is large: at most DISPLAY_MAX cells at rest, PLAY_MAX while playing
// (it refines again on pause).
const DISPLAY_MAX = 1_000_000;
const PLAY_MAX = 250_000;

export function setupMachining({ engine, getThree, $, state, readJob, setStatus, showError, libraries, simOpts, confirmSimRes, simGpu, showBackend, simLock, machineArg }) {
  const m = {
    open: false,
    session: null, // sim_open result (+ decoded arrays)
    r: null, // Float32Array radii, or block tops for 3-axis (mirror of the engine stock)
    xyz: false,
    op: null, // Uint16Array cut-by tags
    pos: 0,
    playing: false,
    busy: false,
    target: 0,
    last: 0,
    colors: {}, // tag -> [r, g, b]
    hiddenTags: new Set(), // operations hidden in the tree (view only): cuts in grey, no tool
  };
  state.machining = m;
  // bumped by reset(): an open() still waiting on the engine then drops its result
  let epoch = 0;

  /** Dispose an object tree's geometries and materials. */
  const disposeTree = (o) =>
    o.traverse((c) => {
      // (the part's copies share the main view's geometry)
      if (c.userData.shared) return;
      c.geometry?.dispose();
      for (const mat of [].concat(c.material || [])) mat.dispose();
    });

  // ---- the playback's controls: speed in machine time (×1 to ×1000, a log slider), the
  // stock toggle; both remembered
  const remember = (k, v) => {
    try {
      localStorage.setItem(k, String(v));
    } catch {}
  };
  const recall = (k) => {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  };
  const speedFactor = () => Math.pow(10, Math.max(0, Math.min(3, +$("simSpeed").value || 0)));
  function showSpeed() {
    const f = speedFactor();
    $("simSpeedLabel").textContent = `×${f < 10 ? f.toFixed(1).replace(/\.0$/, "") : Math.round(f)}`;
  }
  {
    const sp = recall("marcam.playSpeed");
    if (sp !== null && Number.isFinite(+sp)) $("simSpeed").value = sp;
    const st = recall("marcam.playStock");
    if (st !== null) $("simStock").checked = st === "1";
    showSpeed();
  }
  /** Machine time (s) when `n` segments have been played. */
  const timeAt = (n) => (n > 0 && m.session?.times ? m.session.times[Math.min(n, m.session.times.length) - 1] : 0);
  /** Segments played by machine time `t`. */
  function segmentsBy(t) {
    const ts = m.session.times;
    let lo = 0, hi = ts.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (ts[mid] <= t) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }
  const fmtClock = (t) => {
    const h = Math.floor(t / 3600), mi = Math.floor((t % 3600) / 60), se = Math.floor(t % 60);
    return h ? `${h}:${String(mi).padStart(2, "0")}:${String(se).padStart(2, "0")}` : `${mi}:${String(se).padStart(2, "0")}`;
  };

  const opKey = (o) => `${o.program}#${o.id}#${o.kind}`;
  const savedColors = () => {
    try {
      return JSON.parse(localStorage.getItem("marcam.opColors") || "{}");
    } catch {
      return {};
    }
  };

  function renderLegend() {
    const box = $("opLegend");
    box.innerHTML = "";
    if (!m.session) return;
    const saved = savedColors();
    const proj = m.session.project_ops;
    // colours follow the operation (its id), as the tree shows them: the same whatever is
    // hidden (hidden operations are not played, so the list has fewer entries)
    const setupOps = state.tree?.doc?.setups?.find((x) => x.id === state.tree?.setup)?.operations || [];
    m.session.ops.forEach((o, i) => {
      if (proj) o.project_id = proj.ids[i];
      const k = o.project_id != null ? setupOps.findIndex((x) => x.id === o.project_id) : -1;
      const hex = (proj && proj.colors[i]) || saved[opKey(o)] || PALETTE[(k >= 0 ? k : i) % PALETTE.length];
      m.colors[o.tag] = hexToRgb(hex);
      const row = document.createElement("label");
      row.className = "oprow";
      const hid = m.hiddenTags.has(o.tag);
      row.classList.toggle("op-hidden", hid);
      row.innerHTML = `<input type="color" value="${hex}" data-tag="${o.tag}" aria-label="Colour for ${o.program} ${o.kind}"> <span>${o.program}: ${o.kind.replace(/_/g, " ")}${o.project_id ? ` (${o.project_id})` : ""}${hid ? " (hidden)" : ""}</span>`;
      box.appendChild(row);
    });
    const un = document.createElement("div");
    un.className = "oprow small muted";
    un.innerHTML = `<span class="swatch" style="background: rgb(${uncut().map((c) => Math.round(c * 255)).join(",")})"></span> uncut stock`;
    box.appendChild(un);
  }

  // a hidden operation's cuts: grey (the material is still gone; it is just not highlighted)
  const HIDDEN_CUT = [0.5, 0.52, 0.56];
  const colorOf = (tag) => (tag ? (m.hiddenTags.has(tag) ? HIDDEN_CUT : m.colors[tag] || [1, 0, 1]) : uncut());

  /** Which playback operations the tree's eyes hide (by project id, or a 0.1 job's planning
   *  order). */
  function refreshHidden() {
    // (hidden operations are left out of the playback altogether: nothing played is hidden)
    m.hiddenTags = new Set();
  }

  /** The tree's eyes changed: the playback plays the visible operations, so it is opened
   *  again at the same place (nothing is planned again). */
  async function applyHidden() {
    if (!m.session) return;
    if (m.open) {
      const at = m.pos;
      await open();
      if (m.session) await seek(Math.min(at, m.session.segments));
    } else {
      refreshHidden();
      renderLegend();
    }
  }

  // the engine grid as rows x cols (rotary: X rows of na cells; 3-axis: Y rows of nx cells)
  const dims = () => {
    const g = m.session.grid;
    return m.xyz ? { R: g.ny, C: g.nx } : { R: g.nx, C: g.na };
  };
  /** Smallest stride whose display grid has at most `limit` cells (rotary: dividing na). */
  function strideFor(limit) {
    const { R, C } = dims();
    let k = 1;
    for (;;) {
      if ((m.xyz || C % k === 0) && Math.ceil(R / k) * Math.ceil(C / k) <= limit) return k;
      k++;
      if (k > 64) return 64;
    }
  }
  function displayGrid(k) {
    const g = m.session.grid;
    return m.xyz
      ? { x0: g.x0, y0: g.y0, cell: g.cell * k, nx: Math.ceil(g.nx / k), ny: Math.ceil(g.ny / k) }
      : { x0: g.x0, dx: g.dx * k, nx: Math.ceil(g.nx / k), na: g.na / k };
  }
  /** Recompute display rows rd0..rd1 from the engine mirror (deepest cell of each block). */
  function fillRows(rd0, rd1) {
    const k = m.stride;
    const { R, C } = dims();
    const CD = Math.ceil(C / k);
    for (let rd = rd0; rd <= rd1; rd++) {
      for (let cd = 0; cd < CD; cd++) {
        let best = Infinity, tag = 0;
        for (let r = rd * k; r < Math.min(R, rd * k + k); r++)
          for (let c = cd * k; c < Math.min(C, cd * k + k); c++) {
            const v = m.r[r * C + c];
            if (v < best) {
              best = v;
              tag = m.op[r * C + c];
            }
          }
        m.dr[rd * CD + cd] = best;
        m.dop[rd * CD + cd] = tag;
      }
    }
  }

  /** Build the stock solid and the tool from the current arrays. */
  async function buildScene() {
    const v = await getThree();
    if (!v || v.failed) return;
    const { THREE } = v;
    if (!v.groups.machining) {
      v.groups.machining = new THREE.Group();
      v.scene.add(v.groups.machining);
      v.groups.work = new THREE.Group(); // turns with A
      v.groups.machining.add(v.groups.work);
      v.groups.toolModel = new THREE.Group();
      v.groups.machining.add(v.groups.toolModel);
    }
    const work = v.groups.work;
    for (const c of [...work.children]) {
      work.remove(c);
      disposeTree(c);
    }
    const s = m.session;
    // the toolpath played so far, on the part (it turns with A)
    buildPath(THREE, work);
    if (!m.stock) {
      // no stock: the part itself turns under the tool
      for (const c of v.groups.part.children) {
        const k = c.clone(true);
        k.traverse((x) => (x.userData.shared = true));
        work.add(k);
      }
      m.stockMesh = null;
      m.ghost = null;
      m.toolNumber = null;
      showMachining(true);
      placeTool();
      return;
    }
    const { R, C } = dims();
    m.dgrid = displayGrid(m.stride);
    m.dr = new Float32Array(Math.ceil(R / m.stride) * Math.ceil(C / m.stride));
    m.dop = new Uint16Array(m.dr.length);
    fillRows(0, Math.ceil(R / m.stride) - 1);
    const mesh = m.xyz ? blockSolid(m.dgrid, m.dr, s.stock_box.min[2], (k) => colorOf(m.dop[k])) : heightmapSolid(m.dgrid, m.dr, (k) => colorOf(m.dop[k]));
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(mesh.positions, 3));
    g.setAttribute("color", new THREE.BufferAttribute(mesh.colors, 3));
    g.setIndex(new THREE.BufferAttribute(mesh.indices, 1));
    g.computeVertexNormals();
    m.stockMesh = new THREE.Mesh(g, new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.55, metalness: 0.15 }));
    work.add(m.stockMesh);
    // ghost of the original bar or block: shows how much has been removed
    let ghost;
    if (m.xyz) {
      const { min, max } = s.stock_box;
      ghost = stockObject(THREE, new THREE.BoxGeometry(max[0] - min[0], max[1] - min[1], max[2] - min[2]), 0.1);
      ghost.position.set((min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2);
    } else {
      const { x0, dx, nx } = s.grid;
      ghost = stockObject(THREE, new THREE.CylinderGeometry(s.stock_radius, s.stock_radius, nx * dx, 64, 1, false).rotateZ(-Math.PI / 2).translate(x0 + (nx * dx) / 2, 0, 0), 0.1);
    }
    ghost.visible = $("simGhost").checked;
    m.ghost = ghost;
    work.add(ghost);
    m.toolNumber = null;
    showMachining(true);
    placeTool();
  }

  /** The played toolpath as lines in the part's frame (machine tip turned back by A), each
   *  cut in its operation's colour, rapids red; drawn up to the playback's position. */
  function buildPath(THREE, work) {
    const p = m.session.path;
    const n = m.session.segments;
    const pos = new Float32Array(n * 6);
    const col = new Float32Array(n * 6);
    let prev = null;
    for (let k = 0; k < n; k++) {
      const [x, y, z, a, rapid, , tag] = p.subarray(7 * k, 7 * k + 7);
      const r = (a * Math.PI) / 180;
      const c = Math.cos(r), sn = Math.sin(r);
      const q = [x, y * c + z * sn, -y * sn + z * c];
      const from = prev || q;
      pos.set(from, 6 * k);
      pos.set(q, 6 * k + 3);
      const rgb = rapid ? colorRgb("rapid") : m.colors[tag] || colorRgb("cut");
      col.set(rgb, 6 * k);
      col.set(rgb, 6 * k + 3);
      prev = q;
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    g.setAttribute("color", new THREE.BufferAttribute(col, 3));
    m.pathLine = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.9 }));
    m.pathLine.geometry.setDrawRange(0, 2 * m.pos);
    work.add(m.pathLine);
  }

  /** Only the machining view while it is open. */
  function showMachining(on) {
    const v = state.three;
    if (!v || v.failed) return;
    for (const k of ["part", "stock", "sim", "path", "tool"]) v.groups[k].visible = !on && (k !== "sim" || $("showSim").checked);
    if (!on) {
      v.groups.part.visible = $("showPart").checked;
      v.groups.stock.visible = $("showStock").checked;
    }
    if (v.groups.machining) v.groups.machining.visible = on;
  }

  /** Tool model: shank + cutting end, tip at the origin, axis +Z (the spindle). */
  function toolModel(THREE, t) {
    const grp = new THREE.Group();
    const mat = new THREE.MeshStandardMaterial({ color: colorNum("tool"), metalness: 0.6, roughness: 0.35 });
    const d = t?.diameter ?? 3.175;
    const R = d / 2;
    const flute = Math.max(t?.flute_length ?? 8, 2);
    const add = (geo, z) => {
      const mesh = new THREE.Mesh(geo.rotateX(Math.PI / 2), mat);
      mesh.position.z = z;
      grp.add(mesh);
    };
    if (t?.kind === "vbit") {
      const h = R / Math.tan(((t.angle ?? 90) * Math.PI) / 360);
      // ConeGeometry's apex is at +Y; turned -90deg about X it points down (-Z): tip at 0
      const cone = new THREE.Mesh(new THREE.ConeGeometry(R, h, 32).rotateX(-Math.PI / 2), mat);
      cone.position.z = h / 2;
      grp.add(cone);
      add(new THREE.CylinderGeometry(R, R, 20, 32), h + 10);
    } else if (t?.kind === "ball") {
      const ball = new THREE.Mesh(new THREE.SphereGeometry(R, 24, 16), mat);
      ball.position.z = R;
      grp.add(ball);
      add(new THREE.CylinderGeometry(R, R, flute + 20 - R, 32), R + (flute + 20 - R) / 2);
    } else {
      add(new THREE.CylinderGeometry(R, R, flute + 20, 32), (flute + 20) / 2);
    }
    return grp;
  }

  function placeTool() {
    const v = state.three;
    if (!v || v.failed || !m.session) return;
    const s = m.session;
    const k = Math.max(0, Math.min(m.pos, s.segments) - 1);
    const p = s.path;
    // machine tool tip, A, rapid, tool, operation tag
    const [x, y, z, a, , toolNo, opTag] = p.subarray(7 * k, 7 * k + 7);
    if (toolNo !== m.toolNumber) {
      const tm = v.groups.toolModel;
      for (const c of [...tm.children]) {
        tm.remove(c);
        disposeTree(c);
      }
      tm.add(toolModel(v.THREE, s.tools.find((t) => t.number === toolNo)));
      m.toolNumber = toolNo;
    }
    const op = s.ops.find((o) => o.tag === opTag);
    const where = m.xyz ? `X${x.toFixed(1)} Y${y.toFixed(1)} Z${z.toFixed(2)}` : y ? `A${a.toFixed(1)} · Y${y.toFixed(2)} Z${z.toFixed(2)}` : `A${a.toFixed(1)}`;
    // the work turns by A (right-handed about +X; 0 on a vise); the tool stands at its tip
    v.groups.work.rotation.x = (a * Math.PI) / 180;
    v.groups.toolModel.position.set(x, y, z);
    // the tool is always drawn: what is played is what is shown
    v.groups.toolModel.visible = true;
    m.pathLine?.geometry.setDrawRange(0, 2 * Math.min(m.pos, s.segments));
    const clock = s.times?.length ? ` · ${fmtClock(timeAt(m.pos))} / ${fmtClock(s.times[s.times.length - 1])}` : "";
    $("simLabel").textContent = `move ${m.pos.toLocaleString("en")} / ${s.segments.toLocaleString("en")}${clock}${op ? ` · ${op.program}: ${op.kind.replace(/_/g, " ")}` : ""} · ${where} · ${m.stock ? (m.gpu ? "GPU" : "CPU") : "no stock"}`;
  }

  /** Apply a sim_seek result: rows changed -> update vertices and colours in place. */
  function applyRows(res) {
    m.pos = res.pos;
    if (res.rows) {
      // the engine mirror first (rows of C cells), then the display rows they fall in; the
      // WASM playback sends base64, the GPU one typed arrays
      const { C } = dims();
      const [r0] = res.rows;
      const h = m.xyz ? res.z : res.r;
      m.r.set(typeof h === "string" ? f32(h) : h, r0 * C);
      m.op.set(typeof res.op === "string" ? u16(res.op) : res.op, r0 * C);
    }
    const k = m.stride;
    if (res.rows && m.stockMesh && m.xyz) {
      // display rows are Y rows; vertex v is display cell v (the walls share those vertices)
      const [j0, j1] = res.rows.map((r) => Math.floor(r / k));
      fillRows(j0, j1);
      const { nx } = m.dgrid;
      const pos = m.stockMesh.geometry.attributes.position.array;
      const col = m.stockMesh.geometry.attributes.color.array;
      for (let v = j0 * nx; v < (j1 + 1) * nx; v++) {
        pos[3 * v + 2] = m.dr[v];
        col.set(colorOf(m.dop[v]), 3 * v);
      }
      m.stockMesh.geometry.attributes.position.needsUpdate = true;
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
      m.stockMesh.geometry.computeVertexNormals();
    } else if (res.rows && m.stockMesh) {
      const [i0, i1] = res.rows.map((r) => Math.floor(r / k));
      fillRows(i0, i1);
      const { na, nx } = m.dgrid;
      const pos = m.stockMesh.geometry.attributes.position.array;
      const col = m.stockMesh.geometry.attributes.color.array;
      const setV = (vi, i, j) => {
        const k = i * na + j;
        const phi = ((j + 0.5) * 2 * Math.PI) / na;
        pos[3 * vi + 1] = m.dr[k] * Math.cos(phi);
        pos[3 * vi + 2] = m.dr[k] * Math.sin(phi);
        const c = colorOf(m.dop[k]);
        col[3 * vi] = c[0];
        col[3 * vi + 1] = c[1];
        col[3 * vi + 2] = c[2];
      };
      for (let i = i0; i <= i1; i++) for (let j = 0; j < na; j++) setV(i * na + j, i, j);
      // end caps repeat the first and last rings
      if (i0 === 0) for (let j = 0; j < na; j++) setV(nx * na + j, 0, j);
      if (i1 === nx - 1) for (let j = 0; j < na; j++) setV(nx * na + na + 1 + j, nx - 1, j);
      m.stockMesh.geometry.attributes.position.needsUpdate = true;
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
      m.stockMesh.geometry.computeVertexNormals();
    }
    $("simScrub").value = String(m.pos);
    placeTool();
  }

  async function seek(n) {
    if (!m.session) return;
    // no stock: only the tool and the path move
    if (!m.stock) {
      m.pos = Math.max(0, Math.min(n, m.session.segments));
      $("simScrub").value = String(m.pos);
      placeTool();
      return;
    }
    m.target = n;
    if (m.busy) return;
    m.busy = true;
    const session = m.session;
    try {
      while (m.target !== m.pos) {
        const want = m.target;
        let result;
        if (m.gpu) {
          // on the GPU: run the segments, read back only the rows they touched
          const gpu = m.gpu;
          const rows = gpu.seek(want);
          result = { pos: gpu.pos, rows: null };
          if (rows) {
            const { h, op } = gpu.decode(await gpu.readRows(rows[0], rows[1]));
            result = { pos: gpu.pos, rows, [m.xyz ? "z" : "r"]: h, op };
          }
        } else {
          result = (await engine("simSeek", want)).result;
        }
        // closed (or another opened) while the engine worked: nothing to show
        if (m.session !== session || !result) break;
        applyRows(result);
        if (result.pos === want || m.target === want) break;
      }
    } catch (e) {
      // cancelled (the playback went with the engine's worker): nothing more to show
      if (e.message !== "cancelled") throw e;
    } finally {
      m.busy = false;
    }
  }

  async function open() {
    showError("");
    let job;
    try {
      job = readJob();
    } catch (e) {
      return showError(e.message);
    }
    // one simulation at a time (the engine does one thing at a time, and two on the GPU at
    // once can stall it): after the one running
    if (!simLock) return openLocked(job);
    return simLock.run("playback", () => {
      setStatus("Opening machining simulation...", "busy");
      return openLocked(job);
    });
  }

  async function openLocked(job) {
    // the stock simulated along (Stock on), or only the tool and the path
    const stock = !!$("simStock").checked;
    if (stock && confirmSimRes && !(await confirmSimRes())) {
      setStatus("Machining simulation not started", "done");
      return;
    }
    const mine = epoch;
    const keep = m.open ? m.pos : 0;
    try {
      const setup = job.schema_version === "0.2" ? state.tree?.setup : undefined;
      const machine = machineArg?.() ?? null;
      const { result } = await engine("simOpen", job, { ...(simOpts?.() || {}), libraries: libraries(), setup, machine: typeof machine === "string" ? machine : null, path_only: !stock }, state.mesh);
      result.path = f32(result.path);
      result.times = result.times ? f32(result.times) : null;
      m.session = result;
      m.stock = stock;
      m.xyz = result.kind === "xyz";
      m.pos = 0;
      m.clock = null;
      m.gpu?.destroy();
      m.gpu = null;
      if (!stock) {
        if (mine !== epoch) return void (m.session = null);
        m.backend = "";
        renderLegend();
        await buildScene();
        $("simScrub").max = String(result.segments);
        $("simScrub").value = "0";
        $("simControls").classList.remove("hidden");
        m.open = true;
        document.body.dataset.machining = "open";
        if (keep) await seek(Math.min(keep, result.segments));
        setStatus(`Playback: ${result.segments.toLocaleString("en")} moves, ${result.ops.length} operations, ${fmtClock(result.times?.[result.times.length - 1] || 0)} of machining (no stock)`, "done");
        return;
      }
      const n = m.xyz ? result.grid.nx * result.grid.ny : result.grid.nx * result.grid.na;
      m.r = new Float32Array(n).fill(m.xyz ? result.stock_box.max[2] : result.stock_radius);
      m.op = new Uint16Array(n);
      m.stride = strideFor(DISPLAY_MAX);
      // the stock on the GPU when there is one and the engine offers the job to it
      const { gpu, why } = simGpu ? await simGpu() : { gpu: null, why: "" };
      let backend = `CPU (${why})`;
      if (gpu) {
        try {
          const { result: g } = await engine("gpuOpen", job, { ...(simOpts?.() || {}), libraries: libraries(), setup }, state.mesh);
          const same = m.xyz ? g.grid.nx === result.grid.nx && g.grid.ny === result.grid.ny : g.grid.nx === result.grid.nx && g.grid.na === result.grid.na;
          if (!same || g.input.seg_start.length === 0) throw new Error("grid mismatch");
          m.gpu = new GpuStock(gpu, g);
          if (m.gpu.segments !== result.segments) throw new Error(`segments ${m.gpu.segments} vs ${result.segments}`);
          backend = `GPU (${gpu.name})`;
        } catch (e) {
          m.gpu?.destroy();
          m.gpu = null;
          backend = `CPU (${e.message})`;
        }
      }
      if (mine !== epoch) {
        // the project was replaced meanwhile
        m.gpu?.destroy();
        m.gpu = null;
        m.session = null;
        return;
      }
      m.backend = backend;
      showBackend?.(backend);
      await engine("simSeek", 0);
      refreshHidden();
      renderLegend();
      await buildScene();
      $("simScrub").max = String(result.segments);
      $("simScrub").value = "0";
      $("simControls").classList.remove("hidden");
      m.open = true;
      document.body.dataset.machining = "open";
      if (keep) await seek(Math.min(keep, result.segments));
      setStatus(`Machining simulation: ${result.segments.toLocaleString("en")} moves, ${result.ops.length} operations`, "done");
    } catch (e) {
      if (e.message === "cancelled") return setStatus("Machining simulation cancelled", "done");
      showError(e.message);
      setStatus("Machining simulation failed", "error");
    }
  }

  function close() {
    m.playing = false;
    m.open = false;
    m.gpu?.destroy();
    m.gpu = null;
    $("simControls").classList.add("hidden");
    $("simPlay").textContent = "Play";
    document.body.dataset.machining = "";
    showMachining(false);
  }

  /** A new project: close the playback and forget its session, GPU stock and meshes. */
  function reset() {
    epoch++;
    close();
    Object.assign(m, { session: null, r: null, op: null, pos: 0, target: 0, busy: false, stockMesh: null, ghost: null, dgrid: null, dr: null, dop: null, toolNumber: null, colors: {} });
    $("opLegend").innerHTML = "";
    const v = state.three;
    if (v && !v.failed && v.groups.machining) {
      v.scene.remove(v.groups.machining);
      disposeTree(v.groups.machining);
      delete v.groups.machining;
      delete v.groups.work;
      delete v.groups.toolModel;
    }
  }

  /** Rebuild the display at the stride for the current state (playing or at rest). */
  async function restride() {
    // (without stock there is no grid to show coarser while playing)
    if (!m.session || !m.stock) return;
    const want = strideFor(m.playing ? PLAY_MAX : DISPLAY_MAX);
    if (want !== m.stride) {
      m.stride = want;
      await buildScene();
    }
  }

  function frame(t) {
    if (!m.playing) {
      restride();
      return;
    }
    const dt = Math.min(0.1, (t - (m.last || t)) / 1000);
    m.last = t;
    let next;
    if (m.session.times?.length) {
      // machine time runs `speed` times faster than the clock
      if (m.clock == null) m.clock = timeAt(m.pos);
      m.clock += dt * speedFactor();
      next = Math.max(m.pos + 1, segmentsBy(m.clock));
    } else {
      next = Math.round(m.pos + Math.max(1, 300 * speedFactor() * dt));
    }
    next = Math.min(m.session.segments, next);
    if (!m.busy) seek(next);
    if (m.pos >= m.session.segments) {
      m.playing = false;
      $("simPlay").textContent = "Play";
      document.body.dataset.playing = "";
      restride();
      return;
    }
    requestAnimationFrame(frame);
  }

  $("simOpen").addEventListener("click", open);
  $("simClose").addEventListener("click", close);
  $("simPlay").addEventListener("click", () => {
    if (!m.session) return;
    m.playing = !m.playing;
    $("simPlay").textContent = m.playing ? "Pause" : "Play";
    document.body.dataset.playing = m.playing ? "yes" : "";
    if (m.playing) {
      if (m.pos >= m.session.segments) seek(0);
      m.last = 0;
      restride().then(() => requestAnimationFrame(frame));
    }
  });
  $("simStep").addEventListener("click", () => {
    m.clock = null;
    if (m.session) seek(Math.min(m.session.segments, m.pos + 1));
  });
  $("simScrub").addEventListener("input", () => {
    m.clock = null;
    seek(+$("simScrub").value);
  });
  $("simEnd").addEventListener("click", () => {
    m.clock = null;
    if (m.session) seek(m.session.segments);
  });
  $("simSpeed").addEventListener("input", () => {
    showSpeed();
    remember("marcam.playSpeed", $("simSpeed").value);
  });
  // stock on or off: the playback again, at the same place
  $("simStock").addEventListener("change", async () => {
    remember("marcam.playStock", $("simStock").checked ? "1" : "0");
    if (!m.open) return;
    const playing = m.playing;
    m.playing = false;
    await open();
    if (playing && m.session) $("simPlay").click();
  });
  $("simGhost").addEventListener("change", () => m.ghost && (m.ghost.visible = $("simGhost").checked));
  $("opLegend").addEventListener("input", (ev) => {
    const inp = ev.target.closest("input[type=color]");
    if (!inp) return;
    const tag = +inp.dataset.tag;
    m.colors[tag] = hexToRgb(inp.value);
    const o = m.session.ops.find((x) => x.tag === tag);
    const saved = savedColors();
    saved[opKey(o)] = inp.value;
    try {
      localStorage.setItem("marcam.opColors", JSON.stringify(saved));
    } catch {}
    // recolour every vertex from the tag mirror
    if (m.stockMesh && m.xyz) {
      const col = m.stockMesh.geometry.attributes.color.array;
      for (let k = 0; k < m.dop.length; k++) col.set(colorOf(m.dop[k]), 3 * k);
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
    } else if (m.stockMesh) {
      const { na, nx } = m.dgrid;
      const col = m.stockMesh.geometry.attributes.color.array;
      const paint = (vi, k) => col.set(colorOf(m.dop[k]), 3 * vi);
      for (let k = 0; k < nx * na; k++) paint(k, k);
      for (let j = 0; j < na; j++) {
        paint(nx * na + j, j);
        paint(nx * na + na + 1 + j, (nx - 1) * na + j);
      }
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
    }
  });

  /** The engine's worker was ended (a cancel): its playback is gone, so this one closes. */
  function lost() {
    if (m.session || m.open) close();
    m.session = null;
  }

  /** The colours changed (View > Colours): the open playback drawn again. */
  async function recolor() {
    if (!m.open || !m.session) return;
    m.toolNumber = null;
    await buildScene();
  }

  return { open, close, seek, reset, applyHidden, colorOf, lost, recolor, state: m };
}
