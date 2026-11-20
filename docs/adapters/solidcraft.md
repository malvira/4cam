# marcam in SolidCraft (plugin adapter)

`adapters/solidcraft` packages marcam as a [SolidCraft](https://github.com/tangency-labs/solidcraft)
command plugin: a WebAssembly component for SolidCraft's `craft:plugin@0.1.0` world
(capability `commands@0.1`). Its `plan` entry runs any marcam operation on a body in the
open design and writes the G-code to a folder the user granted; `describe` lists the
operations and their fields; `plan-project` plans a whole marcam project from a folder. Nothing in SolidCraft changes:
the plugin uses SolidCraft's published WIT, manifest and commands only, and no SolidCraft code is
vendored or modified. The WIT file in `adapters/solidcraft/wit/` is an unchanged copy of
SolidCraft's `crates/craft-plugin/wit/plugin.wit` (Apache-2.0).

This is an experimental adapter, built against SolidCraft 0.43 (commit `31a14bac`), whose plugin
runtime is itself marked experimental. A minor version of `craft:plugin` 0.x may break this.

## What it does

1. An agent (over MCP) or `solidcraft-cli` calls an entry with JSON inputs. The plugin ships
   no UI. SolidCraft's own dialog draws only scalar fields, so it does not open these entries:
   every planning entry takes `bodies`, a list.
2. The plugin calls `model.mesh {body, tolerance}`, SolidCraft's read-only mesh of the
   evaluated body (component-local mm, at most 8192 vertices).
3. It builds a one-setup marcam project with the engine's `marcam::body::body_project`, which
   the FreeCAD adapter shares:
   - the body as a mesh model, its chosen axis turned onto the rotary X axis and its front
     (largest X) moved to X0;
   - bar stock, the tool, and the operation filled by marcam's own defaults (`op_defaults`),
     with any operation fields the user typed on top.
4. It plans and posts the project with marcam (the same calls as `marcam run`), and checks the
   machine limits, the G-code and gouges.
5. It writes three files to the granted folder: `<name>.nc`, `<name>.marcam.toml` and the
   body as `<name>.stl`. SolidCraft refuses any single plugin file write over 1 MiB, so a
   longer program is written gzipped as `<name>.nc.gz` (in the knob test, `raster3d`:
   131,760 lines). Unzip it before sending it to the machine. It returns a JSON summary: the files, line counts, the time
   estimate, issues and gouge depth.

Every entry has `effect: read`: it changes nothing in the design and can call only
`model.mesh`. Its only other reach is writing to the one named folder, and SolidCraft
stages those writes until the call succeeds.

`marcam run <name>.marcam.toml` reproduces the program exactly. The project names the STL
and the machine, and marcam reads the STL the same way the plugin did.

## Entries

SolidCraft caps a manifest at 16 KiB and 32 entries, and its schemas are fixed in the
manifest. One entry per operation (about 1 KiB each) outgrew the cap at 15 operations, so
the operations share one entry and the plugin describes them at run time:

| Entry | Does |
| --- | --- |
| `plan` | Any operation (`operation`, its fields in `params`) on one body; writes `<name>.nc`, `<name>.marcam.toml` and `<name>.stl` |
| `describe` | The operations the plugin plans, each with its title, tool kinds and fields (unit, range, choices, help), the inputs, and the machines; `operation` limits it to one. Reads and writes nothing |
| `plan-project` | The marcam project at `project` (a path in the `project` folder; its mesh files beside it): every setup's programs, written as `<program>.nc` |
| `rotary-rough`, `rotary-finish`, `helical-knurl`, `flank-chamfer` | `plan` for that operation, its fields at the top level (as the earlier one-entry-per-operation plugin took them) |

`plan` takes every operation but `rotary_turning`, which needs a revolved profile; the
plugin gets a mesh, and `rotary_rough` covers the same work on one. A new operation costs the
manifest only its name; the manifest is about 6.7 KiB.

```json
{"entry": "plan", "params": {"bodies": ["Knob"], "operation": "pocket",
  "params": {"adaptive": true, "stepover": 0.15, "depth": 2}}}
```

Inputs of `plan` (and the four operations' own entries). Only `bodies` (and for `plan`,
`operation`) is required; marcam defaults everything else.

| Input | Default |
| --- | --- |
| `bodies` | (required) one body |
| `operation` | (`plan`, required) the operation type, e.g. `pocket` or `rotary_rough` |
| `params` | (`plan`) the operation's fields, as `describe` lists them; a field also given at the top level is refused |
| `tolerance` | 0.05 mm, the mesh chord tolerance passed to `model.mesh` |
| `axis` | `X`: the body's axis, through the component origin, that becomes the rotary axis |
| `machine` | `makera-z1-4axis`; any built-in marcam machine |
| `tool_kind` | the first tool kind the operation takes (flat, ball, vbit) |
| `tool_diameter` | 3.175 mm |
| `vbit_angle` | 90° |
| `stock_radius` | the body's largest radius + 0.5 mm, rounded up to 0.5 mm |
| `stock_length` | the body's length + 15 mm, rounded up |
| `spindle_rpm` | 12000 |
| `name` | `<body>_<operation>`; names the program and the files |

A wrong field is refused as `invalid-input` naming it. The four operations' own entries keep
their fields typed in the manifest, but for the ramp angle, smoothing, regions, tabs,
direction, leads, ramp and rest tool (marcam's defaults; `plan` takes them all).

Permissions: write on the `output` folder; read on the `project` folder (`plan-project`
only). The plugin calls one SolidCraft command, `model.mesh`, and nothing else.

Limits asked for: 100,000,000,000 fuel (the default host ceiling), 128 MiB and a 30 s
deadline. There are no command groups and no network.

## Build

```sh
sh scripts/build-solidcraft-plugin.sh
```

The script:

1. builds the guest for `wasm32-unknown-unknown` with `wit-bindgen`;
2. wraps it as a component with no WASI adapter (`wit-component`, in `src/bin/pack.rs`);
3. writes `target/solidcraft-plugin/marcam.zip`, holding `plugin.json` and `plugin.wasm`.

## Install and run (headless)

The steps below use `solidcraft-cli`. The desktop app does the same through
Preferences > Plugins and the Plugins menu.

1. Inspect the package to get its approval hash. This runs no plugin code.

   ```sh
   solidcraft-cli exec plugin.inspect '{"path": "target/solidcraft-plugin/marcam"}'
   ```

2. Install it and grant the output folder, then run an entry. Grants live in the session, so
   do both in one script:

   ```json
   {"commands": [
    {"command": "plugin.install", "params": {"path": "marcam.zip", "approval": "<hash>",
      "approve": {"command_groups": [], "folders": [{"name": "output", "path": "C:/cam/out", "read": false, "write": true}]}}},
    {"command": "plugin.run", "params": {"path": "<plugins folder>/marcam", "approval": "<hash>",
      "entry": "rotary-finish", "params": {"bodies": ["Knob"]}}}
   ]}
   ```

## Proof

- `cargo test -p marcam-solidcraft` checks three things:
  - the manifest follows SolidCraft's rules (ids, entry count, list inputs) and stays under
    8 KiB;
  - `describe` lists every operation and its fields;
  - bad inputs are refused with the field named;
  - the body is placed and defaulted as described;
  - `plan`, for every operation, run on the knob fixture as `model.mesh` returns it, writes
    G-code byte-identical to `marcam run` on the project it wrote; the four operations' own
    entries write what `plan` does; `plan-project`, reading that project back from its folder,
    writes the same G-code; it reads nothing outside its folder.
- `SOLIDCRAFT_CLI=... sh scripts/e2e-solidcraft.sh` does the real thing in one
  `solidcraft-cli` session. It points SolidCraft's data folder at a scratch directory, so
  nothing of the user's set-up is touched. In that session it:
  1. revolves a knob (the knob example's profile) about X;
  2. installs the zip with write and read grants on a scratch folder;
  3. runs `plan` for every operation, the four operations' own entries, `describe`, and
     `plan-project` on a project `plan` wrote;
  4. compares each `.nc` with what `marcam run` makes from the written project.

  On 2026-11-20, against SolidCraft `31a14bac`, `plan` gave G-code byte-identical to
  `marcam run` for all 15 operations, and the own entries and `plan-project` matched it.
  Byte-identical across WebAssembly and native needs the engine's transcendental math from the
  `libm` crate (`marcam_core::det`): the platform's C library differed in the last bits and
  once flipped two points of the knob's rotary finish.

  The check is skipped when `SOLIDCRAFT_CLI` is not set.

## Not done

- B-rep faces are not yet geometry selections. The part comes in only as a whole-body mesh.
  marcam's own picks (`geometry::face_regions`) work on that mesh, but there is no mapping from
  SolidCraft face ids yet.
- There is no turning from a profile. A revolved body could give its profile exactly, but
  SolidCraft does not hand one over.
- `plan` is one operation per call. For several, edit the written project and run
  `plan-project` (or `marcam run`).
- SolidCraft's 1 MiB limit on a file write is why long programs come gzipped. A larger
  limit, or appending writes, in SolidCraft would remove that step.
- Results are files only. Nothing is stored in the design, because SolidCraft's `storage`
  capability is not out yet.
