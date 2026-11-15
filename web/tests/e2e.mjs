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

  // 5. LinuxCNC machine (G93)
  await js("document.getElementById('machine').value = 'linuxcnc'; document.getElementById('machine').dispatchEvent(new Event('change')); document.getElementById('generate').click()");
  await waitDone("LinuxCNC generation", 60000);
  const g93 = await js("document.getElementById('gcode').value.includes('\\nG93 ')");
  check("LinuxCNC post uses G93 inverse time", g93);

  // 6. quick settings edit the JSON
  await js("const i = document.getElementById('q_stock_radius'); i.value = 13; i.dispatchEvent(new Event('change')); true");
  const r = await js("JSON.parse(document.getElementById('job').value).stock.radius");
  check("quick setting writes into the job JSON", r === 13, `stock.radius = ${r}`);

  // 7. errors are shown, not thrown
  await js("document.getElementById('job').value = '{ not json'; document.getElementById('generate').click(); true");
  const err = await waitFor(() => js("document.getElementById('error').textContent"), "an error message", 5000);
  check("invalid JSON shows an error", /not valid/.test(err), err.slice(0, 60));

  // 8. mesh example: rough + two finishes against knob.stl
  await js("document.getElementById('machine').value = 'makera-z1'; document.getElementById('example').value = 'knob_finish'; document.getElementById('loadExample').click(); true");
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
