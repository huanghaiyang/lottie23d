import { readFileSync } from 'node:fs';
const j = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const walk = (o, layer) => {
  if (Array.isArray(o)) return o.forEach((x) => walk(x, layer));
  if (!o || typeof o !== 'object') return;
  if (o.ty === 'sh' && o.ks && typeof o.ks.x === 'string') {
    console.log('LAYER', JSON.stringify(layer?.nm), 'EXPR:');
    console.log(o.ks.x);
    console.log('---');
  }
  if (Array.isArray(o.layers)) o.layers.forEach((l) => walk(l.shapes, l));
  Object.values(o).forEach((v) => walk(v, layer));
};
walk(j.layers, null);
