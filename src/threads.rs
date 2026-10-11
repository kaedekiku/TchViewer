//! スレの巡回と保持（監視中のスレ・スレタブで開いているスレ）
//!
//! - 巡回は1つのタスクで行い、監視中または開いているスレだけを見に行く（設定を変えても Twitch はつなぎ直さない）
//! - 取得したレスは、スレタブの表示用にメモリにだけ保持する（1スレ POSTS_MAX 件まで。ファイルには保存しない）
//! - 監視中のスレの新着は NG → 自動ホワイトリスト → ホワイトリスト の順に判定し、
//!   通ったものは OBS 表示・読み上げ、通らなかったものは「番号だけ読む」などの扱いにする

use crate::boards::{self, Post, ThreadWatcher, POSTS_MAX};
use crate::ng::{self, NgHit};
use crate::speech::SpeechItem;
use crate::{timestamp, App};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// スレタブを同時に開ける上限
pub const MAX_OPEN_TABS: usize = 20;

#[derive(Clone, Debug, Default)]
pub struct ThreadView {
    pub url: String,
    pub title: String,
    pub posts: Vec<Post>,
}

impl App {
    // ------------------------------------------------------------ 判定

    pub fn ng_check(&self, thread_key: &str, p: &Post) -> Option<NgHit> {
        ng::check(&self.config.read().unwrap().ng, thread_key, &p.name, &p.uid, &p.text)
    }

    /// ホワイトリスト（LiveFake の一覧 ＋ TchViewer 独自の一覧）に入っているか
    pub fn allows(&self, uid: &str) -> bool {
        !uid.is_empty() && (self.whitelist.read().unwrap().allows(uid) || self.own_wl.lock().unwrap().contains(uid))
    }

    /// 【試験的】登録ワードを含むレスの ID を自動でホワイトリストに入れる（NG のレスは呼ばれない）
    pub fn auto_whitelist(&self, p: &Post) {
        let opts = self.config.read().unwrap().whitelist.clone();
        if !opts.auto_enabled || p.uid.is_empty() || self.allows(&p.uid) {
            return;
        }
        let Some(word) = crate::text::ng_hit(&opts.auto_words, &p.text) else { return };
        if self.own_wl.lock().unwrap().add_auto(&p.uid, word, opts.auto_limit.clamp(1, 100)) {
            self.status(format!("ホワイトリストに自動追加: ID:{}（ワード「{word}」）", p.uid));
        }
    }

    /// 監視中のスレで、ホワイトリストに無い ID のレスが来たとき（OBS には出さない）
    pub fn unlisted(&self, key: &str, url: &str, source: &str, site: &str, title: &str, p: &Post) {
        let skey = format!("u:{key}:{}", p.no);
        let msg = json!({
            "type": "unlisted", "key": key, "url": url, "source": source, "site": site, "thread": title,
            "no": p.no, "name": p.name, "uid": p.uid, "text": p.text.chars().take(300).collect::<String>(),
            "time": timestamp(), "speechKey": skey, "boot": self.boot,
        });
        let _ = self.ui_tx.send(msg.to_string());
        let sp = self.config.read().unwrap().speech.clone();
        if !sp.enabled || !sp.sources.get(source).copied().unwrap_or(false) || p.no >= 1001 {
            return;
        }
        let item = match sp.unlisted_mode.as_str() {
            "chime" => Some(SpeechItem::chime(skey)),
            "full" => {
                let c = crate::Comment { source: source.into(), site: site.into(), no: Some(p.no), name: p.name.clone(), text: p.text.clone(), ..Default::default() };
                let t = crate::speech::speech_text(&c, &sp);
                (!t.is_empty()).then(|| SpeechItem::text(t, Some(skey)))
            }
            "none" => None,
            _ => Some(SpeechItem::text(crate::speech::number_text(source, p.no), Some(skey))),
        };
        if let Some(item) = item {
            self.enqueue_item(item);
        }
    }

    // ------------------------------------------------------------ 保持とスレタブへの通知

    /// 取得したレスを保持する（初回は置き換え、2回目以降は追加）
    pub fn store_posts(&self, key: &str, url: &str, title: &str, posts: &[Post], first: bool) {
        let mut m = self.thread_views.lock().unwrap();
        let v = m.entry(key.to_string()).or_default();
        v.url = url.to_string();
        if !title.is_empty() {
            v.title = title.to_string();
        }
        if first {
            v.posts.clear();
        }
        v.posts.extend(posts.iter().cloned());
        let over = v.posts.len().saturating_sub(POSTS_MAX);
        v.posts.drain(..over);
    }

    /// スレタブ表示用の1件（NG・ホワイトリストの判定結果つき）
    pub fn post_json(&self, key: &str, p: &Post) -> Value {
        let hit = self.ng_check(key, p);
        json!({
            "no": p.no, "name": p.name, "mail": p.mail, "date": p.date, "uid": p.uid, "text": p.text,
            "ng": hit.as_ref().map(|h| h.mode.clone()), "ngReason": hit.map(|h| h.reason), "wl": self.allows(&p.uid),
        })
    }

    /// スレタブへ新着を知らせる
    /// スレタブへ新着を送る。initial は開いた直後の読み込み（ビュワー下の「スレの最新レス」には出さない）
    pub fn thread_updated(&self, info: &boards::ThreadInfo, title: &str, posts: &[Post], initial: bool) {
        let key = info.key();
        let list: Vec<Value> = posts.iter().map(|p| self.post_json(&key, p)).collect();
        let msg = json!({
            "type": "thread_update", "key": key, "url": info.read_url(), "source": info.source(), "site": info.site(),
            "title": title, "posts": list, "initial": initial, "time": timestamp(), "boot": self.boot,
        });
        let _ = self.ui_tx.send(msg.to_string());
    }

    /// スレタブを開いたとき・NG やホワイトリストを変えたときの全件
    pub fn thread_snapshot(&self, key: &str) -> Value {
        let v = self.thread_views.lock().unwrap().get(key).cloned().unwrap_or_default();
        let monitored = self.watchers.lock().unwrap().contains_key(key) && self.is_monitored_key(key);
        let list: Vec<Value> = v.posts.iter().map(|p| self.post_json(key, p)).collect();
        json!({ "key": key, "url": v.url, "title": v.title, "posts": list, "monitored": monitored })
    }

    fn is_monitored_key(&self, key: &str) -> bool {
        self.config.read().unwrap().threads.iter().any(|u| ng::thread_key_of(u).as_deref() == Some(key))
    }

    // ------------------------------------------------------------ 巡回

    /// 設定（監視するスレ・開いているタブ）に合わせて巡回対象をそろえる。新しく加わったスレはすぐ取得する
    pub fn sync_threads(self: &Arc<Self>) {
        let cfg = self.config();
        let mut wanted: Vec<(String, boards::ThreadInfo, bool, bool)> = vec![];
        for (url, monitored, open) in cfg.threads.iter().map(|u| (u, true, false)).chain(cfg.open_tabs.iter().map(|u| (u, false, true))) {
            match boards::parse_thread_url(url) {
                Ok(info) => {
                    let key = info.key();
                    if let Some(w) = wanted.iter_mut().find(|w| w.0 == key) {
                        w.2 |= monitored;
                        w.3 |= open;
                    } else {
                        wanted.push((key, info, monitored, open));
                    }
                }
                Err(e) if monitored => self.status(format!("URLエラー: {url} — {e}")),
                Err(_) => {}
            }
        }
        let monitored_keys: Vec<String> = wanted.iter().filter(|w| w.2).map(|w| w.0.clone()).collect();
        self.stats.lock().unwrap().retain(|k, _| k.starts_with("Twitch ") || monitored_keys.contains(k));
        let mut new_keys = vec![];
        {
            let mut ws = self.watchers.lock().unwrap();
            ws.retain(|k, _| wanted.iter().any(|w| &w.0 == k));
            for (key, info, monitored, open) in wanted {
                if let Some(w) = ws.get(&key).cloned() {
                    // 既にあるスレ: 巡回中なら終わってから切り替える
                    tokio::spawn(async move {
                        let mut g = w.lock().await;
                        g.set_monitored(monitored);
                        g.open = open;
                    });
                } else {
                    // 新しいスレ: 印を付けた状態で作り、すぐ取得する
                    let mut w = ThreadWatcher::new(info.clone());
                    w.set_monitored(monitored);
                    w.open = open;
                    ws.insert(key.clone(), Arc::new(tokio::sync::Mutex::new(w)));
                    new_keys.push(key.clone());
                }
                if monitored {
                    self.stat(&key, |s| {
                        s.source = info.source().into();
                        s.site = info.site().into();
                        s.label = info.label();
                        if s.state.is_empty() {
                            s.state = "connecting".into();
                        }
                    });
                }
            }
        }
        // 対象外になったスレの保持分を捨てる
        {
            let ws = self.watchers.lock().unwrap();
            self.thread_views.lock().unwrap().retain(|k, _| ws.contains_key(k));
        }
        for key in new_keys {
            let app = self.clone();
            tokio::spawn(async move { app.poll_one(&key).await });
        }
    }

    async fn poll_one(self: &Arc<Self>, key: &str) {
        let Some(w) = self.watchers.lock().unwrap().get(key).cloned() else { return };
        let mut g = w.lock().await;
        // 間隔は取得を始めた時刻から測る
        let started = Instant::now();
        let r = g.poll(self, &self.http).await;
        g.last_poll = started;
        g.fails = if r.is_ok() { 0 } else { g.fails.saturating_add(1) };
        if let Err(e) = r {
            if g.monitored {
                self.stat(key, |s| {
                    s.state = "error".into();
                    s.last_error = format!("[{}] {e}", timestamp());
                });
            }
            self.status(format!("{} 取得失敗 {}: {e}", g.info.site(), g.info.label()));
        }
    }

    /// 開いているスレタブをすぐ取得する（「今すぐ更新」）
    pub async fn refresh_open_tabs(self: &Arc<Self>) {
        let mut keys = vec![];
        for (k, w) in self.watchers.lock().unwrap().iter() {
            if w.try_lock().map(|g| g.open).unwrap_or(false) {
                keys.push(k.clone());
            }
        }
        futures_util::future::join_all(keys.iter().map(|k| self.poll_one(k))).await;
    }

    /// 巡回のタスク（起動時に1つだけ作る）。1秒ごとに、間隔が来たスレを取得する
    ///
    /// - 監視するスレは「巡回間隔」、スレタブは「スレタブの自動更新」の間隔（両方なら短いほう）
    /// - 同じホストへは1件ずつ順に問い合わせる
    /// - 失敗が続いたスレは間隔を倍々に延ばす（最大300秒）
    pub async fn run_poller(self: Arc<Self>) {
        // ホワイトリストの初回読み込み（読み直し待ち 300ms を含む）を先に済ませる
        tokio::time::sleep(Duration::from_millis(700)).await;
        let busy: Arc<std::sync::Mutex<HashSet<String>>> = Default::default();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let cfg = self.config();
            let now = Instant::now();
            let list: Vec<_> = self.watchers.lock().unwrap().iter().map(|(k, w)| (k.clone(), w.clone())).collect();
            let mut due: HashMap<String, Vec<String>> = HashMap::new();
            for (key, w) in list {
                if busy.lock().unwrap().contains(&key) {
                    continue;
                }
                // 取得中（スレタブを開いた直後など）は次の機会に回す
                let Ok(g) = w.try_lock() else { continue };
                let mut iv: Option<Duration> = None;
                if g.monitored {
                    iv = Some(cfg.poll_interval());
                }
                if g.open && cfg.tab_auto {
                    iv = Some(iv.map_or(cfg.tab_interval(), |v| v.min(cfg.tab_interval())));
                }
                let Some(iv) = iv else { continue };
                if now.duration_since(g.last_poll) >= backoff(iv, g.fails) {
                    due.entry(g.info.host()).or_default().push(key);
                }
            }
            for (_, keys) in due {
                busy.lock().unwrap().extend(keys.iter().cloned());
                let (app, busy) = (self.clone(), busy.clone());
                tokio::spawn(async move {
                    for k in keys {
                        app.poll_one(&k).await;
                        busy.lock().unwrap().remove(&k);
                    }
                });
            }
        }
    }
}

/// 失敗が続いたときの間隔（2, 4, 8…倍。300秒を超えない。もともと長いときはそのまま）
fn backoff(iv: Duration, fails: u32) -> Duration {
    if fails == 0 {
        return iv;
    }
    let max = Duration::from_secs(300).max(iv);
    iv.saturating_mul(1u32 << fails.min(5)).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_grows_and_caps() {
        let s = Duration::from_secs;
        assert_eq!(backoff(s(15), 0), s(15));
        assert_eq!(backoff(s(15), 1), s(30));
        assert_eq!(backoff(s(15), 3), s(120));
        assert_eq!(backoff(s(15), 9), s(300));
        assert_eq!(backoff(s(10), 2), s(40));
    }
}
