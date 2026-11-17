# marcam

An open-source CAM engine for 4th-axis (rotary) machining, written in Rust.

> **Renamed:** this project was called 4cam until 2026-11-17. The crates are now `marcam-*`
> (library `marcam`), the command is `marcam` (`4cam` still works), and projects save as
> `.marcam.json`. Files from before (`.4cam.json`, `4cam-machine` / `4cam-tools` profiles)
> still open. See [CHANGELOG.md](CHANGELOG.md).

You describe the stock, the part (a revolved profile and/or a triangle mesh), the tools and
the operations in a JSON job file. marcam plans machine-neutral toolpaths, posts them to G-code
for a machine profile, checks the result for unsafe moves, and renders a PNG preview of each
program.

## Status

- **Milestone 0**: reproduces, byte for byte, the knurled-knob programs that were cut on a
  Makera Z1 (`tests/golden/`).
- **Milestone 1** (rotary finishing): STL (binary and ASCII) and 3MF import, a radial
  drop-cutter for flat, ball and V-bit tools with a spatial index, the `rotary_finish`
  strategy, and a LinuxCNC profile that uses G93 inverse-time feeds.
- **Profiles:**
  - machine profiles for the Makera Z1, Carvera and Carvera Air (each with 4th axis) and a
    generic LinuxCNC machine;
  - a Makera starter tool library;
  - importers for Makera's Fusion 360 tool libraries and machine definitions;
  - checks of each job against the machine's limits.

  See [docs/profiles.md](docs/profiles.md) and [docs/roadmap.md](docs/roadmap.md).
- **Milestone 3** (web frontend): a static web app that runs the engine in the browser
  (WebAssembly in a Web Worker). It is tested end to end in headless Chrome/Edge.
- **Milestone 2** (roughing and simulation):
  - `rotary_rough`: radial step-down roughing from a mesh.
  - A radial-heightmap stock model that simulates material removal.
  - Gouge checks against the part.
  - Python and WebAssembly packages.

## Operations

- `rotary_turning`: roughs the stock down to the profile in radial levels with a flat
  endmill moving helically along X, then finishes it in one helical pass.
- `helical_knurl`: a diamond knurl, cut as families of helical V grooves over a conical body.
- `flank_chamfer`: chamfers an edge with the flank of a V-bit while the part turns.
- `rotary_rough`: roughs bar stock down to the mesh plus an `allowance`.
  - It cuts in radial levels `step_down` apart; each level is a helix advancing `step_x`
    per turn.
  - At every point the tool sits at the lower of the level and the drop-cutter surface, so a
    level never cuts more than one step and never into the allowance.
  - Levels alternate direction so A stays bounded.
- `rotary_finish`: finishes the part mesh with a radial drop-cutter. The tool can be flat,
  ball or V-bit. There are two patterns:
  - `along`: rasters along X at a constant A. Set the stepover with `stepover_deg`, or with
    `stepover_mm` (measured at the largest radius).
  - `around`: one helix around the axis, advancing `stepover_mm` per turn.

  Straight moves are split wherever the surface rises more than `tolerance` (default
  0.005 mm) above them. At a vertical wall the tool climbs before it crosses. `around` adds
  one full turn at the foot of each wall (`clean_walls`, on by default), because a helix only
  reaches a wall's foot at one angle per turn. If the job has
  no `part.mesh`, `rotary_finish` uses `part.profile`, revolved into a 720-facet mesh.

## Usage

Build once:

```bash
cargo build --release
```

Plan a job. This writes one `.nc` and one `.png` preview per program and verifies each
program:

```bash
target/release/marcam run examples/knob/knob.job.json --out out
```

```bash
target/release/marcam run examples/knob/knob_finish.job.json --machine linuxcnc --out out
```

The other commands:

- Toolpath smoothing: operations that make dense paths take a `smoothing` tolerance (engine
  default 0.005 mm for finishing, 0.01 for roughing, 0 off). Points are dropped while the
  path stays within it (measured square to the move and along the tool, so a steep wall is
  not cut into), rotary moves are reduced in X, radius and arc length (at most 45 degrees of
  A per move), corners at one Z are rounded where the blend is worth it, and on a vise the
  post writes circular runs as G2/G3 (each replaced move within the tolerance of the arc).
  The knob example keeps `smoothing: 0`, so its programs stay the ones that were cut.
- `marcam verify file.nc [--plunge 60] [--safe 15.5] [--ramp 30]` flags feed moves where Z
  falls faster than the plunge feed, and X/A rapids below the safe radius. It reads `G93`
  (inverse-time) files too. Like the planner, it lets descents gentler than the ramp angle
  (default 30 degrees, each operation's `ramp_angle`) run at any feed; steeper descents and
  plunges keep to the plunge feed. `--ramp 0` holds every descent to it.
- `marcam simulate job.json [--out dir] [--dx 0.05] [--na 720]` runs every program in order
  on one simulated bar (a radial heightmap) and compares the result with the part.
  - It reports rapids through stock, the material left and any gouges.
  - It writes `<program>_stock.png` for each program: a side view, and an unrolled map of
    the material left.
  - It skips tools without a full shape (a V-bit with no `diameter`).
- `marcam run` also gouge-checks each turning, roughing and finishing program against the part.
- `marcam toolpath job.json` prints the machine-neutral toolpath as JSON.
- `marcam mesh-info part.3mf` prints the triangle count, the bounding box and the largest
  radius about X.

Built-in machines are `makera-z1-4axis` (the default), `makera-carvera-4axis`,
`makera-carvera-air-4axis` and `linuxcnc-4axis`. The short names `makera-z1`, `carvera`,
`carvera-air` and `linuxcnc` work too.

The machine comes from `--machine`, else the job's `"machine"`, else the Z1. `--machine`
also takes a profile file or a URL. `run` checks the job against the machine's limits.

Profiles and tool libraries:

- `marcam profiles list` shows the built-in and imported profiles and libraries.
- `marcam profiles show <id>` prints a profile.
- `marcam profiles import <file-or-url>` reads marcam JSON, Fusion 360 `.tools`/`.json` tool
  libraries, or `.mch`/`.f3d` machines. It saves the result to the user profile folder.
- `marcam profiles export <id> --out file.json` writes a profile out.

Jobs can use library tools:

```json
{"number": 1, "library": "makera-starter", "tool": "spiral-o-metal-3.175x12"}
```

See [docs/profiles.md](docs/profiles.md).

## Web app

Build the engine and serve the repository. Either command does both:

```bash
sh web/serve.sh
```

```powershell
powershell -File web/serve.ps1
```

Then open <http://localhost:8000/web/>. Nothing is uploaded: the engine runs in your
browser.

- **Project bar** (top):
  - **New**: a dialog with the project name, the machine and the setup type: Rotary 4th
    axis (simultaneous), Indexed 3+1 (the bar at fixed angles) or 3-axis vise. It also takes
    the part (an STL, its units, its axis or up direction, with a placement preview) and the
    stock (from the part with a margin, or given). The STL can also come later.
  - **Open**: a `.marcam.json` project file, or a `.project.json` / `.job.json`; or drop the
    file on the page.
  - **Save** / **Save as**: one `.marcam.json` file with the part mesh embedded, so it opens
    whole anywhere.
  - **Import STL** (or drop one), binary or ASCII, mm or inch, through the same dialog, with
    the current setup's type pre-selected:
    - The part is placed: on the rotary axis, its front at X0; or Z up on the vise.
    - Bar or block stock is sized round it, and the part size is shown.
    - A new part gets a roughing and a finishing operation from the engine's defaults; an
      existing project's operations are recomputed for it. Then it is generated.
  - In the project tree, **Edit setup** changes a setup's type, machine, stock, origin and
    holding. A new type converts the setup: the part is placed again, the stock re-sized, and
    operations that do not fit are removed (with a warning first and an Undo after; starters
    are added if none are left). The rest get their defaults again, and everything
    regenerates. **+ Setup** adds a setup for the same part, with the same choices. Only
    operation types that fit a setup are offered.
  - The project's name, with an "unsaved" mark. New, Open and examples ask before
    discarding changes. Results the project no longer matches are marked out of date until
    Generate.
  - The example picker.
- **Left panel:**
  - the machine (built-in Makera Z1, Carvera, Carvera Air, LinuxCNC, imported profiles, or custom JSON), with a summary of its limits;
  - a tool library panel: view, add, edit and remove tools, import a file or URL (Fusion 360 `.tools`/`.json`/`.mch`/`.f3d` or marcam JSON), export JSON, and "use as T#" to put a tool in the job (kept in browser storage);
  - "Attach mesh file": the mesh an opened project names (STL or 3MF), as it is;
  - quick settings that write into the job JSON (stock, tool sizes, feeds, steps);
  - the job JSON editor.
- **Generate:** shows one tab per program with:
  - the line count, verifier issues and gouge check;
  - the G-code, with **Download .nc**;
  - a 3D view (three.js, loaded from jsDelivr), with the toolpath wrapped around the part
    (cuts blue, rapids red), the stock and the part, and a move scrubber;
  - a 2D side view.
- **Simulate stock:** runs the material-removal simulation and shows the stock surface,
  coloured by the material left on the part.

`node web/tests/e2e.mjs` drives the page in an installed Chrome or Edge in headless mode,
over the DevTools protocol, with no npm packages. It checks:

- the knob programs match the golden G-code byte for byte, with 0 issues;
- the 3D view renders and the scrubber works;
- **Download .nc** saves the right file;
- the simulation shows no gouges;
- the LinuxCNC post uses G93;
- quick settings and error messages work;
- the mesh rough/finish job;
- the phone layout.

The test saves screenshots to `web/tests/screenshots/`.

## Python and WebAssembly

Both packages expose the same JSON API:

- `plan_and_post(job, machine, mesh)` returns each program's G-code, verifier issues, gouge
  check and a tool-tip polyline.
- `simulate(job, ...)` returns the stock heightmap after each program.
- There are also `example_job`, `machine` and `version`.

Python: build the wheel with the standard library only, then install it:

```bash
python scripts/build_wheel.py
```

```bash
pip install target/wheels/marcam-0.1.0-cp39-abi3-win_amd64.whl
```

The wheel name depends on your platform. To use it:

```python
import marcam
res = marcam.plan_and_post(marcam.example_job(), machine="linuxcnc")
print(res["programs"][0]["gcode"][:200])
```

WebAssembly: `scripts/build-wasm.sh` (or `.ps1`) writes `web/pkg/marcam.wasm` and
`marcam.js`. The module has plain exports, so wasm-bindgen is not needed. `marcam.js`
wraps them for browsers and Node:

```js
import { load } from "./pkg/marcam.js";
const cam = await load(new URL("./pkg/marcam.wasm", import.meta.url));
const res = cam.planAndPost(cam.exampleJob(), "makera-z1");
```

`scripts/check.sh` runs everything:

- `cargo fmt` and `cargo clippy`;
- the Rust tests;
- the WebAssembly package under Node, checked against the golden G-code;
- the Python wheel, installed in a fresh venv and tested.

## Examples

`examples/knob/` has the jobs, the revolved `knob.stl` and, in `output/`, the G-code and
PNG previews they produce, and in `output/sim/` the simulated stock after each program. You
can look at all of these without running anything. Each preview
has two panels: a side view (X against radius, with the finished part filled in grey) and
an unrolled view (X against A, coloured by radius).

## Coordinates

- A rotates the part about X, in degrees. Positive A turns it right-handed about +X.
- X0 is the front face of the part, which extends toward -X.
- Y0/Z0 is the rotation axis, so Z is the radius at the tool tip.
- Units are mm.

A mesh goes on the axis with `part.mesh.axis` (`"x"`, `"y"` or `"z"`: the mesh axis that
becomes X) and `part.mesh.translate`.

## Tests

```bash
cargo test
```

The tests cover:

- the golden G-code;
- the drop-cutter against a brute-force search, and against the analytic knob profile
  (worst error about 1e-4 mm);
- finishing paths, which must never dip more than 0.01 mm below the part;
- the simulated stock after mesh roughing and finishing: no gouges, the allowance kept, and
  the body within 0.06 mm of the part;
- G93 feed arithmetic;
- the mesh import formats;
- the CLI end to end.

## Layout

- `crates/marcam-core`: the library (`marcam`). It holds the job schema, geometry, mesh
  I/O, the drop-cutter, the strategies, the toolpath IR, the post-processor, the verifier
  and the PNG previews.
- `crates/marcam-cli`: the `marcam` binary.
- `crates/marcam-wasm`: the WebAssembly build and its JSON API (`js/marcam.js` wraps it).
- `crates/marcam-py`: the Python bindings (pyo3, abi3, Python 3.9 or later).
- `web/`: the browser app (`index.html`, `app.js`, `worker.js`) and its end-to-end test.
- `scripts/`: the build and check scripts.
- `crates/marcam-core/profiles/`: built-in machine profiles and tool libraries
  (`profiles/_downloads/` at the root is a git-ignored cache of vendor files).
- `docs/`: the profile formats, Makera sources and the roadmap;
  [docs/embedding.md](docs/embedding.md) for embedding marcam (Rust, C/C++, C#/.NET, Python,
  JavaScript; runnable examples in `examples/embed/`) and
  [docs/packaging.md](docs/packaging.md) for the crates, wheel and npm package (prepared,
  not published).
- `examples/`: example jobs and their outputs.
- `tests/fixtures/`: test fixtures. These are the knob's original Python part and G-code
  generators, and a synthetic Fusion-format tool library. `crates/marcam-fixtures`
  regenerates `examples/knob/knob.stl`. Part generation is a fixture concern, not an engine
  feature.

## License

Apache-2.0. See `LICENSE`.
