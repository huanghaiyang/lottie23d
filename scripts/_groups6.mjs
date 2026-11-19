import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find(x => x.id.startsWith('6_1_'));
const mask = a.layers.find(l => l.nm === 'mask');
for (const g of mask.shapes) {
  if (g.ty !== 'gr') continue;
  const tr = g.it.find(x => x.ty === 'tr');
  const sh = g.it.find(x => x.ty === 'sh');
  const p = tr?.ks?.p?.k;
  const v = sh?.ks?.k?.v || [];
  let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
  for (const pt of v){x0=Math.min(x0,pt[0]);y0=Math.min(y0,pt[1]);x1=Math.max(x1,pt[0]);y1=Math.max(y1,pt[1]);}
  console.log(`group "${g.nm}" tr.p=${JSON.stringify(p)} vbbox=[${x0.toFixed(1)},${y0.toFixed(1)}..${x1.toFixed(1)},${y1.toFixed(1)}] n=${v.length}`);
}
