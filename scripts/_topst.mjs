import fs from 'fs';
const d = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
for (const l of d.layers) {
  if (/^Can \d/.test(l.nm)) {
    console.log(`ind=${l.ind} nm="${l.nm}" st=${l.st} sr=${l.sr} tm=${l.tm?JSON.stringify(l.tm.k):'-'} ip=${l.ip} op=${l.op} ref=${l.refId.slice(0,8)}`);
  }
}
