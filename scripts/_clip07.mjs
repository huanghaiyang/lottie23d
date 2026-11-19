import { readFileSync } from 'node:fs';
const lines = readFileSync('debug_clip.txt', 'utf8').split(/\r?\n/);
let rows = [];
let cur = null;
for (const ln of lines) {
  if (ln.includes('owner=')) {
    cur = { ln, after: null };
    rows.push(cur);
  } else if (ln.includes('after=') && cur) {
    cur.after = Number(ln.match(/after=(\d+)/)[1]);
  }
}
const bad = rows.filter(r => r.after === 0);
const keep = rows.filter(r => r.after > 0);
console.log('total', rows.length, 'after0', bad.length, 'keep', keep.length);
const grab = (ln, key) => (ln.match(new RegExp(key + '=([^ ]+)')) || [])[1];
console.log('\n=== after=0 samples ===');
for (const r of bad.slice(0, 8)) {
  console.log(`src=${grab(r.ln,'src')} subjB=${grab(r.ln,'subjB')} maskB=${grab(r.ln,'maskB')} maskLocal=${grab(r.ln,'maskLocal')} rel=${grab(r.ln,'rel')}`);
}
console.log('\n=== after>0 samples ===');
for (const r of keep.slice(0, 8)) {
  console.log(`after=${r.after} src=${grab(r.ln,'src')} subjB=${grab(r.ln,'subjB')} maskB=${grab(r.ln,'maskB')} maskLocal=${grab(r.ln,'maskLocal')}`);
}
