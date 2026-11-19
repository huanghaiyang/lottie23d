import fs from 'node:fs';
const lines = fs.readFileSync('debug_clip.txt', 'utf8').split(/\r?\n/);
const mainRe = /f=([\d.]+) owner=(.*?) (print \d+) src=(\S+) lf=([-\d.]+) vis=(\S+) op=([\d.]+) subj=(\d+) mask=(\d+)/;
const relRe = /rel tx=([-\d.]+) ty=([-\d.]+) sx=([-\d.]+) content tx=([-\d.]+) ty=([-\d.]+) matte tx=([-\d.]+) ty=([-\d.]+) parent=(\S+) parentTx=([-\d.]+),([-\d.]+)/;
let cur = null;
for (const ln of lines) {
  const m = ln.match(mainRe);
  if (m) { cur = {f:+m[1], owner:m[2], layer:m[3], src:m[4], lf:+m[5], vis:m[6]==='true', op:+m[7], subj:+m[8], mask:+m[9]}; continue; }
  const r = ln.match(relRe);
  if (r && cur) {
    Object.assign(cur, {relTx:+r[1], relTy:+r[2], cTx:+r[4], cTy:+r[5], mTx:+r[6], mTy:+r[7], parent:r[8], pTx:+r[9], pTy:+r[10]});
    if (cur.owner === 'Can 4' && cur.layer === 'print 5' && cur.op > 0.05) {
      console.log(`f=${cur.f} op=${cur.op} vis=${cur.vis} cTx=${cur.cTx} cTy=${cur.cTy} mTx=${cur.mTx} mTy=${cur.mTy} pTx=${cur.pTx} pTy=${cur.pTy} relTx=${cur.relTx} relTy=${cur.relTy}`);
    }
    cur = null;
  }
}
