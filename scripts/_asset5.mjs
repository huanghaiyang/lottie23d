import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
for (const aid of ['5_1_9012e0d0', '6_1_e60256c2']) {
  const a = d.assets.find(x => x.id.startsWith(aid));
  console.log('\n=== asset', aid);
  for (const l of a.layers) {
    const ks = l.ks || {};
    const g = k => ks[k] ? JSON.stringify(ks[k].k ?? ks[k]) : '-';
    console.log(`  ind=${l.ind} nm=${l.nm} ty=${l.ty} tt=${l.tt ?? 0} parent=${l.parent ?? '-'} p=${g('p')} a=${g('a')} s=${g('s')} r=${g('r')}`);
  }
}
