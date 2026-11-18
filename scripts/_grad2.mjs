import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
let radial = [], linear = [];
(function w(o) {
  if (o && typeof o === 'object') {
    if (o.ty === 'gf') (o.t === 2 ? radial : linear).push(o);
    for (const k in o) w(o[k]);
  }
})(j);
const f = radial[0];
console.log('radial count', radial.length, 'linear', linear.length);
console.log('keys:', Object.keys(f).join(','));
console.log(JSON.stringify(f, null, 1).slice(0, 1000));
// 渐变起止点 s/e 是否存在
const withSE = radial.filter(r => r.s && r.e).length;
console.log('radial with s/e:', withSE, '/', radial.length);
const linSE = linear.filter(r => r.s && r.e).length;
console.log('linear with s/e:', linSE, '/', linear.length);
