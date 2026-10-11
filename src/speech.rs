//! 読み上げ: 棒読みちゃん（TCP 50001）/ SAPI（Windows 標準）/ VOICEVOX（HTTP API）
//!
//! 読み上げ文の作り方は LiveFakeTauri2 (App.tsx ttsSpeak) に準拠:
//!   サイト別の前置き「レスN番さん / したらばN番さん / ジャパンくんN番さん」、
//!   URLは読まない（掲示板の YouTube は「ユーチューブ」、Twitch のチャットは「URL省略」）、1001以降のシステムレスは読まない、
//!   長文は「、以下省略」

use crate::config::{SpeechConfig, VoicevoxConfig};
use crate::Comment;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

/// 読み上げの順番待ちの1件
#[derive(Clone, Debug)]
pub struct SpeechItem {
    pub text: String,
    /// 画面で「今読んでいるもの」を示すための印（"c:コメントID" / "u:スレ:番号"）
    pub key: Option<String>,
    /// 文ではなく通知音を鳴らす
    pub chime: bool,
}

impl SpeechItem {
    pub fn text(text: String, key: Option<String>) -> Self {
        Self { text, key, chime: false }
    }
    pub fn chime(key: String) -> Self {
        Self { text: String::new(), key: Some(key), chime: true }
    }
}

/// 番号だけの読み上げ文（「レス123番さん」/「したらば123番さん」/「ジャパンくん123番さん」）
pub fn number_text(source: &str, no: u32) -> String {
    match source {
        "shitaraba" => format!("したらば{no}番さん"),
        "jpnkn" => format!("ジャパンくん{no}番さん"),
        _ => format!("レス{no}番さん"),
    }
}

/// 通知音（短い「ポン」）の WAV をその場で作る（音声ファイルを同梱しない）
pub fn chime_wav() -> Vec<u8> {
    const RATE: u32 = 22050;
    let n = RATE as usize * 18 / 100; // 0.18秒
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let env = (1.0 - i as f32 / n as f32).powi(2); // だんだん小さく
        let v = ((t * 1046.5 * std::f32::consts::TAU).sin() * 0.6 + (t * 1568.0 * std::f32::consts::TAU).sin() * 0.25) * env * 0.35;
        pcm.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
    }
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // モノラル
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(&pcm);
    w
}

pub fn speech_text(c: &Comment, sp: &SpeechConfig) -> String {
    let mut body = c.text.clone();
    if c.source == "twitch" && !c.emotes.is_empty() {
        // スタンプは名前のまま送る（棒読みちゃんの教育で読み替えられる）。同じスタンプは emote_max 個まで。
        // 「読まない」なら全部取り除く（位置はコードポイント単位）
        let limit = if sp.emote_mode == "none" { 0 } else { sp.emote_max.clamp(1, 100) as usize };
        let mut emotes = c.emotes.clone();
        emotes.sort_by_key(|e| e.start);
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        let mut drop = vec![];
        for e in emotes {
            let n = seen.entry(e.id.clone()).or_default();
            *n += 1;
            if *n > limit {
                drop.push(e);
            }
        }
        let mut cps: Vec<char> = body.chars().collect();
        for e in drop.iter().rev() {
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
                    let reading = if c.source == "twitch" {
                        " URL省略"
                    } else if lower[i..].contains("youtu") {
                        " ユーチューブ"
                    } else {
                        ""
                    };
                    format!("{}{}", &w[..i], reading).trim().to_string()
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
        body = body.chars().take(sp.max_length).collect::<String>() + "、以下省略";
    }
    let prefix = match (c.source.as_str(), c.no) {
        ("twitch", _) if sp.twitch_name => format!("{}さん", c.name),
        ("twitch", _) => String::new(),
        (src, Some(n)) => number_text(src, n),
        _ => String::new(),
    };
    if prefix.is_empty() { body } else { format!("{prefix} {body}") }
}

/// URL を読まない・空白をまとめる・最大文字数で省略する（イベントの読み上げ文用）
pub fn clean_for_speech(text: &str, sp: &SpeechConfig) -> String {
    let c = Comment { source: "twitch".into(), text: text.into(), ..Default::default() };
    let sp = SpeechConfig { twitch_name: false, ..sp.clone() };
    speech_text(&c, &sp).replace('\0', "").chars().take(2000).collect()
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
        _ => send_bouyomi(&sp.host, sp.port, text, sp.bouyomi.params())
            .await
            .map_err(|e| format!("棒読みちゃんに接続できません（{}:{}）: {e}", sp.host, sp.port)),
    }
}

/// 棒読みちゃんの読み上げの命令
/// コマンド0x0001(読み上げ) / 速度・音程・音量（-1=棒読みちゃん側の設定）/ 声質（0=既定）/ 文字コード0=UTF-8 / 長さ / 本文
fn bouyomi_packet(text: &str, params: [i16; 4]) -> Vec<u8> {
    let body = text.as_bytes();
    let mut packet = Vec::with_capacity(15 + body.len());
    packet.extend_from_slice(&1i16.to_le_bytes());
    for v in params {
        packet.extend_from_slice(&v.to_le_bytes());
    }
    packet.push(0u8);
    packet.extend_from_slice(&(body.len() as i32).to_le_bytes());
    packet.extend_from_slice(body);
    packet
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

pub async fn send_bouyomi(host: &str, port: u16, text: &str, params: [i16; 4]) -> std::io::Result<()> {
    let packet = bouyomi_packet(text, params);
    let mut sock = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect((host, port)))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))??;
    sock.write_all(&packet).await?;
    sock.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BouyomiConfig;

    fn emote_comment(text: &str, emotes: &[(&str, usize, usize)]) -> Comment {
        Comment {
            source: "twitch".into(),
            name: "a".into(),
            text: text.into(),
            emotes: emotes.iter().map(|&(id, start, end)| crate::Emote { id: id.into(), start, end }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn urls_and_long_text() {
        let sp = SpeechConfig { twitch_name: false, ..Default::default() };
        // Twitch のチャットの URL は「URL省略」
        let c = Comment { source: "twitch".into(), text: "見て https://example.com/a これ".into(), ..Default::default() };
        assert_eq!(speech_text(&c, &sp), "見て URL省略 これ");
        let c = Comment { source: "twitch".into(), text: "https://youtu.be/x".into(), ..Default::default() };
        assert_eq!(speech_text(&c, &sp), "URL省略");
        // 掲示板は今までどおり（読まない、YouTube は「ユーチューブ」）
        let c = Comment { source: "5ch".into(), text: "見て https://example.com/a これ ttps://youtu.be/x".into(), ..Default::default() };
        assert_eq!(speech_text(&c, &sp), "見て これ ユーチューブ");
        // 長文
        let short = SpeechConfig { max_length: 3, ..sp.clone() };
        let c = Comment { source: "twitch".into(), text: "あいうえお".into(), ..Default::default() };
        assert_eq!(speech_text(&c, &short), "あいう、以下省略");
    }

    #[test]
    fn emotes_sent_and_limited() {
        let sp = SpeechConfig { twitch_name: false, ..Default::default() };
        // 既定: スタンプ名をそのまま送る（同じスタンプは3個まで）
        let c = emote_comment("こんにちは kaede1Hi", &[("1", 6, 13)]);
        assert_eq!(speech_text(&c, &sp), "こんにちは kaede1Hi");
        let c = emote_comment("Kappa Kappa Kappa Kappa LUL", &[("25", 0, 4), ("25", 6, 10), ("25", 12, 16), ("25", 18, 22), ("9", 24, 26)]);
        assert_eq!(speech_text(&c, &sp), "Kappa Kappa Kappa LUL");
        let one = SpeechConfig { emote_max: 1, ..sp.clone() };
        assert_eq!(speech_text(&c, &one), "Kappa LUL");
        // 範囲外の値は 1〜100 に収める
        let zero = SpeechConfig { emote_max: 0, ..sp.clone() };
        assert_eq!(speech_text(&c, &zero), "Kappa LUL");
        // 読まない
        let none = SpeechConfig { emote_mode: "none".into(), ..sp.clone() };
        assert_eq!(speech_text(&c, &none), "");
        // スタンプだけのコメントも名前つきで読む
        let named = SpeechConfig { twitch_name: true, ..Default::default() };
        let c = emote_comment("kaede1Hi", &[("1", 0, 7)]);
        assert_eq!(speech_text(&c, &named), "aさん kaede1Hi");
    }

    #[test]
    fn bouyomi_params() {
        // 既定は棒読みちゃん側の設定（-1）と既定の声（0）
        assert_eq!(BouyomiConfig::default().params(), [-1, -1, -1, 0]);
        let c = BouyomiConfig { set_speed: true, speed: 999, set_tone: true, tone: 10, set_volume: true, volume: 50, voice: 3 };
        assert_eq!(c.params(), [300, 50, 50, 3]);
        let c = BouyomiConfig { voice: 9, ..Default::default() };
        assert_eq!(c.params()[3], 0);
        let c = BouyomiConfig { voice: 10002, ..Default::default() };
        assert_eq!(c.params()[3], 10002);
    }

    #[test]
    fn bouyomi_packet_layout() {
        let p = bouyomi_packet("あ", [120, -1, 80, 2]);
        assert_eq!(&p[..10], &[1, 0, 120, 0, 0xff, 0xff, 80, 0, 2, 0]);
        assert_eq!(p[10], 0);
        assert_eq!(&p[11..15], &3i32.to_le_bytes());
        assert_eq!(&p[15..], "あ".as_bytes());
    }
}
