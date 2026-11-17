// Types for the marcam package. Every function returns a JSON-able object; a failure is
// { error: string } (see docs/api.md in the repository for each result's fields).
export type Json = any;
/** A project or job: an object, or its JSON text. */
export type Doc = object | string;
/** Mesh bytes (STL or 3MF). */
export type Bytes = Uint8Array | ArrayBuffer;

export interface Marcam {
  version(): Json;
  exampleJob(): Json;
  machine(name?: string): Json;
  planAndPost(doc: Doc, machine?: string | object, mesh?: Bytes, opts?: object): Json;
  simulate(doc: Doc, opts?: object, mesh?: Bytes): Json;
  profileList(): Json;
  opTypes(): Json;
  checkProject(doc: Doc): Json;
  faceRegions(doc: Doc, model: string, mesh?: Bytes): Json;
  wizardKnurl(doc: Doc, params: object, mesh?: Bytes): Json;
  opDefaults(doc: Doc, request: object, mesh?: Bytes): Json;
  importProfile(bytes: Bytes, name: string): Json;
  meshTriangles(doc: Doc, opts?: object, mesh?: Bytes): Json;
  simOpen(doc: Doc, opts?: object, mesh?: Bytes): Json;
  simSeek(n: number): Json;
  simClose(): Json;
  svgPaths(svg: string, opts?: object): Json;
  estimateGcode(gcode: string, machine?: string | object): Json;
  simGrid(doc: Doc, opts?: object, mesh?: Bytes): Json;
  gpuOpen(doc: Doc, opts?: object, mesh?: Bytes): Json;
  gpuStats(opts: object, keys: Bytes): Json;
}

/** Load the bundled engine (Node: from disk; browser: next to this module). */
export function init(): Promise<Marcam>;
/** Load the engine from a URL or the wasm bytes. */
export function load(source: string | URL | Bytes): Promise<Marcam>;
