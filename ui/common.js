// TchViewer アプリ画面の共通部品（メイン画面・画像ウィンドウで使う）
// 本体とのやり取りは api(cmd, args) だけ。アプリ内では Tauri の IPC、
// デバッグ版をブラウザで開いたとき（/ui/index.html）は /dev/api を使う。
// 外部から来た文字列（コメント・名前・ID・URL など）は必ず textContent で表示する。
'use strict';

const TAURI = window.__TAURI__;
const $ = (id) => document.getElementById(id);

async function api(cmd, args = {}) {
  if (TAURI) return TAURI.core.invoke('api', { cmd, args });
  const r = await fetch('/dev/api', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-TchViewer-Dev': '1' }, body: JSON.stringify({ cmd, args }) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

function onEvent(cb) {
  if (TAURI) TAURI.event.listen('tv', (e) => cb(JSON.parse(e.payload)));
  else { const es = new EventSource('/events?history=0&ui=1'); es.onmessage = (e) => cb(JSON.parse(e.data)); }
}

let toastTimer = 0;
// action: [ボタンの文字, 押したときの処理]（「取り消す」など。付けたときは少し長く出す）
function toast(text, action) {
  const t = $('toast');
  if (!t) return;
  t.replaceChildren(text);
  t.classList.toggle('act', !!action);
  if (action) {
    const b = document.createElement('button');
    b.className = 'mini';
    b.textContent = action[0];
    b.addEventListener('click', () => { t.classList.remove('show'); action[1](); });
    t.append(b);
  }
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), action ? 5000 : 2400);
}
function flashSaved(id) {
  const e = $(id);
  if (!e) return;
  e.textContent = '保存しました';
  clearTimeout(e._t);
  e._t = setTimeout(() => { e.textContent = ''; }, 1500);
}
async function safe(fn) {
  try { return await fn(); } catch (e) { toast(String(e.message || e)); throw e; }
}
function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = String(text);
  return e;
}
function button(text, cls, onclick, title) {
  const b = el('button', cls || 'mini', text);
  b.type = 'button';
  if (title) b.title = title;
  b.addEventListener('click', (e) => { e.stopPropagation(); onclick(e); });
  return b;
}

// ---- 外部からのドロップを拒否する（ファイルを落とすと画面が切り替わってしまうのを防ぐ）
window.addEventListener('dragover', (e) => { if (!e.dataTransfer || e.dataTransfer.types.includes('Files')) e.preventDefault(); });
window.addEventListener('drop', (e) => { if (!e.dataTransfer || e.dataTransfer.types.includes('Files')) e.preventDefault(); });

// ---- 右クリックメニュー（項目: [文字, 処理] / 'sep' / {head: 文字}）
function showMenu(x, y, items) {
  const m = $('menu');
  m.replaceChildren(...items.map((it) => {
    if (it === 'sep') return el('div', 'sep');
    if (it.head) return el('div', 'head', it.head);
    return button(it[0], '', () => { hideMenu(); it[1](); });
  }));
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  m.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
function hideMenu() { const m = $('menu'); if (m) m.hidden = true; }
window.addEventListener('click', hideMenu);
window.addEventListener('blur', hideMenu);
window.addEventListener('keydown', (e) => { if (e.key === 'Escape') hideMenu(); });

// ---- 読みにくい色の自動調整（背景との見やすさの比率が 4.5:1 未満の色だけ、色合いを保って明るさを変える）
function hexRgb(h) { return [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255); }
function luminance(rgb) {
  const [r, g, b] = rgb.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) { const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); }
function rgbHsl([r, g, b]) {
  const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min, s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h / 6, s, l];
}
function hslRgb([h, s, l]) {
  if (s === 0) return [l, l, l];
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s, p = 2 * l - q;
  const f = (t) => { t = (t + 1) % 1; return t < 1 / 6 ? p + (q - p) * 6 * t : t < 1 / 2 ? q : t < 2 / 3 ? p + (q - p) * (2 / 3 - t) * 6 : p; };
  return [f(h + 1 / 3), f(h), f(h - 1 / 3)];
}
function readableColor(hex, bgHex, min = 4.5) {
  if (!/^#[0-9a-f]{6}$/i.test(hex || '') || !/^#[0-9a-f]{6}$/i.test(bgHex || '')) return hex;
  const bg = hexRgb(bgHex);
  let rgb = hexRgb(hex);
  if (contrast(rgb, bg) >= min) return hex;
  const hsl = rgbHsl(rgb);
  const up = luminance(bg) < 0.4; // 暗い背景なら明るく、明るい背景なら暗くする
  for (let i = 0; i < 50 && contrast(rgb, bg) < min; i++) {
    hsl[2] = Math.min(1, Math.max(0, hsl[2] + (up ? 0.02 : -0.02)));
    rgb = hslRgb(hsl);
  }
  return '#' + rgb.map((c) => Math.round(c * 255).toString(16).padStart(2, '0')).join('');
}

function applyTheme(t) {
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
}
