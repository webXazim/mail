use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::tenancy;
use crate::state::AppState;

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .map(|mailbox| mailbox.id)
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before editing mail settings"))
}

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<(Value,)> = sqlx::query_as("SELECT payload FROM mailbox_settings WHERE mailbox_id = $1")
        .bind(mailbox_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut settings = row.map(|(p,)| p).unwrap_or_else(|| json!({}));
    if let Some(object) = settings.as_object_mut() {
        object.remove("twoFactor");
    }
    Ok(Json(settings))
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(mut payload): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    if !payload.is_object() {
        return Err(ApiError::bad_request(
            "Settings payload must be a JSON object",
        ));
    }

    if let Some(object) = payload.as_object_mut() {
        object.remove("twoFactor");
    }

    sqlx::query(
        "INSERT INTO mailbox_settings (mailbox_id, user_id, payload) VALUES ($1, $2, $3)
         ON CONFLICT (mailbox_id) DO UPDATE SET
             user_id = EXCLUDED.user_id,
             payload = EXCLUDED.payload,
             updated_at = now()",
    )
    .bind(mailbox_id)
    .bind(auth.user_id)
    .bind(&payload)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "settings.update", json!({ "mailbox_id": mailbox_id })).await;
    Ok(Json(payload))
}
