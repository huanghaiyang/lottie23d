import fs from 'node:fs';
const lines = fs.readFileSync('debug_clip.txt', 'utf8').split(/\r?\n/);
const re = /f=([\d.]+) owner=(.*?) (print \d+) src=(\S+) lf=([-\d.]+) vis=(\S+) op=([\d.]+) subj=(\d+) mask=(\d+)/;
const rows = [];
for (let i=0;i<lines.length;i++){
  const m = lines[i].match(re);
  if (!m) continue;
  let after = null;
  const nxt = lines[i+1] && lines[i+1].match(/after=(\d+)/);
  if (nxt) after = +nxt[1];
  rows.push({f:+m[1], owner:m[2], layer:m[3], src:m[4], lf:+m[5], vis:m[6]==='true', op:+m[7], subj:+m[8], mask:+m[9], after});
}
// visible content, masked source visible, but intersection empty
const bad = rows.filter(r => r.op > 0.05 && r.vis && r.after === 0);
const all = rows.filter(r => r.op > 0.05 && r.vis);
console.log('可见裁剪帧', all.length, '其中求交为空', bad.length);
const grp = {};
for (const r of bad) {
  const k = `${r.owner} / ${r.layer} / src=${r.src}`;
  (grp[k] ||= []).push(`f=${r.f} lf=${r.lf} subj=${r.subj} mask=${r.mask}`);
}
for (const [k,v] of Object.entries(grp).sort((a,b)=>b[1].length-a[1].length)) {
  console.log(`\n[${k}] x${v.length}`);
  console.log('  ' + v.slice(0,8).join('  '));
}
// also: visible but mask=0
const mask0 = all.filter(r => r.mask === 0);
console.log('\n可见帧 mask=0 计数', mask0.length);
