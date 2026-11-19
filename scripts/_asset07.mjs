import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const assets = d.assets || [];
// 找含 print5 / Can 4 的预合成
for (const a of assets) {
  const layers = a.layers || [];
  const names = layers.map(l => l.nm || '').join(' ');
  if (/print 5|Can 4|print5/i.test(names)) {
    console.log('=== asset', a.id.slice(0, 12), 'layers=', layers.length);
    for (const l of layers) {
      const ks = l.ks || {};
      const ch = [
        ks.p && ks.p.a ? 'p' : '',
        ks.s && ks.s.a ? 's' : '',
        ks.r && ks.r.a ? 'r' : '',
        ks.o && ks.o.a ? 'o' : '',
        ks.a && ks.a.a ? 'a' : '',
      ].filter(Boolean).join('');
      const pos = ks.p && ks.p.k && !ks.p.a ? JSON.stringify(ks.p.k) : '';
      console.log(`  ind=${l.ind ?? '-'} par=${l.parent ?? '-'} td=${l.td ?? 0} ty=${l.ty} tt=${l.tt ?? 0} hd=${!!l.hd} [${ch}] ${l.nm || ''} ${pos}`);
    }
  }
}
// 全局：是否有 hd 层、ty2 层、图片素材
const all = [...d.layers];
for (const a of assets) all.push(...(a.layers || []));
console.log('\nlayers total scanned=', all.length);
console.log('hd layers=', all.filter(l => l.hd).length);
console.log('ty2 layers=', all.filter(l => l.ty === 2).length);
console.log('image assets=', assets.filter(a => a.type === 'image' || a.ref).length);
