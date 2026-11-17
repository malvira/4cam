"""Tests for the installed `marcam` wheel (plain asserts; run with python)."""
from pathlib import Path

import marcam

ROOT = Path(__file__).resolve().parents[3]


def test_example_matches_golden():
    job = marcam.example_job()
    res = marcam.plan_and_post(job, max_points=1000)
    lines = [p["lines"] for p in res["programs"]]
    assert lines == [21787, 15146], lines
    for p in res["programs"]:
        assert p["issues"] == [], p["issues"]
        golden = (ROOT / "tests" / "golden" / f"{p['name']}.nc").read_text().replace("\r\n", "\n")
        assert p["gcode"] == golden, p["name"]


def test_linuxcnc_and_errors():
    res = marcam.plan_and_post(marcam.example_job(), machine="linuxcnc")
    assert "G93" in res["programs"][0]["gcode"]
    assert marcam.machine("linuxcnc")["feed_mode"] == "inverse_time"
    for bad in ("{", {"schema_version": "0.1"}):
        try:
            marcam.plan_and_post(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("expected ValueError")


def test_mesh_job_and_simulation():
    job_path = ROOT / "examples" / "knob" / "knob_finish.job.json"
    stl = ROOT / "examples" / "knob" / "knob.stl"
    job = job_path.read_text()
    res = marcam.plan_and_post(job, mesh=str(stl), max_points=1000)
    assert [p["name"] for p in res["programs"]] == [
        "knob_3_mesh_rough",
        "knob_4_finish_along",
        "knob_5_finish_around",
    ]
    for p in res["programs"]:
        assert p["issues"] == []
        assert p["gouge"]["max_depth"] < 0.01
    # bytes work too
    res2 = marcam.plan_and_post(job, mesh=stl.read_bytes(), max_points=1000)
    assert res2["programs"][0]["gcode"] == res["programs"][0]["gcode"]
    sim = marcam.simulate(marcam.example_job(), dx=0.2, na=90)
    assert sim["stocks"][0]["rapid_hits"] == 0
    assert sim["stocks"][0]["gouge_cells"] == 0



def test_time_estimates():
    res = marcam.plan_and_post(marcam.example_job(), max_points=1000)
    total = sum(p["time"]["total_s"] for p in res["programs"])
    assert abs(res["time_total_s"] - total) < 1e-6 and total > 60
    for p in res["programs"]:
        ops = sum(o["seconds"] for o in p["time"]["ops"])
        assert ops <= p["time"]["total_s"] + 1e-6
    # any G-code: 100 mm at 600 mm/min on the Z1 is a little over 10 s with acceleration
    e = marcam.estimate("G0 X0 Y0 Z5\nG1 X100 F600\n")
    assert e["machine"] == "makera-z1-4axis" and 10.0 < e["feed_s"] < 10.1, e


if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_"):
            fn()
            print("ok", name)
    print("python OK", marcam.version())
