import { readFileSync } from 'node:fs';
const j = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const walk = (o, path, layer) => {
  if (Array.isArray(o)) return o.forEach((v, i) => walk(v, `${path}[${i}]`, layer));
  if (!o || typeof o !== 'object') return;
  if (typeof o.x === 'string') {
    console.log(`LAYER=${layer?.nm} (ind=${layer?.ind}) PATH=${path}`);
    console.log('   ', o.x.slice(0, 140).replace(/\n/g, ' '));
  }
  if (Array.isArray(o.layers)) o.layers.forEach((l) => walk(l, `layer(${l.ind})`, l));
  Object.entries(o).forEach(([k, v]) => {
    if (k !== 'layers') walk(v, `${path}.${k}`, layer);
  });
};
walk(j, 'root', null);
