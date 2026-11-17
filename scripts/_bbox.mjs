import { readFileSync } from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2];
const j = JSON.parse(readFileSync(file, 'utf8'));
const bytes = conv.convertBytes(JSON.stringify(j), {});
const m = parseModel(bytes);
console.log('size', j.w, j.h, 'frames', m.frameCount);
for (const mesh of m.meshes) {
  const p = mesh.positions;
  let minx = 1e9, miny = 1e9, maxx = -1e9, maxy = -1e9;
  for (let i = 0; i < p.length; i += 3) {
    minx = Math.min(minx, p[i]); maxx = Math.max(maxx, p[i]);
    miny = Math.min(miny, p[i + 1]); maxy = Math.max(maxy, p[i + 1]);
  }
  console.log(`${mesh.name.padEnd(26)} vert=${mesh.vertexCount} bbox x[${minx.toFixed(1)},${maxx.toFixed(1)}] y[${miny.toFixed(1)},${maxy.toFixed(1)}]`);
}
