# 4cam

An open-source CAM engine for 4th-axis (rotary) machining, written in Rust.

You describe the stock, the part (a revolved profile and/or a triangle mesh), the tools and
the operations in a JSON job file. 4cam plans machine-neutral toolpaths, posts them to G-code
for a machine profile, checks the result for unsafe moves, and renders a PNG preview of each
program.

## Status

- **Milestone 0**: reproduces, byte for byte, the knurled-knob programs that were cut on a
  Makera Z1 (`tests/golden/`).
- **Milestone 1** (rotary finishing): STL (binary and ASCII) and 3MF import, a radial
  drop-cutter for flat, ball and V-bit tools with a spatial index, the `rotary_finish`
  strategy, and a LinuxCNC profile that uses G93 inverse-time feeds.
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
target/release/4cam run examples/knob/knob.job.json --out out
```

```bash
target/release/4cam run examples/knob/knob_finish.job.json --machine linuxcnc --out out
```

The other commands:

- `4cam verify file.nc [--plunge 60] [--safe 15.5]` flags feed moves where Z falls faster
  than the plunge feed, and X/A rapids below the safe radius. It reads `G93` (inverse-time)
  files too.
- `4cam simulate job.json [--out dir] [--dx 0.05] [--na 720]` runs every program in order
  on one simulated bar (a radial heightmap) and compares the result with the part.
  - It reports rapids through stock, the material left and any gouges.
  - It writes `<program>_stock.png` for each program: a side view, and an unrolled map of
    the material left.
  - It skips tools without a full shape (a V-bit with no `diameter`).
- `4cam run` also gouge-checks each turning, roughing and finishing program against the part.
- `4cam toolpath job.json` prints the machine-neutral toolpath as JSON.
- `4cam revolve job.json part.stl [--segments 720]` writes the job's profile out as an STL.
- `4cam mesh-info part.3mf` prints the triangle count, the bounding box and the largest
  radius about X.

Built-in machines are `makera-z1` (the default) and `linuxcnc`. To use another machine,
pass `--machine path/to/machine.json` (see `machines/`).

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
pip install target/wheels/fourcam-0.1.0-cp39-abi3-win_amd64.whl
```

The wheel name depends on your platform. To use it:

```python
import fourcam
res = fourcam.plan_and_post(fourcam.example_job(), machine="linuxcnc")
print(res["programs"][0]["gcode"][:200])
```

WebAssembly: `scripts/build-wasm.sh` (or `.ps1`) writes `web/pkg/fourcam.wasm` and
`fourcam.js`. The module has plain exports, so wasm-bindgen is not needed. `fourcam.js`
wraps them for browsers and Node:

```js
import { load } from "./pkg/fourcam.js";
const cam = await load(new URL("./pkg/fourcam.wasm", import.meta.url));
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

- `crates/fourcam-core`: the library (`fourcam`). It holds the job schema, geometry, mesh
  I/O, the drop-cutter, the strategies, the toolpath IR, the post-processor, the verifier
  and the PNG previews.
- `crates/fourcam-cli`: the `4cam` binary.
- `crates/fourcam-wasm`: the WebAssembly build and its JSON API (`js/fourcam.js` wraps it).
- `crates/fourcam-py`: the Python bindings (pyo3, abi3, Python 3.9 or later).
- `scripts/`: the build and check scripts.
- `machines/`: machine profiles.
- `examples/`: example jobs and their outputs.
- `reference/`: the original scripts the engine was ported from.

## License

Apache-2.0. See `LICENSE`.
