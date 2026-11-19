"""The FreeCAD adapter against a mocked FreeCAD API (FreeCAD itself is not installed here):
a fake shape whose tessellate() serves the knob mesh, fake FreeCAD/FreeCADGui modules for
the macro. Each plan's G-code must be what `marcam run` makes from the project it wrote.

    python adapters/freecad/tests/test_freecad.py <path to the marcam CLI>
"""

import importlib.machinery
import os
import struct
import subprocess
import sys
import tempfile
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
sys.path.insert(0, os.path.dirname(HERE))
import marcam  # noqa: E402
import marcam_freecad as fc  # noqa: E402

CLI = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else os.path.join(ROOT, "target", "release", "marcam")


class Vector:
    def __init__(self, x, y, z):
        self.x, self.y, self.z = x, y, z


def read_stl(path):
    """Points and facets of a binary STL (shared vertices merged), FreeCAD-style."""
    data = open(path, "rb").read()
    (n,) = struct.unpack_from("<I", data, 80)
    index, points, facets = {}, [], []
    for t in range(n):
        vals = struct.unpack_from("<12f", data, 84 + 50 * t)
        tri = []
        for k in range(3):
            p = vals[3 + 3 * k: 6 + 3 * k]
            if p not in index:
                index[p] = len(points)
                points.append(p)
            tri.append(index[p])
        facets.append(tuple(tri))
    return points, facets


class FakeShape:
    """What the adapter uses of Part.Shape: tessellate(tolerance) -> (points, facets)."""

    def __init__(self, points, facets, perm=(0, 1, 2)):
        self.points, self.facets, self.perm = points, facets, perm
        self.asked = None

    def tessellate(self, tolerance):
        self.asked = tolerance
        return [Vector(*(p[i] for i in self.perm)) for p in self.points], list(self.facets)


class FakeObject:
    def __init__(self, label, shape):
        self.Label, self.Name, self.Shape = label, label, shape


def knob(perm=(0, 1, 2)):
    pts, facets = read_stl(os.path.join(ROOT, "examples", "knob", "knob.stl"))
    return FakeObject("Knob", FakeShape(pts, facets, perm))


def cli_gcode(project_path, program, out):
    subprocess.run([CLI, "run", project_path, "--out", out], check=True, capture_output=True)
    return open(os.path.join(out, program + ".nc"), encoding="utf-8").read()


def test_plans_match_the_cli():
    with tempfile.TemporaryDirectory() as tmp:
        for op, inputs in [
            ("rotary_finish", {}),
            ("rotary_rough", {"step_down": 0.5}),
            ("helical_knurl", {"count": 20}),
            ("raster3d", {"tool_kind": "tapered_ball", "tip_diameter": 1.0, "taper_angle": 4.0, "scallop": 0.05}),
        ]:
            obj = knob()
            out = os.path.join(tmp, op)
            res = fc.plan(obj, op, out_dir=out, **inputs)
            assert obj.Shape.asked == fc.DEFAULT_TOLERANCE
            assert res["name"] == f"knob_{op}", res["name"]
            assert sorted(res["files"]) == sorted([f"knob_{op}.stl", f"knob_{op}.marcam.toml", f"knob_{op}.nc"])
            prog = res["programs"][0]
            assert not prog["issues"], prog["issues"][:3]
            assert prog["collisions"]["count"] == 0
            got = open(os.path.join(out, f"knob_{op}.nc"), encoding="utf-8").read()
            assert got == prog["gcode"]
            want = cli_gcode(os.path.join(out, f"knob_{op}.marcam.toml"), f"knob_{op}", os.path.join(out, "cli"))
            assert got == want, f"{op}: the plugin's G-code differs from marcam run"
            for k, v in inputs.items():
                if k not in ("tool_kind", "tip_diameter", "taper_angle"):
                    assert res["project"]["setups"][0]["operations"][0][k] == v
            print(f"ok {op}: {prog['lines']} lines, identical to marcam run")


def test_a_body_along_z_is_turned_onto_x():
    on_x = fc.plan(knob(), "rotary_finish")
    on_z = fc.plan(knob(perm=(1, 2, 0)), "rotary_finish", axis="Z")
    stock = lambda r: r["project"]["setups"][0]["stock"]  # noqa: E731
    assert stock(on_x) == stock(on_z) == {"shape": "cylinder", "radius": 10.5, "length": 37.0}
    assert on_x["programs"][0]["gcode"] == on_z["programs"][0]["gcode"]
    print("ok axis Z: same stock and G-code as along X")


def test_bad_inputs_name_the_field():
    for op, inputs, field in [
        ("rotary_finish", {"machine": "nope"}, "machine"),
        ("rotary_finish", {"axis": "W"}, "axis"),
        ("rotary_finish", {"count": 3}, "count"),
        ("rotary_turning", {}, "operation"),
        ("no_such_op", {}, "operation"),
    ]:
        try:
            fc.plan(knob(), op, **inputs)
        except ValueError as e:
            assert str(e).startswith(field + ":"), f"{op} {inputs}: {e}"
        else:
            raise AssertionError(f"{op} {inputs}: accepted")
    try:
        fc.plan(FakeObject("Empty", FakeShape([], [])), "rotary_finish")
    except ValueError as e:
        assert "no faces" in str(e)
    else:
        raise AssertionError("an empty shape was accepted")
    print("ok bad inputs")


def test_the_macro():
    messages = []
    app = types.ModuleType("FreeCAD")
    app.Console = types.SimpleNamespace(PrintMessage=messages.append)
    gui = types.ModuleType("FreeCADGui")
    obj = knob()
    gui.Selection = types.SimpleNamespace(getSelection=lambda: [obj])
    sys.modules["FreeCAD"], sys.modules["FreeCADGui"] = app, gui
    loader = importlib.machinery.SourceFileLoader("macro", os.path.join(HERE, "..", "marcam_plan.FCMacro"))
    macro = types.ModuleType(loader.name)
    macro.__file__ = loader.path
    loader.exec_module(macro)
    assert "rotary_finish" in macro.OPERATIONS and "rotary_turning" not in macro.OPERATIONS
    with tempfile.TemporaryDirectory() as tmp:
        res = macro.run("helical_knurl", tmp)
        assert os.path.exists(os.path.join(tmp, "knob_helical_knurl.nc"))
    assert "marcam helical_knurl on Knob" in messages[0] and "0 issue(s)" in messages[0], messages
    gui.Selection = types.SimpleNamespace(getSelection=lambda: [])
    try:
        macro.run("helical_knurl", "x")
    except ValueError as e:
        assert "select one body" in str(e)
    else:
        raise AssertionError("ran without a selection")
    print("ok macro")


if __name__ == "__main__":
    print(marcam.version())
    test_plans_match_the_cli()
    test_a_body_along_z_is_turned_onto_x()
    test_bad_inputs_name_the_field()
    test_the_macro()
    print("FREECAD ADAPTER OK (mocked FreeCAD API)")
