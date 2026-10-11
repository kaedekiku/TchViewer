//! Twitch のイベント（サブスク・ギフト・ビッツ・レイド・連続視聴・チャンネルポイントなど）
//!
//! - ログインなし: チャットの流れ（IRC）の USERNOTICE と、PRIVMSG の bits / custom-reward-id タグから作る
//! - ログインあり: EventSub から、IRC に流れないもの（入力の無い報酬・自動報酬・フォロー）を作る（eventsub.rs）
//! - 種類ごとに「チャット欄に流す」「読み上げる」「読み上げる文」を設定できる（config.twitch_events）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// イベントの種類と、既定の設定（流す, 読む, 読み上げる文）
pub const KINDS: [(&str, &str, bool, bool, &str); 11] = [
    ("sub", "サブスク", true, true, "{name}さんがサブスクしました {message}"),
    ("resub", "継続サブスク", true, true, "{name}さんがサブスクしました（{months}か月目） {message}"),
    ("subgift", "サブスクギフト", true, true, "{name}さんが{recipient}さんにサブスクをギフト"),
    ("submysterygift", "まとめてギフト", true, true, "{name}さんがサブスクを{count}個ギフト"),
    ("giftpaidupgrade", "ギフトの継続", true, true, "{name}さんがギフトのサブスクを継続"),
    ("bits", "ビッツ", true, true, "{name}さんが{bits}ビッツ {message}"),
    ("raid", "レイド", true, true, "{name}さんが{viewers}人でレイド"),
    ("watchstreak", "連続視聴", true, true, "{name}さんが{count}回連続視聴 {message}"),
    ("points", "チャンネルポイント", true, true, "{name}さんが「{reward}」を引き換え {message}"),
    ("follow", "フォロー", true, false, "{name}さんがフォロー"),
    ("announcement", "アナウンス", true, true, "{message}"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventSetting {
    /// ビュワー・OBS に流す
    pub show: bool,
    /// 読み上げる（全体の「新着コメントを読み上げる」がオンのとき）
    pub speak: bool,
    /// 読み上げる文（{name} {months} {count} {bits} {viewers} {recipient} {reward} {message}）
    pub template: String,
}

impl Default for EventSetting {
    fn default() -> Self {
        Self { show: true, speak: true, template: String::new() }
    }
}

pub fn default_settings() -> HashMap<String, EventSetting> {
    KINDS.iter().map(|(k, _, show, speak, tpl)| (k.to_string(), EventSetting { show: *show, speak: *speak, template: tpl.to_string() })).collect()
}

/// 足りない種類を既定値で補う（設定ファイルが古いとき）
pub fn fill_defaults(map: &mut HashMap<String, EventSetting>) {
    for (k, v) in default_settings() {
        let e = map.entry(k).or_insert_with(|| v.clone());
        if e.template.trim().is_empty() {
            e.template = v.template;
        }
    }
}

pub fn label(kind: &str) -> &'static str {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("イベント")
}

/// イベントの中身（ビュワー・OBS・読み上げ用）
#[derive(Clone, Debug, Default, Serialize)]
pub struct TwitchEvent {
    pub kind: String,
    pub label: String,
    /// 読み上げる文の {message} を空にした要約（表示用）
    pub summary: String,
    pub months: u32,
    pub count: u32,
    pub bits: u32,
    pub viewers: u32,
    pub recipient: String,
    pub reward: String,
}

impl TwitchEvent {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.into(), label: label(kind).into(), ..Default::default() }
    }

    /// 読み上げる文（テンプレートに差し込む）
    pub fn render(&self, template: &str, name: &str, message: &str) -> String {
        let recipient = if self.recipient.is_empty() { "誰か" } else { &self.recipient };
        template
            .replace("{name}", name)
            .replace("{months}", &self.months.to_string())
            .replace("{count}", &self.count.to_string())
            .replace("{bits}", &self.bits.to_string())
            .replace("{viewers}", &self.viewers.to_string())
            .replace("{recipient}", recipient)
            .replace("{reward}", &self.reward)
            .replace("{message}", message)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// ビッツの応援文から「Cheer100」のような応援スタンプの文字を除く（読み上げ用）
pub fn strip_cheermotes(text: &str) -> String {
    text.split_whitespace()
        .filter(|w| {
            let letters = w.trim_end_matches(|c: char| c.is_ascii_digit());
            !(letters.len() < w.len() && !letters.is_empty() && letters.chars().all(|c| c.is_ascii_alphabetic()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// IRC の USERNOTICE をイベントにする。対象外の種類なら None
pub fn from_usernotice(tags: &HashMap<String, String>) -> Option<TwitchEvent> {
    let p = |k: &str| tags.get(&format!("msg-param-{k}")).cloned().unwrap_or_default();
    let n = |k: &str| p(k).parse::<u32>().unwrap_or(0);
    let msg_id = tags.get("msg-id").map(String::as_str).unwrap_or("");
    let mut e = match msg_id {
        "sub" => {
            let mut e = TwitchEvent::new("sub");
            e.months = n("cumulative-months").max(1);
            e
        }
        "resub" => {
            let mut e = TwitchEvent::new("resub");
            e.months = n("cumulative-months").max(1);
            e
        }
        "subgift" | "anonsubgift" => {
            let mut e = TwitchEvent::new("subgift");
            e.recipient = p("recipient-display-name");
            if e.recipient.is_empty() {
                e.recipient = p("recipient-user-name");
            }
            e.months = n("months");
            e
        }
        "submysterygift" | "anonsubmysterygift" => {
            let mut e = TwitchEvent::new("submysterygift");
            e.count = n("mass-gift-count").max(1);
            e
        }
        "giftpaidupgrade" | "anongiftpaidupgrade" | "primepaidupgrade" => TwitchEvent::new("giftpaidupgrade"),
        "raid" => {
            let mut e = TwitchEvent::new("raid");
            e.viewers = n("viewerCount");
            e
        }
        "viewermilestone" if p("category") == "watch-streak" => {
            let mut e = TwitchEvent::new("watchstreak");
            e.count = n("value");
            e
        }
        "announcement" => TwitchEvent::new("announcement"),
        _ => return None,
    };
    e.label = label(&e.kind).into();
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn usernotice() {
        let e = from_usernotice(&tags(&[("msg-id", "resub"), ("msg-param-cumulative-months", "12")])).unwrap();
        assert_eq!((e.kind.as_str(), e.months), ("resub", 12));
        let e = from_usernotice(&tags(&[("msg-id", "subgift"), ("msg-param-recipient-display-name", "たろう")])).unwrap();
        assert_eq!(e.render("{name}さんが{recipient}さんにギフト", "はなこ", ""), "はなこさんがたろうさんにギフト");
        let e = from_usernotice(&tags(&[("msg-id", "viewermilestone"), ("msg-param-category", "watch-streak"), ("msg-param-value", "5")])).unwrap();
        assert_eq!(e.render("{name}さんが{count}回連続視聴 {message}", "a", ""), "aさんが5回連続視聴");
        assert!(from_usernotice(&tags(&[("msg-id", "ritual")])).is_none());
        assert_eq!(strip_cheermotes("Cheer100 ありがとう cheer50 abc12x"), "ありがとう abc12x");
    }
}
