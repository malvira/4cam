# Embedding 4cam

4cam is meant to be easy for other software to embed, the way FDM slicers embed a slicing
engine. Every host gets the same engine, defaults and checks. The surfaces below are the
supported API. Everything else is internal and may change.

## The document

The document is a JSON project, `"schema_version": "0.2"`. It holds:

- models: a revolved profile and/or a mesh file;
- tools: inline, or references to a library;
- setups: machine, stock, holding, part, operations, and outputs (G-code files).

  Stock is a bar on the rotary axis (`{"shape": "cylinder", ...}`), the previous setup's
  result (`"from_previous"`), or a block in a vise (`{"shape": "box", "min": [x, y, z],
  "max": [x, y, z]}`, work coordinates, Z up, Z0 normally the top). Box setups take the 3-axis
  operation types (`face`, `pocket`, `drill`, `contour`, `raster3d`); bar setups take the
  rotary ones. A program can't mix the two. Example: `examples/plate/plate.project.json`.

0.1 jobs upgrade automatically. Machine profiles and tool libraries are separate JSON files
(`docs/profiles.md`).

## Rust (`fourcam` crate)

| Call | What it does |
| --- | --- |
| `project::Project::from_json_any(text)` | Parse a 0.1 job or 0.2 project. |
| `project::jobs_from_any_with(text, libraries, meshes)` | One plannable job per setup. |
| `plan_with_mesh(&job, mesh)` | Plan a setup into toolpaths (machine-neutral). |
| `post(&toolpath, &machine)` | Toolpath to G-code. |
| `verify_gcode(text, &limits)` | Unsafe-move check. |
| `profiles::check_limits` | Check a job against a machine profile's limits. |
| `gouge_check` | Check toolpaths against the part. |
| `ops::registry()` | Operation types and their parameters. |
| `ops::check` | Validate an operation. |
| `project::Project::op_defaults` / `ops::defaults` | A new operation with every field filled from the tool, spindle, machine limits and part faces. |
| `geometry::face_regions` | Pickable faces with stable ids. |
| `wizards::KnurlWizard` | The knurl wizard. |
| `sim::Playback` | Material-removal playback (rotary). |
| `sim3::Heightmap3` | Material removal for 3-axis programs (a Z heightmap of the block). |
| `project::simulate_project` | Simulate all setups in order. |

## WebAssembly (`web/pkg/fourcam.js`)

All calls take and return JSON. No wasm-bindgen is needed.

- **Planning and posting:**
  - `planAndPost(doc, machine, mesh, {setup, libraries})` returns G-code, issues, the limit
    check, the gouge check and a toolpath polyline. Polyline points are `[x, r, a, kind]` for
    rotary programs (kind 0 rapid, 1 feed) and `[x, y, z, kind]` for 3-axis ones (kind 2
    rapid, 3 feed); box setups also return `stock_box`.
- **Operations:**
  - `opTypes()` lists the operation types;
  - `opDefaults(doc, {type, setup, tool, geometry, machine}, mesh)` returns a new operation
    with every field filled;
  - `checkProject(doc)` upgrades and validates a document.
- **Geometry and wizards:**
  - `faceRegions(doc, model, mesh)` returns the model's faces with stable ids;
  - `wizardKnurl(doc, params, mesh)` runs the knurl wizard.
- **Simulation:**
  - `simulate(doc, {dx, na, cell}, mesh)` simulates material removal for the whole job.
    Box setups return `"kind": "xyz"`, an X/Y `grid` (`cell` mm, default about 240 cells
    along the longer side), the part top per cell and the block tops `z` after each program;
  - `simOpen(doc, {setup, dx, na, cell}, mesh)` and `simSeek(n)` play it back move by move
    (for box setups the changed rows are Y rows, with `z` and the operation tags).
- **Profiles:**
  - `profileList()` lists the built-in machines and tool libraries;
  - `importProfile(bytes, name)` reads a 4cam or Fusion 360 profile file.

## Python (`import fourcam`)

The same JSON API is available from Python as `plan_and_post`, `simulate`, `example_job`,
`machine` and `version`.

## CLI (`4cam`)

The CLI covers the same features:

- `run`, `simulate` and `verify`;
- `upgrade`;
- `ops list|show|check|defaults`;
- `faces`;
- `wizard knurl`;
- `profiles list|show|import|export`;
- `mesh-info`.
