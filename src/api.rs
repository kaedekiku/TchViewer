//! アプリ画面から呼ばれる処理（Tauri IPC の api コマンド）
//!
//! 画面は `api(cmd, args)` の1つの入口だけを使う。デバッグ版では同じ処理を /dev/api からも呼べる。

use crate::config::{Appearance, SpeechConfig};
use crate::{layouts, speech, timestamp, App, Comment, Emote, PORT};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{key} の形式が違います: {e}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    twitch_channels: Vec<String>,
    twitch_badges: bool,
    threads: Vec<String>,
    poll_seconds: u64,
    initial_posts: usize,
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

        // ---- 接続
        "save_connection" => {
            let c: Connection = serde_json::from_value(args).map_err(|e| format!("形式が違います: {e}"))?;
            app.update_config(|cfg| {
                cfg.twitch_channels = c.twitch_channels.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                cfg.threads = c.threads.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                cfg.twitch_badges = c.twitch_badges;
                cfg.poll_seconds = c.poll_seconds.clamp(10, 300);
                cfg.initial_posts = c.initial_posts.min(1000);
            })?;
            app.status("接続設定を保存して、接続し直しました".into());
            app.apply_connections();
            Ok(json!({ "ok": true }))
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

        // ---- ワードNG（次に取得したレスから効く。接続はやり直さない）
        "save_ng" => {
            let words: Vec<String> = arg(&args, "ngWords")?;
            app.update_config(|cfg| cfg.ng_words = words.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())?;
            Ok(json!({ "ok": true }))
        }

        // ---- LiveFake 連携
        "whitelist_status" => Ok(serde_json::to_value(app.whitelist_status()).unwrap_or_default()),
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
