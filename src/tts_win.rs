//! Windows 標準の音声機能
//!   - SAPI（Windows の音声合成）: COM を専用スレッド（STA）で動かし、1件ずつ同期で読み上げる
//!   - WAV 再生（VOICEVOX 用）: PlaySoundW でメモリから直接再生する（一時ファイルを作らない）
//!
//! LiveFakeTauri2 (crates/core-tts) を参考にした。SAPI の声の一覧は Windows が管理する
//! 音声データの一覧（OS側のレジストリ）を SAPI 経由で読むだけで、TchViewer が書き込むことはない。

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
use windows::Win32::Media::Speech::{
    ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SpObjectToken, SpObjectTokenCategory, SpVoice, SPF_ASYNC, SPF_IS_NOT_XML,
    SPF_PURGEBEFORESPEAK, SPRS_IS_SPEAKING, SPVOICESTATUS,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED};

/// SAPI 5 の声と、Windows 10/11 の新しい声（OneCore）の両方を探す
const VOICE_CATEGORIES: [&str; 2] = [
    "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices",
    "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech_OneCore\\Voices",
];

#[derive(Clone, Debug, Serialize)]
pub struct VoiceInfo {
    /// 声の識別子（設定に保存する）
    pub id: String,
    pub name: String,
}

static SAPI_STOP: AtomicBool = AtomicBool::new(false);

enum Request {
    List(mpsc::Sender<Result<Vec<VoiceInfo>, String>>),
    Speak { text: String, voice: String, rate: i32, volume: u32, resp: mpsc::Sender<Result<(), String>> },
}

fn worker() -> &'static mpsc::Sender<Request> {
    static SENDER: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Request>();
        let _ = std::thread::Builder::new().name("sapi".into()).spawn(move || worker_loop(rx));
        tx
    })
}

fn worker_loop(rx: mpsc::Receiver<Request>) {
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let voice: Result<ISpVoice, String> = if init {
        unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) }.map_err(|e| format!("SAPIを初期化できません: {e}"))
    } else {
        Err("COMを初期化できません".into())
    };
    for req in rx {
        match req {
            Request::List(resp) => {
                let _ = resp.send(if init { list_voices_inner() } else { Err("COMを初期化できません".into()) });
            }
            Request::Speak { text, voice: id, rate, volume, resp } => {
                let r = match &voice {
                    Ok(v) => speak_inner(v, &text, &id, rate, volume),
                    Err(e) => Err(e.clone()),
                };
                let _ = resp.send(r);
            }
        }
    }
}

unsafe fn pwstr_to_string(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

fn list_voices_inner() -> Result<Vec<VoiceInfo>, String> {
    let mut out: Vec<VoiceInfo> = vec![];
    for cat_id in VOICE_CATEGORIES {
        unsafe {
            let Ok(cat) = CoCreateInstance::<_, ISpObjectTokenCategory>(&SpObjectTokenCategory, None, CLSCTX_ALL) else { continue };
            if cat.SetId(&HSTRING::from(cat_id), false).is_err() {
                continue;
            }
            let Ok(tokens) = cat.EnumTokens(PCWSTR::null(), PCWSTR::null()) else { continue };
            let mut count = 0u32;
            if tokens.GetCount(&mut count).is_err() {
                continue;
            }
            for i in 0..count {
                let Ok(token) = tokens.Item(i) else { continue };
                let id = token.GetId().map(|p| pwstr_to_string(p)).unwrap_or_default();
                let name = token.GetStringValue(PCWSTR::null()).map(|p| pwstr_to_string(p)).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                // 同じ声が両方の一覧にあるときは1つにまとめる
                if out.iter().any(|v| v.name == name && !name.is_empty()) {
                    continue;
                }
                out.push(VoiceInfo { id, name: if name.is_empty() { format!("Voice {i}") } else { name } });
            }
        }
    }
    Ok(out)
}

fn speak_inner(voice: &ISpVoice, text: &str, voice_id: &str, rate: i32, volume: u32) -> Result<(), String> {
    SAPI_STOP.store(false, Ordering::SeqCst);
    unsafe {
        if !voice_id.is_empty() {
            let token: ISpObjectToken = CoCreateInstance(&SpObjectToken, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
            // 見つからない声（削除された等）のときは既定の声のまま読む
            if token.SetId(PCWSTR::null(), &HSTRING::from(voice_id), false).is_ok() {
                let _ = voice.SetVoice(&token);
            }
        }
        voice.SetRate(rate.clamp(-10, 10)).map_err(|e| e.to_string())?;
        voice.SetVolume(volume.min(100) as u16).map_err(|e| e.to_string())?;
        let wide = HSTRING::from(text);
        voice
            // SPF_IS_NOT_XML: 視聴者のコメントに <silence> などの指定を書かれても、読み方を操作されないようにする
            .Speak(PCWSTR(wide.as_ptr()), (SPF_PURGEBEFORESPEAK.0 | SPF_ASYNC.0 | SPF_IS_NOT_XML.0) as u32, None)
            .map_err(|e| format!("読み上げに失敗: {e}"))?;
        loop {
            if SAPI_STOP.swap(false, Ordering::SeqCst) {
                let empty = HSTRING::new();
                let _ = voice.Speak(PCWSTR(empty.as_ptr()), SPF_PURGEBEFORESPEAK.0 as u32, None);
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
            let mut status = SPVOICESTATUS::default();
            voice.GetStatus(&mut status, std::ptr::null_mut()).map_err(|e| e.to_string())?;
            if status.dwRunningState != SPRS_IS_SPEAKING.0 as u32 {
                break;
            }
        }
    }
    Ok(())
}

/// SAPI の声の一覧
pub fn list_voices() -> Result<Vec<VoiceInfo>, String> {
    let (tx, rx) = mpsc::channel();
    worker().send(Request::List(tx)).map_err(|_| "SAPIのスレッドが動いていません".to_string())?;
    rx.recv().map_err(|_| "SAPIが応答しません".to_string())?
}

/// SAPI で読み上げる（読み終わるまで戻らない）
pub fn sapi_speak(text: &str, voice: &str, rate: i32, volume: u32) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    worker()
        .send(Request::Speak { text: text.into(), voice: voice.into(), rate, volume, resp: tx })
        .map_err(|_| "SAPIのスレッドが動いていません".to_string())?;
    rx.recv().map_err(|_| "SAPIが応答しません".to_string())?
}

/// WAV をメモリから再生する（再生し終わるまで戻らない）
pub fn play_wav(wav: &[u8]) -> Result<(), String> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("WAVデータではありません".into());
    }
    let ok = unsafe { PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
    if ok.as_bool() { Ok(()) } else { Err("音声を再生できませんでした".into()) }
}

/// 読み上げ中の SAPI と WAV 再生を止める
pub fn stop_all() {
    SAPI_STOP.store(true, Ordering::SeqCst);
    // PlaySoundW に NULL を渡すと、再生中の音を止める
    unsafe {
        let _ = PlaySoundW(PCWSTR::null(), None, SND_ASYNC | SND_NODEFAULT);
    }
}
