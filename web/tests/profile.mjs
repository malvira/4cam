// Where the main thread's time goes while the knob is planned again and simulated in the
// background: a CPU profile over the edit, the functions by self time (and the long tasks).
//
//   node web/tests/profile.mjs [--root other-checkout]
import { sleep, startApp, waitFor } from "./harness.mjs";

const arg = (name, dflt) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : dflt);
const ROOT = arg("--root", undefined);
const app = await startApp(ROOT ? { root: ROOT } : {});
const { js, S } = app;
try {
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await waitFor(() => js("document.body.dataset.state === 'done' && !!window.__marcam.state.result && window.__marcam.planIdle()"), "the knob planned", 240000);
  await sleep(2000);
  await waitFor(() => js("!window.__marcam.simLock?.busy && !window.__marcam.simRing?.active"), "idle", 240000).catch(() => {});
  if (process.argv.includes("--playback")) {
    await js("(() => { const m = window.__marcam.state.machining; if (!m.view.stock) window.__marcam.machining.toggle('stock'); document.getElementById('simOpen').click(); return true; })()");
    await waitFor(() => js("(() => { const m = window.__marcam.state.machining; return !!(m.open && m.session && m.stockMesh && !m.busy); })()"), "the playback", 240000);
  }
  await S("Profiler.enable");
  await S("Profiler.setSamplingInterval", { interval: 200 });
  await S("Profiler.start");
  await js(`(() => { const P = (window.__long = []); new PerformanceObserver((l) => { for (const e of l.getEntries()) P.push([Math.round(e.startTime), Math.round(e.duration)]); }).observe({ type: 'longtask' }); const t = document.getElementById('job'); const j = JSON.parse(t.value); for (const p of j.programs || []) for (const o of p.operations || []) o.feed = Math.round(o.feed * 0.9); for (const s of j.setups || []) for (const o of s.operations || []) o.feed = Math.round(o.feed * 0.9); t.value = JSON.stringify(j, null, 2); t.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
  await sleep(800);
  await waitFor(() => js("window.__marcam.planIdle() && document.body.dataset.state !== 'busy'"), "the replan", 240000);
  await js("window.__marcam.statsInBackground?.(); true");
  await sleep(500);
  await waitFor(() => js("!window.__marcam.simLock.busy && !window.__marcam.simRing.active"), "the simulation", 240000);
  await sleep(500);
  const { profile } = await S("Profiler.stop");
  const self = new Map();
  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const dt = profile.timeDeltas;
  const counts = new Map();
  profile.samples.forEach((id, i) => counts.set(id, (counts.get(id) || 0) + (dt[i] || 0)));
  for (const [id, us] of counts) {
    const n = byId.get(id);
    const f = n.callFrame;
    const name = `${f.functionName || "(anon)"} ${f.url.split("/").pop()}:${f.lineNumber + 1}`;
    self.set(name, (self.get(name) || 0) + us);
  }
  const top = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 25);
  for (const [name, us] of top) console.log(`${(us / 1000).toFixed(0).padStart(7)} ms  ${name}`);
  console.log("long tasks:", JSON.stringify(await js("window.__long")));
} finally {
  await app.close();
}
