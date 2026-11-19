import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));

function bbox(slice, vc) {
  let x0=1e9,y0=1e9,z0=1e9,x1=-1e9,y1=-1e9,z1=-1e9;
  for (let i=0;i<vc;i++){
    const x=slice[i*3],y=slice[i*3+1],z=slice[i*3+2];
    x0=Math.min(x0,x);x1=Math.max(x1,x);y0=Math.min(y0,y);y1=Math.max(y1,y);z0=Math.min(z0,z);z1=Math.max(z1,z);
  }
  return {w:x1-x0,h:y1-y0,d:z1-z0,cx:(x0+x1)/2,cy:(y0+y1)/2};
}

const collapsed = [];
for (const mesh of m.meshes) {
  if (!mesh.framePositions || !mesh.frameOffsets) continue;
  const off = mesh.frameOffsets;
  const sl = f => mesh.framePositions.subarray(off[f], off[f+1]);
  const ref = bbox(mesh.positions, mesh.vertexCount);
  // 逐帧 bbox
  let worst = 1e9, framesCollapsed = 0;
  const n = m.frameCount;
  for (let f=0;f<n;f++){
    const s=sl(f); const vc=s.length/3;
    if (vc===0) continue;
    const b=bbox(s,vc);
    const diag=Math.max(b.w,b.h);
    worst=Math.min(worst,diag);
    if (diag < 1e-2) framesCollapsed++;
  }
  const refDiag=Math.max(ref.w,ref.h);
  if (refDiag > 2 && worst < 1e-2) {
    collapsed.push({name:mesh.name, vc:mesh.vertexCount, refDiag:refDiag.toFixed(1), collapsed:framesCollapsed, n, color:[...mesh.color].map(x=>x.toFixed(2)).join(',')});
  }
}
console.log('塌点网格数', collapsed.length, '/ 总', m.meshes.length);
const byName={};
for(const c of collapsed){const k=c.name.replace(/Fill|Gradient|Stroke|\d+/g,'').trim();(byName[k]=byName[k]||[]).push(c);}
for(const [k,arr] of Object.entries(byName).sort((a,b)=>b[1].length-a[1].slice(0,12))){
  console.log(`\n[${k}] x${arr.length}`);
  arr.slice(0,4).forEach(c=>console.log('  ',c.name,'vc='+c.vc,'ref='+c.refDiag,'col='+c.color,'collapsed='+c.collapsed+'/'+c.n));
}
