import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
for (const l of d.layers) {
  if (/Can 4|Can4/.test(l.nm)) console.log('TOP', JSON.stringify({nm:l.nm, ind:l.ind, ty:l.ty, tt:l.tt, td:l.td, ref:l.refId, parent:l.parent}));
}
for (const a of d.assets) {
  const names = a.layers.map(l => `${l.ind}:${l.nm}(tt${l.tt},p${l.parent??'-'})`).join(' | ');
  console.log(a.id.slice(0,18), names);
}
