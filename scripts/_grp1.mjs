import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('1_'));
function walk(items, depth, label) {
  for (const it of items ?? []) {
    const pad = '  '.repeat(depth);
    if (it.ty === 'gr') {
      const tr = it.it?.find((i) => i.ty === 'tr');
      let ts = '-';
      if (tr) ts = `p=[${tr.p?.k}] a=[${tr.a?.k}] s=[${tr.s?.k}]`;
      console.log(pad + label + ' GR', it.nm, 'tr=', ts);
      walk(it.it, depth + 1, label);
    } else if (it.ty === 'sh') {
      const v = it.ks?.k?.v ?? [];
      const xs=v.map(p=>p[0]), ys=v.map(p=>p[1]);
      console.log(pad + label + ' SH', it.nm, `x=[${Math.min(...xs).toFixed(1)},${Math.max(...xs).toFixed(1)}] y=[${Math.min(...ys).toFixed(1)},${Math.max(...ys).toFixed(1)}]`);
    }
  }
}
for (const l of a.layers) {
  if (l.ind === 3 || l.ind === 4) {
    console.log('=== ind', l.ind, l.nm, 'par=', l.parent, '===');
    walk(l.shapes, 0, `i${l.ind}`);
  }
}
