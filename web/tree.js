// Project tree (document model 0.2): setups -> outputs (G-code files) -> operations.
// Operations can be added (forms built from the engine's operation registry), edited, turned
// on/off, coloured, reordered, deleted, and limited to faces picked on the 3D model. The
// project JSON in the job editor is the single source of truth: every edit rewrites it.

const PALETTE = ["#3b82f6", "#f59e0b", "#10b981", "#ef4444", "#a855f7", "#14b8a6", "#e11d48", "#84cc16"];
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export function setupTree({ engine, $, state, readJob, writeJob, showError, onChange, getThree, machineArg }) {
  const t = { doc: null, problems: [], types: [], setup: null, selected: null, picking: false, regions: null };
  state.tree = t;

  async function loadTypes() {
    if (!t.types.length) t.types = (await engine("opTypes")).result;
  }

  /** Re-read the editor: 0.1 jobs are shown upgraded (the editor switches to 0.2 on the first edit). */
  async function refresh() {
    await loadTypes();
    let raw;
    try {
      raw = readJob();
    } catch {
      $("tree").innerHTML = '<div class="muted small">(fix the JSON to see the project)</div>';
      return;
    }
    try {
      const { result } = await engine("checkProject", raw);
      t.doc = result.project;
      t.problems = result.problems;
      t.upgraded = result.upgraded;
    } catch (e) {
      $("tree").innerHTML = `<div class="error">${esc(e.message)}</div>`;
      return;
    }
    if (!t.doc.setups.some((s) => s.id === t.setup)) t.setup = t.doc.setups[0]?.id ?? null;
    render();
  }

  /** Write the project back to the editor and refresh everything that depends on it. */
  async function commit() {
    writeJob(t.doc);
    await refresh();
    onChange?.();
  }

  const setup = () => t.doc.setups.find((s) => s.id === t.setup);
  const opById = (id) => (t.draft && t.draft.op.id === id ? t.draft.op : setup().operations.find((o) => o.id === id));
  const typeOf = (kind) => t.types.find((x) => x.kind === kind);
  const colorOf = (op, i) => op.color || PALETTE[i % PALETTE.length];
  // a vise setup (box stock) takes the 3-axis types; a rotary setup takes everything, the
  // 3-axis types indexed to a fixed A (3+1)
  const isBox = (s) => s?.stock?.shape === "box";
  const fits = (ty, s) => !isBox(s) || ty.machine_needs.includes("xyz");
  const pointsText = (v) => v.map((q) => q.join(" ")).join("; ");

  function render() {
    const box = $("tree");
    if (!t.doc) return;
    const s = setup();
    const setupSel = t.doc.setups
      .map((x) => `<option value="${esc(x.id)}" ${x.id === t.setup ? "selected" : ""}>${esc(x.name || x.id)}</option>`)
      .join("");
    let html = `<div class="row small"><label for="treeSetup">Setup</label><select id="treeSetup">${setupSel}</select>
      <button type="button" id="treeWizard" title="Add turning, knurl and chamfer from the model's faces">Knurl wizard</button></div>`;
    if (t.upgraded) html += `<div class="muted small">0.1 job shown as a 0.2 project; it is saved as 0.2 on the first edit.</div>`;
    let n = 0;
    for (const out of s.outputs) {
      html += `<div class="tree-out" data-out="${esc(out.name)}"><div class="tree-outname">${esc(out.name)}.nc <span class="muted">${esc(out.title)}</span></div>`;
      out.operations.forEach((id, k) => {
        const op = opById(id);
        if (!op) return;
        const i = s.operations.indexOf(op);
        const geo = op.geometry?.length ? ` · ${op.geometry.map((g) => (g.kind === "faces" ? `${g.ids.length} face${g.ids.length > 1 ? "s" : ""}` : g.kind.replace("_", " "))).join(", ")}` : "";
        html += `<div class="tree-op ${t.selected === id ? "sel" : ""} ${op.enabled === false ? "off" : ""}" data-op="${esc(id)}">
          <input type="checkbox" data-act="enable" ${op.enabled === false ? "" : "checked"} aria-label="Enable ${esc(id)}">
          <input type="color" data-act="color" value="${colorOf(op, i)}" aria-label="Colour of ${esc(id)}">
          <span class="tree-label" data-act="select">${esc(op.name || typeOf(op.type)?.title || op.type)} <span class="muted">${esc(id)} · T${op.tool}${geo}</span></span>
          <button type="button" data-act="up" ${k === 0 ? "disabled" : ""} aria-label="Move ${esc(id)} up">▲</button>
          <button type="button" data-act="down" ${k === out.operations.length - 1 ? "disabled" : ""} aria-label="Move ${esc(id)} down">▼</button>
          <button type="button" data-act="delete" aria-label="Delete ${esc(id)}">✕</button></div>`;
        n++;
      });
      html += `</div>`;
    }
    const opts = t.types.filter((x) => fits(x, s)).map((x) => `<option value="${x.kind}">${esc(x.title)}</option>`).join("");
    const outs = s.outputs.map((o) => `<option value="${esc(o.name)}">${esc(o.name)}</option>`).join("");
    html += `<div class="row small"><select id="treeAddType" aria-label="Operation type">${opts}</select>
      <select id="treeAddOut" aria-label="Output file">${outs}<option value="__new">new file</option></select>
      <button type="button" id="treeAdd">Add operation</button></div>`;
    if (t.problems.length) html += `<div class="error small">${t.problems.map(esc).join("<br>")}</div>`;
    if (t.draft) html += `<div class="muted small">New operation ${esc(t.draft.op.id)} (not saved until Apply)</div>`;
    html += `<div id="opEditor"></div>`;
    box.innerHTML = html;
    box.dataset.ops = String(n);
    renderEditor();
  }

  /** Form for the selected operation, built from the registry's parameter specs. */
  function renderEditor() {
    const ed = $("opEditor");
    const op = t.selected && opById(t.selected);
    if (!op) {
      ed.innerHTML = "";
      return;
    }
    const ty = typeOf(op.type);
    const tools = (t.doc.tools || []).map((x) => x.number);
    let html = `<form class="opform" id="opForm"><div class="tree-outname">${esc(ty?.title || op.type)} <span class="muted">${esc(op.id)}</span></div>
      <label>Name <input name="__name" value="${esc(op.name || "")}"></label>`;
    for (const p of ty.params) {
      const v = op[p.name];
      // rotary_finish uses one stepover or the other, depending on the pattern
      if (p.name === "stepover_deg" && op.pattern === "around") continue;
      if (p.name === "stepover_mm" && op.pattern === "along" && op.stepover_deg != null) continue;
      // a vise has no rotary axis to index
      if (p.name === "index_a" && isBox(setup())) continue;
      const label = `${p.name.replace(/_/g, " ")}${p.unit ? ` (${p.unit})` : ""}`;
      if (p.name === "tool") {
        html += `<label>${label}<select name="tool">${tools.map((n) => `<option ${n === v ? "selected" : ""}>${n}</option>`).join("")}</select></label>`;
      } else if (p.kind === "choice") {
        html += `<label>${label}<select name="${p.name}">${p.choices.map((c) => `<option ${c === v ? "selected" : ""}>${c}</option>`).join("")}</select></label>`;
      } else if (p.kind === "boolean") {
        html += `<label>${label}<input type="checkbox" name="${p.name}" ${(v ?? p.default) ? "checked" : ""}></label>`;
      } else if (p.kind === "points") {
        html += `<label title="${esc(p.help)}">${label} <span class="muted">x y; x y; ...</span><input name="${p.name}" value="${esc(Array.isArray(v) ? pointsText(v) : "")}" placeholder="required"></label>`;
      } else {
        const text = v == null ? "" : Array.isArray(v) ? v.join(", ") : v;
        html += `<label title="${esc(p.help)}">${label}<input name="${p.name}" value="${esc(text)}" placeholder="${p.default != null ? esc(Array.isArray(p.default) ? p.default.join(", ") : p.default) : p.required ? "required" : ""}"></label>`;
      }
    }
    const faces = op.geometry?.find((g) => g.kind === "faces");
    const ax = op.geometry?.find((g) => g.kind === "axial_range");
    // face and axial selections bound rotary operations (X/A); 3-axis types take their area as parameters
    if (!ty.machine_needs.includes("xyz") && ty.geometry.some((g) => g === "mesh" || g === "profile" || g === "axial_range")) {
      html += `<div class="row small"><button type="button" id="pickFaces">${t.picking ? "Done picking" : "Pick faces on the model"}</button>
        <span id="pickInfo">${faces ? `${faces.ids.length} face(s): ${faces.ids.map(esc).join(", ")}` : "whole model"}</span>
        ${faces ? '<button type="button" id="clearFaces">Clear</button>' : ""}</div>
        <label>axial range (mm)<input name="__axial" value="${ax ? ax.x.join(", ") : ""}" placeholder="all"></label>`;
    }
    html += `<div id="opErrors" class="error small hidden"></div><div class="row"><button type="submit" class="primary">Apply</button>${t.draft ? ' <button type="button" id="cancelDraft">Cancel</button>' : ""}</div></form>`;
    ed.innerHTML = html;
  }

  function readForm() {
    const op = opById(t.selected);
    const ty = typeOf(op.type);
    const f = $("opForm");
    const out = { ...op };
    out.name = f.elements.__name.value.trim();
    if (!out.name) delete out.name;
    for (const p of ty.params) {
      const el = f.elements[p.name];
      if (!el) continue;
      if (p.kind === "boolean") out[p.name] = el.checked;
      else if (p.name === "tool" || p.kind === "integer") out[p.name] = el.value === "" ? undefined : parseInt(el.value, 10);
      else if (p.kind === "choice") out[p.name] = el.value;
      else if (p.kind === "points") {
        const pts = el.value.split(";").map((q) => q.split(/[,\s]+/).filter(Boolean).map(Number)).filter((q) => q.length);
        out[p.name] = pts.length ? pts : undefined;
      } else if (["vec2", "rect", "numbers", "integers"].includes(p.kind)) {
        const nums = el.value.split(/[,\s]+/).filter(Boolean).map(Number);
        out[p.name] = nums.length ? nums : undefined;
      } else out[p.name] = el.value === "" ? undefined : Number(el.value);
      if (out[p.name] === undefined) delete out[p.name];
    }
    const axial = f.elements.__axial;
    if (axial) {
      out.geometry = (out.geometry || []).filter((g) => g.kind !== "axial_range");
      const v = axial.value.split(/[,\s]+/).filter(Boolean).map(Number);
      if (v.length === 2) out.geometry.push({ kind: "axial_range", x: v });
      if (!out.geometry.length) delete out.geometry;
    }
    return out;
  }

  // ------------------------------------------------------------ face picking
  async function startPicking() {
    const v = await getThree();
    if (!v || v.failed) return showError("Face picking needs the 3D view");
    const s = setup();
    const model = s.part.find((id) => t.doc.models.find((m) => m.id === id)?.kind === "mesh") || s.part[0];
    const { result } = await engine("faceRegions", t.doc, model, state.mesh);
    const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    t.regions = { model, list: result.regions, tri: new Float32Array(bytes(result.triangles).buffer), of: new Uint16Array(bytes(result.region_of).buffer) };
    const { THREE } = v;
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(t.regions.tri, 3));
    g.setAttribute("color", new THREE.BufferAttribute(new Float32Array(t.regions.tri.length).fill(0.7), 3));
    g.computeVertexNormals();
    t.pickMesh = new THREE.Mesh(g, new THREE.MeshStandardMaterial({ vertexColors: true, side: THREE.DoubleSide }));
    v.scene.add(t.pickMesh);
    v.groups.part.visible = false;
    t.picking = true;
    paintPicked();
    renderEditor();
  }

  function stopPicking() {
    const v = state.three;
    if (t.pickMesh) {
      v.scene.remove(t.pickMesh);
      t.pickMesh.geometry.dispose();
      t.pickMesh = null;
    }
    if (v && !v.failed) v.groups.part.visible = $("showPart").checked;
    t.picking = false;
    renderEditor();
  }

  function pickedIds() {
    const f = opById(t.selected)?.geometry?.find((g) => g.kind === "faces" && g.model === t.regions?.model);
    return f ? f.ids : [];
  }

  function paintPicked() {
    if (!t.pickMesh) return;
    const ids = new Set(pickedIds());
    const col = t.pickMesh.geometry.attributes.color.array;
    for (let k = 0; k < t.regions.of.length; k++) {
      const on = ids.has(t.regions.list[t.regions.of[k]].id);
      const c = on ? [0.15, 0.39, 0.92] : [0.72, 0.74, 0.78];
      for (let v = 0; v < 3; v++) col.set(c, 9 * k + 3 * v);
    }
    t.pickMesh.geometry.attributes.color.needsUpdate = true;
  }

  /** Toggle the face under a canvas point (CSS pixels relative to the canvas). */
  async function pickAt(px, py) {
    const v = state.three;
    if (!t.picking || !t.pickMesh) return null;
    const el = v.renderer.domElement;
    const ndc = new v.THREE.Vector2((px / el.clientWidth) * 2 - 1, -(py / el.clientHeight) * 2 + 1);
    const ray = new v.THREE.Raycaster();
    ray.setFromCamera(ndc, v.camera);
    const hit = ray.intersectObject(t.pickMesh)[0];
    if (!hit) return null;
    const region = t.regions.list[t.regions.of[hit.faceIndex]];
    const op = opById(t.selected);
    op.geometry = op.geometry || [];
    let f = op.geometry.find((g) => g.kind === "faces" && g.model === t.regions.model);
    if (!f) op.geometry.push((f = { kind: "faces", model: t.regions.model, ids: [] }));
    f.ids = f.ids.includes(region.id) ? f.ids.filter((x) => x !== region.id) : [...f.ids, region.id];
    if (!f.ids.length) op.geometry = op.geometry.filter((g) => g !== f);
    if (!op.geometry.length) delete op.geometry;
    if (!t.draft) writeJob(t.doc);
    paintPicked();
    if (t.draft) await refreshDefaults();
    else renderEditor();
    $("pickInfo").textContent = f.ids.length ? `${f.ids.length} face(s): ${f.ids.join(", ")}` : "whole model";
    return region;
  }

  // ------------------------------------------------------------ events
  $("tree").addEventListener("click", async (ev) => {
    const el = ev.target;
    if (el.id === "treeAdd") {
      const kind = $("treeAddType").value;
      const ty = typeOf(kind);
      const s = setup();
      let id;
      for (let k = s.operations.length + 1; ; k++) if (!s.operations.some((o) => o.id === `op${k}`)) { id = `op${k}`; break; }
      // a draft with every field filled by the engine's defaults (from the setup's tools,
      // spindle, machine limits and part faces); it joins the project on Apply
      const out = $("treeAddOut").value;
      t.selected = id;
      t.edited = new Set();
      try {
        const { result } = await engine("opDefaults", t.doc, { type: kind, setup: s.id, machine: machineArg?.() ?? null }, state.mesh);
        t.draft = { op: { id, ...result }, out, title: ty.title };
        render();
      } catch (e) {
        t.draft = null;
        t.selected = null;
        render();
        showError(`Cannot add ${ty.title}: ${e.message}`);
      }
      return;
    }
    if (el.id === "cancelDraft") {
      t.draft = null;
      t.selected = null;
      render();
      return;
    }
    if (el.id === "treeWizard") {
      try {
        const s = setup();
        const doc = structuredClone(t.doc);
        const used = new Set(doc.setups.flatMap((x) => x.outputs.map((o) => o.name)));
        const params = { setup: s.id };
        if (used.has("knob_1_profile") || used.has("knob_2_knurl")) Object.assign(params, { turn_output: `${s.id}_turn`, knurl_output: `${s.id}_knurl` });
        const { result } = await engine("wizardKnurl", doc, params, state.mesh);
        t.doc = result.project;
        await commit();
      } catch (e) {
        showError(e.message);
      }
      return;
    }
    if (el.id === "pickFaces") return t.picking ? stopPicking() : startPicking();
    if (el.id === "clearFaces") {
      const op = opById(t.selected);
      op.geometry = (op.geometry || []).filter((g) => g.kind !== "faces");
      if (!op.geometry.length) delete op.geometry;
      paintPicked();
      return commit();
    }
    const row = el.closest(".tree-op");
    if (!row) return;
    const id = row.dataset.op;
    const s = setup();
    const out = s.outputs.find((o) => o.operations.includes(id));
    const act = el.dataset.act;
    if (act === "select") {
      t.selected = t.selected === id ? null : id;
      if (t.picking) stopPicking();
      render();
    } else if (act === "up" || act === "down") {
      const k = out.operations.indexOf(id);
      const j = act === "up" ? k - 1 : k + 1;
      [out.operations[k], out.operations[j]] = [out.operations[j], out.operations[k]];
      await commit();
    } else if (act === "delete") {
      s.operations = s.operations.filter((o) => o.id !== id);
      for (const o of s.outputs) o.operations = o.operations.filter((x) => x !== id);
      s.outputs = s.outputs.filter((o) => o.operations.length);
      if (t.selected === id) t.selected = null;
      await commit();
    }
  });
  // fields the user typed into keep their values when defaults are refreshed
  $("tree").addEventListener("input", (ev) => {
    const f = ev.target.closest("#opForm");
    if (f && ev.target.name && t.edited) t.edited.add(ev.target.name);
  });

  /** Re-derive the draft's defaults (tool or geometry changed), keeping edited fields. */
  async function refreshDefaults() {
    if (!t.draft) return;
    const cur = readForm();
    try {
      const { result } = await engine(
        "opDefaults",
        t.doc,
        { type: cur.type, setup: t.setup, tool: cur.tool, geometry: cur.geometry || [], machine: machineArg?.() ?? null },
        state.mesh,
      );
      const next = { ...cur };
      for (const [k, v] of Object.entries(result)) if (!t.edited.has(k) && k !== "tool") next[k] = v;
      t.draft.op = next;
      renderEditor();
    } catch (e) {
      const box = $("opErrors");
      if (box) {
        box.textContent = e.message;
        box.classList.remove("hidden");
      }
    }
  }
  state.tree.refreshDefaults = refreshDefaults;

  $("tree").addEventListener("change", async (ev) => {
    const el = ev.target;
    if (el.closest("#opForm") && el.name === "tool" && t.draft) {
      t.edited.delete("tool");
      return refreshDefaults();
    }
    if (el.closest("#opForm") && el.name === "__axial" && t.draft) return refreshDefaults();
    if (el.id === "treeSetup") {
      t.setup = el.value;
      t.selected = null;
      return render();
    }
    const row = el.closest(".tree-op");
    if (!row) return;
    const op = opById(row.dataset.op);
    if (el.dataset.act === "enable") op.enabled = el.checked;
    if (el.dataset.act === "color") op.color = el.value;
    await commit();
  });
  $("tree").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const op = readForm();
    const doc = structuredClone(t.doc);
    const s = doc.setups.find((x) => x.id === t.setup);
    if (t.draft) {
      s.operations.push(op);
      let outName = t.draft.out;
      if (outName === "__new") {
        outName = `${s.id}_${op.id}`;
        s.outputs.push({ name: outName, title: t.draft.title, operations: [] });
      }
      s.outputs.find((o) => o.name === outName).operations.push(op.id);
    } else {
      s.operations[s.operations.findIndex((o) => o.id === op.id)] = op;
    }
    // the engine parses and checks the result before it replaces the project
    try {
      const { result } = await engine("checkProject", doc);
      if (result.problems.length) throw new Error(result.problems.join("; "));
      t.doc = result.project;
      t.draft = null;
      await commit();
    } catch (e) {
      const box = $("opErrors");
      box.textContent = e.message.replace(/^project 0\.2: /, "");
      box.classList.remove("hidden");
    }
  });
  // clicks on the 3D view pick faces while picking
  const host = $("view3d");
  host.addEventListener("click", (ev) => {
    if (!t.picking) return;
    const r = state.three.renderer.domElement.getBoundingClientRect();
    pickAt(ev.clientX - r.left, ev.clientY - r.top);
  });

  return { refresh, pickAt, state: t };
}
