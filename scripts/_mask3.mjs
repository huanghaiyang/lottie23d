import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('3_'));
const mask = a.layers.find((l) => l.ind === 2);
function num(v, i = 0) { return Array.isArray(v) ? v[i] ?? 0 : v ?? 0; }
function walk(items, depth) {
  for (const it of items ?? []) {
    const pad = '  '.repeat(depth);
    if (it.ty === 'gr') {
      const tr = (it.it ?? []).find((i) => i.ty === 'tr');
      let tstr = '';
      if (tr) {
        const k = tr.ks?.k;
        const v = Array.isArray(k) && k.length && typeof k[0] === 'object' ? k[0].s : k;
        tstr = v ? ` tr p=[${num(v,0)},${num(v,1)}] a=[${num(v,2)},${num(v,3)}] s=[${num(v,4)},${num(v,5)}] r=${num(v,6)}` : ' tr(k=?)';
      }
      console.log(pad + 'GR', it.nm, tstr);
      walk(it.it, depth + 1);
    } else if (it.ty === 'sh') {
      const v = it.ks?.k?.v ?? [];
      const xs = v.map((p) => p[0]), ys = v.map((p) => p[1]);
      console.log(pad + 'SH', it.nm, 'n=', v.length, 'x=[', Math.min(...xs).toFixed(1), Math.max(...xs).toFixed(1), '] y=[', Math.min(...ys).toFixed(1), Math.max(...ys).toFixed(1), ']');
      console.log(pad + '  v=', v.slice(0, 3).map((p) => p.map((x) => x.toFixed(1)).join(',')).join(' | '));
    }
  }
}
walk(mask.shapes, 0);
console.log('layer ks: p=', JSON.stringify(mask.ks.p.k), 'a=', JSON.stringify(mask.ks.a.k), 's=', JSON.stringify(mask.ks.s?.k), 'r=', JSON.stringify(mask.ks.r?.k));
