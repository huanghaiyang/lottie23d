import { readFileSync } from 'node:fs';
const d = JSON.parse(readFileSync('data/shot_07.json', 'utf8'));
d.layers.forEach((l, i) => {
  if (l.tt != null && l.tt !== 0) {
    console.log('i' + i, 'ind=' + l.ind, l.nm, 'tt=' + l.tt, 'tp=' + (l.tp ?? '-'), 'ip=' + l.ip, 'op=' + l.op);
  }
});
