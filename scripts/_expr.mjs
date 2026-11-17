import { readFileSync } from 'node:fs';

const file = process.argv[2];
const j = JSON.parse(readFileSync(file, 'utf8'));
const list = j.layers || [];
for (const l of list) {
  const ks = l.ks || {};
  const exprs = {};
  for (const k of ['p', 'r', 'o', 'a', 's']) {
    const prop = ks[k];
    if (prop && typeof prop === 'object' && typeof prop.x === 'string') exprs[k] = prop.x.slice(0, 90).replace(/\n/g, ' ');
  }
  const ef = (l.ef || []).map((e) => e.nm || e.ty);
  console.log(
    `#${l.ind} ty=${l.ty} parent=${l.parent ?? '-'} nm=${JSON.stringify(l.nm)}`,
    ef.length ? ` ef=[${ef.join(',')}]` : '',
  );
  for (const [k, v] of Object.entries(exprs)) console.log(`    ${k}: ${v}`);
}
