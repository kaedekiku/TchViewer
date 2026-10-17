//! NG（LiveFakeTauri2 と同じ考え方）
//!
//! - 種類: ワード（本文）/ 名前 / ID（完全一致）
//! - 処理: "abone"（スレタブで「あぼーん」と表示）/ "hide"（スレタブでも出さない）。
//!   どちらも OBS には出さず、読み上げず、自動ホワイトリストの対象にもしない
//! - 範囲: "global"（全体）/ "thread"（そのスレだけ）
//! - ワード・名前は `/…/` で正規表現（大小無視）、それ以外は大小無視の部分一致

use crate::config::{NgConfig, NgEntry};
use crate::text::{has_invisible, ng_match};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct NgHit {
    pub mode: String,
    pub reason: String,
}

/// スレの識別子（"5ch news4vip/123" の形）。範囲指定の比較に使う
pub fn thread_key_of(url: &str) -> Option<String> {
    crate::boards::parse_thread_url(url).ok().map(|i| format!("{} {}", i.site(), i.label()))
}

fn in_scope(e: &NgEntry, thread_key: &str) -> bool {
    e.scope != "thread" || thread_key_of(&e.scope_url).is_some_and(|k| k == thread_key)
}

/// 一致した NG（透明あぼーんを優先）
pub fn check(ng: &NgConfig, thread_key: &str, name: &str, uid: &str, text: &str) -> Option<NgHit> {
    let mut best: Option<NgHit> = None;
    let mut consider = |e: &NgEntry, kind: &str| -> bool {
        let hit = NgHit { mode: if e.mode == "hide" { "hide".into() } else { "abone".into() }, reason: format!("NG{kind}: {}", e.value) };
        let is_hide = hit.mode == "hide";
        if best.as_ref().is_none_or(|b| b.mode != "hide") {
            best = Some(hit);
        }
        is_hide
    };
    for e in ng.words.iter().filter(|e| in_scope(e, thread_key)) {
        if ng_match(&e.value, text) && consider(e, "ワード") {
            return best;
        }
    }
    if matches!(ng.invisible.as_str(), "abone" | "hide") && (has_invisible(text) || has_invisible(name)) {
        let e = NgEntry { value: "見えない文字".into(), mode: ng.invisible.clone(), ..Default::default() };
        if consider(&e, "") {
            return best;
        }
    }
    for e in ng.names.iter().filter(|e| in_scope(e, thread_key)) {
        if ng_match(&e.value, name) && consider(e, "名前") {
            return best;
        }
    }
    if !uid.is_empty() {
        for e in ng.ids.iter().filter(|e| in_scope(e, thread_key)) {
            if e.value.trim() == uid && consider(e, " ID") {
                return best;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(v: &str, mode: &str, scope: &str, url: &str) -> NgEntry {
        NgEntry { value: v.into(), mode: mode.into(), scope: scope.into(), scope_url: url.into() }
    }

    #[test]
    fn matching() {
        let t = "https://mi.5ch.io/test/read.cgi/news4vip/111/";
        let other = "https://mi.5ch.io/test/read.cgi/news4vip/222/";
        let key = thread_key_of(t).unwrap();
        let ng = NgConfig {
            words: vec![entry("荒らし", "abone", "global", ""), entry("/^w+$/", "hide", "thread", other)],
            names: vec![entry("コテ", "abone", "global", "")],
            ids: vec![entry("AbC0", "hide", "global", "")],
            ..Default::default()
        };
        assert_eq!(check(&ng, &key, "名無し", "x", "荒らしです").unwrap().mode, "abone");
        assert!(check(&ng, &key, "名無し", "x", "www").is_none()); // 別スレ限定
        assert_eq!(check(&ng, &thread_key_of(other).unwrap(), "名無し", "x", "www").unwrap().mode, "hide");
        assert_eq!(check(&ng, &key, "コテハン", "x", "普通").unwrap().reason, "NG名前: コテ");
        assert_eq!(check(&ng, &key, "名無し", "AbC0", "荒らし").unwrap().mode, "hide"); // 透明を優先
        assert!(check(&ng, &key, "名無し", "abc0", "普通").is_none()); // ID は完全一致
    }

    #[test]
    fn invisible_chars() {
        let key = "5ch news4vip/111";
        let mut ng = NgConfig::default();
        // 既定（オフ）では当たらない
        assert!(check(&ng, key, "名無し", "x", "あ\u{200B}い").is_none());
        ng.invisible = "abone".into();
        assert_eq!(check(&ng, key, "名無し", "x", "あ\u{200B}い").unwrap().mode, "abone");
        assert!(check(&ng, key, "名無し", "x", "abc\u{202E}gpj.exe").is_some());
        assert!(check(&ng, key, "名\u{2060}無し", "x", "普通").is_some()); // 名前も見る
        // 絵文字に使われる文字は対象外
        assert!(check(&ng, key, "名無し", "x", "👨\u{200D}👩\u{200D}👧").is_none());
        assert!(check(&ng, key, "名無し", "x", "\u{2764}\u{FE0F}").is_none());
        // dat の数値文字参照で届いても、本文に直してから判定する
        let text = crate::text::html_to_text("あ&#8203;い &#x202e;う");
        assert_eq!(check(&ng, key, "名無し", "x", &text).unwrap().reason, "NG: 見えない文字");
        // 透明あぼーんが通常より優先
        ng.invisible = "hide".into();
        ng.words.push(entry("あ", "abone", "global", ""));
        assert_eq!(check(&ng, key, "名無し", "x", "あ\u{200B}").unwrap().mode, "hide");
    }
}
