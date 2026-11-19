import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_07.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));

let colorTrack=0, grad=0, gradAnim=0, tex=0;
let animPos=0, staticM=0, animVerts=0, staticVerts=0;
let transparent=0, alpha0=0, staticHidden=0;
let framePosFloats=0, animFloatsPerFrame=0, gradAnimVerts=0;
let tris=0, verts=0;
const frames = m.frameCount;
for (const mesh of m.meshes) {
  const tri = mesh.indexCount/3; tris += tri; verts += mesh.vertexCount;
  const alpha = mesh.colors ? mesh.colors[3] : mesh.color[3];
  if (mesh.colors) colorTrack++;
  if (mesh.gradient) { grad++; if (mesh.animated) gradAnim++; gradAnimVerts += mesh.animated? mesh.vertexCount:0; }
  if (mesh.texture) tex++;
  if (alpha < 0.004) alpha0++; else if (alpha < 0.999) transparent++;
  if (mesh.animated) { animPos++; animVerts += mesh.vertexCount; }
  else { staticM++; staticVerts += mesh.vertexCount; if (!mesh.staticVisible? true:false) staticHidden++; }
  if (mesh.framePositions) framePosFloats += mesh.framePositions.length;
  if (mesh.animated && mesh.framePositions) animFloatsPerFrame += mesh.vertexCount*3;
}
console.log(file);
console.log(`frames=${frames} meshes=${m.meshes.length} layers=${new Set(m.meshes.map(x=>x.transformIndex)).size}`);
console.log(`verts=${verts} tris=${Math.round(tris)}`);
console.log(`animatedPos=${animPos} (verts=${animVerts})  static=${staticM} (verts=${staticVerts})`);
console.log(`colorTrack=${colorTrack}  gradient=${grad} (anim=${gradAnim}, animVerts=${gradAnimVerts})  texture=${tex}`);
console.log(`transparent=${transparent}  alpha0=${alpha0}`);
console.log(`framePositions total floats=${framePosFloats} (~${(framePosFloats*4/1048576).toFixed(1)} MB)`);
console.log(`per-frame animated vertex floats=${animFloatsPerFrame} (~${(animFloatsPerFrame*4/1048576).toFixed(1)} MB/frame upload)`);
// model size hint
const raw = fs.readFileSync(file);
console.log(`source json KB=${(raw.length/1024).toFixed(0)}`);
