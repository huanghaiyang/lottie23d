import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_07.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const re = /¥|Qty|ENERGY|KOMBUCHA|MATCHA|COFFEE|drink|Outlines 2|Layer 1 Outlines/i;
let n=0;
for (const mesh of m.meshes) {
  if (!re.test(mesh.name||'')) continue;
  const c = mesh.color;
  const col = c?`rgba(${c[0].toFixed(2)},${c[1].toFixed(2)},${c[2].toFixed(2)},${c[3].toFixed(2)})`:'-';
  console.log(`kind=${mesh.kind} vert=${mesh.vertexCount} idx=${mesh.indexCount} anim=${mesh.animated} ${col}  ${mesh.name}`);
  if (++n>=50) break;
}
console.log('total meshes', m.meshes.length);
