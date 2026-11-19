import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);
const g = (ks, k) => {
  const v = ks?.[k]?.k;
  if (Array.isArray(v) && v.length && typeof v[0] === 'object' && 's' in v[0]) return v[0].s;
  return v;
};
for (const a of d.assets) {
  if (!a.layers?.length || a.layers.length > 6) continue;
  console.log('=== asset', a.id.slice(0, 6));
  for (const l of a.layers) {
    const p = g(l.ks, 'p'), an = g(l.ks, 'a'), s = g(l.ks, 's');
    console.log(`  ind${l.ind} ${l.nm} par=${l.parent ?? '-'} p=[${num(p,0).toFixed(1)},${num(p,1).toFixed(1)}] a=[${num(an,0).toFixed(1)},${num(an,1).toFixed(1)}] s=[${num(s,0)},${num(s,1)}]`);
  }
}
