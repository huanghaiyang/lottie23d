import { readFileSync } from 'node:fs';
import { parseModel } from '../sdk/index.mjs';

const file = process.argv[2];
const frames = process.argv.slice(3).map(Number);
const bytes = readFileSync(file);
const m = parseModel(bytes);
console.log(`frames=${m.frameCount} ${m.width}x${m.height} fps=${m.sampleFps} meshes=${m.meshes.length}`);
for (const mesh of m.meshes) {
  const track = m.transforms[mesh.transformIndex];
  const poses = frames.map((f) => {
    const o = f * 16;
    if (!track) return '-';
    return `(${track[o + 12].toFixed(1)},${track[o + 13].toFixed(1)})`;
  });
  let alpha = '-';
  if (mesh.colors) {
    alpha = frames.map((f) => mesh.colors[f * 4 + 3].toFixed(2)).join(',');
  } else {
    alpha = mesh.color[3].toFixed(2);
  }
  let bbox = '';
  if (mesh.positions.length) {
    let x0 = 1e9, x1 = -1e9, y0 = 1e9, y1 = -1e9;
    for (let i = 0; i < mesh.positions.length; i += 3) {
      x0 = Math.min(x0, mesh.positions[i]); x1 = Math.max(x1, mesh.positions[i]);
      y0 = Math.min(y0, mesh.positions[i + 1]); y1 = Math.max(y1, mesh.positions[i + 1]);
    }
    bbox = `x[${x0.toFixed(0)},${x1.toFixed(0)}] y[${y0.toFixed(0)},${y1.toFixed(0)}]`;
  }
  const grad = mesh.gradient ? ` GRAD ${mesh.gradient.kind} stops=${mesh.gradient.stops.length}` : '';
  console.log(`${mesh.index} ${mesh.name.padEnd(26)} tr=${mesh.transformIndex} ${poses.join(' ')} a=[${alpha}] ${bbox}${grad}`);
}
