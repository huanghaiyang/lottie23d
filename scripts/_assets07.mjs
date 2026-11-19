import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);
for (const a of d.assets) {
  if (!a.layers?.length) continue;
  const ps = a.layers.map((l) => {
    const k = l.ks?.p?.k;
    const p = Array.isArray(k) && k.length && Array.isArray(k[0].s) ? k[0].s : k;
    return `ind${l.ind}(${l.nm})p=${p ? [Math.round(num(p,0)), Math.round(num(p,1))] : '-'},par=${l.parent ?? '-'},tt=${l.tt ?? 0},td=${l.td ?? 0}`;
  });
  console.log('ASSET', a.id, 'layers=' + a.layers.length);
  ps.forEach((x) => console.log('   ', x));
}
