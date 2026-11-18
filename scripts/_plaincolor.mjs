import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_05.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
let n=0;
for (const mesh of m.meshes) {
  if (mesh.gradient) continue;
  const c=mesh.color;
  console.log(`kind=${mesh.kind} rgba=(${c[0].toFixed(2)},${c[1].toFixed(2)},${c[2].toFixed(2)},${c[3].toFixed(2)}) vert=${mesh.vertexCount} ${mesh.name}`);
  if(++n>=40) break;
}
