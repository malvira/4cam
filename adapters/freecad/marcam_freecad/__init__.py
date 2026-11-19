"""marcam in FreeCAD: plan one marcam operation on a FreeCAD body and write its G-code.

Uses the `marcam` Python package (the wheel). In FreeCAD's Python console or a macro:

    import marcam_freecad as fc
    result = fc.plan(App.ActiveDocument.Knob, "rotary_finish", out_dir="C:/cam/out")
    print(fc.summary(result))

The body is tessellated (`Shape.tessellate`, FreeCAD's mm), and the engine builds the
project around it with every field defaulted: body on the rotary axis (`axis`, default X,
front at X0), bar stock, tool, machine and the operation's own fields. These are the same
rules as the SolidCraft plugin (`marcam.body_project`). Pass any of them as keywords:
`machine="makera-carvera-4axis"`, `tool_kind="ball"`, `stepover_mm=0.3`, ...

With `out_dir`, it writes `<name>.nc` per program, the project as `<name>.marcam.toml` (the
format marcam writes) and the body as `<name>.stl`; `marcam run <name>.marcam.toml` reproduces
the G-code exactly.
"""

import os
import struct

import marcam

__all__ = ["plan", "stl_bytes", "summary", "DEFAULT_TOLERANCE"]

#: Chordal tolerance of the tessellation (mm).
DEFAULT_TOLERANCE = 0.05


def _shape(obj):
    """A FreeCAD object's shape (or the shape itself)."""
    return getattr(obj, "Shape", obj)


def stl_bytes(shape, tolerance=DEFAULT_TOLERANCE):
    """Binary STL of a FreeCAD shape, from `Shape.tessellate(tolerance)` (global mm)."""
    points, facets = shape.tessellate(tolerance)
    if not facets:
        raise ValueError("the shape has no faces to machine (is it a solid?)")
    p = [(float(v.x), float(v.y), float(v.z)) for v in points]
    out = bytearray(b"marcam: FreeCAD body (Shape.tessellate, mm)".ljust(80, b"\0"))
    out += struct.pack("<I", len(facets))
    for i, j, k in facets:
        a, b, c = p[i], p[j], p[k]
        u = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
        w = (c[0] - a[0], c[1] - a[1], c[2] - a[2])
        n = (u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0])
        length = (n[0] ** 2 + n[1] ** 2 + n[2] ** 2) ** 0.5 or 1.0
        out += struct.pack("<12fH", *(x / length for x in n), *a, *b, *c, 0)
    return bytes(out)


def plan(obj, operation, out_dir=None, tolerance=DEFAULT_TOLERANCE, **inputs):
    """Plan `operation` (a marcam operation type, e.g. "rotary_finish") on a FreeCAD object.

    Returns {"name", "project", "programs", "files"}: the project marcam built, its programs
    (G-code, issues, gouge and holder checks, time), and the files written to `out_dir`.
    Raises ValueError with the engine's message on a bad input.
    """
    stl = stl_bytes(_shape(obj), tolerance)
    inputs.setdefault("body", getattr(obj, "Label", None) or getattr(obj, "Name", None) or "body")
    built = marcam.body_project(operation, stl, **inputs)
    project, name = built["project"], built["name"]
    result = marcam.plan_and_post(project, mesh=stl)
    files = []
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
        writes = [(f"{name}.stl", stl), (f"{name}.marcam.toml", marcam.to_toml(project).encode())]
        writes += [(f'{p["name"]}.nc', p["gcode"].encode()) for p in result["programs"]]
        for fname, data in writes:
            with open(os.path.join(out_dir, fname), "wb") as f:
                f.write(data)
            files.append(fname)
    return {"name": name, "project": project, "programs": result["programs"], "files": files}


def summary(result):
    """One line per program: lines, issues, time, gouge and holder checks."""
    out = []
    for p in result["programs"]:
        coll = (p.get("collisions") or {}).get("count", 0)
        mins = (p.get("time") or {}).get("total_s", 0.0) / 60.0
        out.append(f'{p["name"]}: {p["lines"]} lines, {len(p["issues"])} issue(s), '
                   f'{mins:.1f} min, {coll} holder/shank collision(s)')
    return "\n".join(out)
