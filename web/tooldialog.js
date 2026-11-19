// Tools for a project, from the libraries: a browser (search, type, size) that adds the tool
// picked to the project without a T number (numbered later, in "Tool numbers"), or a new one
// typed in; and the tool numbers dialog that gives the project's tools their T numbers.

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const KINDS = ["flat", "ball", "vbit", "tapered_ball", "bull"];

/** The library tools on offer: the built-in libraries and the person's, flattened. */
function libraryTools(state, userLibraries) {
  const out = [];
  for (const lib of [...(state.registry?.libraries || []), ...(userLibraries() || [])]) {
    for (const t of lib.tools || []) {
      out.push({ library: lib.id, libraryName: lib.name || lib.id, id: t.id, name: t.name || t.id, kind: t.shape?.kind, diameter: t.shape?.diameter ?? t.shape?.tip_diameter, flute: t.flute_length });
    }
  }
  return out;
}

export function setupToolDialogs({ $, state, readJob, writeJob, userLibraries }) {
  // ------------------------------------------------------------ the library browser
  const dlg = document.createElement("dialog");
  dlg.id = "toolPick";
  dlg.className = "tool-dialog";
  dlg.innerHTML = `<form method="dialog" id="toolPickForm">
    <h3>Add a tool to the project</h3>
    <div class="row small"><input id="tpSearch" placeholder="Search (name, size)" aria-label="Search tools">
      <select id="tpKind" aria-label="Tool type"><option value="">every type</option>${KINDS.map((k) => `<option>${k}</option>`).join("")}</select></div>
    <div id="tpList" class="tp-list" role="listbox" aria-label="Library tools"></div>
    <details id="tpNew"><summary>New tool...</summary>
      <div class="row small"><label>Type <select id="tpNewKind">${KINDS.slice(0, 3).map((k) => `<option>${k}</option>`).join("")}</select></label>
        <label>Diameter (mm) <input id="tpNewDia" type="number" step="0.001" value="3.175" size="6"></label>
        <label>Flute (mm) <input id="tpNewFlute" type="number" step="0.1" value="12" size="5"></label>
        <label>Angle (deg, V) <input id="tpNewAngle" type="number" step="1" value="90" size="4"></label></div>
      <div class="row small"><input id="tpNewName" placeholder="Description" aria-label="New tool description"> <button type="button" id="tpNewAdd">Add this tool</button></div>
    </details>
    <div class="muted small">Added without a tool number: give it one in Tools > Tool numbers (until then the G-code uses the next free number and says so).</div>
    <div class="row"><button value="cancel" type="submit">Cancel</button></div>
  </form>`;
  document.body.appendChild(dlg);
  let resolveWith = null;

  function renderList() {
    const q = $("tpSearch").value.trim().toLowerCase();
    const kind = $("tpKind").value;
    const tools = libraryTools(state, userLibraries).filter((t) => (!kind || t.kind === kind) && (!q || `${t.name} ${t.diameter} ${t.kind} ${t.libraryName}`.toLowerCase().includes(q)));
    $("tpList").innerHTML = tools.length
      ? tools.map((t, i) => `<button type="button" class="tp-row" data-i="${i}" role="option"><strong>${esc(t.name)}</strong> <span class="muted">${esc(t.kind || "")} ${t.diameter ?? ""} mm · ${esc(t.libraryName)}</span></button>`).join("")
      : `<div class="muted small">No tool matches.</div>`;
    $("tpList").dataset.tools = JSON.stringify(tools);
  }
  $("tpSearch").addEventListener("input", renderList);
  $("tpKind").addEventListener("change", renderList);

  /** Add `entry` (without its number) to the project, unindexed; resolves to its number. */
  function addToProject(entry) {
    const doc = readJob();
    doc.tools = doc.tools || [];
    const number = Math.max(0, ...doc.tools.map((t) => t.number)) + 1;
    doc.tools.push({ number, ...entry });
    doc.unindexed = [...new Set([...(doc.unindexed || []), number])];
    writeJob(doc);
    return number;
  }
  $("tpList").addEventListener("click", (ev) => {
    const row = ev.target.closest(".tp-row");
    if (!row) return;
    const t = JSON.parse($("tpList").dataset.tools)[Number(row.dataset.i)];
    const n = addToProject({ library: t.library, tool: t.id, description: t.name });
    dlg.close();
    resolveWith?.(n);
    resolveWith = null;
  });
  $("tpNewAdd").addEventListener("click", () => {
    const kind = $("tpNewKind").value;
    const d = Number($("tpNewDia").value) || 3.175;
    const t = { description: $("tpNewName").value.trim() || `${d} mm ${kind}`, kind, diameter: d, flute_length: Number($("tpNewFlute").value) || 12 };
    if (kind === "vbit") t.angle = Number($("tpNewAngle").value) || 90;
    const n = addToProject(t);
    dlg.close();
    resolveWith?.(n);
    resolveWith = null;
  });
  dlg.addEventListener("close", () => {
    resolveWith?.(null);
    resolveWith = null;
  });

  /** Browse the libraries; resolves to the new tool's number (null: cancelled). */
  function pick(kinds) {
    $("tpSearch").value = "";
    // the operation's tool types first
    $("tpKind").value = kinds?.length === 1 ? kinds[0] : "";
    renderList();
    dlg.showModal();
    return new Promise((r) => (resolveWith = r));
  }

  // ------------------------------------------------------------ tool numbers
  const num = document.createElement("dialog");
  num.id = "toolNumbers";
  num.className = "tool-dialog";
  document.body.appendChild(num);

  function openNumbers() {
    const doc = readJob();
    const un = new Set(doc.unindexed || []);
    const label = (t) => t.description || (t.library ? `${t.library}/${t.tool}` : `${t.kind} ${t.diameter}`);
    num.innerHTML = `<form method="dialog" id="toolNumbersForm"><h3>Tool numbers</h3>
      <div class="muted small">The T number each tool has in the machine's tool changer. A tool without one is written as the next free number, with a warning.</div>
      <table class="tn-table"><thead><tr><th>Tool</th><th>T number</th><th>Has one</th></tr></thead><tbody>
      ${(doc.tools || []).map((t, i) => `<tr><td>${esc(label(t))}</td><td><input type="number" min="1" step="1" data-i="${i}" class="tn-num" value="${t.number}" size="4"></td><td><input type="checkbox" data-i="${i}" class="tn-has" ${un.has(t.number) ? "" : "checked"}></td></tr>`).join("")}
      </tbody></table>
      <div id="tnError" class="error small hidden"></div>
      <div class="row"><button type="button" id="tnApply" class="primary">Apply</button> <button value="cancel" type="submit">Cancel</button></div></form>`;
    num.showModal();
    num.querySelector("#tnApply").addEventListener("click", () => {
      const nums = [...num.querySelectorAll(".tn-num")].map((e) => Math.round(Number(e.value)));
      const has = [...num.querySelectorAll(".tn-has")].map((e) => e.checked);
      if (nums.some((n) => !(n >= 1)) || new Set(nums).size !== nums.length) {
        const e = num.querySelector("#tnError");
        e.textContent = "Each tool needs its own number, 1 or more.";
        e.classList.remove("hidden");
        return;
      }
      // renumber: the operations follow their tool
      const map = new Map(doc.tools.map((t, i) => [t.number, nums[i]]));
      doc.tools.forEach((t, i) => (t.number = nums[i]));
      for (const s of doc.setups || []) for (const o of s.operations || []) if (map.has(o.tool)) o.tool = map.get(o.tool);
      doc.unindexed = doc.tools.filter((t, i) => !has[i]).map((t) => t.number);
      if (!doc.unindexed.length) delete doc.unindexed;
      writeJob(doc);
      num.close();
      window.__marcam.tree?.refresh();
    });
  }

  return { pick, openNumbers };
}
