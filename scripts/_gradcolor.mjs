import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_05.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
let grad=0, gradBlack=0, plain=0, plainBlack=0;
const sample=[];
for (const mesh of m.meshes) {
  const c=mesh.color;
  const dark = c? (c[0]<0.02&&c[1]<0.02&&c[2]<0.02) : true;
  if(mesh.gradient){grad++; if(dark)gradBlack++; if(sample.length<8)sample.push(['grad',c,mesh.name]);}
  else {plain++; if(dark)plainBlack++;}
}
console.log(file,'meshes=',m.meshes.length);
console.log(`gradient=${grad} black=${gradBlack}  plain=${plain} black=${plainBlack}`);
for(const s of sample) console.log(s[0], s[1], s[2]);
