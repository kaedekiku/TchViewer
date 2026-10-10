//! 設定（exe と同じフォルダの data/config.json に保存。AppData・レジストリは使わない）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// 例: ["some_channel"]
    pub twitch_channels: Vec<String>,
    /// 5ch / したらば / JPNKN の read.cgi URL
    pub threads: Vec<String>,
    /// Twitch のバッジ画像を取得する（非公式の方法。問題が出たらオフにする）
    pub twitch_badges: bool,
    /// 掲示板の巡回間隔（秒）。LiveFakeに合わせ 10〜300、既定15
    pub poll_seconds: u64,
    /// 接続時に既存レスを末尾から何件表示するか（読み上げはしない）
    pub initial_posts: usize,
    /// 5ch / したらば / JPNKN の本文に対するワードNG（`/…/` は正規表現）
    pub ng_words: Vec<String>,
    /// LiveFake の data フォルダ、または broadcast-whitelist.json そのもののパス
    pub whitelist_path: String,
    /// アプリ画面のテーマ: "system"（Windows に合わせる）/ "light" / "dark"
    pub ui_theme: String,
    pub speech: SpeechConfig,
    /// 旧形式の見た目の設定。初回起動時に data/layouts/標準.json へ引き継ぐ（以降は使わない）
    pub appearance: Appearance,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            twitch_channels: vec![],
            threads: vec![],
            twitch_badges: true,
            poll_seconds: 15,
            initial_posts: 0,
            ng_words: vec![],
            whitelist_path: String::new(),
            ui_theme: "system".into(),
            speech: SpeechConfig::default(),
            appearance: Appearance::default(),
        }
    }
}

impl Config {
    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.poll_seconds.clamp(10, 300))
    }
}

/// OBS表示の見た目（値の検証は表示側でも行う）
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub max_items: u32,
    /// 何秒で消すか（0=消さない）
    pub fade_seconds: u32,
    /// true=新しいものを上に積む
    pub newest_on_top: bool,
    /// コメント同士の間隔(px)
    pub gap: u32,
    /// 文字の縁取り(px, 0=なし)と色
    pub outline_width: f32,
    pub outline_color: String,
    pub board: BoardStyle,
    pub twitch: TwitchStyle,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            max_items: 15,
            fade_seconds: 0,
            newest_on_top: false,
            gap: 6,
            outline_width: 0.0,
            outline_color: "#000000".into(),
            board: BoardStyle::default(),
            twitch: TwitchStyle::default(),
        }
    }
}

/// 5ch / したらば / JPNKN のレス
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoardStyle {
    /// 空欄=既定のフォント
    pub font_family: String,
    /// 本文の文字サイズ(px)
    pub font_size: u32,
    /// 名前・ID・レス番号の行の文字サイズ（本文に対する%）
    pub meta_scale: u32,
    pub text_color: String,
    pub name_color: String,
    pub id_color: String,
    pub bg_color: String,
    /// 背景の不透明度 0〜1
    pub bg_opacity: f32,
    pub show_site: bool,
    pub show_no: bool,
    pub show_name: bool,
    pub show_id: bool,
}

impl Default for BoardStyle {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 22,
            meta_scale: 72,
            text_color: "#ffffff".into(),
            name_color: "#ffffff".into(),
            id_color: "#d0d0d0".into(),
            bg_color: "#000000".into(),
            bg_opacity: 0.55,
            show_site: true,
            show_no: true,
            show_name: true,
            show_id: true,
        }
    }
}

/// Twitch のチャット
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TwitchStyle {
    pub font_family: String,
    pub font_size: u32,
    pub meta_scale: u32,
    pub text_color: String,
    /// 視聴者が設定した名前の色を使う（false なら name_color）
    pub use_user_color: bool,
    /// 背景に対して読みにくい名前の色だけ、明るさを自動で調整する
    pub fix_name_contrast: bool,
    pub name_color: String,
    pub bg_color: String,
    pub bg_opacity: f32,
    pub show_site: bool,
    pub show_name: bool,
    pub show_badges: bool,
    /// スタンプ（エモート）の高さ(px)
    pub emote_size: u32,
    /// パワーアップで大きくなったスタンプの高さ(px)
    pub giant_emote_size: u32,
    /// バッジの高さ(px)
    pub badge_size: u32,
}

impl Default for TwitchStyle {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 22,
            meta_scale: 72,
            text_color: "#ffffff".into(),
            use_user_color: true,
            fix_name_contrast: false,
            name_color: "#bf94ff".into(),
            bg_color: "#000000".into(),
            bg_opacity: 0.55,
            show_site: true,
            show_name: true,
            show_badges: true,
            emote_size: 32,
            giant_emote_size: 112,
            badge_size: 18,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpeechConfig {
    pub enabled: bool,
    /// 読み上げエンジン: "bouyomi" / "sapi" / "voicevox"
    pub engine: String,
    /// 棒読みちゃん（アプリケーション連携のTCPサーバー）
    pub host: String,
    pub port: u16,
    /// 本文の最大読み上げ文字数（0=無制限）
    pub max_length: usize,
    pub sources: HashMap<String, bool>,
    /// Twitchは「名前さん 本文」と読む
    pub twitch_name: bool,
    pub sapi: SapiConfig,
    pub voicevox: VoicevoxConfig,
}

/// SAPI（Windows 標準の音声合成）
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SapiConfig {
    /// 声の識別子（空欄=Windowsの既定の声）
    pub voice: String,
    /// 速度 -10〜+10
    pub rate: i32,
    /// 音量 0〜100
    pub volume: u32,
}

impl Default for SapiConfig {
    fn default() -> Self {
        Self { voice: String::new(), rate: 0, volume: 100 }
    }
}

/// VOICEVOX（エンジンの HTTP API）
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoicevoxConfig {
    pub endpoint: String,
    /// 話者（スタイル）ID
    pub speaker: i32,
    /// 話速 0.5〜2.0
    pub speed: f64,
    /// 音高 -0.15〜+0.15
    pub pitch: f64,
    /// 抑揚 0〜2.0
    pub intonation: f64,
    /// 音量 0〜2.0
    pub volume: f64,
}

impl Default for VoicevoxConfig {
    fn default() -> Self {
        Self { endpoint: "http://127.0.0.1:50021".into(), speaker: 3, speed: 1.0, pitch: 0.0, intonation: 1.0, volume: 1.0 }
    }
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: "bouyomi".into(),
            host: "127.0.0.1".into(),
            port: 50001,
            max_length: 0,
            sources: ["twitch", "5ch", "shitaraba", "jpnkn"].iter().map(|s| (s.to_string(), true)).collect(),
            twitch_name: true,
            sapi: SapiConfig::default(),
            voicevox: VoicevoxConfig::default(),
        }
    }
}

/// データフォルダ: 環境変数 TCHVIEWER_DATA_DIR があればそれ、なければ exe と同じフォルダの data/
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TCHVIEWER_DATA_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d.trim());
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

/// 一時ファイルに書いてから置き換える（書き込み途中で落ちても設定が壊れないように）
pub fn save(cfg: &Config) -> std::io::Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("config.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(cfg)?)?;
    std::fs::rename(&tmp, config_path())
}

/// 動作ログ data/tchviewer.log。
/// 起動時と 5MB を超えたときに、それまでのログを tchviewer.prev.log に残して（1世代だけ）新しく始める
pub struct LogFile {
    file: Option<std::fs::File>,
    written: u64,
}

const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

impl LogFile {
    pub fn open() -> Self {
        let mut l = LogFile { file: None, written: 0 };
        l.rotate();
        l
    }

    fn rotate(&mut self) {
        self.file = None;
        let dir = data_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let (cur, prev) = (dir.join("tchviewer.log"), dir.join("tchviewer.prev.log"));
        if cur.exists() {
            let _ = std::fs::remove_file(&prev);
            let _ = std::fs::rename(&cur, &prev);
        }
        self.file = std::fs::File::create(cur).ok();
        self.written = 0;
    }

    pub fn write_line(&mut self, line: &str) {
        use std::io::Write;
        if self.written > LOG_MAX_BYTES {
            self.rotate();
        }
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{line}").is_ok() {
                self.written += line.len() as u64 + 1;
            }
        }
    }
}
