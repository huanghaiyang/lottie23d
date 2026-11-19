import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));

function walk(shapes, acc, tr = [0,0]) {
  for (const s of shapes || []) {
    if (s.ty === 'gr') {
      const gtr = s.it?.find(x => x.ty === 'tr');
      const gp = gtr?.ks?.p?.k ?? [0,0];
      walk(s.it, acc, [tr[0]+gp[0], tr[1]+gp[1]]);
    } else if (s.ty === 'sh') {
      const v = s.ks?.k?.v;
      if (v) acc.push({v, tr});
    }
  }
}
function bbox(layer) {
  const acc = []; walk(layer.shapes, acc);
  let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9,n=0;
  for (const {v,tr} of acc) for (const pt of v) {
    n++; x0=Math.min(x0,pt[0]+tr[0]); y0=Math.min(y0,pt[1]+tr[1]);
    x1=Math.max(x1,pt[0]+tr[0]); y1=Math.max(y1,pt[1]+tr[1]);
  }
  return n?`[${x0.toFixed(1)},${y0.toFixed(1)}..${x1.toFixed(1)},${y1.toFixed(1)}] n=${n}`:'empty';
}
for (const aid of ['5_1_9012e0d0','6_1_e60256c2']) {
  const a = d.assets.find(x => x.id.startsWith(aid));
  console.log('===', aid.slice(0,8));
  for (const l of a.layers) console.log(`  ind=${l.ind} ${l.nm} localBbox=${bbox(l)}`);
}
