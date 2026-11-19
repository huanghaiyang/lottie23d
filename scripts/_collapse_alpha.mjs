import fs from 'node:fs';
import { parseModel } from '../sdk/index.mjs';
const bytes = fs.readFileSync('tmp_model.l2d3');
const m = parseModel(bytes);

function bbox(slice, vc) {
  let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
  for (let i=0;i<vc;i++){const x=slice[i*3],y=slice[i*3+1];x0=Math.min(x0,x);x1=Math.max(x1,x);y0=Math.min(y0,y);y1=Math.max(y1,y);}
  return {w:x1-x0,h:y1-y0};
}

let collapsed = 0, visibleBad = 0;
const bad = [];
for (const mesh of m.meshes) {
  if (!mesh.framePositions || !mesh.frameOffsets) continue;
  const off = mesh.frameOffsets;
  const n = m.frameCount;
  const ref = bbox(mesh.positions, mesh.vertexCount);
  const refDiag = Math.max(ref.w, ref.h);
  if (refDiag <= 2) continue;
  let frames = 0, visBad = 0;
  const alphas = [];
  for (let f=0;f<n;f++){
    const s = mesh.framePositions.subarray(off[f], off[f+1]);
    const vc = s.length/3;
    if (!vc) continue;
    const b = bbox(s, vc);
    const diag = Math.max(b.w,b.h);
    if (diag < 1e-2) {
      frames++;
      const a = mesh.colors ? mesh.colors[f*4+3] : mesh.color[3];
      alphas.push(+a.toFixed(2));
      if (a > 0.05) visBad++;
    }
  }
  if (frames) {
    collapsed++;
    if (visBad) {
      visibleBad++;
      bad.push({name:mesh.name, frames, visBad, staticA:+mesh.color[3].toFixed(2), colors:!!mesh.colors, alpha:alphas.join(',')});
    }
  }
}
console.log('塌点网格', collapsed, '其中塌帧仍可见(alpha>0.05)', visibleBad);
bad.slice(0,25).forEach(b=>console.log(`  ${b.name} frames=${b.frames} visBad=${b.visBad} staticA=${b.staticA} track=${b.colors} alpha=[${b.alpha}]`));
