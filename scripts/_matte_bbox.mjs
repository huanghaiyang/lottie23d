import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
function bboxOf(shapes) {
  const xs = [], ys = [];
  const walk = ss => { for (const s of ss||[]) { if (s.ty==='gr') return walk(s.it); if (s.ty==='sh' && s.ks && Array.isArray(s.ks.k)) { for (const p of s.ks.k) { if (p.v) { xs.push(p.v[0]); ys.push(p.v[1]); } } } } };
  walk(shapes);
  if (!xs.length) return null;
  return [Math.min(...xs).toFixed(1), Math.min(...ys).toFixed(1), Math.max(...xs).toFixed(1), Math.max(...ys).toFixed(1)];
}
for (const a of j.assets) {
  if (!a.layers || !a.layers.some(l => /print 6/.test(l.nm||''))) continue;
  console.log('=== asset', a.id.slice(0,12), '===');
  for (const l of a.layers) {
    console.log(` ${l.nm} tt=${l.tt} td=${l.td} parent=${l.parent} bbox=${JSON.stringify(bboxOf(l.shapes))}`);
  }
}
