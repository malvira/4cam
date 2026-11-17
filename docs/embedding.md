# Embedding marcam

marcam is meant to be embedded the way slicers embed a slicing engine: one engine, the same
defaults and checks everywhere, called with JSON. Each language below gets the same
documents, the project in and the results out (`docs/api.md` lists the fields). Every
quickstart has a runnable copy in `examples/embed/`, and `sh scripts/test-embed.sh` runs
them all on the knob example.

| Host | Use | Example |
| --- | --- | --- |
| Rust | the `marcam-core` crate (library `marcam`) | `examples/embed/rust` |
| C, C++, anything with a C FFI | the C library `marcam_c` and `marcam.h` | `examples/embed/c` |
| .NET (SolidWorks add-ins, ...) | the C library through P/Invoke | `examples/embed/csharp` |
| Python (FreeCAD, scripts) | the `marcam` wheel | `examples/embed/python` |
| JavaScript (Node, browsers, Electron) | the `marcam` npm package (WebAssembly) | `examples/embed/node` |
| any language, as a process | the `marcam` command | below |
| FreeCAD | the `marcam_freecad` adapter on the wheel | `adapters/freecad`, `docs/adapters/freecad.md` |
| SolidCraft | the WebAssembly plugin | `adapters/solidcraft`, `docs/adapters/solidcraft.md` |

CAD adapters share one engine function, `body_project` (Rust `marcam::body`, Python
`marcam.body_project`, or `call("body_project", ...)`). It builds a one-setup project round a
part mesh, with every field defaulted, so every CAD host places the body and picks the stock
and tool the same way.

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
use marcam::{load_mesh, plan_with_mesh, post, profiles, project};
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
#include "marcam.h"            /* crates/marcam-capi/include */
char *v = marcam_version();
puts(v);
marcam_free(v);
char *r = marcam_plan_and_post(project_json, NULL, mesh, mesh_len, NULL);
if (strncmp(r, "{\"error\"", 8) == 0) { /* report it */ }
/* ... parse r (JSON) with your JSON library ... */
marcam_free(r);
```

- Link `marcam_c`, built by `cargo build -p marcam-capi --release` as
  `target/release/marcam_c.dll` (+ `.dll.lib`), `libmarcam_c.so` or `.dylib`, with a
  static library beside it.
- Functions: `marcam_plan_and_post`, `marcam_simulate`, `marcam_estimate_gcode`,
  `marcam_op_types`, `marcam_op_defaults`, `marcam_check_project`, `marcam_call` (any
  engine function by name), `marcam_version` and `marcam_abi_version`.
- Free every returned string with `marcam_free`. NULL inputs count as empty. The functions
  can be called from any thread.

## C# / .NET

```csharp
[DllImport("marcam_c")] static extern IntPtr marcam_plan_and_post(
    [MarshalAs(UnmanagedType.LPUTF8Str)] string project, [MarshalAs(UnmanagedType.LPUTF8Str)] string? machine,
    byte[]? mesh, nuint meshLen, [MarshalAs(UnmanagedType.LPUTF8Str)] string? opts);
[DllImport("marcam_c")] static extern void marcam_free(IntPtr s);

IntPtr p = marcam_plan_and_post(File.ReadAllText("knob.project.json"), null, null, 0, null);
try { using var doc = JsonDocument.Parse(Marshal.PtrToStringUTF8(p)!); /* doc.RootElement */ }
finally { marcam_free(p); }
```

Put `marcam_c.dll` next to the add-in, or on the DLL search path.

## Python

```python
import json, marcam
project = json.load(open("knob.project.json"))
result = marcam.plan_and_post(project, mesh=None)   # mesh: a path or bytes
for p in result["programs"]:
    print(p["name"], p["lines"], "lines,", len(p["issues"]), "issues")
open("out.nc", "w").write(result["programs"][0]["gcode"])
sim = marcam.simulate(project, dx=0.1, na=360)
```

- Build the wheel with `python scripts/build_wheel.py`, or `maturin build` in
  `crates/marcam-py`; see `docs/packaging.md`.
- One abi3 wheel serves Python 3.9 and later.

## JavaScript (Node, browsers)

```js
import { init } from "marcam";            // the npm package: marcam.wasm + a small wrapper
const cam = await init();
const result = cam.planAndPost(project, "", meshBytes);   // "": the project's machine
if (result.error) throw new Error(result.error);
for (const p of result.programs) console.log(p.name, p.lines, p.issues.length);
const sim = cam.simulate(project, { res: "normal" }, meshBytes);
```

- `scripts/build-npm.sh` builds the package into `target/pkg/npm`.
- In a page, `load(url)` takes your own URL for `marcam.wasm`.
- Heavy calls block: run them in a Web Worker, as `web/app.js` does.

## The command line

```sh
marcam run knob.project.json --out out/          # G-code, previews, checks; exit code 1 on problems
marcam simulate knob.project.json --sim-res normal
marcam ops defaults rotary_finish knob.project.json
```

## Threads and memory

- The engine has no global state. Calls are independent and can run in parallel.
- Simulations take memory in proportion to their grid, and refuse grids over 40 million
  cells. `simGrid` gives a grid's size before you run it.
