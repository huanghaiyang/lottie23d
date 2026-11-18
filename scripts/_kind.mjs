import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_07.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const byKind = {};
const strokes = [];
for (const mesh of m.meshes) {
  byKind[mesh.kind] = (byKind[mesh.kind] ?? 0) + 1;
  if (mesh.kind === 1) strokes.push(mesh);
}
console.log(file, 'kinds=', JSON.stringify(byKind), 'total=', m.meshes.length);
// 描边网格样例：名称、顶点、颜色、是否动画
for (const s of strokes.slice(0, 15)) {
  console.log(`  stroke ${s.name} vtx=${s.vertexCount} anim=${s.animated} color=${s.color.map(x=>+x.toFixed(2)).join(',')}`);
}
