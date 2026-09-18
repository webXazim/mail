use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::middleware::auth::user_from_cookie;
use crate::services::imap;
use crate::state::AppState;

const BUFFER_CAP: usize = 200;

/// How often the realtime worker scans each provisioned mailbox. Kept at 2s to
/// meet the "new mail in <2s" bar; a future revision should replace the poll
/// with Stalwart's JMAP EventSource (RFC 8620 §7.3) push channel.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Cap on how many new messages one tick will announce.
const MAX_PER_TICK: usize = 25;

/// Fan-out hub for mailbox events. Events carry the target `user_id` so a
/// single process-wide broadcast can serve every authenticated socket without
/// leaking another user's mail. In-memory only: the mail store lives in
/// Stalwart, so this mirrors what the poller observes there.
#[derive(Clone)]
pub struct EventHub {
    tx: broadcast::Sender<String>,
    buffer: Arc<Mutex<VecDeque<Value>>>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            tx,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_CAP))),
        }
    }

    /// Publish an event aimed at exactly one user. The `user_id` is injected
    /// here rather than trusted from the caller.
    pub fn publish_to(&self, user_id: Uuid, mut event: Value) {
        if let Some(obj) = event.as_object_mut() {
            obj.insert("user_id".into(), json!(user_id));
        }
        let msg = event.to_string();
        let _ = self.tx.send(msg);
        let mut buf = self.buffer.lock().unwrap();
        if buf.len() >= BUFFER_CAP {
            buf.pop_front();
        }
        buf.push_back(event);
    }

    fn recent_for(&self, user_id: Uuid) -> Vec<Value> {
        let target = json!(user_id);
        let buf = self.buffer.lock().unwrap();
        buf.iter()
            .filter(|e| e.get("user_id") == Some(&target))
            .cloned()
            .collect()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-user realtime worker (WS2.6): watches Stalwart for new Inbox mail and
/// fresh disk usage, then pushes `new-mail` / `quota` events to that user's
/// sockets. Replaces the old global fake-quota ticker.
pub fn spawn_realtime(state: AppState) {
    tokio::spawn(async move {
        let mut watermarks: HashMap<Uuid, chrono::DateTime<chrono::Utc>> = HashMap::new();
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        loop {
            tick.tick().await;
            let users: Vec<(Uuid, String)> = match sqlx::query_as(
                "SELECT id, mail_account_id FROM users WHERE mail_account_id <> ''",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!("realtime user scan failed: {e}");
                    continue;
                }
            };
            for (user_id, account) in users {
                push_quota(&state, user_id, &account).await;
                poll_new_mail(&state, user_id, &account, &mut watermarks).await;
            }
        }
    });
}

async fn push_quota(state: &AppState, user_id: Uuid, account: &str) {
    match imap::account_quota(&state.mail, account).await {
        Ok(Some((used, total))) => state.hub.publish_to(
            user_id,
            json!({ "kind": "quota", "payload": { "used": used, "total": total } }),
        ),
        Ok(None) => {}
        Err(e) => tracing::debug!(%account, "quota poll failed: {e}"),
    }
}

async fn poll_new_mail(
    state: &AppState,
    user_id: Uuid,
    account: &str,
    watermarks: &mut HashMap<Uuid, chrono::DateTime<chrono::Utc>>,
) {
    let since = watermarks.get(&user_id).cloned();
    let rows = match imap::recent_inbox(
        &state.mail,
        account,
        since.as_ref().map(|d| d.to_rfc3339()).as_deref(),
        MAX_PER_TICK,
    )
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::debug!(%account, "new-mail poll failed: {e}");
            return;
        }
    };
    let rows = rows.as_array().cloned().unwrap_or_default();

    let Some(previous) = since else {
        // First sight: establish the watermark from the newest message without
        // replaying the mailbox as "new".
        if let Some(newest) = rows
            .iter()
            .filter_map(|r| r.get("received_at").and_then(Value::as_str))
            .filter_map(parse_ts)
            .max()
        {
            watermarks.insert(user_id, newest);
        }
        return;
    };

    let mut fresh: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            r.get("received_at")
                .and_then(Value::as_str)
                .and_then(parse_ts)
                .is_some_and(|t| t > previous)
        })
        .collect();
    if fresh.is_empty() {
        return;
    }
    // Announce oldest → newest so notifications stack in arrival order.
    fresh.reverse();
    for row in &fresh {
        state.hub.publish_to(
            user_id,
            json!({
                "kind": "new-mail",
                "payload": {
                    "id": row.get("id").cloned().unwrap_or(json!("")),
                    "thread_id": row.get("thread_id").cloned().unwrap_or(json!("")),
                    "from": row.get("from").and_then(|f| f.get("email")).cloned().unwrap_or(json!("")),
                    "subject": row.get("subject").cloned().unwrap_or(json!("")),
                }
            }),
        );
    }
    if let Some(newest) = fresh
        .iter()
        .filter_map(|r| r.get("received_at").and_then(Value::as_str))
        .filter_map(parse_ts)
        .max()
    {
        watermarks.insert(user_id, newest);
    }
}

fn parse_ts(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

#[derive(Deserialize)]
pub struct PollParam {
    pub poll: Option<String>,
}

/// `/api/ws` serves two modes from the same route, both authenticated by the
/// same-origin HttpOnly session cookie:
///   * WS upgrade (no `?poll=`) - persistent per-user push.
///   * `GET /api/ws?poll=1`      - JSON array of this user's recent events.
pub async fn ws_or_poll(
    State(state): State<AppState>,
    Query(param): Query<PollParam>,
    headers: HeaderMap,
    ws: Option<WebSocketUpgrade>,
) -> Response {
    let hub = state.hub.clone();

    let Some(user) = user_from_cookie(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized",
                "message": "Sign in to receive mailbox events"
            })),
        )
            .into_response();
    };

    if param.poll.is_some() {
        return Json(hub.recent_for(user.user_id)).into_response();
    }

    match ws {
        Some(upgrade) => upgrade
            .on_upgrade(move |socket| handle_socket(socket, hub, user.user_id))
            .into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "bad_request",
                "message": "WebSocket upgrade headers required, or use ?poll=1"
            })),
        )
            .into_response(),
    }
}

async fn handle_socket(mut socket: WebSocket, hub: EventHub, user_id: Uuid) {
    let snapshot = hub.recent_for(user_id);
    if !snapshot.is_empty()
        && socket
            .send(Message::Text(serde_json::to_string(&snapshot).unwrap()))
            .await
            .is_err()
    {
        return;
    }

    let target = json!(user_id);
    let mut rx = hub.tx.subscribe();
    loop {
        tokio::select! {
            recv = rx.recv() => {
                match recv {
                    Ok(msg) => {
                        let Ok(mut value) = serde_json::from_str::<Value>(&msg) else { continue };
                        if value.get("user_id") != Some(&target) {
                            continue;
                        }
                        // The id is an internal routing detail; keep it off the wire.
                        if let Some(obj) = value.as_object_mut() {
                            obj.remove("user_id");
                        }
                        if socket.send(Message::Text(value.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_scoped_to_their_user() {
        let hub = EventHub::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        hub.publish_to(
            alice,
            json!({ "kind": "quota", "payload": { "used": 1, "total": 2 } }),
        );
        hub.publish_to(
            bob,
            json!({ "kind": "quota", "payload": { "used": 9, "total": 9 } }),
        );

        let for_alice = hub.recent_for(alice);
        assert_eq!(for_alice.len(), 1);
        assert_eq!(for_alice[0]["payload"]["used"], 1);

        assert_eq!(hub.recent_for(bob)[0]["payload"]["used"], 9);
        assert_eq!(hub.recent_for(Uuid::new_v4()).len(), 0);
    }

    #[test]
    fn parse_ts_handles_rfc3339() {
        assert!(parse_ts("2026-09-17T16:51:09Z").is_some());
        assert!(parse_ts("not-a-date").is_none());
    }
}
