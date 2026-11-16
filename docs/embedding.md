# Embedding 4cam

4cam is meant to be embedded the way slicers embed a slicing engine: one engine, the same
defaults and checks everywhere, called with JSON. Each language below gets the same
documents, the project in and the results out (`docs/api.md` lists the fields). Every
quickstart has a runnable copy in `examples/embed/`, and `sh scripts/test-embed.sh` runs
them all on the knob example.

| Host | Use | Example |
| --- | --- | --- |
| Rust | the `fourcam-core` crate (library `fourcam`) | `examples/embed/rust` |
| C, C++, anything with a C FFI | the C library `fourcam_c` and `fourcam.h` | `examples/embed/c` |
| .NET (SolidWorks add-ins, ...) | the C library through P/Invoke | `examples/embed/csharp` |
| Python (FreeCAD, scripts) | the `fourcam` wheel | `examples/embed/python` |
| JavaScript (Node, browsers, Electron) | the `fourcam` npm package (WebAssembly) | `examples/embed/node` |
| any language, as a process | the `4cam` command | below |

In every case:
- **Inputs:** a 0.2 project (or a 0.1 job) as JSON, the part mesh's bytes when it has one
  (STL or 3MF), and optionally a machine: a built-in id or a profile's JSON. Without one,
  the project's machine is used, else the Makera Z1.
- **Results:** plan-and-post returns one entry per program: `gcode`, `lines`, `issues`, the
  gouge and holder/shank checks, `time`, and a toolpath polyline for previews.
- **Errors:** an invalid project is an error result (`{"error": "..."}`, or an exception in
  Python), never a crash.
- **Building documents:** don't hand-write operations. `opTypes` lists every operation's
  fields, and `opDefaults` returns a new operation with every field filled from the tool,
  machine and part.

## Rust

```rust
use fourcam::{load_mesh, plan_with_mesh, post, profiles, project};
let libs = profiles::builtin_libraries();
for (setup, job) in project::jobs_from_any_with(&text, &libs, &|src| src.load(dir).ok())? {
    let mesh = load_mesh(&job, dir)?;
    let machine = profiles::builtin_machine(job.machine.as_deref().unwrap_or("makera-z1-4axis")).unwrap();
    for tp in plan_with_mesh(&job, mesh.as_ref())? {
        println!("{setup}/{}: {}", tp.name, post(&tp, &machine.post).lines().count());
    }
}
```

The full API is in `docs/api.md`: simulation (`simulate_job`, `sim3`), checks
(`gouge_check`, `collision_check`, `profiles::check_limits`), estimates (`estimate`) and
operations (`ops`).

## C and C++

```c
#include "fourcam.h"            /* crates/fourcam-capi/include */
char *v = fourcam_version();
puts(v);
fourcam_free(v);
char *r = fourcam_plan_and_post(project_json, NULL, mesh, mesh_len, NULL);
if (strncmp(r, "{\"error\"", 8) == 0) { /* report it */ }
/* ... parse r (JSON) with your JSON library ... */
fourcam_free(r);
```

- Link `fourcam_c`, built by `cargo build -p fourcam-capi --release` as
  `target/release/fourcam_c.dll` (+ `.dll.lib`), `libfourcam_c.so` or `.dylib`, with a
  static library beside it.
- Functions: `fourcam_plan_and_post`, `fourcam_simulate`, `fourcam_estimate_gcode`,
  `fourcam_op_types`, `fourcam_op_defaults`, `fourcam_check_project`, `fourcam_call` (any
  engine function by name), `fourcam_version` and `fourcam_abi_version`.
- Free every returned string with `fourcam_free`. NULL inputs count as empty. The functions
  can be called from any thread.

## C# / .NET

```csharp
[DllImport("fourcam_c")] static extern IntPtr fourcam_plan_and_post(
    [MarshalAs(UnmanagedType.LPUTF8Str)] string project, [MarshalAs(UnmanagedType.LPUTF8Str)] string? machine,
    byte[]? mesh, nuint meshLen, [MarshalAs(UnmanagedType.LPUTF8Str)] string? opts);
[DllImport("fourcam_c")] static extern void fourcam_free(IntPtr s);

IntPtr p = fourcam_plan_and_post(File.ReadAllText("knob.project.json"), null, null, 0, null);
try { using var doc = JsonDocument.Parse(Marshal.PtrToStringUTF8(p)!); /* doc.RootElement */ }
finally { fourcam_free(p); }
```

Put `fourcam_c.dll` next to the add-in, or on the DLL search path.

## Python

```python
import json, fourcam
project = json.load(open("knob.project.json"))
result = fourcam.plan_and_post(project, mesh=None)   # mesh: a path or bytes
for p in result["programs"]:
    print(p["name"], p["lines"], "lines,", len(p["issues"]), "issues")
open("out.nc", "w").write(result["programs"][0]["gcode"])
sim = fourcam.simulate(project, dx=0.1, na=360)
```

- Build the wheel with `python scripts/build_wheel.py`, or `maturin build` in
  `crates/fourcam-py`; see `docs/packaging.md`.
- One abi3 wheel serves Python 3.9 and later.

## JavaScript (Node, browsers)

```js
import { init } from "fourcam";            // the npm package: fourcam.wasm + a small wrapper
const cam = await init();
const result = cam.planAndPost(project, "", meshBytes);   // "": the project's machine
if (result.error) throw new Error(result.error);
for (const p of result.programs) console.log(p.name, p.lines, p.issues.length);
const sim = cam.simulate(project, { res: "normal" }, meshBytes);
```

- `scripts/build-npm.sh` builds the package into `target/pkg/npm`.
- In a page, `load(url)` takes your own URL for `fourcam.wasm`.
- Heavy calls block: run them in a Web Worker, as `web/app.js` does.

## The command line

```sh
4cam run knob.project.json --out out/          # G-code, previews, checks; exit code 1 on problems
4cam simulate knob.project.json --sim-res normal
4cam ops defaults rotary_finish knob.project.json
```

## Threads and memory

- The engine has no global state. Calls are independent and can run in parallel.
- Simulations take memory in proportion to their grid, and refuse grids over 40 million
  cells. `simGrid` gives a grid's size before you run it.
