import { readFileSync } from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';

const conv = await loadConverter();
const file = process.argv[2];
const j = JSON.parse(readFileSync(file, 'utf8'));
const bytes = conv.convertBytes(JSON.stringify(j), {});
const m = parseModel(bytes);
let n = 0;
for (const mesh of m.meshes) {
  if (!mesh.gradient) continue;
  const g = mesh.gradient;
  n += 1;
  if (n > 12) break;
  console.log(
    `#${mesh.index} [${g.kind === 2 ? 'radial' : 'linear'}]`,
    `s=[${g.start.map((x) => x.toFixed(1))}]`,
    `e=[${g.end.map((x) => x.toFixed(1))}]`,
    `stops=${g.stops.map((s) => `${s.offset.toFixed(2)}:${s.color.map((c) => c.toFixed(2)).join('/')}`).join(' ')}`,
    `alpha=${g.alphaStops.map((a) => `${a.offset.toFixed(2)}:${a.alpha.toFixed(2)}`).join(' ') || '-'}`,
  );
}
console.log('gradient meshes:', m.meshes.filter((x) => x.gradient).length);
