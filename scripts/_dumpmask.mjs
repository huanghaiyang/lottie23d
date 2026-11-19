import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find(x => x.id.startsWith('6_1_'));
const mask = a.layers.find(l => l.nm === 'mask');
console.log(JSON.stringify(mask.shapes, null, 1).slice(0, 2500));
