// The setup dialog: one place to choose what a setup is. Used for New project, Import STL,
// Edit setup and Add setup. Name, machine, the setup type (three big choices), the part (an
// STL, its units and its axis or up direction, the origin, a tailstock) with a placement
// preview, and the stock (from the part with a margin, or given).

const DEFAULT_MARGIN = { rotary: 0.5, indexed: 0.5, vise: 2 };
const DEFAULT_ORIGIN = { rotary: 0, indexed: 0, vise: 0.5 };

export function setupDialog({ $, state, parseStl, placePart, KINDS }) {
  const dlg = $("setupDialog");
  let cur = null; // { mode, preset, parsed, resolve }

  const kind = () => dlg.querySelector('input[name="sdKind"]:checked')?.value ?? "rotary";
  const stockMode = () => dlg.querySelector('input[name="sdStock"]:checked')?.value ?? "auto";
  const num = (id) => ($(id).value === "" ? undefined : +$(id).value);
  const bar = (k) => k !== "vise";

  /** The part's triangles in mm, if there is a part (the chosen file, else the project's). */
  function tris() {
    const t = cur?.parsed?.tris ?? cur?.preset?.tris ?? null;
    if (!t) return null;
    return cur.parsed && $("sdUnits").value === "inch" ? t.map((v) => v * 25.4) : t;
  }

  function explicit() {
    if (stockMode() !== "explicit") return undefined;
    return bar(kind()) ? { dia: num("sdDia"), len: num("sdLen") } : { bx: num("sdBx"), by: num("sdBy"), bz: num("sdBz") };
  }

  function options() {
    return { kind: kind(), axis: $("sdAxis").value, up: $("sdAxis").value, margin: num("sdMargin"), explicit: explicit(), origin: num("sdOrigin") };
  }

  /** Labels and fields that depend on the setup type. */
  function layout(fromKindChange) {
    const k = kind();
    $("sdAxisLabel").textContent = bar(k) ? "Part axis along X" : "Up (towards the tool)";
    $("sdOriginLabel").textContent = bar(k) ? "Front at X0, then shift" : "Top below Z0 by";
    $("sdTailRow").classList.toggle("hidden", !bar(k));
    $("sdBar").classList.toggle("hidden", !bar(k));
    $("sdBox").classList.toggle("hidden", bar(k));
    if (fromKindChange) {
      // the type's own defaults, unless the user typed something
      if (!$("sdMargin").dataset.edited) $("sdMargin").value = DEFAULT_MARGIN[k];
      if (!$("sdOrigin").dataset.edited) $("sdOrigin").value = DEFAULT_ORIGIN[k];
      if (!$("sdAxis").dataset.edited) $("sdAxis").value = bar(k) ? "x" : "z";
    }
    preview();
  }

  /** The part and stock as placed, seen from the side (X across; up: radius or Z). */
  function preview() {
    const cv = $("sdPreview");
    const ctx = cv.getContext("2d");
    ctx.clearRect(0, 0, cv.width, cv.height);
    const t = tris();
    const k = kind();
    $("sdWarn").classList.add("hidden");
    if (!t) {
      $("sdInfo").textContent = cur?.mode === "new" ? "No part yet: choose an STL now, or import one after." : "";
      return;
    }
    let place;
    try {
      place = placePart(t, options());
    } catch (e) {
      $("sdInfo").textContent = e.message;
      return;
    }
    const map = (p) => {
      const a = place.axis;
      const q = a === "y" ? [p[1], p[2], p[0]] : a === "z" ? [p[2], p[0], p[1]] : p;
      return [q[0] + place.translate[0], q[1] + place.translate[1], q[2] + place.translate[2]];
    };
    const st = place.stock;
    // the view box: stock and part
    const box = bar(k) ? [-st.length, -st.radius, Math.max(0, place.translate[0] + 1), st.radius] : [st.min[0], st.min[2], st.max[0], st.max[2] + 1];
    const pad = 12;
    const sx = (cv.width - 2 * pad) / (box[2] - box[0]), sy = (cv.height - 2 * pad) / (box[3] - box[1]);
    const s = Math.min(sx, sy);
    const X = (x) => pad + (x - box[0]) * s, Y = (y) => cv.height - pad - (y - box[1]) * s;
    ctx.strokeStyle = "rgba(156, 163, 175, 0.9)";
    ctx.setLineDash([4, 3]);
    if (bar(k)) ctx.strokeRect(X(-st.length), Y(st.radius), st.length * s, 2 * st.radius * s);
    else ctx.strokeRect(X(st.min[0]), Y(st.max[2]), (st.max[0] - st.min[0]) * s, (st.max[2] - st.min[2]) * s);
    ctx.setLineDash([]);
    // the part's edges, side on (X across, Z up)
    ctx.strokeStyle = "rgba(147, 197, 253, 0.35)";
    ctx.beginPath();
    const step = Math.max(1, Math.floor(t.length / 9 / 6000));
    for (let i = 0; i < t.length; i += 9 * step) {
      const a = map([t[i], t[i + 1], t[i + 2]]), b = map([t[i + 3], t[i + 4], t[i + 5]]), c = map([t[i + 6], t[i + 7], t[i + 8]]);
      ctx.moveTo(X(a[0]), Y(a[2]));
      ctx.lineTo(X(b[0]), Y(b[2]));
      ctx.lineTo(X(c[0]), Y(c[2]));
      ctx.closePath();
    }
    ctx.stroke();
    // the origin
    ctx.strokeStyle = "#f59e0b";
    ctx.beginPath();
    if (bar(k)) {
      ctx.moveTo(X(box[0]), Y(0));
      ctx.lineTo(X(box[2]), Y(0));
      ctx.moveTo(X(0), Y(box[1]));
      ctx.lineTo(X(0), Y(box[3]));
    } else {
      ctx.moveTo(X(box[0]), Y(0));
      ctx.lineTo(X(box[2]), Y(0));
    }
    ctx.stroke();
    ctx.fillStyle = "#f59e0b";
    ctx.font = "11px system-ui";
    ctx.fillText(bar(k) ? "X0 (front), rotary axis" : "Z0 (top of the block)", X(bar(k) ? 0 : box[0]) + 4, Y(0) - 4);
    const size = place.size.map((v) => v.toFixed(2)).join(" x ");
    $("sdInfo").textContent = bar(k)
      ? `Part ${size} mm; bar R${st.radius} x ${st.length} mm, the part's front at X0${k === "indexed" ? "; cut from fixed A angles" : ""}.`
      : `Part ${size} mm; block ${+(st.max[0] - st.min[0]).toFixed(3)} x ${+(st.max[1] - st.min[1]).toFixed(3)} x ${+(st.max[2] - st.min[2]).toFixed(3)} mm, Z0 at its top.`;
    if (bar(k) && stockMode() === "explicit" && st.radius < Math.max(...[place.size[1], place.size[2]]) / 2) {
      $("sdWarn").textContent = "The bar is thinner than the part.";
      $("sdWarn").classList.remove("hidden");
    }
  }

  async function chooseFile(file) {
    if (!file) return;
    try {
      cur.parsed = { ...parseStl(new Uint8Array(await file.arrayBuffer())), file };
      $("sdFileName").textContent = `${file.name}: ${cur.parsed.ascii ? "ASCII" : "binary"} STL, ${cur.parsed.tris.length / 9} triangles`;
    } catch (e) {
      cur.parsed = null;
      $("sdFileName").textContent = "";
      $("sdWarn").textContent = `${file.name}: ${e.message}`;
      $("sdWarn").classList.remove("hidden");
    }
    preview();
  }

  /**
   * Open the dialog. `mode`: "new" (project name, machine, type, an optional STL), "import"
   * (a parsed STL given), "edit" (a setup: type, machine, stock, origin, holding), "add" (a new
   * setup for the same part). Resolves to the configuration, or null when cancelled.
   */
  function open(mode, preset = {}) {
    cur?.resolve(null);
    return new Promise((resolve) => {
      cur = { mode, preset, parsed: preset.parsed ? { ...preset.parsed, file: preset.file } : null, resolve };
      const titles = { new: "New project", import: `Import ${preset.file?.name ?? "STL"}`, edit: `Setup: ${preset.setupName ?? ""}`, add: "Add a setup" };
      $("sdTitle").textContent = titles[mode];
      $("sdOk").textContent = { new: "Create", import: "Import", edit: "Apply", add: "Add setup" }[mode];
      $("sdNameRow").firstChild.textContent = mode === "new" ? "Project name " : "Setup name ";
      $("sdName").value = mode === "new" ? preset.name ?? "untitled" : preset.setupName ?? "";
      $("sdMachine").innerHTML = (state.registry?.machines ?? []).map((m) => `<option value="${m.id}">${m.name}</option>`).join("");
      if (preset.machine) $("sdMachine").value = preset.machine;
      const k = preset.kind ?? "rotary";
      for (const r of dlg.querySelectorAll('input[name="sdKind"]')) r.checked = r.value === k;
      // a vise needs the part as a mesh
      const noMesh = (mode === "edit" || mode === "add") && !preset.hasMesh;
      for (const el of dlg.querySelectorAll(".sd-kind")) {
        const off = noMesh && el.dataset.kind === "vise";
        el.classList.toggle("disabled", off);
        el.querySelector("input").disabled = off;
      }
      $("sdFileRow").classList.toggle("hidden", mode !== "new");
      $("sdFile").value = "";
      $("sdFileName").textContent = cur.parsed ? `${preset.file.name}: ${cur.parsed.ascii ? "ASCII" : "binary"} STL, ${cur.parsed.tris.length / 9} triangles` : "";
      $("sdUnits").value = preset.units ?? "mm";
      $("sdUnits").disabled = !(mode === "new" || mode === "import");
      for (const id of ["sdMargin", "sdOrigin", "sdAxis"]) delete $(id).dataset.edited;
      $("sdMargin").value = preset.margin ?? DEFAULT_MARGIN[k];
      $("sdOrigin").value = preset.origin ?? DEFAULT_ORIGIN[k];
      // (the setup's own axis; a change of type resets it to that type's, unless set here)
      $("sdAxis").value = preset.axis ?? (bar(k) ? "x" : "z");
      $("sdTail").checked = !!preset.tailstock;
      const st = preset.stock;
      const ex = preset.explicit;
      dlg.querySelector(`input[name="sdStock"][value="${ex ? "explicit" : "auto"}"]`).checked = true;
      $("sdDia").value = ex?.dia ?? (st?.radius ? 2 * st.radius : "");
      $("sdLen").value = ex?.len ?? st?.length ?? "";
      $("sdBx").value = ex?.bx ?? (st?.max ? st.max[0] - st.min[0] : "");
      $("sdBy").value = ex?.by ?? (st?.max ? st.max[1] - st.min[1] : "");
      $("sdBz").value = ex?.bz ?? (st?.max ? st.max[2] - st.min[2] : "");
      $("sdWarn").classList.add("hidden");
      layout(false);
      if (!dlg.open) dlg.showModal();
    });
  }

  function finish(ok) {
    if (!cur) return;
    const c = cur;
    if (ok) {
      const k = kind();
      const ex = explicit();
      if (ex && Object.values(ex).some((v) => !(v > 0))) {
        $("sdWarn").textContent = "Give every stock size (mm), or choose stock from the part.";
        $("sdWarn").classList.remove("hidden");
        return;
      }
      if (k === "vise" && !c.parsed && !c.preset.tris && c.mode !== "new") {
        $("sdWarn").textContent = "A vise setup needs the part as a mesh: import an STL.";
        $("sdWarn").classList.remove("hidden");
        return;
      }
      const cfg = {
        mode: c.mode,
        name: c.mode === "new" ? $("sdName").value.trim() || "untitled" : undefined,
        setupName: c.mode === "new" ? undefined : $("sdName").value.trim() || undefined,
        machine: $("sdMachine").value || undefined,
        kind: k,
        units: $("sdUnits").value,
        axis: $("sdAxis").value,
        origin: num("sdOrigin"),
        margin: num("sdMargin"),
        explicit: ex,
        tailstock: bar(k) && $("sdTail").checked,
        file: c.parsed?.file ?? null,
        tris: c.parsed?.tris ?? null,
        ascii: !!c.parsed?.ascii,
      };
      cur = null;
      dlg.close();
      c.resolve(cfg);
    } else {
      cur = null;
      dlg.close();
      c.resolve(null);
    }
  }

  // ------------------------------------------------------------ wiring
  dlg.addEventListener("change", (ev) => {
    const t = ev.target;
    if (t.name === "sdKind") return layout(true);
    if (t.id === "sdFile") return chooseFile(t.files[0]);
    if (["sdMargin", "sdOrigin", "sdAxis"].includes(t.id)) t.dataset.edited = "1";
    preview();
  });
  dlg.addEventListener("input", (ev) => {
    if (ev.target.type === "number") preview();
  });
  $("sdForm").addEventListener("submit", (ev) => {
    ev.preventDefault();
    finish(true);
  });
  $("sdCancel").addEventListener("click", () => finish(false));
  dlg.addEventListener("cancel", (ev) => {
    ev.preventDefault();
    finish(false);
  });

  return { open, close: () => finish(false), preview, get mode() { return cur?.mode ?? null; } };
}
