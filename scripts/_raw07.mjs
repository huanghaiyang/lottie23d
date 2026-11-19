import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = j.assets.find(x => x.id.startsWith('4_1_a584'));
for (const l of a.layers) {
  console.log('===', l.nm, '===');
  const g = (l.shapes||[]).find(s=>s.ty==='gr');
  if (g) {
    for (const it of g.it.slice(0,3)) {
      console.log(' it ty=', it.ty, JSON.stringify(it).slice(0, 400));
    }
  }
}
