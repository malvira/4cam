// Plan a marcam project from Node and print its programs.
//   node plan.mjs project.json [mesh.stl]
import { readFileSync } from "node:fs";
import { init } from "marcam";

const cam = await init();
const project = JSON.parse(readFileSync(process.argv[2], "utf8"));
const mesh = process.argv[3] ? readFileSync(process.argv[3]) : undefined;
const result = cam.planAndPost(project, "", mesh); // "": the project's machine
if (result.error) throw new Error(result.error);
console.log(cam.version());
for (const p of result.programs) console.log(`${p.name}: ${p.lines} lines, ${p.issues.length} issue(s)`);
