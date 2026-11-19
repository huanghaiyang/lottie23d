import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
const F = m.frameCount;
let anim=0, staticLike=0, changed=0;
let changedVerts=0, staticVerts=0;
let bucketSwitch=0;
for (const mesh of m.meshes) {
  if (!mesh.animated || !mesh.framePositions) continue;
  anim++;
  const off = mesh.frameOffsets;
  const buckets = mesh.frameBuckets || [];
  if (buckets.some((b,i)=> i>0 && b!==buckets[0])) bucketSwitch++;
  const sl = (f)=>{ const s=off?off[f]:f*mesh.vertexCount*3; const e=off?off[f+1]:(f+1)*mesh.vertexCount*3; return mesh.framePositions.subarray(s,e); };
  const a = sl(0), b = sl(F-1);
  if (a.length!==b.length) { changed++; changedVerts+=mesh.vertexCount; continue; }
  let maxd=0;
  for (let i=0;i<a.length;i+=3){ maxd=Math.max(maxd,Math.abs(a[i]-b[i]),Math.abs(a[i+1]-b[i+1]),Math.abs(a[i+2]-b[i+2])); }
  if (maxd < 1e-3) { staticLike++; staticVerts+=mesh.vertexCount; } else { changed++; changedVerts+=mesh.vertexCount; }
}
console.log(`animated meshes=${anim}, 拓扑切桶=${bucketSwitch}`);
console.log(`帧间顶点近似不变(误标静态)=${staticLike} verts=${staticVerts}`);
console.log(`帧间顶点真实变化=${changed} verts=${changedVerts}`);
