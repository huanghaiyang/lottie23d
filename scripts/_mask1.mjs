import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('1_'));
const mask = a.layers.find((l) => l.ind === 2);
function walk(items, depth) {
  for (const it of items ?? []) {
    const pad = '  '.repeat(depth);
    let extra = '';
    if (it.ty === 'tr') {
      extra = ' ks=' + JSON.stringify(it.ks ?? null).slice(0, 200);
    }
    if (it.ty === 'sh') {
      const v = it.ks?.k?.v ?? [];
      extra = ' n=' + v.length + ' v0=' + JSON.stringify(v[0]);
    }
    console.log(pad + it.ty, it.nm, extra);
    if (it.it) walk(it.it, depth + 1);
  }
}
walk(mask.shapes, 0);
console.log('layer p=', JSON.stringify(mask.ks.p.k), 'a=', JSON.stringify(mask.ks.a.k));
