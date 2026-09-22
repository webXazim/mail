use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header::ORIGIN, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::middleware::auth::{user_from_cookie, AuthUser};
use crate::services::{imap, tenancy};
use crate::state::AppState;

const REALTIME_CHANNEL: &str = "cs_mail_realtime";
const DEFAULT_REPLAY_PAGE: i64 = 250;
const MAX_REPLAY_EVENTS: usize = 5_000;
const SOCKET_AUTH_RECHECK: Duration = Duration::from_secs(30);
const MAX_QUERY_CHANGE_PAGES: usize = 20;
const QUERY_CHANGE_BATCH: usize = 100;
const WATCH_TTL_SECS: i64 = 120;
const INBOX_STALE_BASELINE_SECS: i64 = 300;

#[derive(Debug, Clone)]
pub struct RealtimeEvent {
    pub seq: i64,
    pub user_id: Uuid,
    pub mailbox_id: Option<Uuid>,
    pub kind: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct WireEvent<'a> {
    seq: i64,
    kind: &'a str,
    payload: &'a Value,
    created_at: DateTime<Utc>,
}

impl RealtimeEvent {
    fn wire(&self) -> WireEvent<'_> {
        WireEvent {
            seq: self.seq,
            kind: &self.kind,
            payload: &self.payload,
            created_at: self.created_at.clone(),
        }
    }
}

/// Process-local fan-out only. Durability and cross-replica propagation live in
/// PostgreSQL (`realtime_events` + LISTEN/NOTIFY), so this hub may be recreated
/// without losing client-visible state.
#[derive(Clone)]
pub struct EventHub {
    tx: broadcast::Sender<RealtimeEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1_024);
        Self { tx }
    }

    fn publish_local(&self, event: RealtimeEvent) {
        let _ = self.tx.send(event);
    }

    fn subscribe(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Insert first, notify second. The event row is the durable truth and the
/// notification is only a wake-up hint. If a replica misses NOTIFY while it is
/// restarting, reconnect replay still reads the committed row by sequence.
pub async fn emit_event(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    payload: Value,
) -> Result<i64, String> {
    if kind.is_empty() || kind.len() > 80 {
        return Err("invalid realtime event kind".into());
    }
    if !payload.is_object() {
        return Err("realtime payload must be an object".into());
    }

    let mailbox_id = payload
        .get("mailbox_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    let (seq, created_at): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO realtime_events(user_id, mailbox_id, kind, payload)
         VALUES ($1, $2, $3, $4)
         RETURNING seq, created_at",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .bind(kind)
    .bind(payload.clone())
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("SELECT pg_notify('cs_mail_realtime', $1)")
        .bind(seq.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;

    // Deliver immediately on the current replica. The listener will observe
    // the same seq as well; each socket suppresses <= last_seq duplicates.
    state.hub.publish_local(RealtimeEvent {
        seq,
        user_id,
        mailbox_id,
        kind: kind.to_string(),
        payload,
        created_at,
    });
    Ok(seq)
}

pub fn spawn_realtime(state: AppState) {
    spawn_db_listener(state.clone());
    spawn_mailbox_worker(state.clone());
    spawn_event_cleanup(state);
}

fn spawn_db_listener(state: AppState) {
    tokio::spawn(async move {
        loop {
            let mut listener = match PgListener::connect_with(&state.db).await {
                Ok(listener) => listener,
                Err(err) => {
                    tracing::warn!("realtime listener connect failed: {err}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            if let Err(err) = listener.listen(REALTIME_CHANNEL).await {
                tracing::warn!("realtime LISTEN failed: {err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            tracing::info!(channel = REALTIME_CHANNEL, "realtime database listener ready");

            loop {
                let notification = match listener.recv().await {
                    Ok(notification) => notification,
                    Err(err) => {
                        tracing::warn!("realtime listener disconnected: {err}");
                        break;
                    }
                };
                let Ok(seq) = notification.payload().parse::<i64>() else {
                    tracing::debug!(payload = notification.payload(), "ignored malformed realtime notification");
                    continue;
                };
                match load_event(&state, seq).await {
                    Ok(Some(event)) => state.hub.publish_local(event),
                    Ok(None) => {}
                    Err(err) => tracing::warn!(seq, "realtime event load failed: {err}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn spawn_event_cleanup(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            tick.tick().await;
            let keep_secs = state.realtime_event_retention_secs.max(3600) as i64;
            if let Err(err) = sqlx::query(
                "DELETE FROM realtime_events
                 WHERE created_at < now() - ($1 * interval '1 second')",
            )
            .bind(keep_secs)
            .execute(&state.db)
            .await
            {
                tracing::warn!("realtime event cleanup failed: {err}");
            }
        }
    });
}

fn spawn_mailbox_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(state.realtime_poll_secs.max(1)));
        loop {
            tick.tick().await;
            if let Err(err) = ensure_mailbox_state_rows(&state).await {
                tracing::warn!("realtime mailbox state sync failed: {err}");
                continue;
            }
            let claims = match claim_mailboxes(&state).await {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::warn!("realtime mailbox claim failed: {err}");
                    continue;
                }
            };
            for claim in claims {
                let state = state.clone();
                tokio::spawn(async move {
                    process_mailbox_claim(&state, claim).await;
                });
            }
        }
    });
}

#[derive(Debug)]
struct MailboxClaim {
    user_id: Uuid,
    mailbox_id: Uuid,
    account: String,
    configured_quota: i64,
    initialized: bool,
    inbox_query_state: String,
    email_state: String,
    mailbox_state: String,
    quota_used: Option<i64>,
    configured_quota_total: Option<i64>,
    provider_quota_total: Option<i64>,
    last_polled_at: Option<DateTime<Utc>>,
}

async fn ensure_mailbox_state_rows(state: &AppState) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO realtime_mailbox_state(user_id,mailbox_id)
         SELECT m.user_id,m.id
         FROM mailboxes m
         JOIN users u ON u.id=m.user_id AND u.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om
           ON om.organization_id=m.organization_id AND om.user_id=m.user_id AND om.status='active'
         WHERE m.user_id IS NOT NULL AND m.deleted_at IS NULL AND m.status='active'
           AND COALESCE(m.provider_account_id, '') <> ''
         ON CONFLICT(mailbox_id) DO UPDATE SET user_id=EXCLUDED.user_id",
    )
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn claim_mailboxes(state: &AppState) -> Result<Vec<MailboxClaim>, sqlx::Error> {
    let owner = state.realtime_instance_id;
    let rows: Vec<(Uuid, Uuid, String, i64, bool, String, String, String, Option<i64>, Option<i64>, Option<i64>, Option<DateTime<Utc>>)> =
        sqlx::query_as(
            "WITH candidates AS (
               SELECT r.mailbox_id,r.user_id,m.provider_account_id AS mail_account_id,m.quota_bytes
               FROM realtime_mailbox_state r
               JOIN mailboxes m ON m.id=r.mailbox_id AND m.user_id=r.user_id AND m.deleted_at IS NULL
               JOIN users u ON u.id=r.user_id
               JOIN organizations o ON o.id=m.organization_id AND o.status='active'
               JOIN organization_memberships om
                 ON om.organization_id=m.organization_id AND om.user_id=r.user_id AND om.status='active'
               WHERE u.status='active' AND m.status='active'
                 AND COALESCE(m.provider_account_id, '') <> ''
                 AND r.watch_until IS NOT NULL AND r.watch_until > now()
                 AND (r.lease_until IS NULL OR r.lease_until <= now())
               ORDER BY r.last_polled_at ASC NULLS FIRST,r.mailbox_id
               FOR UPDATE OF r SKIP LOCKED
               LIMIT $2
             )
             UPDATE realtime_mailbox_state r
             SET lease_owner=$1,
                 lease_until=now()+($3 * interval '1 second'),
                 updated_at=now()
             FROM candidates c
             WHERE r.mailbox_id=c.mailbox_id
             RETURNING r.user_id,r.mailbox_id,c.mail_account_id,c.quota_bytes,
                       r.initialized,r.inbox_query_state,r.email_state,r.mailbox_state,
                       r.quota_used,r.configured_quota_total,r.provider_quota_total,r.last_polled_at",
        )
        .bind(owner)
        .bind(state.realtime_batch_size.max(1))
        .bind(state.realtime_lease_secs.max(10) as i64)
        .fetch_all(&state.db)
        .await?;

    Ok(rows.into_iter().map(|(
        user_id,mailbox_id,account,configured_quota,initialized,inbox_query_state,
        email_state,mailbox_state,quota_used,configured_quota_total,provider_quota_total,last_polled_at,
    )| MailboxClaim {
        user_id,mailbox_id,account,configured_quota,initialized,inbox_query_state,email_state,
        mailbox_state,quota_used,configured_quota_total,provider_quota_total,last_polled_at,
    }).collect())
}

async fn process_mailbox_claim(state: &AppState, claim: MailboxClaim) {
    let mut last_error = String::new();

    if let Err(err) = reconcile_quota(state, &claim).await {
        last_error = format!("quota: {err}");
        tracing::debug!(account = %claim.account, "realtime quota read failed: {err}");
    }

    if let Err(err) = reconcile_inbox(state, &claim).await {
        if last_error.is_empty() {
            last_error = format!("inbox: {err}");
        } else {
            last_error.push_str(&format!("; inbox: {err}"));
        }
        tracing::debug!(account = %claim.account, "realtime inbox read failed: {err}");
    }

    if let Err(err) = reconcile_email_state(state, &claim).await {
        if last_error.is_empty() {
            last_error = format!("email-state: {err}");
        } else {
            last_error.push_str(&format!("; email-state: {err}"));
        }
        tracing::debug!(account = %claim.account, "realtime Email/changes failed: {err}");
    }

    if let Err(err) = reconcile_mailbox_state(state, &claim).await {
        if last_error.is_empty() {
            last_error = format!("mailbox-state: {err}");
        } else {
            last_error.push_str(&format!("; mailbox-state: {err}"));
        }
        tracing::debug!(account = %claim.account, "realtime Mailbox/changes failed: {err}");
    }

    if let Err(err) = sqlx::query(
        "UPDATE realtime_mailbox_state
         SET lease_owner = NULL,
             lease_until = NULL,
             last_polled_at = now(),
             last_error = $2,
             updated_at = now()
         WHERE mailbox_id = $1 AND lease_owner = $3",
    )
    .bind(claim.mailbox_id)
    .bind(last_error)
    .bind(state.realtime_instance_id)
    .execute(&state.db)
    .await
    {
        tracing::warn!(user_id = %claim.user_id, "realtime mailbox lease release failed: {err}");
    }
}

async fn reconcile_quota(state: &AppState, claim: &MailboxClaim) -> Result<(), String> {
    let Some((used, provider_total)) = state.stalwart.account_quota(&claim.account).await? else {
        return Ok(());
    };
    let used_i64 = used.min(i64::MAX as u64) as i64;
    let provider_i64 = provider_total.min(i64::MAX as u64) as i64;
    let configured = claim.configured_quota.max(0);
    let changed = claim.quota_used != Some(used_i64)
        || claim.provider_quota_total != Some(provider_i64)
        || claim.configured_quota_total != Some(configured);

    sqlx::query(
        "UPDATE realtime_mailbox_state
         SET quota_used = $2,
             provider_quota_total = $3,
             configured_quota_total = $4,
             updated_at = now()
         WHERE mailbox_id = $1",
    )
    .bind(claim.mailbox_id)
    .bind(used_i64)
    .bind(provider_i64)
    .bind(configured)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if changed {
        emit_event(
            state,
            claim.user_id,
            "quota",
            json!({
                "used": used,
                "total": configured,
                "provider_total": provider_total,
                "quota_in_sync": provider_i64 == configured,
                "mailbox_id": claim.mailbox_id
            }),
        )
        .await?;
    }
    Ok(())
}

async fn reconcile_inbox(state: &AppState, claim: &MailboxClaim) -> Result<(), String> {
    let stale_after_idle = claim
        .last_polled_at
        .map(|last| Utc::now().signed_duration_since(last).num_seconds() > INBOX_STALE_BASELINE_SECS)
        .unwrap_or(claim.initialized);
    if !claim.initialized || claim.inbox_query_state.is_empty() || stale_after_idle {
        let query_state = imap::inbox_query_state(&state.stalwart, &claim.account).await?;
        sqlx::query(
            "UPDATE realtime_mailbox_state
             SET initialized = TRUE,
                 inbox_query_state = $2,
                 updated_at = now()
             WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(query_state.unwrap_or_default())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        if claim.initialized && stale_after_idle {
            emit_event(
                state,
                claim.user_id,
                "resource-changed",
                json!({ "resource": "mailbox", "action": "resume-resync", "mailbox_id": claim.mailbox_id }),
            )
            .await?;
        }
        return Ok(());
    }

    let mut since = claim.inbox_query_state.clone();
    for _ in 0..MAX_QUERY_CHANGE_PAGES {
        let changes = match imap::inbox_query_changes(
            &state.stalwart,
            &claim.account,
            &since,
            QUERY_CHANGE_BATCH,
        )
        .await
        {
            Ok(changes) => changes,
            Err(err) if query_state_expired(&err) => {
                let fresh = imap::inbox_query_state(&state.stalwart, &claim.account)
                    .await?
                    .unwrap_or_default();
                sqlx::query(
                    "UPDATE realtime_mailbox_state
                     SET initialized = TRUE, inbox_query_state = $2, updated_at = now()
                     WHERE mailbox_id = $1",
                )
                .bind(claim.mailbox_id)
                .bind(&fresh)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
                emit_event(
                    state,
                    claim.user_id,
                    "resource-changed",
                    json!({ "resource": "mailbox", "action": "resync", "mailbox_id": claim.mailbox_id }),
                )
                .await?;
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let new_state = changes
            .get("query_state")
            .and_then(Value::as_str)
            .unwrap_or(&since)
            .to_string();
        let emails = changes
            .get("emails")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if !emails.is_empty() {
            for row in &emails {
                let email_id = row.get("id").and_then(Value::as_str).unwrap_or("");
                let sender = row.get("from").and_then(|f| f.get("email")).and_then(Value::as_str).unwrap_or("");
                let subject = row.get("subject").and_then(Value::as_str).unwrap_or("(no subject)");
                emit_event(
                    state,
                    claim.user_id,
                    "new-mail",
                    json!({
                        "id": email_id,
                        "thread_id": row.get("thread_id").cloned().unwrap_or(json!("")),
                        "from": sender,
                        "subject": subject,
                        "received_at": row.get("received_at").cloned().unwrap_or(Value::Null),
                        "mailbox_id": claim.mailbox_id
                    }),
                )
                .await?;
                let notification_key = format!("mail:{email_id}");
                let notification_detail = if sender.is_empty() {
                    subject.to_string()
                } else {
                    format!("{sender} · {subject}")
                };
                if let Err(error) = crate::services::notifications::create_for_mailbox(
                    state,
                    claim.user_id,
                    claim.mailbox_id,
                    "mail",
                    if sender.is_empty() { "New message" } else { "New mail received" },
                    &notification_detail,
                    "/mail/inbox",
                    Some(&notification_key),
                )
                .await
                {
                    tracing::warn!(%error, user_id=%claim.user_id, "failed to persist new-mail notification");
                }
            }
        }
        if changes.get("changed").and_then(Value::as_bool).unwrap_or(false) {
            emit_event(
                state,
                claim.user_id,
                "resource-changed",
                json!({ "resource": "mailbox", "action": "inbox-membership", "mailbox_id": claim.mailbox_id }),
            )
            .await?;
        }

        sqlx::query(
            "UPDATE realtime_mailbox_state
             SET inbox_query_state = $2, initialized = TRUE, updated_at = now()
             WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(&new_state)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        since = new_state;
        if !changes
            .get("has_more_changes")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    // A mailbox changed faster than one worker tick can drain it. Keep the
    // last partial JMAP query state and force the browser to refresh its list;
    // the next tick continues from the saved state rather than dropping mail.
    emit_event(
        state,
        claim.user_id,
        "resource-changed",
        json!({ "resource": "mailbox", "action": "catch-up", "mailbox_id": claim.mailbox_id }),
    )
    .await?;
    Ok(())
}

async fn reconcile_email_state(state: &AppState, claim: &MailboxClaim) -> Result<(), String> {
    let stale_after_idle = claim
        .last_polled_at
        .map(|last| Utc::now().signed_duration_since(last).num_seconds() > INBOX_STALE_BASELINE_SECS)
        .unwrap_or(claim.initialized);
    if claim.email_state.is_empty() || stale_after_idle {
        let fresh = imap::email_state(&state.stalwart, &claim.account)
            .await?
            .unwrap_or_default();
        sqlx::query(
            "UPDATE realtime_mailbox_state SET email_state = $2, updated_at = now() WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(fresh)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let mut since = claim.email_state.clone();
    let mut changed = false;
    for _ in 0..MAX_QUERY_CHANGE_PAGES {
        let delta = match imap::email_changes(&state.stalwart, &claim.account, &since, QUERY_CHANGE_BATCH).await {
            Ok(delta) => delta,
            Err(err) if query_state_expired(&err) => {
                let fresh = imap::email_state(&state.stalwart, &claim.account)
                    .await?
                    .unwrap_or_default();
                sqlx::query(
                    "UPDATE realtime_mailbox_state SET email_state = $2, updated_at = now() WHERE mailbox_id = $1",
                )
                .bind(claim.mailbox_id)
                .bind(fresh)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
                emit_event(
                    state,
                    claim.user_id,
                    "resource-changed",
                    json!({ "resource": "mailbox", "action": "email-state-resync", "mailbox_id": claim.mailbox_id }),
                )
                .await?;
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        changed |= delta.get("changed").and_then(Value::as_bool).unwrap_or(false);
        let next = delta.get("state").and_then(Value::as_str).unwrap_or(&since).to_string();
        sqlx::query(
            "UPDATE realtime_mailbox_state SET email_state = $2, updated_at = now() WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(&next)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        since = next;
        if !delta.get("has_more_changes").and_then(Value::as_bool).unwrap_or(false) {
            break;
        }
    }
    if changed {
        emit_event(
            state,
            claim.user_id,
            "resource-changed",
            json!({ "resource": "mailbox", "action": "message-state", "mailbox_id": claim.mailbox_id }),
        )
        .await?;
    }
    Ok(())
}

async fn reconcile_mailbox_state(state: &AppState, claim: &MailboxClaim) -> Result<(), String> {
    let stale_after_idle = claim
        .last_polled_at
        .map(|last| Utc::now().signed_duration_since(last).num_seconds() > INBOX_STALE_BASELINE_SECS)
        .unwrap_or(claim.initialized);
    if claim.mailbox_state.is_empty() || stale_after_idle {
        let fresh = imap::mailbox_state(&state.stalwart, &claim.account)
            .await?
            .unwrap_or_default();
        sqlx::query(
            "UPDATE realtime_mailbox_state SET mailbox_state = $2, updated_at = now() WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(fresh)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let mut since = claim.mailbox_state.clone();
    let mut changed = false;
    for _ in 0..MAX_QUERY_CHANGE_PAGES {
        let delta = match imap::mailbox_changes(&state.stalwart, &claim.account, &since, QUERY_CHANGE_BATCH).await {
            Ok(delta) => delta,
            Err(err) if query_state_expired(&err) => {
                let fresh = imap::mailbox_state(&state.stalwart, &claim.account)
                    .await?
                    .unwrap_or_default();
                sqlx::query(
                    "UPDATE realtime_mailbox_state SET mailbox_state = $2, updated_at = now() WHERE mailbox_id = $1",
                )
                .bind(claim.mailbox_id)
                .bind(fresh)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
                emit_event(
                    state,
                    claim.user_id,
                    "resource-changed",
                    json!({ "resource": "mailbox", "action": "folder-state-resync", "folders_changed": true, "mailbox_id": claim.mailbox_id }),
                )
                .await?;
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        changed |= delta.get("changed").and_then(Value::as_bool).unwrap_or(false);
        let next = delta.get("state").and_then(Value::as_str).unwrap_or(&since).to_string();
        sqlx::query(
            "UPDATE realtime_mailbox_state SET mailbox_state = $2, updated_at = now() WHERE mailbox_id = $1",
        )
        .bind(claim.mailbox_id)
        .bind(&next)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        since = next;
        if !delta.get("has_more_changes").and_then(Value::as_bool).unwrap_or(false) {
            break;
        }
    }
    if changed {
        emit_event(
            state,
            claim.user_id,
            "resource-changed",
            json!({ "resource": "mailbox", "action": "folder-state", "folders_changed": true, "mailbox_id": claim.mailbox_id }),
        )
        .await?;
    }
    Ok(())
}

fn query_state_expired(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("cannotcalculatechanges")
        || lower.contains("cannot calculate changes")
        || lower.contains("invalidarguments") && lower.contains("query") && lower.contains("state")
}

async fn load_event(state: &AppState, seq: i64) -> Result<Option<RealtimeEvent>, sqlx::Error> {
    let row: Option<(i64, Uuid, Option<Uuid>, String, Value, DateTime<Utc>)> = sqlx::query_as(
        "SELECT seq,user_id,mailbox_id,kind,payload,created_at
         FROM realtime_events WHERE seq=$1",
    )
    .bind(seq)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|(seq,user_id,mailbox_id,kind,payload,created_at)| RealtimeEvent {
        seq,user_id,mailbox_id,kind,payload,created_at,
    }))
}

async fn events_after(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Option<Uuid>,
    after: i64,
    limit: i64,
) -> Result<Vec<RealtimeEvent>, sqlx::Error> {
    let rows: Vec<(i64, Uuid, Option<Uuid>, String, Value, DateTime<Utc>)> = sqlx::query_as(
        "SELECT seq,user_id,mailbox_id,kind,payload,created_at
         FROM realtime_events
         WHERE user_id=$1 AND seq>$2
           AND (mailbox_id IS NULL OR mailbox_id=$3)
         ORDER BY seq ASC
         LIMIT $4",
    )
    .bind(user_id)
    .bind(after)
    .bind(mailbox_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(|(seq,user_id,mailbox_id,kind,payload,created_at)| RealtimeEvent {
        seq,user_id,mailbox_id,kind,payload,created_at,
    }).collect())
}

async fn cursor_bounds(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Option<Uuid>,
) -> Result<(i64, i64), sqlx::Error> {
    let (min_seq,max_seq): (Option<i64>,Option<i64>) = sqlx::query_as(
        "SELECT min(seq),max(seq)
           FROM realtime_events
          WHERE user_id=$1 AND (mailbox_id IS NULL OR mailbox_id=$2)",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await?;
    Ok((min_seq.unwrap_or(0),max_seq.unwrap_or(0)))
}

fn cursor_has_gap(after: i64, min_seq: i64) -> bool {
    after > 0 && min_seq > 0 && after.saturating_add(1) < min_seq
}

async fn touch_mailbox_watch(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO realtime_mailbox_state(user_id,mailbox_id,watch_until,updated_at)
         VALUES($1,$2,now()+($3 * interval '1 second'),now())
         ON CONFLICT(mailbox_id) DO UPDATE
         SET user_id=EXCLUDED.user_id,
             watch_until=GREATEST(
               COALESCE(realtime_mailbox_state.watch_until,now()),
               now()+($3 * interval '1 second')
             ),
             updated_at=now()",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .bind(WATCH_TTL_SECS)
    .execute(&state.db)
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct RealtimeQuery {
    pub poll: Option<String>,
    pub after: Option<i64>,
    pub wait: Option<u64>,
    pub organization_id: Option<Uuid>,
    pub mailbox_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct PollResponse {
    events: Vec<OwnedWireEvent>,
    cursor: i64,
    resync_required: bool,
}

#[derive(Debug, Serialize)]
struct OwnedWireEvent {
    seq: i64,
    kind: String,
    payload: Value,
    created_at: DateTime<Utc>,
}

impl From<RealtimeEvent> for OwnedWireEvent {
    fn from(value: RealtimeEvent) -> Self {
        Self {
            seq: value.seq,
            kind: value.kind,
            payload: value.payload,
            created_at: value.created_at,
        }
    }
}

/// `/api/ws` supports both WebSocket and long-poll fallback using the same
/// HttpOnly refresh-session authentication. `after` is the last sequence the
/// client durably observed; it is safe to resend it after reconnect.
pub async fn ws_or_poll(
    State(state): State<AppState>,
    Query(param): Query<RealtimeQuery>,
    headers: HeaderMap,
    ws: Option<WebSocketUpgrade>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"origin_not_allowed","message":"Realtime connection origin is not allowed"})),
        ).into_response();
    }

    let Some(user) = user_from_cookie(&state, &headers).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized","message":"Sign in to receive realtime events"})),
        ).into_response();
    };

    let mailbox = match tenancy::active_mailbox(
        &state.db,
        user.user_id,
        param.organization_id.or(user.organization_id_hint),
        param.mailbox_id.or(user.mailbox_id_hint),
    ).await {
        Ok(mailbox) => mailbox,
        Err(error) => return error.into_response(),
    };
    let mailbox_id = mailbox.as_ref().map(|value| value.id);
    if let Some(mailbox_id) = mailbox_id {
        if let Err(err) = touch_mailbox_watch(&state,user.user_id,mailbox_id).await {
            tracing::debug!(user_id=%user.user_id,%mailbox_id,"failed to refresh realtime mailbox watch: {err}");
        }
    }

    if param.poll.is_some() {
        return match poll_events(&state,&user,mailbox_id,param.after,param.wait.unwrap_or(25)).await {
            Ok(response) => Json(response).into_response(),
            Err(err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"realtime_unavailable","message":err})),
            ).into_response(),
        };
    }

    match ws {
        Some(upgrade) => {
            let after=param.after;
            upgrade.on_upgrade(move |socket| handle_socket(socket,state,user,mailbox_id,after)).into_response()
        }
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"bad_request","message":"WebSocket upgrade headers required, or use ?poll=1"})),
        ).into_response(),
    }
}

async fn poll_events(
    state: &AppState,
    user: &AuthUser,
    mailbox_id: Option<Uuid>,
    after: Option<i64>,
    wait_seconds: u64,
) -> Result<PollResponse,String> {
    let (min_seq,max_seq)=cursor_bounds(state,user.user_id,mailbox_id).await.map_err(|e|e.to_string())?;
    let requested_after=after.unwrap_or(max_seq).clamp(0,max_seq.max(0));
    let resync_required=after.is_some() && cursor_has_gap(requested_after,min_seq);
    let effective_after=if resync_required { min_seq.saturating_sub(1) } else { requested_after };

    let mut events=events_after(state,user.user_id,mailbox_id,effective_after,DEFAULT_REPLAY_PAGE)
        .await.map_err(|e|e.to_string())?;
    if events.is_empty() && wait_seconds>0 {
        let mut rx=state.hub.subscribe();
        let wait=Duration::from_secs(wait_seconds.clamp(1,30));
        let _=tokio::time::timeout(wait,async {
            loop {
                match rx.recv().await {
                    Ok(event) if event.user_id==user.user_id
                        && event.seq>effective_after
                        && (event.mailbox_id.is_none() || event.mailbox_id==mailbox_id) => break,
                    Ok(_) => {},
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }).await;
        events=events_after(state,user.user_id,mailbox_id,effective_after,DEFAULT_REPLAY_PAGE)
            .await.map_err(|e|e.to_string())?;
    }

    let cursor=events.last().map(|event|event.seq).unwrap_or(max_seq.max(requested_after));
    Ok(PollResponse {
        events: events.into_iter().map(OwnedWireEvent::from).collect(),
        cursor,
        resync_required,
    })
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user: AuthUser,
    mailbox_id: Option<Uuid>,
    after: Option<i64>,
) {
    let mut rx=state.hub.subscribe();
    let (mut sender,mut receiver)=socket.split();

    let (min_seq,latest_seq)=match cursor_bounds(&state,user.user_id,mailbox_id).await {
        Ok(bounds)=>bounds,
        Err(err)=>{
            tracing::warn!(user_id=%user.user_id,"realtime cursor lookup failed: {err}");
            return;
        }
    };
    let requested_after=after.unwrap_or(latest_seq);
    let resync_required=after.is_some() && cursor_has_gap(requested_after,min_seq);
    let mut last_seq=if resync_required { min_seq.saturating_sub(1) } else { requested_after.min(latest_seq) };

    if after.is_some() {
        match replay_to_socket(&state,user.user_id,mailbox_id,last_seq,&mut sender).await {
            Ok(seq)=>last_seq=seq,
            Err(err)=>{
                tracing::debug!(user_id=%user.user_id,"realtime replay failed: {err}");
                return;
            }
        }
    }

    if send_control(
        &mut sender,
        last_seq,
        "ready",
        json!({
            "cursor":last_seq,
            "resync_required":resync_required,
            "retention_seconds":state.realtime_event_retention_secs,
            "mailbox_id":mailbox_id
        }),
    ).await.is_err() { return; }

    let mut auth_tick=tokio::time::interval(SOCKET_AUTH_RECHECK);
    auth_tick.tick().await;
    loop {
        tokio::select! {
            recv=rx.recv()=>{
                match recv {
                    Ok(event)=>{
                        if event.user_id!=user.user_id || event.seq<=last_seq
                            || !(event.mailbox_id.is_none() || event.mailbox_id==mailbox_id) {
                            continue;
                        }
                        if send_event(&mut sender,&event).await.is_err() { break; }
                        last_seq=event.seq;
                    }
                    Err(broadcast::error::RecvError::Lagged(_))=>{
                        match replay_to_socket(&state,user.user_id,mailbox_id,last_seq,&mut sender).await {
                            Ok(seq)=>last_seq=seq,
                            Err(_)=>break,
                        }
                    }
                    Err(broadcast::error::RecvError::Closed)=>break,
                }
            }
            incoming=receiver.next()=>{
                match incoming {
                    Some(Ok(Message::Close(_)))|None=>break,
                    Some(Ok(Message::Text(text)))=>{
                        if text.contains("\"type\":\"ping\"") {
                            if send_control(&mut sender,last_seq,"pong",json!({})).await.is_err(){ break; }
                        }
                    }
                    Some(Ok(Message::Ping(bytes)))=>{
                        if sender.send(Message::Pong(bytes)).await.is_err(){ break; }
                    }
                    Some(Ok(_))=>{},
                    Some(Err(_))=>break,
                }
            }
            _=auth_tick.tick()=>{
                if !session_is_active(&state,&user).await || !mailbox_context_is_active(&state,&user,mailbox_id).await {
                    let _=sender.send(Message::Close(None)).await;
                    break;
                }
                if let Some(mailbox_id)=mailbox_id {
                    let _=touch_mailbox_watch(&state,user.user_id,mailbox_id).await;
                }
                if sender.send(Message::Ping(Vec::new().into())).await.is_err(){ break; }
            }
        }
    }
}

async fn replay_to_socket(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Option<Uuid>,
    mut after: i64,
    sender: &mut SplitSink<WebSocket,Message>,
) -> Result<i64,String> {
    let mut delivered=0usize;
    loop {
        let batch=events_after(state,user_id,mailbox_id,after,DEFAULT_REPLAY_PAGE)
            .await.map_err(|e|e.to_string())?;
        if batch.is_empty(){ return Ok(after); }
        for event in &batch {
            if send_event(sender,event).await.is_err(){ return Err("websocket closed during replay".into()); }
            after=event.seq;
            delivered+=1;
            if delivered>=MAX_REPLAY_EVENTS {
                let (_,latest)=cursor_bounds(state,user_id,mailbox_id).await.map_err(|e|e.to_string())?;
                send_control(sender,latest,"resync-required",json!({"reason":"replay_limit","cursor":latest}))
                    .await.map_err(|_|"websocket closed during resync notification".to_string())?;
                return Ok(latest);
            }
        }
        if batch.len()<DEFAULT_REPLAY_PAGE as usize { return Ok(after); }
    }
}

async fn send_event(
    sender: &mut SplitSink<WebSocket, Message>,
    event: &RealtimeEvent,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&event.wire()).unwrap_or_else(|_| "{}".to_string());
    sender.send(Message::Text(text)).await
}

async fn send_control(
    sender: &mut SplitSink<WebSocket, Message>,
    seq: i64,
    kind: &str,
    payload: Value,
) -> Result<(), axum::Error> {
    let event = json!({
        "seq": seq,
        "kind": kind,
        "payload": payload,
        "created_at": Utc::now()
    });
    sender.send(Message::Text(event.to_string())).await
}

fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        // Non-browser clients do not always send Origin. Authentication is
        // still required through the HttpOnly session cookie.
        return true;
    };

    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    state.cors_origins.iter().any(|allowed| {
        allowed == "*" || allowed.trim_end_matches('/') == origin.trim_end_matches('/')
    }) || state.public_origin.trim_end_matches('/') == origin.trim_end_matches('/')
}

async fn session_is_active(state: &AppState, user: &AuthUser) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
           SELECT 1
           FROM sessions s
           JOIN users u ON u.id = s.user_id
           WHERE s.id = $1 AND s.user_id = $2
             AND s.revoked_at IS NULL AND s.expires_at > now()
             AND u.status = 'active'
         )",
    )
    .bind(user.session_id)
    .bind(user.user_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false)
}


async fn mailbox_context_is_active(state: &AppState, user: &AuthUser, mailbox_id: Option<Uuid>) -> bool {
    let Some(mailbox_id)=mailbox_id else { return true; };
    matches!(
        tenancy::active_mailbox(&state.db,user.user_id,None,Some(mailbox_id)).await,
        Ok(Some(mailbox)) if mailbox.id==mailbox_id && mailbox.status=="active"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_expired_cursor_gap() {
        assert!(!cursor_has_gap(0, 25));
        assert!(!cursor_has_gap(24, 25));
        assert!(!cursor_has_gap(25, 25));
        assert!(cursor_has_gap(10, 25));
    }

    #[test]
    fn query_state_error_detection_is_narrow() {
        assert!(query_state_expired("cannotCalculateChanges"));
        assert!(query_state_expired("invalidArguments: bad query state"));
        assert!(!query_state_expired("temporary network timeout"));
    }
}
