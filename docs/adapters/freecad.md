# marcam in FreeCAD (Python adapter)

`adapters/freecad` plans a marcam operation on a FreeCAD body and writes its G-code. It is
plain Python on top of the `marcam` wheel (see `docs/packaging.md`): no FreeCAD workbench
to install, and no change to FreeCAD.

- `marcam_freecad.plan(obj, operation, out_dir=None, tolerance=0.05, **inputs)`:
  1. tessellates the object's `Shape` (FreeCAD's mm);
  2. has the engine build the project round it (`marcam.body_project`: the body on the
     rotary axis, bar stock, tool, machine and the operation's fields, all defaulted);
  3. plans and posts it.

  It returns the programs (G-code, issues, gouge and holder/shank checks, time). With
  `out_dir` it also writes `<name>.nc`, `<name>.marcam.toml` and the body as `<name>.stl`,
  and `marcam run <name>.marcam.toml` reproduces the G-code exactly.
- `marcam_plan.FCMacro`: a FreeCAD macro that plans the selected body. It asks for the
  operation and the output folder, and prints the summary in the report view.
- The inputs and defaults are the engine's (`marcam::body`), so they are the same as the
  SolidCraft plugin's:

  | Input | Default |
  | --- | --- |
  | `axis` | `X`: the body's axis through its origin, front moved to X0 |
  | `machine` | `makera-z1-4axis` |
  | `tool_kind` | the operation's first tool kind; `tapered_ball` takes `tip_diameter` and `taper_angle` |
  | `tool_diameter` | 3.175 mm |
  | `vbit_angle` | 90° |
  | `stock_radius` | the largest radius + 0.5 mm, rounded up to 0.5 mm |
  | `stock_length` | the length + 15 mm |
  | `spindle_rpm` | 12000 |
  | `name` | `<label>_<operation>` |
  | the operation's fields | marcam's defaults |

  `rotary_turning` is not offered: it needs a revolved profile, and the adapter gets a
  mesh.

```python
import marcam_freecad as fc
result = fc.plan(App.ActiveDocument.Knob, "rotary_finish", out_dir="C:/cam/out", stepover_mm=0.3)
print(fc.summary(result))
```

## Install

1. Put the `marcam` wheel where FreeCAD's Python finds it, for example
   `FreeCAD/bin/python -m pip install marcam-<ver>-cp39-abi3-<platform>.whl`.
2. Put `adapters/freecad` (the `marcam_freecad` folder and the macro) in FreeCAD's macro
   folder.

## Tests and their limit

FreeCAD is not installed on the machine this was built on, so the tests run against a
**mocked FreeCAD API** (`sh scripts/test-freecad.sh`, part of `check.sh`):
- a fake shape whose `tessellate()` serves the knob mesh;
- fake `FreeCAD` and `FreeCADGui` modules for the macro (selection, report view).

They check that:
- four operations (rotary finish, rough, knurl, and a 3D raster with a tapered ball) give
  G-code byte-identical to `marcam run` on the written project;
- a body modelled along Z gives the same stock and G-code as along X;
- bad inputs name the field;
- the macro plans the selection and refuses an empty one.

Not yet tested inside a real FreeCAD (`FreeCADCmd`): that needs FreeCAD installed. The only
FreeCAD calls the adapter makes are `Shape.tessellate`, `Gui.Selection.getSelection`,
`App.Console.PrintMessage`, `App.getUserMacroDir` and the PySide dialogs.
