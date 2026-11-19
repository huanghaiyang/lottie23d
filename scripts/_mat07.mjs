import fs from 'node:fs';
import { parseModel } from '../sdk/index.mjs';
const m = parseModel(fs.readFileSync('tmp_model.l2d3'));
const n = m.frameCount;
function show(kw, f) {
  for (const mesh of m.meshes) {
    if (!mesh.name.includes(kw)) continue;
    const tr = m.transforms[mesh.transformIndex];
    const o = [];
    for (let r=0;r<4;r++){const row=[];for(let c=0;c<4;c++)row.push(+tr[f*16+r*4+c].toFixed(2));o.push(row.join(' '));}
    console.log(`--- ${mesh.name} trk=${mesh.transformIndex} f=${f}`);
    console.log(o.join('\n'));
    return;
  }
}
show('print 6 · Fill 1', 33);
show('Layer 1 Outlines · Fill', 33);
show('print 6 · Fill 1', 0);
