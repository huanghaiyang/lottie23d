import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));

function dumpLayer(l, path, depth) {
  const o = l.ks && l.ks.o;
  const ok = o && o.a === 1 ? o.k.map(k => `t=${k.t} s=${JSON.stringify(k.s)}`) : (o ? JSON.stringify(o.k) : 'none');
  console.log(`${'  '.repeat(depth)}[${l.ty}] ${l.nm} ind=${l.ind} parent=${l.parent ?? '-'} ip=${l.ip} op=${l.op} st=${l.st} sr=${l.sr}`);
  console.log(`${'  '.repeat(depth)}  opacity: ${ok}`);
}

function walkLayers(ls, path, depth) {
  (ls || []).forEach(l => {
    if (/print 9|Layer 1 Outlines|Pick a drink|Can area|Group 21472/.test(l.nm || '')) dumpLayer(l, path, depth);
    if (l.layers) walkLayers(l.layers, path + '/' + l.nm, depth + 1);
  });
}
walkLayers(j.layers, 'root', 0);
(j.assets || []).forEach(a => {
  console.log(`=== asset ${a.nm} fr=${a.fr} ip=${a.ip} op=${a.op} st=${a.st} layers=${a.layers.length}`);
  walkLayers(a.layers, 'asset:' + a.nm, 1);
});
