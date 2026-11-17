// Runs the marcam WebAssembly engine off the main thread.
// Messages in:  { id, cmd, args }   (cmd: version | exampleJob | machine | planAndPost | simulate | meshTriangles)
// Messages out: { id, ok, result } or { id, ok: false, error }
import { load } from "./pkg/marcam.js";

const ready = load(new URL("./pkg/marcam.wasm", import.meta.url));

self.onmessage = async (ev) => {
  const { id, cmd, args } = ev.data;
  try {
    const cam = await ready;
    if (typeof cam[cmd] !== "function") throw new Error(`unknown command ${cmd}`);
    const t0 = performance.now();
    const result = cam[cmd](...(args || []));
    if (result && result.error) {
      self.postMessage({ id, ok: false, error: result.error });
    } else {
      self.postMessage({ id, ok: true, result, ms: performance.now() - t0 });
    }
  } catch (e) {
    self.postMessage({ id, ok: false, error: String(e && e.message ? e.message : e) });
  }
};
