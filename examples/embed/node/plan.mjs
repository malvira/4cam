// Plan a marcam project from Node and print its programs.
//   node plan.mjs project.marcam.toml [mesh.stl]
import { readFileSync } from "node:fs";
import { init } from "marcam";

const cam = await init();
// the project file's text, TOML or JSON (an object works too)
const project = readFileSync(process.argv[2], "utf8");
const mesh = process.argv[3] ? readFileSync(process.argv[3]) : undefined;
const result = cam.planAndPost(project, "", mesh); // "": the project's machine
if (result.error) throw new Error(result.error);
console.log(cam.version());
for (const p of result.programs) console.log(`${p.name}: ${p.lines} lines, ${p.issues.length} issue(s)`);
