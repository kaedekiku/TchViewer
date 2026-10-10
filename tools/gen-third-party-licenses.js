// 配布用のサードパーティライセンス表記 THIRD_PARTY_LICENSES.txt を生成する（開発時のみ使用）
//   使い方: node tools/gen-third-party-licenses.js
// cargo metadata から exe に関わる依存（dev 依存を除く）を集め、各クレート同梱のライセンス文を書き出す。
// 「MIT OR …」のように MIT を選べるものは MIT を選択し、MIT の本文だけを載せる。
'use strict';
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const meta = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'], { cwd: root, maxBuffer: 64 << 20 }).toString());
const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const seen = new Set();
(function walk(id) {
  for (const d of nodes.get(id).deps) {
    if (d.dep_kinds.every((k) => k.kind === 'dev')) continue;
    if (seen.has(d.pkg)) continue;
    seen.add(d.pkg);
    walk(d.pkg);
  }
})(meta.resolve.root);

const isLicenseFile = (f) => /^(licen[cs]e|copying|notice|unlicense)/i.test(f);
const mitSelectable = (expr) => /^\(?\s*([\w.-]+\s+OR\s+)*MIT(\s+OR\s+[\w.-]+)*\s*\)?$/i.test(expr.replace(/\//g, ' OR '));

const MIT_TEXT = (holders) => `MIT License

Copyright (c) ${holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;
const BSD3_TEXT = (holders) => `Copyright (c) ${holders}

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`;

const list = [...seen].map((id) => pkgs.get(id)).sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
let out = `TchViewer が使用しているサードパーティのソフトウェアとライセンス
（${list.length} 件。tools/gen-third-party-licenses.js で自動生成）

TchViewer 本体のライセンスは LICENSE（MIT License）を参照してください。
"MIT OR Apache-2.0" のように選択できるものは、MIT License を選択しています。

`;
const missing = [];
// MPL-2.0 の全文は、同梱しているほかのクレートのものを流用する
const readText = (p, f) => fs.readFileSync(path.join(path.dirname(p.manifest_path), f), 'utf8').replace(/\r\n/g, '\n').trim();
let mplText = '';
for (const p of list) {
  if (p.license === 'MPL-2.0' && !mplText) {
    const f = fs.readdirSync(path.dirname(p.manifest_path)).find(isLicenseFile);
    if (f) mplText = readText(p, f);
  }
}
for (const p of list) {
  const expr = p.license || '(不明)';
  const chooseMit = expr !== 'MIT' && mitSelectable(expr);
  const dir = path.dirname(p.manifest_path);
  let files = fs.readdirSync(dir).filter(isLicenseFile).sort();
  if (chooseMit) {
    const mit = files.filter((f) => /mit/i.test(f));
    if (mit.length) files = mit;
  }
  out += `${'='.repeat(78)}\n${p.name} ${p.version}\nライセンス: ${expr}${chooseMit ? '（MIT を選択）' : ''}\n`;
  if (p.repository) out += `配布元: ${p.repository}\n`;
  out += `${'-'.repeat(78)}\n`;
  if (!files.length) {
    // クレートにライセンス文が同梱されていないときは、標準の文面に作者（Cargo.toml の authors）を入れて補う
    const holders = (p.authors || []).map((a) => a.replace(/\s*<[^>]*>/, '')).join(', ') || `${p.name} の作者`;
    const text = chooseMit || expr === 'MIT' ? MIT_TEXT(holders) : expr === 'BSD-3-Clause' ? BSD3_TEXT(holders) : expr === 'MPL-2.0' ? mplText : '';
    if (!text) missing.push(`${p.name} ${p.version} (${expr})`);
    out += `（クレートにライセンス文のファイルが同梱されていないため、標準の文面で記載）\n\n${text || '上記の配布元を参照してください。'}\n\n`;
    continue;
  }
  for (const f of files) {
    out += `[${f}]\n${fs.readFileSync(path.join(dir, f), 'utf8').replace(/\r\n/g, '\n').trim()}\n\n`;
  }
}
fs.writeFileSync(path.join(root, 'THIRD_PARTY_LICENSES.txt'), out.replace(/\n/g, '\r\n'));
console.log(`THIRD_PARTY_LICENSES.txt: ${list.length} 件`);
const gpl = list.filter((p) => /GPL/i.test(p.license || ''));
console.log(`GPL系: ${gpl.length ? gpl.map((p) => p.name).join(', ') : 'なし'}`);
if (missing.length) console.log(`ライセンス文を用意できなかったもの (${missing.length}):\n  ` + missing.join('\n  '));
