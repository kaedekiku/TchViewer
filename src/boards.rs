//! 5ch / JPNKN（dat差分取得）と したらば（rawmode差分取得）

use crate::text::{html_to_text, ng_hit};
use crate::{App, Comment};
use encoding_rs::{EUC_JP, SHIFT_JIS};
use reqwest::header::{RANGE, USER_AGENT};
use reqwest::StatusCode;
use std::sync::Arc;

const UA_DAT: &str = "Monazilla/1.00 (TchViewer)";
const UA_BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) TchViewer";

#[derive(Clone, Debug)]
pub enum ThreadInfo {
    /// 5ch / JPNKN（5ch互換）
    Dat { site: &'static str, source: &'static str, host: String, board: String, key: String },
    Shitaraba { dir: String, bbs: String, key: String },
}

impl ThreadInfo {
    pub fn site(&self) -> &'static str {
        match self {
            ThreadInfo::Dat { site, .. } => site,
            ThreadInfo::Shitaraba { .. } => "したらば",
        }
    }
    pub fn source(&self) -> &'static str {
        match self {
            ThreadInfo::Dat { source, .. } => source,
            ThreadInfo::Shitaraba { .. } => "shitaraba",
        }
    }
    pub fn label(&self) -> String {
        match self {
            ThreadInfo::Dat { board, key, .. } => format!("{board}/{key}"),
            ThreadInfo::Shitaraba { dir, bbs, key } => format!("{dir}/{bbs}/{key}"),
        }
    }
}

/// read.cgi 形式のスレッドURLを解析する
pub fn parse_thread_url(raw: &str) -> Result<ThreadInfo, String> {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .ok_or("http(s):// で始まるURLを指定してください")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let segs: Vec<&str> = path.split(['/', '?', '#']).filter(|s| !s.is_empty()).collect();
    let i = segs.iter().position(|s| *s == "read.cgi").ok_or("read.cgi 形式のスレッドURLを指定してください")?;
    let args = &segs[i + 1..];
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

    if host == "jbbs.shitaraba.net" || host == "jbbs.livedoor.jp" {
        if args.len() < 3 || !args[..3].iter().all(|s| valid(s)) {
            return Err("したらばの read.cgi URL（/bbs/read.cgi/カテゴリ/番号/スレ番号/）を指定してください".into());
        }
        return Ok(ThreadInfo::Shitaraba { dir: args[0].into(), bbs: args[1].into(), key: args[2].into() });
    }
    if args.len() < 2 || !valid(args[0]) || !args[1].chars().all(|c| c.is_ascii_digit()) {
        return Err("スレッドURL（/test/read.cgi/板/スレ番号/）を指定してください".into());
    }
    let (site, source, host) = if host == "bbs.jpnkn.com" {
        ("JPNKN", "jpnkn", host)
    } else if host.ends_with(".5ch.net") || host.ends_with(".5ch.io") {
        // 5ch は 5ch.io へ移行済み
        ("5ch", "5ch", host.replace(".5ch.net", ".5ch.io"))
    } else {
        return Err(format!("未対応のサイトです: {host}"));
    };
    Ok(ThreadInfo::Dat { site, source, host, board: args[0].into(), key: args[1].into() })
}

/// 日付欄からIDを取り出す。LiveFake と同じく正規表現 `/ID:(\S+)/` の最初の一致
/// （`ID:` の後ろから次の空白の手前まで。末尾の 0 や P などもIDの一部として含む）
pub fn extract_id(date_field: &str) -> String {
    let mut rest = date_field;
    while let Some(i) = rest.find("ID:") {
        let after = &rest[i + 3..];
        let id: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
        if !id.is_empty() {
            return id;
        }
        rest = after;
    }
    String::new()
}

pub struct ThreadWatcher {
    pub info: ThreadInfo,
    /// dat: 改行まで読み終えたバイト数
    bytes: u64,
    res_no: u32,
    first: bool,
    title: String,
    full_notified: bool,
}

#[derive(Clone, Debug)]
pub struct Post {
    pub no: u32,
    pub name: String,
    pub uid: String,
    pub text: String,
}


impl ThreadWatcher {
    pub fn new(info: ThreadInfo) -> Self {
        Self { info, bytes: 0, res_no: 0, first: true, title: String::new(), full_notified: false }
    }

    /// 「状態」ページでの識別子
    pub fn key(&self) -> String {
        format!("{} {}", self.info.site(), self.info.label())
    }

    pub async fn poll(&mut self, app: &Arc<App>, client: &reqwest::Client) -> Result<(), String> {
        let posts = match self.info.clone() {
            ThreadInfo::Dat { host, board, key, .. } => self.poll_dat(client, &host, &board, &key).await?,
            ThreadInfo::Shitaraba { dir, bbs, key } => self.poll_shitaraba(client, &dir, &bbs, &key).await?,
        };
        let (title, res_no) = (self.title.clone(), self.res_no);
        app.stat(&self.key(), |s| {
            s.state = "ok".into();
            s.last_fetch = crate::timestamp();
            s.last_error.clear();
            s.title = title;
            s.res_count = res_no;
        });
        self.deliver(app, posts);
        Ok(())
    }

    async fn poll_dat(&mut self, client: &reqwest::Client, host: &str, board: &str, key: &str) -> Result<Vec<Post>, String> {
        let url = format!("https://{host}/{board}/dat/{key}.dat");
        let mut req = client.get(&url).header(USER_AGENT, UA_DAT);
        if self.bytes > 0 {
            req = req.header(RANGE, format!("bytes={}-", self.bytes));
        }
        let res = req.send().await.map_err(|e| e.to_string())?;
        let status = res.status();
        if status == StatusCode::RANGE_NOT_SATISFIABLE || status == StatusCode::NOT_MODIFIED {
            return Ok(vec![]);
        }
        if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        let buf = res.bytes().await.map_err(|e| e.to_string())?;
        let mut data: &[u8] = &buf;
        if status == StatusCode::OK && self.bytes > 0 {
            // Rangeが無視され全体が返ってきた → 既読位置以降だけ使う
            if (buf.len() as u64) >= self.bytes {
                data = &buf[self.bytes as usize..];
            }
        }
        let Some(last_nl) = data.iter().rposition(|&b| b == b'\n') else { return Ok(vec![]) };
        // 途中の行は次回取り直す
        self.bytes += last_nl as u64 + 1;
        // 5ch・JPNKN の dat は Shift_JIS
        let (text, _, _) = SHIFT_JIS.decode(&data[..last_nl]);
        let mut posts = vec![];
        for line in text.split('\n').filter(|l| !l.is_empty()) {
            // 名前<>メール<>日付 ID<>本文<>スレタイ
            let f: Vec<&str> = line.split("<>").collect();
            self.res_no += 1;
            if let Some(t) = f.get(4).filter(|t| !t.is_empty()) {
                if self.title.is_empty() {
                    self.title = html_to_text(t);
                }
            }
            let uid = extract_id(f.get(2).copied().unwrap_or(""));
            posts.push(Post {
                no: self.res_no,
                name: html_to_text(f.first().copied().unwrap_or("")),
                uid,
                text: html_to_text(f.get(3).copied().unwrap_or("")),
            });
        }
        Ok(posts)
    }

    async fn poll_shitaraba(&mut self, client: &reqwest::Client, dir: &str, bbs: &str, key: &str) -> Result<Vec<Post>, String> {
        let from = if self.first { 1 } else { self.res_no + 1 };
        let url = format!("https://jbbs.shitaraba.net/bbs/rawmode.cgi/{dir}/{bbs}/{key}/{from}-");
        let res = client.get(&url).header(USER_AGENT, UA_BROWSER).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status().as_u16()));
        }
        let buf = res.bytes().await.map_err(|e| e.to_string())?;
        let (text, _, _) = EUC_JP.decode(&buf);
        let mut posts = vec![];
        for line in text.split('\n').filter(|l| !l.is_empty()) {
            // 番号<>名前<>メール<>日付<>本文<>スレタイ<>ID
            let f: Vec<&str> = line.split("<>").collect();
            let Some(no) = f.first().and_then(|s| s.trim().parse::<u32>().ok()) else { continue };
            if no <= self.res_no {
                continue;
            }
            if let Some(t) = f.get(5).filter(|t| !t.is_empty()) {
                if self.title.is_empty() {
                    self.title = html_to_text(t);
                }
            }
            self.res_no = no;
            posts.push(Post {
                no,
                name: html_to_text(f.get(1).copied().unwrap_or("")),
                uid: f.get(6).copied().unwrap_or("").trim().to_string(),
                text: html_to_text(f.get(4).copied().unwrap_or("")),
            });
        }
        Ok(posts)
    }

    fn deliver(&mut self, app: &Arc<App>, posts: Vec<Post>) {
        let (source, site) = match &self.info {
            ThreadInfo::Dat { source, site, .. } => (*source, *site),
            ThreadInfo::Shitaraba { .. } => ("shitaraba", "したらば"),
        };
        let mut initial = false;
        let mut posts = posts;
        if self.first {
            self.first = false;
            initial = true;
            let keep = app.config().initial_posts.min(posts.len());
            posts.drain(..posts.len() - keep);
            let title = if self.title.is_empty() { self.info.label() } else { self.title.clone() };
            app.status(format!("{site} 接続: {title}（{}レス）", self.res_no));
        }
        let ng_words = app.config().ng_words;
        let whitelist = app.whitelist();
        let key = self.key();
        for p in posts {
            if !initial {
                app.stat(&key, |s| s.received += 1);
            }
            // ホワイトリスト（LiveFake連携）のIDだけ通す → ワードNGで除外
            if !whitelist.allows(&p.uid) {
                if !initial {
                    app.stat(&key, |s| s.blocked_whitelist += 1);
                }
                continue;
            }
            if let Some(w) = ng_hit(&ng_words, &p.text) {
                app.log(format!("NG({w}) で除外: {site} {}", p.no));
                if !initial {
                    app.stat(&key, |s| s.blocked_ng += 1);
                }
                continue;
            }
            if !initial {
                app.stat(&key, |s| s.passed += 1);
            }
            app.emit_comment(Comment {
                source: source.into(),
                site: site.into(),
                thread: self.title.clone(),
                no: Some(p.no),
                name: p.name,
                uid: p.uid,
                text: p.text,
                initial,
                ..Default::default()
            });
        }
        if source != "shitaraba" && self.res_no >= 1000 && !self.full_notified {
            self.full_notified = true;
            app.status(format!("{site} スレが埋まりました: {}", self.title));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        match parse_thread_url("https://mi.5ch.net/test/read.cgi/news4vip/1791512525/l50").unwrap() {
            ThreadInfo::Dat { host, board, key, site, .. } => {
                assert_eq!((host.as_str(), board.as_str(), key.as_str(), site), ("mi.5ch.io", "news4vip", "1791512525", "5ch"));
            }
            _ => panic!(),
        }
        assert!(matches!(
            parse_thread_url("https://bbs.jpnkn.com/test/read.cgi/mamiko/1790771579/").unwrap(),
            ThreadInfo::Dat { site: "JPNKN", .. }
        ));
        assert!(matches!(
            parse_thread_url("https://jbbs.shitaraba.net/bbs/read.cgi/internet/20196/1780204030/").unwrap(),
            ThreadInfo::Shitaraba { .. }
        ));
        assert!(parse_thread_url("https://example.com/test/read.cgi/a/1/").is_err());
        assert_eq!(extract_id("2026/10/10(土) 13:52:03.221 ID:TXInyECp01010"), "TXInyECp01010");
        assert_eq!(extract_id("2026/10/10(土) 13:52:03.221 ID: x ID:abc0 BE:1"), "abc0");
        assert_eq!(extract_id("2026/08/09(日) 03:23:35.45"), "");
        assert!(parse_thread_url("https://mi.5ch.io/test/read.cgi/../1/").is_err());
    }
}
