// Solid machining simulation: the stock as a closed solid that the tool cuts as the job plays.
// The engine (WebAssembly, in the worker) keeps the stock heightmap and the cut-by-operation
// tags; this module plays it back, updates the GPU buffers in place for the rows that changed,
// and colours each cut surface by the operation that cut it. Rotary setups play on a bar
// (rows = X cells, A turns the work); 3-axis (vise) setups on a block (rows = Y cells, the
// tool moves in XYZ).

import { blockSolid, heightmapSolid } from "./meshes.js";

const PALETTE = ["#3b82f6", "#f59e0b", "#10b981", "#ef4444", "#a855f7", "#14b8a6", "#e11d48", "#84cc16"];
const UNCUT = [0.62, 0.64, 0.68];

const b64ToBytes = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const f32 = (s) => new Float32Array(b64ToBytes(s).buffer);
const u16 = (s) => new Uint16Array(b64ToBytes(s).buffer);
const hexToRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);

export function setupMachining({ engine, getThree, $, state, readJob, setStatus, showError, libraries }) {
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
  };
  state.machining = m;

  const opKey = (o) => `${o.program}#${o.id}#${o.kind}`;
  const savedColors = () => {
    try {
      return JSON.parse(localStorage.getItem("4cam.opColors") || "{}");
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
    m.session.ops.forEach((o, i) => {
      if (proj) o.project_id = proj.ids[i];
      const hex = (proj && proj.colors[i]) || saved[opKey(o)] || PALETTE[i % PALETTE.length];
      m.colors[o.tag] = hexToRgb(hex);
      const row = document.createElement("label");
      row.className = "oprow";
      row.innerHTML = `<input type="color" value="${hex}" data-tag="${o.tag}" aria-label="Colour for ${o.program} ${o.kind}"> <span>${o.program}: ${o.kind.replace(/_/g, " ")}${o.project_id ? ` (${o.project_id})` : ""}</span>`;
      box.appendChild(row);
    });
    const un = document.createElement("div");
    un.className = "oprow small muted";
    un.innerHTML = `<span class="swatch" style="background: rgb(${UNCUT.map((c) => Math.round(c * 255)).join(",")})"></span> uncut stock`;
    box.appendChild(un);
  }

  const colorOf = (tag) => (tag ? m.colors[tag] || [1, 0, 1] : UNCUT);

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
      c.geometry?.dispose();
      c.material?.dispose();
    }
    const s = m.session;
    const mesh = m.xyz ? blockSolid(s.grid, m.r, s.stock_box.min[2], (k) => colorOf(m.op[k])) : heightmapSolid(s.grid, m.r, (k) => colorOf(m.op[k]));
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(mesh.positions, 3));
    g.setAttribute("color", new THREE.BufferAttribute(mesh.colors, 3));
    g.setIndex(new THREE.BufferAttribute(mesh.indices, 1));
    g.computeVertexNormals();
    m.stockMesh = new THREE.Mesh(g, new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.55, metalness: 0.15 }));
    work.add(m.stockMesh);
    // ghost of the original bar or block: shows how much has been removed
    const ghostMat = new THREE.MeshStandardMaterial({ color: 0x9ca3af, transparent: true, opacity: 0.12, depthWrite: false });
    let ghost;
    if (m.xyz) {
      const { min, max } = s.stock_box;
      ghost = new THREE.Mesh(new THREE.BoxGeometry(max[0] - min[0], max[1] - min[1], max[2] - min[2]), ghostMat);
      ghost.position.set((min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2);
    } else {
      const { x0, dx, nx } = s.grid;
      ghost = new THREE.Mesh(new THREE.CylinderGeometry(s.stock_radius, s.stock_radius, nx * dx, 64, 1, false).rotateZ(-Math.PI / 2).translate(x0 + (nx * dx) / 2, 0, 0), ghostMat);
    }
    ghost.visible = $("simGhost").checked;
    m.ghost = ghost;
    work.add(ghost);
    m.toolNumber = null;
    showMachining(true);
    placeTool();
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
    const mat = new THREE.MeshStandardMaterial({ color: 0xe5b94e, metalness: 0.6, roughness: 0.35 });
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
    const [x, r, a, , toolNo] = [p[6 * k], p[6 * k + 1], p[6 * k + 2], p[6 * k + 3], p[6 * k + 4]];
    if (toolNo !== m.toolNumber) {
      const tm = v.groups.toolModel;
      for (const c of [...tm.children]) tm.remove(c);
      tm.add(toolModel(v.THREE, s.tools.find((t) => t.number === toolNo)));
      m.toolNumber = toolNo;
    }
    const opTag = p[6 * k + 5];
    const op = s.ops.find((o) => o.tag === opTag);
    const where = m.xyz ? `X${x.toFixed(1)} Y${r.toFixed(1)} Z${a.toFixed(2)}` : `A${a.toFixed(1)}`;
    if (m.xyz) {
      // 3-axis: the block stays put, the tool moves to the tip position (x, y, z)
      v.groups.work.rotation.x = 0;
      v.groups.toolModel.position.set(x, r, a);
    } else {
      // the part turns: rotate the work by A (right-handed about +X), tool fixed above the axis
      v.groups.work.rotation.x = (a * Math.PI) / 180;
      v.groups.toolModel.position.set(x, 0, r);
    }
    $("simLabel").textContent = `move ${m.pos.toLocaleString("en")} / ${s.segments.toLocaleString("en")}${op ? ` · ${op.program}: ${op.kind.replace(/_/g, " ")}` : ""} · ${where}`;
  }

  /** Apply a sim_seek result: rows changed -> update vertices and colours in place. */
  function applyRows(res) {
    m.pos = res.pos;
    if (res.rows && m.stockMesh && m.xyz) {
      // rows are Y indices of nx cells; vertex k is cell k (the walls share those vertices)
      const [j0, j1] = res.rows;
      const { nx } = m.session.grid;
      m.r.set(f32(res.z), j0 * nx);
      m.op.set(u16(res.op), j0 * nx);
      const pos = m.stockMesh.geometry.attributes.position.array;
      const col = m.stockMesh.geometry.attributes.color.array;
      for (let k = j0 * nx; k < (j1 + 1) * nx; k++) {
        pos[3 * k + 2] = m.r[k];
        col.set(colorOf(m.op[k]), 3 * k);
      }
      m.stockMesh.geometry.attributes.position.needsUpdate = true;
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
      m.stockMesh.geometry.computeVertexNormals();
    } else if (res.rows && m.stockMesh) {
      const [i0, i1] = res.rows;
      const { na, nx } = m.session.grid;
      m.r.set(f32(res.r), i0 * na);
      m.op.set(u16(res.op), i0 * na);
      const pos = m.stockMesh.geometry.attributes.position.array;
      const col = m.stockMesh.geometry.attributes.color.array;
      const setV = (vi, i, j) => {
        const k = i * na + j;
        const phi = ((j + 0.5) * 2 * Math.PI) / na;
        pos[3 * vi + 1] = m.r[k] * Math.cos(phi);
        pos[3 * vi + 2] = m.r[k] * Math.sin(phi);
        const c = colorOf(m.op[k]);
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
    m.target = n;
    if (m.busy) return;
    m.busy = true;
    try {
      while (m.target !== m.pos) {
        const want = m.target;
        const { result } = await engine("simSeek", want);
        applyRows(result);
        if (result.pos === want || m.target === want) break;
      }
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
    setStatus("Opening machining simulation...", "busy");
    try {
      const setup = job.schema_version === "0.2" ? state.tree?.setup : undefined;
      const { result } = await engine("simOpen", job, { dx: 0.1, na: 240, libraries: libraries(), setup }, state.mesh);
      result.path = f32(result.path);
      m.session = result;
      m.xyz = result.kind === "xyz";
      const n = m.xyz ? result.grid.nx * result.grid.ny : result.grid.nx * result.grid.na;
      m.r = new Float32Array(n).fill(m.xyz ? result.stock_box.max[2] : result.stock_radius);
      m.op = new Uint16Array(n);
      m.pos = 0;
      await engine("simSeek", 0);
      renderLegend();
      await buildScene();
      $("simScrub").max = String(result.segments);
      $("simScrub").value = "0";
      $("simControls").classList.remove("hidden");
      m.open = true;
      document.body.dataset.machining = "open";
      setStatus(`Machining simulation: ${result.segments.toLocaleString("en")} moves, ${result.ops.length} operations`, "done");
    } catch (e) {
      showError(e.message);
      setStatus("Machining simulation failed", "error");
    }
  }

  function close() {
    m.playing = false;
    m.open = false;
    $("simControls").classList.add("hidden");
    $("simPlay").textContent = "Play";
    document.body.dataset.machining = "";
    showMachining(false);
  }

  function frame(t) {
    if (!m.playing) return;
    const dt = Math.min(0.1, (t - (m.last || t)) / 1000);
    m.last = t;
    const speed = +$("simSpeed").value;
    const next = Math.min(m.session.segments, Math.round(m.pos + Math.max(1, speed * dt)));
    if (!m.busy) seek(next);
    if (m.pos >= m.session.segments) {
      m.playing = false;
      $("simPlay").textContent = "Play";
      document.body.dataset.playing = "";
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
      requestAnimationFrame(frame);
    }
  });
  $("simStep").addEventListener("click", () => m.session && seek(Math.min(m.session.segments, m.pos + 1)));
  $("simScrub").addEventListener("input", () => seek(+$("simScrub").value));
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
      localStorage.setItem("4cam.opColors", JSON.stringify(saved));
    } catch {}
    // recolour every vertex from the tag mirror
    if (m.stockMesh && m.xyz) {
      const col = m.stockMesh.geometry.attributes.color.array;
      for (let k = 0; k < m.op.length; k++) col.set(colorOf(m.op[k]), 3 * k);
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
    } else if (m.stockMesh) {
      const { na, nx } = m.session.grid;
      const col = m.stockMesh.geometry.attributes.color.array;
      const paint = (vi, k) => col.set(colorOf(m.op[k]), 3 * vi);
      for (let k = 0; k < nx * na; k++) paint(k, k);
      for (let j = 0; j < na; j++) {
        paint(nx * na + j, j);
        paint(nx * na + na + 1 + j, (nx - 1) * na + j);
      }
      m.stockMesh.geometry.attributes.color.needsUpdate = true;
    }
  });

  return { open, close, seek, state: m };
}
