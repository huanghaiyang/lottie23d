import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find(x => x.id.startsWith('6_1_'));
console.log('asset', a.id.slice(0, 16));
for (const l of a.layers) {
  console.log('\n--- ind', l.ind, 'nm=', l.nm, 'ty=', l.ty, 'tt=', l.tt, 'td=', l.td, 'parent=', l.parent, 'ip=', l.ip, 'op=', l.op, 'st=', l.st);
  const ks = l.ks || {};
  for (const k of ['p', 'a', 's', 'r', 'o', 'sk', 'sa']) {
    if (ks[k]) console.log('  ', k, JSON.stringify(ks[k]));
  }
}
