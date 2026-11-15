# Roadmap

4cam is meant to become a general-purpose CAM engine. For now it stays focused on 4th-axis
(rotary) machining, but the interfaces are being shaped so that focus doesn't box it in.

## Done

- **M0:** reproduce the knurled-knob programs cut on a Makera Z1, byte for byte.
- **M1:** rotary finishing from meshes (STL/3MF), a radial drop-cutter, and the LinuxCNC G93
  post.
- **M2:** mesh roughing, material-removal simulation, gouge checks, and Python and WebAssembly
  packages.
- **M3:** a web app on the WebAssembly build.
- **Profiles:** machine profiles and tool libraries, Makera first, with Fusion 360 importers
  and machine-limit checks.

## Staying general

- **Machines.** A profile describes linear axes with travels, plus zero or more rotary axes,
  each with an axis of rotation, letter, limits or wrap, and whether it carries the work or
  the tool (`docs/profiles.md`). That covers 3-axis mills, 3+1 rotary machines, and later
  5-axis trunnion and head-head machines without a format change.
- **Tools.** Tool libraries don't depend on the machine type. Shapes the strategies don't use
  yet (tapered ball, drill) are stored already.
- **Toolpaths.** The toolpath IR is machine-neutral. Today a move is `(x, r, a)`, the radial
  form of a 4-axis move. The plan is to add a general move with a tool-tip position and a tool
  axis `(x, y, z, i, j, k)` beside it. Rotary strategies keep emitting `(x, r, a)`, which
  converts exactly. Kinematics-aware posts then turn general moves into axis words for each
  machine, including the inverse kinematics for rotary tables and heads.
- **Operations.** These stay independent of the machine. Planning reads only the kinematic
  capabilities it needs, and the limit check reports what a given machine can't do.

## General-purpose architecture (approved for prototyping, branch `arch-0.2`)

**Model.**

- A **Project** (schema 0.2) holds Models, tool library references and Setups. 0.1 jobs
  upgrade automatically.
- A **Setup** has:
  - a machine;
  - stock (cylinder, box or mesh);
  - a work origin and orientation;
  - holding (rotary chuck with or without tailstock, or a vise);
  - fixture keep-outs.

  Stock carries from one setup to the next.
- Each Setup has an ordered list of **Operations**: type, tool reference (library and tool
  id, with feed and speed overrides), geometry selection, parameters, colour and an enabled
  flag.
- **Operation types** are registered plug-ins. Each declares its parameter schema, the tool
  shapes it accepts, the geometry kinds it accepts, the machine abilities it needs, and a
  planner. Hosts query the registry to build their UI.
- **Geometry selection** can be:
  - the whole model;
  - mesh face regions with stable ids (the mesh is split at sharp edges, and hosts may pass
    their own face groups);
  - axial or angular ranges;
  - a revolved profile;
  - curves or 2D paths;
  - containment boundaries.
- **Wizards** read a model and setup and emit ordinary, editable operations. Today's knob
  pipeline becomes the "4th-axis knurl wizard".

Operation ids are strings unique within the project. The simulation's per-cell
"cut by operation" tags and the operation colours key off them, so they carry straight over.

**Phases.** Each phase merges to `main` only when `scripts/check.sh` passes and the Z1 knob
goldens are still byte-identical.

1. **Document model 0.2, with upgrade from 0.1. Done:**
   - `crates/fourcam-core/src/project.rs`;
   - `4cam upgrade`;
   - `run`, `simulate`, `toolpath` and the WASM API accept either version;
   - `examples/knob/*.project.json`.

   A setup is planned by lowering it to a 0.1 job, so the goldens stay identical.
2. **Operation registry. Done:** `crates/fourcam-core/src/ops.rs` describes each type (parameters with units, defaults and limits; tool shapes; geometry; machine needs; whether it shapes the part on purpose). `4cam ops list|show|check`, WASM `opTypes`/`checkProject`. A test keeps the specs in step with the engine.
3. **Setups. Done (rotary):** `4cam run` plans every setup (each with its own machine); a setup with `"stock": {"shape": "from_previous"}` starts from the previous setup's simulated stock (`project::simulate_project`); validation rejects a first setup that carries stock and duplicate output names. Example: `examples/knob/knob_two_setups.project.json`. Not yet: work-offset changes between setups (re-chucking at a different X or A), non-rotary holding.
4. **Geometry selection. Done (prototype):** `crates/fourcam-core/src/geometry.rs` splits a model into face regions at sharp edges (30°), with ids that survive reordering and STL round trips, and a shape guess (plane, cylinder, cone, freeform). An operation's `geometry` (faces, axial range, angular range) becomes a containment boundary in X and A; the drop-cutter still checks the whole model, so selections never gouge neighbouring faces. `4cam faces`, WASM `face_regions` (with per-triangle region ids for picking). Not yet: true surface-limited toolpaths, curves and 2D paths.
5. **Knurl wizard. Done:** `crates/fourcam-core/src/wizards.rs` reads the model's faces (the body cone, extended to the front face, and the front face's edge for the chamfer) and adds turning, a diamond knurl and a V-bit flank chamfer as ordinary operations and outputs. On the knob it rebuilds the program byte-for-byte. `4cam wizard knurl`, WASM `wizardKnurl`.
6. **Frontend setup/operation tree. Done:** the web app's Project panel (`web/tree.js`) shows setups, output files and operations; operations can be added from forms generated from the operation registry (validated by the engine before they join the project), edited, turned on/off, coloured, reordered and deleted; faces are picked by clicking the model; the knurl wizard runs from the tree; Generate and the machining simulation use the chosen setup and the operation colours. Originally planned as: type, tool, geometry chosen by clicking the model,
   reorder, enable, colour.
7. The first 3-axis operations: a toolpath IR with XYZ plus tool direction; facing, contour,
   pocket, drilling and 3D raster; Carvera vise setups.

## Next candidates

- 3-axis strategies (drop-cutter raster and waterline) on the same mesh and tool code.
- Indexed 3+1: 3-axis passes at fixed A angles.
- General moves and kinematics-aware posts. After that come simultaneous 4-axis wrapping and
  5-axis.
- Tool-holder collision checks against the stock model.
