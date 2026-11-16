// The fourcam package's entry: `init()` loads the bundled fourcam.wasm (from disk in Node,
// by URL in a browser or bundler); `load(source)` takes your own URL or bytes.
import { load } from "./fourcam.js";

export { load };

export async function init() {
  const url = new URL("./fourcam.wasm", import.meta.url);
  if (typeof process !== "undefined" && process.versions?.node && url.protocol === "file:") {
    const { readFile } = await import("node:fs/promises");
    return load(await readFile(url));
  }
  return load(url);
}
