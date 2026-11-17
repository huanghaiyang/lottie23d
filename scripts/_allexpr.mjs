import { readFileSync } from 'node:fs';
const files = process.argv.slice(2);
for (const f of files) {
  const j = JSON.parse(readFileSync(f, 'utf8'));
  const out = [];
  const walk = (o) => {
    if (Array.isArray(o)) return o.forEach(walk);
    if (!o || typeof o !== 'object') return;
    if (typeof o.x === 'string') out.push(o.x);
    Object.values(o).forEach(walk);
  };
  walk(j);
  console.log('=====', f, 'count', out.length);
  const seen = new Set();
  for (const e of out) {
    const key = e.slice(0, 60);
    if (seen.has(key)) continue;
    seen.add(key);
    console.log('---');
    console.log(e);
  }
}
