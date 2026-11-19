import fs from 'node:fs';
import { parseModel } from '../sdk/index.mjs';
const m = parseModel(fs.readFileSync('tmp_model.l2d3'));
function bbox(slice, vc) {
  let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
  for (let i=0;i<vc;i++){const x=slice[i*3],y=slice[i*3+1];x0=Math.min(x0,x);x1=Math.max(x1,x);y0=Math.min(y0,y);y1=Math.max(y1,y);}
  return Math.max(x1-x0,y1-y0);
}
const target = process.argv[2] || 'print 6 · Fill 1';
const n = m.frameCount;
for (const mesh of m.meshes) {
  if (!mesh.name.includes(target)) continue;
  if (!mesh.framePositions) continue;
  const off = mesh.frameOffsets;
  const tr = m.transforms[mesh.transformIndex];
  const rows = [];
  for (let f=0;f<n;f++){
    const s=mesh.framePositions.subarray(off[f],off[f+1]);
    const diag = s.length? bbox(s,s.length/3):0;
    const a = mesh.colors? mesh.colors[f*4+3]:mesh.color[3];
    const tx = tr[f*16+12], ty = tr[f*16+13];
    rows.push(`${f}:d=${diag.toFixed(1)} a=${a.toFixed(2)} p=(${tx.toFixed(0)},${ty.toFixed(0)})`);
  }
  console.log('===', mesh.name, 'vc='+mesh.vertexCount, '===');
  console.log(rows.join('\n'));
  break;
}
