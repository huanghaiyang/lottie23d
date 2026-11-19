import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const a = d.assets.find((x) => x.id.startsWith('3_'));
const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);
const g = (ks, k) => {
  const v = ks?.[k]?.k;
  if (Array.isArray(v) && v.length && typeof v[0] === 'object' && 's' in v[0]) return v[0].s;
  return v;
};
const by = {};
for (const l of a.layers) by[l.ind] = l;
// 标准 2D 仿射：M = T(p)·R·Shear·S·T(-a)，列主序 [a,b,c,d,tx,ty]
function mat(layer) {
  const ks = layer.ks;
  const p = g(ks, 'p'), an = g(ks, 'a'), s = g(ks, 's'), r = g(ks, 'r') ?? 0, sk = g(ks, 'sk') ?? 0;
  const sx = num(s, 0) / 100, sy = num(s, 1) / 100;
  const rad = (r * Math.PI) / 180;
  const ca = Math.cos(rad) * sx, sa = Math.sin(rad) * sx;
  const d = sy * Math.cos(rad), c = -sy * Math.sin(rad);
  // T(p) * R*S*T(-a)
  const tx = num(p, 0) - (ca * num(an, 0) + c * num(an, 1));
  const ty = num(p, 1) - (sa * num(an, 0) + d * num(an, 1));
  return { a: ca, b: sa, c, d, tx, ty, p: [num(p,0),num(p,1)], an: [num(an,0),num(an,1)] };
}
function mul(m1, m2) {
  return {
    a: m1.a * m2.a + m1.c * m2.b, b: m1.b * m2.a + m1.d * m2.b,
    c: m1.a * m2.c + m1.c * m2.d, d: m1.b * m2.c + m1.d * m2.d,
    tx: m1.a * m2.tx + m1.c * m2.ty + m1.tx, ty: m1.b * m2.tx + m1.d * m2.ty + m1.ty,
  };
}
function inv(m) {
  const det = m.a * m.d - m.b * m.c;
  const ia = m.d / det, ib = -m.b / det, ic = -m.c / det, id = m.a / det;
  const tx = -(ia * m.tx + ic * m.ty), ty = -(ib * m.tx + id * m.ty);
  return { a: ia, b: ib, c: ic, d: id, tx, ty };
}
for (const ind of [2, 3, 4]) {
  const l = by[ind];
  console.log(`ind${ind} ${l.nm}`, JSON.stringify(mat(l)));
}
const m2 = mat(by[2]), m4 = mat(by[4]), m3loc = mat(by[3]);
const m3 = mul(m4, m3loc);
console.log('m3 world=', JSON.stringify(m3));
const rel = mul(inv(m3), m2);
console.log('relative(ind3<-ind2)=', JSON.stringify(rel));
// mask 轮廓局部 bbox
const mask = by[2];
let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
for (const grp of mask.shapes ?? []) for (const it of grp.it ?? []) if (it.ty === 'sh') {
  const v = it.ks?.k?.v ?? [];
  for (const pt of v) { x0=Math.min(x0,pt[0]); x1=Math.max(x1,pt[0]); y0=Math.min(y0,pt[1]); y1=Math.max(y1,pt[1]); }
}
console.log('maskLocal bbox', [x0,y0,x1,y1]);
const map = (x,y) => [rel.a*x+rel.c*y+rel.tx, rel.b*x+rel.d*y+rel.ty];
console.log('mask mapped corners', map(x0,y0), map(x1,y1));
