//! Account erasure (WS5.4). Two entry points — self-service (password
//! re-confirmation) and admin — share one `erase` routine: destroy the
//! Stalwart mailbox, then delete the user row so every FK-owned record
//! (sessions, contacts, calendar, settings, drafts, aliases, counters)
//! cascades away. Audit rows survive with `actor_id` NULLed.

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::imap;
use crate::state::AppState;

/// Destroy a user's mailbox in the mail store and drop their metadata row.
pub async fn erase(
    state: &AppState,
    user_id: Uuid,
    account_id: Option<String>,
) -> Result<(), ApiError> {
    if let Some(account) = account_id.filter(|a| !a.is_empty()) {
        imap::destroy_account(&state.mail, &account)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "mail_store", e))?;
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

#[derive(Deserialize)]
pub struct DeleteIn {
    password: String,
}

/// `POST /api/account/delete` — irreversible self-service erasure. Requires the
/// current password so a stolen access token alone cannot wipe the account.
pub async fn delete_self(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DeleteIn>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT password_hash, mail_account_id FROM users WHERE id = $1")
            .bind(auth.user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    let (password_hash, account_id) =
        row.ok_or_else(|| ApiError::not_found("Account not found"))?;

    let parsed =
        PasswordHash::new(&password_hash).map_err(|e| ApiError::internal(e.to_string()))?;
    if Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(ApiError::unauthorized("Password is incorrect"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "account.erase",
        json!({ "self": true }),
    )
    .await;

    erase(&state, auth.user_id, account_id).await?;
    Ok(Json(json!({ "ok": true })))
}
