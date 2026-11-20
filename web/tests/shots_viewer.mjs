// Before/after screenshots of the viewer at rest: the knob and the oval part (a facing, a
// pocket and a cut-out), as a person first sees them after the plan.
//
//   node web/tests/shots_viewer.mjs --root <checkout> --tag before|after
import { readFileSync, mkdirSync, copyFileSync } from "node:fs";
import { join } from "node:path";
import { root, sleep, startApp, waitFor } from "./harness.mjs";

const arg = (name, dflt) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : dflt);
const ROOT = arg("--root", undefined);
const TAG = arg("--tag", "after");
const OUT = arg("--out", join(root, "web", "tests", "screenshots"));
const app = await startApp(ROOT ? { root: ROOT } : {});
const { js } = app;
try {
  await js("localStorage.removeItem('marcam.view'); true");
  const ready = async () => {
    await waitFor(() => js("document.body.dataset.state === 'done' && !!window.__marcam.state.result && window.__marcam.planIdle?.() !== false"), "planned", 240000);
    await sleep(2500);
    await js("window.__marcam.setView('iso', {})");
    await sleep(500);
  };
  await js("document.getElementById('example').value = 'knob'; document.getElementById('loadExample').click(); true");
  await sleep(500);
  if (!(await js("document.getElementById('autoGen')?.checked"))) await js("document.getElementById('generate').click(); true");
  await ready();
  await app.shot(`viewer_${TAG}_knob`);
  const fx = join(root, "tests", "fixtures", "picking");
  const gp = JSON.parse(readFileSync(join(fx, "oval_grip.project.json"), "utf8"));
  gp.setups[0].operations = [
    { id: "op1", type: "face", tool: 2, depth: 0.1, step_down: 0.5, stepover: 0.6, feed: 800, plunge_feed: 200 },
    { id: "op2", type: "pocket", tool: 2, rect: [1, 1, 5, 5], depth: 0.1, step_down: 0.5, stepover: 0.5, feed: 800, plunge_feed: 200 },
    { id: "op3", type: "contour", tool: 2, path: [[1, 1], [5, 1], [5, 5]], side: "outside", depth: 0.1, step_down: 1.5, feed: 800, plunge_feed: 200 },
    { id: "op4", type: "rough3d", tool: 1, x_range: [-5, 65], y_range: [-5, 35], stepover: 1.2, sample: 0.4, step_down: 1.5, floor: -8.5, leave: 0.25, feed: 600, plunge_feed: 100 },
  ];
  gp.setups[0].outputs = [{ name: "grip", title: "grip", operations: ["op4", "op1", "op2", "op3"] }];
  const text = JSON.stringify({ format: "marcam-project", version: 1, project: gp, meshes: { "oval_grip.stl": readFileSync(join(fx, "oval_grip.stl")).toString("base64") } });
  await js(`await window.__marcam.project.openText(${JSON.stringify(text)}, "oval.marcam.json").then(() => true)`);
  await sleep(500);
  if (!(await js("document.getElementById('autoGen')?.checked"))) await js("document.getElementById('generate').click(); true");
  await ready();
  await app.shot(`viewer_${TAG}_oval`);
  console.log(`viewer_${TAG}_knob.png, viewer_${TAG}_oval.png`);
} finally {
  await app.close();
}
