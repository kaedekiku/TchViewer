//! 文字列処理: 掲示板HTMLのプレーンテキスト化と、ワードNG判定

use fancy_regex::Regex;
use std::collections::HashMap;
use std::sync::Mutex;

/// `<br>` を改行に、その他のタグを除去し、文字参照を戻す
pub fn html_to_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        match rest[lt..].find('>') {
            Some(gt) => {
                let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
                if tag == "br" || tag.starts_with("br ") || tag.starts_with("br/") {
                    out.push('\n');
                }
                rest = &rest[lt + gt + 1..];
            }
            None => {
                out.push_str(&rest[lt..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let decoded = decode_entities(&out);
    // 5ch の本文は「 <br> 」のように改行の前後に空白が入る
    decoded
        .split('\n')
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .map(|l| l.strip_suffix(' ').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let end = after.find(';').filter(|&e| e > 0 && e <= 10);
        let decoded = end.and_then(|e| {
            let ent = &after[..e];
            let ch = if let Some(num) = ent.strip_prefix('#') {
                let n = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                n.and_then(char::from_u32)
            } else {
                match ent.to_ascii_lowercase().as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                }
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- ワードNG
// LiveFakeTauri2 (App.tsx ngMatch) と同じ規則:
//   `/…/` で囲むと正規表現（大小無視）、それ以外は大小無視の部分一致。
//   正規表現は512文字まで。書式エラーは「一致しない」扱い。

const MAX_USER_REGEX_LEN: usize = 512;

static REGEX_CACHE: Mutex<Option<HashMap<String, Option<Regex>>>> = Mutex::new(None);

fn cached_regex(source: &str) -> Option<Regex> {
    let mut guard = REGEX_CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    if cache.len() >= 1000 {
        cache.clear();
    }
    cache
        .entry(source.to_string())
        .or_insert_with(|| Regex::new(&format!("(?i){source}")).ok())
        .clone()
}

pub fn ng_match(pattern: &str, target: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() {
        return false;
    }
    if p.len() > 2 && p.starts_with('/') && p.ends_with('/') {
        if p.len() > MAX_USER_REGEX_LEN {
            return false;
        }
        return cached_regex(&p[1..p.len() - 1])
            .map(|re| re.is_match(target).unwrap_or(false))
            .unwrap_or(false);
    }
    target.to_lowercase().contains(&p.to_lowercase())
}

/// NGワードのどれかに一致したら、そのワードを返す
pub fn ng_hit<'a>(words: &'a [String], target: &str) -> Option<&'a str> {
    words.iter().find(|w| ng_match(w, target)).map(|w| w.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html() {
        assert_eq!(html_to_text(" あ <br> い&amp;う &#128512; "), "あ\nい&う 😀");
        assert_eq!(html_to_text("<a href=\"x\">&gt;&gt;1</a> 了解"), ">>1 了解");
        assert_eq!(html_to_text("a & b &unknown;"), "a & b &unknown;");
    }

    #[test]
    fn ng() {
        assert!(ng_match("ABC", "xxabcxx"));
        assert!(!ng_match("abd", "xxabcxx"));
        assert!(ng_match("/^死.+ね$/", "死んでね"));
        assert!(ng_match("/(?<=>>)\\d+/", ">>12"));
        assert!(!ng_match("/[/", "["));
        assert!(!ng_match("  ", "abc"));
    }
}
