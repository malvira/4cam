// STEP import in the browser: SolidCraft's kernel compiled to WebAssembly (crates/marcam-step,
// built by scripts/build-step-wasm.sh into pkg/marcam_step.wasm). The module is large, so it is
// fetched only when a STEP file is opened, once per page. It imports nothing: a STEP file goes
// in, a binary STL comes out whose facets carry their B-rep face numbers (the engine's face
// regions, "s1", "s2", ...), with a summary of the bodies and faces.

const dec = new TextDecoder();
let loading = null;

/** Whether `bytes` (or a file named `name`) is a STEP file. */
export function isStep(bytes, name = "") {
  if (/\.(step|stp)$/i.test(name)) return true;
  const head = dec.decode(bytes.subarray(0, Math.min(bytes.length, 64))).trimStart();
  return head.startsWith("ISO-10303-21");
}

function load() {
  if (!loading) {
    loading = (async () => {
      const res = await fetch(new URL("./pkg/marcam_step.wasm", import.meta.url));
      if (!res.ok) throw new Error(`the STEP module is not available (${res.status}): build it with scripts/build-step-wasm.sh`);
      const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {});
      return instance.exports;
    })();
    // a failed load can be tried again
    loading.catch(() => (loading = null));
  }
  return loading;
}

/**
 * A STEP file as a binary STL with its faces, tessellated within `chord` mm:
 * { stl: Uint8Array, bodies, faces, triangles, names, warnings }. Throws with the reason when
 * the file cannot be read.
 */
export async function stepToStl(bytes, chord = 0.01) {
  const x = await load();
  const p = x.ms_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  let ok;
  try {
    ok = x.ms_import(p, bytes.length, chord);
  } finally {
    x.ms_free(p, bytes.length);
  }
  // (memory may have grown: views are taken afresh)
  const info = JSON.parse(dec.decode(new Uint8Array(x.memory.buffer, x.ms_result_ptr(), x.ms_result_len())));
  if (!ok) throw new Error(info.error || "the STEP file could not be read");
  const stl = new Uint8Array(x.memory.buffer, x.ms_stl_ptr(), x.ms_stl_len()).slice();
  return { stl, ...info };
}

window.__marcam = Object.assign(window.__marcam || {}, { step: { isStep, stepToStl, loaded: () => !!loading } });
