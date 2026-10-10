//! レイアウト（OBS 表示の見た目）を名前を付けて複数保存する
//!
//! data/layouts/<名前>.json … 見た目の設定（Appearance）
//! data/layouts/<名前>.css  … 追加の CSS（メモ帳などで編集してもよい）
//! OBS 側は overlay.html?layout=<名前> で使い分ける。「標準」は削除できない。
//!
//! 名前はファイル名になるので、フォルダの区切りや Windows で使えない文字は受け付けない（data/layouts の外を触らせない）。

use crate::config::{data_dir, Appearance};
use std::path::PathBuf;

pub const DEFAULT: &str = "標準";
const MAX_NAME_CHARS: usize = 40;
const MAX_CSS_BYTES: usize = 256 * 1024;

fn dir() -> PathBuf {
    data_dir().join("layouts")
}

/// レイアウト名として使えるか（使えなければ理由）
pub fn check_name(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 || n > MAX_NAME_CHARS {
        return Err(format!("レイアウト名は1〜{MAX_NAME_CHARS}文字にしてください"));
    }
    if name.trim() != name || name.ends_with('.') || name.starts_with('.') {
        return Err("レイアウト名の先頭・末尾に空白やピリオドは使えません".into());
    }
    if name.chars().any(|c| c.is_control() || "\\/:*?\"<>|%#&".contains(c)) {
        return Err("レイアウト名に \\ / : * ? \" < > | % # & は使えません".into());
    }
    let upper = name.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    if ["CON", "PRN", "AUX", "NUL"].contains(&base) || ((base.starts_with("COM") || base.starts_with("LPT")) && base.len() == 4) {
        return Err("この名前は Windows の予約名のため使えません".into());
    }
    Ok(())
}

fn json_path(name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    Ok(dir().join(format!("{name}.json")))
}

fn css_path(name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    Ok(dir().join(format!("{name}.css")))
}

/// 初回起動時: 以前の設定（config.json の appearance）を「標準」レイアウトとして書き出す
pub fn migrate(old: &Appearance) {
    let Ok(p) = json_path(DEFAULT) else { return };
    if !p.exists() {
        let _ = save(DEFAULT, old);
    }
}

/// 一覧（「標準」が先頭、ほかは名前順）
pub fn list() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("json")).then(|| p.file_stem()?.to_str().map(String::from)).flatten()
        })
        .filter(|n| check_name(n).is_ok() && n != DEFAULT)
        .collect();
    names.sort();
    names.insert(0, DEFAULT.to_string());
    names
}

pub fn exists(name: &str) -> bool {
    json_path(name).map(|p| p.exists()).unwrap_or(false)
}

/// 見た目の設定（無ければ既定値）
pub fn load(name: &str) -> Appearance {
    json_path(name)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(name: &str, a: &Appearance) -> Result<(), String> {
    let p = json_path(name)?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(a).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn load_css(name: &str) -> String {
    css_path(name)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .filter(|b| b.len() <= MAX_CSS_BYTES)
        .map(|b| String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_string())
        .unwrap_or_default()
}

pub fn save_css(name: &str, css: &str) -> Result<(), String> {
    if css.len() > MAX_CSS_BYTES {
        return Err("CSS が大きすぎます（256KB まで）".into());
    }
    let p = css_path(name)?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(p, css).map_err(|e| e.to_string())
}

/// 新しいレイアウトを作る（from を指定するとその内容を複製）
pub fn create(name: &str, from: Option<&str>) -> Result<(), String> {
    check_name(name)?;
    if exists(name) {
        return Err(format!("「{name}」は既にあります"));
    }
    let (a, css) = match from {
        Some(f) => (load(f), load_css(f)),
        None => (Appearance::default(), String::new()),
    };
    save(name, &a)?;
    if !css.is_empty() {
        save_css(name, &css)?;
    }
    Ok(())
}

pub fn rename(from: &str, to: &str) -> Result<(), String> {
    if from == DEFAULT {
        return Err("「標準」は名前を変えられません".into());
    }
    check_name(to)?;
    if exists(to) {
        return Err(format!("「{to}」は既にあります"));
    }
    std::fs::rename(json_path(from)?, json_path(to)?).map_err(|e| e.to_string())?;
    let (c1, c2) = (css_path(from)?, css_path(to)?);
    if c1.exists() {
        std::fs::rename(c1, c2).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn delete(name: &str) -> Result<(), String> {
    if name == DEFAULT {
        return Err("「標準」は削除できません".into());
    }
    let _ = std::fs::remove_file(json_path(name)?);
    let _ = std::fs::remove_file(css_path(name)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("ゲーム画面用").is_ok());
        assert!(check_name("雑談 2").is_ok());
        assert!(check_name("").is_err());
        assert!(check_name("../x").is_err());
        assert!(check_name("a\\b").is_err());
        assert!(check_name("con").is_err());
        assert!(check_name("COM1").is_err());
        assert!(check_name(" a").is_err());
        assert!(check_name(".hidden").is_err());
        assert!(check_name(&"あ".repeat(41)).is_err());
    }
}
