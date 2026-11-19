import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const layers = d.layers;
const byTy = {};
for (const l of layers) byTy[l.ty] = (byTy[l.ty] || 0) + 1;
console.log('layers=', layers.length, 'byTy=', JSON.stringify(byTy));

const ass = d.assets || [];
console.log('assets=', ass.length);
for (const a of ass) {
  console.log('  asset', a.id, 'type=', a.type || ('precomp layers=' + (a.layers ? a.layers.length : 0)),
    'ref=', a.ref ? a.ref.slice(0, 20) : '-');
}

const imgs = layers.filter(l => l.ty === 2);
console.log('ty2 image layers=', imgs.length);

const chan = { p: 0, s: 0, r: 0, o: 0, a: 0, op: 0 };
const staticKs = { p: 0, s: 0, r: 0, o: 0 };
for (const l of layers) {
  const ks = l.ks || {};
  if (ks.p && ks.p.a) chan.p++;
  if (ks.s && ks.s.a) chan.s++;
  if (ks.r && ks.r.a) chan.r++;
  if (ks.o && ks.o.a) chan.o++;
  if (ks.a && ks.a.a) chan.a++;
  if (ks.op && ks.op.a) chan.op++;
}
console.log('layer transform animated:', JSON.stringify(chan));

let tt1 = 0, masks = 0;
for (const l of layers) {
  if (l.tt === 1) tt1++;
  if (l.masksProperties && l.masksProperties.length) masks++;
}
console.log('tt=1 matte=', tt1, 'layers with masks=', masks);

// per-layer: parent, td, ind, animated channels, name
console.log('\n--- layers (ind/parent/td/ty/channels) ---');
for (const l of layers) {
  const ks = l.ks || {};
  const ch = [
    ks.p && ks.p.a ? 'p' : '',
    ks.s && ks.s.a ? 's' : '',
    ks.r && ks.r.a ? 'r' : '',
    ks.o && ks.o.a ? 'o' : '',
    ks.a && ks.a.a ? 'a' : '',
  ].filter(Boolean).join('');
  console.log(`${l.ind ?? '-'} par=${l.parent ?? '-'} td=${l.td ?? 0} ty=${l.ty} tt=${l.tt ?? 0} [${ch}] ${l.nm || ''}`);
}
