// 扫描 release exe 中的个人信息痕迹（一次性审计脚本）
const fs = require('fs');
const path = 'src-tauri/target/release/file-toolbox.exe';
const buf = fs.readFileSync(path);
console.log('exe 大小:', (buf.length / 1024 / 1024).toFixed(2), 'MB');

const terms = ['D1637', 'Users', 'deepseekharness', 'project2', '\\file-toolbox', '.cargo', 'registry', 'index.crates.io', 'file_toolbox', 'C:\\Users'];

function ctx(off, span) {
  const s = Math.max(0, off - span / 2);
  const e = Math.min(buf.length, off + span / 2);
  return buf.subarray(s, e).toString('utf8').replace(/[^\x20-\x7e]/g, '·');
}

for (const t of terms) {
  const tb = Buffer.from(t, 'utf8');
  let count = 0;
  const ex = [];
  let idx = 0;
  while ((idx = buf.indexOf(tb, idx)) !== -1) {
    count++;
    if (ex.length < 2) ex.push('...' + ctx(idx, 90) + '...');
    idx += tb.length;
  }
  console.log('ASCII "' + t + '": ' + count + ' 处' + (ex.length ? '  例: ' + ex.join(' | ') : ''));
}

// UTF-16LE 编码的字符串（PE 资源里常用）
function scanUtf16(term) {
  const u = Buffer.from(term, 'utf16le');
  let count = 0;
  let idx = 0;
  while ((idx = buf.indexOf(u, idx)) !== -1) { count++; idx += u.length; }
  return count;
}
console.log('UTF-16 "D1637":', scanUtf16('D1637'), '处');
console.log('UTF-16 "Users":', scanUtf16('Users'), '处');
