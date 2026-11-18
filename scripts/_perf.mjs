import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_07.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));

let animated=0, staticM=0, degStatic=0, transparent=0, grad=0, tex=0, hidden=0;
let tris=0, verts=0, animVerts=0;
const layers = new Set();
let degTri=0;
for (const mesh of m.meshes) {
  layers.add(mesh.transformIndex);
  const tri = mesh.indexCount/3;
  tris += tri; verts += mesh.vertexCount;
  const alpha = mesh.colors ? mesh.colors[3] : mesh.color[3];
  if (alpha < 0.004) hidden++;
  if (alpha < 0.999) transparent++;
  if (mesh.animated) { animated++; animVerts += mesh.vertexCount; } else staticM++;
  if (mesh.gradient) grad++;
  if (mesh.texture) tex++;
  // static degenerate bbox
  if (!mesh.animated) {
    let mnx=1e9,mny=1e9,mxx=-1e9,mxy=-1e9;
    for (let i=0;i<mesh.positions.length;i+=3){const x=mesh.positions[i],y=mesh.positions[i+1];mnx=Math.min(mnx,x);mxx=Math.max(mxx,x);mny=Math.min(mny,y);mxy=Math.max(mxy,y);}
    if (mxx-mnx<1e-3 && mxy-mny<1e-3){degStatic++;degTri+=tri;}
  }
}
console.log(file);
console.log(`meshes=${m.meshes.length} layers=${layers.size} frameCount=${m.frameCount}`);
console.log(`tri=${Math.round(tris)} verts=${verts} animVerts=${animVerts}`);
console.log(`animated=${animated} static=${staticM} hidden=${hidden} transparent=${transparent} gradient=${grad} texture=${tex}`);
console.log(`staticDegenerate=${degStatic} (tri=${Math.round(degTri)})`);
