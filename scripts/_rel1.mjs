import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('1_'));
const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);
const g = (ks, k) => {
  const v = ks?.[k]?.k;
  if (Array.isArray(v) && v.length && typeof v[0] === 'object' && 's' in v[0]) return v[0].s;
  return v;
};
const by = {};
for (const l of a.layers) by[l.ind] = l;
function mat(layer) {
  const ks = layer.ks;
  const p = g(ks, 'p'), an = g(ks, 'a'), s = g(ks, 's'), r = g(ks, 'r') ?? 0, sk = g(ks, 'sk') ?? 0;
  const sx = num(s, 0) / 100, sy = num(s, 1) / 100;
  const rad = (r * Math.PI) / 180;
  const ca = Math.cos(rad) * sx, sa = Math.sin(rad) * sx;
  const dd = sy * Math.cos(rad), c = -sy * Math.sin(rad);
  const tx = num(p, 0) - (ca * num(an, 0) + c * num(an, 1));
  const ty = num(p, 1) - (sa * num(an, 0) + dd * num(an, 1));
  return { a: ca, b: sa, c, d: dd, tx, ty };
}
function mul(m1, m2) {
  return { a: m1.a*m2.a+m1.c*m2.b, b: m1.b*m2.a+m1.d*m2.b, c: m1.a*m2.c+m1.c*m2.d, d: m1.b*m2.c+m1.d*m2.d,
    tx: m1.a*m2.tx+m1.c*m2.ty+m1.tx, ty: m1.b*m2.tx+m1.d*m2.ty+m1.ty };
}
function inv(m) {
  const det = m.a*m.d-m.b*m.c;
  const ia=m.d/det, ib=-m.b/det, ic=-m.c/det, id=m.a/det;
  return { a:ia,b:ib,c:ic,d:id, tx:-(ia*m.tx+ic*m.ty), ty:-(ib*m.tx+id*m.ty) };
}
for (const ind of [2,3,4,5]) {
  const l = by[ind]; if(!l) continue;
  console.log(`ind${ind} ${l.nm} par=${l.parent}`, JSON.stringify(mat(l)));
}
// ind3 parent=4
const m2=mat(by[2]), m4=mat(by[4]), m3=mul(m4,mat(by[3]));
console.log('m3 world=', JSON.stringify(m3));
console.log('rel ind3<-mask=', JSON.stringify(mul(inv(m3),m2)));
console.log('rel ind4<-mask=', JSON.stringify(mul(inv(m4),m2)));
