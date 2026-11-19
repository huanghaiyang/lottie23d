import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
for (const a of j.assets) {
  if (!/6_1_e6|5_1_901/.test(a.id)) continue;
  console.log('===', a.id.slice(0,12));
  for (const l of a.layers) {
    console.log(` ind=${l.ind} nm=${l.nm} parent=${l.parent} tt=${l.tt} td=${l.td}`);
  }
}
