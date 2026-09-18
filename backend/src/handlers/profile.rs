use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::domain::quota;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::billing;
use crate::state::AppState;

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, String, String, bool, i64, String)> = sqlx::query_as(
        "SELECT email, display_name, role, onboarded, quota_bytes, plan FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (email, display_name, role, onboarded, quota_bytes, plan_code) =
        row.ok_or_else(|| ApiError::not_found("User not found"))?;

    // WS4: limits come from the admin-editable `plans` table.
    let plan = billing::for_user(&state, auth.user_id).await?;
    let total = quota_bytes.max(0) as u64;
    let used = 0u64;

    Ok(Json(json!({
        "id": auth.user_id,
        "email": email,
        "display_name": display_name,
        "role": role,
        "onboarded": onboarded,
        "plan": plan_code,
        "plan_name": plan.name,
        "storage": {
            "used_bytes": used,
            "total_bytes": quota_bytes,
            "pct": (quota::used_ratio(used, total) * 100.0).round() as i64
        },
        "limits": {
            "max_attachment_bytes": plan.max_attachment_bytes,
            "max_total_attachment_bytes": plan.max_total_attachment_bytes,
            "mailbox_bytes": plan.mailbox_bytes,
            "max_recipients": plan.max_recipients,
            "daily_send_limit": plan.daily_send_limit,
            "seats": plan.seats
        }
    })))
}

#[derive(Deserialize)]
pub struct UpdateIn {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    onboarded: Option<bool>,
}

/// Bootstrap fields the client may set on first run: the display name shown as
/// the From identity and the `onboarded` flag that dismisses first-run setup.
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateIn>,
) -> Result<Json<Value>, ApiError> {
    if let Some(raw) = body.display_name {
        let name = raw.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request("Display name is required"));
        }
        if name.chars().count() > 80 {
            return Err(ApiError::bad_request("Display name is too long"));
        }
        sqlx::query("UPDATE users SET display_name = $1, updated_at = now() WHERE id = $2")
            .bind(name)
            .bind(auth.user_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    if body.onboarded == Some(true) {
        sqlx::query("UPDATE users SET onboarded = TRUE, updated_at = now() WHERE id = $1")
            .bind(auth.user_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    get(State(state), auth).await
}
