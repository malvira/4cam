# Roadmap

marcam is meant to become a general-purpose CAM engine. For now it stays focused on 4th-axis
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
- **Tools.** Tool libraries don't depend on the machine type. Drills are stored but not used
  by the strategies yet.
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
   - `crates/marcam-core/src/project.rs`;
   - `marcam upgrade`;
   - `run`, `simulate`, `toolpath` and the WASM API accept either version;
   - `examples/knob/*.project.json`.

   A setup is planned by lowering it to a 0.1 job, so the goldens stay identical.
2. **Operation registry. Done:** `crates/marcam-core/src/ops.rs` describes each type (parameters with units, defaults and limits; tool shapes; geometry; machine needs; whether it shapes the part on purpose). `marcam ops list|show|check`, WASM `opTypes`/`checkProject`. A test keeps the specs in step with the engine.
3. **Setups. Done (rotary):** `marcam run` plans every setup (each with its own machine); a setup with `"stock": {"shape": "from_previous"}` starts from the previous setup's simulated stock (`project::simulate_project`); validation rejects a first setup that carries stock and duplicate output names. Example: `examples/knob/knob_two_setups.marcam.toml`. **Carried stock (2026-11-20):** a vise setup can carry its block too, turned over (about X or Y) or end for end, moved by an offset, its part turned with it; the block keeps both its sides' cuts; 3D roughing and adaptive clearing work only where material is left, the simulation starts from it; bars carry turned too (re-chucked end for end). Rotary roughing of a carried bar cuts only along X where the bar still stands above each level. Work offsets (G54...), drilling canned cycles and per-setup G-code options are in (2026-11-20).
4. **Geometry selection. Done (prototype):** `crates/marcam-core/src/geometry.rs` splits a model into face regions at sharp edges (30°), with ids that survive reordering and STL round trips, and a shape guess (plane, cylinder, cone, freeform). An operation's `geometry` (faces, axial range, angular range) becomes a containment boundary in X and A; the drop-cutter still checks the whole model, so selections never gouge neighbouring faces. `marcam faces`, WASM `face_regions` (with per-triangle region ids for picking).

   **Face-bounded rasters. Done:** faces picked on a mesh model limit `raster3d` (vise,
   or indexed on a rotary setup) and `rotary_finish` (both patterns) to where the tool
   rests on them:
   - At plan time the engine rebuilds the regions on the part's mesh and drops the tool on
     the selected faces alone as well as on the whole part. A point counts as on the faces
     where the two heights agree, so the tool still follows (and never gouges) the whole
     surface.
   - Run edges are found by bisection to 1 µm. The tool lifts between runs, and a raster
     links rows at depth only when the link stays on the faces.
   - A tool resting on a face's rim is on that face, so its footprint can reach a tool
     diameter past the edge.
   - Profiles' faces are axial bands: there the X range bounds them as before.
   - Tested by simulation: the dome's cap on the plate, a cone of the knob along and
     around. Each selected face is finished, nothing more than a diameter away is touched,
     and nothing is gouged.

   Not yet: curves and 2D paths as selections.
5. **Knurl wizard. Done:** `crates/marcam-core/src/wizards.rs` reads the model's faces (the body cone, extended to the front face, and the front face's edge for the chamfer) and adds turning, a diamond knurl and a V-bit flank chamfer as ordinary operations and outputs. On the knob it rebuilds the program byte-for-byte. `marcam wizard knurl`, WASM `wizardKnurl`.
6. **Frontend setup/operation tree. Done:** the web app's Project panel (`web/tree.js`) shows setups, output files and operations; operations can be added from forms generated from the operation registry (validated by the engine before they join the project), edited, turned on/off, coloured, reordered and deleted; faces are picked by clicking the model; the knurl wizard runs from the tree; Generate and the machining simulation use the chosen setup and the operation colours. Originally planned as: type, tool, geometry chosen by clicking the model,
   reorder, enable, colour.
7. **First 3-axis operations. Done:**
   - the toolpath IR has `Rapid3`/`Line3` moves (work XYZ plus a tool direction, `[0, 0, 1]`
     for now), posted as G0/G1 X Y Z and checked by the verifier (Y included);
   - box stock in a vise (`"stock": {"shape": "box", ...}`), with the setup's own zero
     comment and a 3-axis machine-limit check (X/Y/Z spans, feed cap; no rotary checks);
   - `face`, `pocket` (rectangle, or any closed outline with islands), `drill` (pecking), `contour` (outside/inside/on a closed
     path) and `raster3d` (drop-cutter zigzag over the mesh in step-down levels, rows linked at
     depth, descents steeper than the ramp angle slowed to the plunge feed) in `strategy/three_axis.rs`, with registry
     entries and engine defaults from the block;
   - `sim3::Heightmap3` simulates them. A test plans `examples/plate/plate.marcam.toml`
     (Carvera, 60x40x10 block, dome fixture) and checks each operation's result in the cut
     block: faced depth, pocket floor and walls, hole depth and width, outline depth outside
     and none inside, no gouge on the dome, no rapids through material, 0 verifier issues;
   - CLI previews (top + front view), web app: example, setup-filtered operation types, forms
     for rectangles and point lists, XYZ path and block in the 3D and side views.

   Web machining simulation for vise setups (added after the phase): `sim3::Playback3` and
   the WASM `simulate`/`simOpen`/`simSeek` handle box setups; the web app shows the block as a
   closed solid, coloured by the operation that cut each cell, with the same playback
   controls, legend and Simulate view as rotary setups.

   Pockets of any shape: `paths` (closed outlines, even-odd: an outline and its islands,
   e.g. from an SVG) instead of `rect`. The tool centre follows offsets of the outline,
   which are iso-lines of the signed distance on a 0.05 mm grid, traced with marching
   squares and simplified to 2 µm:
   - the first loop is at the tool radius, then every stepover inwards, capped at 0.7
     diameters so that sharp offset corners leave nothing;
   - a coverage check adds half-step loops wherever a ridge between walls would be left;
   - loops are cut from the middle out, climb milling, with the last laps along the walls.

   Each loop is entered from above at the plunge feed. Tests simulate an L-shaped pocket
   with a round island at stepovers 0.4 and 0.9: the floor is cut everywhere the tool can
   reach, and the island and everything outside are untouched.

   Not yet: waterline; linking loops without retracting. (Tool-holder checks: done, see
   below.)

## Next candidates

- 3-axis strategies (drop-cutter raster and waterline) on the same mesh and tool code.
- **Relief milling (part 1). Done:**
  - `raster3d` is now "3D parallel finishing": passes at any `angle`, `zigzag` or `one_way`
    (`climb`/`conventional` for a clockwise spindle), with the stepover set from a `scallop`
    target for ball and V-bit tools;
  - `rough3d`: the same raster with a flat tool in step-down levels, leaving stock;
  - grayscale images (PNG/PGM) as relief parts, flat on a vise or wrapped on the bar (then
    machined with `rotary_rough`/`rotary_finish`);
  - the post slows any short move whose rounded coordinates would descend faster than the
    plunge cap (lines the verifier would flag; the goldens have none, so they are unchanged);
  - example `examples/plaque` (Carvera vise, relief from `plaque.png`). Tests: scallop on a
    plane within the 0.01 mm target, one-way climb pass direction, the plaque rough (more than
    0.2 mm left, no gouge) and finish (no gouge beyond 0.003 mm, mean 0.012 mm left), and a
    wrapped relief roughed and finished on the rotary with no gouge.

  Tapered ball tools: done (rotary rough/finish, raster3d, rough3d, engrave; drop-cutter
  checked against brute force, scallop stepover checked by a simulated cut).
  Face selections bound 3-axis rasters too (see "Face-bounded rasters" above).
- **Engraving (part 2). Done:**
  - `engrave` follows polylines and text in a built-in single-stroke font (A-Z, 0-9, some
    punctuation; an original design) at a depth in `step_down` passes, with a V-bit, ball or
    flat tool;
  - `vcarve` carves closed outlines (even-odd) with a V-bit, the depth from the distance to
    the outline / tan(half angle), up to `max_depth`. This is exact for a V-bit, because
    distance is 1-Lipschitz;
  - both work flat on a vise or wrapped round the bar on a rotary setup (path Y = arc length
    at radius `top`, centred on the top at A0); text can be turned (`text_rotate: 90` runs it
    round the bar);
  - SVG import (`engrave::svg_paths`, WASM `svgPaths`, a "Load SVG" button on the form):
    path (lines, curves, arcs), polyline, polygon, line, rect, circle, ellipse, with
    transforms and mm/cm/in sizes;
  - examples: the plaque's engraved title and V-carved star (Carvera), and
    `examples/ring` (Z1): grooves, text round the bar and a V-carved diamond band, all
    wrapped. Tests check groove depth and wall slope, text placement, V-carve depth against
    distance (flat floors, nothing past the outline), and the same wrapped on the rotary.

  - **Outline letters for `vcarve`** (`text`, `text_height`, `text_at`, `text_rotate`,
    `text_weight`):
    - The font is marcam's own single-stroke font inked with a round pen: by default 0.12
      of the cap height wide. No third-party font is bundled. Like the rest of marcam it is
      original work under the repository's Apache-2.0 licence.
    - Each glyph's outline, and the outlines of its holes (O, A, B, 0 ...), are the zero
      loops of the distance field "half pen width minus distance to the strokes", traced
      with marching squares. They are carved like any closed paths, flat or wrapped.
    - Tests: outlines lie a half pen width from the strokes (with 1, 2 and 3 loops for I,
      O and 8), and a simulated V-carve of "4CAM" is at least as deep as the distance to
      the outline allows inside the letters, untouched outside. Text also wraps round the
      bar.

  Not yet: other typefaces (an outline font file reader); the part model does not include
  engraved or indexed features, so the simulation summary counts them as gouges.
- **Machining time estimates. Done:** `estimate.rs` reads G-code with the machine
  profile:
  - feeds as the controller reads them (degrees-as-length, linear-only, G93 inverse time),
    capped by each axis's top feed and the controller's cap;
  - rapids at each axis's rapid rate;
  - a look-ahead planner with junction deviation and per-axis acceleration (trapezoids);
  - dwells, tool changes and spindle starts.

  New optional profile fields: `accel` on axes, `max_rapid` on rotary axes, and `timing`
  (junction deviation, tool change and spindle start seconds), with stated defaults. Times
  are shown per operation, per file and in total: `marcam run`, `marcam estimate <file.nc>`,
  WASM `planAndPost`/`estimateGcode`, Python `estimate`, and the web app's summary and
  operation tree. Hand-computed tests cover straight feeds and rapids, acceleration-limited
  moves, corners and reversals, rotary moves in each feed mode, dwells, tool changes, inch
  units and arcs.
- **Simulation resolution. Done:**
  - presets: Coarse, Normal, Fine and Ultra, or custom cell sizes;
  - in the engine (`sim::Resolution`, `sim_grid`), CLI (`simulate --sim-res`), WASM
    (`res`, `simGrid`) and the web app: a control shows cells, memory and time against
    Normal, warns on very large grids and refuses those over 40 million cells; the setting
    is remembered;
  - playback snapshots are spaced to stay near 256 MB;
  - the 3D view shows at most 1M cells at rest and 250k while playing, then refines on
    pause.

  Tests: finer grids keep known features within half a cell, playback equals batch at every
  preset, and the web view stays watertight at Fine and Ultra.
- **Performance (see docs/gpu.md):** exact drop-cutter index changes (plate planning 21 s ->
  1.5 s, identical G-code).
- ~~Indexed 3+1: 3-axis passes at fixed A angles.~~ Done: on a rotary setup the five 3-axis
  operations take `index_a` (deg). The planner turns A there at the safe radius, runs the
  3-axis strategy around the bar (raster3d on the part turned by A), and returns to Y0. The
  rotary simulation cuts with the tool offset in Y (`sim::stamp_indexed`). Example
  `examples/knob/knob_indexed.marcam.toml`: a D-flat on the shaft at A0 and a cross hole at
  A90 after the knob's turning and knurl, whose G-code stays identical to the goldens. Tests
  check the flat's chord shape, the hole's depth and width, 0 rapids through stock, the
  verifier and the machine limits (Y travel included).
- **General moves and kinematics-aware posts. Done (4-axis):**
  - `Move::Pose` carries a tool tip and a tool direction in the part frame.
    `kinematics::solve_a` finds the A that turns the direction onto the spindle (the turn
    nearest the previous A); the post writes the tip in the machine frame, through the
    machine's axis map.
  - Feeds are the tip's speed along the part, written as each controller reads F.
  - A direction leaning along X needs a fifth axis: the post writes a comment and M0.
  - Indexed 3+1 operations now emit general moves. Their G-code is unchanged apart from
    redundant rapid words.
  - Axis words follow the machine's kinematics (`AxisMap`, see docs/profiles.md).

  Still to come: a second rotary axis (5-axis heads and tables), and simultaneous 4-axis
  wrapping.
- (older note) General moves and kinematics-aware posts. After that come simultaneous 4-axis wrapping and
  5-axis.
- ~~Tool-holder collision checks against the stock model.~~ Done: shank and holder
  checked per move in simulation, `run`, `planAndPost` and the SolidCraft plugin. Not yet:
  chuck, tailstock and vise jaws.
