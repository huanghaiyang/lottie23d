// 独立参考实现：标准 Lottie 组变换 + 层变换 + parent 链，计算各层轮廓局部/世界 bbox。
// 仅用于与 Rust 探针输出对照，不参与产物。
import { readFileSync } from 'node:fs';

const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
const asset = d.assets.find((a) => a.id.startsWith('6_1'));

const num = (v, i = 0) => (Array.isArray(v) ? v[i] ?? 0 : v ?? 0);

// 组 tr：顶点 -> 组局部（a=0 时即 T(p)·S·R·v）
function groupApply(tr, v) {
  const p = tr?.p?.k ?? [0, 0];
  const a = tr?.a?.k ?? [0, 0];
  const s = tr?.s?.k ?? [100, 100];
  const r = ((tr?.r?.k ?? 0) * Math.PI) / 180;
  const sx = num(s, 0) / 100;
  const sy = num(s, 1) / 100;
  let x = v[0] * sx;
  let y = v[1] * sy;
  if (r) {
    const c = Math.cos(r);
    const si = Math.sin(r);
    const nx = x * c - y * si;
    y = x * si + y * c;
    x = nx;
  }
  return [x + num(p, 0) - num(a, 0), y + num(p, 1) - num(a, 1)];
}

// 层矩阵 T(p-a)·S·R，返回平移与缩放（无旋转本例）
function layerMatrix(ks, frame) {
  const evalProp = (key, fb) => {
    const v = ks?.[key]?.k;
    if (Array.isArray(v) && v.length && Array.isArray(v[0].s)) return v[0].s;
    if (Array.isArray(v)) return v;
    return fb;
  };
  const p = evalProp('p', [0, 0]);
  const a = evalProp('a', [0, 0]);
  const s = evalProp('s', [100, 100]);
  return { tx: num(p, 0) - num(a, 0), ty: num(p, 1) - num(a, 1), sx: num(s, 0) / 100, sy: num(s, 1) / 100 };
}

// 收集一层组变换后的局部顶点
function localPoints(layer) {
  const pts = [];
  for (const g of layer.shapes ?? []) {
    const tr = (g.it ?? []).find((i) => i.ty === 'tr');
    for (const it of g.it ?? []) {
      if (it.ty !== 'sh' || !it.ks?.k?.v) continue;
      for (const v of it.ks.k.v) pts.push(groupApply(tr, v));
    }
  }
  return pts;
}

const layers = asset.layers.map((l) => ({ layer: l, mat: layerMatrix(l.ks, 0) }));
const byInd = new Map(layers.map((x) => [x.layer.ind, x]));

function worldPts(x) {
  // parent 链：先局部后父级
  let m = { ...x.mat };
  let cur = x;
  const chain = [];
  while (cur.layer.parent != null) {
    const par = byInd.get(cur.layer.parent);
    if (!par) break;
    chain.unshift(par);
    cur = par;
  }
  // 父矩阵在左：world = parent·local。平移叠加（无旋转缩放）
  for (const par of chain) {
    m.tx += par.mat.tx;
    m.ty += par.mat.ty;
  }
  return m;
}

const bbox = (pts) => {
  if (!pts.length) return null;
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  return [Math.round(Math.min(...xs)), Math.round(Math.min(...ys)), Math.round(Math.max(...xs)), Math.round(Math.max(...ys))];
};

for (const x of layers) {
  const lp = localPoints(x.layer);
  const m = worldPts(x);
  const wp = lp.map((p) => [p[0] * m.sx + m.tx, p[1] * m.sy + m.ty]);
  console.log(
    `ind ${x.layer.ind} ${x.layer.nm} parent=${x.layer.parent ?? '-'}`,
    `layer(tx=${m.tx.toFixed(1)},ty=${m.ty.toFixed(1)})`,
    `local=${JSON.stringify(bbox(lp))}`,
    `world=${JSON.stringify(bbox(wp))}`,
  );
}
