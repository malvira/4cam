// End-to-end test of the web app in a real headless browser, with no npm packages: it drives
// an installed Chrome or Edge over the DevTools protocol (Node 22's built-in WebSocket).
//
//   node web/tests/e2e.mjs            (serves the repo itself on a free port)
//   node web/tests/e2e.mjs --url http://localhost:8000/web/
//
// Screenshots go to web/tests/screenshots/. Needs web/pkg (scripts/build-wasm.*) first.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const shots = join(here, "screenshots");
mkdirSync(shots, { recursive: true });

const argUrl = process.argv.includes("--url") ? process.argv[process.argv.indexOf("--url") + 1] : null;
const children = [];
const cleanup = () => children.forEach((c) => { try { c.kill(); } catch {} });
process.on("exit", cleanup);

function freePort() {
  return new Promise((resolve) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); });
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitFor(fn, what, timeoutMs = 30000) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < timeoutMs) {
    try { last = await fn(); if (last) return last; } catch (e) { last = e; }
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what} (${last})`);
}

function findBrowser() {
  const cands = process.platform === "win32"
    ? ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Microsoft/Edge/Application/msedge.exe"]
    : ["/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"];
  const found = cands.find((p) => existsSync(p));
  if (!found) throw new Error("no Chrome/Edge/Chromium found");
  return found;
}

// ------------------------------------------------------------------ CDP client
class Cdp {
  constructor(ws) {
    this.ws = ws; this.id = 0; this.waiters = new Map(); this.events = [];
    // page dialogs (confirm, prompt, beforeunload) are answered here: OK by default, the
    // prompt's own default text; set dialogAnswer = false to decline
    this.dialogs = []; this.dialogAnswer = true;
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.method === "Page.javascriptDialogOpening") {
        this.dialogs.push(msg.params);
        this.send("Page.handleJavaScriptDialog", { accept: this.dialogAnswer, promptText: msg.params.defaultPrompt ?? "" }, msg.sessionId);
      }
      if (msg.id && this.waiters.has(msg.id)) {
        const { resolve, reject } = this.waiters.get(msg.id);
        this.waiters.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      } else if (msg.method) this.events.push(msg);
    };
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("CDP connect failed")); });
    return new Cdp(ws);
  }
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => this.waiters.set(id, { resolve, reject }));
  }
}

// ------------------------------------------------------------------ run
const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
}

let url = argUrl;
if (!url) {
  const port = await freePort();
  const py = spawn(process.platform === "win32" ? "python" : "python3", ["-m", "http.server", String(port), "--bind", "127.0.0.1", "--directory", root], { stdio: "ignore" });
  children.push(py);
  url = `http://127.0.0.1:${port}/web/`;
  await waitFor(() => fetch(url).then((r) => r.ok), "the web server", 15000);
}

const profile = mkdtempSync(join(tmpdir(), "marcam-e2e-"));
const downloads = join(profile, "downloads");
mkdirSync(downloads);
const dport = await freePort();
const browser = spawn(findBrowser(), [
  "--headless=new", `--remote-debugging-port=${dport}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--window-size=1400,900",
  // WebGPU in headless mode, where the machine has a GPU (the GPU checks fall back otherwise)
  "--enable-unsafe-webgpu", "--enable-gpu",
  // no GPU at all (CI runners): WebGL on the CPU (SwiftShader), so the 3D view still runs
  ...(process.env.MARCAM_E2E_SOFTWARE_GL ? ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] : []),
  "about:blank",
], { stdio: "ignore" });
children.push(browser);
const version = await waitFor(() => fetch(`http://127.0.0.1:${dport}/json/version`).then((r) => r.json()), "the browser", 20000);
const cdp = await Cdp.connect(version.webSocketDebuggerUrl);
const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
const S = (m, p) => cdp.send(m, p, sessionId);
await S("Page.enable");
await S("Runtime.enable");
await S("Emulation.setDeviceMetricsOverride", { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });
await cdp.send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: downloads, eventsEnabled: true });

async function js(expr) {
  const r = await S("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true, replMode: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
}
async function shot(name) {
  const { data } = await S("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(shots, `${name}.png`), Buffer.from(data, "base64"));
}
const state = () => js("document.body.dataset.state");
async function waitDone(what, timeoutMs) {
  return waitFor(async () => { const s = await state(); if (s === "error") throw new Error(await js("document.getElementById('error').textContent")); return s === "done"; }, what, timeoutMs);
}
// Solids must render closed: looking down the rotary axis at either end, the middle of the
// view must show the end face, not the background through an open shell.
const VIEW_PREFIX = process.env.MARCAM_SHOT_PREFIX || "";
async function closedViews(tag) {
  const bg = [0, 0, 0];
  const out = [];
  for (const layer of ["part", "stock", "sim"]) {
    const has = await js(`window.__marcam.state.three.groups.${layer}.children.length > 0`);
    if (!has) continue;
    for (const end of ["front", "back"]) {
      const show = { path: false, axes: false, part: layer === "part", stock: layer === "stock", sim: layer === "sim" };
      await js(`window.__marcam.setView(${JSON.stringify(end)}, ${JSON.stringify(show)})`);
      await sleep(200);
      // the axis line of sight and a small ring around it must all hit the end face
      let px = [], filled = true;
      for (const [fx, fy] of [[0.5, 0.5], [0.49, 0.5], [0.51, 0.5], [0.5, 0.49], [0.5, 0.51]]) {
        const p = String(await js(`window.__marcam.pixelAt(${fx}, ${fy})`)).split(",").map(Number);
        if (!p.some((c, k) => Math.abs(c - bg[k]) > 4)) filled = false; // background is exactly black
        px = p;
      }
      await shot(`${VIEW_PREFIX}view_${tag}_${layer}_${end}`);
      out.push({ layer, end, filled, px });
    }
    await js(`window.__marcam.setView("iso", { path: false, axes: false, part: ${layer === "part"}, stock: ${layer === "stock"}, sim: ${layer === "sim"} })`);
    await sleep(200);
    await shot(`${VIEW_PREFIX}view_${tag}_${layer}_iso`);
  }
  await js(`window.__marcam.setView("iso", { path: true, axes: true, part: true, stock: true, sim: true })`);
  return out;
}

// longer than the auto-regenerate delay (800 ms)
const AUTO_WAIT = 1500;
const golden = (name) => readFileSync(join(root, "tests", "golden", `${name}.nc`), "utf8").replace(/\r\n/g, "\n");

let failed = false;
try {
  await S("Page.navigate", { url });
  await waitFor(async () => (await state()) === "ready", "the engine to load", 30000);
  check("page loads and the WebAssembly engine starts", true, await js("document.getElementById('version').textContent"));

  // 1. knob example on the Makera Z1: byte-identical to the cut G-code
  await js("document.getElementById('generate').click()");
  await waitDone("knob generation", 60000);
  const tabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  check("two programs, both verified OK", JSON.stringify(tabs) === JSON.stringify(["knob_1_profileOK", "knob_2_knurlOK"]), tabs.join(", "));
  const lines = [];
  for (let i = 0; i < 2; i++) {
    await js(`document.querySelectorAll('#tabs button')[${i}].click()`);
    lines.push(await js("document.getElementById('lineCount').textContent"));
    const name = i ? "knob_2_knurl" : "knob_1_profile";
    const text = await js("document.getElementById('gcode').value");
    check(`${name}: G-code view matches the golden file byte for byte`, text === golden(name), `${text.length} chars`);
    const issues = await js("document.querySelector('#summary .issues').textContent");
    check(`${name}: verifier reports 0 issues`, issues.includes("0 issues"), issues.slice(0, 60));
    const holder = await js("document.getElementById('holderCheck')?.textContent || ''");
    check(`${name}: holder/shank check clear`, holder.includes("clear"), holder);
  }
  check("line counts 21,787 / 15,146", lines.join(" / ") === "21,787 / 15,146", lines.join(" / "));

  // 2. 3D view (three.js from the CDN) and the scrubber
  await js("document.querySelectorAll('#tabs button')[0].click()");
  await waitFor(() => js("!!(window.__marcam.state.three && (window.__marcam.state.three.failed || window.__marcam.state.three.cutLines))"), "the 3D view", 30000);
  const three = await js("window.__marcam.state.three.failed ? 'failed: ' + document.getElementById('viewNote').textContent : 'ok'");
  check("3D preview renders (three.js)", three === "ok", three);
  if (three === "ok") {
    const full = await js("window.__marcam.state.three.cutLines.geometry.drawRange.count");
    await js("const s = document.getElementById('scrub'); s.value = 3000; s.dispatchEvent(new Event('input')); true");
    await sleep(300);
    const part = await js("window.__marcam.state.three.cutLines.geometry.drawRange.count");
    const label = await js("document.getElementById('scrubLabel').textContent");
    check("scrubber limits the drawn toolpath", part > 0 && part < full, `${part} of ${full} vertices; ${label}`);
    await shot("01_knob_scrubbed");
    await js("const s = document.getElementById('scrub'); s.value = s.max; s.dispatchEvent(new Event('input')); true");
  }
  await shot("02_knob_generated");

  // 3. download
  await js("document.getElementById('download').click()");
  const file = await waitFor(() => readdirSync(downloads).find((f) => f.endsWith(".nc")), "the .nc download", 15000);
  const dl = readFileSync(join(downloads, file), "utf8");
  check("Download .nc saves the program", file === "knob_1_profile.nc" && dl === golden("knob_1_profile"), file);

  // 4. simulation
  await js("document.getElementById('simulate').click()");
  await waitDone("simulation", 120000);
  const simText = await js("document.getElementById('summary').innerText");
  check("simulated stock: no gouges, no rapids through stock", /deepest gouge 0\.000 mm; 0 rapid/.test(simText), simText.split("\n").pop());
  await shot("03_knob_simulated");
  const kv = await closedViews("knob");
  check("knob: part, stock and simulated stock render closed (end faces filled)",
    kv.length === 6 && kv.every((v) => v.filled), kv.map((v) => `${v.layer}/${v.end}:${v.filled ? "filled" : "OPEN " + v.px}`).join(" "));

  // 4b. solid machining simulation: playback, scrubbing, per-operation colours
  await js("document.getElementById('simOpen').click(); true");
  await waitFor(() => js("document.body.dataset.machining === 'open'"), "the machining simulation", 60000);
  const M = "window.__marcam.machining.state";
  const nseg = await js(`${M}.session.segments`);
  const ops = await js(`${M}.session.ops.map(o => o.kind).join(',')`);
  check("machining simulation opens with the job's operations", nseg > 30000 && ops === "rotary_turning,helical_knurl,flank_chamfer", `${nseg} moves; ${ops}`);
  await js("window.__marcam.setView('iso', {})");
  await sleep(300);
  await shot("08_machining_start");
  const seekTo = async (n) => {
    await js(`const s = document.getElementById('simScrub'); s.value = ${n}; s.dispatchEvent(new Event('input')); true`);
    await waitFor(() => js(`${M}.pos === ${n} && !${M}.busy`), `seeking to ${n}`, 60000);
  };
  const cutCells = (tag) => js(`${M}.op.reduce((n, t) => n + (t === ${tag}), 0)`);
  const midRough = Math.round(nseg * 0.15);
  await seekTo(midRough);
  const rough1 = await cutCells(1);
  const minR = await js(`Math.min(...${M}.r)`);
  check("seeking cuts the stock (mid-roughing)", rough1 > 1000 && minR < 12.5, `${rough1} cells cut by roughing, min radius ${minR.toFixed(2)}`);
  await js("window.__marcam.setView('iso', {})");
  await sleep(300);
  await shot("09_machining_mid_roughing");
  // play advances
  await js("document.getElementById('simSpeed').value = '3000'; document.getElementById('simPlay').click(); true");
  await sleep(1500);
  await js("document.getElementById('simPlay').click(); true");
  await waitFor(() => js(`!${M}.busy`), "playback to settle", 10000);
  const played = await js(`${M}.pos`);
  check("Play advances the simulation", played > midRough + 100, `${midRough} -> ${played}`);
  // scrubbing back recomputes
  await seekTo(500);
  const back = await cutCells(1);
  check("scrubbing back recomputes the stock", back < rough1, `roughing cells ${rough1} at ${midRough}, ${back} at 500`);
  // after the knurl
  await seekTo(nseg);
  const k2 = await cutCells(2), k3 = await cutCells(3);
  check("after the knurl, knurl and chamfer cells carry their operations", k2 > 10000 && k3 > 100, `knurl ${k2} cells, chamfer ${k3}`);
  const closed = String(await js(`const { checkClosed } = await import('./meshes.js'); const g0 = ${M}.stockMesh.geometry; const c0 = checkClosed({ positions: g0.attributes.position.array, indices: g0.index.array }); c0.closed + ' ' + c0.badEdges + ' ' + c0.volume.toFixed(0)`));
  check("the simulated solid stays watertight", closed.startsWith("true"), closed);
  await js("window.__marcam.setView('iso', {})");
  await sleep(300);
  await shot("10_machining_after_knurl");
  // operation colour
  const colorAt = `(() => { const s = ${M}; const i = s.op.indexOf(2); return Array.from(s.stockMesh.geometry.attributes.color.array.slice(3 * i, 3 * i + 3)).map(c => c.toFixed(2)).join(','); })()`;
  const before = await js(colorAt);
  await js("const c = document.querySelector('#opLegend input[data-tag=\"2\"]'); c.value = '#ff0000'; c.dispatchEvent(new Event('input', { bubbles: true })); true");
  const after = await js(colorAt);
  const saved = await js("localStorage.getItem('marcam.opColors')");
  check("changing an operation's colour recolours its cuts (and is saved)", before !== after && after === "1.00,0.00,0.00" && /#ff0000/.test(saved), `${before} -> ${after}`);
  await js("window.__marcam.setView('iso', {})");
  await sleep(300);
  await shot("11_machining_recoloured");
  await js("document.getElementById('simClose').click(); true");

  // 5. LinuxCNC machine (G93)
  await js("document.getElementById('machine').value = 'linuxcnc-4axis'; document.getElementById('machine').dispatchEvent(new Event('change')); document.getElementById('generate').click()");
  await waitDone("LinuxCNC generation", 60000);
  const g93 = await js("document.getElementById('gcode').value.includes('\\nG93 ')");
  check("LinuxCNC post uses G93 inverse time", g93);

  // 5b. machine profiles: the Carvera, with its limits checked
  await js("document.getElementById('machine').value = 'makera-carvera-4axis'; document.getElementById('machine').dispatchEvent(new Event('change')); document.getElementById('generate').click()");
  await waitDone("Carvera generation", 60000);
  const info = await js("document.getElementById('machineInfo').textContent");
  check("Carvera profile shows its limits", /X 360 mm/.test(info) && /ATC 6 slots/.test(info), info);
  const lim = await js("document.getElementById('limits').innerText");
  check("limit check flags the Carvera's 5 RPM rotary cap (5209 > 1800 deg/min)", /Carvera/.test(lim) && /5209 deg\/min/.test(lim) && /1800 deg\/min/.test(lim), lim.split("\n")[1] || lim);

  // 5c. tool library: import a Fusion 360 library by URL, edit a tool, use it as T1
  await js("document.getElementById('libraryPanel').open = true; document.getElementById('importUrl').value = new URL('../tests/fixtures/fusion-tool-library.json', location).href; document.getElementById('importUrlBtn').click(); true");
  const imported = await waitFor(() => js("document.getElementById('importInfo').textContent"), "the library import", 10000);
  check("Fusion tool library imports from a URL", /3 tools/.test(imported) && (await js("document.getElementById('library').value")) === "fusion-tool-library", imported);
  await js("document.querySelector('#toolTable button[data-edit=\"0\"]').click(); document.getElementById('tfName').value = 'Edited Flat 3.0'; document.getElementById('tfDiameter').value = '3.0'; document.getElementById('tfSave').click(); true");
  const row = await js("document.querySelector('#toolTable tr[data-tool]').innerText");
  check("editing a tool updates the library", /Edited Flat 3\.0/.test(row) && /Ø3\b/.test(row), row.replace(/\s+/g, " "));
  await js("const s = document.querySelector('#toolTable select[data-use=\"0\"]'); s.value = '1'; s.dispatchEvent(new Event('change', { bubbles: true })); true");
  const t1 = await js("JSON.parse(document.getElementById('job').value).tools.find(t => t.number === 1)");
  check("library tool becomes the job's T1", t1.description === "Edited Flat 3.0" && t1.diameter === 3 && t1.kind === "flat", JSON.stringify(t1));
  await js("document.getElementById('machine').value = 'makera-z1-4axis'; document.getElementById('machine').dispatchEvent(new Event('change')); document.getElementById('generate').click()");
  await waitDone("generation with the library tool", 60000);
  await js("document.querySelectorAll('#tabs button')[0].click()");
  const head = await js("document.getElementById('gcode').value.split('\\n').slice(0, 3).join(' | ')");
  const issues0 = await js("document.querySelector('#summary .issues').textContent");
  check("generates with the edited tool", head.includes("(T1 Edited Flat 3.0)") && issues0.includes("0 issues"), head);
  // a tapered ball: edited in the form, then added to the job as a new tool number
  await js("document.querySelector('#toolTable button[data-edit=\"1\"]').click(); document.getElementById('tfKind').value = 'tapered_ball'; document.getElementById('tfDiameter').value = '3.175'; document.getElementById('tfTip').value = '1'; document.getElementById('tfTaper').value = '5'; document.getElementById('tfSave').click(); true");
  const tbRow = await js("document.querySelectorAll('#toolTable tr[data-tool]')[1].innerText");
  await js("const s = document.querySelector('#toolTable select[data-use=\"1\"]'); s.value = 'new'; s.dispatchEvent(new Event('change', { bubbles: true })); true");
  const tbTool = await js("JSON.parse(document.getElementById('job').value).tools.find(t => t.kind === 'tapered_ball')");
  check("a tapered ball tool is edited and joins the job", /tapered ball tip Ø1 5°/.test(tbRow) && tbTool && tbTool.tip_diameter === 1 && tbTool.taper_angle === 5 && tbTool.diameter === 3.175, `${tbRow.replace(/\s+/g, " ")} ${JSON.stringify(tbTool)}`);
  await shot("07_tool_library");
  await S("Page.reload");
  await waitFor(async () => (await state()) === "ready", "reload", 30000);
  const kept = await js("[...document.getElementById('library').options].map(o => o.value).join(',') + ' / ' + JSON.stringify(JSON.parse(localStorage.getItem('marcam.libraries')).find(l => l.id === 'fusion-tool-library').tools[0].name)");
  check("tool libraries persist across reloads", /fusion-tool-library/.test(kept) && /Edited Flat 3\.0/.test(kept), kept);
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await sleep(500);

  // 5d. project tree (document model 0.2)
  await js("document.getElementById('machine').value = 'makera-z1-4axis'; document.getElementById('machine').dispatchEvent(new Event('change')); document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.querySelectorAll('#tree .tree-op').length === 3"), "the project tree", 15000);
  const outs = await js("[...document.querySelectorAll('#tree .tree-out')].map(o => o.dataset.out + ':' + o.querySelectorAll('.tree-op').length).join(' ')");
  check("tree shows the knob as one setup, two files, three operations", outs === "knob_1_profile:1 knob_2_knurl:2", outs);
  const gen = async () => {
    await js("document.getElementById('generate').click(); true");
    await waitDone("generation", 120000);
    return js("({ lines: window.__marcam.state.result.programs.map(p => p.lines), knurl: (window.__marcam.state.result.programs.find(p => p.name === 'knob_2_knurl') || {}).gcode || '' })");
  };
  const treeAct = (op, act) => js(`document.querySelector('#tree .tree-op[data-op="${op}"] [data-act="${act}"]').click(); true`);
  const settle = () => waitFor(() => js("!!window.__marcam.state.tree.doc && document.body.dataset.state !== 'busy'"), "tree", 30000).then(() => sleep(400));
  await treeAct("op3", "enable");
  await settle();
  const off = await gen();
  const schema = await js("JSON.parse(document.getElementById('job').value).schema_version");
  check("turning an operation off drops it from the G-code (document saved as 0.2)", schema === "0.2" && off.lines[1] < 15146 && !off.knurl.includes("front chamfer"), `knurl file ${off.lines[1]} lines; schema ${schema}`);
  await treeAct("op3", "enable");
  await settle();
  await treeAct("op3", "up");
  await settle();
  const swapped = await gen();
  const ci = swapped.knurl.indexOf("front chamfer"), ki = swapped.knurl.indexOf("knurl family");
  check("reordering operations reorders the G-code", ci > 0 && ki > ci, `chamfer at ${ci}, knurl at ${ki}`);
  await treeAct("op3", "down");
  await settle();
  // every operation type: the create form comes fully filled and applies without problems
  // the types offered for this (rotary) setup: the 3-axis ones need a vise setup
  const typesAll = await js("[...document.getElementById('treeAddType').options].map(o => o.value).join(',')");
  const allKinds = await js("window.__marcam.state.tree.types.map(t => t.kind).join(',')");
  check("a rotary setup offers every operation type (3-axis ones indexed)", String(allKinds).split(",").length === 14 && String(typesAll).split(",").length === 14, `${typesAll}`);
  const filled = [];
  for (const kind of String(typesAll).split(",")) {
    await js(`document.getElementById('treeAddType').value = '${kind}'; document.getElementById('treeAddOut').value = 'knob_2_knurl'; document.getElementById('treeAdd').click(); true`);
    await waitFor(() => js(`!!document.getElementById('opForm') && !!window.__marcam.state.tree.draft && window.__marcam.state.tree.draft.op.type === '${kind}'`), `the ${kind} form`, 15000);
    // blank fields, but text whose registry default is "" (none: e.g. a V-carve's text)
    const blanks = await js(`(() => { const ty = window.__marcam.tree.state.types.find(x => x.kind === '${kind}'); const none = new Set(ty.params.filter(p => p.default === '').map(p => p.name)); return [...document.getElementById('opForm').querySelectorAll('input, select')].filter(e => e.type !== 'checkbox' && e.type !== 'file' && !['__name', '__axial'].includes(e.name) && !none.has(e.name) && e.value.trim() === '').map(e => e.name).join(','); })()`);
    const before = await js("document.querySelectorAll('#tree .tree-op').length");
    await js("document.getElementById('opForm').requestSubmit(); true");
    await waitFor(() => js(`document.querySelectorAll('#tree .tree-op').length === ${before + 1}`), `${kind} to be added`, 15000);
    const id = await js("[...document.querySelectorAll('#tree .tree-op')].pop().dataset.op");
    filled.push(`${kind}:${blanks || "none blank"}`);
    if (blanks) break;
    await treeAct(id, "delete");
    await settle();
  }
  check("every operation type's new form is fully filled and applies cleanly", filled.length === 14 && filled.every((f) => f.endsWith("none blank")), filled.join(" "));
  // changing the tool refreshes defaults the user hasn't edited
  await js("document.getElementById('treeAddType').value = 'rotary_finish'; document.getElementById('treeAdd').click(); true");
  await waitFor(() => js("window.__marcam.state.tree.draft?.op.type === 'rotary_finish'"), "the finish form", 15000);
  const pat1 = await js("document.getElementById('opForm').elements.pattern.value");
  await js("const f = document.getElementById('opForm'); f.elements.feed.value = '123'; f.elements.feed.dispatchEvent(new Event('input', { bubbles: true })); f.elements.tool.value = '2'; f.elements.tool.dispatchEvent(new Event('change', { bubbles: true })); true");
  await waitFor(() => js("window.__marcam.state.tree.draft?.op.tool === 2 && document.getElementById('opForm').elements.pattern.value === 'along'"), "defaults for the V-bit", 15000);
  const keptFeed = await js("document.getElementById('opForm').elements.feed.value");
  check("changing the tool refreshes untouched defaults and keeps edited ones", pat1 === "around" && keptFeed === "123", `pattern around -> along with the V-bit; edited feed kept at ${keptFeed}`);
  await js("document.getElementById('cancelDraft').click(); true");
  await settle();

  // add an operation from the registry's form
  await js("document.getElementById('treeAddType').value = 'flank_chamfer'; document.getElementById('treeAddOut').value = 'knob_2_knurl'; document.getElementById('treeAdd').click(); true");
  await waitFor(() => js("!!document.getElementById('opForm')"), "the operation form", 10000);
  await js("const f0 = document.getElementById('opForm'); f0.elements.edge.value = ''; f0.requestSubmit(); true");
  const probs = await waitFor(() => js("document.getElementById('opErrors')?.textContent || ''"), "the missing-field message", 10000);
  await js("const f = document.getElementById('opForm'); f.elements.edge.value = '0, 7.2'; f.elements.tip_offset.value = '0.8'; f.elements.passes.value = '0'; f.elements.feed.value = '200'; f.elements.plunge_feed.value = '60'; f.requestSubmit(); true");
  await waitFor(() => js("document.querySelectorAll('#tree .tree-op').length === 4"), "the new operation", 10000);
  await settle();
  const probsAfter = await js("document.querySelector('#tree .error')?.textContent || ''");
  const added = await gen();
  check("adding an operation: the form shows what's missing, then it plans", /missing field `edge`/.test(probs) && probsAfter === "" && added.lines[1] > 15146, `before: ${probs.slice(0, 40)}...; knurl file ${added.lines[1]} lines`);
  // delete everything, then the knurl wizard rebuilds the program
  for (const op of ["op4", "op3", "op2", "op1"]) {
    await treeAct(op, "delete");
    await settle();
  }
  const empty = await js("document.querySelectorAll('#tree .tree-op').length");
  await js("document.getElementById('treeWizard').click(); true");
  await waitFor(() => js("document.querySelectorAll('#tree .tree-op').length === 3"), "the wizard", 15000);
  const wiz = await gen();
  await js("document.querySelectorAll('#tabs button')[0].click(); true");
  const g1 = await js("window.__marcam.state.result.programs[0].gcode");
  check("knurl wizard rebuilds the knob in the tree; G-code is the cut G-code", empty === 0 && g1 === golden("knob_1_profile") && wiz.knurl === golden("knob_2_knurl"), `${empty} ops after deleting, ${wiz.lines.join(" / ")} lines`);
  // operation colour from the tree reaches the machining simulation
  await js("const c = document.querySelector('#tree .tree-op[data-op=\"op1\"] [data-act=\"color\"]'); c.value = '#00ff00'; c.dispatchEvent(new Event('change', { bubbles: true })); true");
  await settle();
  await js("document.getElementById('simOpen').click(); true");
  await waitFor(() => js("document.body.dataset.machining === 'open'"), "the machining simulation", 60000);
  const legend = await js("document.querySelector('#opLegend input[data-tag=\"1\"]').value");
  check("operation colours set in the tree colour the machining simulation", legend === "#00ff00", legend);
  await js("document.getElementById('simClose').click(); true");
  // face picking on the mesh job: finish only the taper
  await js("document.getElementById('example').value = 'knob_finish'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.getElementById('meshInfo').textContent.includes('knob.stl') && document.querySelectorAll('#tree .tree-op').length === 3"), "the mesh project", 15000);
  await gen();
  await js("document.querySelector('#tree .tree-op[data-op=\"op2\"] [data-act=\"select\"]').click(); true");
  await js("document.getElementById('pickFaces').click(); true");
  await waitFor(() => js("!!window.__marcam.state.tree.pickMesh"), "pick mode", 15000);
  await js("window.__marcam.setView('iso', {})");
  const picked = await js(`(() => { const v = window.__marcam.state.three; const p = new v.THREE.Vector3(-7.5, 0, -9.0).project(v.camera); const el = v.renderer.domElement; return [(p.x + 1) / 2 * el.clientWidth, (1 - p.y) / 2 * el.clientHeight]; })()`);
  const region = String(await js(`const pr = await window.__marcam.tree.pickAt(${picked[0]}, ${picked[1]}); pr ? pr.shape + ' ' + pr.x.map(x => x.toFixed(1)).join('..') : 'none'`));
  await shot("13_pick_faces");
  await js("document.getElementById('pickFaces').click(); true");
  await settle();
  const fin = await gen();
  const along = await js("window.__marcam.state.result.programs.find(p => p.name === 'knob_4_finish_along').gcode");
  const xs = along.split("\n").filter((l) => l.startsWith("G1 X")).map((l) => parseFloat(l.split(" ")[1].slice(1)));
  const minX = Math.min(...xs);
  check("picking the taper face limits the finish to it", /^cone -15\.0\.\.-0\.8/.test(region) && minX >= -16.588, `picked ${region}; finish min X ${minX}`);
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await sleep(800);

  // 6. quick settings edit the JSON
  await js("const i = document.getElementById('q_stock_radius'); i.value = 13; i.dispatchEvent(new Event('change')); true");
  const r = await js("JSON.parse(document.getElementById('job').value).stock.radius");
  check("quick setting writes into the job JSON", r === 13, `stock.radius = ${r}`);

  // 7. errors are shown, not thrown
  await js("document.getElementById('job').value = '{ not json'; document.getElementById('generate').click(); true");
  const err = await waitFor(() => js("document.getElementById('error').textContent"), "an error message", 5000);
  check("invalid JSON shows an error", /not valid/.test(err), err.slice(0, 60));

  // 8. mesh example: rough + two finishes against knob.stl
  await js("document.getElementById('machine').value = 'makera-z1-4axis'; document.getElementById('example').value = 'knob_finish'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.getElementById('meshInfo').textContent.includes('knob.stl')"), "the example mesh", 15000);
  await js("document.getElementById('generate').click()");
  await waitDone("mesh job generation", 240000);
  const mtabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  check("mesh job: three programs, all verified OK", mtabs.length === 3 && mtabs.every((t) => t.endsWith("OK")), mtabs.join(", "));
  const gouges = [];
  for (let i = 0; i < 3; i++) {
    await js(`document.querySelectorAll('#tabs button')[${i}].click()`);
    const t = await js("document.getElementById('summary').innerText");
    gouges.push(parseFloat((t.match(/deepest ([0-9.]+) mm into the part/) || [])[1]));
  }
  check("mesh job: gouge check under 0.01 mm", gouges.every((g) => g < 0.01), gouges.join(", "));
  await js("document.querySelectorAll('#tabs button')[2].click()");
  await sleep(1500);
  await shot("04_mesh_finish");
  await js("document.getElementById('simulate').click()");
  await waitDone("mesh simulation", 240000);
  await sleep(1000);
  const msim = await js("document.getElementById('summary').innerText");
  check("mesh job simulated: no gouges", /deepest gouge 0\.0\d\d mm; 0 rapid/.test(msim), msim.split("\n").pop());
  await shot("05_mesh_simulated");
  const mv = await closedViews("mesh");
  check("mesh job: part, stock and simulated stock render closed (end faces filled)",
    mv.length === 6 && mv.every((v) => v.filled), mv.map((v) => `${v.layer}/${v.end}:${v.filled ? "filled" : "OPEN " + v.px}`).join(" "));

  // 8b. resizable panes: drag the bars, keyboard, reset, persistence, minimum sizes
  const rect = (id) => js(`(() => { const r = document.getElementById('${id}').getBoundingClientRect(); return [r.x, r.y, r.width, r.height].map(Math.round).join(','); })()`).then((t) => String(t).split(",").map(Number));
  // 8b. 3-axis plate on the Carvera vise: types, forms with points, generate, preview
  await js("document.getElementById('example').value = 'plate'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.getElementById('meshInfo').textContent.includes('dome.stl')"), "the plate's dome mesh", 15000);
  await settle();
  const plateMachine = await js("document.getElementById('machine').value");
  const plateTypes = await js("[...document.getElementById('treeAddType').options].map(o => o.value).join(',')");
  check("vise setup: the setup's machine and only 3-axis operation types", plateMachine === "makera-carvera-4axis" && plateTypes === "face,pocket,drill,bore,contour,raster3d,rough3d,engrave,vcarve", `${plateMachine}; ${plateTypes}`);
  await js("document.getElementById('treeAddType').value = 'drill'; document.getElementById('treeAddOut').value = 'plate_1_flat'; document.getElementById('treeAdd').click(); true");
  await waitFor(() => js("window.__marcam.state.tree.draft?.op.type === 'drill' && !!document.getElementById('opForm')"), "the drill form", 15000);
  const drillBlank = await js("[...document.getElementById('opForm').querySelectorAll('input, select')].filter(e => e.type !== 'checkbox' && e.name !== '__name' && e.value.trim() === '').map(e => e.name).join(',')");
  const plateOps = await js("document.querySelectorAll('#tree .tree-op').length");
  await js("const f = document.getElementById('opForm'); f.elements.points.value = '30 20; 33 20'; f.elements.points.dispatchEvent(new Event('input', { bubbles: true })); f.requestSubmit(); true");
  await waitFor(() => js(`document.querySelectorAll('#tree .tree-op').length === ${plateOps + 1}`), "the drill to be added", 15000);
  const drillPts = await js("JSON.stringify(JSON.parse(document.getElementById('job').value).setups[0].operations.find(o => o.type === 'drill' && o.points.length === 2)?.points)");
  check("3-axis form: fully filled, and points are typed as 'x y; x y'", drillBlank === "" && drillPts === "[[30,20],[33,20]]", `blank: ${drillBlank || "none"}; points ${drillPts}`);
  // a V-carve whose outline comes from an SVG file, through the form's file input
  await js("document.getElementById('treeAddType').value = 'vcarve'; document.getElementById('treeAddOut').value = 'plate_1_flat'; document.getElementById('treeAdd').click(); true");
  await waitFor(() => js("window.__marcam.state.tree.draft?.op.type === 'vcarve' && !!document.getElementById('opForm')"), "the V-carve form", 15000);
  await js("const inp = document.querySelector('#opForm input[data-svg]'); const dt = new DataTransfer(); dt.items.add(new File(['<svg xmlns=\"http://www.w3.org/2000/svg\"><polygon points=\"0,0 20,0 10,8\"/></svg>'], 'tri.svg', { type: 'image/svg+xml' })); inp.files = dt.files; document.getElementById('opForm').elements.__svg_height.value = '8'; inp.dispatchEvent(new Event('change', { bubbles: true })); true");
  await waitFor(() => js("document.getElementById('opForm').elements.paths.value.includes('30 16')"), "the SVG outline in the form", 15000);
  const svgField = await js("document.getElementById('opForm').elements.paths.value");
  const vOps = await js("document.querySelectorAll('#tree .tree-op').length");
  await js("document.getElementById('opForm').requestSubmit(); true");
  await waitFor(() => js(`document.querySelectorAll('#tree .tree-op').length === ${vOps + 1}`), "the V-carve to be added", 15000);
  check("SVG import replaces the V-carve outline (at the old outline's corner, 8 mm tall)", /^20 24; 40 24; 30 16; 20 24$/.test(svgField), svgField);
  await js("document.getElementById('generate').click()");
  await waitDone("plate generation", 240000);
  const ptabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  const plimits = await js("document.getElementById('summary').innerText");
  const pkinds = await js("[...new Set(window.__marcam.state.result.programs.flatMap(p => { const a = p.toolpath.points; const k = []; for (let i = 3; i < a.length; i += 4) k.push(a[i]); return k; }))].sort().join(',')");
  check("plate: two programs verified OK, machine limits OK, XYZ toolpaths", ptabs.length === 2 && ptabs.every((t) => t.endsWith("OK")) && /Machine limits: OK/.test(plimits) && pkinds === "2,3", `${ptabs.join(", ")}; kinds ${pkinds}`);
  await sleep(1500);
  await shot("07_plate");
  // batch simulation of the block: dome finished with no gouge, no rapids through stock
  await js("document.getElementById('simulate').click()");
  await waitDone("plate simulation", 240000);
  await js("document.querySelectorAll('#tabs button')[1].click()");
  await sleep(1000);
  const psim = await js("document.getElementById('summary').innerText.split('\\n').pop()");
  const pleft = parseFloat((psim.match(/left max ([0-9.]+)/) || [])[1]);
  check("plate simulated: dome finished, no gouge, no rapids through stock", /deepest gouge 0\.00\d mm; 0 rapid/.test(psim) && pleft < 0.5, psim);
  await shot("08_plate_simulated");
  // machining playback: to the end, every operation's colour on the block
  await js("document.getElementById('simOpen').click()");
  await waitFor(() => js("window.__marcam.state.machining.open && !!window.__marcam.state.machining.stockMesh"), "the plate machining view", 60000);
  const psegs = await js("window.__marcam.state.machining.session.segments");
  await js(`document.getElementById('simScrub').value = '${psegs}'; document.getElementById('simScrub').dispatchEvent(new Event('input')); true`);
  await waitFor(() => js(`window.__marcam.state.machining.pos === ${psegs} && !window.__marcam.state.machining.busy`), "the plate playback to finish", 120000);
  const ptags = await js("[...new Set(window.__marcam.state.machining.op)].sort((a, b) => a - b).join(',')");
  const pz = await js("(() => { const m = window.__marcam.state.machining; const nx = m.session.grid.nx; const at = (x, y) => m.r[Math.floor(y / 0.25) * nx + Math.floor(x / 0.25)]; return [at(15, 20), at(40, 6), at(46, 20)].map(v => v.toFixed(2)).join(','); })()");
  const vtx = await js("(() => { const m = window.__marcam.state.machining; const nx = m.session.grid.nx; const k = Math.floor(20 / 0.25) * nx + Math.floor(15 / 0.25); return m.stockMesh.geometry.attributes.position.array[3 * k + 2].toFixed(2); })()");
  check("plate playback: block cut by all eight operations (six + the added drill and V-carve), mesh follows the engine", ptags === "1,2,3,4,5,6,7,8" && pz.startsWith("-3.50,-4.50,") && Math.abs(parseFloat(pz.split(",")[2]) + 0.5) < 0.03 && vtx === "-3.50", `tags ${ptags}; pocket, hole, dome top ${pz}; vertex ${vtx}`);
  await sleep(800);
  await shot("09_plate_machining");
  await js("document.getElementById('simClose').click(); true");
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await settle();

  // 8f. machining time: in the summary, per file and per operation in the tree
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await settle();
  await js("document.getElementById('generate').click()");
  await waitDone("knob generation for times", 240000);
  await settle();
  const tSum = await js("document.getElementById('timeEst')?.innerText || ''");
  const tTree = await js("[...document.querySelectorAll('#tree .tree-time')].map(e => e.textContent).join(' | ')");
  check("time estimates: summary, files and operations in the tree", /^Estimated time: \d/.test(tSum) && String(tTree).split(" | ").length === 5, `${tSum} || ${tTree}`);

  // 8g. simulation resolution: the setting changes the grid (shown before running), the
  // machining view at that grid stays watertight, and Ultra shows a decimated display
  // top-level await: the evaluation does not wait for a promise an async IIFE returns
  const closedNow = async () => (await js("const { checkClosed: cc } = await import('./meshes.js'); const gg = window.__marcam.state.machining.stockMesh.geometry; String(cc({ positions: gg.attributes.position.array, indices: gg.index.array }).closed)")) === "true";
  await js("document.getElementById('simRes').value = 'fine'; document.getElementById('simRes').dispatchEvent(new Event('change')); true");
  await waitFor(() => js("(window.__marcam.state.simGrid?.na === 720) && document.getElementById('simResInfo').textContent.includes('cells')"), "the Fine grid info", 15000);
  const fineInfo = await js("document.getElementById('simResInfo').textContent");
  const fineCells = await js("window.__marcam.state.simGrid.cells");
  await js("document.getElementById('simOpen').click(); true");
  await waitFor(() => js("window.__marcam.state.machining.open && window.__marcam.state.machining.session?.grid.na === 720 && !!window.__marcam.state.machining.stockMesh"), "the Fine machining view", 60000);
  await js("document.getElementById('simScrub').value = '3000'; document.getElementById('simScrub').dispatchEvent(new Event('input')); true");
  await waitFor(() => js("window.__marcam.state.machining.pos === 3000 && !window.__marcam.state.machining.busy"), "the Fine playback", 120000);
  const fineClosed = await closedNow();
  const fineStride = await js("window.__marcam.state.machining.stride");
  await js("document.getElementById('simClose').click(); true");
  check("Fine grid: cell count shown, machining view at 720 cells round, watertight", fineInfo.includes(Number(fineCells).toLocaleString("en")) && fineClosed === true && fineStride === 1, `${fineInfo}; closed ${fineClosed}`);
  await js("document.getElementById('simRes').value = 'ultra'; document.getElementById('simRes').dispatchEvent(new Event('change')); true");
  await waitFor(() => js("window.__marcam.state.simGrid?.na === 1440"), "the Ultra grid info", 15000);
  await js("document.getElementById('simOpen').click(); true");
  await waitFor(() => js("window.__marcam.state.machining.open && window.__marcam.state.machining.session?.grid.na === 1440 && !!window.__marcam.state.machining.stockMesh"), "the Ultra machining view", 60000);
  await js("document.getElementById('simScrub').value = '2000'; document.getElementById('simScrub').dispatchEvent(new Event('input')); true");
  await waitFor(() => js("window.__marcam.state.machining.pos === 2000 && !window.__marcam.state.machining.busy"), "the Ultra playback", 120000);
  const ultra = await js("JSON.stringify({ s: window.__marcam.state.machining.stride, cells: window.__marcam.state.simGrid.cells, disp: window.__marcam.state.machining.dr.length })");
  const ultraClosed = await closedNow();
  await js("document.getElementById('simClose').click(); document.getElementById('simRes').value = 'normal'; document.getElementById('simRes').dispatchEvent(new Event('change')); true");
  const u = JSON.parse(ultra);
  check("Ultra grid: display decimated to at most 1M cells, still watertight", u.cells > 1e6 && u.s >= 2 && u.disp <= 1e6 && ultraClosed === true, `${ultra}; closed ${ultraClosed}`);
  const remembered = await js("JSON.parse(localStorage.getItem('marcam.simRes')).v");
  check("the resolution setting is remembered", remembered === "normal", remembered);

  // 8g2. face-bounded raster: pick the dome's cap for its raster; the finish stays on it
  {
    await js("document.getElementById('example').value = 'plate'; document.getElementById('loadExample').click(); true");
    await waitFor(() => js("document.getElementById('meshInfo').textContent.includes('dome.stl') && !!document.querySelector('#tree .tree-op[data-op=\"op5\"]')"), "the plate again", 15000);
    await js("document.querySelector('#tree .tree-op[data-op=\"op5\"] [data-act=\"select\"]').click(); true");
    await waitFor(() => js("!!document.getElementById('pickFaces')"), "the raster's face picker", 15000);
    await js("document.getElementById('pickFaces').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.pickMesh"), "pick mode on the plate", 15000);
    await js("window.__marcam.setView('iso', {})");
    const at = await js(`(() => { const v = window.__marcam.state.three; const p = new v.THREE.Vector3(46, 20, -0.6).project(v.camera); const el = v.renderer.domElement; return [(p.x + 1) / 2 * el.clientWidth, (1 - p.y) / 2 * el.clientHeight]; })()`);
    const capPick = String(await js(`const pr = await window.__marcam.tree.pickAt(${at[0]}, ${at[1]}); pr ? pr.id + ' ' + pr.shape : 'none'`));
    await js("document.getElementById('pickFaces').click(); true");
    await sleep(400);
    const faces = await js("JSON.stringify(JSON.parse(document.getElementById('job').value).setups[0].operations.find(o => o.id === 'op5').geometry || [])");
    await js("document.getElementById('generate').click(); true");
    await waitDone("generation with the picked cap", 240000);
    const far = await js("(() => { const p = window.__marcam.state.result.programs.find(q => q.name === 'plate_2_dome').toolpath.points; let m = 0; for (let i = 0; i < p.length; i += 4) if (p[i + 3] === 3) m = Math.max(m, Math.hypot(p[i] - 46, p[i + 1] - 20)); return m; })()");
    const okTabs = await js("[...document.querySelectorAll('#tabs button')].every(b => b.textContent.endsWith('OK'))");
    check("picking the dome's cap keeps its raster on the cap (cuts within the cap + the tool radius)", capPick.startsWith("f049d19c2") && /f049d19c2/.test(faces) && far <= 6 + 1.5875 + 0.05 && okTabs, `picked ${capPick}; geometry ${faces}; farthest cut ${far.toFixed(3)} mm from the dome's axis`);
    await shot("13b_pick_dome_cap");
    // back to the whole model (the editor may have closed when generating)
    if (!(await js("!!document.getElementById('clearFaces')"))) {
      await js("document.querySelector('#tree .tree-op[data-op=\"op5\"] [data-act=\"select\"]').click(); true");
    }
    await waitFor(() => js("!!document.getElementById('clearFaces')"), "the clear button", 15000);
    await js("document.getElementById('clearFaces').click(); true");
    await waitFor(() => js("!JSON.parse(document.getElementById('job').value).setups[0].operations.find(o => o.id === 'op5').geometry"), "the selection cleared", 15000);
  }
  // 8h. WebGPU stock simulation against the CPU (WASM): Simulate and playback, rotary and
  // 3-axis; indexed jobs and "GPU off" fall back and say so
  const webgpu = String(await js("const adp = navigator.gpu ? await navigator.gpu.requestAdapter() : null; String(!!adp)")) === "true";
  const simulateWith = async (gpuOn) => {
    await js(`document.getElementById('simGpu').checked = ${gpuOn}; window.__marcam.state.sim = null; document.getElementById('simulate').click(); true`);
    await waitFor(() => js("!!window.__marcam.state.sim && document.body.dataset.state !== 'busy'"), "the simulation", 240000);
    await settle();
    return js("window.__marcam.state.simBackend");
  };
  const keepStocks = (name) => js(`window.__${name} = window.__marcam.state.sim.stocks.map(s => ({ h: Array.from(s.r || s.z), g: s.max_gouge, l: s.max_left, f: s.feature_cells, rh: s.rapid_hits })); true`);
  const compareStocks = () => js("let md = 0; window.__gpuS.forEach((s, p) => s.h.forEach((v, k) => { md = Math.max(md, Math.abs(v - window.__cpuS[p].h[k])); })); JSON.stringify({ md, gpu: window.__gpuS.map(s => [s.l.toFixed(3), s.rh, s.f]), cpu: window.__cpuS.map(s => [s.l.toFixed(3), s.rh, s.f]) })");
  for (const [ex, label] of [["knob", "rotary knob"], ["plate", "3-axis plate"]]) {
    await js(`document.getElementById('example').value = '${ex}'; document.getElementById('loadExample').click(); true`);
    await settle();
    await sleep(1500);
    await js("document.getElementById('generate').click()");
    await waitDone(`${ex} generation for the GPU check`, 240000);
    const bg = await simulateWith(true);
    await keepStocks("gpuS");
    const bc = await simulateWith(false);
    await keepStocks("cpuS");
    const cmp = JSON.parse(await compareStocks());
    if (webgpu) {
      check(`WebGPU Simulate matches the CPU within 2 um (${label})`, String(bg).startsWith("GPU") && bc === "CPU (GPU off)" && cmp.md <= 0.002 && JSON.stringify(cmp.gpu) === JSON.stringify(cmp.cpu), `${bg} vs ${bc}; max difference ${cmp.md.toFixed(4)} mm; ${JSON.stringify(cmp.gpu)}`);
    } else {
      check(`no WebGPU here: Simulate falls back to the CPU and says why (${label})`, String(bg).startsWith("CPU (") && cmp.md === 0, `${bg}`);
    }
  }
  // playback on the GPU ends where the batch simulation does (the plate, Normal grid)
  await js("document.getElementById('simGpu').checked = true; document.getElementById('simOpen').click(); true");
  await waitFor(() => js("window.__marcam.state.machining.open && !!window.__marcam.state.machining.stockMesh"), "the plate machining view", 60000);
  const pn = await js("window.__marcam.state.machining.session.segments");
  await js(`document.getElementById('simScrub').value = '${pn}'; document.getElementById('simScrub').dispatchEvent(new Event('input')); true`);
  await waitFor(() => js(`window.__marcam.state.machining.pos === ${pn} && !window.__marcam.state.machining.busy`), "the playback to the end", 120000);
  const pb = JSON.parse(await js("const mm = window.__marcam.state.machining; let md2 = 0; const last = window.__cpuS[window.__cpuS.length - 1].h; mm.r.forEach((v, k) => { md2 = Math.max(md2, Math.abs(v - last[k])); }); JSON.stringify({ md: md2, backend: mm.backend, label: document.getElementById('simLabel').textContent })"));
  await js("document.getElementById('simClose').click(); true");
  check(webgpu ? "WebGPU playback ends at the CPU's stock within 2 um (label says GPU)" : "no WebGPU: playback runs on the CPU and says so", webgpu ? pb.backend.startsWith("GPU") && pb.label.endsWith("GPU") && pb.md <= 0.002 : pb.backend.startsWith("CPU") && pb.md <= 1e-6, `${pb.backend}; max difference ${pb.md.toFixed(4)} mm; ${pb.label}`);
  // an indexed job: the engine keeps it on the CPU, with the reason shown
  await js("document.getElementById('example').value = 'knob_indexed'; document.getElementById('loadExample').click(); true");
  await settle();
  await sleep(1500);
  await js("document.getElementById('generate').click()");
  await waitDone("indexed knob generation for the GPU check", 240000);
  const bi = await simulateWith(true);
  check("indexed 3+1 jobs fall back to the CPU and say why", webgpu ? /^CPU \(indexed 3\+1/.test(bi) : String(bi).startsWith("CPU ("), bi);
  await js("document.getElementById('simGpu').checked = true; document.getElementById('simGpu').dispatchEvent(new Event('change')); true");

  // 8d. relief plaque from a grayscale image: roughing + 45 deg parallel finish, simulated
  await js("document.getElementById('example').value = 'plaque'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.getElementById('meshInfo').textContent.includes('plaque.png')"), "the plaque image", 15000);
  await js("document.getElementById('generate').click()");
  await waitDone("plaque generation", 240000);
  const qtabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  check("plaque: roughing, finishing and engraving programs verified OK", qtabs.length === 3 && qtabs.every((t) => t.endsWith("OK")), qtabs.join(", "));
  await js("document.getElementById('simulate').click()");
  await waitDone("plaque simulation", 240000);
  await js("document.querySelectorAll('#tabs button')[1].click()");
  await sleep(1500);
  const qsim = await js("document.getElementById('summary').innerText.split('\\n').pop()");
  const qmean = parseFloat((qsim.match(/mean ([0-9.]+)/) || [])[1]);
  check("plaque simulated: finished to the relief, no gouge, no rapids through stock", /deepest gouge 0\.00\d mm; 0 rapid/.test(qsim) && qmean < 0.05, qsim);
  await shot("11_plaque_simulated");

  // 8e. wrapped engraving on the Z1: grooves, text round the bar and a V-carved band
  await js("document.getElementById('example').value = 'ring'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.querySelectorAll('#tree .tree-op').length === 3"), "the ring in the tree", 15000);
  await js("document.getElementById('generate').click()");
  await waitDone("ring generation", 240000);
  const rtabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  await js("document.getElementById('simulate').click()");
  await waitDone("ring simulation", 240000);
  const rcut = await js("(() => { const s = window.__marcam.state.sim; const g = s.grid; const r = s.stocks[0].r; const at = (x, deg) => r[Math.floor((x - g.x0) / g.dx) * g.na + Math.floor(deg / 360 * g.na) % g.na]; return [at(-31, 90), at(-40, 200), at(-25, 90)].map(v => v.toFixed(2)).join(','); })()");
  check("ring: wrapped engraving verified and simulated (diamond floor, groove all round, bar elsewhere)", rtabs.length === 1 && rtabs[0].endsWith("OK") && /^10\.6\d,11\.7\d,12\.00$/.test(rcut), `${rtabs.join(", ")}; ${rcut}`);
  const rsum = await js("document.getElementById('summary').innerText.split('\\n').pop()");
  check("ring: intended engraving is not reported as a gouge", /deepest gouge 0\.0\d\d mm/.test(rsum) && /cut below the part on purpose/.test(rsum), rsum);
  await sleep(1000);
  await shot("12_ring_engraved");

  // 8c. indexed 3+1 on the knob: D-flat at A0, cross hole at A90, simulated on the bar
  await js("document.getElementById('machine').value = 'makera-z1-4axis'; document.getElementById('example').value = 'knob_indexed'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.querySelectorAll('#tree .tree-op').length === 5"), "the indexed knob in the tree", 15000);
  await js("document.getElementById('generate').click()");
  await waitDone("indexed knob generation", 240000);
  const itabs = await js("[...document.querySelectorAll('#tabs button')].map(b => b.textContent)");
  const igold = await js("window.__marcam.state.result.programs.slice(0, 2).map(p => p.gcode)");
  check("indexed knob: three programs OK, turning and knurl still the goldens", itabs.length === 3 && itabs.every((t) => t.endsWith("OK")) && igold[0] === golden("knob_1_profile") && igold[1] === golden("knob_2_knurl"), itabs.join(", "));
  await js("document.getElementById('simulate').click()");
  await waitDone("indexed knob simulation", 240000);
  const iflat = await js("(() => { const s = window.__marcam.state.sim; const g = s.grid; const r = s.stocks[2].r; const at = (x, deg) => r[Math.floor((x - g.x0) / g.dx) * g.na + Math.floor(deg / 360 * g.na) % g.na]; return [at(-20.05, 90), at(-20.05, 270), at(-8.05, 0.5)].map(v => v.toFixed(2)).join(','); })()");
  check("indexed knob simulated: flat at R2.5 on top, round below, hole to R6", iflat === "2.50,3.00,6.00", iflat);
  await js("document.querySelectorAll('#tabs button')[2].click()");
  await sleep(1500);
  await shot("10_knob_indexed");
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await settle();

  async function drag(id, dx, dy) {
    const [x, y, w, h] = await rect(id);
    const cx = x + w / 2, cy = y + h / 2;
    await S("Input.dispatchMouseEvent", { type: "mousePressed", x: cx, y: cy, button: "left", buttons: 1, clickCount: 1 });
    for (let k = 1; k <= 5; k++) await S("Input.dispatchMouseEvent", { type: "mouseMoved", x: cx + (dx * k) / 5, y: cy + (dy * k) / 5, button: "left", buttons: 1 });
    await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: cx + dx, y: cy + dy, button: "left", buttons: 0, clickCount: 1 });
    await sleep(400); // ResizeObservers
  }
  const L0 = (await rect("left"))[2], R0 = (await rect("right"))[2];
  await drag("splitLeft", 120, 0);
  const L1 = (await rect("left"))[2], R1 = (await rect("right"))[2];
  check("dragging the left bar resizes both panes", Math.abs(L1 - L0 - 120) <= 3 && Math.abs(R0 - R1 - 120) <= 3, `left ${L0}->${L1}, right ${R0}->${R1}`);
  const c0 = await js("window.__marcam.state.three.renderer.domElement.clientWidth");
  const code0 = (await rect("codeWrap"))[2];
  await drag("splitCode", 100, 0);
  const code1 = (await rect("codeWrap"))[2];
  const c1 = await js("window.__marcam.state.three.renderer.domElement.clientWidth");
  const v3 = (await rect("view3d"))[2];
  check("dragging the G-code bar resizes the 3D canvas with it", Math.abs(code0 - code1 - 100) <= 3 && Math.abs(c1 - c0 - 100) <= 3 && Math.abs(c1 - v3) <= 2, `G-code ${code0}->${code1}, canvas ${c0}->${c1} (view ${v3})`);
  const s0 = (await rect("side"))[3];
  await drag("split3d", 0, -100);
  const s1 = (await rect("side"))[3];
  const sideBuf = await js("document.getElementById('side').height / (window.devicePixelRatio || 1)");
  check("dragging the 3D/side bar grows the side view, and its canvas follows", Math.abs(s1 - s0 - 100) <= 3 && Math.abs(sideBuf - s1) <= 2, `side ${s0}->${s1}, canvas ${sideBuf}`);
  await js("document.getElementById('libraryPanel').open = true; true");
  const tw0 = (await rect("toolTableWrap"))[3];
  await drag("splitTools", 0, 80);
  const tw1 = (await rect("toolTableWrap"))[3];
  check("dragging the tool list bar resizes the tool list", Math.abs(tw1 - tw0 - 80) <= 3, `${tw0}->${tw1}`);
  await js("document.getElementById('splitLeft').focus(); true");
  await S("Input.dispatchKeyEvent", { type: "keyDown", key: "ArrowRight", code: "ArrowRight", windowsVirtualKeyCode: 39 });
  await S("Input.dispatchKeyEvent", { type: "keyUp", key: "ArrowRight", code: "ArrowRight", windowsVirtualKeyCode: 39 });
  await sleep(100);
  const L2 = (await rect("left"))[2];
  check("arrow keys move a focused bar", L2 === L1 + 10, `${L1}->${L2}`);
  await S("Page.reload");
  await waitFor(async () => (await state()) === "ready", "reload", 30000);
  await sleep(500);
  const sizes = [(await rect("left"))[2], (await rect("codeWrap"))[2], (await rect("side"))[3]];
  check("pane sizes survive a reload", sizes[0] === L2 && Math.abs(sizes[1] - code1) <= 3 && sizes[2] === s1, `left ${sizes[0]}, G-code ${sizes[1]}, side ${sizes[2]}`);
  await drag("splitLeft", -2000, 0);
  await drag("split3d", 0, 2000);
  const mins = [(await rect("left"))[2], (await rect("side"))[3], (await rect("view3d"))[3]];
  check("minimum sizes hold", mins[0] === 300 && mins[1] === 80 && mins[2] >= 250, `left ${mins[0]}, side ${mins[1]}, 3D ${mins[2]}`);
  await shot("12_resized_panes");
  for (const id of ["splitLeft", "splitCode", "split3d", "splitTools"]) {
    const [x, y, w, h] = await rect(id);
    await S("Input.dispatchMouseEvent", { type: "mousePressed", x: x + w / 2, y: y + h / 2, button: "left", buttons: 1, clickCount: 1 });
    await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: x + w / 2, y: y + h / 2, button: "left", buttons: 0, clickCount: 1 });
    await S("Input.dispatchMouseEvent", { type: "mousePressed", x: x + w / 2, y: y + h / 2, button: "left", buttons: 1, clickCount: 2 });
    await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: x + w / 2, y: y + h / 2, button: "left", buttons: 0, clickCount: 2 });
  }
  await sleep(300);
  const reset = [(await rect("left"))[2], (await rect("side"))[3]];
  check("double-click resets bars to the default layout", reset[0] === L0 && reset[1] === s0, `left ${reset[0]} (default ${L0}), side ${reset[1]} (default ${s0})`);

  // 8j. project workflow and setups: New (the setup dialog) -> Import STL (binary, ASCII by
  // drag-and-drop, inch) -> every view refreshed, stock sized, defaults filled -> Generate ->
  // Save -> New -> Open the saved file -> the same G-code; each setup type from New; editing a
  // setup's type converts it (and Undo); adding a setup; unsaved-changes warnings; bad files
  {
    await S("Emulation.setDeviceMetricsOverride", { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });
    const knobStl = readFileSync(join(root, "examples", "knob", "knob.stl"));
    // the same triangles as ASCII, and scaled to inches (binary)
    const n = knobStl.readUInt32LE(80);
    let ascii = "solid knob\n";
    const inch = Buffer.alloc(84 + 50 * n);
    inch.writeUInt32LE(n, 80);
    for (let t = 0; t < n; t++) {
      const o = 84 + 50 * t;
      ascii += " facet normal 0 0 0\n  outer loop\n";
      for (let k = 0; k < 3; k++) {
        const v = [0, 1, 2].map((c) => knobStl.readFloatLE(o + 12 + 12 * k + 4 * c));
        ascii += `   vertex ${v.join(" ")}\n`;
        v.forEach((x, c) => inch.writeFloatLE(x / 25.4, o + 12 + 12 * k + 4 * c));
      }
      ascii += "  endloop\n endfacet\n";
    }
    ascii += "endsolid knob\n";
    const fileJs = (name, buf) => `new File([Uint8Array.from(atob("${Buffer.from(buf).toString("base64")}"), (c) => c.charCodeAt(0))], ${JSON.stringify(name)})`;
    const setFile = (input, name, buf) => js(`(() => { const dt = new DataTransfer(); dt.items.add(${fileJs(name, buf)}); const i = document.getElementById('${input}'); i.files = dt.files; i.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
    const drop = (name, buf) => js(`(() => { const dt = new DataTransfer(); dt.items.add(${fileJs(name, buf)}); document.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true })); document.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true })); return true; })()`);
    const dialogs = () => cdp.dialogs.map((d) => `${d.type}: ${d.message}`);
    const allGcode = () => js("window.__marcam.state.result.programs.map((p) => p.name + '\\n' + p.gcode).join('\\n')");
    const viewsOk = () => js(`(() => { const v = window.__marcam.state.three; const s = document.getElementById('side'); const px = s.getContext('2d').getImageData(0, 0, s.width, s.height).data; let ink = 0; for (let i = 3; i < px.length; i += 4) if (px[i]) ink++;
      return { part: v.groups.part.children.length, stock: v.groups.stock.children.length, path: v.groups.path.children.length, side: ink, tabs: [...document.querySelectorAll('#tabs button')].map((b) => b.textContent), ops: document.querySelectorAll('#tree .tree-op').length, stale: document.body.classList.contains('stale') }; })()`);
    const dialogOpen = () => waitFor(() => js("document.getElementById('setupDialog').open"), "the setup dialog", 15000);
    // fill the setup dialog: { kind, units, axis, name, file: [name, bytes] }, then OK
    async function inDialog(o = {}) {
      await dialogOpen();
      if (o.name !== undefined) await js(`document.getElementById('sdName').value = ${JSON.stringify(o.name)}; true`);
      if (o.kind) await js(`document.querySelector('.sd-kind[data-kind="${o.kind}"]').click(); true`);
      if (o.units) await js(`const u = document.getElementById('sdUnits'); u.value = '${o.units}'; u.dispatchEvent(new Event('change', { bubbles: true })); true`);
      if (o.axis) await js(`const a = document.getElementById('sdAxis'); a.value = '${o.axis}'; a.dispatchEvent(new Event('change', { bubbles: true })); true`);
      if (o.file) {
        await setFile("sdFile", o.file[0], o.file[1]);
        await waitFor(() => js("document.getElementById('sdFileName').textContent.length > 0"), "the STL in the dialog", 15000);
      }
      await sleep(200);
      if (o.shot) await shot(o.shot);
      const info = await js("document.getElementById('sdInfo').textContent");
      if (o.cancel) await js("document.getElementById('sdCancel').click(); true");
      else await js("document.getElementById('sdOk').click(); true");
      return info;
    }
    const doc = async () => JSON.parse(await js("document.getElementById('job').value"));
    const emptyReady = () => waitFor(() => js("!!document.getElementById('emptyState') && document.body.dataset.state === 'ready'"), "the empty project", 15000);

    // New, from a project with unsaved edits: asked first; declining keeps it
    await js("document.getElementById('job').dispatchEvent(new Event('input')); true");
    const dirty0 = await js("!document.getElementById('projectDirty').classList.contains('hidden')");
    cdp.dialogAnswer = false;
    await js("document.getElementById('projectNew').click(); true");
    await sleep(300);
    const kept = await js("document.querySelectorAll('#tree .tree-op').length");
    const noDialog = !(await js("document.getElementById('setupDialog').open"));
    cdp.dialogAnswer = true;
    check("unsaved changes are marked, and New asks before discarding them (declined: nothing lost)", dirty0 && /unsaved changes/.test(dialogs().pop() || "") && kept > 0 && noDialog, `${dialogs().slice(-1)}; ${kept} operations kept`);

    // New: the setup dialog (name, machine, setup type); without an STL, an empty project
    await js("document.getElementById('projectNew').click(); true");
    await dialogOpen();
    const choices = await js("[...document.querySelectorAll('.sd-kind')].map((k) => k.querySelector('b').textContent + ': ' + k.querySelector('small').textContent.slice(0, 30))");
    await inDialog({ name: "bracket", kind: "vise", shot: "15a_new_dialog" });
    await emptyReady();
    const empty = await js("({ name: document.getElementById('projectName').textContent, ops: document.querySelectorAll('#tree .tree-op').length, tabs: document.querySelectorAll('#tabs button').length, gcode: document.getElementById('gcode').value.length, cta: document.getElementById('emptyState').textContent })");
    // nothing to plan: Generate is disabled and says why
    const nothing = await js("document.getElementById('generate').disabled ? document.getElementById('generate').title : 'enabled'");
    check("New: a dialog with the three setup types; without an STL, an empty project with a call to action",
      choices.length === 3 && /Rotary 4th axis/.test(choices[0]) && /Indexed 3\+1/.test(choices[1]) && /3-axis vise/.test(choices[2]) && empty.name === "bracket" && empty.ops === 0 && empty.tabs === 0 && /Import an STL/i.test(empty.cta) && /Import an STL first/.test(nothing),
      `${choices.join(" | ")}; ${JSON.stringify(empty)}`);
    await shot("14a_new_project");
    // the type chosen in New is pre-selected when the STL comes
    await setFile("importStlFile", "dome.stl", readFileSync(join(root, "examples", "plate", "dome.stl")));
    await dialogOpen();
    const pre = await js("document.querySelector('input[name=\"sdKind\"]:checked').value");
    await inDialog({});
    await waitDone("generation after importing into the new vise project", 240000);
    const dv = await doc();
    check("Import STL into it: the dialog keeps the vise chosen in New; block stock, 3-axis operations generated",
      pre === "vise" && dv.setups[0].kind === "vise" && dv.setups[0].stock.shape === "box" && dv.setups[0].operations.map((o) => o.type).join() === "rough3d,raster3d" && (await viewsOk()).tabs.every((t) => t.endsWith("OK")),
      `${pre}; ${JSON.stringify(dv.setups[0].stock)}`);

    // each setup type, from New with the STL chosen in the dialog
    const byKind = {};
    for (const [kind, ops] of [["rotary", "rotary_rough,rotary_finish"], ["indexed", "rough3d,rough3d,raster3d,raster3d"], ["vise", "rough3d,raster3d"]]) {
      await js("document.getElementById('projectNew').click(); true");
      const info = await inDialog({ name: `knob_${kind}`, kind, axis: kind === "vise" ? "x" : "x", file: ["knob.stl", knobStl], shot: `15b_new_${kind}` });
      await waitDone(`generation of a new ${kind} project`, 240000);
      const d = await doc();
      const v = await viewsOk();
      const offered = await js("[...document.getElementById('treeAddType').options].map((o) => o.value)");
      byKind[kind] = { d, offered };
      const fitsOffered = kind === "rotary" ? !offered.includes("pocket") && offered.includes("rotary_finish")
        : kind === "indexed" ? offered.includes("pocket") && !offered.includes("rotary_finish") && !offered.includes("helical_knurl")
        : offered.includes("pocket") && !offered.includes("rotary_finish");
      check(`New ${kind} project with the STL: placed, ${kind === "vise" ? "block" : "bar"} stock, ${ops.split(",").length} starter operations, generated, only fitting types offered`,
        d.setups[0].kind === kind && d.setups[0].operations.map((o) => o.type).join() === ops && (kind !== "indexed" || d.setups[0].operations.map((o) => o.index_a).join() === "0,180,0,180") && v.tabs.length >= 2 && v.tabs.every((t) => t.endsWith("OK")) && v.part > 0 && v.path > 0 && fitsOffered,
        `${info}; ${d.setups[0].operations.map((o) => o.type + (o.index_a !== undefined ? "@" + o.index_a : "")).join(",")}; tabs ${v.tabs}; offered ${offered.length}`);
      await shot(`15c_${kind}_generated`);
    }

    // a rotary project: import, save, reopen (the earlier round trip), then change its type
    await js("document.getElementById('projectNew').click(); true");
    await inDialog({ name: "knob", kind: "rotary" });
    await emptyReady();
    await setFile("importStlFile", "knob.stl", knobStl);
    await inDialog({});
    await waitDone("generation after the STL import", 240000);
    const info1 = await js("document.getElementById('meshInfo').textContent");
    const doc1 = await doc();
    const v1 = await viewsOk();
    check("Import STL (binary): part size shown, bar stock sized round it, the part on the axis with its front at X0",
      /binary STL, 10080 triangles/.test(info1) && /21\.70 x 20\.00 x 20\.00 mm/.test(info1) && JSON.stringify(doc1.setups[0].stock) === '{"shape":"cylinder","radius":10.5,"length":37}' && doc1.models[0].axis === "x" && Math.abs(doc1.models[0].translate[0]) < 1e-6, `${info1}; ${JSON.stringify(doc1.setups[0].stock)}`);
    check("Import STL: starter roughing and finishing filled by the engine's defaults, generated, every view refreshed",
      v1.ops === 2 && v1.tabs.length === 2 && v1.tabs.every((t) => t.endsWith("OK")) && v1.part > 0 && v1.stock > 0 && v1.path > 0 && v1.side > 1000 && !v1.stale && doc1.setups[0].operations.every((o) => o.type && o.feed > 0),
      JSON.stringify(v1));
    const pickRegions = await js("(await window.__marcam.engine('faceRegions', JSON.parse(document.getElementById('job').value), 'part', window.__marcam.state.mesh)).result.regions.length");
    check("Import STL: the new part's faces are there to pick", pickRegions === 8, `${pickRegions} face regions`);
    await shot("14b_imported_stl");
    const gcode1 = await allGcode();
    const name1 = await js("document.getElementById('projectName').textContent");

    // an edit marks the results stale until the next Generate
    await js("const f = document.querySelector('#quick input'); if (f) { f.value = String(+f.value + 0.01); f.dispatchEvent(new Event('change')); } true");
    await sleep(300);
    const stale = await js("({ body: document.body.classList.contains('stale'), banner: !!document.getElementById('staleBanner') })");
    check("an edit marks the results out of date (banner, dimmed views) until Generate", stale.body && stale.banner, JSON.stringify(stale));
    await shot("14c_stale_results");
    await js("document.getElementById('staleRegen').click(); true");
    await waitDone("regeneration", 240000);
    check("Generate again clears the stale mark", !(await js("document.body.classList.contains('stale')")));
    const gcode2 = await allGcode();

    // Save (asks for a name, downloads one file with the mesh in it), then New, then Open it
    const before = new Set(readdirSync(downloads));
    await js("document.getElementById('projectSave').click(); true");
    const saved = await waitFor(() => readdirSync(downloads).find((f) => !before.has(f) && f.endsWith(".marcam.json")), "the saved project", 15000);
    const savedText = readFileSync(join(downloads, saved), "utf8");
    const savedObj = JSON.parse(savedText);
    const clean1 = await js("document.getElementById('projectDirty').classList.contains('hidden')");
    check("Save: one .marcam.json with the project and its STL embedded; the unsaved mark goes",
      saved === `${name1}.marcam.json` && savedObj.format === "marcam-project" && Object.keys(savedObj.meshes)[0] === "knob.stl" && Buffer.from(savedObj.meshes["knob.stl"], "base64").length === 84 + 50 * 10080 && clean1 && savedObj.project.setups[0].kind === "rotary",
      `${saved}; ${Object.keys(savedObj.meshes || {})}`);
    await js("document.getElementById('projectNew').click(); true");
    await inDialog({ cancel: true });
    const dialogsBefore = cdp.dialogs.length;
    await setFile("projectOpenFile", saved, Buffer.from(savedText));
    await waitFor(() => js("document.body.dataset.state === 'ready' && !!document.getElementById('emptyGenerate')"), "the opened project", 30000);
    const openedPlans = await js("window.__marcam.calls.planAndPost || 0");
    await js("document.getElementById('emptyGenerate').click(); true");
    await waitDone("generation after opening the project", 240000);
    check("Open plans nothing until asked: an empty state with Generate, then one plan", (await js("window.__marcam.calls.planAndPost")) === openedPlans + 1);
    const reopened = await allGcode();
    const v2 = await viewsOk();
    check("Open: the saved project comes back whole (no question asked, it was saved) and its G-code is byte-identical",
      reopened === gcode2 && v2.ops === 2 && v2.part > 0 && cdp.dialogs.length === dialogsBefore && (await js("document.getElementById('projectName').textContent")) === name1,
      `${reopened.length} chars vs ${gcode2.length}; dialogs ${cdp.dialogs.length - dialogsBefore}`);
    await shot("14d_reopened");

    // Edit setup: rotary -> 3-axis vise converts it (warned), regenerates, and can be undone
    await js("document.getElementById('treeEditSetup').click(); true");
    await dialogOpen();
    const editPre = await js("({ kind: document.querySelector('input[name=\"sdKind\"]:checked').value, machine: document.getElementById('sdMachine').value, title: document.getElementById('sdTitle').textContent })");
    const nDialogs = cdp.dialogs.length;
    await inDialog({ kind: "vise", shot: "15d_edit_setup_dialog" });
    await waitFor(() => js("document.body.dataset.state === 'ready'"), "the converted setup", 60000);
    const staleAfterEdit = await js("({ stale: document.body.classList.contains('stale'), attention: document.getElementById('generate').classList.contains('attention') })");
    const convertMsg = await js("document.getElementById('error').textContent");
    check("a setup change marks the results out of date and highlights Generate (nothing planned yet)", staleAfterEdit.stale && staleAfterEdit.attention, JSON.stringify(staleAfterEdit));
    await js("document.getElementById('generate').click(); true");
    await waitDone("regeneration after the type change", 240000);
    const warned = cdp.dialogs.slice(nDialogs).map((d) => d.message).join(" ");
    const dc = await doc();
    const vc = await viewsOk();
    const undoShown = await js("!document.getElementById('undoBar').classList.contains('hidden')");
    const offeredC = await js("[...document.getElementById('treeAddType').options].map((o) => o.value)");
    check("Edit setup: rotary to 3-axis vise warns, removes the rotary operations, adds 3-axis ones, re-places the part in a block and regenerates",
      editPre.kind === "rotary" && /removes op1 \(rotary_rough\), op2 \(rotary_finish\)/.test(warned) && dc.setups[0].kind === "vise" && dc.setups[0].stock.shape === "box" && dc.setups[0].operations.map((o) => o.type).join() === "rough3d,raster3d" && dc.models.find((m) => m.kind === "mesh").axis === "x" && vc.tabs.every((t) => t.endsWith("OK")) && vc.part > 0 && undoShown && !offeredC.includes("rotary_finish") && /removed/.test(convertMsg),
      `${JSON.stringify(editPre)}; ${warned.slice(0, 120)}; ${dc.setups[0].operations.map((o) => o.type)}; ${JSON.stringify(dc.setups[0].stock)}; axis ${dc.models.find((m) => m.kind === "mesh").axis}; tabs ${vc.tabs}; part ${vc.part}; undo ${undoShown}; error ${(await js("document.getElementById('error').textContent")).slice(0, 80)}`);
    await shot("15e_converted_to_vise");
    await js("document.getElementById('undoSetup').click(); true");
    await waitFor(() => js("document.body.dataset.state === 'ready'"), "the undone setup", 60000);
    await js("document.getElementById('generate').click(); true");
    await waitDone("regeneration after Undo", 240000);
    const du = await doc();
    check("Undo brings the rotary setup back, regenerated", du.setups[0].kind === "rotary" && du.setups[0].operations.map((o) => o.type).join() === "rotary_rough,rotary_finish" && (await allGcode()) === gcode2,
      du.setups[0].operations.map((o) => o.type).join());

    // + Setup: a second setup (indexed 3+1) for the same part, its own placement
    await js("document.getElementById('treeAddSetup').click(); true");
    await inDialog({ kind: "indexed", name: "Flats", shot: "15f_add_setup_dialog" });
    await waitFor(() => js("document.body.dataset.state === 'ready'"), "the added setup", 60000);
    await js("document.getElementById('generate').click(); true");
    await waitDone("generation of the added setup", 240000);
    const da = await doc();
    const va = await viewsOk();
    check("+ Setup: an indexed 3+1 setup joins the project with its own part placement and starters, generated",
      da.setups.length === 2 && da.setups[1].kind === "indexed" && da.setups[1].name === "Flats" && da.setups[1].part[0] !== da.setups[0].part[0] && da.setups[1].operations.length === 4 && (await js("document.getElementById('treeSetup').options.length")) === 2 && va.tabs.every((t) => t.endsWith("OK")),
      `${da.setups.map((s) => s.id + ":" + s.kind + ":" + s.operations.length).join(", ")}; tabs ${va.tabs}`);
    await shot("15g_two_setups");

    // ASCII STL dropped on the page, and an inch STL: the same part
    await js("document.getElementById('projectNew').click(); true");
    await inDialog({ name: "untitled", kind: "rotary" });
    await emptyReady();
    await drop("knob_ascii.stl", Buffer.from(ascii));
    await inDialog({});
    await waitDone("generation after dropping an ASCII STL", 240000);
    const infoA = await js("document.getElementById('meshInfo').textContent");
    const gA = await allGcode();
    check("drag-and-drop an ASCII STL: imported and generated like the binary one", /ASCII STL, 10080 triangles/.test(infoA) && /21\.70 x 20\.00 x 20\.00 mm/.test(infoA) && gA.replace(/knob_ascii/g, "knob") === gcode1, infoA);
    await shot("14e_ascii_dropped");
    await js("document.getElementById('projectNew').click(); true");
    await inDialog({ name: "untitled", kind: "rotary" });
    await emptyReady();
    await setFile("importStlFile", "knob_inch.stl", inch);
    await inDialog({ units: "inch" });
    await waitDone("generation after an inch STL", 240000);
    const infoI = await js("document.getElementById('meshInfo').textContent");
    check("an inch STL with units = inch comes in at the same size in mm", /inch/.test(infoI) && /21\.70 x 20\.00 x 20\.00 mm/.test(infoI), infoI);

    // a second STL into a project with a part: the dialog (type pre-selected), then replace or add
    await setFile("importStlFile", "knob.stl", knobStl);
    await dialogOpen();
    const preI = await js("document.querySelector('input[name=\"sdKind\"]:checked').value");
    await inDialog({});
    await waitDone("generation after replacing the part", 240000);
    const asked = dialogs().pop() || "";
    check("importing into a project with a part: the setup type pre-selected, then replace (OK) or add (Cancel)", preI === "rotary" && /Replace the part with knob\.stl/.test(asked), `${preI}; ${asked}`);

    // bad files: a clear error, nothing replaced
    const opsBefore = await js("document.querySelectorAll('#tree .tree-op').length");
    await setFile("importStlFile", "notes.stl", Buffer.from("these are not triangles"));
    await sleep(500);
    const badStl = await js("document.getElementById('error').textContent");
    await setFile("projectOpenFile", "broken.marcam.json", Buffer.from("{ not json"));
    await sleep(500);
    const badJson = await js("document.getElementById('error').textContent");
    check("bad files: a clear error and the project left as it was",
      /notes\.stl: not an STL file/.test(badStl) && /broken\.marcam\.json: not a marcam project/.test(badJson) && (await js("document.querySelectorAll('#tree .tree-op').length")) === opsBefore && !(await js("document.getElementById('setupDialog').open")),
      `${badStl} | ${badJson}`);
    await shot("14g_bad_file");

    // full reset (Mar: "after a new project, the old model is still hanging around"): New ->
    // Import A -> Generate (+ playback, face picking) -> New: nothing of A left; Import B:
    // only B; New three times: the scene and the GPU memory do not grow
    {
      const domeStl = readFileSync(join(root, "examples", "plate", "dome.stl"));
      const domeTris = domeStl.readUInt32LE(80);
      const scene = () => js(`(() => { const v = window.__marcam.state.three; let objs = 0, tris = 0; v.scene.traverse((o) => { if (o.geometry) { objs++; if (o.isMesh) tris += (o.geometry.index ? o.geometry.index.count : o.geometry.attributes.position.count) / 3; } });
        const g = v.groups; const kids = (k) => g[k] ? g[k].children.length : 0;
        return { objs, tris, part: kids('part'), stock: kids('stock'), path: kids('path'), sim: kids('sim'), tool: kids('tool'), machining: !!g.machining, pickMesh: !!window.__marcam.state.tree.pickMesh,
          geometries: v.renderer.info.memory.geometries, textures: v.renderer.info.memory.textures,
          partTris: g.part.children.reduce((n, m) => n + (m.geometry.index ? m.geometry.index.count : m.geometry.attributes.position.count) / 3, 0),
          gcode: document.getElementById('gcode').value.length, tabs: document.querySelectorAll('#tabs button').length, ops: document.querySelectorAll('#tree .tree-op').length,
          mesh: !!window.__marcam.state.mesh, meshInfo: document.getElementById('meshInfo').textContent, result: !!window.__marcam.state.result, sim: !!window.__marcam.state.sim,
          playback: !!window.__marcam.state.machining.open, session: !!window.__marcam.state.machining.session, undo: !document.getElementById('undoBar').classList.contains('hidden'),
          error: document.getElementById('error').textContent, side: (() => { const c = document.getElementById('side'); const px = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let ink = 0; for (let i = 3; i < px.length; i += 4) if (px[i]) ink++; return ink; })() }; })()`);
      const workerSession = () => js("await window.__marcam.engine('simSeek', 1).then((r) => !(r.result && r.result.error), () => false)");
      const newEmpty = async () => {
        await js("document.getElementById('projectNew').click(); true");
        await inDialog({ name: "untitled", kind: "rotary" });
        await emptyReady();
        await sleep(300);
        return scene();
      };
      const base = await newEmpty();
      // A: the knob, generated, with the playback open and faces being picked
      await setFile("importStlFile", "knob.stl", knobStl);
      await inDialog({});
      await waitDone("generation of part A", 240000);
      await js("document.getElementById('simOpen').click(); true");
      await waitFor(() => js("!!window.__marcam.state.machining?.open && document.body.dataset.state !== 'busy'"), "the playback of A", 120000);
      await js("document.getElementById('simClose').click(); true");
      await js("document.querySelectorAll('#tree .tree-op [data-act=\"select\"]')[1].click(); true");
      const canPick = await js("!!document.getElementById('pickFaces')");
      if (canPick) {
        await js("document.getElementById('pickFaces').click(); true");
        await waitFor(() => js("!!window.__marcam.state.tree.pickMesh"), "picking on A", 15000);
      }
      const withA = await scene();
      const sessionA = await workerSession();
      // New: nothing of A
      const afterNew = await newEmpty();
      const sessionAfter = await workerSession();
      check("full reset: after New, no part, stock, toolpaths, playback, pick highlight, G-code, tree, side view, mesh or worker session of the old project",
        withA.part > 0 && withA.path > 0 && withA.machining && withA.gcode > 0 && sessionA &&
        afterNew.objs === base.objs && afterNew.part + afterNew.stock + afterNew.path + afterNew.sim + afterNew.tool === 0 && !afterNew.machining && !afterNew.pickMesh &&
        afterNew.gcode === 0 && afterNew.tabs === 0 && afterNew.ops === 0 && !afterNew.mesh && /No mesh/.test(afterNew.meshInfo) && !afterNew.result && !afterNew.sim &&
        !afterNew.playback && !afterNew.session && !afterNew.undo && !afterNew.error && afterNew.side === 0 && !sessionAfter && afterNew.geometries <= base.geometries,
        `picking ${canPick}; with A ${JSON.stringify(withA)}; after New ${JSON.stringify(afterNew)}; base ${base.objs} objects ${base.geometries} geometries; worker session ${sessionA} -> ${sessionAfter}`);
      await shot("14h_reset_after_new");
      // B: only B's geometry and G-code
      await setFile("importStlFile", "dome.stl", domeStl);
      await inDialog({});
      await waitDone("generation of part B", 240000);
      const withB = await scene();
      const gB = await allGcode();
      check("after the reset, importing B shows only B: B's triangles, B's G-code (nothing named after A)",
        withB.partTris === domeTris && /dome\.stl/.test(withB.meshInfo) && !/knob/i.test(gB) && /dome/i.test(gB) && withB.tabs >= 2 && !withB.machining && !withB.pickMesh,
        `${withB.partTris} part triangles (B has ${domeTris}); ${withB.meshInfo}; ${withB.tabs} programs`);
      await shot("14i_only_b");
      // New three times: no growth
      const runs = [];
      for (let k = 0; k < 3; k++) runs.push(await newEmpty());
      check("New three times: the scene's objects and the renderer's geometries and textures do not grow",
        runs.every((r) => r.objs === base.objs && r.geometries <= base.geometries && r.textures <= base.textures && !r.mesh && r.gcode === 0),
        runs.map((r) => `${r.objs} objects, ${r.geometries} geometries, ${r.textures} textures`).join("; ") + ` (base ${base.objs}/${base.geometries}/${base.textures})`);
      // an example goes through the same reset
      await js("document.getElementById('example').value = 'knob_finish'; document.getElementById('loadExample').click(); true");
      await settle();
      await js("document.getElementById('generate').click(); true");
      await waitDone("the example after the resets", 240000);
      await js("document.getElementById('example').value = 'ring'; document.getElementById('loadExample').click(); true");
      await settle();
      const ex = await scene();
      // (the ring's own bar and part show at once: drawn from the project)
      check("loading an example resets too: the previous example's part, toolpaths and mesh are gone; only the ring's bar and part",
        ex.path === 0 && ex.part === 1 && ex.stock === 1 && !ex.mesh && ex.gcode === 0 && !ex.result && ex.partTris !== domeTris, JSON.stringify(ex));
    }

    // leaving with unsaved changes: the browser asks
    await js("document.getElementById('job').dispatchEvent(new Event('input')); true");
    await S("Page.reload");
    await waitFor(() => js("document.body.dataset.state === 'ready'"), "the page after reload", 30000);
    check("leaving with unsaved changes: the browser asks first", cdp.dialogs.some((d) => d.type === "beforeunload"), dialogs().slice(-1).join());
  }

  // 8k. planning only when asked: view actions plan nothing and mark nothing stale; an edit
  // marks the results out of date and waits for Generate; auto-regenerate (off by default)
  // plans once after a drag is released
  {
    const plans = () => js("window.__marcam.calls.planAndPost || 0");
    const isStale = () => js("document.body.classList.contains('stale')");
    await js("document.getElementById('example').value = 'knob_finish'; document.getElementById('loadExample').click(); true");
    await settle();
    await js("document.getElementById('generate').click()");
    await waitDone("knob mesh generation for the plan count", 240000);
    const p0 = await plans();
    const autoDefault = await js("document.getElementById('autoGen').checked");
    // orbit, pan and zoom on the 3D view (mouse drags and the wheel)
    const [vx, vy, vw, vh] = await rect("view3d");
    const drag = async (button, buttons) => {
      await S("Input.dispatchMouseEvent", { type: "mousePressed", x: vx + vw / 2, y: vy + vh / 2, button, buttons, clickCount: 1 });
      for (let k = 1; k <= 8; k++) await S("Input.dispatchMouseEvent", { type: "mouseMoved", x: vx + vw / 2 + 12 * k, y: vy + vh / 2 + 5 * k, button, buttons });
      await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: vx + vw / 2 + 96, y: vy + vh / 2 + 40, button, buttons: 0, clickCount: 1 });
    };
    await drag("left", 1);
    await drag("right", 2);
    for (let k = 0; k < 5; k++) await S("Input.dispatchMouseEvent", { type: "mouseWheel", x: vx + vw / 2, y: vy + vh / 2, deltaX: 0, deltaY: k % 2 ? 120 : -120 });
    await js("window.__marcam.setView('front', {}); window.__marcam.setView('back', {}); window.__marcam.setView('iso', {}); true");
    // layers, scrubbing the moves, switching program tabs, resizing panes, selecting in the tree
    for (const id of ["showRapids", "showStock", "showPart", "showSim"]) await js(`document.getElementById('${id}').click(); document.getElementById('${id}').click(); true`);
    for (const v of [10, 200, 2000, 0]) await js(`const s = document.getElementById('scrub'); s.value = ${v}; s.dispatchEvent(new Event('input')); true`);
    await js("document.querySelectorAll('#tabs button')[1].click(); document.querySelectorAll('#tabs button')[0].click(); true");
    const [lx, ly, lw, lh] = await rect("splitLeft");
    await S("Input.dispatchMouseEvent", { type: "mousePressed", x: lx + lw / 2, y: ly + lh / 2, button: "left", buttons: 1, clickCount: 1 });
    await S("Input.dispatchMouseEvent", { type: "mouseMoved", x: lx + lw / 2 + 60, y: ly + lh / 2, button: "left", buttons: 1 });
    await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: lx + lw / 2 + 60, y: ly + lh / 2, button: "left", buttons: 0, clickCount: 1 });
    await js("document.querySelector('#tree .tree-op [data-act=\"select\"]').click(); true");
    await sleep(300);
    await js("document.querySelector('#tree .tree-op [data-act=\"select\"]').click(); true");
    // machining playback: open, play a little, scrub, close
    await js("document.getElementById('simOpen').click(); true");
    await waitFor(() => js("!!window.__marcam.state.machining?.open && document.body.dataset.state !== 'busy'"), "the machining playback", 120000);
    await js("const s = document.getElementById('simScrub'); s.value = Math.floor(s.max / 2); s.dispatchEvent(new Event('input')); true");
    await sleep(300);
    await js("document.getElementById('simClose').click(); true");
    await sleep(800);
    const viewPlans = (await plans()) - p0;
    const viewStale = await isStale();
    const dirtyAfterView = await js("!document.getElementById('projectDirty').classList.contains('hidden')");
    check("view actions (orbit, pan, zoom, presets, layers, scrub, tabs, pane resize, tree selection, playback) plan nothing and mark nothing stale",
      viewPlans === 0 && !viewStale && !dirtyAfterView, `${viewPlans} plans; stale ${viewStale}; unsaved ${dirtyAfterView}`);
    await shot("16a_after_view_actions");
    // a colour (display only): saved, nothing stale, nothing planned
    await js("const c = document.querySelector('#tree .tree-op input[data-act=\"color\"]'); c.value = '#ff00aa'; c.dispatchEvent(new Event('change', { bubbles: true })); true");
    await sleep(600);
    check("changing an operation's colour plans nothing and leaves the results current (only unsaved)",
      (await plans()) === p0 && !(await isStale()) && (await js("!document.getElementById('projectDirty').classList.contains('hidden')")), `${(await plans()) - p0} plans; stale ${await isStale()}`);
    // a real edit: out of date, Generate highlighted, nothing planned until Generate
    await js("const f = document.querySelector('#quick input'); f.value = String(+f.value + 0.01); f.dispatchEvent(new Event('change')); true");
    await sleep(AUTO_WAIT);
    const edited = await js("({ stale: document.body.classList.contains('stale'), banner: !!document.getElementById('staleBanner'), attention: document.getElementById('generate').classList.contains('attention') })");
    check("editing a parameter marks the results out of date and highlights Generate, and plans nothing",
      autoDefault === false && edited.stale && edited.banner && edited.attention && (await plans()) === p0, `${JSON.stringify(edited)}; auto-regenerate on by default: ${autoDefault}; ${(await plans()) - p0} plans`);
    await shot("16b_out_of_date");
    await js("document.getElementById('generate').click()");
    await waitDone("generation after the edit", 240000);
    check("Generate: exactly one plan, and the results are current again", (await plans()) === p0 + 1 && !(await isStale()) && !(await js("document.getElementById('generate').classList.contains('attention')")));
    // auto-regenerate on: twenty values while the mouse is held, one plan after the release
    await js("document.getElementById('autoGen').click(); true");
    const p1 = await plans();
    const [qx, qy, qw, qh] = await js("(() => { const r = document.querySelector('#quick input').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()");
    await S("Input.dispatchMouseEvent", { type: "mousePressed", x: qx + qw / 2, y: qy + qh / 2, button: "left", buttons: 1, clickCount: 1 });
    for (let k = 1; k <= 20; k++) {
      await js(`const f = document.querySelector('#quick input'); f.value = String(+f.value + 0.001); f.dispatchEvent(new Event('change')); true`);
      await sleep(60);
    }
    await sleep(AUTO_WAIT);
    const whileHeld = (await plans()) - p1;
    await S("Input.dispatchMouseEvent", { type: "mouseReleased", x: qx + qw / 2, y: qy + qh / 2, button: "left", buttons: 0, clickCount: 1 });
    await waitFor(async () => (await plans()) > p1, "the auto-regenerate", 15000);
    await waitDone("the auto-regenerated plan", 240000);
    await sleep(AUTO_WAIT);
    const afterRelease = (await plans()) - p1;
    check("auto-regenerate: 20 edits while dragging plan nothing; one plan after the release",
      whileHeld === 0 && afterRelease === 1 && !(await isStale()), `${whileHeld} while held, ${afterRelease} after`);
    await js("document.getElementById('autoGen').click(); true");
    await shot("16c_auto_regenerated");
  }

  // 8l. picking: the bracket's detected holes and pocket start operations; the picking mode
  // lights up what an operation takes, chips list the picks, "select all similar", hover
  // labels, a click on the canvas toggles a feature, wrong features are refused with a note
  {
    await S("Emulation.setDeviceMetricsOverride", { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });
    const fx = join(root, "tests", "fixtures", "picking");
    const proj = JSON.parse(readFileSync(join(fx, "bracket.project.json"), "utf8"));
    const stl64 = readFileSync(join(fx, "bracket.stl")).toString("base64");
    const file = JSON.stringify({ format: "marcam-project", version: 1, project: proj, meshes: { "bracket.stl": stl64 } });
    cdp.dialogAnswer = true;
    await js(`window.__marcam.project.openText(${JSON.stringify(file)}, "bracket.marcam.json").then(() => true)`);
    await waitFor(() => js("document.querySelectorAll('#detected .detected-btn').length === 3"), "the detected features", 30000);
    const detectedText = await js("[...document.querySelectorAll('#detected .detected-btn')].map((b) => b.textContent)");
    check("an imported part's holes and pockets are detected, holes grouped by size (drilled with a tool of their size, else bored)",
      detectedText.includes("Drill 4 × 5.00 mm through") && detectedText.includes("Bore 1 × 6.00 mm blind") && detectedText.some((x) => /^Pocket 20\.00 x 24\.00/.test(x)), detectedText.join(" | "));
    await shot("17a_detected_features");
    const doc = async () => JSON.parse(await js("document.getElementById('job').value"));
    // Drill 4 x 5.00: a drilling operation with the 5 mm tool, the four centres, through depth
    await js("[...document.querySelectorAll('#detected .detected-btn')][0].click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.length === 1, "the drilling operation", 15000);
    const drill = (await doc()).setups[0].operations[0];
    check("Drill 4 × 5.00: drilling with the 5 mm tool, four points, from the top, through (12.5), peck one diameter",
      drill.type === "drill" && drill.tool === 1 && drill.points.length === 4 && drill.top === 0 && drill.depth === 12.5 && drill.peck === 5, JSON.stringify(drill));
    // picking mode on it: the four holes picked as chips
    // the new operation is selected: its editor offers picking
    await waitFor(() => js("window.__marcam.state.tree.selected === 'op1' && !!document.getElementById('pickFaces')"), "the drilling editor", 15000);
    await js("document.getElementById('pickFaces').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.pickMesh && document.querySelectorAll('#pickChips .pick-chip').length === 4"), "picking on the drill", 30000);
    await sleep(400);
    // a canvas point that hits the feature `pred` picks (by its pickable), found by casting a
    // ray at its triangles' centres: [client x, y, canvas x, y]
    const screenOf = (pred) => js(`(() => { const v = window.__marcam.state.three, t = window.__marcam.state.tree; const pk = t.pick; const target = pk.list.find(${pred}); if (!target) return null;
      const want = new Set(target.faces); const el = v.renderer.domElement; const r = el.getBoundingClientRect(); const tri = t.regions.tri; const ray = new v.THREE.Raycaster();
      // the centre and points near each corner (a deep wall shows only near its rim)
      const W = [[1/3, 1/3, 1/3], [0.9, 0.05, 0.05], [0.05, 0.9, 0.05], [0.05, 0.05, 0.9]];
      for (let k = 0; k < t.regions.of.length; k++) { if (!want.has(t.regions.list[t.regions.of[k]].id)) continue;
        for (const w of W) {
          const c = new v.THREE.Vector3(...[0, 1, 2].map((d) => w[0] * tri[9*k+d] + w[1] * tri[9*k+3+d] + w[2] * tri[9*k+6+d])).project(v.camera);
          if (Math.abs(c.x) > 0.95 || Math.abs(c.y) > 0.95) continue;
          ray.setFromCamera(new v.THREE.Vector2(c.x, c.y), v.camera); const hit = ray.intersectObject(t.pickMesh)[0];
          if (hit && want.has(t.regions.list[t.regions.of[hit.faceIndex]].id)) { const px = (c.x + 1) / 2 * el.clientWidth, py = (1 - c.y) / 2 * el.clientHeight; return [r.left + px, r.top + py, px, py]; } } }
      return null; })()`);
    const chips = () => js("document.querySelectorAll('#pickChips .pick-chip').length");
    // remove one chip: three points
    await js("document.querySelector('#pickChips [data-unpick]').click(); true");
    await waitFor(async () => (await chips()) === 3, "a chip removed", 10000);
    const three = await js("window.__marcam.state.tree.draft?.op || JSON.parse(JSON.stringify(window.__marcam.state.tree.doc.setups[0].operations[0]))");
    // select all similar: back to four
    await js("document.getElementById('pickSimilar').click(); true");
    await waitFor(async () => (await chips()) === 4, "select all similar", 10000);
    check("chips list the picks; removing one drops its point; select all similar adds the rest",
      three.points.length === 3 && (await js("window.__marcam.state.tree.doc.setups[0].operations[0].points.length")) === 4);
    // the hole at (40, 8) on the canvas: hover shows its label, a click un-picks it
    const at = await screenOf("(x) => x.kind === 'hole' && x.data.through && Math.abs(x.data.center[0] - 40) < 0.01 && Math.abs(x.data.center[1] - 8) < 0.01");
    await S("Input.dispatchMouseEvent", { type: "mouseMoved", x: at[0], y: at[1], buttons: 0 });
    await waitFor(() => js("/Hole 5\\.00 mm through/.test(document.getElementById('pickInfo').textContent)"), "the hover label", 10000);
    const hovered = await js("document.getElementById('pickInfo').textContent");
    await shot("17b_picking_holes");
    const picked = await js(`const pr = await window.__marcam.tree.pickAt(${at[2]}, ${at[3]}); pr && pr.pickable ? pr.pickable.kind : "none"`);
    await waitFor(async () => (await chips()) === 3, "the hole clicked off", 10000);
    // a click on the pocket floor: drilling does not take it
    // (the pocket floor is no drilling target, so it is not in the picking list: find it by
    // its faces among all regions instead)
    const floor = await js(`(() => { const v = window.__marcam.state.three, t = window.__marcam.state.tree; const el = v.renderer.domElement; const tri = t.regions.tri; const ray = new v.THREE.Raycaster();
      for (let k = 0; k < t.regions.of.length; k++) { const z = (tri[9*k+2]+tri[9*k+5]+tri[9*k+8])/3; if (Math.abs(z + 5) > 1e-6) continue;
        const c = new v.THREE.Vector3((tri[9*k]+tri[9*k+3]+tri[9*k+6])/3, (tri[9*k+1]+tri[9*k+4]+tri[9*k+7])/3, z).project(v.camera);
        ray.setFromCamera(new v.THREE.Vector2(c.x, c.y), v.camera); const hit = ray.intersectObject(t.pickMesh)[0];
        if (hit && hit.faceIndex === k) return [(c.x + 1) / 2 * el.clientWidth, (1 - c.y) / 2 * el.clientHeight]; }
      return null; })()`);
    await js(`window.__marcam.tree.pickAt(${floor[0]}, ${floor[1]}).then(() => true)`);
    const refused = await js("document.getElementById('pickNotes')?.textContent || ''");
    check("hovering a hole shows its label; clicking it toggles it; a face drilling cannot take is refused with a note",
      /Hole 5\.00 mm through/.test(hovered) && picked === "hole" && (await chips()) === 3 && /not a hole/.test(refused), `${hovered} | ${picked} | ${refused}`);
    await js("document.getElementById('pickFaces').click(); true");
    await sleep(400);
    check("done picking writes the picks once: three points", (await doc()).setups[0].operations[0].points.length === 3 && !(await js("!!window.__marcam.state.tree.pickMesh")));
    // the blind 6 mm hole: no 6 mm tool, so it is bored (with the largest that fits: 5 mm)
    await js("[...document.querySelectorAll('#detected .detected-btn')].find((b) => /^Bore 1 × 6\.00 mm blind/.test(b.textContent)).click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.length === 2, "the boring", 15000);
    const bored = (await doc()).setups[0].operations[1];
    // a drilling picked on it says use Bore instead, and becomes a bore
    await js("document.getElementById('treeAddType').value = 'drill'; document.getElementById('treeAdd').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.draft && !!document.getElementById('pickFaces')"), "the drilling draft", 15000);
    await js("document.getElementById('pickFaces').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.pickMesh && !!window.__marcam.state.tree.pick"), "picking for the draft", 30000);
    await sleep(300);
    const blindAt = await screenOf("(x) => x.kind === 'hole' && !x.data.through");
    await js(`const pr = await window.__marcam.tree.pickAt(${blindAt[2]}, ${blindAt[3]}); true`);
    await waitFor(() => js("!!document.getElementById('boreInstead')"), "the Bore instead button", 10000);
    const boreNote = await js("document.getElementById('pickNotes').textContent");
    await js("document.getElementById('boreInstead').click(); true");
    await waitFor(() => js("window.__marcam.state.tree.draft?.op.type === 'bore'"), "the draft bored", 10000);
    await js("document.querySelector('#opForm button[type=submit]').click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.length === 3, "the bore applied", 15000);
    const converted = (await doc()).setups[0].operations[2];
    check("holes without a tool of their size are bored; a drilling on one says use Bore instead, and Bore instead makes it a bore",
      bored.type === "bore" && bored.tool === 1 && bored.diameter === 6 && bored.depth === 6 && JSON.stringify(bored.points) === "[[46,20]]" &&
      /Bore/.test(boreNote) && converted.type === "bore" && converted.diameter === 6 && JSON.stringify(converted.points) === "[[46,20]]",
      `${JSON.stringify({ type: bored.type, tool: bored.tool, d: bored.diameter, depth: bored.depth })} | ${boreNote} | ${converted.type}`);
    // the pocket: its outline and island, its depth
    await js("[...document.querySelectorAll('#detected .detected-btn')].find((b) => b.dataset.start === 'pocket').click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.length === 4, "the pocket operation", 15000);
    const pocket = (await doc()).setups[0].operations.find((o) => o.type === "pocket");
    check("the pocket takes its outline, island and depth",
      pocket.paths.length === 2 && pocket.depth === 5 && pocket.top === 0, JSON.stringify({ paths: pocket.paths?.length, depth: pocket.depth }));
    // it all plans and verifies
    await js("document.getElementById('error').textContent = ''; document.getElementById('generate').click(); true");
    await waitDone("generation of the picked bracket", 240000);
    const tabs = await js("[...document.querySelectorAll('#tabs button')].map((b) => b.textContent)");
    const arcs = await js("window.__marcam.state.result.programs.reduce((n, p) => n + p.gcode.split('\\n').filter((l) => l.startsWith('G3 ')).length, 0)");
    check("the picked operations plan and verify clean; the bores are written as G3 arcs", tabs.length >= 1 && tabs.every((x) => x.endsWith("OK")) && arcs > 0, `${tabs.join(", ")}; ${arcs} arcs`);
    await shot("17c_picked_generated");
    // a 3D parallel finish says what it will do: the plate's dome raster has no roughing
    await js("document.getElementById('example').value = 'plate'; document.getElementById('loadExample').click(); true");
    await settle();
    const rid = await js("window.__marcam.state.tree.doc.setups[0].operations.find((o) => o.type === 'raster3d').id");
    await js(`document.querySelector('#tree .tree-op[data-op="${rid}"] [data-act="select"]').click(); true`);
    await waitFor(() => js("!!document.getElementById('rasterInfo')"), "the raster's info", 10000);
    const rinfo = await js("document.getElementById('rasterInfo').textContent");
    check("a 3D parallel finish shows its stepover, rows and levels, and suggests a roughing when none comes first",
      /stepover [\d.]+ mm/.test(rinfo) && /\d+ rows/.test(rinfo) && /no 3D roughing before it/.test(rinfo), rinfo);
  }

  // 8m. picking on every operation type: rotary turning (a band -> its X span), knurl (a
  // cone -> the body), chamfer (a round edge), engraving wrapped round the bar (its radius,
  // the text centred), facing on an indexed setup (a side face -> its A index and depth)
  {
    const screenOf = (pred) => js(`(() => { const v = window.__marcam.state.three, t = window.__marcam.state.tree; const pk = t.pick; const target = pk.list.find(${pred}); if (!target) return null;
      const want = new Set(target.faces); const el = v.renderer.domElement; const r = el.getBoundingClientRect(); const tri = t.regions.tri; const ray = new v.THREE.Raycaster();
      const W = [[1/3, 1/3, 1/3], [0.9, 0.05, 0.05], [0.05, 0.9, 0.05], [0.05, 0.05, 0.9]];
      for (let k = 0; k < t.regions.of.length; k++) { if (!want.has(t.regions.list[t.regions.of[k]].id)) continue;
        for (const w of W) {
          const c = new v.THREE.Vector3(...[0, 1, 2].map((d) => w[0] * tri[9*k+d] + w[1] * tri[9*k+3+d] + w[2] * tri[9*k+6+d])).project(v.camera);
          if (Math.abs(c.x) > 0.95 || Math.abs(c.y) > 0.95) continue;
          ray.setFromCamera(new v.THREE.Vector2(c.x, c.y), v.camera); const hit = ray.intersectObject(t.pickMesh)[0];
          if (hit && want.has(t.regions.list[t.regions.of[hit.faceIndex]].id)) { const px = (c.x + 1) / 2 * el.clientWidth, py = (1 - c.y) / 2 * el.clientHeight; return [r.left + px, r.top + py, px, py]; } } }
      return null; })()`);
    const doc = async () => JSON.parse(await js("document.getElementById('job').value"));
    // pick `pred` for operation `k` (by its row) and write it; returns the operation
    async function pickFor(k, pred, what) {
      const id = await js(`document.querySelectorAll('#tree .tree-op')[${k}].dataset.op`);
      if ((await js("window.__marcam.state.tree.selected")) !== id) await js(`document.querySelector('#tree .tree-op[data-op="${id}"] [data-act="select"]').click(); true`);
      await waitFor(() => js(`window.__marcam.state.tree.selected === '${id}' && !!document.getElementById('pickFaces')`), `${what}: its editor`, 15000);
      await js("document.getElementById('pickFaces').click(); true");
      await waitFor(() => js("!!window.__marcam.state.tree.pickMesh && !!window.__marcam.state.tree.pick"), `${what}: picking`, 30000);
      await sleep(300);
      const at = await screenOf(pred);
      if (!at) throw new Error(`${what}: the feature is not visible`);
      const got = await js(`const pr = await window.__marcam.tree.pickAt(${at[2]}, ${at[3]}); pr && pr.pickable ? pr.pickable : null`);
      await js("document.getElementById('pickFaces').click(); true");
      await waitFor(() => js("!window.__marcam.state.tree.pickMesh"), `${what}: done`, 15000);
      await sleep(300);
      const s = (await doc()).setups.find((x) => x.operations.some((o) => o.id === id));
      return { op: s.operations.find((o) => o.id === id), got };
    }
    // the knob: turning, knurl, chamfer
    await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
    await settle();
    const turn = await pickFor(0, "(x) => x.kind === 'cylindrical_band' && Math.abs(x.data.r[1] - 5) < 0.01", "turning");
    const knurl = await pickFor(1, "(x) => x.kind === 'revolved_band' && x.data.shape === 'cone' && x.data.r[1] > 9.9", "knurl");
    const chamfer = await pickFor(2, "(x) => x.kind === 'circular_edge'", "chamfer");
    check("rotary picks: turning takes the boss's X span, the knurl the taper as its body, the chamfer a round edge",
      turn.got?.kind === "cylindrical_band" && JSON.stringify(turn.op.x_range) === JSON.stringify(turn.got.data.x) &&
      knurl.op.body_back[0] === -15 && Math.abs(knurl.op.body_back[1] - 10) < 0.01 && knurl.op.body_front[0] < 0 &&
      chamfer.got?.kind === "circular_edge" && chamfer.op.edge[0] === chamfer.got.data.x && chamfer.op.edge[1] === chamfer.got.data.r,
      JSON.stringify({ x_range: turn.op.x_range, back: knurl.op.body_back, front: knurl.op.body_front, edge: chamfer.op.edge }));
    await shot("17d_rotary_picks");
    // the ring: its text engraving onto the bar (R12), centred on the band picked
    await js("document.getElementById('example').value = 'ring'; document.getElementById('loadExample').click(); true");
    await settle();
    const rows = await js("[...document.querySelectorAll('#tree .tree-op')].map((r) => r.querySelector('.tree-label').textContent)");
    const textRow = Math.max(0, (await doc()).setups[0].operations.findIndex((o) => o.type === "engrave" && o.text));
    const eng = await pickFor(textRow, "(x) => x.kind === 'cylindrical_band'", "engraving");
    check("engraving picks a cylinder: its radius is the surface, the text centred on it (round the top at A0)",
      eng.got?.kind === "cylindrical_band" && Math.abs(eng.op.top - eng.got.data.r[1]) < 1e-6 && Math.abs(eng.op.text_at[1] + eng.op.text_height / 2) < 1e-6,
      `${rows.length} rows; top ${eng.op.top}; text_at ${eng.op.text_at}`);
    // the bracket on a 4th axis (indexed): a new facing operation, its underside picked
    const fx = join(root, "tests", "fixtures", "picking");
    const proj = JSON.parse(readFileSync(join(fx, "bracket.project.json"), "utf8"));
    Object.assign(proj.setups[0], { id: "idx", name: "Indexed", kind: "indexed", machine: "makera-z1-4axis", stock: { shape: "cylinder", radius: 40, length: 70 }, holding: { kind: "rotary_chuck", tailstock: false } });
    const file = JSON.stringify({ format: "marcam-project", version: 1, project: proj, meshes: { "bracket.stl": readFileSync(join(fx, "bracket.stl")).toString("base64") } });
    await js(`window.__marcam.project.openText(${JSON.stringify(file)}, "bracket_indexed.marcam.json").then(() => true)`);
    await settle();
    await js("document.getElementById('treeAddType').value = 'face'; document.getElementById('treeAdd').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.draft && !!document.getElementById('pickFaces')"), "the facing draft", 15000);
    await js("document.getElementById('pickFaces').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.pickMesh && !!window.__marcam.state.tree.pick"), "picking a side face", 30000);
    // orbit underneath to see the underside
    await js("(() => { const v = window.__marcam.state.three; v.camera.position.set(30, -50, -70); v.controls.target.set(30, 20, -6); v.controls.update(); v.renderer.render(v.scene, v.camera); return true; })()");
    await sleep(300);
    const sideAt = await screenOf("(x) => x.kind === 'side_face' && Math.abs(Math.abs(x.data.index_a) - 180) < 1e-6");
    const side = sideAt ? await js(`const pr = await window.__marcam.tree.pickAt(${sideAt[2]}, ${sideAt[3]}); pr && pr.pickable ? pr.pickable.kind : "none"`) : "not visible";
    await js("document.getElementById('pickFaces').click(); true");
    await sleep(400);
    await js("document.querySelector('#opForm button[type=submit]').click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.some((o) => o.type === "face"), "the facing applied", 15000);
    const face = (await doc()).setups[0].operations.find((o) => o.type === "face");
    check("indexed: picking the underside sets the facing's A index (180) and its depth from the bar (40 - 12 = 28)",
      side === "side_face" && Math.abs(Math.abs(face.index_a) - 180) < 1e-6 && Math.abs(face.depth - 28) < 1e-6, `${side}; index_a ${face.index_a}; depth ${face.depth}`);
    await shot("17e_indexed_side_face");
    // every operation type offers picking
    const types = await js("window.__marcam.state.tree.types.filter((t) => !(t.picks || []).length).map((t) => t.kind)");
    check("every operation type takes picks", types.length === 0, types.join(", "));
  }

  // 8n. (Mar) removing every operation keeps the model: part and stock come from the project,
  // Generate says why it cannot; disabling all and reopening such a project too. And toolpath
  // visibility per operation, per file, per setup (view only)
  {
    const doc = async () => JSON.parse(await js("document.getElementById('job').value"));
    const view = () => js("(() => { const v = window.__marcam.state.three; return { part: v.groups.part.children.length, stock: v.groups.stock.children.length, path: v.groups.path.children.reduce((n, l) => n + (l.geometry?.attributes.position?.count || 0), 0) }; })()");
    // import the knob into a vise setup (two starter operations), generate
    const knobStl = readFileSync(join(root, "examples", "knob", "knob.stl")).toString("base64");
    const proj0 = { schema_version: "0.2", name: "noops", models: [], tools: [], setups: [] };
    await js(`window.__marcam.project.openText(${JSON.stringify(JSON.stringify({ format: "marcam-project", version: 1, project: proj0 }))}, "noops.marcam.json").then(() => true)`);
    await settle();
    await js(`(() => { const dt = new DataTransfer(); dt.items.add(new File([Uint8Array.from(atob("${knobStl}"), (c) => c.charCodeAt(0))], "knob.stl")); const i = document.getElementById('importStlFile'); i.files = dt.files; i.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
    await waitFor(() => js("document.getElementById('setupDialog').open"), "the import dialog", 15000);
    await js("document.querySelector('.sd-kind[data-kind=\"vise\"]').click(); document.getElementById('sdOk').click(); true");
    await waitDone("the knob imported on a vise", 240000);
    // delete every operation, one by one
    for (const id of await js("[...document.querySelectorAll('#tree .tree-op')].map((r) => r.dataset.op).reverse()")) {
      await js(`document.querySelector('#tree .tree-op[data-op="${id}"] [data-act="delete"]').click(); true`);
      await sleep(600);
    }
    await sleep(800);
    const none = await view();
    const g0 = await js("({ disabled: document.getElementById('generate').disabled, title: document.getElementById('generate').title, empty: document.getElementById('emptyState')?.textContent || '', detected: !!document.getElementById('detected'), ops: document.querySelectorAll('#tree .tree-op').length })");
    check("every operation removed: the part and stock stay, no old toolpath, Generate disabled with the reason, an empty state that says so",
      none.part > 0 && none.stock > 0 && none.path === 0 && g0.ops === 0 && g0.disabled && /No operation/.test(g0.title) && /No operations/.test(g0.empty) && g0.detected,
      `${JSON.stringify(none)} ${JSON.stringify(g0)}`);
    await shot("18a_no_operations");
    // reopen it saved with no operations: still the part and stock
    const saved = await js("JSON.stringify((() => { const st = window.__marcam.state; let bin = ''; for (let i = 0; i < st.mesh.length; i += 8192) bin += String.fromCharCode(...st.mesh.subarray(i, i + 8192)); return { format: 'marcam-project', version: 1, project: JSON.parse(document.getElementById('job').value), meshes: { 'knob.stl': btoa(bin) } }; })())");
    await js(`window.__marcam.project.openText(${JSON.stringify(saved)}, "noops2.marcam.json").then(() => true)`);
    await settle();
    await sleep(800);
    const reopened = await view();
    check("a project saved with no operations reopens with its part and stock", reopened.part > 0 && reopened.stock > 0, JSON.stringify(reopened));
    // add an operation back: Generate works
    await js("document.getElementById('treeAddType').value = 'raster3d'; document.getElementById('treeAdd').click(); true");
    await waitFor(() => js("!!window.__marcam.state.tree.draft"), "the new raster", 15000);
    await js("document.querySelector('#opForm button[type=submit]').click(); true");
    await waitFor(async () => (await doc()).setups[0].operations.length === 1, "the raster applied", 15000);
    await sleep(500);
    const enabled = await js("!document.getElementById('generate').disabled");
    await js("document.getElementById('generate').click(); true");
    await waitDone("generation after adding an operation back", 240000);
    const back = await view();
    check("an operation added back: Generate is enabled again and plans", enabled && back.path > 0 && back.part > 0, JSON.stringify(back));
    // disabled, all of them: the same as none
    await js("document.querySelector('#tree .tree-op [data-act=\"enable\"]').click(); true");
    await sleep(800);
    const off = await view();
    const g1 = await js("({ disabled: document.getElementById('generate').disabled, title: document.getElementById('generate').title })");
    check("every operation disabled: the part and stock stay, Generate disabled with the reason", off.part > 0 && off.stock > 0 && g1.disabled && /enable one/.test(g1.title), `${JSON.stringify(off)} ${JSON.stringify(g1)}`);
    await js("document.querySelector('#tree .tree-op [data-act=\"enable\"]').click(); true");
    await sleep(500);

    // visibility: the plate (several operations in one file), generated
    await js("document.getElementById('example').value = 'plate'; document.getElementById('loadExample').click(); true");
    await settle();
    await js("document.getElementById('generate').click(); true");
    await waitDone("the plate for visibility", 240000);
    const plans0 = await js("window.__marcam.calls.planAndPost || 0");
    const gcode0 = await js("window.__marcam.state.result.programs.map((p) => p.gcode).join('\\n')");
    const all = (await view()).path;
    const opIds = await js("window.__marcam.state.tree.doc.setups[0].outputs[0].operations");
    // hide the first operation (facing)
    await js(`document.querySelector('#tree .tree-op[data-op="${opIds[0]}"] [data-act="eye"]').click(); true`);
    await sleep(500);
    const lessOne = (await view()).path;
    const eye = await js(`document.querySelector('#tree .tree-op[data-op="${opIds[0]}"] [data-act="eye"]').classList.contains('off')`);
    // hide all, then show only the pocket (Alt-click)
    await js("document.getElementById('eyeNone').click(); true");
    await sleep(400);
    const noneShown = (await view()).path;
    await js(`(() => { const b = document.querySelector('#tree .tree-op[data-op="${opIds[1]}"] [data-act="eye"]'); b.dispatchEvent(new MouseEvent('click', { bubbles: true, altKey: true })); return true; })()`);
    await sleep(400);
    const only = (await view()).path;
    const hiddenNow = await js("[...window.__marcam.state.hidden]");
    const stale = await js("document.body.classList.contains('stale')");
    const plans1 = await js("window.__marcam.calls.planAndPost || 0");
    const gcode1 = await js("window.__marcam.state.result.programs.map((p) => p.gcode).join('\\n')");
    check("hiding toolpaths is view only: one hidden (fewer moves drawn, its eye off), all hidden (none), show only one (Alt-click); nothing planned, nothing out of date, the same G-code",
      lessOne < all && lessOne > 0 && eye && noneShown === 0 && only > 0 && only < all && hiddenNow.length === (await js("window.__marcam.state.tree.doc.setups[0].operations.length")) - 1 && plans1 === plans0 && !stale && gcode1 === gcode0,
      `${all} -> ${lessOne} -> ${noneShown} -> ${only}; hidden ${hiddenNow.length}; plans ${plans1 - plans0}; stale ${stale}`);
    await shot("18b_toolpaths_hidden");
    // saved and reopened: the same ones hidden
    const savedPlate = await js("JSON.stringify((() => { const st = window.__marcam.state; let bin = ''; for (let i = 0; i < st.mesh.length; i += 8192) bin += String.fromCharCode(...st.mesh.subarray(i, i + 8192)); return { format: 'marcam-project', version: 1, project: JSON.parse(document.getElementById('job').value), meshes: { 'dome.stl': btoa(bin) }, view: { hidden: [...st.hidden] } }; })())");
    await js(`window.__marcam.project.openText(${JSON.stringify(savedPlate)}, "plate_view.marcam.json").then(() => true)`);
    await settle();
    const restored = await js("[...window.__marcam.state.hidden].sort().join(',')");
    check("which toolpaths are hidden is saved with the project and comes back on open", restored === hiddenNow.slice().sort().join(","), restored);
    await js("document.getElementById('eyeAll').click(); true");
  }

  // 9. phone width: layout stacks, no horizontal scroll
  await S("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
  await sleep(800);
  const overflow = await js("document.documentElement.scrollWidth - document.documentElement.clientWidth");
  check("phone width: no horizontal scroll", overflow <= 0, `${overflow}px`);
  await shot("06_phone");

  const errors = cdp.events.filter((e) => e.method === "Runtime.exceptionThrown").map((e) => e.params.exceptionDetails.exception?.description || e.params.exceptionDetails.text);
  if (errors.length) console.log(`uncaught page errors:\n${errors.join("\n---\n")}`);
  check("no uncaught page errors", errors.length === 0, errors.join(" | ").slice(0, 300));
} catch (e) {
  failed = true;
  console.log(`FAIL  ${e.message}`);
  try { await shot("99_failure"); } catch {}
} finally {
  try { await cdp.send("Browser.close"); } catch {}
  cleanup();
  await sleep(500);
  try { rmSync(profile, { recursive: true, force: true }); } catch {}
}
const bad = results.filter((r) => !r.ok).length + (failed ? 1 : 0);
console.log(`\n${results.length - results.filter((r) => !r.ok).length}/${results.length} checks passed; screenshots in web/tests/screenshots/`);
process.exit(bad ? 1 : 0);
