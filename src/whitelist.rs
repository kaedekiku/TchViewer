//! LiveFake連携（TCVホワイトリスト）
//!
//! LiveFake が書き出す `broadcast-whitelist.json` を読み取り専用で監視し、
//! 載っているIDのレスだけを OBS 表示・読み上げに通す。
//!   - 1秒ごとに更新日時とサイズを見て、変わっていたら全部読んですぐ閉じる
//!     （std::fs::read は共有モード READ|WRITE|DELETE で開くので、LiveFake の置き換え保存を妨げない）
//!   - 読めない・壊れている・形式が違うときは、直前に正しく読めた一覧を使い続ける
//!   - ファイルが無い状態が続いたら空の一覧にする
//!   - `date` が日本時間の今日でなければ空の一覧として扱う（0時を過ぎたら自動で無効）

use crate::App;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub const FILE_NAME: &str = "broadcast-whitelist.json";
const MAX_FILE_SIZE: u64 = 1024 * 1024;
const MAX_IDS: usize = 1000;
const MAX_ID_LEN: usize = 32;
/// ファイルが無い状態がこの回数（=秒）続いたら空の一覧にする
const MISSING_LIMIT: u32 = 3;

#[derive(Clone, Debug, Default)]
pub struct Whitelist {
    /// 直前に正しく読めた内容
    pub date: String,
    pub ids: HashSet<String>,
}

impl Whitelist {
    /// 日本時間の今日の日付と一致するときだけ有効
    pub fn allows(&self, uid: &str) -> bool {
        !uid.is_empty() && self.date == jst_today() && self.ids.contains(uid)
    }
    pub fn active_count(&self) -> usize {
        if self.date == jst_today() { self.ids.len() } else { 0 }
    }
}

/// 設定画面に出す状態
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhitelistStatus {
    pub path: String,
    pub state: String,
    pub date: String,
    pub today: String,
    pub active: bool,
    pub ids: Vec<String>,
}

/// 設定値（dataフォルダ or ファイルそのもの）から読むファイルのパスを決める
pub fn resolve_path(setting: &str) -> Option<PathBuf> {
    let s = setting.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    let p = PathBuf::from(s);
    if p.is_dir() || !s.to_ascii_lowercase().ends_with(".json") {
        Some(p.join(FILE_NAME))
    } else {
        Some(p)
    }
}

/// 中身を検証して取り込む。形式が違えば Err
pub fn parse(bytes: &[u8]) -> Result<Whitelist, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "UTF-8ではありません")?;
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("JSONとして読めません: {e}"))?;
    let date = v.get("date").and_then(|d| d.as_str()).ok_or("date がありません")?;
    if !is_ymd(date) {
        return Err("date が YYYY-MM-DD 形式ではありません".into());
    }
    let arr = v.get("ids").and_then(|i| i.as_array()).ok_or("ids が配列ではありません")?;
    let ids = arr
        .iter()
        .filter_map(|x| x.as_str())
        .filter(|s| (1..=MAX_ID_LEN).contains(&s.chars().count()) && !s.chars().any(char::is_whitespace))
        .take(MAX_IDS)
        .map(String::from)
        .collect();
    Ok(Whitelist { date: date.to_string(), ids })
}

fn is_ymd(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// 日本時間(UTC+9)の今日の日付 YYYY-MM-DD（PCのタイムゾーンに関係なく）
pub fn jst_today() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    ymd_from_days(((secs + 9 * 3600) / 86400) as i64)
}

/// 1970-01-01 からの日数 → YYYY-MM-DD（Howard Hinnant の civil_from_days）
fn ymd_from_days(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

enum ReadResult {
    Ok(Whitelist),
    Missing,
    Bad(String),
}

fn read_once(path: &PathBuf) -> ReadResult {
    match std::fs::metadata(path) {
        Err(_) => return ReadResult::Missing,
        Ok(m) if m.len() > MAX_FILE_SIZE => return ReadResult::Bad(format!("ファイルが大きすぎます（{}バイト）", m.len())),
        Ok(_) => {}
    }
    // 一度に読んで閉じる
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() as u64 > MAX_FILE_SIZE => ReadResult::Bad("ファイルが大きすぎます".into()),
        Ok(bytes) => match parse(&bytes) {
            Ok(w) => ReadResult::Ok(w),
            Err(e) => ReadResult::Bad(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ReadResult::Missing,
        Err(e) => ReadResult::Bad(e.to_string()),
    }
}

/// 監視ループ（設定が変わるたびに作り直される）
pub async fn watch(app: Arc<App>, setting: String) {
    let Some(path) = resolve_path(&setting) else {
        app.set_whitelist(Whitelist::default(), "未設定（LiveFakeのdataフォルダを指定してください）".into(), String::new());
        app.status("ホワイトリスト: 未設定のため、掲示板のレスは表示しません".into());
        return;
    };
    let shown = path.display().to_string();
    app.status(format!("ホワイトリスト: 監視開始 {shown}"));
    let mut last_sig: Option<(Option<SystemTime>, u64)> = None;
    let mut missing = 0u32;
    let mut last_active: Option<(String, usize)> = None;
    let mut last_error = String::new();
    loop {
        let sig = std::fs::metadata(&path).ok().map(|m| (m.modified().ok(), m.len()));
        if sig.is_none() {
            last_sig = None; // 再び現れたら必ず読み直す
            missing += 1;
            if missing == MISSING_LIMIT {
                app.set_whitelist(Whitelist::default(), "ファイルがありません（LiveFakeでTCVホワイトリストがオンか確認してください）".into(), shown.clone());
            }
        } else if sig != last_sig {
            missing = 0;
            // 置き換えの一瞬や書き込み直後に当たったら、少し待って読み直す
            let mut result = read_once(&path);
            if !matches!(result, ReadResult::Ok(_)) {
                tokio::time::sleep(Duration::from_millis(300)).await;
                result = read_once(&path);
            }
            match result {
                ReadResult::Ok(w) => {
                    last_sig = sig;
                    last_error.clear();
                    app.set_whitelist(w, "読み込み済み".into(), shown.clone());
                }
                ReadResult::Missing => {} // 次の周回で数える
                ReadResult::Bad(e) => {
                    // 直前の一覧を使い続ける。同じファイルを何度も読まないよう署名は進める
                    last_sig = sig;
                    if e != last_error {
                        app.status(format!("ホワイトリストを読めません（直前の一覧を使います）: {e}"));
                        last_error = e.clone();
                    }
                    app.set_whitelist_state(format!("読めません（直前の一覧を使用中）: {e}"));
                }
            }
        } else {
            missing = 0;
        }
        // 有効件数の変化（読み込み・0時の切り替え）を知らせる
        let (today, count) = (jst_today(), app.whitelist().active_count());
        if last_active.as_ref() != Some(&(today.clone(), count)) {
            app.status(format!("ホワイトリスト: 有効なID {count}件（{today}）"));
            last_active = Some((today, count));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(ymd_from_days(0), "1970-01-01");
        assert_eq!(ymd_from_days(20_736), "2026-10-10");
        assert_eq!(ymd_from_days(11_016), "2000-02-29");
        assert!(is_ymd("2026-10-10"));
        assert!(!is_ymd("2026/10/10"));
        assert!(!is_ymd("2026-1-10"));
    }

    #[test]
    fn parsing() {
        let w = parse("{\n  \"date\": \"2026-10-10\",\n  \"ids\": [\"AbCdEfGh0\", \"xYz12345\", 5, \"\", \"a b\", \"123456789012345678901234567890123\"]\n}".as_bytes()).unwrap();
        assert_eq!(w.ids.len(), 2);
        assert!(w.ids.contains("AbCdEfGh0"));
        assert!(parse(b"{\"date\":\"2026/10/10\",\"ids\":[]}").is_err());
        assert!(parse(b"{\"date\":\"2026-10-10\",\"ids\":\"x\"}").is_err());
        assert!(parse(b"{\"date\":\"2026-10-10\",\"ids\":[").is_err());
    }

    #[test]
    fn allows_only_today_exact() {
        let w = Whitelist { date: jst_today(), ids: ["AbC0".to_string()].into() };
        assert!(w.allows("AbC0"));
        assert!(!w.allows("abc0")); // 大文字小文字は区別
        assert!(!w.allows("AbC"));
        assert!(!w.allows(""));
        let old = Whitelist { date: "2000-01-01".into(), ids: ["AbC0".to_string()].into() };
        assert!(!old.allows("AbC0"));
    }
}
