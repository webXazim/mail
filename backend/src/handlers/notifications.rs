use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::tenancy;
use crate::state::AppState;
use crate::ws::emit_event;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NotificationRow {
    id: Uuid,
    kind: String,
    title: String,
    detail: String,
    action_url: String,
    read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    before: Option<DateTime<Utc>>,
}
fn default_limit() -> i64 { 50 }

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Option<Uuid>, ApiError> {
    Ok(tenancy::active_mailbox(&state.db,auth.user_id,auth.organization_id_hint,auth.mailbox_id_hint).await?.map(|m|m.id))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.clamp(1, 100);
    let mailbox_id=active_mailbox_id(&state,&auth).await?;
    let rows: Vec<NotificationRow> = sqlx::query_as(
        "SELECT id, kind, title, detail, action_url, read_at, created_at
         FROM user_notifications
         WHERE user_id=$1 AND dismissed_at IS NULL
           AND (mailbox_id IS NULL OR mailbox_id=$2)
           AND ($3::timestamptz IS NULL OR created_at < $3)
         ORDER BY created_at DESC,id DESC LIMIT $4",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(query.before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let has_more = rows.len() as i64 > limit;
    let mut rows = rows;
    if has_more { rows.truncate(limit as usize); }
    let next_before = rows.last().map(|row| row.created_at);
    let unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_notifications
         WHERE user_id=$1 AND dismissed_at IS NULL AND read_at IS NULL
           AND (mailbox_id IS NULL OR mailbox_id=$2)",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "notifications": rows,
        "unread": unread,
        "has_more": has_more,
        "next_before": next_before,
    })))
}

pub async fn mark_read(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id=active_mailbox_id(&state,&auth).await?;
    let changed = sqlx::query(
        "UPDATE user_notifications SET read_at=COALESCE(read_at,now())
         WHERE id=$1 AND user_id=$2 AND dismissed_at IS NULL
           AND (mailbox_id IS NULL OR mailbox_id=$3)",
    )
    .bind(id)
    .bind(auth.user_id)
    .bind(mailbox_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if changed.rows_affected() == 0 { return Err(ApiError::not_found("Notification not found")); }
    let _ = emit_event(&state, auth.user_id, "resource-changed", json!({"resource":"notifications"})).await;
    Ok(Json(json!({"ok": true})))
}

pub async fn mark_all_read(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id=active_mailbox_id(&state,&auth).await?;
    sqlx::query(
        "UPDATE user_notifications SET read_at=COALESCE(read_at,now())
         WHERE user_id=$1 AND dismissed_at IS NULL AND read_at IS NULL
           AND (mailbox_id IS NULL OR mailbox_id=$2)",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let _ = emit_event(&state, auth.user_id, "resource-changed", json!({"resource":"notifications"})).await;
    Ok(Json(json!({"ok": true})))
}

pub async fn dismiss(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id=active_mailbox_id(&state,&auth).await?;
    let changed = sqlx::query(
        "UPDATE user_notifications SET dismissed_at=now(),read_at=COALESCE(read_at,now())
         WHERE id=$1 AND user_id=$2 AND dismissed_at IS NULL
           AND (mailbox_id IS NULL OR mailbox_id=$3)",
    )
    .bind(id)
    .bind(auth.user_id)
    .bind(mailbox_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if changed.rows_affected() == 0 { return Err(ApiError::not_found("Notification not found")); }
    let _ = emit_event(&state, auth.user_id, "resource-changed", json!({"resource":"notifications"})).await;
    Ok(Json(json!({"ok": true})))
}
