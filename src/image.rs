//! 画像ウィンドウ用の画像の判定・取得・保存
//!
//! - 判定は LiveFakeTauri2 と同じ: 拡張子（jpg/jpeg/png/gif/webp/bmp）＋ data/ImageViewURLReplace.txt の読み替え規則
//!   （1行1規則: `正規表現<TAB>置き換え後のURL[<TAB>リファラ]`。# で始まる行は無視。最初に一致した規則を使う）
//! - 取得は本体が行い、画面へはデータとして渡す（画面から外部へ直接つながない）
//!   - http / https のみ。ユーザー名・パスワード付きの URL は拒否
//!   - 接続先の全アドレスが公開アドレスであることを確かめてから、そのアドレスに固定して接続する（家庭内ネットワークなどへの接続を拒否）
//!   - リダイレクトは5回まで、その都度同じ確認をする
//!   - サイズは設定の上限（既定 20MB・最大 100MB）まで。中身が画像でなければ表示しない

use base64::Engine;
use fancy_regex::Regex;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug)]
pub struct Rule {
    pattern: String,
    replacement: String,
    referer: Option<String>,
}

fn rules_path() -> PathBuf {
    crate::config::data_dir().join("ImageViewURLReplace.txt")
}

/// 読み替え規則を読む（無ければ説明だけの空ファイルを作る）
pub fn load_rules() -> Vec<Rule> {
    let p = rules_path();
    if !p.exists() {
        let _ = std::fs::create_dir_all(crate::config::data_dir());
        let _ = std::fs::write(
            &p,
            "# 画像として開く URL の読み替え規則（LiveFake の ImageViewURLReplace.txt と同じ書式）\n\
             # 1行1規則: 正規表現<TAB>置き換え後のURL<TAB>リファラ（省略可）\n\
             # 例: https?://imgur\\.com/(\\w+)$\thttps://i.imgur.com/$1.jpg\n",
        );
    }
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let parts: Vec<&str> = l.splitn(3, '\t').collect();
            (parts.len() >= 2 && parts[0].len() <= 512).then(|| Rule {
                pattern: parts[0].trim().to_string(),
                replacement: parts[1].trim().to_string(),
                referer: parts.get(2).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            })
        })
        .collect()
}

/// 画像として開く URL なら (取得する URL, リファラ) を返す
pub fn resolve(url: &str, rules: &[Rule]) -> Option<(String, Option<String>)> {
    for r in rules {
        let Ok(re) = Regex::new(&r.pattern) else { continue };
        if re.is_match(url).unwrap_or(false) {
            return Some((re.replace(url, r.replacement.as_str()).into_owned(), r.referer.clone()));
        }
    }
    let lower = url.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or("");
    let ext = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"].iter().any(|e| path.ends_with(e));
    // X（旧Twitter）の画像は拡張子が付かない
    let twimg = lower.starts_with("https://pbs.twimg.com/media/");
    (ext || twimg).then(|| (url.to_string(), None))
}

// ---------------------------------------------------------------- 接続先の確認

fn ipv4_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF
        || (o[0] == 198 && (18..=19).contains(&o[1])) // ベンチマーク
        || o[0] >= 240)
}

fn ipv6_public(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_public(v4);
    }
    let s = ip.segments();
    // NAT64 (64:ff9b::/96) と 6to4 (2002::/16) は中の IPv4 で判定
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return ipv4_public(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
    }
    if s[0] == 0x2002 {
        return ipv4_public(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // ULA
        || (s[0] & 0xffc0) == 0xfe80 // リンクローカル
        || (s[0] == 0x2001 && s[1] == 0x0db8) // ドキュメント用
        || s[0..6] == [0, 0, 0, 0, 0, 0]) // IPv4 互換 (::a.b.c.d)
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_public(v4),
        IpAddr::V6(v6) => ipv6_public(v6),
    }
}

async fn check_target(u: &Url) -> Result<Vec<SocketAddr>, String> {
    if !matches!(u.scheme(), "http" | "https") {
        return Err("http / https 以外の URL は開けません".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("ユーザー名・パスワード付きの URL は開けません".into());
    }
    let host = u.host_str().ok_or("URL にホスト名がありません")?;
    let port = u.port_or_known_default().ok_or("ポートが分かりません")?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port)).await.map_err(|e| format!("接続先が見つかりません: {e}"))?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public_ip(a.ip())) {
        return Err("このPCや家庭内ネットワークのアドレスには接続できません".into());
    }
    Ok(addrs)
}

/// 中身が画像か（Content-Type と先頭のバイトで判定）
fn image_mime(content_type: &str, head: &[u8]) -> Option<String> {
    let sniff = if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else if head.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    };
    // 先頭のバイトで判定できたものを優先（Content-Type の偽装に惑わされない）
    sniff.map(String::from).or_else(|| {
        let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        (ct.starts_with("image/") && ct != "image/svg+xml").then_some(ct)
    })
}

pub struct Fetched {
    pub url: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub async fn fetch(url: &str, referer: Option<&str>, max_bytes: usize) -> Result<Fetched, String> {
    let mut current = Url::parse(url).map_err(|_| "URL の形式が正しくありません".to_string())?;
    for _ in 0..=MAX_REDIRECTS {
        let addrs = check_target(&current).await?;
        let host = current.host_str().unwrap_or("").to_string();
        // 確かめたアドレスに固定して接続する（確認後に別のアドレスへ向け直される手口への対策）
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client.get(current.as_str()).header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) TchViewer");
        if let Some(r) = referer {
            req = req.header(reqwest::header::REFERER, r);
        }
        let mut res = req.send().await.map_err(|e| format!("取得できません: {e}"))?;
        if res.status().is_redirection() {
            let loc = res.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or("転送先がありません")?;
            current = current.join(loc).map_err(|_| "転送先の URL が正しくありません".to_string())?;
            continue;
        }
        if !res.status().is_success() {
            return Err(format!("取得できません（HTTP {}）", res.status().as_u16()));
        }
        if res.content_length().is_some_and(|n| n as usize > max_bytes) {
            return Err(format!("画像が大きすぎます（上限 {}MB）", max_bytes / 1024 / 1024));
        }
        let ct = res.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| format!("取得できません: {e}"))? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > max_bytes {
                return Err(format!("画像が大きすぎます（上限 {}MB）", max_bytes / 1024 / 1024));
            }
        }
        let mime = image_mime(&ct, &bytes[..bytes.len().min(16)]).ok_or("画像ではありませんでした")?;
        return Ok(Fetched { url: current.to_string(), mime, bytes });
    }
    Err("転送が多すぎます".into())
}

pub fn data_url(f: &Fetched) -> String {
    format!("data:{};base64,{}", f.mime, base64::engine::general_purpose::STANDARD.encode(&f.bytes))
}

/// 保存するファイル名（URL の最後の部分から。使えない文字は _ に置き換え、拡張子をそろえる）
pub fn file_name(url: &str, mime: &str) -> String {
    let last = Url::parse(url).ok().and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(String::from))).unwrap_or_default();
    let mut name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(80).collect();
    name = name.trim_matches(['.', '_']).to_string();
    let ext = match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "jpg",
    };
    let stem = name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(name);
    format!("{}.{ext}", if stem.is_empty() { "image" } else { &stem })
}

/// 同じ名前があれば「名前 (1).jpg」のようにずらす
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (1..10000).map(|i| dir.join(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists()).unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect() {
        assert!(resolve("https://example.com/a/b.JPG?x=1", &[]).is_some());
        assert!(resolve("https://pbs.twimg.com/media/ABC?format=jpg&name=small", &[]).is_some());
        assert!(resolve("https://example.com/page.html", &[]).is_none());
        let rules = vec![Rule { pattern: r"https?://imgur\.com/(\w+)$".into(), replacement: "https://i.imgur.com/$1.jpg".into(), referer: Some("https://imgur.com/".into()) }];
        let (u, r) = resolve("https://imgur.com/abc123", &rules).unwrap();
        assert_eq!(u, "https://i.imgur.com/abc123.jpg");
        assert_eq!(r.as_deref(), Some("https://imgur.com/"));
    }

    #[test]
    fn public_ip() {
        for s in ["127.0.0.1", "10.0.0.1", "192.168.1.1", "172.16.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fe80::1", "fd00::1", "::ffff:192.168.0.1", "64:ff9b::a00:1", "2002:7f00:1::"] {
            assert!(!is_public_ip(s.parse().unwrap()), "{s}");
        }
        for s in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(s.parse().unwrap()), "{s}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(file_name("https://i.imgur.com/abc.jpeg?x=1", "image/jpeg"), "abc.jpg");
        assert_eq!(file_name("https://pbs.twimg.com/media/HNp?format=jpg", "image/png"), "HNp.png");
        assert_eq!(file_name("https://example.com/", "image/gif"), "image.gif");
        assert_eq!(image_mime("text/html", b"\x89PNG\r\n\x1a\nxxxx").as_deref(), Some("image/png"));
        assert!(image_mime("image/svg+xml", b"<svg").is_none());
    }
}
