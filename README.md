# TchViewer

Twitch + 5ch + したらば + JPNKN のコメントビューア（OBS ブラウザソース用）。
5ch / したらば / JPNKN は、LiveFake の配信ホワイトリストに載っているID（＝自分宛て）のレスだけを表示・読み上げる。

- 利用者向けの説明書: [docs/使い方.html](docs/使い方.html)（配布物にも同梱）
- 関連: LiveFake（5ch / したらば / JPNKN 専用ブラウザ）… https://github.com/kaedekiku/LiveFakeTauri2

## 起動
`TchViewer.exe` をダブルクリック（インストール不要）。専用ウィンドウが開き、ウィンドウを閉じると終了する。

- OBS ブラウザソース: `http://localhost:11280/overlay.html`（レイアウト指定は `?layout=名前`）
- `TchViewer.exe --headless` でウィンドウを出さずに動かせる（動作確認用）
- 画面表示には Windows 標準の WebView2 を使う（Windows 11 は標準搭載）

## ポータブル構成（配布物）
```
TchViewer/
├─ TchViewer.exe              … 本体（アプリ画面・オーバーレイを内蔵）
├─ LICENSE                    … TchViewer のライセンス（MIT）
├─ THIRD_PARTY_LICENSES.txt   … 使用ライブラリのライセンス表記
├─ 使い方.html                … 利用者向けの説明書
└─ data/                      … 初回起動時に作成。データはすべてここ（AppData・レジストリは使わない）
   ├─ config.json             … 設定
   ├─ layouts/                … レイアウト（名前.json = 見た目 / 名前.css = 追加CSS）
   ├─ window.json             … ウィンドウの位置・大きさ
   ├─ badges-cache.json       … Twitch バッジ一覧の予備（取得に失敗したとき用）
   ├─ tchviewer.log           … 動作ログ（今回の起動分。5MBを超えたら prev へ回す）
   ├─ tchviewer.prev.log      … 動作ログ（前回分）
   └─ webview/                … WebView2 の作業データ
```

## アプリ画面
| メニュー | 内容 |
|---|---|
| ビュワー | OBS に出るのと同じコメント（掲示板はホワイトリスト・NG通過分＋Twitch全チャット）。絞り込み・検索・もう一度読む・コピー |
| 接続 | Twitch チャンネル / スレッドURL / 巡回間隔 / Twitch バッジ取得のオン・オフ |
| LiveFake連携 | data フォルダ（またはファイル）をダイアログで選択。ホワイトリストの状態 |
| NGワード | 掲示板の本文のワードNG（`/…/` は正規表現・大小無視） |
| 読み上げ | 棒読みちゃん / SAPI / VOICEVOX |
| 表示（OBS） | レイアウトの作成・複製・名前変更・削除、見た目の調整、フォント選択、追加CSS、プレビュー、OBS用URL |
| 状態 | 取得元ごとの状態・件数（受信 / 表示 / ホワイトリスト外 / NG）とログ |
| 情報 | テーマ（Windows に合わせる / ライト / ダーク）・バージョン・データフォルダ・ライセンス |

上部のランプで、Twitch と各スレの接続状態（緑=正常 / 黄=接続中 / 赤=エラー）を常に表示する。
ウィンドウの位置・大きさ・最大化は data/window.json に保存して次回復元する（画面外になる位置なら復元しない）。
ビュワーでは、背景に対して読みにくい Twitch の名前色だけ、色合いを保って明るさを自動調整する（見やすさの比率 4.5:1 以上）。OBS 側はレイアウトの設定でオン・オフ。

## 取得方式
| ソース | 方式 | 文字コード |
|---|---|---|
| Twitch | 匿名IRC（WebSocket, justinfan）。ログイン不要・読み取り専用 | UTF-8 |
| 5ch | `https://{host}/{板}/dat/{key}.dat` を Range で差分取得（UA: Monazilla） | Shift_JIS |
| したらば | `rawmode.cgi/{dir}/{bbs}/{key}/{n}-` で新着のみ取得 | EUC-JP |
| JPNKN | 5ch互換dat（`bbs.jpnkn.com/{板}/dat/{key}.dat`） | Shift_JIS |

巡回間隔は LiveFakeTauri2 に合わせて 10〜300秒（既定15秒）。

## LiveFake連携（TCVホワイトリスト）
LiveFake が書き出す `broadcast-whitelist.json` を読み取り専用で監視する。
- 1秒ごとに更新日時とサイズを確認し、変わっていたら一度に読んで閉じる
- 読めない・壊れている → 直前に正しく読めた一覧を使う。ファイルが3秒以上無い → 空の一覧
- `date` が日本時間の今日でない → 空の一覧（0時を過ぎたら自動で無効）
- 空の一覧・未設定のときは掲示板のレスを1件も通さない
- 処理順: 差分取得 → ホワイトリスト → ワードNG → OBS表示・読み上げ

## 読み上げ
| エンジン | 仕組み |
|---|---|
| 棒読みちゃん | 「アプリケーション連携」TCPサーバー（既定 127.0.0.1:50001）へ送信 |
| SAPI | Windows 標準の音声合成（SAPI 5 の声と Windows 10/11 の新しい声）。XML として解釈しない設定で読む |
| VOICEVOX | このPCの VOICEVOX で音声を作り、メモリから直接再生（一時ファイルを作らない） |

読み上げ文は LiveFakeTauri2 に準拠（「レスN番さん / したらばN番さん / ジャパンくんN番さん」、URLは読まない、長文は「、長文のため以下省略」）。
棒読みちゃんの `教育(` `忘却(` はそのまま送る（利用者の判断で残している）。

## レイアウト（OBS のシーンごとの使い分け）
- 「表示（OBS）」でレイアウトを作り、OBS のブラウザソースごとに `overlay.html?layout=名前` を指定する
- 見た目の設定は即時に OBS へ反映。追加CSS（`data/layouts/名前.css`）は `!important` なしで設定値を上書きできる
- OBS のソースごとの上書き用URLパラメータ: `max` / `fade` / `size` / `top=1` / `hide=twitch,5ch` / `history`
- オーバーレイのクラス: `.msg` / `.board` `.twitch` / `.s-5ch` `.shitaraba` `.jpnkn` / `.badge` `.meta` `.no` `.name` `.uid` `.text` / `.badges img` / `img.emote(.giant)`
- コメントの配信は WebSocket（ソースをたくさん置いても、ブラウザの同時接続数の制限で止まらない）
- フォントはインストール済みのもの（ユーザー単位でインストールしたものも含む）から選べる

## Twitch のバッジ
Twitch のサイトが内部で使っている取得口（非公式）から、グローバルバッジとチャンネル独自のバッジの画像アドレスを取得する（接続時と1時間ごと）。
Twitch 側の変更で使えなくなる可能性がある。「接続」ページで取得をオフにできる。画像は `static-cdn.jtvnw.net/badges/v1/` のものだけ使う。

## セキュリティ
- HTTP サーバーは 127.0.0.1 のみ。Host ヘッダーが `localhost:11280` / `127.0.0.1:11280` 以外の要求は拒否（DNSリバインディング対策）
- HTTP で提供するのは OBS が読むもの（オーバーレイ・レイアウト・コメント配信）だけ。設定の変更はアプリ画面（Tauri IPC）からのみ
- アプリ画面は exe に内蔵し、CSP で外部スクリプトを禁止。コメント・名前・ID は textContent で表示
- レイアウト名はファイル名として安全な文字だけ受け付ける

## ビルド（開発者向け）
```
cargo test
cargo build --release                   # → target/release/TchViewer.exe
cargo audit                             # リリース前に、使っている部品に既知の脆弱性が無いか確認（cargo install cargo-audit）
node tools/gen-third-party-licenses.js  # THIRD_PARTY_LICENSES.txt を再生成（依存を変えたら必ず実行）
node tools/make-icon.js                 # アイコン（icons/icon.ico）を再生成
```
- アプリ画面は `ui/`、オーバーレイは `public/overlay.html`（どちらもビルド時に exe へ内蔵）
- デバッグ版だけ、`http://localhost:11280/ui/index.html` でアプリ画面をブラウザから確かめられる

## ライセンス方針
GPL系のライブラリは使わない。できるだけ MIT（または MIT を選べる二重ライセンス）を使う。
Tauri 2 の依存に MPL-2.0（cssparser / selectors など）と Apache-2.0 のみのもの（tao など）を含む。いずれも GPL 系ではない。

## 未実装・今後
- 次スレ自動追跡（保留）
- パワーアップで巨大化したスタンプの、実際の配信での確認（`msg-id=gigantified-emote-message` で判定）
