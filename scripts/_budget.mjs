import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const files = ['data/shot_01.json','data/shot_02.json','data/shot_03.json','data/shot_04.json','data/shot_05.json','data/shot_06.json','data/shot_07.json','data/shot_08.json','data/shot_09.json','public/animations/dalek.json','public/animations/lights.json','public/animations/starfish.json'];
for (const f of files) {
  if(!fs.existsSync(f)) continue;
  const j=JSON.parse(fs.readFileSync(f,'utf8'));
  const m=parseModel(conv.convertBytes(JSON.stringify(j),{}));
  let animVerts=0, animMeshes=0, red=0, redVerts=0;
  const same=(a,b)=>{if(a.length!==b.length)return false;for(let i=0;i<a.length;i+=3){if(Math.abs(a[i]-b[i])>1e-3||Math.abs(a[i+1]-b[i+1])>1e-3||Math.abs(a[i+2]-b[i+2])>1e-3)return false;}return true;};
  for(const mesh of m.meshes){
    if(!mesh.animated||!mesh.framePositions)continue;
    animMeshes++; animVerts+=mesh.vertexCount;
    const off=mesh.frameOffsets;
    const n=m.frameCount;
    const sl=f=>off?mesh.framePositions.subarray(off[f],off[f+1]):mesh.framePositions.subarray(f*mesh.vertexCount*3,(f+1)*mesh.vertexCount*3);
    let allSame=true;
    for(let k=1;k<n;k++){if(!same(sl(0),sl(k))){allSame=false;break;}}
    if(allSame){red++;redVerts+=mesh.vertexCount;}
  }
  const floatsPerFrame=animVerts*3;
  const realFloats=(animVerts-redVerts)*3;
  const cap8=Math.max(24,Math.floor(8_000_000/Math.max(1,floatsPerFrame)));
  const cap16=Math.max(24,Math.floor(16_000_000/Math.max(1,realFloats||1)));
  console.log(`${f.padEnd(34)} animMesh=${String(animMeshes).padStart(4)} animVert=${String(animVerts).padStart(7)} red=${String(red).padStart(4)} realVert=${String(animVerts-redVerts).padStart(7)} frame=${String(m.frameCount).padStart(4)} cap8=${cap8} cap16real=${cap16}`);
}
