import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find(x => x.id.startsWith('1_1_'));
console.log('asset1 layers=', a.layers.length);
for (const l of a.layers) {
  const ks = l.ks || {};
  const g = k => ks[k] ? JSON.stringify(ks[k].k ?? ks[k]) : '-';
  console.log(`  ind=${l.ind} nm=${l.nm} ty=${l.ty} tt=${l.tt ?? 0} td=${l.td ?? 0} parent=${l.parent ?? '-'} p=${g('p')} a=${g('a')} s=${g('s')} r=${g('r')}`);
}
