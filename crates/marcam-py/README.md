# marcam (Python)

Python bindings for [marcam](https://github.com/malvira/marcam), a 4th-axis rotary CAM engine.
JSON in, JSON out: the same documents as the C, WebAssembly and CLI interfaces.

```python
import marcam, json
project = json.load(open("knob.project.json"))
result = marcam.plan_and_post(project)            # machine: the project's, else the Makera Z1
for p in result["programs"]:
    print(p["name"], p["lines"], "lines,", len(p["issues"]), "issues")
open(result["programs"][0]["name"] + ".nc", "w").write(result["programs"][0]["gcode"])
```

Also `simulate`, `estimate`, `example_job`, `machine` and `version`. See the repository's
`docs/embedding.md`. Apache-2.0.
