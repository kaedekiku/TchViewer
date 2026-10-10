//! ウィンドウの位置・大きさ・最大化の状態を data/window.json に保存し、次回起動時に復元する
//!
//! Tauri 公式の window-state プラグインは保存先が AppData のため使わず、自前で保存する。
//! モニターを外したなどで画面の外に出てしまう位置なら復元しない（既定の位置で開く）。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

fn path() -> std::path::PathBuf {
    crate::config::data_dir().join("window.json")
}

fn load() -> Option<WindowState> {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn save(st: &WindowState) {
    if let Ok(s) = serde_json::to_string_pretty(st) {
        let _ = std::fs::write(path(), s);
    }
}

/// 保存されている位置・大きさを当てはめる（画面内に収まるときだけ）
pub fn restore(win: &WebviewWindow) {
    let Some(st) = load() else { return };
    if st.width < 320 || st.height < 240 {
        return;
    }
    // ウィンドウの上端の中央付近が、どれかのモニターの中にあれば復元する（タイトルバーをつかめるように）
    let (cx, cy) = (st.x + st.width as i32 / 2, st.y + 16);
    let visible = win.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
    });
    if !visible {
        return;
    }
    let _ = win.set_size(PhysicalSize::new(st.width, st.height));
    let _ = win.set_position(PhysicalPosition::new(st.x, st.y));
    if st.maximized {
        let _ = win.maximize();
    }
}

static LAST: Mutex<Option<WindowState>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// 今の状態を記録する。最大化中は、最大化前の位置・大きさを残したまま「最大化」だけ記録する
fn capture(win: &WebviewWindow) -> Option<WindowState> {
    let maximized = win.is_maximized().unwrap_or(false);
    let minimized = win.is_minimized().unwrap_or(false);
    let mut last = LAST.lock().unwrap();
    if minimized {
        return *last;
    }
    let mut st = last.unwrap_or_default();
    st.maximized = maximized;
    if !maximized {
        if let (Ok(p), Ok(s)) = (win.outer_position(), win.inner_size()) {
            st.x = p.x;
            st.y = p.y;
            st.width = s.width;
            st.height = s.height;
        }
    }
    *last = Some(st);
    Some(st)
}

/// 移動・大きさの変更のたびに呼ぶ。0.6秒間変化が無くなったら保存する（ドラッグ中に何度も書かない）
pub fn changed(win: &WebviewWindow) {
    if capture(win).is_none() {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        if GENERATION.load(Ordering::SeqCst) == generation {
            if let Some(st) = *LAST.lock().unwrap() {
                save(&st);
            }
        }
    });
}

/// 閉じるときにすぐ保存する
pub fn closing(win: &WebviewWindow) {
    if let Some(st) = capture(win) {
        save(&st);
    }
}

/// 起動時: 保存済みの状態を「最後の状態」として覚えておく（最大化で開いたときの通常サイズを失わないため）
pub fn init() {
    *LAST.lock().unwrap() = load();
}
