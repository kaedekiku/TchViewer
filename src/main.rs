//! TchViewer — Twitch + 5ch + したらば + JPNKN コメントビューア（OBSブラウザソース用）
//!
//! - アプリ画面（ビュワー・設定）は exe に内蔵し、Tauri 2 / WebView2 の専用ウィンドウで開く。
//!   画面と本体のやり取りは Tauri の IPC（api コマンド 1 つ）だけで行う。
//! - OBS 用の表示は http://localhost:11280/overlay.html で配信する（HTTP は OBS が読むものだけ）。
//! - データは exe と同じフォルダの data/ にだけ保存する（WebView2 の作業データも data/webview）。

// リリース版ではコンソール画面を出さない
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod api;
mod badges;
mod boards;
mod config;
mod events;
mod eventsub;
#[cfg(windows)]
mod fonts;
mod image;
mod layouts;
mod links;
mod ng;
mod server;
mod speech;
mod text;
mod threads;
#[cfg(windows)]
mod tts_win;
mod twitch;
mod whitelist;
mod window_state;

use config::Config;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

pub const PORT: u16 = 11280;
/// 再読み込み時の再送・ビュワーの「もう一度読む」に使う直近のコメント数
const HISTORY_MAX: usize = 1000;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Emote {
    pub id: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Comment {
    pub source: String,
    pub site: String,
    pub thread: String,
    pub no: Option<u32>,
    pub name: String,
    pub uid: String,
    pub text: String,
    /// 接続時に表示した既存レス（読み上げない）
    pub initial: bool,
    // Twitch のみ
    pub login: String,
    pub color: String,
    pub badges: String,
    pub emotes: Vec<Emote>,
    /// パワーアップ「スタンプを巨大化」されたメッセージ（最後のスタンプを大きく表示）
    pub giant: bool,
    /// バッジの画像（badges タグを画像に変換したもの）
    #[serde(rename = "badgeList")]
    pub badge_list: Vec<badges::BadgeImage>,
    /// Twitch のイベント（サブスク・ギフト・ビッツ・レイド・連続視聴・チャンネルポイントなど）
    pub event: Option<events::TwitchEvent>,
}

/// 取得元ごとの状況（「状態」ページ・上部のランプ用）
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStat {
    /// "twitch" / "5ch" / "shitaraba" / "jpnkn"
    pub source: String,
    pub site: String,
    /// チャンネル名 / スレッドの板・番号
    pub label: String,
    /// スレタイ（掲示板のみ）
    pub title: String,
    /// "connecting" / "ok" / "error"
    pub state: String,
    pub last_fetch: String,
    pub last_error: String,
    /// 現在のレス数（掲示板のみ）
    pub res_count: u32,
    /// 接続後に新しく届いた件数
    pub received: u64,
    /// そのうち OBS に流した件数
    pub passed: u64,
    pub blocked_whitelist: u64,
    pub blocked_ng: u64,
}

pub struct App {
    config: RwLock<Config>,
    /// 画面・OBSへ流すメッセージ（JSON文字列）
    tx: broadcast::Sender<String>,
    /// アプリ画面だけに流すメッセージ（番号読みのレス・スレタブの新着など。OBS には送らない）
    ui_tx: broadcast::Sender<String>,
    /// 掲示板の取得に使う HTTP クライアント
    http: reqwest::Client,
    /// 巡回中のスレ（監視中・スレタブで開いているもの）
    watchers: Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<boards::ThreadWatcher>>>>,
    /// 取得済みのレス（スレタブの表示用。メモリのみ）
    thread_views: Mutex<std::collections::HashMap<String, threads::ThreadView>>,
    /// TchViewer 独自のホワイトリスト（手動・自動）
    own_wl: Mutex<whitelist::OwnWhitelist>,
    /// Twitch ログイン（トークンはメモリと暗号化ファイルだけ。画面には渡さない）
    auth: Mutex<Option<eventsub::AuthData>>,
    auth_status: Mutex<eventsub::AuthStatus>,
    login_task: Mutex<Option<JoinHandle<()>>>,
    eventsub_task: Mutex<Option<JoinHandle<()>>>,
    eventsub_connected: std::sync::atomic::AtomicBool,
    /// 画像ウィンドウで開いた画像（保存用。直近5件・メモリのみ）
    images: Mutex<VecDeque<(String, String, Vec<u8>)>>,
    /// 直近のコメント（id, コメント, 送信済みJSON）
    history: Mutex<VecDeque<(u64, Comment, String)>>,
    /// ステータス欄用（直近分）
    status_history: Mutex<VecDeque<String>>,
    /// data/tchviewer.log（起動ごとに作り直す。コンソールが無いので不具合調査用）
    log_file: Mutex<config::LogFile>,
    seq: AtomicU64,
    boot: u64,
    /// Twitch 接続・掲示板巡回のタスク
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// ホワイトリスト監視のタスク（接続とは別にやり直せるよう分ける）
    whitelist_task: Mutex<Option<JoinHandle<()>>>,
    /// 読み上げの順番待ち（世代番号, 1件）。停止すると世代が進み、古い世代は読まずに捨てる
    speech_tx: mpsc::UnboundedSender<(u64, speech::SpeechItem)>,
    speech_gen: AtomicU64,
    whitelist: RwLock<whitelist::Whitelist>,
    whitelist_status: Mutex<whitelist::WhitelistStatus>,
    stats: Mutex<BTreeMap<String, SourceStat>>,
    /// Twitch のバッジ画像の一覧
    pub badges: RwLock<badges::BadgeStore>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn timestamp() -> String {
    // 時刻ライブラリを増やさないため、日本時間(UTC+9)固定で 時:分:秒 を出す
    let s = now_ms() / 1000 + 9 * 3600;
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

impl App {
    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    /// 設定を書き換えて保存する
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let cfg = {
            let mut c = self.config.write().unwrap();
            f(&mut c);
            c.clone()
        };
        config::save(&cfg).map_err(|e| format!("設定を保存できません: {e}"))?;
        Ok(cfg)
    }

    pub fn whitelist(&self) -> whitelist::Whitelist {
        self.whitelist.read().unwrap().clone()
    }

    pub fn set_whitelist(&self, w: whitelist::Whitelist, state: String, path: String) {
        let mut st = self.whitelist_status.lock().unwrap();
        st.path = path;
        st.state = state;
        st.date = w.date.clone();
        let mut ids: Vec<String> = w.ids.iter().cloned().collect();
        ids.sort();
        st.ids = ids;
        *self.whitelist.write().unwrap() = w;
    }

    pub fn set_whitelist_state(&self, state: String) {
        self.whitelist_status.lock().unwrap().state = state;
    }

    pub fn whitelist_status(&self) -> whitelist::WhitelistStatus {
        let mut st = self.whitelist_status.lock().unwrap().clone();
        st.today = whitelist::jst_today();
        st.active = !st.date.is_empty() && st.date == st.today;
        st
    }

    /// 取得元の状況を更新する
    pub fn stat(&self, key: &str, f: impl FnOnce(&mut SourceStat)) {
        let mut m = self.stats.lock().unwrap();
        f(m.entry(key.to_string()).or_default());
    }

    pub fn stats(&self) -> Vec<SourceStat> {
        self.stats.lock().unwrap().values().cloned().collect()
    }

    /// ログファイル（とデバッグ版のコンソール）にだけ出す
    pub fn log(&self, text: String) {
        let line = format!("[{}] {text}", timestamp());
        println!("{line}");
        self.log_file.lock().unwrap().write_line(&line);
    }

    /// ログと画面のステータス欄に出す
    pub fn status(&self, text: String) {
        self.log(text.clone());
        let line = format!("[{}] {text}", timestamp());
        {
            let mut h = self.status_history.lock().unwrap();
            h.push_back(line.clone());
            while h.len() > 200 {
                h.pop_front();
            }
        }
        let msg = serde_json::json!({ "type": "status", "text": line, "boot": self.boot });
        let _ = self.tx.send(msg.to_string());
    }

    pub fn status_history(&self) -> Vec<String> {
        self.status_history.lock().unwrap().iter().cloned().collect()
    }

    pub fn emit_comment(&self, mut c: Comment) {
        let cfg = self.config();
        let speak = match c.event.as_mut() {
            // イベント: 種類ごとの設定で、流すか・読むか・読み上げる文を決める
            Some(e) => {
                let set = cfg.twitch_events.get(&e.kind).cloned().unwrap_or_default();
                if !set.show {
                    return;
                }
                let tpl = if set.template.trim().is_empty() { "{name} {message}".to_string() } else { set.template.clone() };
                e.summary = e.render(&tpl, &c.name, "");
                let msg = if e.kind == "bits" { events::strip_cheermotes(&c.text) } else { c.text.clone() };
                (cfg.speech.twitch_on && set.speak).then(|| e.render(&tpl, &c.name, &msg)).map(|t| speech::clean_for_speech(&t, &cfg.speech)).filter(|t| !t.is_empty())
            }
            None => speech::should_speak(&c, &cfg.speech),
        };
        let kind = if c.source == "twitch" { "twitch" } else { "thread" };
        let id = self.seq.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let mut v = serde_json::to_value(&c).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.insert("type".into(), "comment".into());
            o.insert("id".into(), id.into());
            o.insert("boot".into(), self.boot.into());
            o.insert("ts".into(), now_ms().into());
            o.insert("time".into(), timestamp().into());
        }
        let data = v.to_string();
        {
            let mut h = self.history.lock().unwrap();
            h.push_back((id, c, data.clone()));
            while h.len() > HISTORY_MAX {
                h.pop_front();
            }
        }
        let _ = self.tx.send(data);
        if let Some(text) = speak {
            self.enqueue_item(speech::SpeechItem::text(text, Some(format!("c:{id}"))).kind(kind));
        }
    }

    /// 直近 n 件のコメント（送信済みJSON）
    pub fn recent(&self, n: usize) -> Vec<String> {
        let h = self.history.lock().unwrap();
        h.iter().skip(h.len().saturating_sub(n)).map(|(_, _, d)| d.clone()).collect()
    }

    pub fn find_comment(&self, id: u64) -> Option<Comment> {
        self.history.lock().unwrap().iter().find(|(i, _, _)| *i == id).map(|(_, c, _)| c.clone())
    }

    /// レイアウトの変更を OBS 側へすぐ反映する（そのレイアウトを使っているソースだけが反映する）
    pub fn broadcast_layout(&self, layout: &str) {
        let msg = serde_json::json!({ "type": "appearance", "layout": layout, "appearance": layouts::load(layout), "boot": self.boot });
        let _ = self.tx.send(msg.to_string());
    }

    pub fn broadcast_layout_css(&self, layout: &str) {
        let msg = serde_json::json!({ "type": "layout_css", "layout": layout, "boot": self.boot });
        let _ = self.tx.send(msg.to_string());
    }

    pub fn enqueue_speech(&self, text: String) {
        self.enqueue_item(speech::SpeechItem::text(text, None));
    }

    pub fn enqueue_item(&self, item: speech::SpeechItem) {
        let _ = self.speech_tx.send((self.speech_gen.load(Ordering::SeqCst), item));
    }

    /// アプリ画面だけに知らせる
    pub fn ui_event(&self, v: serde_json::Value) {
        let _ = self.ui_tx.send(v.to_string());
    }

    /// 読み上げ中のものと順番待ちをすべて止める
    pub fn stop_speech(&self) {
        self.speech_gen.fetch_add(1, Ordering::SeqCst);
        #[cfg(windows)]
        tts_win::stop_all();
    }

    /// ホワイトリストの監視をやり直す
    pub fn restart_whitelist(self: &Arc<Self>) {
        let mut t = self.whitelist_task.lock().unwrap();
        if let Some(old) = t.take() {
            old.abort();
        }
        *t = Some(tokio::spawn(whitelist::watch(self.clone(), self.config().whitelist_path)));
    }

    /// 設定に従って Twitch 接続をやり直し、掲示板の巡回対象をそろえる
    pub fn apply_connections(self: &Arc<Self>) {
        let mut tasks = self.tasks.lock().unwrap();
        for t in tasks.drain(..) {
            t.abort();
        }
        self.stats.lock().unwrap().retain(|k, _| !k.starts_with("Twitch "));
        let cfg = self.config();
        let channels: Vec<String> = cfg.twitch_channels.iter().map(|c| twitch::normalize_channel(c)).filter(|c| !c.is_empty()).collect();
        for ch in &channels {
            tasks.push(tokio::spawn(twitch::run(self.clone(), ch.clone())));
        }
        if cfg.twitch_badges && !channels.is_empty() {
            tasks.push(tokio::spawn(badges::run(self.clone(), channels)));
        }
        drop(tasks);
        self.sync_threads();
    }
}

/// 読み上げは1本の順番待ちで送る（送信順＝読み上げ順にするため）
async fn speech_worker(app: Arc<App>, mut rx: mpsc::UnboundedReceiver<(u64, speech::SpeechItem)>) {
    let client = reqwest::Client::new();
    let mut last_error: Option<Instant> = None;
    while let Some((generation, item)) = rx.recv().await {
        if generation < app.speech_gen.load(Ordering::SeqCst) {
            continue; // 停止より前に積まれたものは読まない
        }
        let sp = app.config().speech;
        // 積まれた後で、その種類（Twitch / スレ）の読み上げがオフにされたものは読まない
        if item.kind.is_some_and(|k| !sp.kind_on(k)) {
            continue;
        }
        // 画面で「今読んでいるもの」を示す
        if let Some(k) = &item.key {
            app.ui_event(serde_json::json!({ "type": "speaking", "key": k }));
        }
        let started = Instant::now();
        let text = item.text.clone();
        let result = if item.chime {
            #[cfg(windows)]
            {
                tokio::task::spawn_blocking(|| tts_win::play_wav(&speech::chime_wav())).await.map_err(|e| e.to_string()).and_then(|r| r)
            }
            #[cfg(not(windows))]
            Ok(())
        } else {
            speech::speak(&sp, &client, &text).await
        };
        let what = if item.chime { "通知音".to_string() } else { text.chars().take(30).collect::<String>() };
        app.log(format!("読み上げ({}) {:.1}秒: {what}", sp.engine, started.elapsed().as_secs_f32()));
        if let Err(e) = result {
            if last_error.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                last_error = Some(Instant::now());
                app.status(e);
            }
        }
    }
}

/// エラーをメッセージボックスで知らせて終了する（コンソールが無いため）
fn fatal(msg: &str) -> ! {
    eprintln!("{msg}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, caption) = (wide(msg), wide("TchViewer"));
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR) };
    }
    std::process::exit(1)
}

/// 画面からの呼び出し口（Tauri IPC）。処理の中身は api.rs にまとめる
#[tauri::command]
async fn api(
    handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<App>>,
    cmd: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    api::dispatch(state.inner(), Some(&handle), &cmd, args).await
}

fn main() {
    // `--headless`: ウィンドウを出さずにサーバーだけ動かす（動作確認・自動テスト用）
    let headless = std::env::args().any(|a| a == "--headless" || a == "--no-browser");
    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| fatal(&format!("初期化に失敗しました: {e}")));
    tauri::async_runtime::set(rt.handle().clone());
    let _guard = rt.enter();

    let (tx, _) = broadcast::channel(1024);
    let (ui_tx, _) = broadcast::channel(1024);
    let (speech_tx, speech_rx) = mpsc::unbounded_channel();
    let app = Arc::new(App {
        config: RwLock::new(config::load()),
        tx,
        ui_tx,
        http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_else(|e| fatal(&format!("初期化に失敗しました: {e}"))),
        watchers: Mutex::new(std::collections::HashMap::new()),
        thread_views: Mutex::new(std::collections::HashMap::new()),
        own_wl: Mutex::new(whitelist::OwnWhitelist::load()),
        images: Mutex::new(VecDeque::new()),
        auth: Mutex::new(None),
        auth_status: Mutex::new(eventsub::AuthStatus { state: "none".into(), ..Default::default() }),
        login_task: Mutex::new(None),
        eventsub_task: Mutex::new(None),
        eventsub_connected: std::sync::atomic::AtomicBool::new(false),
        history: Mutex::new(VecDeque::new()),
        status_history: Mutex::new(VecDeque::new()),
        log_file: Mutex::new(config::LogFile::open()),
        seq: AtomicU64::new(0),
        boot: now_ms(),
        tasks: Mutex::new(vec![]),
        whitelist_task: Mutex::new(None),
        speech_tx,
        speech_gen: AtomicU64::new(0),
        whitelist: RwLock::new(whitelist::Whitelist::default()),
        whitelist_status: Mutex::new(whitelist::WhitelistStatus::default()),
        stats: Mutex::new(BTreeMap::new()),
        badges: RwLock::new(badges::BadgeStore::default()),
    });
    tokio::spawn(speech_worker(app.clone(), speech_rx));

    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", PORT)))
        .unwrap_or_else(|e| fatal(&format!("ポート {PORT} を開けません。TchViewer が既に起動していないか確認してください。\n\n{e}")));
    app.log(format!("TchViewer {} 起動", env!("CARGO_PKG_VERSION")));
    app.log(format!("OBSブラウザソース: http://localhost:{PORT}/overlay.html"));
    app.log(format!("データフォルダ: {}", config::data_dir().display()));
    // 以前の見た目の設定を「標準」レイアウトへ引き継ぐ
    layouts::migrate(&app.config().appearance);
    app.restart_whitelist();
    app.apply_connections();
    tokio::spawn(app.clone().run_poller());
    app.start_saved_login();
    let server = tokio::spawn(server::serve(listener, app.clone()));

    if headless {
        if let Err(e) = rt.block_on(server) {
            fatal(&format!("サーバーエラー: {e}"));
        }
        return;
    }

    let app_for_setup = app.clone();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app.clone())
        .invoke_handler(tauri::generate_handler![api])
        .setup(move |handle| {
            // デバッグ版のみ: 環境変数 TCHVIEWER_START_PAGE で最初に開くページを指定できる（画面確認用）
            #[cfg(debug_assertions)]
            let start = std::env::var("TCHVIEWER_START_PAGE").ok().filter(|p| p.chars().all(|c| c.is_ascii_alphabetic())).map(|p| format!("index.html?page={p}")).unwrap_or_else(|| "index.html".into());
            #[cfg(not(debug_assertions))]
            let start = "index.html".to_string();
            let win = tauri::WebviewWindowBuilder::new(handle, "main", tauri::WebviewUrl::App(start.into()))
                .title("TchViewer")
                .inner_size(1180.0, 820.0)
                .min_inner_size(640.0, 480.0)
                // 画面内のドラッグ（タブの並べ替え・文字のドラッグ）を使うため、外部ファイルの受け取りは無効にする
                .disable_drag_drop_handler()
                // 前回の位置・大きさを当てはめてから表示する（ちらつき防止）
                .visible(false)
                // WebView2 の作業データを AppData ではなく exe フォルダの data/webview に置く
                .data_directory(config::data_dir().join("webview"))
                .build()?;
            window_state::init();
            window_state::restore(&win);
            let _ = win.set_theme(api::theme_of(&app_for_setup.config().ui_theme));
            let _ = win.show();
            let w = win.clone();
            win.on_window_event(move |e| match e {
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => window_state::changed(&w),
                tauri::WindowEvent::CloseRequested { .. } => window_state::closing(&w),
                _ => {}
            });
            // コメント・状態の変化（OBS と共通のもの・アプリ画面だけのもの）を画面へ流す
            for mut rx in [app_for_setup.tx.subscribe(), app_for_setup.ui_tx.subscribe()] {
                let handle = handle.handle().clone();
                tokio::spawn(async move {
                    use tauri::Emitter;
                    loop {
                        match rx.recv().await {
                            Ok(msg) => {
                                let _ = handle.emit_to("main", "tv", msg);
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        fatal(&format!("ウィンドウを開けませんでした（WebView2 が必要です）: {e}"));
    }
}
