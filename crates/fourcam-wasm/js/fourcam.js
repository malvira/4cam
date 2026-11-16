// 4cam WebAssembly wrapper (ES module, browser and Node). No build step and no dependencies.
//
//   import { load } from "./fourcam.js";
//   const cam = await load(new URL("./fourcam.wasm", import.meta.url));   // or the bytes
//   const result = cam.planAndPost(jobObject, "makera-z1");
//
// Every function returns plain JSON-able objects; failures come back as { error: "..." }.

const enc = new TextEncoder();
const dec = new TextDecoder();

async function instantiate(source) {
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
    return (await WebAssembly.instantiate(source, {})).instance;
  }
  const resp = await fetch(source);
  if (!resp.ok) throw new Error(`could not load ${source}: ${resp.status}`);
  return (await WebAssembly.instantiate(await resp.arrayBuffer(), {})).instance;
}

export async function load(source) {
  const inst = await instantiate(source);
  const { memory, fc_alloc, fc_free, fc_call } = inst.exports;

  function put(bytes) {
    if (!bytes || bytes.length === 0) return [0, 0];
    const ptr = fc_alloc(bytes.length);
    new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
    return [ptr, bytes.length];
  }

  function arg(v) {
    if (v === undefined || v === null) return new Uint8Array(0);
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    if (typeof v === "string") return enc.encode(v);
    return enc.encode(JSON.stringify(v));
  }

  function call(name, a, b, c) {
    const args = [enc.encode(name), arg(a), arg(b), arg(c)].map(put);
    const res = fc_call(...args.flat());
    for (const [p, n] of args) if (p) fc_free(p, n);
    const len = new DataView(memory.buffer).getUint32(res, true);
    const text = dec.decode(new Uint8Array(memory.buffer, res + 4, len));
    fc_free(res, len + 4);
    return JSON.parse(text);
  }

  return {
    version: () => call("version"),
    exampleJob: () => call("example_job"),
    machine: (name) => call("machine", name || ""),
    // machine: built-in id/alias ("makera-z1-4axis", "carvera", ...) or a machine profile
    // object; opts.libraries: extra tool libraries for {"library", "tool"} references
    planAndPost: (job, machine, meshBytes, opts = {}) =>
      call("plan_and_post_opts", job, { ...opts, machine: machine ?? null }, meshBytes),
    simulate: (job, opts, meshBytes) => call("simulate", job, opts || {}, meshBytes),
    // built-in machine profiles and tool libraries
    profileList: () => call("profile_list"),
    // operation registry, and a 0.1/0.2 document upgraded + checked against it
    opTypes: () => call("op_types"),
    checkProject: (doc) => call("check_project", doc),
    faceRegions: (doc, model, meshBytes) => call("face_regions", doc, model, meshBytes),
    wizardKnurl: (doc, params, meshBytes) => call("wizard_knurl", doc, params || {}, meshBytes),
    // a new operation with every field filled: opts {type, setup, tool, geometry, machine}
    opDefaults: (doc, opts, meshBytes) => call("op_defaults", doc, opts || {}, meshBytes),
    // convert a profile file (4cam JSON, Fusion .tools/.json/.mch/.f3d) -> {kind, profile, notes}
    importProfile: (bytes, name) => call("import_profile", bytes, name || "profile.json"),
    meshTriangles: (job, opts, meshBytes) => call("mesh_triangles", job, opts || {}, meshBytes),
    // material-removal playback: open a session, then seek to any segment
    simOpen: (job, opts, meshBytes) => call("sim_open", job, opts || {}, meshBytes),
    simSeek: (n) => call("sim_seek", String(n)),
    // polylines (mm, Y up) from SVG text, placed with its box at opts.at and opts.height tall
    svgPaths: (svgText, opts) => call("svg_paths", svgText, opts || {}),
    // machining time of any G-code text: {total_s, feed_s, rapid_s, other_s, ...}
    estimateGcode: (gcode, machine) => call("estimate_gcode", gcode, machine || ""),
  };
}
