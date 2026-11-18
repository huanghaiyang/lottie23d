import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
const a = (j.assets||[]).find(x=>x.nm==='Can 1');
const p1 = a.layers.find(l=>l.nm==='print 1');
function kf(v){ return v && Array.isArray(v.k) && v.k.some(k=>k&&k.o!==undefined); }
function col(fl){
  const c=fl.c; if(!c) return '?';
  const k=c.k;
  if(Array.isArray(k)) return k.slice(0,3).map(x=>+x.toFixed(2)).join(',');
  if(k&&Array.isArray(k.s)) return k.s[0].slice(0,3).map(x=>+x.toFixed(2)).join(',');
  return JSON.stringify(k);
}
p1.shapes.forEach((g,i)=>{
  if(g.ty!=='gr') return;
  const paths=(g.it||[]).filter(s=>['sh','rc','el'].includes(s.ty));
  const fills=(g.it||[]).filter(s=>['fl','gf'].includes(s.ty));
  const tr=(g.it||[]).find(s=>s.ty==='tr');
  const pathKf=paths.map(p=>kf(p.ks||p));
  const trAnim=tr&&(kf(tr.p)||kf(tr.a)||kf(tr.r)||kf(tr.s));
  const fill=fills[0];
  const fdesc=fill?(fill.ty==='gf'?'GRAD':('fill '+col(fill))):'none';
  console.log(`G${i+1} ${g.nm} trAnim=${!!trAnim} pathKf=${JSON.stringify(pathKf)} ${fdesc} paths=${paths.length}`);
});
