use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    before: Option<DateTime<Utc>>,
    category: Option<String>,
}
fn default_limit() -> i64 { 50 }

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ActivityRow {
    id: Uuid,
    time: DateTime<Utc>,
    category: String,
    action: String,
    detail: String,
}

fn category_sql() -> &'static str {
    "CASE
       WHEN action LIKE 'auth.%' THEN CASE WHEN action LIKE 'auth.login%' THEN 'sign-in' ELSE 'security' END
       WHEN action LIKE 'billing.%' THEN 'billing'
       WHEN action LIKE 'mail.%' THEN 'mail'
       WHEN action LIKE 'calendar.%' OR action LIKE 'contacts.%' OR action LIKE 'settings.%' THEN 'general'
       ELSE 'general'
     END"
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.clamp(1, 100);
    let category = query.category.as_deref().filter(|v| matches!(*v, "sign-in" | "security" | "billing" | "mail" | "general"));
    let sql = format!(
        "SELECT id, at AS time, {category} AS category,
                CASE
                  WHEN action = 'auth.login' THEN 'Signed in'
                  WHEN action = 'auth.password_changed' THEN 'Password changed'
                  WHEN action = 'auth.password_reset' THEN 'Password reset'
                  WHEN action LIKE 'auth.2fa.%' THEN 'Two-factor authentication changed'
                  WHEN action = 'billing.order_create' THEN 'Plan order created'
                  WHEN action = 'billing.order_paid_submitted' THEN 'Payment submitted'
                  WHEN action = 'billing.order_cancel' THEN 'Plan order cancelled'
                  WHEN action = 'mail.schedule.create' THEN 'Scheduled message created'
                  WHEN action = 'mail.schedule.send' THEN 'Scheduled message sent'
                  WHEN action = 'mail.schedule.cancel' THEN 'Scheduled message cancelled'
                  WHEN action = 'settings.update' THEN 'Settings updated'
                  ELSE replace(initcap(replace(action, '.', ' ')), '_', ' ')
                END AS action,
                CASE
                  WHEN detail ? 'email' THEN detail->>'email'
                  WHEN detail ? 'plan' THEN 'Plan: ' || (detail->>'plan')
                  WHEN detail ? 'target' THEN detail->>'target'
                  WHEN detail ? 'id' THEN 'Reference ' || left(detail->>'id', 12)
                  ELSE ''
                END AS detail
         FROM audit_log
         WHERE actor_id = $1
           AND ($2::timestamptz IS NULL OR at < $2)
           AND ($3::text IS NULL OR ({category}) = $3)
         ORDER BY at DESC, id DESC LIMIT $4",
        category = category_sql(),
    );
    let mut rows: Vec<ActivityRow> = sqlx::query_as(&sql)
        .bind(auth.user_id)
        .bind(query.before)
        .bind(category)
        .bind(limit + 1)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let has_more = rows.len() as i64 > limit;
    if has_more { rows.truncate(limit as usize); }
    let next_before = rows.last().map(|row| row.time);
    Ok(Json(serde_json::json!({"entries": rows, "has_more": has_more, "next_before": next_before})))
}
