import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = (j.assets||[]).find(x=>x.nm==='Can 1');
const kf = v => v && (Array.isArray(v.k) && v.k.some(k=>k&&k.o!==undefined));
for(const l of a.layers){
  const shapes = l.shapes||[];
  const shAnim = shapes.some(s=>JSON.stringify(s).includes('"k":['));
  const m = l.masks||[];
  const pAnim = kf(l.ks&&l.ks.p);
  console.log(`ind=${l.ind} nm=${l.nm} ty=${l.ty} tt=${l.tt||0} ip=${l.ip} op=${l.op} masks=${m.length} shAnim=${shAnim} pAnim=${pAnim} shapes=${shapes.length}`);
}
