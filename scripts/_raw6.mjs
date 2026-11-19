import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find(x => x.id.startsWith('6_1_'));
for (const l of a.layers) {
  console.log(`ind=${l.ind} nm="${l.nm}" tt=${l.tt} td=${l.td} parent=${JSON.stringify(l.parent)} ref=${l.refId||''}`);
}
