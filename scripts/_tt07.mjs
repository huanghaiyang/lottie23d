import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
for (const a of d.assets) {
  if (!a.layers?.length || a.layers.length > 6) continue;
  console.log('=== asset', a.id.slice(0, 6));
  for (const l of a.layers) {
    console.log(`  ind${l.ind} ${l.nm} ty=${l.ty} parent=${l.parent ?? '-'} tt=${l.tt ?? '-'} tp=${l.tp ?? '-'}`);
  }
}
// 顶层 tt/tp
console.log('=== TOP tt layers');
for (const l of d.layers) {
  if (l.tt != null) console.log(`  ind${l.ind} ${l.nm} ty=${l.ty} tt=${l.tt} tp=${l.tp ?? '-'} parent=${l.parent ?? '-'}`);
}
