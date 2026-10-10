//! Twitch チャット（匿名IRC over WebSocket。ログイン不要・読み取り専用）

use crate::{App, Comment, Emote};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// "ID" / "#ID" / "https://www.twitch.tv/ID" を受け付ける
pub fn normalize_channel(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    let s = s
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("twitch.tv/")
        .trim_start_matches('#');
    s.split(['/', '?']).next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
}

pub async fn run(app: Arc<App>, channel: String) {
    let key = format!("Twitch #{channel}");
    app.stat(&key, |s| {
        s.source = "twitch".into();
        s.site = "Twitch".into();
        s.label = format!("#{channel}");
        s.state = "connecting".into();
    });
    loop {
        let reason = match connect_once(&app, &channel, &key).await {
            Err(e) => format!("Twitch 切断: #{channel}（{e}）5秒後に再接続"),
            Ok(()) => format!("Twitch 切断: #{channel} 5秒後に再接続"),
        };
        app.stat(&key, |s| {
            s.state = "error".into();
            s.last_error = format!("[{}] {reason}", crate::timestamp());
        });
        app.status(reason);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn connect_once(app: &Arc<App>, channel: &str, key: &str) -> Result<(), String> {
    let (ws, _) = tokio_tungstenite::connect_async("wss://irc-ws.chat.twitch.tv:443").await.map_err(|e| e.to_string())?;
    let (mut tx, mut rx) = ws.split();
    let nick = format!("justinfan{}", 10000 + (std::process::id() as u64 * 7919 + now_ms()) % 80000);
    for line in [
        "CAP REQ :twitch.tv/tags twitch.tv/commands".to_string(),
        "PASS SCHMOOPIIE".to_string(),
        format!("NICK {nick}"),
        format!("JOIN #{channel}"),
    ] {
        tx.send(Message::text(line)).await.map_err(|e| e.to_string())?;
    }
    app.status(format!("Twitch 接続: #{channel}"));
    app.stat(key, |s| {
        s.state = "ok".into();
        s.last_error.clear();
    });
    while let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| e.to_string())?;
        let Message::Text(text) = msg else { continue };
        for line in text.split("\r\n").filter(|l| !l.is_empty()) {
            if line.starts_with("PING") {
                tx.send(Message::text("PONG :tmi.twitch.tv")).await.map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(mut c) = parse_privmsg(line, channel) {
                if app.config().twitch_badges {
                    c.badge_list = app.badges.read().unwrap().resolve(channel, &c.badges);
                }
                app.stat(key, |s| {
                    s.received += 1;
                    s.passed += 1;
                    s.last_fetch = crate::timestamp();
                });
                app.emit_comment(c);
            }
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn unescape_tag(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some(':') => out.push(';'),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

pub fn parse_privmsg(line: &str, channel: &str) -> Option<Comment> {
    let mut tags = HashMap::new();
    let mut rest = line;
    if let Some(r) = rest.strip_prefix('@') {
        let (t, r) = r.split_once(' ')?;
        for kv in t.split(';') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            tags.insert(k.to_string(), unescape_tag(v));
        }
        rest = r;
    }
    // :login!login@login.tmi.twitch.tv PRIVMSG #channel :text
    let rest = rest.strip_prefix(':')?;
    let (prefix, rest) = rest.split_once(' ')?;
    let login = prefix.split('!').next()?.to_string();
    let rest = rest.strip_prefix("PRIVMSG ")?;
    let (_, text) = rest.split_once(" :")?;
    let mut text = text.to_string();
    if let Some(a) = text.strip_prefix("\u{1}ACTION ").and_then(|t| t.strip_suffix('\u{1}')) {
        text = a.to_string();
    }
    let mut emotes = vec![];
    if let Some(spec) = tags.get("emotes").filter(|s| !s.is_empty()) {
        for part in spec.split('/') {
            let Some((id, ranges)) = part.split_once(':') else { continue };
            for r in ranges.split(',') {
                if let Some((s, e)) = r.split_once('-') {
                    if let (Ok(s), Ok(e)) = (s.parse(), e.parse()) {
                        emotes.push(Emote { id: id.to_string(), start: s, end: e });
                    }
                }
            }
        }
        emotes.sort_by_key(|e| e.start);
    }
    let name = tags.get("display-name").filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| login.clone());
    Some(Comment {
        source: "twitch".into(),
        site: "Twitch".into(),
        thread: format!("#{channel}"),
        name,
        login,
        color: tags.get("color").cloned().unwrap_or_default(),
        badges: tags.get("badges").cloned().unwrap_or_default(),
        text,
        // パワーアップでスタンプを巨大化したメッセージには msg-id=gigantified-emote-message が付く
        giant: !emotes.is_empty() && tags.get("msg-id").is_some_and(|v| v == "gigantified-emote-message"),
        emotes,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel() {
        assert_eq!(normalize_channel("https://www.twitch.tv/Some_One?x=1"), "some_one");
        assert_eq!(normalize_channel("#abc"), "abc");
    }

    #[test]
    fn privmsg() {
        let line = "@badges=subscriber/12;color=#FF0000;display-name=テスト;emotes=25:0-4,12-16 :foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :Kappa こんにちは Kappa";
        let c = parse_privmsg(line, "bar").unwrap();
        assert_eq!(c.name, "テスト");
        assert_eq!(c.login, "foo");
        assert_eq!(c.text, "Kappa こんにちは Kappa");
        assert_eq!(c.emotes.len(), 2);
        assert_eq!((c.emotes[1].start, c.emotes[1].end), (12, 16));
        assert!(!c.giant);
        let g = parse_privmsg("@display-name=a;emotes=25:0-4;msg-id=gigantified-emote-message :a!a@a.tmi.twitch.tv PRIVMSG #bar :Kappa", "bar").unwrap();
        assert!(g.giant);
        assert!(parse_privmsg(":tmi.twitch.tv 001 justinfan1 :Welcome", "bar").is_none());
    }
}
