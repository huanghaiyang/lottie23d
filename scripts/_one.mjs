import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const F = m.frameCount; const frame = 4;

function bbox(arr, o0, o1) {
  let mnx=1e9,mny=1e9,mxx=-1e9,mxy=-1e9,n=0;
  for (let i=o0;i<o1;i+=3){const x=arr[i],y=arr[i+1];n++;mnx=Math.min(mnx,x);mxx=Math.max(mxx,x);mny=Math.min(mny,y);mxy=Math.max(mxy,y);}
  return {n, w:+(mxx-mnx).toFixed(2), h:+(mxy-mny).toFixed(2), mn:[+mnx.toFixed(1),+mny.toFixed(1)], mx:[+mxx.toFixed(1),+mxy.toFixed(1)]};
}

let shown=0;
for (const mesh of m.meshes) {
  if (!/print/.test(mesh.name||'')) continue;
  const alpha = mesh.colors ? mesh.colors[frame*4+3] : mesh.color[3];
  if (alpha < 0.004) continue;
  const sb = bbox(mesh.positions,0,mesh.positions.length);
  let fb;
  if (mesh.framePositions && mesh.frameOffsets){const off=mesh.frameOffsets;fb=bbox(mesh.framePositions,off[frame],off[frame+1]);} else fb=sb;
  const deg = fb.w<1e-3 && fb.h<1e-3;
  if (!deg) continue;
  console.log('===', mesh.name, 'vtx=',mesh.vertexCount,'buckets=',mesh.buckets.length,'color=',mesh.color.map(c=>+c.toFixed(2)).join(','));
  console.log('  static', JSON.stringify(sb));
  console.log('  f4    ', JSON.stringify(fb));
  // all frames bbox
  if (mesh.framePositions && mesh.frameOffsets){const off=mesh.frameOffsets;const line=[];for(let f=0;f<F;f++){const b=bbox(mesh.framePositions,off[f],off[f+1]);line.push(`${f}:${b.w.toFixed(1)}x${b.h.toFixed(1)}`);}console.log('  frames',line.join(' '));}
  shown++; if(shown>=6) break;
}
console.log('shown',shown);
