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
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
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

const profile = mkdtempSync(join(tmpdir(), "fourcam-e2e-"));
const downloads = join(profile, "downloads");
mkdirSync(downloads);
const dport = await freePort();
const browser = spawn(findBrowser(), [
  "--headless=new", `--remote-debugging-port=${dport}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--window-size=1400,900", "about:blank",
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
const VIEW_PREFIX = process.env.FOURCAM_SHOT_PREFIX || "";
async function closedViews(tag) {
  const bg = [0, 0, 0];
  const out = [];
  for (const layer of ["part", "stock", "sim"]) {
    const has = await js(`window.__4cam.state.three.groups.${layer}.children.length > 0`);
    if (!has) continue;
    for (const end of ["front", "back"]) {
      const show = { path: false, axes: false, part: layer === "part", stock: layer === "stock", sim: layer === "sim" };
      await js(`window.__4cam.setView(${JSON.stringify(end)}, ${JSON.stringify(show)})`);
      await sleep(200);
      // the axis line of sight and a small ring around it must all hit the end face
      let px = [], filled = true;
      for (const [fx, fy] of [[0.5, 0.5], [0.49, 0.5], [0.51, 0.5], [0.5, 0.49], [0.5, 0.51]]) {
        const p = String(await js(`window.__4cam.pixelAt(${fx}, ${fy})`)).split(",").map(Number);
        if (!p.some((c, k) => Math.abs(c - bg[k]) > 4)) filled = false; // background is exactly black
        px = p;
      }
      await shot(`${VIEW_PREFIX}view_${tag}_${layer}_${end}`);
      out.push({ layer, end, filled, px });
    }
    await js(`window.__4cam.setView("iso", { path: false, axes: false, part: ${layer === "part"}, stock: ${layer === "stock"}, sim: ${layer === "sim"} })`);
    await sleep(200);
    await shot(`${VIEW_PREFIX}view_${tag}_${layer}_iso`);
  }
  await js(`window.__4cam.setView("iso", { path: true, axes: true, part: true, stock: true, sim: true })`);
  return out;
}

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
  }
  check("line counts 21,787 / 15,146", lines.join(" / ") === "21,787 / 15,146", lines.join(" / "));

  // 2. 3D view (three.js from the CDN) and the scrubber
  await js("document.querySelectorAll('#tabs button')[0].click()");
  await waitFor(() => js("!!(window.__4cam.state.three && (window.__4cam.state.three.failed || window.__4cam.state.three.cutLines))"), "the 3D view", 30000);
  const three = await js("window.__4cam.state.three.failed ? 'failed: ' + document.getElementById('viewNote').textContent : 'ok'");
  check("3D preview renders (three.js)", three === "ok", three);
  if (three === "ok") {
    const full = await js("window.__4cam.state.three.cutLines.geometry.drawRange.count");
    await js("const s = document.getElementById('scrub'); s.value = 3000; s.dispatchEvent(new Event('input')); true");
    await sleep(300);
    const part = await js("window.__4cam.state.three.cutLines.geometry.drawRange.count");
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
  const M = "window.__4cam.machining.state";
  const nseg = await js(`${M}.session.segments`);
  const ops = await js(`${M}.session.ops.map(o => o.kind).join(',')`);
  check("machining simulation opens with the job's operations", nseg > 30000 && ops === "rotary_turning,helical_knurl,flank_chamfer", `${nseg} moves; ${ops}`);
  await js("window.__4cam.setView('iso', {})");
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
  await js("window.__4cam.setView('iso', {})");
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
  await js("window.__4cam.setView('iso', {})");
  await sleep(300);
  await shot("10_machining_after_knurl");
  // operation colour
  const colorAt = `(() => { const s = ${M}; const i = s.op.indexOf(2); return Array.from(s.stockMesh.geometry.attributes.color.array.slice(3 * i, 3 * i + 3)).map(c => c.toFixed(2)).join(','); })()`;
  const before = await js(colorAt);
  await js("const c = document.querySelector('#opLegend input[data-tag=\"2\"]'); c.value = '#ff0000'; c.dispatchEvent(new Event('input', { bubbles: true })); true");
  const after = await js(colorAt);
  const saved = await js("localStorage.getItem('4cam.opColors')");
  check("changing an operation's colour recolours its cuts (and is saved)", before !== after && after === "1.00,0.00,0.00" && /#ff0000/.test(saved), `${before} -> ${after}`);
  await js("window.__4cam.setView('iso', {})");
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
  check("limit check flags the feed cap (F5209 > 4000 mm/min)", /Carvera/.test(lim) && /F5209/.test(lim), lim.split("\n")[1] || lim);

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
  await shot("07_tool_library");
  await S("Page.reload");
  await waitFor(async () => (await state()) === "ready", "reload", 30000);
  const kept = await js("[...document.getElementById('library').options].map(o => o.value).join(',') + ' / ' + JSON.stringify(JSON.parse(localStorage.getItem('4cam.libraries')).find(l => l.id === 'fusion-tool-library').tools[0].name)");
  check("tool libraries persist across reloads", /fusion-tool-library/.test(kept) && /Edited Flat 3\.0/.test(kept), kept);
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await sleep(500);

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

  // 9. phone width: layout stacks, no horizontal scroll
  await S("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
  await sleep(800);
  const overflow = await js("document.documentElement.scrollWidth - document.documentElement.clientWidth");
  check("phone width: no horizontal scroll", overflow <= 0, `${overflow}px`);
  await shot("06_phone");

  const errors = cdp.events.filter((e) => e.method === "Runtime.exceptionThrown").map((e) => e.params.exceptionDetails.exception?.description || e.params.exceptionDetails.text);
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
