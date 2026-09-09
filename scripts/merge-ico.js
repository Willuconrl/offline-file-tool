// 把多个单尺寸 ICO 合并为一个多尺寸 icon.ico（标准 ICO 容器格式）
const fs = require('fs');
const path = require('path');

const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');
const sizes = [16, 32, 48, 128, 256];
const parts = sizes.map((s) => ({
  size: s,
  data: fs.readFileSync(path.join(iconsDir, `part_${s}.ico`)),
}));

// 每个单尺寸 ICO：6 字节头 + 16 字节目录项 + 图像数据
for (const p of parts) {
  if (p.data.readUInt16LE(2) !== 1 || p.data.readUInt16LE(4) !== 1) {
    throw new Error(`part_${p.size}.ico 不是单条目 ICO`);
  }
  const entrySize = p.data.readUInt32LE(6 + 8);
  if (22 + entrySize !== p.data.length) {
    throw new Error(`part_${p.size}.ico 长度不符`);
  }
}

// 组装多尺寸 ICO
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2); // type: icon
header.writeUInt16LE(parts.length, 4); // count

let offset = 6 + 16 * parts.length;
const entries = [];
const blobs = [];
for (const p of parts) {
  const entry = Buffer.alloc(16);
  entry[0] = p.size >= 256 ? 0 : p.size; // width
  entry[1] = p.size >= 256 ? 0 : p.size; // height
  entry[2] = 0; // palette
  entry[3] = 0;
  entry.writeUInt16LE(1, 4); // planes
  entry.writeUInt16LE(32, 6); // bpp
  entry.writeUInt32LE(p.data.length - 22, 8); // data size
  entry.writeUInt32LE(offset, 12); // offset
  entries.push(entry);
  blobs.push(p.data.subarray(22));
  offset += p.data.length - 22;
}

const out = Buffer.concat([header, ...entries, ...blobs]);
fs.writeFileSync(path.join(iconsDir, 'icon.ico'), out);
console.log('icon.ico 已生成:', out.length, 'bytes,', parts.length, '个尺寸条目');
