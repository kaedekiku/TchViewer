// アプリアイコン icons/icon.ico を生成する（開発時のみ使用）: node tools/make-icon.js
// 角丸の四角（Twitch紫→5ch橙のグラデーション）に白い吹き出しと「T」。16/32/48/256px の32bit BMPを1つのICOにまとめる。
'use strict';
const fs = require('fs');
const path = require('path');

const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const A = hex('#9147ff'), B = hex('#e05a2b');

// 0..1 の座標で、その点の色 [r,g,b,a] を返す
function shade(x, y) {
  const r = 0.2; // 角丸
  const inside = (px, py, x0, y0, x1, y1, rr) => {
    const cx = Math.min(Math.max(px, x0 + rr), x1 - rr), cy = Math.min(Math.max(py, y0 + rr), y1 - rr);
    return (px - cx) ** 2 + (py - cy) ** 2 <= rr * rr && px >= x0 && px <= x1 && py >= y0 && py <= y1;
  };
  if (!inside(x, y, 0.02, 0.02, 0.98, 0.98, r)) return [0, 0, 0, 0];
  const t = (x + y) / 2;
  const bg = A.map((a, i) => Math.round(a + (B[i] - a) * t));
  // 吹き出し（白）とその尻尾
  const bubble = inside(x, y, 0.17, 0.2, 0.83, 0.68, 0.12);
  const tail = y >= 0.66 && y <= 0.84 && x >= 0.3 && x <= 0.3 + (0.84 - y) * 0.9 && x <= 0.46;
  if (!bubble && !tail) return [...bg, 255];
  // 吹き出しの中の「T」（背景色）
  const tBar = y >= 0.3 && y <= 0.39 && x >= 0.32 && x <= 0.68;
  const tStem = y >= 0.3 && y <= 0.6 && x >= 0.455 && x <= 0.545;
  if (bubble && (tBar || tStem)) return [...bg, 255];
  return [255, 255, 255, 255];
}

function bmp(size) {
  const ss = 4; // 4x4 スーパーサンプリングでなめらかに
  const px = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let acc = [0, 0, 0, 0];
      for (let sy = 0; sy < ss; sy++) for (let sx = 0; sx < ss; sx++) {
        const c = shade((x + (sx + 0.5) / ss) / size, (y + (sy + 0.5) / ss) / size);
        acc = [acc[0] + c[0] * c[3], acc[1] + c[1] * c[3], acc[2] + c[2] * c[3], acc[3] + c[3]];
      }
      const a = acc[3] / (ss * ss);
      const o = ((size - 1 - y) * size + x) * 4; // BMPは下の行から
      if (a > 0) {
        px[o] = Math.round(acc[2] / acc[3]); px[o + 1] = Math.round(acc[1] / acc[3]); px[o + 2] = Math.round(acc[0] / acc[3]);
      }
      px[o + 3] = Math.round(a);
    }
  }
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(size, 4);
  header.writeInt32LE(size * 2, 8); // XOR + AND マスクぶん
  header.writeUInt16LE(1, 12);
  header.writeUInt16LE(32, 14);
  header.writeUInt32LE(px.length, 20);
  const maskRow = Math.ceil(size / 32) * 4;
  return Buffer.concat([header, px, Buffer.alloc(maskRow * size)]);
}

const sizes = [16, 32, 48, 256];
const images = sizes.map(bmp);
const dir = Buffer.alloc(6 + 16 * sizes.length);
dir.writeUInt16LE(0, 0);
dir.writeUInt16LE(1, 2);
dir.writeUInt16LE(sizes.length, 4);
let offset = dir.length;
sizes.forEach((s, i) => {
  const e = 6 + 16 * i;
  dir.writeUInt8(s === 256 ? 0 : s, e);
  dir.writeUInt8(s === 256 ? 0 : s, e + 1);
  dir.writeUInt16LE(1, e + 4);
  dir.writeUInt16LE(32, e + 6);
  dir.writeUInt32LE(images[i].length, e + 8);
  dir.writeUInt32LE(offset, e + 12);
  offset += images[i].length;
});
const out = path.join(__dirname, '..', 'icons', 'icon.ico');
fs.writeFileSync(out, Buffer.concat([dir, ...images]));
console.log(`${out}: ${offset} bytes`);
