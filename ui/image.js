// 画像ウィンドウ: 本体が取得した画像（データ）を表示する。右クリックで保存など
'use strict';

const url = new URLSearchParams(location.search).get('u') || '';
$('iUrl').textContent = url;
$('iUrl').title = url;

async function save() {
  const r = await safe(() => api('image_save', { url }));
  if (r && r.saved) toast(`保存しました: ${r.saved}`);
}
function openBrowser() { safe(() => api('open_url', { url })); }
$('iSave').addEventListener('click', save);
$('iBrowser').addEventListener('click', openBrowser);

window.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  showMenu(e.clientX, e.clientY, [
    ['画像を保存', save],
    ['画像の URL をコピー', () => navigator.clipboard.writeText(url).then(() => toast('URL をコピーしました'))],
    ['ブラウザで開く', openBrowser],
  ]);
});

(async () => {
  try {
    const cfg = await api('get_config');
    applyTheme(cfg.uiTheme);
    const r = await api('image_fetch', { url });
    const img = el('img');
    img.alt = url;
    img.src = r.dataUrl; // 本体が確かめて取得した画像だけを data: で表示する
    img.addEventListener('click', () => $('iStage').classList.toggle('actual'));
    img.addEventListener('load', () => { $('iInfo').textContent = `${img.naturalWidth}×${img.naturalHeight} / ${(r.size / 1024).toFixed(0)}KB`; });
    $('iStage').replaceChildren(img);
  } catch (e) {
    $('iNote').textContent = `画像を表示できません: ${e.message || e}`;
  }
})();
