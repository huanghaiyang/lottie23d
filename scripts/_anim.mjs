import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_07.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));

function frameSlice(mesh, f) {
  const off = mesh.frameOffsets;
  if (off) return mesh.framePositions.subarray(off[f], off[f+1]);
  const n = mesh.vertexCount*3; return mesh.framePositions.subarray(f*n, (f+1)*n);
}
function same(a,b){ if(a.length!==b.length)return false; for(let i=0;i<a.length;i+=3){ if(Math.abs(a[i]-b[i])>1e-3||Math.abs(a[i+1]-b[i+1])>1e-3)return false;} return true; }

let anim=0, redundant=0, redundantVerts=0, realAnim=0, realVerts=0;
let hiddenAnim=0, hiddenRedundant=0;
for (const mesh of m.meshes) {
  if (!mesh.animated || !mesh.framePositions) continue;
  anim++;
  const a0 = frameSlice(mesh,0);
  let allSame=true;
  for(let f=1;f<m.frameCount;f++){ if(!same(a0,frameSlice(mesh,f))){allSame=false;break;} }
  const alpha0 = mesh.colors?mesh.colors[3]:mesh.color[3];
  const hidden = alpha0<0.004;
  if(allSame){ redundant++; redundantVerts+=mesh.vertexCount; if(hidden)hiddenRedundant++; }
  else { realAnim++; realVerts+=mesh.vertexCount; }
  if(hidden) hiddenAnim++;
}
console.log(file,'frames=',m.frameCount);
console.log(`animated=${anim} redundant(static-geo)=${redundant} (verts=${redundantVerts}) realAnim=${realAnim} (verts=${realVerts})`);
console.log(`hiddenAtF0=${hiddenAnim} hiddenRedundant=${hiddenRedundant}`);
console.log(`animated floats/frame=${(realVerts*3)} budget 8M -> maxFrames=${Math.floor(8_000_000/Math.max(1,realVerts*3))}`);
