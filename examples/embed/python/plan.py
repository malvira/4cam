"""Plan a marcam project from Python and print its programs.

    python plan.py project.marcam.toml [mesh.stl]
"""
import sys

import marcam

# the project file's text, TOML or JSON (a dict works too)
project = open(sys.argv[1], encoding="utf-8").read()
mesh = sys.argv[2] if len(sys.argv) > 2 else None  # a path or bytes
result = marcam.plan_and_post(project, mesh=mesh)  # raises on an invalid project
print(marcam.version())
for p in result["programs"]:
    print(f'{p["name"]}: {p["lines"]} lines, {len(p["issues"])} issue(s)')
