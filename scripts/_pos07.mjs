import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);
const pinfo = (ks) => {
  const k = ks?.p?.k;
  if (Array.isArray(k) && k.length && typeof k[0] === 'object' && 's' in k[0]) {
    return 'KFRAMES ' + JSON.stringify(k.map((x) => [Math.round(x.t), Math.round(num(x.s, 0)), Math.round(num(x.s, 1))]));
  }
  if (Array.isArray(k)) return 'STATIC ' + JSON.stringify(k.map((x) => Math.round(x)));
  return 'NONE';
};
d.layers.forEach((l, i) => {
  const nm = l.nm || '';
  if (/BG|Machine body|Layer 8|Layer 4|Layer 1/i.test(nm) || l.ty === 1) {
    console.log('i' + i, 'ind=' + l.ind, 'ty=' + l.ty, nm, 'parent=' + (l.parent ?? '-'), 'tp=' + (l.tp ?? '-'), pinfo(l.ks));
  }
});
