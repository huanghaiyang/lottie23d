import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const F = m.frameCount; const frame = 4;
function bbox(arr,o0,o1){let mnx=1e9,mny=1e9,mxx=-1e9,mxy=-1e9;for(let i=o0;i<o1;i+=3){const x=arr[i],y=arr[i+1];mnx=Math.min(mnx,x);mxx=Math.max(mxx,x);mny=Math.min(mny,y);mxy=Math.max(mxy,y);}return{w:+(mxx-mnx).toFixed(2),h:+(mxy-mny).toFixed(2)};}
for (const mesh of m.meshes) {
  if (!/mask 2|print 1|Outlines/.test(mesh.name||'')) continue;
  const alpha = mesh.colors ? mesh.colors[frame*4+3] : mesh.color[3];
  let fb = mesh.framePositions&&mesh.frameOffsets ? bbox(mesh.framePositions,mesh.frameOffsets[frame],mesh.frameOffsets[frame+1]) : bbox(mesh.positions,0,mesh.positions.length);
  const sb = bbox(mesh.positions,0,mesh.positions.length);
  console.log(`${mesh.name.padEnd(34)} alpha=${alpha.toFixed(2)} static=${sb.w}x${sb.h} f4=${fb.w}x${fb.h} anim=${mesh.animated}`);
}
