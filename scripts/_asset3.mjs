import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('3_'));
function bbox(items) {
  let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9,n=0;
  const walk = (its) => { for (const it of its??[]) { if (it.ty==='gr') walk(it.it); else if (it.ty==='sh') { for (const p of (it.ks?.k?.v??[])) { n++; x0=Math.min(x0,p[0]); y0=Math.min(y0,p[1]); x1=Math.max(x1,p[0]); y1=Math.max(y1,p[1]); } } } };
  walk(items);
  return n?`x=[${x0.toFixed(1)},${x1.toFixed(1)}] y=[${y0.toFixed(1)},${y1.toFixed(1)}] n=${n}`:'-';
}
for (const l of a.layers) {
  console.log(`ind=${l.ind} ty=${l.ty} tt=${l.tt} tp=${l.tp} par=${l.parent} nm=${l.nm} p=${JSON.stringify(l.ks?.p?.k)} a=${JSON.stringify(l.ks?.a?.k)} ${bbox(l.shapes)}`);
}
