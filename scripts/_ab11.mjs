import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('11_'));
for (const l of a.layers) {
  console.log('ind', l.ind, l.nm, 'p=', JSON.stringify(l.ks?.p), 's=', JSON.stringify(l.ks?.s));
}
