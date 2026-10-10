// TchViewer アプリ画面（ビュワー・設定）
// 本体とのやり取りは api(cmd, args) だけ。アプリ内では Tauri の IPC、
// デバッグ版をブラウザで開いたとき（/ui/index.html）は /dev/api を使う。
// 外部から来た文字列（コメント・名前・ID など）は必ず textContent で表示する。
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
  else { const es = new EventSource('/events?history=0'); es.onmessage = (e) => cb(JSON.parse(e.data)); }
}

let toastTimer = 0;
function toast(text) {
  const t = $('toast');
  t.textContent = text;
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), 2200);
}
function flashSaved(id) {
  const el = $(id);
  if (!el) return;
  el.textContent = '保存しました';
  clearTimeout(el._t);
  el._t = setTimeout(() => { el.textContent = ''; }, 1500);
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
const lines = (s) => s.split('\n').map((x) => x.trim()).filter(Boolean);


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

// ビュワーの名前の色を、今の背景（テーマ）に対して読みやすく調整する
function viewerBg() { return getComputedStyle(document.documentElement).getPropertyValue('--panel').trim(); }
function recolorViewerNames() {
  const bg = viewerBg();
  for (const n of document.querySelectorAll('.vitem .name[data-c]')) n.style.color = readableColor(n.dataset.c, bg);
}
function applyTheme(t) {
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
  recolorViewerNames();
}
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', recolorViewerNames);
$('uiTheme').addEventListener('change', () => safe(async () => {
  applyTheme($('uiTheme').value);
  await api('set_theme', { theme: $('uiTheme').value });
}));

let current = {};   // 本体の設定（画面に無い項目も保ったまま保存し直すため）
let info = {};

// ============================================================ ページ切り替え
for (const b of document.querySelectorAll('.side button')) {
  b.addEventListener('click', () => showPage(b.dataset.page));
}
function showPage(name) {
  for (const b of document.querySelectorAll('.side button')) b.classList.toggle('active', b.dataset.page === name);
  for (const p of document.querySelectorAll('.page')) p.classList.toggle('active', p.id === `page-${name}`);
  if (name === 'look' && !$('preview').src) updatePreview();
  if (name === 'status' || name === 'livefake') refreshStatus();
}

// ============================================================ ビュワー
const VIEWER_MAX = 1000;
const vList = $('vList');
let lastViewerId = 0, viewerBoot = 0;

function viewerFilter() {
  const on = new Set([...document.querySelectorAll('#vFilter input:checked')].map((i) => i.value));
  const q = $('vSearch').value.trim().toLowerCase();
  for (const item of vList.children) {
    const show = on.has(item.dataset.source) && (!q || item.dataset.search.includes(q));
    item.hidden = !show;
  }
  $('vEmpty').hidden = vList.children.length > 0;
}
$('vFilter').addEventListener('change', viewerFilter);
$('vSearch').addEventListener('input', viewerFilter);
$('vClear').addEventListener('click', () => { vList.replaceChildren(); viewerFilter(); });
$('vTest').addEventListener('click', () => safe(() => api('test_comment')));

function renderBody(m) {
  const box = el('div', 'text');
  if (m.source !== 'twitch' || !Array.isArray(m.emotes) || !m.emotes.length) { box.textContent = m.text; return box; }
  const cps = Array.from(m.text); // Twitchのエモート位置はコードポイント単位
  let pos = 0;
  m.emotes.forEach((e, i) => {
    if (!/^[\w-]+$/.test(e.id) || e.start < pos) return;
    const giant = m.giant && i === m.emotes.length - 1;
    box.append(cps.slice(pos, e.start).join(''));
    const img = el('img', giant ? 'emote giant' : 'emote');
    img.alt = cps.slice(e.start, e.end + 1).join('');
    img.src = `https://static-cdn.jtvnw.net/emoticons/v2/${encodeURIComponent(e.id)}/default/dark/${giant ? '3.0' : '2.0'}`;
    box.append(img);
    pos = e.end + 1;
  });
  box.append(cps.slice(pos).join(''));
  return box;
}

function addViewer(m) {
  if (m.boot !== viewerBoot) { viewerBoot = m.boot; lastViewerId = 0; }
  if (m.id <= lastViewerId) return;
  lastViewerId = m.id;
  const atBottom = vList.scrollHeight - vList.scrollTop - vList.clientHeight < 40;
  const item = el('div', `vitem ${m.source === '5ch' ? 's-5ch' : m.source}`);
  item.dataset.source = m.source;
  item.dataset.search = `${m.name} ${m.uid} ${m.text}`.toLowerCase();
  const meta = el('div', 'meta');
  meta.append(el('span', '', m.time || ''), el('span', 'site', m.site));
  if (m.thread) meta.append(el('span', '', m.thread));
  if (m.no) meta.append(el('span', '', `${m.no}`));
  const name = el('span', 'name', m.name);
  if (m.source === 'twitch' && /^#[0-9a-f]{6}$/i.test(m.color || '')) { name.dataset.c = m.color; name.style.color = readableColor(m.color, viewerBg()); }
  if (m.source === 'twitch' && Array.isArray(m.badgeList)) {
    const bs = el('span', 'badges');
    for (const b of m.badgeList) {
      if (typeof b.url !== 'string' || !/^https:\/\/static-cdn\.jtvnw\.net\/badges\/v1\/[0-9a-f-]+$/i.test(b.url)) continue;
      const img = el('img'); img.src = `${b.url}/2`; img.alt = img.title = String(b.title || ''); bs.append(img);
    }
    if (bs.children.length) meta.append(bs);
  }
  meta.append(name);
  if (m.uid) meta.append(el('span', '', `ID:${m.uid}`));
  const ops = el('div', 'ops');
  const speak = el('button', '', '読む');
  speak.title = 'もう一度読み上げる';
  speak.addEventListener('click', () => safe(() => api('speak_comment', { id: m.id })));
  const copy = el('button', '', 'コピー');
  copy.addEventListener('click', async () => { await navigator.clipboard.writeText(m.text); toast('本文をコピーしました'); });
  ops.append(speak, copy);
  item.append(meta, ops, renderBody(m));
  vList.append(item);
  while (vList.children.length > VIEWER_MAX) vList.firstElementChild.remove();
  const on = new Set([...document.querySelectorAll('#vFilter input:checked')].map((i) => i.value));
  const q = $('vSearch').value.trim().toLowerCase();
  item.hidden = !(on.has(m.source) && (!q || item.dataset.search.includes(q)));
  $('vEmpty').hidden = true;
  if ($('vAuto').checked && atBottom) vList.scrollTop = vList.scrollHeight;
}

// ============================================================ 接続
function fillConnection(c) {
  $('cTwitch').value = c.twitchChannels.join('\n');
  $('cThreads').value = c.threads.join('\n');
  $('cPoll').value = c.pollSeconds;
  $('cInitial').value = c.initialPosts;
  $('cBadges').checked = c.twitchBadges;
}
$('cSave').addEventListener('click', () => safe(async () => {
  const args = {
    twitchChannels: lines($('cTwitch').value),
    threads: lines($('cThreads').value),
    pollSeconds: Number($('cPoll').value) || 15,
    initialPosts: Number($('cInitial').value) || 0,
    twitchBadges: $('cBadges').checked,
  };
  await api('save_connection', args);
  Object.assign(current, args);
  toast('保存して接続し直しました');
  setTimeout(refreshStatus, 1500);
}));

// ============================================================ LiveFake連携
async function refreshWhitelist() {
  const w = await api('whitelist_status');
  $('wlPath').value = current.whitelistPath || '';
  $('wlState').textContent = w.state || '-';
  $('wlDate').textContent = w.date || '-';
  $('wlDate').className = w.date && !w.active ? 'off' : '';
  $('wlToday').textContent = w.today;
  $('wlCount').textContent = w.active ? w.ids.length : 0;
  $('wlIds').replaceChildren(...(w.active ? w.ids : []).map((id) => el('span', '', id)));
}
async function pickWhitelist(mode) {
  const r = await safe(() => api('pick_whitelist', { mode }));
  if (r && r.path) { current.whitelistPath = r.path; toast('LiveFake連携の場所を設定しました'); setTimeout(refreshWhitelist, 1200); }
}
$('wlPickFolder').addEventListener('click', () => pickWhitelist('folder'));
$('wlPickFile').addEventListener('click', () => pickWhitelist('file'));
$('wlClear').addEventListener('click', () => safe(async () => {
  await api('save_whitelist_path', { path: '' });
  current.whitelistPath = '';
  toast('LiveFake連携を解除しました');
  setTimeout(refreshWhitelist, 500);
}));

// ============================================================ NGワード
let ngTimer = 0;
$('ngWords').addEventListener('input', () => {
  clearTimeout(ngTimer);
  ngTimer = setTimeout(() => safe(async () => {
    await api('save_ng', { ngWords: lines($('ngWords').value) });
    flashSaved('ngSaved');
  }), 500);
});

// ============================================================ 読み上げ
// data-s="sapi.rate" のようにパスで設定と結びつける（区切りは最初の "." だけ。sources.5ch のようなキーのため）
const splitS = (p) => { const i = p.indexOf('.'); return i < 0 ? [p] : [p.slice(0, i), p.slice(i + 1)]; };
function setOptions(sel, items, value) {
  // 一覧に無い値（保存済みの声など）も選べるよう残す
  const opts = items.map(([v, t]) => { const o = el('option', '', t); o.value = v; return o; });
  if (value !== undefined && value !== '' && !items.some(([v]) => String(v) === String(value))) {
    const o = el('option', '', `（保存済み）${value}`); o.value = value; opts.push(o);
  }
  sel.replaceChildren(...opts);
  if (value !== undefined) sel.value = String(value);
}
function fillSpeech(sp) {
  setOptions($('sapiVoice'), [['', 'Windows の既定の声']], sp.sapi?.voice ?? '');
  setOptions($('vvSpeaker'), [], sp.voicevox?.speaker ?? 3);
  for (const inp of document.querySelectorAll('[data-s]')) {
    const ks = splitS(inp.dataset.s);
    const v = ks.length === 1 ? sp[ks[0]] : sp[ks[0]]?.[ks[1]];
    if (inp.type === 'checkbox') inp.checked = !!v; else inp.value = v ?? '';
  }
  $('speech').dataset.engine = sp.engine || 'bouyomi';
}
function readSpeech() {
  const sp = structuredClone(current.speech || {});
  for (const inp of document.querySelectorAll('[data-s]')) {
    const ks = splitS(inp.dataset.s);
    const v = inp.type === 'checkbox' ? inp.checked : (inp.type === 'number' || inp.dataset.num) ? Number(inp.value) : inp.value.trim();
    if (ks.length === 1) sp[ks[0]] = v; else (sp[ks[0]] ??= {})[ks[1]] = v;
  }
  return sp;
}
let speechTimer = 0;
function queueSpeechSave() {
  $('speech').dataset.engine = $('spEngine').value;
  clearTimeout(speechTimer);
  speechTimer = setTimeout(() => safe(async () => {
    const sp = readSpeech();
    await api('save_speech', { speech: sp });
    current.speech = sp;
    flashSaved('spSaved');
  }), 300);
}
$('speech').addEventListener('input', (e) => { if (e.target.dataset.s) queueSpeechSave(); });
$('speech').addEventListener('change', (e) => { if (e.target.dataset.s) queueSpeechSave(); });
async function loadSapiVoices(quiet) {
  const { voices } = await api('sapi_voices');
  setOptions($('sapiVoice'), [['', 'Windows の既定の声'], ...voices.map((v) => [v.id, v.name])], readSpeech().sapi?.voice ?? '');
  if (!quiet) toast(`SAPIの声: ${voices.length}件`);
}
async function loadVoicevoxSpeakers(quiet) {
  const endpoint = document.querySelector('[data-s="voicevox.endpoint"]').value.trim();
  const { speakers } = await api('voicevox_speakers', { endpoint });
  setOptions($('vvSpeaker'), speakers.map((s) => [s.id, s.name]), readSpeech().voicevox?.speaker ?? 3);
  if (!quiet) toast(`VOICEVOXの話者: ${speakers.length}件`);
}
$('sapiReload').addEventListener('click', () => safe(() => loadSapiVoices(false)));
$('vvReload').addEventListener('click', () => safe(() => loadVoicevoxSpeakers(false)));
$('spTest').addEventListener('click', () => safe(() => api('speech_test')));
$('spStop').addEventListener('click', () => safe(() => api('speech_stop')));

// ============================================================ 表示（OBS）
const getPath = (o, p) => p.split('.').reduce((x, k) => (x == null ? x : x[k]), o);
const setPath = (o, p, v) => { const ks = p.split('.'); const last = ks.pop(); ks.reduce((x, k) => (x[k] ??= {}), o)[last] = v; };
function fillLook(a) {
  for (const inp of document.querySelectorAll('[data-a]')) {
    const v = getPath(a, inp.dataset.a);
    if (inp.type === 'checkbox') inp.checked = !!v; else inp.value = v ?? '';
  }
  previewFonts();
}
// フォント欄は、選んだフォントでその欄自体を表示して見本にする
function previewFonts() {
  for (const inp of document.querySelectorAll('input[list="fonts"]')) {
    inp.style.fontFamily = inp.value ? `"${inp.value.replace(/["\\]/g, '')}", sans-serif` : '';
  }
}
// インストール済みフォントの一覧（ユーザー単位でインストールしたものも含む）
async function loadFonts() {
  const fonts = await api('fonts');
  $('fonts').replaceChildren(...fonts.map((f) => { const o = el('option', '', f.label); o.value = f.family; return o; }));
}
function readLook() {
  const a = structuredClone(layoutLook || {});
  for (const inp of document.querySelectorAll('[data-a]')) {
    const v = inp.type === 'checkbox' ? inp.checked : inp.type === 'number' ? Number(inp.value) : inp.value.trim();
    setPath(a, inp.dataset.a, v);
  }
  return a;
}
async function saveLook(a) {
  await api('layout_save', { name: layoutName, appearance: a });
  layoutLook = a;
  flashSaved('lookSaved');
}

// ---- レイアウト
let layoutName = '標準', layoutLook = {};
const layoutUrl = (name) => (name === '標準' ? info.overlayUrl : `${info.overlayUrl}?layout=${encodeURIComponent(name)}`);
function updatePreview() {
  const u = layoutUrl(layoutName);
  $('preview').src = `${u}${u.includes('?') ? '&' : '?'}max=12&history=12`;
}
async function loadLayouts(select) {
  const names = await api('layouts');
  if (select && names.includes(select)) layoutName = select;
  if (!names.includes(layoutName)) layoutName = '標準';
  $('lySelect').replaceChildren(...names.map((n) => { const o = el('option', '', n); o.value = n; return o; }));
  $('lySelect').value = layoutName;
  await openLayout(layoutName);
}
async function openLayout(name) {
  layoutName = name;
  const r = await api('layout_get', { name });
  layoutLook = r.appearance;
  fillLook(layoutLook);
  $('lyCss').value = r.css;
  $('lyCssPath').textContent = `${info.dataDir}\\layouts\\${name}.css`;
  $('ovUrl').value = layoutUrl(name);
  $('lyRename').disabled = $('lyDelete').disabled = name === '標準';
  if ($('page-look').classList.contains('active')) updatePreview();
}
$('lySelect').addEventListener('change', () => safe(() => openLayout($('lySelect').value)));
$('lyNew').addEventListener('click', () => safe(async () => {
  const name = $('lyName').value.trim();
  await api('layout_create', { name, from: null });
  $('lyName').value = '';
  await loadLayouts(name);
  toast(`レイアウト「${name}」を作りました`);
}));
$('lyDup').addEventListener('click', () => safe(async () => {
  const name = $('lyName').value.trim();
  await api('layout_create', { name, from: layoutName });
  $('lyName').value = '';
  await loadLayouts(name);
  toast(`レイアウト「${name}」を複製して作りました`);
}));
$('lyRename').addEventListener('click', () => safe(async () => {
  const to = $('lyName').value.trim();
  const from = layoutName;
  await api('layout_rename', { from, to });
  $('lyName').value = '';
  await loadLayouts(to);
  toast(`「${from}」を「${to}」に変更しました。OBS のURLも変更してください`);
}));
// 削除は2回押しで確定（誤操作防止）
let deleteArmed = 0;
$('lyDelete').addEventListener('click', () => safe(async () => {
  if (Date.now() - deleteArmed > 3000) {
    deleteArmed = Date.now();
    $('lyDelete').textContent = 'もう一度押すと削除';
    setTimeout(() => { $('lyDelete').textContent = '削除'; }, 3000);
    return;
  }
  deleteArmed = 0;
  $('lyDelete').textContent = '削除';
  const name = layoutName;
  await api('layout_delete', { name });
  await loadLayouts('標準');
  toast(`レイアウト「${name}」を削除しました`);
}));
$('lyCssSave').addEventListener('click', () => safe(async () => {
  await api('layout_save_css', { name: layoutName, css: $('lyCss').value });
  flashSaved('lyCssSaved');
}));
let lookTimer = 0;
$('look').addEventListener('input', (e) => {
  if (!e.target.dataset.a) return;
  if (e.target.list) previewFonts();
  clearTimeout(lookTimer);
  lookTimer = setTimeout(() => safe(() => saveLook(readLook())), 300);
});
$('lookTest').addEventListener('click', () => safe(() => api('test_comment')));
// 既定に戻すのも2回押しで確定
let resetArmed = 0;
$('lookReset').addEventListener('click', () => safe(async () => {
  if (Date.now() - resetArmed > 3000) {
    resetArmed = Date.now();
    $('lookReset').textContent = 'もう一度押すと既定に戻す';
    setTimeout(() => { $('lookReset').textContent = '表示を既定に戻す'; }, 3000);
    return;
  }
  resetArmed = 0;
  $('lookReset').textContent = '表示を既定に戻す';
  await api('layout_save', { name: layoutName, appearance: {} });
  await openLayout(layoutName);
  toast('表示を既定に戻しました（追加CSSはそのままです）');
}));
$('ovCopy').addEventListener('click', async () => { await navigator.clipboard.writeText($('ovUrl').value); toast('URLをコピーしました'); });

// ============================================================ 状態・上部ランプ
const STATE_TEXT = { ok: '正常', error: 'エラー', connecting: '接続中' };
async function refreshStatus() {
  const { sources, log } = await api('stats');
  // 上部のランプ
  $('lamps').replaceChildren(...sources.map((s) => {
    const l = el('span', `lamp ${s.state}`, s.source === 'twitch' ? s.label : `${s.site} ${s.title || s.label}`);
    l.title = `${s.site} ${s.label}：${STATE_TEXT[s.state] || s.state}${s.lastError ? `\n${s.lastError}` : ''}`;
    return l;
  }));
  // 状態ページ
  $('statsBody').replaceChildren(...sources.map((s) => {
    const tr = el('tr');
    const src = el('td');
    src.append(el('div', '', `${s.site} ${s.label}`));
    if (s.title) src.append(el('div', 'sub-line', s.title));
    const st = el('td', `state-${s.state}`, STATE_TEXT[s.state] || s.state);
    if (s.lastError) st.append(el('div', 'sub-line', s.lastError));
    const board = s.source !== 'twitch';
    tr.append(src, st, el('td', '', s.lastFetch || '-'), el('td', 'num', board ? s.resCount : '-'), el('td', 'num', s.received), el('td', 'num', s.passed),
      el('td', 'num', board ? s.blockedWhitelist : '-'), el('td', 'num', board ? s.blockedNg : '-'));
    return tr;
  }));
  if (!sources.length) {
    const tr = el('tr'); const td = el('td', 'sub-line', '接続先がありません。「接続」で設定してください。'); td.colSpan = 8; tr.append(td);
    $('statsBody').replaceChildren(tr);
  }
  const box = $('log');
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 30;
  box.textContent = log.join('\n');
  if (atBottom) box.scrollTop = box.scrollHeight;
  if ($('page-livefake').classList.contains('active')) refreshWhitelist();
}

// ============================================================ 起動
(async () => {
  info = await api('app_info');
  $('ver').textContent = `v${info.version}`;
  $('infoVer').textContent = info.version;
  $('infoData').textContent = info.dataDir;
  $('infoUrl').textContent = info.overlayUrl;
  $('ovUrl').value = info.overlayUrl;

  current = await api('get_config');
  $('uiTheme').value = current.uiTheme || 'system';
  applyTheme($('uiTheme').value);
  fillConnection(current);
  $('ngWords').value = current.ngWords.join('\n');
  fillSpeech(current.speech);
  await loadLayouts('標準');
  loadFonts().catch(() => {});
  refreshWhitelist().catch(() => {});
  // 保存済みの声・話者の名前を出すため、一覧を静かに読み込んでおく
  if (current.speech.engine === 'sapi') loadSapiVoices(true).catch(() => {});
  if (current.speech.engine === 'voicevox') loadVoicevoxSpeakers(true).catch(() => {});

  for (const d of await api('recent', { n: 300 })) addViewer(JSON.parse(d));
  viewerFilter();
  onEvent((m) => {
    if (m.type === 'comment') addViewer(m);
  });
  refreshStatus();
  setInterval(() => refreshStatus().catch(() => {}), 2000);
})().catch((e) => toast(`起動に失敗しました: ${e.message || e}`));
