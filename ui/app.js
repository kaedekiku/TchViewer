// TchViewer メイン画面（ビュワー・スレ・設定）。共通部品は common.js
'use strict';

const lines = (s) => s.split(/\r?\n/).map((x) => x.trim()).filter(Boolean);
let current = {};   // 本体の設定（画面に無い項目も保ったまま保存し直すため）
let info = {};
let stats = [];     // 取得元ごとの状況（2秒ごとに更新）

// ============================================================ ページ切り替え
for (const b of document.querySelectorAll('.side button')) b.addEventListener('click', () => showPage(b.dataset.page));
// ---- マスク（設定ページの登録スレ・NG・ID・パスなどを「＊＊＊」で隠す。オン・オフは保存して次回も同じにする。設定が読めるまではオン）
const MASK_PAGES = ['connect', 'events', 'whitelist', 'ng', 'speech', 'look', 'status', 'info'];
const isMasked = (name) => $(`page-${name}`)?.classList.contains('masked');
// 今見ているページがマスク中なら、お知らせなどに出す値も伏せる
const secret = (v) => (document.querySelector('.page.active.masked') ? '＊＊＊' : v);
function setMask(name, on, init, save) {
  const page = $(`page-${name}`);
  page.classList.toggle('masked', on);
  const b = page.querySelector('.maskbtn');
  b.textContent = on ? 'マスク中' : 'マスクする';
  b.title = on ? '押すと中身を表示します' : '押すと登録スレ・NG・ID などを＊＊＊で隠します';
  // マスク中に隠していたツールチップを戻す
  if (!on) for (const n of page.querySelectorAll('[data-mtitle]')) { n.title = n.dataset.mtitle; delete n.dataset.mtitle; }
  if (name === 'ng' && !init) renderNg();
  if (save) safe(() => api('save_mask', { page: name, on })).catch(() => {});
}
for (const name of MASK_PAGES) {
  const b = button('', 'mini maskbtn', () => setMask(name, !isMasked(name), false, true));
  $(`page-${name}`).querySelector('h1').append(b);
  setMask(name, true, true);
}
// マスク中のページでは、URL などが入ったツールチップを出さない
document.addEventListener('mouseover', (e) => {
  const page = e.target.closest?.('.page.masked');
  if (!page) return;
  for (let n = e.target; n && n !== page; n = n.parentElement) {
    if (n.title && !n.classList.contains('maskbtn')) { n.dataset.mtitle = n.title; n.removeAttribute('title'); }
  }
}, true);

function showPage(name) {
  for (const b of document.querySelectorAll('.side button')) b.classList.toggle('active', b.dataset.page === name);
  for (const p of document.querySelectorAll('.page')) p.classList.toggle('active', p.id === `page-${name}`);
  if (name === 'look' && !$('preview').src) updatePreview();
  if (name === 'whitelist') refreshWhitelist().catch(() => {});
  if (name === 'status' || name === 'connect') refreshStatus().catch(() => {});
  if (name === 'ng') renderNg();
  if (name === 'events') refreshAuth().catch(() => {});
}

// ============================================================ 「今読んでいるもの」の表示
let speakingKey = '';
function markSpeaking(key) {
  for (const e of document.querySelectorAll('.speaking')) e.classList.remove('speaking');
  speakingKey = key;
  for (const e of document.querySelectorAll(`[data-sk="${CSS.escape(key)}"]`)) e.classList.add('speaking');
}

// ============================================================ ビュワー
const VIEWER_MAX = 1000;
const vList = $('vList');
let lastViewerId = 0, viewerBoot = 0;
const filterOn = () => new Set([...document.querySelectorAll('#vFilter input:checked')].map((i) => i.value));
function viewerFilter() {
  const on = filterOn();
  for (const item of vList.children) item.hidden = !on.has(item.dataset.source);
  $('vEmpty').hidden = vList.children.length > 0;
}
$('vFilter').addEventListener('change', viewerFilter);
$('vClear').addEventListener('click', () => { vList.replaceChildren(); viewerFilter(); });
$('vTest').addEventListener('click', () => safe(() => api('test_comment')));
$('vPanel').addEventListener('change', () => safe(async () => {
  $('uPanel').hidden = !$('vPanel').checked;
  await api('save_ui', { unlistedPanel: $('vPanel').checked });
  current.unlistedPanel = $('vPanel').checked;
}));

function viewerBg() { return getComputedStyle(document.documentElement).getPropertyValue('--panel').trim(); }
function recolorViewerNames() {
  const bg = viewerBg();
  for (const n of document.querySelectorAll('.vitem .name[data-c]')) n.style.color = readableColor(n.dataset.c, bg);
}
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', recolorViewerNames);

const BADGE_RE = /^https:\/\/static-cdn\.jtvnw\.net\/badges\/v1\/[0-9a-f-]+$/i;
// ---- 本文中の URL（ビュワー・スレの最新レス・スレタブ共通）
// 「ttp://」「tp://」「ps://」「s://」のような書き方も http(s) として扱う（LiveFake と同じ）
const URL_RE = /(?<![A-Za-z])((?:h?ttps?|tps?|ps|s):\/\/[^\s<>"'「」『』（）【】、。，．　]+)/g;
const URL_TAIL_RE = /[、。，．！？!?）)」』】]+$/; // 文末の句読点・閉じかっこは URL に含めない
// 見えない文字（本体の text::is_invisible と同じ）を含む URL はリンクにしない
const INVISIBLE_RE = /[\u200B\u200C\u2060\uFEFF\u200E\u200F\u202A-\u202E\u2066-\u2069\u115F\u1160\u3164\uFFA0]/;
const IMAGE_HINT_RE = /\.(jpe?g|png|gif|webp|bmp)(\?|#|$)|^https:\/\/pbs\.twimg\.com\/media\//i;
// 開ける形の URL に整える。開けない形（長すぎる・見えない文字・ユーザー名付き・形式が不正）なら null。
// 本当の判定は本体側（links.rs）でもう一度行う
function normalizeUrl(raw) {
  const m = /^([a-z]+):\/\//i.exec(raw);
  if (!m || raw.length > 2048 || INVISIBLE_RE.test(raw)) return null;
  const scheme = m[1].toLowerCase().endsWith('s') ? 'https' : 'http';
  try {
    const u = new URL(`${scheme}://${raw.slice(m[0].length)}`);
    if (u.username || u.password) return null;
    return u.href; // 日本語ドメインは xn-- で始まる本来の形になる
  } catch { return null; }
}
// 文字列を、URL だけリンクにして box に足す。truncated: 末尾が切れた本文（切れ目にかかる URL はリンクにしない）
function appendLinked(box, text, truncated) {
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    let raw = m[1];
    const tail = raw.match(URL_TAIL_RE)?.[0] || '';
    raw = raw.slice(0, raw.length - tail.length);
    const end = m.index + raw.length;
    const url = (truncated && end >= text.length) ? null : normalizeUrl(raw);
    if (!url) continue;
    box.append(text.slice(last, m.index), linkNode(raw, url));
    last = end;
  }
  box.append(text.slice(last));
  return box;
}
function linkNode(raw, url) {
  const a = el('span', 'link', raw);
  a.dataset.url = url;
  a.title = `${url}\nクリックで開く・右クリックでコピー`;
  if (IMAGE_HINT_RE.test(url)) a.classList.add('img');
  return a;
}
// クリック: 画像は画像ウィンドウ、それ以外は既定のブラウザ（同じ URL は 0.5 秒に1回まで）
const lastOpen = new Map();
function openLink(url, forceBrowser) {
  const now = Date.now();
  if (now - (lastOpen.get(url) || 0) < 500) return;
  lastOpen.set(url, now);
  if (lastOpen.size > 200) lastOpen.clear();
  safe(async () => {
    const img = !forceBrowser && await api('image_check', { url });
    if (img) await api('open_image', { url });
    else await api('open_url', { url });
  }).catch(() => {});
}
document.addEventListener('click', (e) => {
  const link = e.target.closest?.('.link');
  if (!link) return;
  e.preventDefault();
  openLink(link.dataset.url);
});
// 右クリック: コピー・開く（スレタブの「選んだ文字を NG」より先に処理する）
document.addEventListener('contextmenu', (e) => {
  const link = e.target.closest?.('.link');
  if (!link) return;
  e.preventDefault();
  e.stopPropagation();
  const url = link.dataset.url;
  const items = [{ head: url.length > 60 ? `${url.slice(0, 60)}…` : url },
    ['URL をコピー', async () => { await navigator.clipboard.writeText(url); toast('URL をコピーしました'); }],
    ['ブラウザで開く', () => openLink(url, true)]];
  if (link.classList.contains('img')) items.push(['画像ウィンドウで開く', () => openLink(url)]);
  showMenu(e.clientX, e.clientY, items);
}, true);

function renderTwitchBody(m) {
  const box = el('div', 'text');
  if (m.source !== 'twitch' || !Array.isArray(m.emotes) || !m.emotes.length) return appendLinked(box, m.text || '');
  const cps = Array.from(m.text); // Twitchのエモート位置はコードポイント単位
  let pos = 0;
  m.emotes.forEach((e, i) => {
    if (!/^[\w-]+$/.test(e.id) || e.start < pos) return;
    const giant = m.giant && i === m.emotes.length - 1;
    appendLinked(box, cps.slice(pos, e.start).join(''));
    const img = el('img', giant ? 'emote giant' : 'emote');
    img.loading = 'lazy'; // 画面に入ったものだけ読み込む
    img.decoding = 'async';
    img.alt = cps.slice(e.start, e.end + 1).join('');
    img.src = `https://static-cdn.jtvnw.net/emoticons/v2/${encodeURIComponent(e.id)}/default/dark/${giant ? '3.0' : '2.0'}`;
    box.append(img);
    pos = e.end + 1;
  });
  appendLinked(box, cps.slice(pos).join(''));
  return box;
}

function viewerItem(m) {
  const item = el('div', `vitem ${m.source === '5ch' ? 's-5ch' : m.source}`);
  item.dataset.source = m.source;
  item.dataset.sk = `c:${m.id}`;
  const meta = el('div', 'meta');
  meta.append(el('span', '', m.time || ''), el('span', 'sitename', m.site));
  if (m.thread) meta.append(el('span', '', m.thread));
  if (m.no) meta.append(el('span', '', `${m.no}`));
  if (m.source === 'twitch' && Array.isArray(m.badgeList)) {
    const bs = el('span', 'badges');
    for (const b of m.badgeList) {
      if (typeof b.url !== 'string' || !BADGE_RE.test(b.url)) continue;
      const img = el('img'); img.loading = 'lazy'; img.decoding = 'async'; img.src = `${b.url}/2`; img.alt = img.title = String(b.title || ''); bs.append(img);
    }
    if (bs.children.length) meta.append(bs);
  }
  const name = el('span', 'name', m.name);
  if (m.source === 'twitch' && /^#[0-9a-f]{6}$/i.test(m.color || '')) { name.dataset.c = m.color; name.style.color = readableColor(m.color, viewerBg()); }
  meta.append(name);
  if (m.uid) meta.append(el('span', '', `ID:${m.uid}`));
  const ops = el('div', 'ops');
  ops.append(
    button('読む', 'mini', () => safe(() => api('speak_comment', { id: m.id })), 'もう一度読み上げる'),
    button('コピー', 'mini', async () => { await navigator.clipboard.writeText(m.text); toast('本文をコピーしました'); }),
  );
  if (m.event) {
    // イベント: 種類のラベル・要約（読み上げる文の {message} を除いたもの）・メッセージ
    item.classList.add('event', `ev-${m.event.kind}`);
    meta.prepend(el('span', 'evlabel', m.event.label));
    item.append(meta, ops, el('div', 'summary', m.event.summary));
    if (m.text) item.append(renderTwitchBody(m));
  } else {
    item.append(meta, ops, renderTwitchBody(m));
  }
  if (`c:${m.id}` === speakingKey) item.classList.add('speaking');
  return item;
}

// 大量のチャットが来ても画面の書き換えは1フレームに1回にまとめる
let pendingViewer = [];
function addViewer(m) {
  if (m.boot !== viewerBoot) { viewerBoot = m.boot; lastViewerId = 0; }
  if (m.id <= lastViewerId) return;
  lastViewerId = m.id;
  pendingViewer.push(m);
  if (pendingViewer.length === 1) requestAnimationFrame(flushViewer);
}
function flushViewer() {
  const batch = pendingViewer; pendingViewer = [];
  const atBottom = vList.scrollHeight - vList.scrollTop - vList.clientHeight < 40;
  const on = filterOn();
  const frag = document.createDocumentFragment();
  for (const m of batch) { const it = viewerItem(m); it.hidden = !on.has(m.source); frag.append(it); }
  vList.append(frag);
  while (vList.children.length > VIEWER_MAX) vList.firstElementChild.remove();
  $('vEmpty').hidden = true;
  if ($('vAuto').checked && atBottom) vList.scrollTop = vList.scrollHeight;
}

// ---- スレの最新レス（開いているスレタブに届いたレス ＋ 監視するスレで番号だけ読んだレス）
const U_MAX = 300;
const uRows = new Map(); // "key:no" → 行（同じレスを2回出さない）
function addLatest(m) {
  const id = `${m.key}:${m.no}`;
  const sk = m.speechKey || `u:${m.key}:${m.no}`;
  const old = uRows.get(id);
  if (old) {
    // スレタブ側で先に出ていたレスを番号だけ読んだ → 読み上げ中の印を合わせる
    old.dataset.sk = sk;
    if (sk === speakingKey) old.classList.add('speaking');
    return;
  }
  const list = $('uList');
  const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 30;
  const it = el('div', 'uitem');
  it.dataset.sk = sk;
  it.dataset.uid = m.uid || '';
  it._id = id;
  it._ng = { key: m.key, name: m.name || '', uid: m.uid || '', text: String(m.text || '') };
  const site = el('span', `site ${m.source === '5ch' ? 's-5ch' : m.source}`, m.site);
  site.title = m.thread || '';
  const full = String(m.text || '');
  const text = full.slice(0, 300);
  // 本体からの番号読みは 300 文字で切れて届く。切れ目にかかる URL はリンクにしない
  const snip = appendLinked(el('span', 'snip'), text, full.length > 300 || text.length >= 300);
  it.append(el('span', 'uid', m.time || ''), site, el('span', 'no', `${m.no}`), el('span', 'uid', m.uid ? `ID:${m.uid}` : ''), el('span', 'wlslot'), snip);
  it.title = `${m.thread ? `${m.thread}\n` : ''}${m.name}\n${text}`;
  const ops = el('div', 'ops');
  if (m.uid) ops.append(el('span', 'wlop'));
  ops.append(button('スレ', 'mini', () => openThread(m.url, m.no), 'スレタブで開く'));
  if (m.uid) ops.append(button('NG', 'mini', () => addNg('ids', m.uid, 'abone', 'global', ''), 'この ID を NG にする'));
  it.append(ops);
  setRowWl(it, !!m.wl);
  if (sk === speakingKey) it.classList.add('speaking');
  list.append(it);
  uRows.set(id, it);
  while (list.children.length > U_MAX) { const f = list.firstElementChild; uRows.delete(f._id); f.remove(); }
  if (atBottom) list.scrollTop = list.scrollHeight;
}
// 行のホワイトリストの表示（印と「入れる」/「外す」）
function setRowWl(it, on) {
  const uid = it.dataset.uid;
  if (!uid) return;
  it.classList.toggle('wl', on);
  it.querySelector('.wlslot').replaceChildren(...(on ? [el('span', 'wlmark', 'WL')] : []));
  it.querySelector('.wlop').replaceChildren(on
    ? button('外す', 'mini', () => removeWhitelist(uid), 'この ID をホワイトリストから外す')
    : button('入れる', 'mini', () => addWhitelist(uid), 'この ID をホワイトリストに入れる'));
}
// ホワイトリストが変わったら欄の印を付け直す
async function refreshLatestWl() {
  const rows = [...$('uList').children].filter((r) => r.dataset.uid);
  if (!rows.length) return;
  const hit = new Set(await api('wl_check', { ids: [...new Set(rows.map((r) => r.dataset.uid))] }));
  for (const r of rows) setRowWl(r, hit.has(r.dataset.uid));
}
// スレタブの新着（開いた直後の読み込みは除く。NG のレスは出さない）
function addLatestFromThread(m) {
  if (m.initial) return;
  for (const p of m.posts) {
    if (p.ng) continue;
    addLatest({ key: m.key, url: m.url, source: m.source, site: m.site, thread: m.title, no: p.no, name: p.name, uid: p.uid, text: p.text, time: m.time, wl: p.wl });
  }
}
$('uClear').addEventListener('click', () => { $('uList').replaceChildren(); uRows.clear(); });
// NG を変えたら、欄に出ているレスのうち NG に当たるもの（通常・透明あぼーん）を消す
async function refreshLatestNg() {
  const rows = [...$('uList').children].filter((r) => r._ng);
  if (!rows.length) return;
  const hits = await api('ng_filter', { items: rows.map((r) => r._ng) });
  rows.forEach((r, i) => { if (hits[i]) { uRows.delete(r._id); r.remove(); } });
}

async function addWhitelist(id) {
  await safe(() => api('whitelist_add', { id }));
  toast(`ID:${secret(id)} をホワイトリストに入れました`, ['取り消す', () => removeWhitelist(id)]);
}
async function removeWhitelist(id) {
  await safe(() => api('whitelist_remove', { id }));
  toast(`ID:${secret(id)} をホワイトリストから外しました`);
}

// ============================================================ スレ（スレタブ）
let tabs = [];             // [{key, url, site, source, title, monitored}]
let activeKey = '';
const threadCache = new Map(); // key → {url, title, posts, monitored}
let idFilter = '';         // ID で絞り込み中ならその ID

async function loadTabs(select) {
  tabs = await api('thread_tabs');
  if (select) activeKey = select;
  if (!tabs.some((t) => t.key === activeKey)) activeKey = tabs[0]?.key || '';
  renderTabs();
  if (activeKey) await activateTab(activeKey);
  else { $('tList').replaceChildren(); $('tHead').hidden = true; $('tEmpty').hidden = false; }
}

// タブの幅: 全部が1段に収まるなら最大220pxまで広げ、収まらなければ縮める。最小110pxでも収まらなければ、同じ幅のまま次の段へ折り返す
const TAB_MAX = 220, TAB_MIN = 110, TAB_GAP = 2;
function fitTabs() {
  const box = $('tTabs');
  const n = box.children.length;
  const w = box.clientWidth;
  if (!n || !w) return;
  let per = (w - TAB_GAP * (n - 1)) / n;
  if (per < TAB_MIN) {
    const cols = Math.max(1, Math.floor((w + TAB_GAP) / (TAB_MIN + TAB_GAP)));
    per = (w - TAB_GAP * (cols - 1)) / cols;
  }
  box.style.setProperty('--tabw', `${Math.floor(Math.min(TAB_MAX, per))}px`);
}
new ResizeObserver(fitTabs).observe($('tTabs'));

let dragTab = null;
function renderTabs() {
  $('tTabs').replaceChildren(...tabs.map((t) => {
    const tab = el('div', `tab ${t.source === '5ch' ? 's-5ch' : t.source}${t.key === activeKey ? ' active' : ''}`);
    tab.draggable = true;
    tab.title = `${t.title || t.key}\n${t.url}`;
    tab.append(el('span', `site ${t.source === '5ch' ? 's-5ch' : t.source}`, t.site));
    if (t.monitored) { const d = el('span', 'mon'); d.title = '監視中'; tab.append(d); }
    tab.append(el('span', 'ttl', t.title || t.key));
    tab.append(button('×', 'x', () => closeTab(t), 'タブを閉じる'));
    tab.addEventListener('click', () => activateTab(t.key));
    tab.addEventListener('dragstart', (e) => { dragTab = t.key; e.dataTransfer.setData('text/x-tchv-tab', t.key); e.dataTransfer.effectAllowed = 'move'; });
    tab.addEventListener('dragover', (e) => { if (dragTab) { e.preventDefault(); tab.classList.add('dragover'); } });
    tab.addEventListener('dragleave', () => tab.classList.remove('dragover'));
    tab.addEventListener('drop', (e) => {
      e.preventDefault(); tab.classList.remove('dragover');
      if (!dragTab || dragTab === t.key) return;
      const from = tabs.findIndex((x) => x.key === dragTab);
      const [moved] = tabs.splice(from, 1);
      tabs.splice(tabs.findIndex((x) => x.key === t.key), 0, moved);
      dragTab = null;
      renderTabs();
      safe(() => api('thread_reorder', { urls: tabs.map((x) => x.url) }));
    });
    tab.addEventListener('dragend', () => { dragTab = null; });
    return tab;
  }));
  fitTabs();
}

async function openThread(url, scrollNo) {
  const r = await safe(() => api('thread_open', { url }));
  showPage('threads');
  await loadTabs(r.key);
  if (scrollNo) setTimeout(() => scrollToPost(scrollNo), 300);
}
$('tUrlForm').addEventListener('submit', (e) => {
  e.preventDefault();
  const url = $('tUrl').value.trim();
  if (!url) return;
  openThread(url).then(() => { $('tUrl').value = ''; }).catch(() => {});
});

async function closeTab(t) {
  await safe(() => api('thread_close', { url: t.url }));
  threadCache.delete(t.key);
  if (activeKey === t.key) activeKey = '';
  await loadTabs();
}

async function activateTab(key) {
  activeKey = key;
  idFilter = '';
  renderTabs();
  const snap = await api('thread_get', { key });
  if (key !== activeKey) return;
  threadCache.clear();
  threadCache.set(key, snap);
  // 開いた直後はまだスレタイが分からないので、読み込めたらタブの見出しも更新する
  const tab = tabs.find((t) => t.key === key);
  if (tab && snap.title && !tab.title) { tab.title = snap.title; renderTabs(); }
  renderThread(true);
}

async function refreshActive() {
  if (!activeKey) return;
  const key = activeKey;
  const list = $('tList');
  const snap = await api('thread_get', { key });
  if (key !== activeKey) return;
  const old = threadCache.get(key);
  threadCache.set(key, snap);
  // 前の内容と並びが同じなら、NG・ホワイトリストの状態が変わったレスだけを描き直す
  const same = old && !idFilter && old.posts.length === snap.posts.length && old.posts.every((p, i) => p.no === snap.posts[i].no);
  if (!same) {
    const top = list.scrollTop;
    renderThread(false);
    list.scrollTop = top;
    return;
  }
  const t = threadTab();
  const counts = idCounts(snap.posts);
  const nodes = new Map();
  for (const n of list.children) if (n.dataset.no) nodes.set(n.dataset.no, n);
  snap.posts.forEach((p, i) => {
    const o = old.posts[i];
    if (o.ng === p.ng && o.wl === p.wl && o.ngReason === p.ngReason) return;
    const no = String(p.no);
    const cur = nodes.get(no);
    const node = postNode(p, counts, t); // 透明あぼーんなら null
    if (cur && node) cur.replaceWith(node);
    else if (cur) cur.remove();
    else if (node) {
      // 透明あぼーんを外したレスは、後ろのレスの前に入れ直す
      let next = null;
      for (let j = i + 1; j < snap.posts.length && !next; j++) next = nodes.get(String(snap.posts[j].no)) || null;
      list.insertBefore(node, next);
    }
    if (node) nodes.set(no, node); else nodes.delete(no);
  });
}

function threadTab() { return tabs.find((t) => t.key === activeKey); }

function renderThread(toBottom) {
  const t = threadTab();
  const snap = threadCache.get(activeKey);
  $('tEmpty').hidden = !!t;
  $('tHead').hidden = !t;
  if (!t || !snap) { $('tList').replaceChildren(); return; }
  $('tTitle').textContent = snap.title || t.title || t.key;
  $('tMonitor').checked = !!t.monitored;
  $('tFilter').hidden = !idFilter;
  $('tFilter').textContent = idFilter ? `ID:${idFilter} のレスだけ表示中 ×` : '';
  const counts = idCounts(snap.posts);
  const frag = document.createDocumentFragment();
  for (const p of snap.posts) {
    if (idFilter && p.uid !== idFilter) continue;
    const node = postNode(p, counts, t);
    if (node) frag.append(node);
  }
  $('tList').replaceChildren(frag);
  if (toBottom) $('tList').scrollTop = $('tList').scrollHeight;
}

function idCounts(posts) {
  const m = new Map();
  for (const p of posts) if (p.uid) { const a = m.get(p.uid) || []; a.push(p.no); m.set(p.uid, a); }
  return m;
}

// 本文: URL とアンカー（>>N）を、文字として安全に組み立てる
const ANCHOR_RE = /(>>|＞＞)(\d{1,4})(?:-(\d{1,4}))?/g;
function renderBody(text) {
  const box = el('div', 'pb');
  let last = 0;
  // アンカー（>>N）の間の文字は、URL をリンクにして足す
  for (const m of text.matchAll(ANCHOR_RE)) {
    appendLinked(box, text.slice(last, m.index));
    {
      const a = el('span', 'anchor', m[0]);
      a.dataset.from = m[2];
      a.dataset.to = m[3] || m[2];
      box.append(a);
    }
    last = m.index + m[0].length;
  }
  appendLinked(box, text.slice(last));
  return box;
}

function postNode(p, counts, t, compact) {
  if (p.ng === 'hide' && !compact) return null;
  const node = el('div', `post${p.wl ? ' wl' : ''}${p.ng ? ' abone' : ''}`);
  node.dataset.no = p.no;
  node.dataset.sk = `u:${activeKey}:${p.no}`;
  const head = el('div', 'ph');
  head.append(el('span', 'pno', p.no));
  if (p.uid) {
    const wb = p.wl
      ? button('WL済', 'mini wlbtn on', () => removeWhitelist(p.uid), 'ホワイトリストに入っています（押すと外す）')
      : button('WL', 'mini wlbtn', () => addWhitelist(p.uid), 'この ID をホワイトリストに入れる');
    head.append(wb);
  }
  const name = el('span', 'pname', p.name);
  name.addEventListener('click', (e) => { e.stopPropagation(); nameMenu(e, p, t); });
  head.append(name);
  if (p.mail) head.append(el('span', '', `[${p.mail}]`));
  head.append(el('span', '', p.date.replace(/\s*ID:\S+/, '')));
  if (p.uid) {
    const list = counts.get(p.uid) || [];
    const idx = list.indexOf(p.no) + 1;
    const id = el('span', `pid${list.length >= 5 ? ' many' : ''}`, `ID:${p.uid} (${idx}/${list.length})`);
    id.addEventListener('click', (e) => { e.stopPropagation(); idMenu(e, p, t); });
    head.append(id);
  }
  node.append(head);
  if (p.ng) node.append(el('div', 'pb', `あぼーん（${p.ngReason || 'NG'}）`));
  else node.append(renderBody(p.text));
  if (node.dataset.sk === speakingKey) node.classList.add('speaking');
  return node;
}

function scrollToPost(no) {
  const n = $('tList').querySelector(`.post[data-no="${no}"]`);
  if (n) { n.scrollIntoView({ block: 'center' }); n.classList.add('speaking'); setTimeout(() => n.classList.remove('speaking'), 1500); }
}

function nameMenu(e, p, t) {
  showMenu(e.clientX, e.clientY, [
    { head: `名前: ${p.name}` },
    ['名前を NG に追加（全体）', () => addNg('names', p.name, 'abone', 'global', '')],
    ['名前を NG に追加（このスレだけ）', () => addNg('names', p.name, 'abone', 'thread', t.url)],
  ]);
}
function idMenu(e, p, t) {
  const items = [{ head: `ID:${p.uid}` }];
  items.push([idFilter ? '絞り込みを解除' : 'この ID のレスだけ表示', () => { idFilter = idFilter ? '' : p.uid; renderThread(false); }]);
  items.push('sep');
  if (p.wl) items.push(['ホワイトリストから外す', () => removeWhitelist(p.uid)]);
  else items.push(['ホワイトリストに入れる', () => addWhitelist(p.uid)]);
  items.push('sep');
  items.push(['ID を NG に追加（全体）', () => addNg('ids', p.uid, 'abone', 'global', '')]);
  items.push(['ID を NG に追加（このスレだけ）', () => addNg('ids', p.uid, 'abone', 'thread', t.url)]);
  items.push(['ID をコピー', () => navigator.clipboard.writeText(p.uid)]);
  showMenu(e.clientX, e.clientY, items);
}

// リンクのクリック（画像は画像ウィンドウ、それ以外は既定のブラウザ）

// アンカーにマウスを乗せると、そのレスを表示
let popupTimer = 0;
$('tList').addEventListener('mouseover', (e) => {
  const a = e.target.closest('.anchor');
  if (!a) return;
  clearTimeout(popupTimer);
  const snap = threadCache.get(activeKey);
  if (!snap) return;
  const from = Number(a.dataset.from), to = Math.min(Number(a.dataset.to), from + 20);
  const counts = idCounts(snap.posts);
  const nodes = snap.posts.filter((p) => p.no >= from && p.no <= to).map((p) => postNode(p, counts, threadTab(), true)).filter(Boolean);
  if (!nodes.length) return;
  const pop = $('popup');
  pop.replaceChildren(...nodes);
  pop.hidden = false;
  const r = a.getBoundingClientRect();
  const pr = pop.getBoundingClientRect();
  pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - pr.width - 8))}px`;
  pop.style.top = `${r.bottom + pr.height + 8 < innerHeight ? r.bottom + 4 : Math.max(8, r.top - pr.height - 4)}px`;
});
$('tList').addEventListener('mouseout', (e) => {
  if (!e.target.closest('.anchor')) return;
  popupTimer = setTimeout(() => { $('popup').hidden = true; }, 250);
});
$('popup').addEventListener('mouseover', () => clearTimeout(popupTimer));
$('popup').addEventListener('mouseleave', () => { $('popup').hidden = true; });

// 本文の文字を選んで右クリック → NG ワードに追加
$('tList').addEventListener('contextmenu', (e) => {
  const sel = String(window.getSelection() || '').trim();
  const t = threadTab();
  if (!sel || !t) return;
  e.preventDefault();
  const short = sel.length > 20 ? `${sel.slice(0, 20)}…` : sel;
  showMenu(e.clientX, e.clientY, [
    { head: `「${short}」` },
    ['NG ワードに追加（全体）', () => addNg('words', sel, 'abone', 'global', '')],
    ['NG ワードに追加（このスレだけ）', () => addNg('words', sel, 'abone', 'thread', t.url)],
    ['NG ワードに追加（全体・透明あぼーん）', () => addNg('words', sel, 'hide', 'global', '')],
    'sep',
    ['コピー', () => navigator.clipboard.writeText(sel)],
  ]);
});

// 選んだ文字をドラッグ → 下に出る受け皿に落とすと NG ワードに追加
$('tList').addEventListener('dragstart', (e) => {
  const sel = String(window.getSelection() || '').trim();
  if (!sel) return;
  e.dataTransfer.setData('text/plain', sel);
  $('dropZone').hidden = false;
});
$('tList').addEventListener('dragend', () => { $('dropZone').hidden = true; });
for (const zone of $('dropZone').children) {
  zone.addEventListener('dragover', (e) => { e.preventDefault(); zone.classList.add('over'); });
  zone.addEventListener('dragleave', () => zone.classList.remove('over'));
  zone.addEventListener('drop', (e) => {
    e.preventDefault();
    zone.classList.remove('over');
    $('dropZone').hidden = true;
    const text = e.dataTransfer.getData('text/plain').trim();
    const t = threadTab();
    if (text && t) addNg('words', text, 'abone', zone.dataset.scope, zone.dataset.scope === 'thread' ? t.url : '');
  });
}

$('tMonitor').addEventListener('change', () => safe(async () => {
  const t = threadTab();
  if (!t) return;
  await api('thread_monitor', { url: t.url, on: $('tMonitor').checked });
  current.threads = (await api('get_config')).threads;
  toast($('tMonitor').checked ? '監視を始めました' : '監視をやめました');
  await loadTabs(activeKey);
  renderConnThreads();
}));
$('tBottom').addEventListener('click', () => { $('tList').scrollTop = $('tList').scrollHeight; });
$('tBrowser').addEventListener('click', () => { const t = threadTab(); if (t) safe(() => api('open_url', { url: t.url })); });
$('tFilter').addEventListener('click', () => { idFilter = ''; renderThread(false); });

// スレタブの自動更新（開いているタブすべてが対象）
async function saveTabPoll() {
  const sec = Math.min(300, Math.max(10, Math.round(Number($('tPoll').value)) || 15));
  $('tPoll').value = sec;
  await safe(() => api('save_tab_poll', { auto: $('tAuto').checked, seconds: sec }));
  current.tabAuto = $('tAuto').checked;
  current.tabPollSeconds = sec;
  $('tPoll').disabled = !$('tAuto').checked;
}
$('tAuto').addEventListener('change', () => { saveTabPoll().then(() => toast($('tAuto').checked ? `スレタブを ${$('tPoll').value} 秒ごとに更新します` : 'スレタブの自動更新をやめました')).catch(() => {}); });
$('tPoll').addEventListener('change', () => { saveTabPoll().catch(() => {}); });
$('tRefresh').addEventListener('click', () => safe(async () => {
  $('tRefresh').disabled = true;
  try { await api('thread_refresh'); } finally { $('tRefresh').disabled = false; }
}));

function onThreadUpdate(m) {
  const snap = threadCache.get(m.key);
  const tab = tabs.find((t) => t.key === m.key);
  if (tab && m.title && tab.title !== m.title) { tab.title = m.title; renderTabs(); }
  if (!snap) return;
  if (m.title) snap.title = m.title;
  const known = new Set(snap.posts.map((p) => p.no));
  const fresh = m.posts.filter((p) => !known.has(p.no));
  snap.posts.push(...fresh);
  if (snap.posts.length > 1100) snap.posts.splice(0, snap.posts.length - 1100);
  if (m.key !== activeKey || !fresh.length) return;
  if (idFilter) { renderThread(false); return; }
  const list = $('tList');
  const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 60;
  const counts = idCounts(snap.posts);
  const frag = document.createDocumentFragment();
  for (const p of fresh) { const n = postNode(p, counts, threadTab()); if (n) frag.append(n); }
  list.append(frag);
  $('tTitle').textContent = snap.title;
  if (atBottom) list.scrollTop = list.scrollHeight;
}

// ============================================================ NG
async function addNg(kind, value, mode, scope, scopeUrl) {
  value = String(value || '').trim();
  if (!value) return;
  // 正規表現（/…/）は 512 文字まで（それより長いものは本体側で使わないため、ここで止める）
  if (value.length > 2 && value.startsWith('/') && value.endsWith('/') && value.length > 512) { toast('NG の正規表現は512文字までです'); return; }
  const ng = structuredClone(current.ng || { words: [], names: [], ids: [] });
  if (ng[kind].some((e) => e.value === value && e.scope === scope && (scope !== 'thread' || e.scopeUrl === scopeUrl))) { toast('既に NG に入っています'); return; }
  ng[kind].push({ value, mode, scope, scopeUrl: scope === 'thread' ? scopeUrl : '' });
  await saveNg(ng);
  const label = { words: 'ワード', names: '名前', ids: 'ID' }[kind];
  toast(`NG${label}に追加しました: ${secret(value.length > 20 ? `${value.slice(0, 20)}…` : value)}`);
}
async function saveNg(ng) {
  await safe(() => api('save_ng', { ng }));
  current.ng = ng;
  renderNg();
}
function threadLabel(url) {
  const t = tabs.find((x) => x.url === url);
  return t?.title || titles.get(url) || statForThread(url)?.title || url;
}
function renderNg() {
  const ng = current.ng || { words: [], names: [], ids: [] };
  for (const [kind, id, countId] of [['words', 'ngWords', 'ngCountWords'], ['names', 'ngNames', 'ngCountNames'], ['ids', 'ngIds', 'ngCountIds']]) {
    $(countId).textContent = `${ng[kind].length}件`;
    $(id).replaceChildren(...ng[kind].map((e, i) => {
      const row = el('div', 'ngrow');
      row.append(el('span', 'val', e.value));
      const mode = el('select');
      for (const [v, t] of [['abone', '通常あぼーん'], ['hide', '透明あぼーん']]) { const o = el('option', '', t); o.value = v; mode.append(o); }
      mode.value = e.mode;
      mode.addEventListener('change', () => { const n = structuredClone(ng); n[kind][i].mode = mode.value; saveNg(n); });
      const scope = el('span', 'scope', e.scope === 'thread' ? `このスレだけ: ${threadLabel(e.scopeUrl)}` : '全体');
      scope.title = e.scopeUrl || '';
      row.append(mode, scope, button('削除', 'mini', () => { const n = structuredClone(ng); n[kind].splice(i, 1); saveNg(n); }));
      return row;
    }));
  }
  $('ngInvisible').value = ['abone', 'hide'].includes(ng.invisible) ? ng.invisible : 'off';
  // 追加欄の範囲: 全体 ＋ 監視中・開いているスレ
  const sel = $('ngScope');
  const keep = sel.value;
  const urls = [...new Set([...(current.threads || []), ...tabs.map((t) => t.url)])];
  const opts = [['global', '全体']].concat(urls.map((u, i) => [u, isMasked('ng') ? `このスレだけ: ＊＊＊（${i + 1}）` : `このスレだけ: ${threadLabel(u)}`]));
  sel.replaceChildren(...opts.map(([v, t]) => { const o = el('option', '', t); o.value = v; return o; }));
  if ([...sel.options].some((o) => o.value === keep)) sel.value = keep;
}
$('ngInvisible').addEventListener('change', () => {
  const ng = structuredClone(current.ng || { words: [], names: [], ids: [] });
  ng.invisible = $('ngInvisible').value;
  saveNg(ng).then(() => toast('見えない文字の NG を変更しました')).catch(() => {});
});
$('ngForm').addEventListener('submit', (e) => {
  e.preventDefault();
  const scope = $('ngScope').value;
  addNg($('ngKind').value, $('ngValue').value, $('ngMode').value, scope === 'global' ? 'global' : 'thread', scope === 'global' ? '' : scope)
    .then(() => { $('ngValue').value = ''; });
});

// ============================================================ 接続
const STATE_TEXT = { ok: '正常', error: 'エラー', connecting: '接続中' };
function statForChannel(ch) { return stats.find((s) => s.source === 'twitch' && s.label === `#${ch}`); }
function statForThread(url) {
  // 状態の識別子は「板/スレ番号」（したらばは「カテゴリ/番号/スレ番号」）
  return stats.find((s) => s.source !== 'twitch' && url.includes(`/${s.label}/`));
}
function renderConnTwitch() {
  $('cTwList').replaceChildren(...(current.twitchChannels || []).map((ch) => {
    const c = el('span', 'ch');
    const st = statForChannel(ch);
    const lamp = el('span', `lamp ${st?.state || 'none'}`);
    lamp.title = st ? `${STATE_TEXT[st.state] || st.state}${st.lastError ? `\n${st.lastError}` : ''}` : '';
    c.append(lamp, el('span', '', ch), button('×', '', () => saveTwitch((current.twitchChannels || []).filter((x) => x !== ch)), '外す'));
    return c;
  }));
  if (!(current.twitchChannels || []).length) $('cTwList').replaceChildren(el('div', 'hint', 'まだ登録されていません。'));
  $('cBadges').checked = current.twitchBadges !== false;
}
async function saveTwitch(channels) {
  await safe(() => api('save_twitch', { channels, badges: $('cBadges').checked }));
  current = await api('get_config');
  renderConnTwitch();
  setTimeout(() => refreshStatus().catch(() => {}), 1500);
}
$('cTwForm').addEventListener('submit', (e) => {
  e.preventDefault();
  const v = $('cTwInput').value.trim();
  if (!v) return;
  saveTwitch([...(current.twitchChannels || []), ...lines(v.replace(/[\s,、]+/g, '\n'))]).then(() => { $('cTwInput').value = ''; });
});
$('cBadges').addEventListener('change', () => saveTwitch(current.twitchChannels || []));

const titles = new Map(); // URL → スレタイ（確認時に取得したもの）
let dragRow = -1;
function renderConnThreads() {
  const list = current.threads || [];
  if (!list.length) { $('cThList').replaceChildren(el('div', 'hint', 'まだ登録されていません。上の欄にスレッドの URL を貼り付けてください。')); return; }
  $('cThList').replaceChildren(...list.map((url, i) => {
    const row = el('div', 'trow');
    const st = statForThread(url);
    const src = st?.source || (url.includes('shitaraba') ? 'shitaraba' : url.includes('jpnkn') ? 'jpnkn' : '5ch');
    const site = st?.site || { shitaraba: 'したらば', jpnkn: 'JPNKN', '5ch': '5ch' }[src];
    const handle = el('span', 'handle', '⋮⋮');
    handle.title = 'ドラッグで並べ替え';
    row.draggable = true;
    row.addEventListener('dragstart', (e) => { dragRow = i; e.dataTransfer.setData('text/x-tchv-row', String(i)); });
    row.addEventListener('dragover', (e) => { if (dragRow >= 0) { e.preventDefault(); row.classList.add('dragover'); } });
    row.addEventListener('dragleave', () => row.classList.remove('dragover'));
    row.addEventListener('drop', (e) => {
      e.preventDefault(); row.classList.remove('dragover');
      if (dragRow < 0 || dragRow === i) return;
      const next = [...list];
      const [moved] = next.splice(dragRow, 1);
      next.splice(i, 0, moved);
      dragRow = -1;
      saveThreads(next);
    });
    row.addEventListener('dragend', () => { dragRow = -1; });
    const inf = el('div', 'info');
    if (st?.title) titles.set(url, st.title);
    inf.append(el('div', 'ttl', st?.title || titles.get(url) || url));
    const sub = el('div', 'sub');
    sub.append(el('span', `site ${src === '5ch' ? 's-5ch' : src}`, site), el('span', `lamp ${st?.state || 'none'}`), el('span', '', st ? (STATE_TEXT[st.state] || st.state) : '未接続'));
    if (st?.resCount) {
      const bar = el('progress'); bar.max = 1000; bar.value = Math.min(st.resCount, 1000);
      sub.append(bar, el('span', '', `${st.resCount}/1000`));
    }
    if (st?.lastError) sub.append(el('span', 'off', st.lastError));
    inf.append(sub);
    row.append(handle, inf,
      button('タブで開く', 'mini', () => openThread(url)),
      button('ブラウザで開く', 'mini', () => safe(() => api('open_url', { url }))),
      button('削除', 'mini', () => saveThreads(list.filter((u) => u !== url))));
    return row;
  }));
}
async function saveThreads(list) {
  await safe(() => api('save_threads', { threads: list }));
  current = await api('get_config');
  renderConnThreads();
  await loadTabs(activeKey);
  setTimeout(() => refreshStatus().catch(() => {}), 1500);
}
$('cThForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const urls = $('cThInput').value.split(/\s+/).map((s) => s.trim()).filter(Boolean);
  if (!urls.length) return;
  const msg = $('cThMsg');
  msg.className = 'msg'; msg.textContent = '確認しています…';
  const ok = [], bad = [];
  for (const u of urls) {
    try {
      const r = await api('probe_thread', { url: u });
      if (r.title) titles.set(r.url, r.title);
      ok.push(r.url);
    } catch (err) { bad.push(`${u}: ${err.message || err}`); }
  }
  const list = [...(current.threads || [])];
  for (const u of ok) if (!list.includes(u)) list.push(u);
  if (ok.length) await saveThreads(list);
  msg.className = bad.length ? 'msg err' : 'msg';
  msg.textContent = bad.length ? `追加できませんでした — ${bad.join(' / ')}` : `${ok.length}件追加しました`;
  if (!bad.length) $('cThInput').value = '';
});
$('cThInput').addEventListener('paste', () => setTimeout(() => {
  // URL の貼り付けは、そのまま追加まで行う
  if (/^h?ttps?:\/\//.test($('cThInput').value.trim())) $('cThForm').requestSubmit();
}, 0));
for (const id of ['cPoll', 'cInitial']) {
  $(id).addEventListener('change', () => safe(async () => {
    await api('save_poll', { pollSeconds: Number($('cPoll').value) || 15, initialPosts: Number($('cInitial').value) || 0 });
    toast('保存しました');
  }));
}

// ============================================================ ホワイトリスト
async function refreshWhitelist() {
  const w = await api('whitelist_status');
  const lf = w.livefake;
  // TchViewer の一覧 ＋ LiveFake の一覧（読み取りだけ）
  const rows = w.own.map((e) => {
    const r = el('div', 'wlrow');
    r.append(el('span', 'id', e.id), el('span', `src ${e.source}`, e.source === 'auto' ? `自動（${e.reason}）` : '手動'), el('span', 'when', e.time),
      button('外す', 'mini', () => removeWhitelist(e.id)));
    return r;
  });
  for (const id of (lf.active ? lf.ids : [])) {
    if (w.own.some((e) => e.id === id)) continue;
    const r = el('div', 'wlrow');
    r.append(el('span', 'id', id), el('span', 'src livefake', 'LiveFake'), el('span', 'when', 'LiveFake 側で外してください'));
    rows.push(r);
  }
  $('wlOwn').replaceChildren(...(rows.length ? rows : [el('div', 'hint', 'まだありません。')]));
  $('wlRemoved').replaceChildren(...(w.removed.length ? w.removed.map((id) => {
    const r = el('div', 'wlrow');
    r.append(el('span', 'id', id), button('入れ直す', 'mini', () => addWhitelist(id)));
    return r;
  }) : [el('div', 'hint', 'ありません。')]));
  $('wlAutoCount').textContent = `今日の自動追加: ${w.autoCount}件`;
  $('wlPath').value = current.whitelistPath || '';
  $('wlState').textContent = lf.state || '-';
  $('wlDate').textContent = lf.date || '-';
  $('wlDate').className = lf.date && !lf.active ? 'off' : '';
  $('wlToday').textContent = lf.today;
  $('wlCount').textContent = lf.active ? lf.ids.length : 0;
  $('wlIds').replaceChildren(...(lf.active ? lf.ids : []).map((id) => el('span', '', id)));
}
$('wlAddForm').addEventListener('submit', (e) => {
  e.preventDefault();
  const id = $('wlAddInput').value.trim().replace(/^ID:/i, '');
  if (id) addWhitelist(id).then(() => { $('wlAddInput').value = ''; });
});
function fillWhitelistOpts() {
  const o = current.whitelist || {};
  $('wlAutoOn').checked = !!o.autoEnabled;
  $('wlAutoWords').value = (o.autoWords || []).join('\n');
  $('wlAutoLimit').value = o.autoLimit || 20;
}
let wlTimer = 0;
for (const id of ['wlAutoOn', 'wlAutoWords', 'wlAutoLimit']) {
  $(id).addEventListener('input', () => {
    clearTimeout(wlTimer);
    wlTimer = setTimeout(() => safe(async () => {
      const opts = { autoEnabled: $('wlAutoOn').checked, autoWords: lines($('wlAutoWords').value), autoLimit: Math.min(100, Math.max(1, Number($('wlAutoLimit').value) || 20)) };
      await api('save_whitelist_opts', { opts });
      current.whitelist = opts;
      flashSaved('wlOptsSaved');
    }), 400);
  });
}
async function pickWhitelist(mode) {
  const r = await safe(() => api('pick_whitelist', { mode }));
  if (r && r.path) { current.whitelistPath = r.path; toast('LiveFake の場所を設定しました'); setTimeout(() => refreshWhitelist().catch(() => {}), 1200); }
}
$('wlPickFolder').addEventListener('click', () => pickWhitelist('folder'));
$('wlPickFile').addEventListener('click', () => pickWhitelist('file'));
$('wlClear').addEventListener('click', () => safe(async () => {
  await api('save_whitelist_path', { path: '' });
  current.whitelistPath = '';
  toast('LiveFake 連携を解除しました');
  setTimeout(() => refreshWhitelist().catch(() => {}), 500);
}));

// ============================================================ 読み上げ
// data-s="sapi.rate" のようにパスで設定と結びつける（区切りは最初の "." だけ。sources.5ch のようなキーのため）
const splitS = (p) => { const i = p.indexOf('.'); return i < 0 ? [p] : [p.slice(0, i), p.slice(i + 1)]; };
function setOptions(sel, items, value) {
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
    // data-invert: 「棒読みちゃんの設定」のように、チェックが入っている＝値を使わない
    if (inp.type === 'checkbox') inp.checked = inp.dataset.invert ? !v : !!v; else inp.value = v ?? '';
  }
  $('speech').dataset.engine = sp.engine || 'bouyomi';
  $('vSpTw').checked = !!sp.twitchOn;
  $('vSpTh').checked = !!sp.threadOn;
  syncBouyomiParams();
}
// 棒読みちゃんの速度などの数値表示と、「棒読みちゃんの設定」のときはスライダーを使えなくする
function syncBouyomiParams() {
  for (const out of document.querySelectorAll('.byparams output')) {
    const range = document.querySelector(`[data-s="${out.dataset.for}"]`);
    const key = out.dataset.for.split('.')[1];
    const use = document.querySelector(`[data-s="bouyomi.set${key[0].toUpperCase()}${key.slice(1)}"]`);
    range.disabled = use.checked;
    out.textContent = use.checked ? '—' : range.value;
  }
}
function readSpeech() {
  const sp = structuredClone(current.speech || {});
  for (const inp of document.querySelectorAll('[data-s]')) {
    const ks = splitS(inp.dataset.s);
    const v = inp.type === 'checkbox' ? (inp.dataset.invert ? !inp.checked : inp.checked) : (inp.type === 'number' || inp.dataset.num) ? Number(inp.value) : inp.value.trim();
    if (ks.length === 1) sp[ks[0]] = v; else (sp[ks[0]] ??= {})[ks[1]] = v;
  }
  return sp;
}
// ビュワーの「Twitch読み上げ」「スレ読み上げ」は、「読み上げ」のページの同じボタンと同じ設定
// （オフにした種類の順番待ちは、本体が読む直前に飛ばす）
const SP_KINDS = [['vSpTw', 'twitchOn', 'Twitch'], ['vSpTh', 'threadOn', 'スレ']];
const spBox = (key) => document.querySelector(`#speech [data-s="${key}"]`);
for (const [id, key, label] of SP_KINDS) {
  $(id).addEventListener('change', () => {
    spBox(key).checked = $(id).checked;
    queueSpeechSave();
    toast(`${label}の読み上げを${$(id).checked ? 'オン' : 'オフ'}にしました`);
  });
}
let speechTimer = 0;
function queueSpeechSave() {
  $('speech').dataset.engine = $('spEngine').value;
  for (const [id, key] of SP_KINDS) $(id).checked = spBox(key).checked;
  clearTimeout(speechTimer);
  speechTimer = setTimeout(() => safe(async () => {
    const sp = readSpeech();
    await api('save_speech', { speech: sp });
    current.speech = sp;
    flashSaved('spSaved');
  }), 300);
}
$('speech').addEventListener('input', (e) => { if (e.target.dataset.s) { syncBouyomiParams(); queueSpeechSave(); } });
$('speech').addEventListener('change', (e) => {
  // 同じスタンプを読む数は 1〜100 の整数
  if (e.target.dataset.s === 'emoteMax') e.target.value = Math.min(100, Math.max(1, Math.round(Number(e.target.value)) || 3));
  if (e.target.dataset.s) queueSpeechSave();
});
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

// ============================================================ 表示（OBS）・レイアウト
const getPath = (o, p) => p.split('.').reduce((x, k) => (x == null ? x : x[k]), o);
const setPath = (o, p, v) => { const ks = p.split('.'); const last = ks.pop(); ks.reduce((x, k) => (x[k] ??= {}), o)[last] = v; };
function previewFonts() {
  for (const inp of document.querySelectorAll('input[list="fonts"]')) {
    inp.style.fontFamily = inp.value ? `"${inp.value.replace(/["\\]/g, '')}", sans-serif` : '';
  }
}
async function loadFonts() {
  const fonts = await api('fonts');
  $('fonts').replaceChildren(...fonts.map((f) => { const o = el('option', '', f.label); o.value = f.family; return o; }));
}
function fillLook(a) {
  for (const inp of document.querySelectorAll('[data-a]')) {
    const v = getPath(a, inp.dataset.a);
    if (inp.type === 'checkbox') inp.checked = !!v; else inp.value = v ?? '';
  }
  previewFonts();
}
let layoutName = '標準', layoutLook = {};
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
// 削除・既定に戻すは2回押しで確定（誤操作防止）
function twoStep(btn, label, confirmLabel, fn) {
  let armed = 0;
  btn.addEventListener('click', () => safe(async () => {
    if (Date.now() - armed > 3000) {
      armed = Date.now();
      btn.textContent = confirmLabel;
      setTimeout(() => { btn.textContent = label; }, 3000);
      return;
    }
    armed = 0;
    btn.textContent = label;
    await fn();
  }));
}
twoStep($('lyDelete'), '削除', 'もう一度押すと削除', async () => {
  const name = layoutName;
  await api('layout_delete', { name });
  await loadLayouts('標準');
  toast(`レイアウト「${name}」を削除しました`);
});
twoStep($('lookReset'), '表示を既定に戻す', 'もう一度押すと既定に戻す', async () => {
  await api('layout_save', { name: layoutName, appearance: {} });
  await openLayout(layoutName);
  toast('表示を既定に戻しました（追加CSSはそのままです）');
});
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
$('ovCopy').addEventListener('click', async () => { await navigator.clipboard.writeText($('ovUrl').value); toast('URLをコピーしました'); });

// ============================================================ Twitch イベント・ログイン
const AUTH_TEXT = { none: 'ログインしていません', waiting: 'ブラウザでの許可を待っています', connecting: '接続しています…', connected: 'ログイン中・受信中', error: 'エラー' };
let authStatus = {};
function showAuth(st) {
  authStatus = st || {};
  const who = st.loggedIn && st.login ? `（${st.login}）` : '';
  $('twAuthState').textContent = `${AUTH_TEXT[st.state] || st.state}${who}${st.message ? ` — ${st.message}` : ''}`;
  $('twCodeBox').hidden = st.state !== 'waiting' || !st.userCode;
  $('twCode').textContent = st.userCode || '';
  $('twLogout').disabled = !st.loggedIn && st.state !== 'waiting';
}
async function refreshAuth() { showAuth(await api('twitch_auth_status')); }
$('twLogin').addEventListener('click', () => safe(async () => {
  const st = await api('twitch_login');
  showAuth(st);
  if (st.verificationUri) await api('open_url', { url: st.verificationUri });
}));
$('twOpen').addEventListener('click', () => { if (authStatus.verificationUri) safe(() => api('open_url', { url: authStatus.verificationUri })); });
$('twLogout').addEventListener('click', () => safe(async () => { await api('twitch_logout'); await refreshAuth(); }));

let eventKinds = [];
function renderEvents() {
  const ev = current.twitchEvents || {};
  $('evTable').replaceChildren(...eventKinds.map((k) => {
    const s = ev[k.kind] || { show: k.show, speak: k.speak, template: k.template };
    const row = el('div', 'evrow');
    const show = el('label', 'tog'); const showIn = el('input'); showIn.type = 'checkbox'; showIn.checked = !!s.show; show.append(showIn, 'チャットに流す');
    const speak = el('label', 'tog'); const speakIn = el('input'); speakIn.type = 'checkbox'; speakIn.checked = !!s.speak; speak.append(speakIn, '読み上げる');
    const tpl = el('input'); tpl.value = s.template; tpl.placeholder = k.template;
    const save = () => saveEvents(k.kind, { show: showIn.checked, speak: speakIn.checked, template: tpl.value });
    showIn.addEventListener('change', save);
    speakIn.addEventListener('change', save);
    tpl.addEventListener('input', () => { clearTimeout(tpl._t); tpl._t = setTimeout(save, 400); });
    row.append(el('span', '', k.label), show, speak, tpl, button('テスト', 'mini', () => safe(() => api('test_event', { kind: k.kind })), 'この種類のテストを流す'));
    return row;
  }));
}
async function saveEvents(kind, setting) {
  const ev = structuredClone(current.twitchEvents || {});
  ev[kind] = setting;
  await safe(() => api('save_twitch_events', { events: ev }));
  current.twitchEvents = ev;
  flashSaved('evSaved');
}

// ============================================================ 状態
async function refreshStatus() {
  const r = await api('stats');
  stats = r.sources;
  $('statsBody').replaceChildren(...stats.map((s) => {
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
  if (!stats.length) {
    const tr = el('tr'); const td = el('td', 'sub-line', '接続先がありません。「接続」で登録してください。'); td.colSpan = 8; tr.append(td);
    $('statsBody').replaceChildren(tr);
  }
  const box = $('log');
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 30;
  box.textContent = r.log.join('\n');
  if (atBottom) box.scrollTop = box.scrollHeight;
  if ($('page-connect').classList.contains('active')) { renderConnTwitch(); renderConnThreads(); }
}

// ============================================================ 情報
$('uiTheme').addEventListener('change', () => safe(async () => {
  applyTheme($('uiTheme').value);
  recolorViewerNames();
  await api('set_theme', { theme: $('uiTheme').value });
}));
$('imgDirPick').addEventListener('click', () => safe(async () => {
  const r = await api('pick_image_dir');
  if (r.path) { $('imgDir').value = r.path; current.image.saveDir = r.path; flashSaved('imgSaved'); }
}));
$('imgMax').addEventListener('change', () => safe(async () => {
  const mb = Math.min(100, Math.max(1, Number($('imgMax').value) || 20));
  $('imgMax').value = mb;
  await api('save_image_opts', { maxMb: mb });
  flashSaved('imgSaved');
}));

// ============================================================ 本体からの通知
function onMessage(m) {
  switch (m.type) {
    case 'comment': addViewer(m); break;
    case 'unlisted': addLatest(m); break;
    case 'speaking': markSpeaking(m.key); break;
    case 'thread_update': onThreadUpdate(m); addLatestFromThread(m); break;
    case 'twitch_auth': showAuth(m.status); break;
    case 'ng_changed':
    case 'wl_changed':
      refreshActive().catch(() => {});
      if (m.type === 'wl_changed') refreshLatestWl().catch(() => {});
      if (m.type === 'ng_changed') refreshLatestNg().catch(() => {});
      if ($('page-whitelist').classList.contains('active')) refreshWhitelist().catch(() => {});
      break;
    default: break;
  }
}

// ============================================================ 起動
(async () => {
  info = await api('app_info');
  $('infoVer').textContent = info.version;
  $('infoData').textContent = info.dataDir;
  $('infoUrl').textContent = info.overlayUrl;
  $('imgRules').textContent = `${info.dataDir}\\ImageViewURLReplace.txt`;

  current = await api('get_config');
  $('uiTheme').value = current.uiTheme || 'system';
  applyTheme($('uiTheme').value);
  for (const name of MASK_PAGES) setMask(name, current.masks?.[name] !== false, true);
  $('vPanel').checked = current.unlistedPanel !== false;
  $('uPanel').hidden = !$('vPanel').checked;
  $('cPoll').value = current.pollSeconds;
  $('tAuto').checked = current.tabAuto !== false;
  $('tPoll').value = current.tabPollSeconds || 15;
  $('tPoll').disabled = !$('tAuto').checked;
  $('cInitial').value = current.initialPosts;
  $('imgDir').value = current.image?.saveDir || '';
  $('imgMax').value = current.image?.maxMb || 20;
  fillSpeech(current.speech);
  fillWhitelistOpts();
  eventKinds = await api('event_kinds');
  renderEvents();
  refreshAuth().catch(() => {});
  await loadLayouts('標準');
  loadFonts().catch(() => {});
  if (current.speech.engine === 'sapi') loadSapiVoices(true).catch(() => {});
  if (current.speech.engine === 'voicevox') loadVoicevoxSpeakers(true).catch(() => {});

  for (const d of await api('recent', { n: 300 })) addViewer(JSON.parse(d));
  onEvent(onMessage);
  await refreshStatus();
  renderConnTwitch();
  renderConnThreads();
  await loadTabs();
  renderNg();
  // 状態の更新は「状態」「接続」のページを見ているときだけ（取得・表示・読み上げは本体側で続く）
  setInterval(() => {
    if (document.hidden || !['status', 'connect'].some((p) => $(`page-${p}`).classList.contains('active'))) return;
    refreshStatus().catch(() => {});
  }, 2000);
  const startPage = new URLSearchParams(location.search).get('page');
  if (startPage && $(`page-${startPage}`)) showPage(startPage);
})().catch((e) => toast(`起動に失敗しました: ${e.message || e}`));
