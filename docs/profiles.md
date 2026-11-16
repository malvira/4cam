# Machine profiles and tool libraries

4cam keeps what it knows about machines and cutters in two versioned JSON formats. Built-in
ones live in `profiles/`. Your own go in the user profile folder (`4cam profiles dir`) or, in
the web app, in browser storage.

## Machine profile (`"format": "4cam-machine"`, `"version": 1`)

```json
{
  "format": "4cam-machine", "version": 1,
  "id": "makera-z1-4axis", "name": "Makera Z1 with 4th axis",
  "vendor": "Makera", "model": "Z1",
  "sources": [{ "url": "...", "note": "what came from where" }],
  "kinematics": {
    "linear": [
      { "letter": "X", "direction": [1, 0, 0], "min": -10, "max": 200, "max_feed": 4000, "max_rapid": 4000 }
    ],
    "rotary": [
      { "letter": "A", "axis": [1, 0, 0], "carries": "workpiece", "handedness": "right",
        "continuous": true, "max_feed": 21600 }
    ],
    "spindle_axis": [0, 0, 1]
  },
  "rotary_envelope": { "max_diameter": 80, "max_length": 150 },
  "spindle": { "min_rpm": 0, "max_rpm": 13000, "power_w": 150 },
  "max_feed": 4000,
  "tool_changer": { "kind": "manual", "max_tool_diameter": 8 },
  "notes": ["anything uncertain"],
  "post": { "...": "G-code templates, see below" }
}
```

**Kinematics.** Kinematics is a general description. It has any number of linear axes and
any number of rotary axes. Each rotary axis has its axis of rotation, what it carries
(`workpiece` for a table or rotary chuck, `tool` for a head), its handedness, and either
`continuous` or `min`/`max` in degrees.

The same format describes:

- a 3-axis mill (no rotary axes);
- a 3+1 machine like the Z1;
- later, 5-axis trunnions or head-head machines.

Today's rotary strategies need a rotary axis about X (any letter). The limit check says so
if a machine has none.

**Axis directions and posting.**
- **Linear `direction`.** This is the tool's motion relative to the work when the axis
  moves positive. Fusion 360 machine files give the carriage's own direction instead, so the
  importer reverses axes that move the table rather than the head. A Makera table's Y
  `[0, -1, 0]` becomes `[0, 1, 0]`.
- **The axis map.** 4cam's toolpaths use one convention: X, Y, Z are the tool's motion
  relative to the work, and A turns the work right-handed about +X. When a profile loads,
  `axis_map` works out each word for this machine:
  - each linear word comes from the axis that moves the tool along that direction, with its
    sign;
  - the rotary word comes from the axis about X, with its letter;
  - the rotary sign flips for a left-handed axis, for an axis along -X, and for a head that
    turns the tool instead of the work.
- **The post writes through that map.** Every built-in profile maps one to one. A machine
  whose A turns the other way gets every A word negated, and is still correct.
- **Unsupported.** Linear axes that are not along X, Y or Z are refused.

**Units.** Linear values are in mm and mm/min; rotary feeds are in deg/min.

**Optional fields.** Anything a source doesn't give is left out, and the reason goes in
`notes`.

**Post templates (`post`).** These fields control the G-code 4cam writes:

| Field | Meaning |
| --- | --- |
| `feed_mode` | How the controller reads F on moves that include A: `degrees_as_length` (GRBL/Smoothieware/Makera, F along the X-Z-A vector with degrees counted as mm), `linear_only`, or `inverse_time` (G93). |
| `decimals` | Number of decimals in axis words. |
| `origin_comment`, `preamble`, `reset_rotary`, `tool_change`, `spindle_on`, `dwell`, `postamble` | Lines of G-code. They may contain `{tool}`, `{rpm}` and `{safe}`. |

**Validation.** A profile is checked when it loads. The error names the field, for example
`axis X: min -10 must be below max -50` or `axis letters repeat`.

**Limit check.** After planning, `check_limits` reports problems with the job and its
toolpaths:

- *Errors*, which the machine cannot do:
  - spindle rpm outside the range;
  - stock larger than the rotary envelope;
  - a tool wider than the spindle takes;
  - a tool number outside the ATC slots;
  - X or Z travel exceeded;
  - A outside its limits.
- *Warnings*, which run but not as planned: feeds or A rates above the controller's caps. The
  controller clamps them, so the job runs slower.

`4cam run` prints both kinds and fails on errors. The web app shows them per program.

## Tool library (`"format": "4cam-tools"`, `"version": 1`)

```json
{
  "format": "4cam-tools", "version": 1, "id": "makera-starter", "name": "Makera starter tools",
  "tools": [
    {
      "id": "spiral-o-metal-3.175x12", "number": 1,
      "name": "Spiral O Metal 3.175 x 12 mm (single flute)", "vendor": "Makera",
      "shape": { "kind": "flat", "diameter": 3.175 },
      "flute_length": 12, "overall_length": 38, "shank_diameter": 3.175, "stickout": 20,
      "flutes": 1,
      "presets": [{ "material": "aluminium", "rpm": 12000, "feed": 500, "plunge": 200 }]
    }
  ]
}
```

These are the shapes a tool can have, and how the strategies use each:

| Shape | Fields | Used by the strategies as |
| --- | --- | --- |
| `flat` | `diameter` | a flat end mill |
| `ball` | `diameter` | a ball end mill |
| `bull` | `diameter`, `corner_radius` | a flat end mill (conservative: a flat cutter never goes lower than the bull nose could) |
| `vbit` | `angle` (included), `diameter`, optional `tip_diameter` | a sharp cone (the tip flat is ignored) |
| `tapered_ball` | `tip_diameter`, `taper_angle` (each side from the axis, deg), `diameter` | a ball of the tip's size blending (tangent) into a cone up to the diameter; drop-cutter, scallop stepover and simulation use the true shape (a taper of 0 is a plain ball). The GPU simulation has no such shape, so these programs run on the CPU |
| `drill` | `diameter`, `point_angle` | stored only; not usable yet |

### What is above the flutes: shank, stickout, holder

A tool's body above the cutting edge is checked against the stock (see `4cam run`'s
"holder/shank check"). Library and inline tools take these fields. Every one has a default:

| Field | Meaning | Default |
| --- | --- | --- |
| `flute_length` | cutting length from the tip | 3 cutting diameters |
| `shank_diameter` | shank above the flutes | the cutting diameter |
| `stickout` | tip to holder | flute length + 5 mm (never less than the flute length) |
| `holder_diameter` | holder / collet nut | 20 mm (inline tools only for now) |
| `holder_length` | holder length above the stickout | 30 mm (inline tools only for now) |

The check runs the material-removal simulation and, before each move cuts, tests every tool
position along it against the stock as the earlier moves left it:

- rotary stock: the whole bar, where the bar beyond the simulated range is taken as uncut;
- 3-axis stock: the block heightmap.

It reports each move where the shank or holder reaches into material: the program, the
G-code line, the tool, which part touched, how deep, and where. The check covers rotary,
indexed 3+1 and vise programs, rapids included.

The check does not model chucks, tailstocks or vises.

The rotary stock map stores only each ray's outer radius. A hole the tool cuts along its own
axis therefore still reads as solid inside. To avoid false alarms there, on rotary stock the
shank is checked only outside the cutting radius, where the flutes have not already cut.
The cost: a shank no wider than the cut is not flagged for a slot deeper than the flutes on
rotary stock. Planning's flute-length rule covers that for rotary operations. The 3-axis
heightmap is exact and checks the whole shank.

For rotary operations, planning already requires the flute length to be at least the radial
depth plus 1 mm. That keeps the shank and holder above the bar there, so in practice the
check matters for 3-axis and indexed work, where walls rise beside the tool.

The library does not depend on the machine type. A job uses a library tool either inline, or
by reference:

```json
"tools": [{ "number": 1, "library": "makera-starter", "tool": "spiral-o-metal-3.175x12" }]
```

`number` sets the T number. An optional `description` replaces the library name in the G-code
header.

## Importing

`4cam profiles import <file-or-url>`, or **Import** in the web app's tool library panel,
reads any of these formats:

- **4cam JSON:** either of the formats above.
- **Fusion 360 tool library:** `.tools` (a zip containing `tools.json`) or the JSON itself.
  The tool types map as follows:
  - flat, slot and face mills → `flat`;
  - ball and lollipop mills → `ball`;
  - bull nose → `bull`;
  - chamfer, engrave and spot tools → `vbit` (Fusion `TA` is the half angle, so `angle = 2*TA`;
    `tip-diameter` is kept);
  - tapered mills → `tapered_ball`;
  - drills → `drill`.

  Presets keep the material name, `n` (rpm), `v_f` (feed), `v_f_plunge`, stepdown and
  stepover. Holders, thread mills and other tools are skipped, with a note.
- **Fusion 360 machine definition:** `.mch` JSON, or a `.f3d` design that embeds one, as in
  Makera's downloads. The import reads:
  - the kinematic tree: linear axes with travels and per-axis feeds, and rotary axes with
    their axis of rotation and limits;
  - the head's spindle range, power and largest tool;
  - the tool changer;
  - the multi-axis feed method: `FPM`/`combination` becomes `degrees_as_length`, and inverse
    time becomes `inverse_time`.

  Makera machines get Makera post templates. Missing data is reported in `notes`.
- **Older 4cam machine file:** post templates only.

## Makera sources (checked 2026-11-15)

Makera's software page (https://www.makera.com/pages/software) links its CAM profiles to
https://github.com/MakeraInc/CarveraProfiles. The Fusion 360 download is
`CAM_Post_Processors/Fusion360-profiles/Fusion-Profiles.zip` (commit 2e69312, 2026-08-05).
It contains:

- machine designs for the Z1, Carvera and Carvera Air, in 3-axis and 4-axis versions
  (`.f3d`, with an embedded `simulation.mch`);
- the post `Carvera.cps` (Makera Carvera Community Post v1.4.6);
- tool libraries (`.tools`, `.csv`, `.hsmlib`).

**License.** The repository has no license file, and the post is
"Copyright (C) 2012-2022 by Autodesk, Inc.". So none of these files are copied into this
repository:

- 4cam's built-in profiles are written from the facts in them, with sources cited in each
  profile.
- The importers accept the original files at runtime.
- The tests use synthetic files in the same format (`tests/fixtures/`).
- `crates/fourcam-core/tests/makera_downloads.rs` runs against the real files when they are
  cached in `profiles/_downloads/` (ignored by git).

**What the Makera files say:**

- **Z1 4-axis:**
  - X -10..200, Y -200..10, Z 0..100;
  - A about +X, right-handed, no limits;
  - 4000 mm/min;
  - spindle 0–13000 rpm, 150 W, 8 mm largest tool.

  The file declares a 99-tool changer, but the Z1 changes tools by hand, so our profile says
  `manual`.
- **Carvera and Carvera Air 4-axis:** A about +X, 4000 mm/min, 6-tool changer, no travel or
  spindle limits. Ours come from published specs and are marked as such in `notes`.
- **Feed mode:** the post uses `FEED_DPM` with `DPM_COMBINATION`, which is what 4cam calls
  `degrees_as_length`. It matches the Z1 profile that produced the cut knob G-code.
- **Where `Carvera.cps` differs from the 4cam Z1 templates** (none of these change feeds or
  motion):
  - the tool change is `T1M6`, where ours is `M6 T1`;
  - the program starts `G90 G94 G17` + `G21`, where ours is `G21 G90 G94` + `G17`;
  - at the end it turns A back to 0 and retracts with `G28` before `M30`, where ours does
    `G0 Z<safe>`.

**Uncertain specs:**

- The Carvera's 15000 rpm comes from a review, not a Makera spec sheet.
- For the Carvera Air spindle, Makera says 0–13000 rpm and a community guide says
  3000–13000.
- The Carvera's 4th-axis envelope is unknown.
- MakeraCAM's own tool library format is not published, so it isn't supported.
