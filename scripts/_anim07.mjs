import fs from 'node:fs';
import { loadConverter, parseModel } from '../sdk/index.mjs';
const conv = await loadConverter();
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const m = parseModel(conv.convertBytes(JSON.stringify(j), {}));
console.log('frames', m.frameCount, 'meshes', m.meshes.length, 'transforms', m.transforms.length, 'frameRate', m.frameRate, 'ip', m.ip, 'op', m.op);

// 矩阵轨道逐帧方差（平移 x,y / 缩放 / 旋转）
const trackVar = new Array(m.transforms.length).fill(null).map(() => ({ move: 0, scale: 0, rot: 0, n: 0, samples: [] }));
for (const t of m.transforms) {
  const a = new Float32Array(t);
  if (!a.length) continue;
  const n = a.length / 16;
}
const stats = new Map();
m.meshes.forEach((mesh, i) => {
  const ti = mesh.transformIndex;
  if (!stats.has(ti)) stats.set(ti, { meshes: 0, anim: 0, color: 0, vert: 0, alpha: mesh.color ? mesh.color[3] : 1, kinds: {} });
  const s = stats.get(ti);
  s.meshes++; s.kinds[mesh.kind] = (s.kinds[mesh.kind] || 0) + 1;
  if (mesh.animated && mesh.framePositions) s.anim++;
  if (mesh.colors) s.color++;
  s.vert += mesh.vertexCount;
});

// 变换轨道变化量：取首末帧平移/缩放/旋转差
const trackInfo = m.transforms.map((t, idx) => {
  const a = new Float32Array(t);
  const n = a.length / 16;
  const f0 = a.slice(0, 16), f1 = a.slice((n - 1) * 16, n * 16);
  const dx = f1[12] - f0[12], dy = f1[13] - f0[13];
  const sx = Math.hypot(f1[0], f1[1]), sy = Math.hypot(f1[4], f1[5]);
  const sx0 = Math.hypot(f0[0], f0[1]), sy0 = Math.hypot(f0[4], f0[5]);
  const rot = Math.atan2(f1[1], f1[0]) - Math.atan2(f0[1], f0[0]);
  const maxMove = Math.abs(dx) + Math.abs(dy);
  const maxScale = Math.abs(sx - sx0) + Math.abs(sy - sy0);
  return { idx, n, maxMove, maxScale, rot: Math.abs(rot) };
});

let moving = 0, staticTrack = 0, animMesh = 0, colorMesh = 0, invisible = 0;
for (const t of trackInfo) {
  if (t.maxMove > 0.5 || t.maxScale > 0.01 || t.rot > 0.01) moving++; else staticTrack++;
}
for (const mesh of m.meshes) {
  if (mesh.animated && mesh.framePositions) animMesh++;
  if (mesh.colors) colorMesh++;
  if ((mesh.color ? mesh.color[3] : 1) < 0.004) invisible++;
}
console.log('变换轨道: 运动', moving, '静态', staticTrack);
console.log('网格: 顶点动画', animMesh, '颜色轨道', colorMesh, '静态不可见', invisible);

// 各层（transformIndex）汇总表
const rows = [...stats.entries()].map(([ti, s]) => {
  const t = trackInfo[ti] || { maxMove: 0, maxScale: 0, rot: 0 };
  return { ti, ...s, move: t.maxMove, scale: t.maxScale, rot: t.rot };
});
rows.sort((a, b) => Math.max(b.move, b.scale) - Math.max(a.move, a.scale));
console.log('\n=== 运动幅度最大的层（前 25）===');
for (const r of rows.slice(0, 25)) {
  console.log(`ti=${String(r.ti).padStart(3)} meshes=${String(r.meshes).padStart(3)} vert=${String(r.vert).padStart(6)} anim=${r.anim} color=${r.color} move=${r.move.toFixed(1)} scale=${r.scale.toFixed(3)} rot=${r.rot.toFixed(2)} alpha=${r.alpha.toFixed(2)} kinds=${JSON.stringify(r.kinds)}`);
}
console.log('\n=== 颜色轨道层 ===');
for (const r of rows.filter(r => r.color > 0).slice(0, 20)) {
  console.log(`ti=${String(r.ti).padStart(3)} meshes=${r.meshes} color=${r.color} alpha=${r.alpha.toFixed(2)}`);
}
