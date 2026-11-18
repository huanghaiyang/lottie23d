import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
function isAnim(v){ return v && typeof v==='object' && Array.isArray(v.k) && v.k.some(k=>k && k.t!==undefined); }

function scan(items, depth, out){
  for(const it of items||[]){
    const ty=it.ty||'?'; const nm=it.nm||'';
    const anim=[];
    for(const k of ['ks','p','s','r','sk','sa','pt','or','os','ir','is','w','c','o','h','v']){
      if(it[k] && isAnim(it[k])) anim.push(k);
    }
    const expr = typeof it.x==='string' && it.x.trim() ? it.x.slice(0,80) : '';
    out.push('  '.repeat(depth)+`${ty} ${nm} anim=[${anim.join(',')}]${expr?' EXPR='+expr:''}`);
    if(Array.isArray(it.it)) scan(it.it, depth+1, out);
  }
}
function walk(layers,where){
  for(const l of layers||[]){
    if(l.nm==='print 1'){
      const out=[]; scan(l.shapes,0,out);
      console.log('\n###',where,l.nm,'layer ks anim?', JSON.stringify({p:isAnim(l.ks?.p),s:isAnim(l.ks?.s),r:isAnim(l.ks?.r),o:isAnim(l.ks?.o)}));
      console.log(out.join('\n'));
    }
  }
}
walk(j.layers,'root');
(j.assets||[]).forEach(a=>walk(a.layers,'asset '+a.nm));
