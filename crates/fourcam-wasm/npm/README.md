# fourcam (npm)

[4cam](https://github.com/malvira/4cam), a 4th-axis rotary CAM engine, as WebAssembly with
a tiny ES-module wrapper: no dependencies, Node 18+ and browsers. JSON in, JSON out.

```js
import { init } from "fourcam";
const cam = await init();
const project = JSON.parse(await (await fetch("knob.project.json")).text());
const result = cam.planAndPost(project);          // machine: the project's, else the Makera Z1
for (const p of result.programs) console.log(p.name, p.lines, "lines,", p.issues.length, "issues");
```

`cam.simulate`, `cam.estimateGcode`, `cam.opTypes`, `cam.opDefaults` and more: see
`docs/api.md` and `docs/embedding.md` in the repository. Apache-2.0.
