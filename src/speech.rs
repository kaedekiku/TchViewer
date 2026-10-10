//! 読み上げ: 棒読みちゃん（TCP 50001）/ SAPI（Windows 標準）/ VOICEVOX（HTTP API）
//!
//! 読み上げ文の作り方は LiveFakeTauri2 (App.tsx ttsSpeak) に準拠:
//!   サイト別の前置き「レスN番さん / したらばN番さん / ジャパンくんN番さん」、
//!   URLは読まない（YouTubeは「ユーチューブ」）、1001以降のシステムレスは読まない、
//!   長文は「、長文のため以下省略」

use crate::config::{SpeechConfig, VoicevoxConfig};
use crate::Comment;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub fn speech_text(c: &Comment, sp: &SpeechConfig) -> String {
    let mut body = c.text.clone();
    if c.source == "twitch" && !c.emotes.is_empty() {
        // エモートは読まない（位置はコードポイント単位）
        let mut cps: Vec<char> = body.chars().collect();
        let mut emotes = c.emotes.clone();
        emotes.sort_by_key(|e| std::cmp::Reverse(e.start));
        for e in emotes {
            if e.start <= e.end && e.end < cps.len() {
                cps.drain(e.start..=e.end);
            }
        }
        body = cps.into_iter().collect();
    }
    let words: Vec<String> = body
        .split_whitespace()
        .map(|w| {
            let lower = w.to_ascii_lowercase();
            let url_at = ["https://", "http://", "ttps://", "ttp://"]
                .iter()
                .filter_map(|p| lower.find(p))
                .min();
            match url_at {
                Some(i) => {
                    let reading = if lower[i..].contains("youtu") { " ユーチューブ" } else { "" };
                    format!("{}{}", &w[..i], reading)
                }
                None => w.to_string(),
            }
        })
        .filter(|w| !w.trim().is_empty())
        .collect();
    let mut body = words.join(" ").trim().to_string();
    if body.is_empty() {
        return String::new();
    }
    if sp.max_length > 0 && body.chars().count() > sp.max_length {
        body = body.chars().take(sp.max_length).collect::<String>() + "、長文のため以下省略";
    }
    let prefix = match (c.source.as_str(), c.no) {
        ("twitch", _) if sp.twitch_name => format!("{}さん", c.name),
        ("twitch", _) => String::new(),
        ("shitaraba", Some(n)) => format!("したらば{n}番さん"),
        ("jpnkn", Some(n)) => format!("ジャパンくん{n}番さん"),
        (_, Some(n)) => format!("レス{n}番さん"),
        _ => String::new(),
    };
    if prefix.is_empty() { body } else { format!("{prefix} {body}") }
}

/// 読み上げ対象なら読み上げ文を返す
pub fn should_speak(c: &Comment, sp: &SpeechConfig) -> Option<String> {
    if !sp.enabled || c.initial || !sp.sources.get(&c.source).copied().unwrap_or(false) {
        return None;
    }
    if c.no.is_some_and(|n| n >= 1001) {
        return None;
    }
    let text: String = speech_text(c, sp).replace('\0', "").chars().take(2000).collect();
    (!text.is_empty()).then_some(text)
}

/// 設定されたエンジンで1件読み上げる。SAPI / VOICEVOX は読み終わるまで待つ（順番を守るため）
pub async fn speak(sp: &SpeechConfig, client: &reqwest::Client, text: &str) -> Result<(), String> {
    match sp.engine.as_str() {
        "sapi" => {
            let (text, voice, rate, volume) = (text.to_string(), sp.sapi.voice.clone(), sp.sapi.rate, sp.sapi.volume);
            tokio::task::spawn_blocking(move || crate::tts_win::sapi_speak(&text, &voice, rate, volume))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("SAPI: {e}"))
        }
        "voicevox" => {
            let wav = voicevox_synthesize(client, &sp.voicevox, text).await.map_err(|e| format!("VOICEVOX（{}）: {e}", sp.voicevox.endpoint))?;
            tokio::task::spawn_blocking(move || crate::tts_win::play_wav(&wav)).await.map_err(|e| e.to_string())?
        }
        _ => send_bouyomi(&sp.host, sp.port, text)
            .await
            .map_err(|e| format!("棒読みちゃんに接続できません（{}:{}）: {e}", sp.host, sp.port)),
    }
}

/// VOICEVOX の http://127.0.0.1:50021 のような、ローカルの http アドレスだけを受け付ける
fn voicevox_base(endpoint: &str) -> Result<String, String> {
    let e = endpoint.trim().trim_end_matches('/');
    let rest = e.strip_prefix("http://").ok_or("http:// で始まるアドレスを指定してください")?;
    let host = rest.split([':', '/']).next().unwrap_or("");
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err("VOICEVOX はこのPC（127.0.0.1 / localhost）のアドレスを指定してください".into());
    }
    Ok(e.to_string())
}

/// 話者の一覧（GET /speakers）を「キャラクター名（スタイル名）」と ID の組で返す
pub async fn voicevox_speakers(client: &reqwest::Client, endpoint: &str) -> Result<Vec<(i64, String)>, String> {
    let base = voicevox_base(endpoint)?;
    let res = client.get(format!("{base}/speakers")).timeout(std::time::Duration::from_secs(10)).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let v: serde_json::Value = serde_json::from_slice(&res.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut out = vec![];
    for sp in v.as_array().into_iter().flatten() {
        let name = sp.get("name").and_then(|n| n.as_str()).unwrap_or("");
        for st in sp.get("styles").and_then(|s| s.as_array()).into_iter().flatten() {
            if let Some(id) = st.get("id").and_then(|i| i.as_i64()) {
                let style = st.get("name").and_then(|n| n.as_str()).unwrap_or("");
                out.push((id, format!("{name}（{style}）")));
            }
        }
    }
    Ok(out)
}

/// audio_query → synthesis で WAV を作る
async fn voicevox_synthesize(client: &reqwest::Client, vv: &VoicevoxConfig, text: &str) -> Result<Vec<u8>, String> {
    let base = voicevox_base(&vv.endpoint)?;
    let speaker = vv.speaker.to_string();
    let res = client
        .post(format!("{base}/audio_query"))
        .query(&[("text", text), ("speaker", speaker.as_str())])
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("audio_query HTTP {}", res.status().as_u16()));
    }
    let mut query: serde_json::Value = serde_json::from_slice(&res.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(o) = query.as_object_mut() {
        o.insert("speedScale".into(), vv.speed.clamp(0.5, 2.0).into());
        o.insert("pitchScale".into(), vv.pitch.clamp(-0.15, 0.15).into());
        o.insert("intonationScale".into(), vv.intonation.clamp(0.0, 2.0).into());
        o.insert("volumeScale".into(), vv.volume.clamp(0.0, 2.0).into());
    }
    let res = client
        .post(format!("{base}/synthesis"))
        .query(&[("speaker", speaker.as_str())])
        .header("Content-Type", "application/json")
        .body(query.to_string())
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("synthesis HTTP {}", res.status().as_u16()));
    }
    Ok(res.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

pub async fn send_bouyomi(host: &str, port: u16, text: &str) -> std::io::Result<()> {
    // コマンド0x0001(読み上げ) / 速度・音程・音量 -1=棒読みちゃん側の設定 / 声質0=既定 / 文字コード0=UTF-8
    let body = text.as_bytes();
    let mut packet = Vec::with_capacity(15 + body.len());
    packet.extend_from_slice(&1i16.to_le_bytes());
    packet.extend_from_slice(&(-1i16).to_le_bytes());
    packet.extend_from_slice(&(-1i16).to_le_bytes());
    packet.extend_from_slice(&(-1i16).to_le_bytes());
    packet.extend_from_slice(&0i16.to_le_bytes());
    packet.push(0u8);
    packet.extend_from_slice(&(body.len() as i32).to_le_bytes());
    packet.extend_from_slice(body);
    let mut sock = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect((host, port)))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))??;
    sock.write_all(&packet).await?;
    sock.shutdown().await
}
