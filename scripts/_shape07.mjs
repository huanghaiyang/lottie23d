import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = j.assets[0];
function walk(shapes, depth, out) {
  for (const s of shapes) {
    if (s.ty === 'gr') { walk(s.it, depth + 1, out); continue; }
    let info = { ty: s.ty, nm: s.nm };
    if (s.ty === 'sh') {
      const xy = s.ks && s.ks.k;
      if (Array.isArray(xy)) {
        const pts = xy.filter(p => p && p.i !== undefined);
        if (pts.length) {
          const xs = pts.map(p => p.o[0]), ys = pts.map(p => p.o[1]);
          info.pts = pts.length;
          info.bbox = [Math.min(...xs).toFixed(1), Math.min(...ys).toFixed(1), Math.max(...xs).toFixed(1), Math.max(...ys).toFixed(1)];
        } else if (Array.isArray(xy[0])) {
          info.static = xy.map(n => +n.toFixed(1));
        }
      }
    }
    if (s.ty === 'fl' || s.ty === 'st') info.c = s.c && s.c.k, info.o = s.o && s.o.k;
    out.push('  '.repeat(depth) + JSON.stringify(info));
  }
}
for (const l of a.layers) {
  console.log('===', l.nm, '===');
  const out = [];
  walk(l.shapes || [], 0, out);
  console.log(out.join('\n'));
}
