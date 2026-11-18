import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const F = m.frameCount;
const frame = Number(process.argv[2] ?? 4);

function bbox(arr, o0, o1) {
  let mnx=1e9,mny=1e9,mxx=-1e9,mxy=-1e9,n=0;
  for (let i=o0;i<o1;i+=3){const x=arr[i],y=arr[i+1];n++;mnx=Math.min(mnx,x);mxx=Math.max(mxx,x);mny=Math.min(mny,y);mxy=Math.max(mxy,y);}
  return {n, w:mxx-mnx, h:mxy-mny};
}

const agg = {};
const push=(k,cat)=>{const a=agg[k]||(agg[k]={visHealthy:0,visDeg:0,hidden:0});a[cat]++;};

for (const mesh of m.meshes) {
  const key=(mesh.name||'').replace(/\s*·.*$/,'').replace(/\s*\d+$/,'');
  // alpha at frame
  let alpha;
  if (mesh.colors) alpha = mesh.colors[frame*4+3]; else alpha = mesh.color[3];
  const visible = alpha >= 0.004;
  let b;
  if (mesh.animated && mesh.framePositions && mesh.frameOffsets) {
    const off=mesh.frameOffsets; b=bbox(mesh.framePositions, off[frame], off[frame+1]);
  } else b=bbox(mesh.positions,0,mesh.positions.length);
  const deg = b.w<1e-3 && b.h<1e-3;
  if (!visible) push(key,'hidden');
  else if (deg) push(key,'visDeg'); else push(key,'visHealthy');
}
console.log('frame',frame,'of',F);
for (const [k,v] of Object.entries(agg).sort((a,b)=>b[1].visDeg-a[1].visDeg)) {
  console.log(`${k.padEnd(28)} visHealthy=${v.visHealthy} visDeg=${v.visDeg} hidden=${v.hidden}`);
}
