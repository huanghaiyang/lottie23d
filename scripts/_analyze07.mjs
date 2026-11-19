import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));

const propAnim = (v) => {
  if (!v) return null;
  const k = v.k;
  if (Array.isArray(k)) return { kf: k.length, expr: v.x || null };
  if (v.x) return { kf: 0, expr: v.x };
  return { static: k };
};

const stat = {
  layers: 0, byType: {},
  kf: { p: 0, r: 0, s: 0, o: 0, a: 0 },
  exprLayers: [], exprShapes: [], exprCount: 0,
  masks: 0, matte: {}, shapeAnim: {}, shapeProps: {},
  colors: 0, gradients: 0, images: 0,
};

const exprSet = new Set();

function walkShapes(shapes, path) {
  for (const sh of shapes || []) {
    for (const prop of ['sw', 'sc', 'st', 'so', 'op', 'fc', 'fd', 'ty', 'hd']) {
      if (sh[prop]) {
        const a = propAnim(sh[prop]);
        if (a && a.kf) { stat.shapeAnim[prop] = (stat.shapeAnim[prop] || 0) + 1; stat.shapeProps[`${sh.ty}:${prop}`] = (stat.shapeProps[`${sh.ty}:${prop}`] || 0) + 1; }
        if (a && a.expr) { stat.exprShapes.push(`${path}/${sh.nm||sh.ty}.${prop}=${a.expr}`); stat.exprCount++; exprSet.add(a.expr); }
      }
    }
    if (sh.ty === 'gr') walkShapes(sh.it, path + '/' + (sh.nm || 'gr'));
    if (sh.ty === 'gf' || sh.ty === 'gf2' || sh.ty === 'gf3') stat.gradients++;
    if (sh.ty === 'fl' || sh.ty === 'fl2') stat.colors++;
    if (sh.ty === 'st' || sh.ty === 'st2') stat.colors++;
    if (sh.ty === 'rc' || sh.ty === 'rd' || sh.ty === 'sr' || sh.ty === 'sh' || sh.ty === 'tr') {}
  }
}

function walkLayers(L, where) {
  for (const l of L || []) {
    stat.layers++;
    stat.byType[l.ty] = (stat.byType[l.ty] || 0) + 1;
    const ks = l.ks || {};
    for (const k of ['p', 'r', 's', 'o', 'a']) {
      const a = propAnim(ks[k]);
      if (a && a.kf) stat.kf[k]++;
      if (a && a.expr) { stat.exprLayers.push(`${where}/${l.nm}.${k}=${a.expr}`); stat.exprCount++; exprSet.add(a.expr); }
    }
    if (l.masksProperties && l.masksProperties.length) stat.masks += l.masksProperties.length;
    if (l.tt) stat.matte[l.tt] = (stat.matte[l.tt] || 0) + 1;
    if (l.ty === 2) stat.images++;
    if (l.shapes) walkShapes(l.shapes, where + '/' + l.nm);
    if (l.ty === 0 && l.layers) walkLayers(l.layers, where + '/' + l.nm);
  }
}
walkLayers(j.layers, 'root');
for (const a of j.assets || []) walkLayers(a.layers, 'asset:' + a.id);

console.log('=== 层统计 ===');
console.log('总层', stat.layers, '类型', JSON.stringify(stat.byType));
console.log('关键帧层', JSON.stringify(stat.kf));
console.log('表达式数', stat.exprCount, '唯一', exprSet.size);
console.log('遮罩', stat.masks, '哑光tt', JSON.stringify(stat.matte));
console.log('形状动画属性', JSON.stringify(stat.shapeAnim));
console.log('形状prop明细', JSON.stringify(stat.shapeProps, null, 1));
console.log('纯色填充/描边', stat.colors, '渐变', stat.gradients, '图片层', stat.images);
console.log('\n=== 表达式（层）===');
stat.exprLayers.slice(0, 40).forEach(e => console.log(' ', e));
console.log('\n=== 表达式（形状）===');
stat.exprShapes.slice(0, 40).forEach(e => console.log(' ', e));
console.log('\n=== 唯一表达式 ===');
[...exprSet].forEach(e => console.log(' ', e));
