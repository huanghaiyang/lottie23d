import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('3_'));
const l = a.layers.find((x) => x.nm === 'print 8');
console.log('ind', l.ind, 'parent', l.parent, 'p', JSON.stringify(l.ks.p));
for (const g of l.shapes ?? []) {
  const tr = (g.it ?? []).find((i) => i.ty === 'tr');
  const sh = (g.it ?? []).filter((i) => i.ty === 'sh');
  console.log('GROUP', g.nm, 'tr=', JSON.stringify(tr?.ks?.k ?? tr?.ks));
  for (const s of sh) {
    const v = s.ks?.k?.v;
    console.log('  path v=', Array.isArray(v) ? v.length + ' pts, first=' + JSON.stringify(v[0]) : JSON.stringify(v).slice(0, 120));
  }
}
