"""Plan a 4cam project from Python and print its programs.

    python plan.py project.json [mesh.stl]
"""
import json
import sys

import fourcam

project = json.load(open(sys.argv[1], encoding="utf-8"))
mesh = sys.argv[2] if len(sys.argv) > 2 else None  # a path or bytes
result = fourcam.plan_and_post(project, mesh=mesh)  # raises on an invalid project
print(fourcam.version())
for p in result["programs"]:
    print(f'{p["name"]}: {p["lines"]} lines, {len(p["issues"])} issue(s)')
