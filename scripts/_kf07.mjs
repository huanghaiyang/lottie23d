import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));

function kf(v) {
  const k = v && v.k;
  if (Array.isArray(k)) {
    return k.map(x => {
      const s = x.s;
      const val = Array.isArray(s) ? s.map(n => Number(n).toFixed(1)).join(',') : Number(s).toFixed(1);
      return `t=${x.t} s=[${val}]`;
    }).join(' | ');
  }
  return Array.isArray(k) ? k.map(n => Number(n).toFixed(1)).join(',') : Number(k).toFixed(1);
}

const dump = (L, where) => {
  for (const l of L || []) {
    const ks = l.ks || {};
    const line = [
      where,
      `ty=${l.ty}`,
      `nm=${l.nm}`,
      `ip=${l.ip} op=${l.op} st=${l.st ?? '-'} sr=${l.sr ?? '-'}`,
      `tt=${l.tt ?? 0}`,
      `p=${kf(ks.p)}`,
      `r=${kf(ks.r)}`,
      `s=${kf(ks.s)}`,
      `o=${kf(ks.o)}`,
      `a=${kf(ks.a)}`,
    ].join('  ');
    console.log(line);
    if (l.ty === 0 && l.layers) dump(l.layers, where + '/' + l.nm);
  }
};
dump(j.layers, 'root');
console.log('\n=== assets ===');
for (const a of j.assets || []) {
  console.log('asset', a.id, 'layers', (a.layers || []).length);
  dump(a.layers, '  ' + a.id);
}
