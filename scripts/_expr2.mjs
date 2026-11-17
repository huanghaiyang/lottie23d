import { readFileSync } from 'node:fs';
const j = JSON.parse(readFileSync(process.argv[2], 'utf8'));
for (const l of j.layers) {
  const p = l.ks?.p;
  if (p && typeof p.x === 'string' && p.x.includes('pointOnPath')) {
    console.log('=====', l.ind, JSON.stringify(l.nm));
    console.log('P EXPR:', p.x);
    console.log('p.k:', JSON.stringify(p.k));
    for (const e of l.ef || []) {
      console.log('EF', e.nm, e.mn, JSON.stringify(e.ef));
    }
  }
}
