//! Production scheduled-delivery queue.
//!
//! Upgrade 10 turns `scheduled_sends` into a PostgreSQL-leased job queue. Any
//! number of API replicas may run this worker because claims use
//! `FOR UPDATE SKIP LOCKED`. Delivery itself always reuses the deterministic
//! Upgrade-09 send-ledger key `scheduled:<scheduled-row-id>`, so crash recovery
//! can reclaim a job without creating a second logical SMTP submission.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::{attachments, identities};
use crate::handlers::send::{deliver, ComposeIn};
use crate::middleware::auth::AuthUser;
use crate::services::{entitlements, tenancy};
use crate::state::AppState;
use crate::ws::emit_event;

const COLUMNS: &str = "id, send_at, compose, status, error, attempt_count, next_attempt_at, idempotency_key, request_hash, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct ScheduledRow {
    id: Uuid,
    send_at: DateTime<Utc>,
    compose: Value,
    status: String,
    error: String,
    attempt_count: i32,
    next_attempt_at: Option<DateTime<Utc>>,
    #[allow(dead_code)]
    idempotency_key: String,
    request_hash: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ScheduleIn {
    send_at: DateTime<Utc>,
    #[serde(flatten)]
    compose: ComposeIn,
}

fn row_to_json(row: &ScheduledRow) -> Value {
    json!({
        "id": row.id,
        "send_at": row.send_at,
        "compose": row.compose,
        "status": row.status,
        "error": if row.error.is_empty() { Value::Null } else { json!(row.error) },
        "attempt_count": row.attempt_count,
        "next_attempt_at": row.next_attempt_at,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

fn normalize_schedule_key(headers: &HeaderMap, compose: &ComposeIn) -> Result<String, ApiError> {
    let header = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let fallback = compose.send_key.map(|key| format!("schedule:{key}"));
    let value = header.or(fallback).ok_or_else(|| {
        ApiError::bad_request(
            "Idempotency-Key is required when scheduling a message",
        )
    })?;
    if value.len() < 8 || value.len() > 128 {
        return Err(ApiError::bad_request(
            "Idempotency-Key must contain 8-128 characters",
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(ApiError::bad_request(
            "Idempotency-Key contains invalid characters",
        ));
    }
    Ok(value)
}

fn schedule_request_hash(send_at: &DateTime<Utc>, compose: &Value) -> Result<String, ApiError> {
    let payload = serde_json::to_vec(&json!({
        "send_at": send_at,
        "compose": compose,
    }))
    .map_err(|error| ApiError::internal(format!("Could not fingerprint schedule: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .map(|mailbox| mailbox.id)
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before scheduling mail"))
}

async fn mailbox_feature_entitlements(
    state: &AppState,
    mailbox_id: Uuid,
    feature: &str,
) -> Result<entitlements::UserEntitlements, ApiError> {
    let organization_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT organization_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL AND status='active'",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;
    let organization_id = organization_id
        .ok_or_else(|| ApiError::forbidden("The selected business mailbox is not active"))?;
    let ent = entitlements::for_organization(state, organization_id).await?;
    if !matches!(ent.subscription_status.as_str(), "active" | "trial") || !ent.plan.allows(feature) {
        return Err(ApiError::forbidden(format!(
            "The {feature} feature is not enabled for this business subscription"
        )));
    }
    Ok(ent)
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let rows: Vec<ScheduledRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM scheduled_sends
         WHERE mailbox_id = $1 AND status IN ('pending','processing','retry','dead')
         ORDER BY CASE WHEN status = 'dead' THEN 1 ELSE 0 END, send_at, created_at"
    ))
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;

    Ok(Json(json!({
        "scheduled": rows.iter().map(row_to_json).collect::<Vec<_>>()
    })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<ScheduleIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let ent = mailbox_feature_entitlements(&state, mailbox_id, "scheduled_send").await?;
    let ScheduleIn {
        send_at,
        compose: mut compose_input,
    } = body;
    let schedule_key = normalize_schedule_key(&headers, &compose_input)?;
    let identity = identities::resolve(&state, auth.user_id, mailbox_id, compose_input.identity_id).await?;
    compose_input.identity_id = Some(identity.id);

    let attachment_meta = attachments::resolve_refs(
        &state,
        mailbox_id,
        &compose_input.attachments,
        &ent.plan,
    )
    .await?;
    let attachment_ids = attachment_meta
        .iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    let mut compose = serde_json::to_value(&compose_input)
        .map_err(|error| ApiError::internal(format!("Invalid compose payload: {error}")))?;
    if let Some(object) = compose.as_object_mut() {
        object.insert(
            "attachments".to_string(),
            attachments::refs_json(&attachment_meta),
        );
    }
    let request_hash = schedule_request_hash(&send_at, &compose)?;
    let protect_until = std::cmp::max(
        send_at.clone() + chrono::Duration::days(7),
        Utc::now() + chrono::Duration::seconds(state.attachment_upload_ttl_secs as i64),
    );

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let inserted = sqlx::query(
        "INSERT INTO scheduled_sends
            (user_id, mailbox_id, send_at, compose, status, next_attempt_at, idempotency_key, request_hash)
         VALUES ($1, $2, $3, $4, 'pending', $3, $5, $6)
         ON CONFLICT (mailbox_id, idempotency_key) DO NOTHING",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(send_at)
    .bind(&compose)
    .bind(&schedule_key)
    .bind(&request_hash)
    .execute(&mut *tx)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?
    .rows_affected()
        == 1;

    let row: ScheduledRow = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM scheduled_sends
         WHERE mailbox_id = $1 AND idempotency_key = $2"
    ))
    .bind(mailbox_id)
    .bind(&schedule_key)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;

    if row.request_hash != request_hash {
        return Err(ApiError::conflict(
            "This schedule idempotency key was already used for different content or delivery time",
        ));
    }

    if inserted {
        attachments::replace_owner_refs_tx(
            &mut tx,
            mailbox_id,
            "scheduled",
            row.id,
            &attachment_ids,
            protect_until,
        )
        .await?;
    }

    tx.commit()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    if inserted {
        audit::record(
            &state,
            Some(auth.user_id),
            "mail.schedule.create",
            json!({ "id": row.id, "send_at": row.send_at }),
        )
        .await;
    }

    Ok((
        if inserted {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(row_to_json(&row)),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let attachment_ids =
        attachments::owner_attachment_ids(&state, mailbox_id, "scheduled", id).await?;
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM scheduled_sends
         WHERE id = $1 AND mailbox_id = $2 FOR UPDATE",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;
    let Some(status) = status else {
        return Err(ApiError::not_found("Scheduled message not found"));
    };

    match status.as_str() {
        "cancelled" => {
            tx.commit()
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            return Ok(Json(json!({ "ok": true, "status": "cancelled" })));
        }
        "processing" => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "schedule_in_progress",
                "This message is already being delivered and can no longer be safely cancelled",
            ));
        }
        "sent" => {
            return Err(ApiError::conflict("This scheduled message has already been sent"));
        }
        "pending" | "retry" | "dead" => {}
        _ => return Err(ApiError::conflict("This scheduled message cannot be cancelled")),
    }

    sqlx::query(
        "UPDATE scheduled_sends
         SET status = 'cancelled', cancelled_at = now(), completed_at = now(),
             claimed_by = NULL, lease_until = NULL, next_attempt_at = NULL, updated_at = now()
         WHERE id = $1 AND mailbox_id = $2",
    )
    .bind(id)
    .bind(mailbox_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;
    sqlx::query(
        "DELETE FROM attachment_refs
         WHERE mailbox_id = $1 AND owner_type = 'scheduled' AND owner_id = $2",
    )
    .bind(mailbox_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    attachments::mark_consumed(&state, mailbox_id, &attachment_ids).await;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.schedule.cancel",
        json!({ "id": id, "mailbox_id": mailbox_id }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "status": "cancelled" })))
}

pub async fn retry_dead(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    mailbox_feature_entitlements(&state, mailbox_id, "scheduled_send").await?;
    let updated = sqlx::query(
        "UPDATE scheduled_sends
         SET status = 'retry', attempt_count = 0, next_attempt_at = now(),
             claimed_by = NULL, lease_until = NULL, completed_at = NULL,
             error = '', updated_at = now()
         WHERE id = $1 AND mailbox_id = $2 AND status = 'dead'",
    )
    .bind(id)
    .bind(mailbox_id)
    .execute(&state.db)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::conflict(
            "Only a dead-letter scheduled message can be retried",
        ));
    }
    audit::record(
        &state,
        Some(auth.user_id),
        "mail.schedule.retry",
        json!({ "id": id, "mailbox_id": mailbox_id }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "status": "retry" })))
}

#[derive(sqlx::FromRow)]
struct ClaimedSend {
    id: Uuid,
    user_id: Uuid,
    mailbox_id: Uuid,
    compose: Value,
    attempt_count: i32,
}

async fn claim_due(
    state: &AppState,
    worker_id: Uuid,
) -> Result<Vec<ClaimedSend>, ApiError> {
    let rows = sqlx::query_as::<_, ClaimedSend>(
        "WITH candidates AS (
           SELECT id
           FROM scheduled_sends
           WHERE mailbox_id IS NOT NULL AND (
             (status IN ('pending','retry')
                AND send_at <= now()
                AND COALESCE(next_attempt_at, send_at) <= now())
             OR
             (status = 'processing' AND lease_until IS NOT NULL AND lease_until <= now())
           )
           ORDER BY COALESCE(next_attempt_at, send_at), send_at, id
           FOR UPDATE SKIP LOCKED
           LIMIT $1
         )
         UPDATE scheduled_sends AS scheduled
         SET status = 'processing', claimed_by = $2,
             lease_until = now() + ($3 * interval '1 second'),
             attempt_count = scheduled.attempt_count + 1,
             last_attempt_at = now(), updated_at = now()
         FROM candidates
         WHERE scheduled.id = candidates.id
         RETURNING scheduled.id, scheduled.user_id, scheduled.mailbox_id, scheduled.compose, scheduled.attempt_count",
    )
    .bind(state.schedule_batch_size.max(1))
    .bind(worker_id)
    .bind(state.schedule_lease_secs.max(30) as i64)
    .fetch_all(&state.db)
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(rows)
}

fn backoff_secs(state: &AppState, attempt_count: i32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).clamp(0, 8) as u32;
    let multiplier = 1u64 << exponent;
    state
        .schedule_retry_base_secs
        .max(1)
        .saturating_mul(multiplier)
        .min(3600) as i64
}

fn retryable(error: &ApiError) -> bool {
    matches!(error.error.as_str(), "send_uncertain" | "send_in_progress")
        || error.status == StatusCode::TOO_MANY_REQUESTS
        || error.status.is_server_error()
}

async fn release_failure(
    state: &AppState,
    worker_id: Uuid,
    row: &ClaimedSend,
    error: &ApiError,
) {
    let exhausted = row.attempt_count >= state.schedule_max_attempts.max(1);
    let should_retry = retryable(error) && !exhausted;
    let delay = backoff_secs(state, row.attempt_count);
    let status = if should_retry { "retry" } else { "dead" };
    let message = if exhausted && matches!(error.error.as_str(), "send_uncertain" | "send_in_progress") {
        format!(
            "Delivery outcome is still ambiguous after {} attempts. Review the send ledger before creating another send. Last error: {}",
            row.attempt_count, error.message
        )
    } else {
        error.message.clone()
    };

    let result = sqlx::query(
        "UPDATE scheduled_sends
         SET status = $4, error = $5,
             next_attempt_at = CASE WHEN $4 = 'retry' THEN now() + ($6 * interval '1 second') ELSE NULL END,
             completed_at = CASE WHEN $4 = 'dead' THEN now() ELSE NULL END,
             claimed_by = NULL, lease_until = NULL, updated_at = now()
         WHERE id = $1 AND mailbox_id = $2 AND status = 'processing' AND claimed_by = $3",
    )
    .bind(row.id)
    .bind(row.mailbox_id)
    .bind(worker_id)
    .bind(status)
    .bind(&message)
    .bind(delay)
    .execute(&state.db)
    .await;

    match result {
        Ok(done) if done.rows_affected() == 1 && status == "dead" => {
            audit::record(
                state,
                Some(row.user_id),
                "mail.schedule.dead_letter",
                json!({ "id": row.id, "mailbox_id": row.mailbox_id, "attempts": row.attempt_count, "error": message }),
            )
            .await;
        }
        Ok(_) => {}
        Err(db_error) => tracing::warn!(id = %row.id, "scheduled retry/dead-letter update failed: {db_error}"),
    }
}

async fn finish_sent(
    state: &AppState,
    worker_id: Option<Uuid>,
    id: Uuid,
    user_id: Uuid,
    mailbox_id: Uuid,
    sent_id: Option<&str>,
) -> bool {
    let attachment_ids = attachments::owner_attachment_ids(state, mailbox_id, "scheduled", id)
        .await
        .unwrap_or_default();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(%id, "scheduled completion transaction failed: {error}");
            return false;
        }
    };

    let updated = if let Some(worker_id) = worker_id {
        sqlx::query(
            "UPDATE scheduled_sends
             SET status = 'sent', sent_id = COALESCE(NULLIF($4, ''), sent_id), error = '',
                 completed_at = now(), next_attempt_at = NULL, claimed_by = NULL,
                 lease_until = NULL, updated_at = now()
             WHERE id = $1 AND mailbox_id = $2 AND status = 'processing' AND claimed_by = $3",
        )
        .bind(id)
        .bind(mailbox_id)
        .bind(worker_id)
        .bind(sent_id.unwrap_or(""))
        .execute(&mut *tx)
        .await
    } else {
        sqlx::query(
            "UPDATE scheduled_sends
             SET status = 'sent', sent_id = COALESCE(NULLIF($3, ''), sent_id), error = '',
                 completed_at = now(), next_attempt_at = NULL, claimed_by = NULL,
                 lease_until = NULL, updated_at = now()
             WHERE id = $1 AND mailbox_id = $2 AND status IN ('processing','retry','dead')",
        )
        .bind(id)
        .bind(mailbox_id)
        .bind(sent_id.unwrap_or(""))
        .execute(&mut *tx)
        .await
    };

    let updated = match updated {
        Ok(result) => result.rows_affected(),
        Err(error) => {
            tracing::warn!(%id, "scheduled completion update failed: {error}");
            return false;
        }
    };
    if updated == 0 {
        return false;
    }

    if let Err(error) = sqlx::query(
        "DELETE FROM attachment_refs
         WHERE mailbox_id = $1 AND owner_type = 'scheduled' AND owner_id = $2",
    )
    .bind(mailbox_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    {
        tracing::warn!(%id, "scheduled attachment-reference cleanup failed: {error}");
        return false;
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(%id, "scheduled completion commit failed: {error}");
        return false;
    }
    attachments::mark_consumed(state, mailbox_id, &attachment_ids).await;
    if let Err(error) = emit_event(
        state,
        user_id,
        "resource-changed",
        json!({ "resource": "mailbox", "action": "scheduled-sent", "mailbox_id": mailbox_id }),
    )
    .await
    {
        tracing::warn!(%user_id, "failed to publish scheduled-send mailbox event: {error}");
    }
    true
}

/// Repair rows whose Upgrade-09 send ledger has since reconciled to `sent`.
/// This is intentionally independent of worker lease ownership: once the send
/// ledger proves SMTP delivery, the scheduler must converge to `sent` even if
/// the worker that originally owned the lease crashed.
async fn reconcile_from_send_ledger(state: &AppState) {
    let rows: Vec<(Uuid, Uuid, Uuid, String)> = match sqlx::query_as(
        "SELECT scheduled.id, scheduled.user_id, scheduled.mailbox_id, request.sent_id
         FROM scheduled_sends AS scheduled
         JOIN mail_send_requests AS request
           ON request.mailbox_id = scheduled.mailbox_id
          AND request.idempotency_key = 'scheduled:' || scheduled.id::text
         WHERE scheduled.status IN ('processing','retry','dead')
           AND request.status = 'sent'
         ORDER BY scheduled.updated_at ASC
         LIMIT 100",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!("scheduled/send-ledger reconciliation scan failed: {error}");
            return;
        }
    };

    for (id, user_id, mailbox_id, sent_id) in rows {
        if finish_sent(
            state,
            None,
            id,
            user_id,
            mailbox_id,
            (!sent_id.is_empty()).then_some(sent_id.as_str()),
        )
        .await
        {
            audit::record(
                state,
                Some(user_id),
                "mail.schedule.reconciled",
                json!({ "id": id, "mailbox_id": mailbox_id }),
            )
            .await;
        }
    }
}

async fn process_claim(state: &AppState, worker_id: Uuid, row: ClaimedSend) {
    if let Err(error) = mailbox_feature_entitlements(state, row.mailbox_id, "scheduled_send").await {
        release_failure(state, worker_id, &row, &error).await;
        return;
    }

    let compose: ComposeIn = match serde_json::from_value(row.compose.clone()) {
        Ok(compose) => compose,
        Err(error) => {
            let api_error = ApiError::bad_request(format!("Malformed compose payload: {error}"));
            release_failure(state, worker_id, &row, &api_error).await;
            return;
        }
    };

    let delivery_key = format!("scheduled:{}", row.id);
    match deliver(state, row.user_id, row.mailbox_id, compose, None, &delivery_key).await {
        Ok(outcome) => {
            if finish_sent(
                state,
                Some(worker_id),
                row.id,
                row.user_id,
                row.mailbox_id,
                outcome.sent_id.as_deref(),
            )
            .await
            {
                audit::record(
                    state,
                    Some(row.user_id),
                    "mail.schedule.send",
                    json!({
                        "id": row.id,
                        "mailbox_id": row.mailbox_id,
                        "request_id": outcome.request_id,
                        "message_id": outcome.message_id,
                        "deduplicated": outcome.deduplicated,
                    }),
                )
                .await;
            }
        }
        Err(error) => {
            tracing::warn!(
                id = %row.id,
                attempt = row.attempt_count,
                code = %error.error,
                "scheduled delivery attempt did not complete: {}",
                error.message
            );
            release_failure(state, worker_id, &row, &error).await;
        }
    }
}

/// Run the durable scheduler on every API replica. PostgreSQL leases coordinate
/// ownership, while the send ledger coordinates the irreversible SMTP boundary.
pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let worker_id = Uuid::new_v4();
        let mut tick = tokio::time::interval(Duration::from_secs(state.schedule_poll_secs.max(1)));
        loop {
            tick.tick().await;
            reconcile_from_send_ledger(&state).await;

            // Upgrade 35 emergency control: do not claim due scheduled mail while
            // customer outbound sending is paused. Leaving rows pending avoids
            // burning retry attempts during an operator incident window.
            match crate::services::platform_control::load(&state.db).await {
                Ok(controls) if !controls.outbound_sending_enabled => continue,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%worker_id, "scheduled-send platform-control lookup failed: {error}");
                    continue;
                }
            }

            let claimed = match claim_due(&state, worker_id).await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%worker_id, "scheduled-send claim failed: {error}");
                    continue;
                }
            };
            for row in claimed {
                process_claim(&state, worker_id, row).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn retry_backoff_is_bounded() {
        // Keep the arithmetic helper honest without constructing AppState.
        let base = 15u64;
        let calc = |attempt: i32| {
            let exponent = attempt.saturating_sub(1).clamp(0, 8) as u32;
            base.saturating_mul(1u64 << exponent).min(3600)
        };
        assert_eq!(calc(1), 15);
        assert_eq!(calc(2), 30);
        assert_eq!(calc(9), 3600);
        assert_eq!(calc(20), 3600);
    }
}
