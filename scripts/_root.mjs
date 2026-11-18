import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
console.log('root ip/op/fr/st:', j.ip, j.op, j.fr, j.st);
const byId = Object.fromEntries((j.assets || []).map(a => [a.id, a.nm]));
for (const l of j.layers) {
  if (l.ty === 0) {
    console.log(`ty0 ${(byId[l.refId]||'?').padEnd(14)} ind=${l.ind} parent=${l.parent??'-'} ip=${l.ip} op=${l.op} st=${l.st} sr=${l.sr} tm=${l.tm?JSON.stringify(l.tm.k):'-'}`);
  } else {
    console.log(`ty${l.ty} ${(l.nm||'').padEnd(20)} ind=${l.ind} parent=${l.parent??'-'} ip=${l.ip} op=${l.op} st=${l.st} sr=${l.sr}`);
  }
}
