import fs from 'node:fs';
const dirs = ['data', 'public/animations'];
const files = [];
for (const d of dirs) for (const f of fs.readdirSync(d).filter(x => x.endsWith('.json'))) files.push(`${d}/${f}`);
for (const f of files) {
  let j;
  try { j = JSON.parse(fs.readFileSync(f, 'utf8')); } catch { continue; }
  const walk = (L, where) => {
    for (const l of L || []) {
      if (l.ty === 5) {
        const s = l.t?.d?.k?.[0]?.s ?? {};
        console.log(f, '|', where, '| ty5 |', l.nm, '| txt=', JSON.stringify(s.txt ?? ''), '| fill=', JSON.stringify(s.f ?? s.fc ?? '?'));
      }
      if (l.ty === 0 && l.layers) walk(l.layers, where + '/' + l.nm);
    }
  };
  walk(j.layers, 'root');
  for (const a of j.assets || []) walk(a.layers, 'asset:' + (a.nm || a.id));
}
