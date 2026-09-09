/* i18n 键完整性检查：五语言键集一致 + 前后端引用键都存在 */
'use strict';
const fs = require('fs');
const path = require('path');
const root = path.join(__dirname, '..');
const src = fs.readFileSync(path.join(root, 'ui', 'i18n.js'), 'utf8');
const m = src.match(/const DICT = (\{[\s\S]*?\n  \});/);
if (!m) { console.error('DICT not found'); process.exit(1); }
const dict = eval('(' + m[1] + ')');
const langs = Object.keys(dict);
const keys = Object.keys(dict[langs[0]]);
let bad = 0;
for (const l of langs) {
  const k = Object.keys(dict[l]);
  const missing = keys.filter((x) => !k.includes(x));
  const extra = k.filter((x) => !keys.includes(x));
  if (missing.length || extra.length) {
    bad++;
    console.log(l, 'missing:', JSON.stringify(missing), 'extra:', JSON.stringify(extra));
  }
}
console.log('langs:', langs.join(','), '| keys:', keys.length, bad ? 'BAD ' + bad : 'OK');
const js = fs.readFileSync(path.join(root, 'ui', 'app.js'), 'utf8');
const html = fs.readFileSync(path.join(root, 'ui', 'index.html'), 'utf8');
const used = new Set();
for (const x of (js + html).matchAll(/\bt\('([a-z0-9_]+)'/g)) used.add(x[1]);
for (const x of html.matchAll(/data-i18n(?:-placeholder)?="([a-z0-9_]+)"/g)) used.add(x[1]);
const missingRefs = [...used].filter((k) => !keys.includes(k));
if (missingRefs.length) {
  console.log('REFS TO MISSING KEYS:', JSON.stringify(missingRefs));
  process.exit(1);
}
console.log('all referenced keys exist. OK');
