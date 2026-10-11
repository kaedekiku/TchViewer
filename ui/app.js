// TchViewer メイン画面（ビュワー・スレ・設定）。共通部品は common.js
'use strict';

const lines = (s) => s.split(/\r?\n/).map((x) => x.trim()).filter(Boolean);
let current = {};   // 本体の設定（画面に無い項目も保ったまま保存し直すため）
let info = {};
let stats = [];     // 取得元ごとの状況（2秒ごとに更新）

// ============================================================ ページ切り替え
for (const b of document.querySelectorAll('.side button')) b.addEventListener('click', () => showPage(b.dataset.page));
function showPage(name) {
  for (const b of document.querySelectorAll('.side button')) b.classList.toggle('active', b.dataset.page === name);
  for (const p of document.querySelectorAll('.page')) p.classList.toggle('active', p.id === `page-${name}`);
  if (name === 'look' && !$('preview').src) updatePreview();
  if (name === 'whitelist') refreshWhitelist().catch(() => {});
  if (name === 'status' || name === 'connect') refreshStatus().catch(() => {});
  if (name === 'ng') renderNg();
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
function renderTwitchBody(m) {
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
      const img = el('img'); img.src = `${b.url}/2`; img.alt = img.title = String(b.title || ''); bs.append(img);
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
  item.append(meta, ops, renderTwitchBody(m));
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

// ---- 番号だけ読んだレス
const U_MAX = 300;
function addUnlisted(m) {
  const list = $('uList');
  const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 30;
  const it = el('div', 'uitem');
  it.dataset.sk = m.speechKey;
  const site = el('span', `site ${m.source === '5ch' ? 's-5ch' : m.source}`, m.site);
  site.title = m.thread || '';
  it.append(el('span', 'uid', m.time), site, el('span', 'no', `${m.no}`), el('span', 'uid', m.uid ? `ID:${m.uid}` : ''), el('span', 'snip', m.text));
  it.title = `${m.name}\n${m.text}`;
  const ops = el('div', 'ops');
  if (m.uid) ops.append(button('入れる', 'mini', () => addWhitelist(m.uid), 'この ID をホワイトリストに入れる'));
  ops.append(button('スレ', 'mini', () => openThread(m.url, m.no), 'スレタブで開く'));
  if (m.uid) ops.append(button('NG', 'mini', () => addNg('ids', m.uid, 'abone', 'global', ''), 'この ID を NG にする'));
  it.append(ops);
  if (m.speechKey === speakingKey) it.classList.add('speaking');
  list.append(it);
  while (list.children.length > U_MAX) list.firstElementChild.remove();
  if (atBottom) list.scrollTop = list.scrollHeight;
}
$('uClear').addEventListener('click', () => $('uList').replaceChildren());

async function addWhitelist(id) {
  await safe(() => api('whitelist_add', { id }));
  toast(`ID:${id} をホワイトリストに入れました`);
}
async function removeWhitelist(id) {
  await safe(() => api('whitelist_remove', { id }));
  toast(`ID:${id} をホワイトリストから外しました`);
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
  threadCache.set(key, snap);
  // 開いた直後はまだスレタイが分からないので、読み込めたらタブの見出しも更新する
  const tab = tabs.find((t) => t.key === key);
  if (tab && snap.title && !tab.title) { tab.title = snap.title; renderTabs(); }
  renderThread(true);
}

async function refreshActive() {
  if (!activeKey) return;
  const list = $('tList');
  const top = list.scrollTop;
  const snap = await api('thread_get', { key: activeKey });
  threadCache.set(activeKey, snap);
  renderThread(false);
  list.scrollTop = top;
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
const TOKEN_RE = /(h?ttps?:\/\/[^\s<>"'「」]+)|(>>|＞＞)(\d{1,4})(?:-(\d{1,4}))?/g;
const IMAGE_HINT_RE = /\.(jpe?g|png|gif|webp|bmp)(\?|#|$)|^https:\/\/pbs\.twimg\.com\/media\//i;
function renderBody(text) {
  const box = el('div', 'pb');
  let last = 0;
  for (const m of text.matchAll(TOKEN_RE)) {
    box.append(text.slice(last, m.index));
    if (m[1]) {
      // 「ttp://」「tp://」のような書き方も http として扱う
      const fixed = m[1].startsWith('http') ? m[1] : `h${m[1]}`;
      const a = el('span', 'link', m[1]);
      a.dataset.url = fixed;
      a.title = fixed;
      if (IMAGE_HINT_RE.test(fixed)) a.classList.add('img');
      box.append(a);
    } else {
      const a = el('span', 'anchor', m[0]);
      a.dataset.from = m[3];
      a.dataset.to = m[4] || m[3];
      box.append(a);
    }
    last = m.index + m[0].length;
  }
  box.append(text.slice(last));
  return box;
}

function postNode(p, counts, t, compact) {
  if (p.ng === 'hide' && !compact) return null;
  const node = el('div', `post${p.wl ? ' wl' : ''}${p.ng ? ' abone' : ''}`);
  node.dataset.no = p.no;
  node.dataset.sk = `u:${activeKey}:${p.no}`;
  const head = el('div', 'ph');
  head.append(el('span', 'pno', p.no));
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
  if (p.wl) head.append(el('span', 'wlmark', 'ホワイトリスト'));
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
$('tList').addEventListener('click', (e) => {
  const link = e.target.closest('.link');
  if (!link) return;
  safe(async () => {
    const img = await api('image_check', { url: link.dataset.url });
    if (img) await api('open_image', { url: link.dataset.url });
    else await api('open_url', { url: link.dataset.url });
  });
});

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
  const ng = structuredClone(current.ng || { words: [], names: [], ids: [] });
  if (ng[kind].some((e) => e.value === value && e.scope === scope && (scope !== 'thread' || e.scopeUrl === scopeUrl))) { toast('既に NG に入っています'); return; }
  ng[kind].push({ value, mode, scope, scopeUrl: scope === 'thread' ? scopeUrl : '' });
  await saveNg(ng);
  const label = { words: 'ワード', names: '名前', ids: 'ID' }[kind];
  toast(`NG${label}に追加しました: ${value.length > 20 ? `${value.slice(0, 20)}…` : value}`);
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
  // 追加欄の範囲: 全体 ＋ 監視中・開いているスレ
  const sel = $('ngScope');
  const keep = sel.value;
  const urls = [...new Set([...(current.threads || []), ...tabs.map((t) => t.url)])];
  const opts = [['global', '全体']].concat(urls.map((u) => [u, `このスレだけ: ${threadLabel(u)}`]));
  sel.replaceChildren(...opts.map(([v, t]) => { const o = el('option', '', t); o.value = v; return o; }));
  if ([...sel.options].some((o) => o.value === keep)) sel.value = keep;
}
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
    case 'unlisted': addUnlisted(m); break;
    case 'speaking': markSpeaking(m.key); break;
    case 'thread_update': onThreadUpdate(m); break;
    case 'ng_changed':
    case 'wl_changed':
      refreshActive().catch(() => {});
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
  $('vPanel').checked = current.unlistedPanel !== false;
  $('uPanel').hidden = !$('vPanel').checked;
  $('cPoll').value = current.pollSeconds;
  $('cInitial').value = current.initialPosts;
  $('imgDir').value = current.image?.saveDir || '';
  $('imgMax').value = current.image?.maxMb || 20;
  fillSpeech(current.speech);
  fillWhitelistOpts();
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
  setInterval(() => refreshStatus().catch(() => {}), 2000);
  const startPage = new URLSearchParams(location.search).get('page');
  if (startPage && $(`page-${startPage}`)) showPage(startPage);
})().catch((e) => toast(`起動に失敗しました: ${e.message || e}`));
