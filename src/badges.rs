//! Twitch のバッジ画像（非公式の取得方法）
//!
//! Twitch のサイト自身が使っている GraphQL の取得口（gql.twitch.tv）から、グローバルバッジと
//! チャンネル独自のバッジ（サブスク・ビッツなど）の画像アドレスを取得する。
//! - 非公式のため、Twitch 側の変更で使えなくなることがある。設定で取得をオフにできる
//! - 取得は接続時と1時間ごと。結果はメモリに置き、data/badges-cache.json にも保存して
//!   取得に失敗したときの予備にする
//! - 受け取った画像アドレスは static-cdn.jtvnw.net/badges/v1/ のものだけ使う

use crate::App;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Twitch のウェブサイトが公開している Client-Id（非公式の利用）
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const IMAGE_PREFIX: &str = "https://static-cdn.jtvnw.net/badges/v1/";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BadgeImage {
    pub title: String,
    /// 画像アドレス（末尾のサイズ指定 /1 を除いたもの。表示側で /2 や /3 を付ける）
    pub url: String,
}

/// "setID/version" → 画像
type BadgeMap = HashMap<String, BadgeImage>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BadgeStore {
    pub global: BadgeMap,
    /// チャンネル名（小文字）→ そのチャンネル独自のバッジ
    pub channels: HashMap<String, BadgeMap>,
}

impl BadgeStore {
    /// IRC の badges タグ（例: "broadcaster/1,subscriber/12"）を画像に変換する。チャンネル独自のものを優先
    pub fn resolve(&self, channel: &str, tag: &str) -> Vec<BadgeImage> {
        let ch = self.channels.get(channel);
        tag.split(',')
            .filter(|b| !b.is_empty())
            .filter_map(|b| ch.and_then(|m| m.get(b)).or_else(|| self.global.get(b)).cloned())
            .collect()
    }
}

fn cache_path() -> std::path::PathBuf {
    crate::config::data_dir().join("badges-cache.json")
}

fn load_cache() -> BadgeStore {
    std::fs::read_to_string(cache_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_cache(store: &BadgeStore) {
    if let Ok(s) = serde_json::to_string(store) {
        let _ = std::fs::write(cache_path(), s);
    }
}

/// GraphQL の応答の配列を BadgeMap にする（形式・画像アドレスを確かめる）
fn to_map(v: Option<&Value>) -> BadgeMap {
    let mut m = BadgeMap::new();
    for b in v.and_then(|x| x.as_array()).into_iter().flatten() {
        let (Some(set), Some(ver), Some(url)) = (
            b.get("setID").and_then(|x| x.as_str()),
            b.get("version").and_then(|x| x.as_str()),
            b.get("imageURL").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let Some(rest) = url.strip_prefix(IMAGE_PREFIX) else { continue };
        // <uuid>/<size> の形だけ受け付ける
        let uuid = rest.split('/').next().unwrap_or("");
        if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            continue;
        }
        let title = b.get("title").and_then(|x| x.as_str()).unwrap_or("").chars().take(100).collect();
        m.insert(format!("{set}/{ver}"), BadgeImage { title, url: format!("{IMAGE_PREFIX}{uuid}") });
    }
    m
}

async fn fetch(client: &reqwest::Client, channels: &[String]) -> Result<BadgeStore, String> {
    let fields = "setID version title imageURL(size: NORMAL)";
    let users: String = channels
        .iter()
        .enumerate()
        .map(|(i, ch)| format!("u{i}: user(login: \"{ch}\") {{ broadcastBadges {{ {fields} }} }}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = format!("query {{ badges {{ {fields} }} {users} }}");
    let res = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", WEB_CLIENT_ID)
        .header("Content-Type", "application/json")
        .body(json!({ "query": query }).to_string())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let v: Value = serde_json::from_slice(&res.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let data = v.get("data").ok_or("応答の形式が変わった可能性があります")?;
    let global = to_map(data.get("badges"));
    if global.is_empty() {
        return Err("グローバルバッジを取得できません（応答の形式が変わった可能性があります）".into());
    }
    let mut store = BadgeStore { global, channels: HashMap::new() };
    for (i, ch) in channels.iter().enumerate() {
        store.channels.insert(ch.clone(), to_map(data.get(format!("u{i}")).and_then(|u| u.get("broadcastBadges"))));
    }
    Ok(store)
}

/// 取得を続けるタスク（接続をやり直すたびに作り直される）
pub async fn run(app: Arc<App>, channels: Vec<String>) {
    // チャンネル名はクエリに埋め込むので、英数字と _ だけに限る（normalize_channel 済みだが念のため）
    let channels: Vec<String> = channels.into_iter().filter(|c| !c.is_empty() && c.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')).collect();
    let client = reqwest::Client::new();
    loop {
        match fetch(&client, &channels).await {
            Ok(store) => {
                let n: usize = store.channels.values().map(|m| m.len()).sum();
                app.log(format!("Twitchバッジを取得: グローバル{}件 / チャンネル{}件", store.global.len(), n));
                save_cache(&store);
                *app.badges.write().unwrap() = store;
            }
            Err(e) => {
                let cache = load_cache();
                let note = if cache.global.is_empty() { "前回の一覧もありません" } else { "前回取得した一覧を使います" };
                app.status(format!("Twitchバッジを取得できません（{note}）: {e}"));
                if !cache.global.is_empty() {
                    *app.badges.write().unwrap() = cache;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_resolve() {
        let v = json!([
            { "setID": "subscriber", "version": "12", "title": "1年", "imageURL": "https://static-cdn.jtvnw.net/badges/v1/eca7fc07-0f88-42ba-bad5-1b7d4feeb1cd/1" },
            { "setID": "evil", "version": "1", "title": "x", "imageURL": "https://evil.example/badges/v1/abc/1" },
            { "setID": "bad", "version": "1", "title": "x", "imageURL": "https://static-cdn.jtvnw.net/badges/v1/<script>/1" }
        ]);
        let m = to_map(Some(&v));
        assert_eq!(m.len(), 1);
        assert_eq!(m["subscriber/12"].url, "https://static-cdn.jtvnw.net/badges/v1/eca7fc07-0f88-42ba-bad5-1b7d4feeb1cd");
        let mut store = BadgeStore::default();
        store.global.insert("broadcaster/1".into(), BadgeImage { title: "配信者".into(), url: "g".into() });
        store.channels.insert("foo".into(), m);
        let r = store.resolve("foo", "broadcaster/1,subscriber/12,unknown/1");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].title, "配信者");
    }
}
