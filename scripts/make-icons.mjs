// 离线生成应用图标：纯 Node 实现的 PNG/ICO 编码，无需任何依赖
// 用法: node scripts/make-icons.mjs
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(__dirname, '..', 'src-tauri', 'icons');
mkdirSync(OUT_DIR, { recursive: true });

// ---------- PNG 编码 ----------
const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

function encodePNG(width, height, rgba) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0; // filter: None
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type: RGBA
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([
    sig,
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw, { level: 9 })),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

// ---------- 绘制 ----------
// 圆角矩形 SDF（含 1px 抗锯齿），返回值 >0 表示在图形内部（0~1 为边缘过渡）
function roundedRectSDF(x, y, x0, y0, x1, y1, r) {
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  const dx = x - cx;
  const dy = y - cy;
  const dist = Math.sqrt(dx * dx + dy * dy) - r;
  return 0.5 - dist;
}

// 圆环 SDF
function ringSDF(x, y, cx, cy, rOuter, rInner) {
  const d = Math.hypot(x - cx, y - cy);
  return Math.min(rOuter - d, d - rInner) + 0.5;
}

function drawIcon(size) {
  const rgba = Buffer.alloc(size * size * 4);
  const s = size / 512; // 相对 512 的缩放
  // 背景渐变（蓝 → 紫），圆角方形
  const bgR = 112 * s;
  const top = [56, 96, 245];
  const bottom = [96, 56, 240];
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const a = Math.max(0, Math.min(1, roundedRectSDF(x + 0.5, y + 0.5, s, s, size - s, size - s, bgR)));
      if (a <= 0) {
        rgba[i + 3] = 0;
        continue;
      }
      const t = y / size;
      const r = Math.round(top[0] + (bottom[0] - top[0]) * t);
      const g = Math.round(top[1] + (bottom[1] - top[1]) * t);
      const b = Math.round(top[2] + (bottom[2] - top[2]) * t);
      rgba[i] = r;
      rgba[i + 1] = g;
      rgba[i + 2] = b;
      rgba[i + 3] = Math.round(a * 255);
    }
  }
  // 白色工具箱图形
  const white = [255, 255, 255];
  const paint = (x, y, a, color) => {
    const i = (y * size + x) * 4;
    const sa = rgba[i + 3] / 255;
    const na = Math.max(0, Math.min(1, a));
    const oa = na * (1 - sa) + sa;
    if (oa <= 0) return;
    rgba[i] = Math.round((color[0] * na * (1 - sa) + rgba[i] * sa) / oa);
    rgba[i + 1] = Math.round((color[1] * na * (1 - sa) + rgba[i + 1] * sa) / oa);
    rgba[i + 2] = Math.round((color[2] * na * (1 - sa) + rgba[i + 2] * sa) / oa);
    rgba[i + 3] = Math.round(oa * 255);
  };
  // 提手（圆环）
  const hx = 256 * s, hy = 178 * s, rO = 84 * s, rI = 50 * s;
  // 箱体（圆角矩形）
  const bx0 = 116 * s, by0 = 170 * s, bx1 = 396 * s, by1 = 400 * s, br = 40 * s;
  // 锁扣（箱体中间的小横条，用背景色在白色上“挖出”）
  const lx0 = 210 * s, ly0 = 286 * s, lx1 = 302 * s, ly1 = 312 * s, lr = 13 * s;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const px = x + 0.5, py = y + 0.5;
      const ring = ringSDF(px, py, hx, hy, rO, rI);
      const body = roundedRectSDF(px, py, bx0, by0, bx1, by1, br);
      const latch = roundedRectSDF(px, py, lx0, ly0, lx1, ly1, lr);
      let a = Math.max(0, Math.min(1, Math.max(ring, body)));
      if (a > 0) paint(x, y, a, white);
      const la = Math.max(0, Math.min(1, latch));
      if (la > 0) {
        // 用背景渐变色重画锁扣区域
        const t = y / size;
        const color = [
          Math.round(top[0] + (bottom[0] - top[0]) * t),
          Math.round(top[1] + (bottom[1] - top[1]) * t),
          Math.round(top[2] + (bottom[2] - top[2]) * t),
        ];
        paint(x, y, la, color);
      }
    }
  }
  return rgba;
}

// ---------- 输出 ----------
function savePNG(size, file) {
  const png = encodePNG(size, size, drawIcon(size));
  writeFileSync(join(OUT_DIR, file), png);
  console.log(`已生成 ${file} (${size}x${size})`);
  return png;
}

function buildICO(png) {
  // ICO 容器内嵌 PNG（Vista+ 支持）
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // count
  const entry = Buffer.alloc(16);
  entry[0] = 0; // 256px
  entry[1] = 0;
  entry[2] = 0; // palette
  entry[3] = 0;
  entry.writeUInt16LE(1, 4); // planes
  entry.writeUInt16LE(32, 6); // bpp
  entry.writeUInt32LE(png.length, 8); // size
  entry.writeUInt32LE(22, 12); // offset
  return Buffer.concat([header, entry, png]);
}

const png512 = savePNG(512, 'icon.png');
const png256 = savePNG(256, '128x128@2x.png');
savePNG(128, '128x128.png');
savePNG(32, '32x32.png');
writeFileSync(join(OUT_DIR, 'icon.ico'), buildICO(png256));
console.log('已生成 icon.ico');
console.log('全部图标生成完毕。');
