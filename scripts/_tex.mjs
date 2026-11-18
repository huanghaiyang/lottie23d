import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';

const conv = await loadConverter();
const files = ['shot_01', 'shot_02', 'shot_07'];
for (const f of files) {
  const j = JSON.parse(fs.readFileSync(`data/${f}.json`, 'utf8'));
  const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
  let tex = 0, gradLin = 0, gradRad = 0, plain = 0;
  const radSample = [];
  for (const mesh of m.meshes) {
    if (mesh.texture) tex++;
    if (mesh.gradient) {
      if (mesh.gradient.kind === 2) { gradRad++; if (radSample.length < 2) radSample.push(mesh.gradient); }
      else gradLin++;
    } else plain++;
  }
  console.log(`${f}: meshes=${m.meshes.length} texture=${tex} gradLin=${gradLin} gradRad=${gradRad} plain=${plain}`);
  for (const g of radSample) {
    console.log('  radial start', g.start.map((v) => v.toFixed(1)), 'end', g.end.map((v) => v.toFixed(1)),
      'stops', g.stops.map((s) => `${s.offset.toFixed(2)}:${s.color.map((c) => c.toFixed(2)).join(',')}`).join(' '));
  }
}
