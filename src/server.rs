//! OBS 用の HTTP サーバー（127.0.0.1 のみ）
//!
//! 配信するのは OBS が読むものだけ: オーバーレイ / 見た目の設定（読み取り）/ コメントの配信。
//! 設定を変える窓口は持たない（アプリ画面は Tauri IPC で本体とやり取りする）。
//! DNS リバインディング対策として、Host ヘッダーが localhost:11280 / 127.0.0.1:11280 以外の要求は拒否する。
//!
//! デバッグ版だけ、ブラウザから画面を確かめるための /ui/ と /dev/api を追加する（リリース版には入らない）。

use crate::{App, PORT};
use axum::extract::{Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

const OVERLAY_HTML: &str = include_str!("../public/overlay.html");

pub async fn serve(listener: tokio::net::TcpListener, app: Arc<App>) -> std::io::Result<()> {
    let router = Router::new()
        .route("/overlay.html", get(|| async { Html(OVERLAY_HTML) }))
        .route("/events", get(events))
        .route("/ws", get(ws))
        .route("/api/appearance", get(get_appearance))
        .route("/layout.css", get(get_layout_css));
    #[cfg(debug_assertions)]
    let router = router
        .route("/ui/{*path}", get(dev_ui))
        .route("/dev/api", axum::routing::post(dev_api));
    let router = router.with_state(app).layer(middleware::from_fn(check_host));
    axum::serve(listener, router).await
}

/// Host ヘッダーがこのPCの TchViewer 宛てでなければ拒否する
async fn check_host(req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let ok = [format!("localhost:{PORT}"), format!("127.0.0.1:{PORT}"), format!("[::1]:{PORT}")].iter().any(|a| a == host);
    if !ok {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    next.run(req).await
}

async fn events(
    State(app): State<Arc<App>>,
    Query(q): Query<HashMap<String, String>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let n = q.get("history").and_then(|s| s.parse::<usize>().ok()).unwrap_or(30).min(200);
    // 先に購読してから履歴を取る（取りこぼし防止。重複は表示側が id で除外する）
    let rx = app.tx.subscribe();
    let hist = app.recent(n);
    let past = futures_util::stream::iter(hist.into_iter().map(|d| Ok(Event::default().data(d))));
    let live = BroadcastStream::new(rx).filter_map(|r| async move { r.ok().map(|d| Ok(Event::default().data(d))) });
    Sse::new(past.chain(live)).keep_alive(KeepAlive::default())
}

/// ?layout=名前 のレイアウト名（無い・使えない名前なら「標準」）
fn layout_param(q: &HashMap<String, String>) -> String {
    q.get("layout")
        .map(|s| s.trim().to_string())
        .filter(|n| crate::layouts::check_name(n).is_ok() && crate::layouts::exists(n))
        .unwrap_or_else(|| crate::layouts::DEFAULT.to_string())
}

async fn get_appearance(Query(q): Query<HashMap<String, String>>) -> Response {
    let name = layout_param(&q);
    Json(serde_json::json!({ "layout": name, "appearance": crate::layouts::load(&name) })).into_response()
}

/// レイアウトの追加 CSS（OBS 側で <link> として読み込む）
async fn get_layout_css(Query(q): Query<HashMap<String, String>>) -> Response {
    let css = crate::layouts::load_css(&layout_param(&q));
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], css).into_response()
}

/// コメントの配信（WebSocket）。ブラウザの「同じサイトへの同時接続は6本まで」の制限を受けないので、
/// OBS にブラウザソースをたくさん置いても止まりにくい。送る内容は /events と同じ JSON
async fn ws(State(app): State<Arc<App>>, Query(q): Query<HashMap<String, String>>, upgrade: axum::extract::ws::WebSocketUpgrade) -> Response {
    use axum::extract::ws::Message;
    let n = q.get("history").and_then(|s| s.parse::<usize>().ok()).unwrap_or(30).min(200);
    upgrade.on_upgrade(move |mut socket| async move {
        let mut rx = app.tx.subscribe();
        for d in app.recent(n) {
            if socket.send(Message::Text(d.into())).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(d) => if socket.send(Message::Text(d.into())).await.is_err() { return },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                },
                incoming = socket.recv() => match incoming {
                    // 表示側からは何も受け取らない（切断の検知だけ）
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    _ => {}
                },
            }
        }
    })
}

// ---------------------------------------------------------------- デバッグ版のみ

/// ui/ フォルダの画面をブラウザで確かめるための配信（ディスクから読む）
#[cfg(debug_assertions)]
async fn dev_ui(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    if path.contains("..") || path.contains('\\') {
        return StatusCode::FORBIDDEN.into_response();
    }
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui").join(&path);
    let mime = match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    };
    match std::fs::read(&file) {
        Ok(b) => ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-store")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// アプリ画面の api 呼び出しをブラウザから試すための窓口。
/// 専用ヘッダーを必須にして、他のサイトからは送れないようにする（付けると事前確認が必要になり、ブラウザが止める）
#[cfg(debug_assertions)]
async fn dev_api(State(app): State<Arc<App>>, headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>) -> Response {
    if headers.get("x-tchviewer-dev").is_none() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cmd = body.get("cmd").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let args = body.get("args").cloned().unwrap_or(serde_json::Value::Null);
    match crate::api::dispatch(&app, None, &cmd, args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}
