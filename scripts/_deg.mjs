import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
let deg = 0, thin = 0, ok = 0, tex = 0, grad = 0;
const degNames = {};
for (const mesh of m.meshes) {
  if (mesh.texture) tex++;
  if (mesh.gradient) grad++;
  const p = mesh.positions;
  let mnx=1e9,mny=1e9,mxx=-1e9,mxy=-1e9;
  for (let i=0;i<p.length;i+=3){mnx=Math.min(mnx,p[i]);mxx=Math.max(mxx,p[i]);mny=Math.min(mny,p[i+1]);mxy=Math.max(mxy,p[i+1]);}
  const w=mxx-mnx, h=mxy-mny;
  if (w<1e-3 && h<1e-3){deg++; degNames[mesh.name]=(degNames[mesh.name]||0)+1;}
  else if (w<1e-3 || h<1e-3) thin++; else ok++;
}
console.log(`meshes=${m.meshes.length} texture=${tex} gradient=${grad} degenerate=${deg} thinLine=${thin} ok=${ok}`);
console.log('degenerate by name:', JSON.stringify(degNames, null, 1));
