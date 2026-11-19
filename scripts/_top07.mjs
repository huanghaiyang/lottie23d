import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
for (const l of d.layers) {
  const ks = l.ks || {};
  const p = ks.p ? JSON.stringify(ks.p.k ?? ks.p) : '-';
  const a = ks.a ? JSON.stringify(ks.a.k ?? ks.a) : '-';
  const s = ks.s ? JSON.stringify(ks.s.k ?? ks.s) : '-';
  const r = ks.r ? JSON.stringify(ks.r.k ?? ks.r) : '-';
  console.log(`ind=${l.ind} nm=${l.nm} ty=${l.ty} tt=${l.tt ?? 0} refId=${l.refId ?? '-'} parent=${l.parent ?? '-'} p=${p} a=${a} s=${s} r=${r}`);
}
