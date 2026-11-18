import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const file = process.argv[2] ?? 'data/shot_05.json';
const j = JSON.parse(fs.readFileSync(file, 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
let g=0,ga=0,gs=0;
for (const mesh of m.meshes) {
  if(!mesh.gradient) continue; g++;
  if(mesh.animated){ga++;} else gs++;
}
console.log(`gradient=${g} animated=${ga} static=${gs}`);
// 各样例渐变动画占比
for(const f of ['data/shot_01.json','data/shot_05.json','data/shot_08.json']){
  const jj=JSON.parse(fs.readFileSync(f,'utf8')); const mm=parseModel(conv.convertBytes(JSON.stringify(jj),{}));
  let a=0,b=0; for(const x of mm.meshes){if(x.gradient){b++; if(x.animated)a++;}}
  console.log(f,`grad=${b} anim=${a}`);
}
