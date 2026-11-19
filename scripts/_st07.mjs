import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const collect = (o, out) => {
  if (!o || typeof o !== 'object') return;
  if (Array.isArray(o)) { o.forEach((x) => collect(x, out)); return; }
  if (o.ty === 'sh' && o.ks?.k) {
    const k = o.ks.k;
    if (Array.isArray(k) && k.length && typeof k[0].t === 'number') out.push(k.map((x) => Math.round(x.t)));
  }
  Object.values(o).forEach((x) => collect(x, out));
};
for (const i of [25, 27, 19, 21]) {
  const l = d.layers[i];
  const out = [];
  collect(l.shapes, out);
  console.log('i' + i, l.nm, 'st=' + l.st, 'ip=' + l.ip, 'op=' + l.op, 'pathKfT=' + JSON.stringify(out.slice(0, 4)));
}
