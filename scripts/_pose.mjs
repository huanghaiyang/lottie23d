import { readFileSync } from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';

const conv = await loadConverter();
const file = process.argv[2];
const frames = process.argv.slice(3).map(Number);
const j = JSON.parse(readFileSync(file, 'utf8'));
const bytes = conv.convertBytes(JSON.stringify(j), {});
const m = parseModel(bytes);
console.log('frameCount', m.frameCount, 'transforms', m.transforms.length);
for (const mesh of m.meshes) {
  const track = m.transforms[mesh.transformIndex];
  const pose = frames.map((f) => {
    const o = f * 16;
    if (!track) return '-';
    return `(${track[o + 12].toFixed(1)}, ${track[o + 13].toFixed(1)})`;
  });
  console.log(`${mesh.name.padEnd(28)} tr=${mesh.transformIndex} vert=${mesh.vertexCount} ${pose.join('  ')}`);
}
