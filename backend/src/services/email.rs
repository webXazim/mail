use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::state::AppState;

const VERIFY_TTL_SECS: i64 = 24 * 3600;
const RESET_TTL_SECS: i64 = 15 * 60;

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Opaque single-use token: 256 bits of hex, stored only as a sha256 hash.
pub fn random_token() -> String {
    let a = Uuid::new_v4().as_simple().to_string();
    let b = Uuid::new_v4().as_simple().to_string();
    format!("{a}{b}")
}

async fn persist_token(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    token: &str,
    ttl_secs: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO email_tokens (user_id, token_hash, kind, expires_at)
         VALUES ($1, $2, $3, now() + interval '1 second' * $4)",
    )
    .bind(user_id)
    .bind(token_hash(token))
    .bind(kind)
    .bind(ttl_secs)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Issue a verification token and deliver the email.
/// Returns the click-through URL (to the SPA /verify-email route).
/// TODO(WS2): replace the log-only delivery with real SMTP submission once the
/// Stalwart bridge exists. `HARBOR_DEV_RETURN_TOKEN_LINKS` then goes away.
pub async fn send_verification(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let token = random_token();
    persist_token(state, user_id, "verify", &token, VERIFY_TTL_SECS).await?;
    let link = format!("{}/verify-email?token={token}", state.public_origin);
    deliver(state, "Verify your email", email, display_name, &link);
    audit::record(state, Some(user_id), "auth.verify.token_issued", json!({})).await;
    Ok(link)
}

pub async fn send_password_reset(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let token = random_token();
    persist_token(state, user_id, "reset", &token, RESET_TTL_SECS).await?;
    let link = format!("{}/reset-password?token={token}", state.public_origin);
    deliver(state, "Reset your password", email, display_name, &link);
    audit::record(
        state,
        Some(user_id),
        "auth.password_reset.token_issued",
        json!({}),
    )
    .await;
    Ok(link)
}

fn deliver(_state: &AppState, subject: &str, email: &str, display_name: &str, link: &str) {
    // Placeholder until SMTP is wired (WS2). Log at info so the URL is
    // discoverable in development when the token isn't echoed back.
    tracing::info!(
        email = %email,
        display_name = %display_name,
        subject = %subject,
        link = %link,
        "auth email prepared (SMTP bridge not yet configured)"
    );
}
