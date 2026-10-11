//! アプリ画面から呼ばれる処理（Tauri IPC の api コマンド）
//!
//! 画面は `api(cmd, args)` の1つの入口だけを使う。デバッグ版では同じ処理を /dev/api からも呼べる。

use crate::config::{Appearance, SpeechConfig};
use crate::{layouts, speech, timestamp, App, Comment, Emote, PORT};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{key} の形式が違います: {e}"))
}

pub async fn dispatch(app: &Arc<App>, handle: Option<&tauri::AppHandle>, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
        "fonts" => {
            #[cfg(windows)]
            let r = tokio::task::spawn_blocking(crate::fonts::list).await.map_err(|e| e.to_string())?;
            #[cfg(not(windows))]
            let r: Result<Vec<String>, String> = Ok(vec![]);
            Ok(json!(r?))
        }
        "app_info" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "dataDir": crate::config::data_dir().display().to_string(),
            "overlayUrl": format!("http://localhost:{PORT}/overlay.html"),
        })),
        "get_config" => Ok(serde_json::to_value(app.config()).unwrap_or_default()),
        // アプリ画面のテーマ（タイトルバーの色も合わせる）
        "set_theme" => {
            let theme: String = arg(&args, "theme")?;
            let theme = if ["light", "dark"].contains(&theme.as_str()) { theme } else { "system".into() };
            app.update_config(|cfg| cfg.ui_theme = theme.clone())?;
            if let Some(w) = handle.and_then(|h| tauri::Manager::get_webview_window(h, "main")) {
                let _ = w.set_theme(theme_of(&theme));
            }
            Ok(json!({ "ok": true }))
        }

        // ---- 接続（Twitch はつなぎ直す。掲示板は巡回対象をそろえるだけ）
        "save_twitch" => {
            let channels: Vec<String> = arg(&args, "channels")?;
            let badges: bool = arg(&args, "badges").unwrap_or(true);
            app.update_config(|cfg| {
                let mut seen = vec![];
                for c in channels.iter().map(|c| crate::twitch::normalize_channel(c)).filter(|c| !c.is_empty()) {
                    if !seen.contains(&c) {
                        seen.push(c);
                    }
                }
                cfg.twitch_channels = seen;
                cfg.twitch_badges = badges;
            })?;
            app.apply_connections();
            Ok(json!({ "ok": true }))
        }
        // 監視するスレの一覧（並び順どおり）
        "save_threads" => {
            let urls: Vec<String> = arg(&args, "threads")?;
            let mut list: Vec<String> = vec![];
            for u in urls {
                let info = crate::boards::parse_thread_url(&u)?;
                let url = info.read_url();
                if !list.contains(&url) {
                    list.push(url);
                }
            }
            app.update_config(|cfg| cfg.threads = list)?;
            app.sync_threads();
            Ok(json!({ "ok": true }))
        }
        "save_poll" => {
            let poll: u64 = arg(&args, "pollSeconds")?;
            let initial: usize = arg(&args, "initialPosts")?;
            app.update_config(|cfg| {
                cfg.poll_seconds = poll.clamp(10, 300);
                cfg.initial_posts = initial.min(1000);
            })?;
            Ok(json!({ "ok": true }))
        }
        // 貼り付けた URL を確かめて、スレタイを返す（登録前の確認用）
        "probe_thread" => {
            let url: String = arg(&args, "url")?;
            let info = crate::boards::parse_thread_url(&url)?;
            let key = info.key();
            let title = app.thread_views.lock().unwrap().get(&key).map(|v| v.title.clone()).filter(|t| !t.is_empty());
            let title = match title {
                Some(t) => t,
                None => crate::boards::fetch_title(&info, &app.http).await.unwrap_or_default(),
            };
            Ok(json!({ "url": info.read_url(), "key": key, "site": info.site(), "source": info.source(), "title": title }))
        }
        "stats" => Ok(json!({ "sources": app.stats(), "log": app.status_history() })),
        "recent" => {
            let n: usize = arg(&args, "n").unwrap_or(200);
            Ok(json!(app.recent(n.min(1000))))
        }
        "test_comment" => {
            test_comment(app);
            Ok(json!({ "ok": true }))
        }
        "save_ui" => {
            let panel: bool = arg(&args, "unlistedPanel")?;
            app.update_config(|cfg| cfg.unlisted_panel = panel)?;
            Ok(json!({ "ok": true }))
        }

        // ---- スレタブ
        "thread_tabs" => {
            let cfg = app.config();
            let tabs: Vec<Value> = cfg
                .open_tabs
                .iter()
                .filter_map(|u| crate::boards::parse_thread_url(u).ok())
                .map(|i| {
                    let key = i.key();
                    let title = app.thread_views.lock().unwrap().get(&key).map(|v| v.title.clone()).unwrap_or_default();
                    let monitored = cfg.threads.iter().any(|t| crate::ng::thread_key_of(t).as_deref() == Some(key.as_str()));
                    json!({ "key": key, "url": i.read_url(), "site": i.site(), "source": i.source(), "title": title, "monitored": monitored })
                })
                .collect();
            Ok(json!(tabs))
        }
        "thread_open" => {
            let url: String = arg(&args, "url")?;
            let info = crate::boards::parse_thread_url(&url)?;
            let url = info.read_url();
            app.update_config(|cfg| {
                if !cfg.open_tabs.contains(&url) {
                    cfg.open_tabs.push(url.clone());
                }
            })?;
            if app.config().open_tabs.len() > crate::threads::MAX_OPEN_TABS {
                app.update_config(|cfg| cfg.open_tabs.retain(|u| u != &url))?;
                return Err(format!("スレタブは {} 枚までです。使わないタブを閉じてください", crate::threads::MAX_OPEN_TABS));
            }
            app.sync_threads();
            Ok(json!({ "key": info.key(), "url": url }))
        }
        "thread_close" => {
            let url: String = arg(&args, "url")?;
            let key = crate::ng::thread_key_of(&url);
            app.update_config(|cfg| cfg.open_tabs.retain(|u| crate::ng::thread_key_of(u) != key))?;
            app.sync_threads();
            Ok(json!({ "ok": true }))
        }
        "thread_reorder" => {
            let urls: Vec<String> = arg(&args, "urls")?;
            app.update_config(|cfg| {
                let mut next: Vec<String> = urls.iter().filter(|u| cfg.open_tabs.contains(u)).cloned().collect();
                next.extend(cfg.open_tabs.iter().filter(|u| !urls.contains(u)).cloned());
                cfg.open_tabs = next;
            })?;
            Ok(json!({ "ok": true }))
        }
        "thread_get" => {
            let key: String = arg(&args, "key")?;
            Ok(app.thread_snapshot(&key))
        }
        // スレタブ・接続一覧の「監視する」
        "thread_monitor" => {
            let url: String = arg(&args, "url")?;
            let on: bool = arg(&args, "on")?;
            let info = crate::boards::parse_thread_url(&url)?;
            let (url, key) = (info.read_url(), info.key());
            app.update_config(|cfg| {
                cfg.threads.retain(|u| crate::ng::thread_key_of(u).as_deref() != Some(key.as_str()));
                if on {
                    cfg.threads.push(url.clone());
                }
            })?;
            app.sync_threads();
            Ok(json!({ "ok": true }))
        }
        // 本文の URL を既定のブラウザで開く（http / https のみ）
        "open_url" => {
            let url: String = arg(&args, "url")?;
            let u = url::Url::parse(url.trim()).map_err(|_| "URL の形式が正しくありません")?;
            if !matches!(u.scheme(), "http" | "https") {
                return Err("http / https 以外の URL は開けません".into());
            }
            let handle = handle.ok_or("アプリ画面でのみ使えます")?;
            use tauri_plugin_opener::OpenerExt;
            handle.opener().open_url(u.as_str(), None::<&str>).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        // ---- NG（取得済みのレスにもすぐ効く。接続はやり直さない）
        "save_ng" => {
            let ng: crate::config::NgConfig = arg(&args, "ng")?;
            let clean = |v: Vec<crate::config::NgEntry>| -> Vec<crate::config::NgEntry> {
                v.into_iter()
                    .filter(|e| !e.value.trim().is_empty())
                    .map(|mut e| {
                        e.value = e.value.trim().to_string();
                        if e.mode != "hide" {
                            e.mode = "abone".into();
                        }
                        if e.scope != "thread" {
                            e.scope = "global".into();
                            e.scope_url.clear();
                        }
                        e
                    })
                    .collect()
            };
            app.update_config(|cfg| {
                cfg.ng = crate::config::NgConfig { words: clean(ng.words), names: clean(ng.names), ids: clean(ng.ids) };
            })?;
            app.ui_event(json!({ "type": "ng_changed" }));
            Ok(json!({ "ok": true }))
        }

        // ---- ホワイトリスト（LiveFake の一覧 ＋ TchViewer 独自の一覧）
        "whitelist_status" => {
            let lf = app.whitelist_status();
            let mut own = app.own_wl.lock().unwrap();
            let auto_count = own.auto_count();
            let own = own.clone();
            Ok(json!({ "livefake": lf, "own": own.entries, "removed": own.removed, "autoCount": auto_count, "today": crate::whitelist::jst_today() }))
        }
        "whitelist_add" => {
            let id: String = arg(&args, "id")?;
            if !app.own_wl.lock().unwrap().add_manual(&id) {
                return Err("ID の形式が正しくありません".into());
            }
            app.ui_event(json!({ "type": "wl_changed" }));
            Ok(json!({ "ok": true }))
        }
        "whitelist_remove" => {
            let id: String = arg(&args, "id")?;
            app.own_wl.lock().unwrap().remove(&id);
            app.ui_event(json!({ "type": "wl_changed" }));
            Ok(json!({ "ok": true }))
        }
        "save_whitelist_opts" => {
            let mut o: crate::config::WhitelistOptions = arg(&args, "opts")?;
            o.auto_limit = o.auto_limit.clamp(1, 100);
            o.auto_words = o.auto_words.into_iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
            app.update_config(|cfg| cfg.whitelist = o)?;
            Ok(json!({ "ok": true }))
        }
        "save_whitelist_path" => {
            let path: String = arg(&args, "path")?;
            app.update_config(|cfg| cfg.whitelist_path = path.trim().to_string())?;
            app.restart_whitelist();
            Ok(json!({ "ok": true }))
        }
        "pick_whitelist" => {
            let mode: String = arg(&args, "mode").unwrap_or_else(|_| "folder".into());
            let handle = handle.ok_or("ファイル選択はアプリ画面でのみ使えます")?.clone();
            let current = crate::whitelist::resolve_path(&app.config().whitelist_path);
            let picked = tokio::task::spawn_blocking(move || pick_path(&handle, &mode, current)).await.map_err(|e| e.to_string())?;
            match picked {
                Some(path) => {
                    app.update_config(|cfg| cfg.whitelist_path = path.clone())?;
                    app.restart_whitelist();
                    Ok(json!({ "path": path }))
                }
                None => Ok(json!({ "path": Value::Null })),
            }
        }

        // ---- 画像ウィンドウ
        // 画像として開く URL か（開くときの URL とリファラ）
        "image_check" => {
            let url: String = arg(&args, "url")?;
            Ok(json!(crate::image::resolve(&url, &crate::image::load_rules()).map(|(u, _)| u)))
        }
        "open_image" => {
            let url: String = arg(&args, "url")?;
            let handle = handle.ok_or("アプリ画面でのみ使えます")?.clone();
            static IMAGE_SEQ: AtomicU64 = AtomicU64::new(0);
            let n = IMAGE_SEQ.fetch_add(1, Ordering::Relaxed);
            let page = format!("image.html?u={}", url::form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>());
            tauri::WebviewWindowBuilder::new(&handle, format!("image-{n}"), tauri::WebviewUrl::App(page.into()))
                .title("画像 - TchViewer")
                .inner_size(900.0, 700.0)
                .data_directory(crate::config::data_dir().join("webview"))
                .disable_drag_drop_handler()
                .build()
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }
        "image_fetch" => {
            let url: String = arg(&args, "url")?;
            let (target, referer) = crate::image::resolve(&url, &crate::image::load_rules()).unwrap_or((url.clone(), None));
            let max = app.config().image.max_mb.clamp(1, 100) as usize * 1024 * 1024;
            let f = crate::image::fetch(&target, referer.as_deref(), max).await?;
            let data = crate::image::data_url(&f);
            let size = f.bytes.len();
            {
                let mut c = app.images.lock().unwrap();
                c.retain(|(u, _, _)| u != &url);
                c.push_back((url.clone(), f.mime.clone(), f.bytes));
                while c.len() > 5 {
                    c.pop_front();
                }
            }
            Ok(json!({ "dataUrl": data, "mime": f.mime, "size": size, "url": f.url }))
        }
        "image_save" => {
            let url: String = arg(&args, "url")?;
            let (mime, bytes) = app.images.lock().unwrap().iter().find(|(u, _, _)| u == &url).map(|(_, m, b)| (m.clone(), b.clone())).ok_or("画像をもう一度開いてから保存してください")?;
            let mut dir = app.config().image.save_dir;
            if dir.is_empty() || !std::path::Path::new(&dir).is_dir() {
                // 初めて保存するとき（または保存先が無いとき）は保存先を選ぶ
                let handle = handle.ok_or("アプリ画面でのみ使えます")?.clone();
                let picked = tokio::task::spawn_blocking(move || pick_dir(&handle, "画像の保存先を選んでください")).await.map_err(|e| e.to_string())?;
                let Some(p) = picked else { return Ok(json!({ "saved": Value::Null })) };
                app.update_config(|cfg| cfg.image.save_dir = p.clone())?;
                dir = p;
            }
            let path = crate::image::unique_path(std::path::Path::new(&dir), &crate::image::file_name(&url, &mime));
            std::fs::write(&path, bytes).map_err(|e| format!("保存できません: {e}"))?;
            Ok(json!({ "saved": path.display().to_string() }))
        }
        "pick_image_dir" => {
            let handle = handle.ok_or("アプリ画面でのみ使えます")?.clone();
            let picked = tokio::task::spawn_blocking(move || pick_dir(&handle, "画像の保存先を選んでください")).await.map_err(|e| e.to_string())?;
            if let Some(p) = &picked {
                app.update_config(|cfg| cfg.image.save_dir = p.clone())?;
            }
            Ok(json!({ "path": picked }))
        }
        "save_image_opts" => {
            let mb: u32 = arg(&args, "maxMb")?;
            app.update_config(|cfg| cfg.image.max_mb = mb.clamp(1, 100))?;
            Ok(json!({ "ok": true }))
        }

        // ---- 読み上げ（接続はやり直さない）
        "save_speech" => {
            let sp: SpeechConfig = arg(&args, "speech")?;
            app.update_config(|cfg| cfg.speech = sp)?;
            Ok(json!({ "ok": true }))
        }
        "sapi_voices" => {
            #[cfg(windows)]
            let r = tokio::task::spawn_blocking(crate::tts_win::list_voices).await.map_err(|e| e.to_string())?;
            #[cfg(not(windows))]
            let r: Result<Vec<String>, String> = Err("Windows でのみ使えます".into());
            Ok(json!({ "voices": r? }))
        }
        "voicevox_speakers" => {
            let endpoint: String = arg(&args, "endpoint").unwrap_or_else(|_| crate::config::VoicevoxConfig::default().endpoint);
            let list = speech::voicevox_speakers(&reqwest::Client::new(), &endpoint).await.map_err(|e| format!("VOICEVOX に接続できません: {e}"))?;
            Ok(json!({ "speakers": list.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>() }))
        }
        "speech_test" => {
            app.enqueue_speech("レス123番さん 読み上げのテストです".into());
            Ok(json!({ "ok": true }))
        }
        "speech_stop" => {
            app.stop_speech();
            Ok(json!({ "ok": true }))
        }
        // ビュワーの「もう一度読む」（読み上げのオン・オフやソースの設定に関係なく読む）
        "speak_comment" => {
            let id: u64 = arg(&args, "id")?;
            let c = app.find_comment(id).ok_or("古いコメントのため読み上げできません")?;
            let text = speech::speech_text(&c, &app.config().speech);
            if !text.is_empty() {
                app.enqueue_speech(text);
            }
            Ok(json!({ "ok": true }))
        }

        // ---- レイアウト（OBS表示の見た目。接続はやり直さない。OBS 側へすぐ反映）
        "layouts" => Ok(json!(layouts::list())),
        "layout_get" => {
            let name: String = arg(&args, "name")?;
            layouts::check_name(&name)?;
            Ok(json!({ "appearance": layouts::load(&name), "css": layouts::load_css(&name) }))
        }
        "layout_save" => {
            let name: String = arg(&args, "name")?;
            let a: Appearance = arg(&args, "appearance")?;
            layouts::save(&name, &a)?;
            app.broadcast_layout(&name);
            Ok(json!({ "ok": true }))
        }
        "layout_save_css" => {
            let name: String = arg(&args, "name")?;
            let css: String = arg(&args, "css")?;
            layouts::save_css(&name, &css)?;
            app.broadcast_layout_css(&name);
            Ok(json!({ "ok": true }))
        }
        "layout_create" => {
            let name: String = arg(&args, "name")?;
            let from: Option<String> = arg(&args, "from").unwrap_or(None);
            layouts::create(name.trim(), from.as_deref())?;
            Ok(json!({ "ok": true }))
        }
        "layout_rename" => {
            let from: String = arg(&args, "from")?;
            let to: String = arg(&args, "to")?;
            layouts::rename(&from, to.trim())?;
            Ok(json!({ "ok": true }))
        }
        "layout_delete" => {
            let name: String = arg(&args, "name")?;
            layouts::delete(&name)?;
            Ok(json!({ "ok": true }))
        }

        // デバッグ版のみ: 監視中のスレに試しのレスを流し込む（動作確認用）
        #[cfg(debug_assertions)]
        "dev_inject" => {
            let key: String = arg(&args, "key")?;
            let w = app.watchers.lock().unwrap().get(&key).cloned().ok_or("そのスレは巡回していません")?;
            let mut g = w.lock().await;
            let no = g.inject(app, &arg::<String>(&args, "name")?, &arg::<String>(&args, "uid")?, &arg::<String>(&args, "text")?);
            Ok(json!({ "no": no }))
        }

        _ => Err(format!("不明な操作です: {cmd}")),
    }
}

/// 設定値 → ウィンドウのテーマ（None は Windows の設定に合わせる）
pub fn theme_of(s: &str) -> Option<tauri::Theme> {
    match s {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    }
}

/// フォルダを選ぶダイアログ（ブロッキング）
fn pick_dir(handle: &tauri::AppHandle, title: &str) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    handle.dialog().file().set_title(title).blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

/// フォルダ / ファイルを選ぶダイアログ（ブロッキング。呼び出し側で spawn_blocking する）
fn pick_path(handle: &tauri::AppHandle, mode: &str, current: Option<std::path::PathBuf>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = handle.dialog().file();
    if let Some(dir) = current.as_ref().and_then(|p| p.parent()).filter(|p| p.is_dir()) {
        d = d.set_directory(dir);
    }
    let picked = if mode == "file" {
        d.set_title("broadcast-whitelist.json を選んでください").add_filter("JSON", &["json"]).blocking_pick_file()
    } else {
        d.set_title("LiveFake の data フォルダを選んでください").blocking_pick_folder()
    };
    picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

/// 押すたびに Twitch → 5ch → したらば → JPNKN の順にテストコメントを送る
fn test_comment(app: &Arc<App>) {
    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);
    let i = TEST_SEQ.fetch_add(1, Ordering::Relaxed) % 4;
    let (source, site) = [("twitch", "Twitch"), ("5ch", "5ch"), ("shitaraba", "したらば"), ("jpnkn", "JPNKN")][i as usize];
    let mut c = Comment {
        source: source.into(),
        site: site.into(),
        thread: "テスト".into(),
        name: "テスト".into(),
        no: (source != "twitch").then_some(123),
        uid: if source == "twitch" { String::new() } else { "TeSt1234".into() },
        text: format!("これはテストコメントです。表示確認用。（{}）", timestamp()),
        color: "#9147ff".into(),
        ..Default::default()
    };
    if source == "twitch" {
        // スタンプ（Kappa）と、パワーアップで巨大化したスタンプの表示確認用
        c.text = "スタンプ Kappa と巨大スタンプ Kappa".into();
        c.emotes = vec![Emote { id: "25".into(), start: 5, end: 9 }, Emote { id: "25".into(), start: 19, end: 23 }];
        c.giant = true;
        // バッジの表示確認用（グローバルバッジ）
        c.badges = "broadcaster/1,moderator/1,vip/1".into();
        c.badge_list = app.badges.read().unwrap().resolve("", &c.badges);
    }
    app.emit_comment(c);
}
