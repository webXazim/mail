//! WS3.3: scheduled sends. Each row holds a full compose payload (the same
//! shape `POST /api/send` accepts) plus a delivery time. A worker spawned from
//! `main.rs` delivers due rows through the shared `handlers::send::deliver`.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::send::{deliver, ComposeIn};
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const COLUMNS: &str = "id, send_at, compose";

#[derive(sqlx::FromRow)]
struct ScheduledRow {
    id: Uuid,
    send_at: DateTime<Utc>,
    compose: Value,
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
    })
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let rows: Vec<ScheduledRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM scheduled_sends
         WHERE user_id = $1 AND status = 'pending' ORDER BY send_at"
    ))
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "scheduled": rows.iter().map(row_to_json).collect::<Vec<_>>()
    })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ScheduleIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let compose = serde_json::to_value(&body.compose)
        .map_err(|e| ApiError::internal(format!("Invalid compose payload: {e}")))?;

    let row = sqlx::query_as::<_, ScheduledRow>(&format!(
        "INSERT INTO scheduled_sends (user_id, send_at, compose)
         VALUES ($1, $2, $3) RETURNING {COLUMNS}"
    ))
    .bind(auth.user_id)
    .bind(body.send_at)
    .bind(compose)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.schedule.create",
        json!({ "id": row.id, "send_at": row.send_at }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let deleted = sqlx::query("DELETE FROM scheduled_sends WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();

    if deleted == 0 {
        return Err(ApiError::not_found("Scheduled message not found"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.schedule.cancel",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

struct DueSend {
    id: Uuid,
    user_id: Uuid,
    compose: Value,
}

async fn due(state: &AppState, limit: i64) -> Result<Vec<DueSend>, ApiError> {
    let rows: Vec<(Uuid, Uuid, Value)> = sqlx::query_as(
        "SELECT id, user_id, compose FROM scheduled_sends
         WHERE status = 'pending' AND send_at <= now()
         ORDER BY send_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|(id, user_id, compose)| DueSend {
            id,
            user_id,
            compose,
        })
        .collect())
}

async fn finish(state: &AppState, id: Uuid, sent_id: Option<&str>, error: Option<&str>) {
    let (status, sent_id, error) = match error {
        Some(e) => ("failed", "", e),
        None => ("sent", sent_id.unwrap_or(""), ""),
    };
    if let Err(e) = sqlx::query(
        "UPDATE scheduled_sends
            SET status = $2, sent_id = $3, error = $4, updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(sent_id)
    .bind(error)
    .execute(&state.db)
    .await
    {
        tracing::warn!(%id, "scheduled_sends status update failed: {e}");
    }
}

/// Poll pending scheduled sends and deliver those that are due. One task per
/// process; the API runs single-instance so there is no claim race to worry
/// about (a multi-instance deploy would need `FOR UPDATE SKIP LOCKED`).
pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await;
        loop {
            tick.tick().await;
            let pending = match due(&state, 20).await {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!("scheduled send scan failed: {e}");
                    continue;
                }
            };

            for row in pending {
                let account: Option<(String,)> =
                    sqlx::query_as("SELECT email FROM users WHERE id = $1")
                        .bind(row.user_id)
                        .fetch_optional(&state.db)
                        .await
                        .unwrap_or(None);
                let Some((email,)) = account else {
                    finish(&state, row.id, None, Some("Account no longer exists")).await;
                    continue;
                };

                let compose: ComposeIn = match serde_json::from_value(row.compose) {
                    Ok(compose) => compose,
                    Err(e) => {
                        let message = format!("Malformed compose payload: {e}");
                        finish(&state, row.id, None, Some(&message)).await;
                        continue;
                    }
                };

                match deliver(&state, row.user_id, &email, compose, None).await {
                    Ok(outcome) => {
                        finish(&state, row.id, outcome.sent_id.as_deref(), None).await;
                        audit::record(
                            &state,
                            Some(row.user_id),
                            "mail.schedule.send",
                            json!({ "id": row.id, "message_id": outcome.message_id }),
                        )
                        .await;
                    }
                    Err(e) => {
                        let message = e.to_string();
                        tracing::warn!(id = %row.id, "scheduled send failed: {message}");
                        finish(&state, row.id, None, Some(&message)).await;
                    }
                }
            }
        }
    });
}
