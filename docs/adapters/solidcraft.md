# marcam in SolidCraft (plugin adapter)

`adapters/solidcraft` packages marcam as a [SolidCraft](https://github.com/tangency-labs/solidcraft)
command plugin: a WebAssembly component for SolidCraft's `craft:plugin@0.1.0` world
(capability `commands@0.1`). Each entry runs one marcam operation on a body in the open
design and writes the G-code to a folder the user granted. Nothing in SolidCraft changes:
the plugin uses SolidCraft's published WIT, manifest and commands only, and no SolidCraft code is
vendored or modified. The WIT file in `adapters/solidcraft/wit/` is an unchanged copy of
SolidCraft's `crates/craft-plugin/wit/plugin.wit` (Apache-2.0).

This is an experimental adapter, built against SolidCraft 0.43 (commit `31a14bac`), whose plugin
runtime is itself marked experimental. A minor version of `craft:plugin` 0.x may break this.

## What it does

1. SolidCraft draws the entry's dialog from its input schema. The plugin ships no UI.
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
5. It writes three files to the granted folder: `<name>.nc`, `<name>.project.json` and the
   body as `<name>.stl`. SolidCraft refuses any single plugin file write over 1 MiB, so a
   longer program is written gzipped as `<name>.nc.gz` (in the knob test, `raster3d`:
   131,760 lines). Unzip it before sending it to the machine. It returns a JSON summary: the files, line counts, the time
   estimate, issues and gouge depth.

Every entry has `effect: read`: it changes nothing in the design and can call only
`model.mesh`. Its only other reach is writing to the one named folder, and SolidCraft
stages those writes until the call succeeds.

`marcam run <name>.project.json` reproduces the program exactly. The project names the STL
and the machine, and marcam reads the STL the same way the plugin did.

## Entries

There is one entry per marcam operation type, generated from `ops::registry()`, with `_`
spelled `-`:

- `rotary-rough`, `rotary-finish`, `helical-knurl`, `flank-chamfer`;
- `face`, `pocket`, `drill`, `contour`, `raster3d`, `rough3d`, `engrave`, `vcarve` (the 3-axis
  operations run indexed, with `index_a`).

`rotary_turning` is left out because it needs a revolved profile, and the plugin gets a
mesh. `rotary-rough` covers the same work on a mesh. `parting` is left to a marcam project
(a cut-off belongs with the part's other operations, and SolidCraft caps a manifest at
16 KiB, about 1 KiB an entry).

Inputs. Only `bodies` is required; marcam defaults everything else.

| Input | Default |
| --- | --- |
| `bodies` | (required) one body |
| `tolerance` | 0.05 mm, the mesh chord tolerance passed to `model.mesh` |
| `axis` | `X`: the body's axis, through the component origin, that becomes the rotary axis |
| `machine` | `makera-z1-4axis`; any built-in marcam machine |
| `tool_kind` | the first tool kind the operation takes (flat, ball, vbit) |
| `tool_diameter` | 3.175 mm |
| `vbit_angle` | 90° (V-bit entries) |
| `stock_radius` | the body's largest radius + 0.5 mm, rounded up to 0.5 mm |
| `stock_length` | the body's length + 15 mm, rounded up |
| `spindle_rpm` | 12000 |
| `name` | `<body>_<operation>`; names the program and the files |
| every field of the operation | marcam's defaults for that tool, machine and part |

The manifest (`plugin.json`) is generated from the registry, so a new operation or field
appears without hand edits. SolidCraft caps a manifest at 16 KiB. The schemas therefore
carry types and minimums but no titles, and a test fails if the manifest outgrows the cap.

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
  - the manifest follows SolidCraft's rules (ids, entry count, list inputs, 16 KiB);
  - bad inputs are refused with the field named;
  - the body is placed and defaulted as described;
  - every entry, run on the knob fixture as `model.mesh` returns it, writes G-code
    byte-identical to `marcam run` on the project it wrote.
- `SOLIDCRAFT_CLI=... sh scripts/e2e-solidcraft.sh` does the real thing in one
  `solidcraft-cli` session. It points SolidCraft's data folder at a scratch directory, so
  nothing of the user's set-up is touched. In that session it:
  1. revolves a knob (the knob example's profile) about X;
  2. installs the zip with a grant on a scratch folder;
  3. runs every entry on it;
  4. compares each `.nc` with what `marcam run` makes from the written project.

  On 2026-11-16, against SolidCraft `31a14bac`, all 12 entries gave G-code byte-identical to
  `marcam run`, about 9 s for the whole session. SolidCraft meshed the knob to 971 triangles at
  the default tolerance.

  The check is skipped when `SOLIDCRAFT_CLI` is not set.

## Not done

- B-rep faces are not yet geometry selections. The part comes in only as a whole-body mesh.
  marcam's own picks (`geometry::face_regions`) work on that mesh, but there is no mapping from
  SolidCraft face ids yet.
- There is no turning from a profile. A revolved body could give its profile exactly, but
  SolidCraft does not hand one over.
- One operation per call. A multi-operation program means several calls; or edit the written
  project and use `marcam run`.
- SolidCraft's 1 MiB limit on a file write is why long programs come gzipped. A larger
  limit, or appending writes, in SolidCraft would remove that step.
- Results are files only. Nothing is stored in the design, because SolidCraft's `storage`
  capability is not out yet.
