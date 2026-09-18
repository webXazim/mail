use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let row: Option<(Value,)> = sqlx::query_as("SELECT payload FROM settings WHERE user_id = $1")
        .bind(auth.user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let settings = row.map(|(p,)| p).unwrap_or_else(|| json!({}));
    Ok(Json(settings))
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !payload.is_object() {
        return Err(ApiError::bad_request(
            "Settings payload must be a JSON object",
        ));
    }

    sqlx::query(
        "INSERT INTO settings (user_id, payload) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET payload = EXCLUDED.payload, updated_at = now()",
    )
    .bind(auth.user_id)
    .bind(&payload)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "settings.update", json!({})).await;
    Ok(Json(payload))
}
