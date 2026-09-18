use axum::http::StatusCode;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::services::{mime, smtp};
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
pub async fn send_verification(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let token = random_token();
    persist_token(state, user_id, "verify", &token, VERIFY_TTL_SECS).await?;
    let link = format!("{}/verify-email?token={token}", state.public_origin);
    let subject = if display_name.is_empty() {
        "Verify your email".into()
    } else {
        format!("Verify your email, {display_name}")
    };
    deliver(state, &subject, email, display_name, &link).await?;
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
    let subject = if display_name.is_empty() {
        "Reset your password".into()
    } else {
        format!("Reset your password, {display_name}")
    };
    deliver(state, &subject, email, display_name, &link).await?;
    audit::record(
        state,
        Some(user_id),
        "auth.password_reset.token_issued",
        json!({}),
    )
    .await;
    Ok(link)
}

/// Build and submit a one-off transactional email via the configured SMTP
/// relay (the Stalwart bridge files it into the recipient's mailbox).
/// The envelope + header sender is the recipient's own (already provisioned)
/// Stalwart account, so an unauthenticated relay accepts it as local->local
/// delivery just like the regular send path.
/// Delivery is best-effort in development (`return_token_links` echoes the
/// link back to the client anyway) and load-bearing in production, where a
/// failed submission surfaces a 502 so the client can retry.
async fn deliver(
    state: &AppState,
    subject: &str,
    to_email: &str,
    display_name: &str,
    link: &str,
) -> Result<(), ApiError> {
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("Harbor Mail".to_string()),
            email: to_email.to_string(),
        },
        to: vec![mime::Address {
            name: (!display_name.is_empty()).then(|| display_name.to_string()),
            email: to_email.to_string(),
        }],
        cc: vec![],
        subject: subject.to_string(),
        body_text: format!(
            "Use this link to continue:\n\n{link}\n\nIf you did not request this, you can ignore this email.\n"
        ),
        body_html: Some(format!(
            "<p>Use the button below to continue:</p>\
             <p><a href=\"{link}\">{link}</a></p>\
             <p>If you did not request this, you can ignore this email.</p>\n"
        )),
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.mail.default_domain.clone(),
    };
    let bytes = outgoing
        .build()
        .map_err(|e| ApiError::internal(format!("MIME build failed: {e}")))?;

    match smtp::send(&state.smtp, to_email, &[to_email.to_string()], &bytes).await {
        Ok(reply) => {
            tracing::info!(
                email = %to_email,
                subject = %subject,
                reply = %reply,
                "transactional email submitted via SMTP"
            );
            Ok(())
        }
        Err(e) if state.return_token_links => {
            // Dev-only fallback: the link is echoed in the API response, so a
            // missing relay must not block local flows. Log loudly instead.
            tracing::warn!(email = %to_email, subject = %subject, error = %e,
                "SMTP unavailable; transactional email skipped (development mode)");
            Ok(())
        }
        Err(e) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "smtp_submission",
            format!("Unable to deliver transactional email: {e}"),
        )),
    }
}
