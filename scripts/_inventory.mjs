import { readdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dirs = ['data', 'public/animations'];
const shapeTy = new Map();
const layerTy = new Map();
for (const d of dirs) {
  for (const f of readdirSync(join(root, d))) {
    if (!f.endsWith('.json')) continue;
    const j = JSON.parse(readFileSync(join(root, d, f), 'utf8'));
    const walk = (o, inLayer) => {
      if (Array.isArray(o)) return o.forEach((x) => walk(x, inLayer));
      if (!o || typeof o !== 'object') return;
      if ('ty' in o) {
        if (inLayer) layerTy.set(o.ty, (layerTy.get(o.ty) || 0) + 1);
        else shapeTy.set(o.ty, (shapeTy.get(o.ty) || 0) + 1);
      }
      if (Array.isArray(o.layers)) o.layers.forEach((l) => walk(l, true));
      if (Array.isArray(o.it)) o.it.forEach((c) => walk(c, false));
      Object.values(o).forEach((v) => {
        if (v && typeof v === 'object' && !Array.isArray(v.it) && !Array.isArray(v.layers)) walk(v, inLayer);
      });
    };
    walk(j, false);
  }
}
console.log('layer ty:', [...layerTy.entries()].sort().map(([k, v]) => `${k}=${v}`).join(' '));
console.log('shape ty:', [...shapeTy.entries()].sort().map(([k, v]) => `${k}=${v}`).join(' '));
