import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
for (const i of [25, 26, 27, 28, 36, 37]) {
  const l = d.layers[i];
  const s = JSON.stringify(l.shapes ?? []);
  const hasKf = /"k"\s*:\s*\[/.test(s) && s.includes('"t"');
  console.log('i' + i, l.nm, 'ty=' + l.ty, 'td=' + (l.td ?? 0), 'tt=' + (l.tt ?? 0),
    'st=' + l.st, 'sr=' + l.sr, 'ip=' + l.ip, 'op=' + l.op, 'shapeKf=' + hasKf,
    'p=' + JSON.stringify(l.ks?.p?.k?.[0] ? l.ks.p.k.map((k) => k.s) : l.ks?.p?.k));
}
