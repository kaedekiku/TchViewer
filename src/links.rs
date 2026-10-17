//! 本文中の URL を既定のブラウザで開く前の検査
//!
//! 画面側でも同じ判定をしているが、本体側で必ずもう一度確かめる（画面側の判定は信用しない）。
//! - http / https のみ（file: やネットワーク共有、実行ファイルは開かない。LiveFake と同じ）
//! - 「https://見た目のドメイン@実際の行き先/」のようなユーザー名付きの URL は開かない
//! - 見えない文字・制御文字を含むもの、2048文字を超えるものは開かない
//! - このPC・家庭内ネットワークのアドレス（localhost・127.x・192.168.x・.local など、名前を引いた結果も含む）は開かない

use std::net::IpAddr;
use url::{Host, Url};

pub const MAX_URL_LEN: usize = 2048;

/// 名前を引かずに分かる範囲の検査。通れば整えた URL を返す
pub fn validate(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_URL_LEN {
        return Err("URL が長すぎます".into());
    }
    if raw.chars().any(|c| c.is_control() || c.is_whitespace() || crate::text::is_invisible(c)) {
        return Err("見えない文字を含む URL は開けません".into());
    }
    let u = Url::parse(raw).map_err(|_| "URL の形式が正しくありません")?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("http / https 以外の URL は開けません".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("ユーザー名付きの URL（見た目と行き先が違う可能性があるもの）は開けません".into());
    }
    match u.host().ok_or("URL にホスト名がありません")? {
        // 127.1 や 2130706433 のような書き方も、ここでは 127.0.0.1 として届く
        Host::Ipv4(ip) => check_ip(IpAddr::V4(ip))?,
        Host::Ipv6(ip) => check_ip(IpAddr::V6(ip))?,
        Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            let local = d == "localhost"
                || [".localhost", ".local", ".lan", ".home", ".internal", ".intranet", ".home.arpa"].iter().any(|s| d.ends_with(s))
                || !d.contains('.');
            if local {
                return Err("このPCや家庭内ネットワークのアドレスは開けません".into());
            }
        }
    }
    Ok(u)
}

fn check_ip(ip: IpAddr) -> Result<(), String> {
    // ::ffff:127.0.0.1 のような IPv4 の書き換えも中身で判定する
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    };
    if crate::image::is_public_ip(ip) {
        Ok(())
    } else {
        Err("このPCや家庭内ネットワークのアドレスは開けません".into())
    }
}

/// 開く前の検査（名前を引いて、行き先が家庭内ネットワークでないことも確かめる）
pub async fn check_open(raw: &str) -> Result<Url, String> {
    let u = validate(raw)?;
    crate::image::check_target(&u).await?;
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn allows_normal_urls() {
        for u in [
            "https://x.com/@example",
            "https://twitter.com/example/status/123",
            "https://bsky.app/profile/example.bsky.social",
            "http://example.com/a?b=c#d",
            "https://8.8.8.8/",
        ] {
            assert!(validate(u).is_ok(), "{u}");
        }
    }

    #[test]
    fn blocks_dangerous_urls() {
        for u in [
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "https://x.com@evil.example/",
            "https://user:pass@example.com/",
            "http://localhost:11280/",
            "http://foo.localhost/",
            "http://printer.local/",
            "http://router/",
            "http://127.0.0.1:50021/",
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://192.168.0.1/",
            "http://10.0.0.1/",
            "https://exa\u{200B}mple.com/",
            "https://example.com/\u{202E}gpj.exe",
        ] {
            assert!(validate(u).is_err(), "{u}");
        }
        assert!(validate(&format!("https://example.com/{}", "a".repeat(2100))).is_err());
    }
}
