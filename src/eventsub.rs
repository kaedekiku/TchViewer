//! Twitch ログイン（デバイスコード方式）と EventSub（WebSocket）
//!
//! チャットの流れ（IRC）に届かないもの（入力の無いチャンネルポイント報酬・自動報酬・フォロー）を受け取る。
//! - ログイン: 公開型クライアントのデバイスコード方式（クライアントシークレットは使わない）
//!   求める許可は読み取りだけ: channel:read:redemptions（報酬の引き換え）/ moderator:read:followers（フォロー）
//! - ログイン情報は Windows の DPAPI で暗号化して data/twitch-auth.bin に保存（同じ PC・同じ Windows ユーザーでだけ復号できる）。
//!   画面側には渡さず、ログにも書かない
//! - 接続先は eventsub.wss.twitch.tv だけ。接続先の変更指示も同じホストのときだけ従う
//! - 通知は message_id で重複を除き、10分より古い通知は捨てる

use crate::events::TwitchEvent;
use crate::{App, Comment};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

/// TchViewer の公開型クライアントID（秘密の値ではない）
pub const CLIENT_ID: &str = "5lz40jm835v729hntzd1cza97hakch";
const SCOPES: &str = "channel:read:redemptions moderator:read:followers";
const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: String,
    /// アクセストークンの期限（UNIX 秒）
    pub expires_at: u64,
    pub user_id: String,
    pub login: String,
}

/// 画面に出す状態（トークンは含めない）
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub logged_in: bool,
    pub login: String,
    /// "none" / "waiting"（コード入力待ち）/ "connecting" / "connected" / "error"
    pub state: String,
    pub message: String,
    pub user_code: String,
    pub verification_uri: String,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn auth_path() -> std::path::PathBuf {
    crate::config::data_dir().join("twitch-auth.bin")
}

// ---------------------------------------------------------------- DPAPI（保存時の暗号化）

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    const ENTROPY: &[u8] = b"TchViewer/twitch-auth";
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        if protect {
            CryptProtectData(&input, windows::core::PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).is_ok()
        } else {
            CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).is_ok()
        }
    };
    if !ok || out.pbData.is_null() {
        return None;
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Some(v)
}

#[cfg(not(windows))]
fn dpapi(_data: &[u8], _protect: bool) -> Option<Vec<u8>> {
    None
}

pub fn load_auth() -> Option<AuthData> {
    let enc = std::fs::read(auth_path()).ok()?;
    let plain = dpapi(&enc, false)?;
    serde_json::from_slice(&plain).ok()
}

fn save_auth(a: &AuthData) -> Result<(), String> {
    let plain = serde_json::to_vec(a).map_err(|e| e.to_string())?;
    let enc = dpapi(&plain, true).ok_or("ログイン情報を暗号化できません")?;
    std::fs::create_dir_all(crate::config::data_dir()).map_err(|e| e.to_string())?;
    let tmp = auth_path().with_extension("bin.tmp");
    std::fs::write(&tmp, enc).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, auth_path()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- トークン

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

async fn validate(http: &reqwest::Client, token: &str) -> Result<(String, String), String> {
    let r = http.get("https://id.twitch.tv/oauth2/validate").header("Authorization", format!("OAuth {token}")).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("ログインを確認できません（HTTP {}）", r.status().as_u16()));
    }
    let v: Value = serde_json::from_slice(&r.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let id = v.get("user_id").and_then(|x| x.as_str()).ok_or("ユーザーIDを取得できません")?.to_string();
    let login = v.get("login").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok((id, login))
}

async fn refresh(http: &reqwest::Client, a: &AuthData) -> Result<AuthData, String> {
    let r = http
        .post("https://id.twitch.tv/oauth2/token")
        .form(&[("client_id", CLIENT_ID), ("grant_type", "refresh_token"), ("refresh_token", a.refresh_token.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("ログインの有効期限が切れました。もう一度ログインしてください".into());
    }
    let t: TokenResponse = serde_json::from_slice(&r.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(AuthData { access_token: t.access_token, refresh_token: t.refresh_token, expires_at: now() + t.expires_in, ..a.clone() })
}

impl App {
    pub fn auth_status(&self) -> AuthStatus {
        self.auth_status.lock().unwrap().clone()
    }

    fn set_auth_status(&self, f: impl FnOnce(&mut AuthStatus)) {
        let st = {
            let mut s = self.auth_status.lock().unwrap();
            f(&mut s);
            s.clone()
        };
        self.ui_event(json!({ "type": "twitch_auth", "status": st }));
    }

    /// ログイン中のチャンネルのイベントを EventSub で受け取れているか（チャット側の報酬表示を止める判定）
    pub fn eventsub_covers(&self, channel: &str) -> bool {
        self.eventsub_connected.load(Ordering::SeqCst) && self.auth_status.lock().unwrap().login.eq_ignore_ascii_case(channel)
    }

    /// 起動時: 保存済みのログイン情報があれば EventSub を始める
    pub fn start_saved_login(self: &Arc<Self>) {
        if let Some(a) = load_auth() {
            *self.auth.lock().unwrap() = Some(a.clone());
            self.set_auth_status(|s| {
                s.logged_in = true;
                s.login = a.login.clone();
                s.state = "connecting".into();
            });
            self.restart_eventsub();
        }
    }

    /// ログインを始める（コードを表示し、利用者が許可するまで待つ）
    pub async fn start_login(self: &Arc<Self>) -> Result<AuthStatus, String> {
        let r = self
            .http
            .post("https://id.twitch.tv/oauth2/device")
            .form(&[("client_id", CLIENT_ID), ("scopes", SCOPES)])
            .send()
            .await
            .map_err(|e| format!("Twitch に接続できません: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("ログインを始められません（HTTP {}）", r.status().as_u16()));
        }
        let v: Value = serde_json::from_slice(&r.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let device_code = v.get("device_code").and_then(|x| x.as_str()).ok_or("応答の形式が違います")?.to_string();
        let user_code = v.get("user_code").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let uri = v.get("verification_uri").and_then(|x| x.as_str()).unwrap_or("").to_string();
        // 開くのは Twitch の確認ページだけ（偽のページへ誘導されないように）
        if !uri.starts_with("https://www.twitch.tv/activate") {
            return Err("想定外のログインページが返されたため中止しました".into());
        }
        let interval = v.get("interval").and_then(|x| x.as_u64()).unwrap_or(5).clamp(1, 30);
        let expires = v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(1800).min(1800);
        self.set_auth_status(|s| {
            s.state = "waiting".into();
            s.message = "ブラウザで Twitch にログインし、表示されたコードを入力して許可してください".into();
            s.user_code = user_code.clone();
            s.verification_uri = uri.clone();
        });
        let app = self.clone();
        let task = tokio::spawn(async move { app.poll_login(device_code, interval, expires).await });
        if let Some(old) = self.login_task.lock().unwrap().replace(task) {
            old.abort();
        }
        Ok(self.auth_status())
    }

    async fn poll_login(self: Arc<Self>, device_code: String, mut interval: u64, expires: u64) {
        let deadline = now() + expires;
        while now() < deadline {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            let r = self
                .http
                .post("https://id.twitch.tv/oauth2/token")
                .form(&[("client_id", CLIENT_ID), ("scopes", SCOPES), ("device_code", device_code.as_str()), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])
                .send()
                .await;
            let Ok(r) = r else { continue };
            let ok = r.status().is_success();
            let body = r.bytes().await.unwrap_or_default();
            if !ok {
                let msg = String::from_utf8_lossy(&body);
                if msg.contains("authorization_pending") {
                    continue;
                }
                if msg.contains("slow_down") {
                    interval += 5;
                    continue;
                }
                self.set_auth_status(|s| {
                    s.state = "error".into();
                    s.message = "ログインが取り消されたか、時間切れになりました".into();
                    s.user_code.clear();
                });
                return;
            }
            let Ok(t) = serde_json::from_slice::<TokenResponse>(&body) else { return };
            match validate(&self.http, &t.access_token).await {
                Ok((user_id, login)) => {
                    let a = AuthData { access_token: t.access_token, refresh_token: t.refresh_token, expires_at: now() + t.expires_in, user_id, login: login.clone() };
                    if let Err(e) = save_auth(&a) {
                        self.status(format!("ログイン情報を保存できません: {e}"));
                    }
                    *self.auth.lock().unwrap() = Some(a);
                    self.set_auth_status(|s| {
                        s.logged_in = true;
                        s.login = login.clone();
                        s.state = "connecting".into();
                        s.message.clear();
                        s.user_code.clear();
                    });
                    self.status(format!("Twitch にログインしました: {login}"));
                    self.restart_eventsub();
                }
                Err(e) => self.set_auth_status(|s| {
                    s.state = "error".into();
                    s.message = e;
                }),
            }
            return;
        }
        self.set_auth_status(|s| {
            s.state = "error".into();
            s.message = "時間切れになりました。もう一度ログインしてください".into();
            s.user_code.clear();
        });
    }

    /// ログアウト（Twitch 側の許可も取り消し、保存した情報を消す）
    pub async fn logout(self: &Arc<Self>) {
        if let Some(t) = self.login_task.lock().unwrap().take() {
            t.abort();
        }
        if let Some(t) = self.eventsub_task.lock().unwrap().take() {
            t.abort();
        }
        self.eventsub_connected.store(false, Ordering::SeqCst);
        let a = self.auth.lock().unwrap().take();
        if let Some(a) = a {
            let _ = self.http.post("https://id.twitch.tv/oauth2/revoke").form(&[("client_id", CLIENT_ID), ("token", a.access_token.as_str())]).send().await;
        }
        let _ = std::fs::remove_file(auth_path());
        self.set_auth_status(|s| *s = AuthStatus { state: "none".into(), ..Default::default() });
        self.status("Twitch からログアウトしました".into());
    }

    fn restart_eventsub(self: &Arc<Self>) {
        let app = self.clone();
        let task = tokio::spawn(async move { app.eventsub_loop().await });
        if let Some(old) = self.eventsub_task.lock().unwrap().replace(task) {
            old.abort();
        }
    }

    /// 期限が近ければ更新したトークンを返す
    async fn valid_token(self: &Arc<Self>) -> Result<AuthData, String> {
        let a = self.auth.lock().unwrap().clone().ok_or("ログインしていません")?;
        if a.expires_at > now() + 300 {
            return Ok(a);
        }
        let fresh = refresh(&self.http, &a).await?;
        save_auth(&fresh)?;
        *self.auth.lock().unwrap() = Some(fresh.clone());
        Ok(fresh)
    }

    async fn eventsub_loop(self: Arc<Self>) {
        let mut url = EVENTSUB_URL.to_string();
        loop {
            match self.eventsub_session(&url).await {
                Ok(Some(next)) => {
                    url = next; // Twitch からの接続先変更（同じホストのみ）
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    self.eventsub_connected.store(false, Ordering::SeqCst);
                    let fatal = e.contains("ログイン");
                    self.set_auth_status(|s| {
                        s.state = "error".into();
                        s.message = e.clone();
                    });
                    self.status(format!("Twitch イベント（EventSub）: {e}"));
                    if fatal {
                        return;
                    }
                }
            }
            url = EVENTSUB_URL.to_string();
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    /// 1回分の接続。接続先の変更指示があれば Some(新しい URL)
    async fn eventsub_session(self: &Arc<Self>, url: &str) -> Result<Option<String>, String> {
        let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| format!("接続できません: {e}"))?;
        let (mut tx, mut rx) = ws.split();
        let mut keepalive = 30u64;
        let mut seen: VecDeque<String> = VecDeque::new();
        loop {
            let msg = match tokio::time::timeout(Duration::from_secs(keepalive + 15), rx.next()).await {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => return Err(format!("切断されました: {e}")),
                Ok(None) => return Err("切断されました".into()),
                Err(_) => return Err("応答がありません".into()),
            };
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Ping(p) => {
                    let _ = tx.send(Message::Pong(p)).await;
                    continue;
                }
                Message::Close(_) => return Err("切断されました".into()),
                _ => continue,
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            let meta = &v["metadata"];
            let id = meta["message_id"].as_str().unwrap_or("").to_string();
            if !id.is_empty() {
                if seen.contains(&id) {
                    continue;
                }
                seen.push_back(id);
                while seen.len() > 300 {
                    seen.pop_front();
                }
            }
            // 10分より古い通知は捨てる（送り直し対策）
            if let Some(ts) = meta["message_timestamp"].as_str().and_then(parse_rfc3339) {
                if ts + 600 < now() {
                    continue;
                }
            }
            match meta["message_type"].as_str().unwrap_or("") {
                "session_welcome" => {
                    keepalive = v["payload"]["session"]["keepalive_timeout_seconds"].as_u64().unwrap_or(30).clamp(10, 600);
                    let session = v["payload"]["session"]["id"].as_str().unwrap_or("").to_string();
                    self.subscribe_all(&session).await?;
                    self.eventsub_connected.store(true, Ordering::SeqCst);
                    self.set_auth_status(|s| {
                        s.state = "connected".into();
                        s.message.clear();
                    });
                    self.status("Twitch イベント（EventSub）に接続しました".into());
                }
                "session_reconnect" => {
                    let next = v["payload"]["session"]["reconnect_url"].as_str().unwrap_or("");
                    // 接続先の変更は Twitch の EventSub のホストのときだけ従う
                    if next.starts_with("wss://eventsub.wss.twitch.tv/") {
                        return Ok(Some(next.to_string()));
                    }
                    return Err("想定外の接続先が指示されたため接続し直します".into());
                }
                "notification" => self.on_notification(&v["payload"]),
                "revocation" => {
                    let t = v["payload"]["subscription"]["type"].as_str().unwrap_or("");
                    self.status(format!("Twitch イベントの受け取りが取り消されました: {t}"));
                }
                _ => {} // session_keepalive など
            }
        }
    }

    async fn subscribe_all(self: &Arc<Self>, session: &str) -> Result<(), String> {
        let a = self.valid_token().await?;
        let uid = a.user_id.as_str();
        let subs = [
            ("channel.channel_points_custom_reward_redemption.add", "1", json!({ "broadcaster_user_id": uid })),
            ("channel.channel_points_automatic_reward_redemption.add", "2", json!({ "broadcaster_user_id": uid })),
            ("channel.follow", "2", json!({ "broadcaster_user_id": uid, "moderator_user_id": uid })),
        ];
        for (ty, ver, cond) in subs {
            let body = json!({ "type": ty, "version": ver, "condition": cond, "transport": { "method": "websocket", "session_id": session } });
            let r = self
                .http
                .post("https://api.twitch.tv/helix/eventsub/subscriptions")
                .header("Client-Id", CLIENT_ID)
                .header("Authorization", format!("Bearer {}", a.access_token))
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if r.status().as_u16() == 401 {
                return Err("ログインの有効期限が切れました。もう一度ログインしてください".into());
            }
            if !r.status().is_success() {
                self.status(format!("Twitch イベント {ty} を受け取れません（HTTP {}）", r.status().as_u16()));
            }
        }
        Ok(())
    }

    fn on_notification(&self, p: &Value) {
        let ty = p["subscription"]["type"].as_str().unwrap_or("");
        let e = &p["event"];
        let name = e["user_name"].as_str().or(e["user_login"].as_str()).unwrap_or("").to_string();
        let channel = e["broadcaster_user_login"].as_str().unwrap_or("").to_string();
        let (ev, text) = match ty {
            "channel.channel_points_custom_reward_redemption.add" => {
                let mut ev = TwitchEvent::new("points");
                ev.reward = e["reward"]["title"].as_str().unwrap_or("").chars().take(100).collect();
                (ev, e["user_input"].as_str().unwrap_or("").to_string())
            }
            "channel.channel_points_automatic_reward_redemption.add" => {
                let mut ev = TwitchEvent::new("points");
                let kind = e["reward"]["type"].as_str().unwrap_or("");
                ev.reward = automatic_reward_name(kind);
                (ev, e["message"]["text"].as_str().unwrap_or("").to_string())
            }
            "channel.follow" => (TwitchEvent::new("follow"), String::new()),
            _ => return,
        };
        self.emit_comment(Comment {
            source: "twitch".into(),
            site: "Twitch".into(),
            thread: format!("#{channel}"),
            name,
            text,
            event: Some(ev),
            ..Default::default()
        });
    }
}

/// 自動報酬の種類 → 表示名（種類の名前は Twitch 側で増えることがある。知らないものはそのまま出す）
fn automatic_reward_name(kind: &str) -> String {
    match kind {
        "single_message_bypass_sub_mode" => "サブスク限定モード中にメッセージを送信",
        "send_highlighted_message" => "メッセージをハイライト",
        "random_sub_emote_unlock" => "ランダムなスタンプを解除",
        "chosen_sub_emote_unlock" => "スタンプを選んで解除",
        "chosen_modified_sub_emote_unlock" => "加工済みスタンプを解除",
        "message_effect" => "メッセージエフェクト",
        "gigantify_an_emote" => "スタンプを巨大化",
        "celebration" => "画面上のセレブレーション",
        other => return other.to_string(),
    }
    .to_string()
}

/// "2026-10-11T04:12:34.123456Z" → UNIX 秒（時刻ライブラリを増やさないための最小限の解析）
fn parse_rfc3339(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // days_from_civil（Howard Hinnant）
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86400 + hh * 3600 + mm * 60 + ss).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-11T00:00:00.123Z"), Some(1_791_676_800));
        assert!(parse_rfc3339("x").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let enc = dpapi(b"secret", true).unwrap();
        assert_ne!(enc, b"secret");
        assert_eq!(dpapi(&enc, false).unwrap(), b"secret");
    }
}
