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

## Next candidates

- 3-axis strategies (drop-cutter raster and waterline) on the same mesh and tool code.
- Indexed 3+1: 3-axis passes at fixed A angles.
- General moves and kinematics-aware posts. After that come simultaneous 4-axis wrapping and
  5-axis.
- Tool-holder collision checks against the stock model.
