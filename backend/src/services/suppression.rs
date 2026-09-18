//! Suppression list storage and one-click unsubscribe tokens (WS7.2).

use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::suppression as policy;
use crate::error::ApiError;
use crate::state::AppState;

/// Ten years: an unsubscribe link must not rot in an old mailbox.
const UNSUB_TTL_SECS: usize = 10 * 365 * 24 * 3600;

#[derive(Serialize, Deserialize)]
struct UnsubClaims {
    email: String,
    kind: String,
    exp: usize,
}

pub fn unsubscribe_token(secret: &str, email: &str) -> String {
    let exp = Utc::now().timestamp() as usize + UNSUB_TTL_SECS;
    let claims = UnsubClaims {
        email: policy::normalize(email),
        kind: "unsubscribe".into(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("unsubscribe token encoding cannot fail")
}

pub fn verify_unsubscribe_token(secret: &str, token: &str) -> Option<String> {
    let data = decode::<UnsubClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    if data.claims.kind == "unsubscribe" {
        Some(data.claims.email)
    } else {
        None
    }
}

/// RFC 8058 one-click target for a single recipient.
pub fn unsubscribe_url(state: &AppState, email: &str) -> String {
    let token = unsubscribe_token(&state.jwt_secret, email);
    format!(
        "{}/api/unsubscribe?token={token}",
        state.public_origin.trim_end_matches('/')
    )
}

pub async fn suppress(
    state: &AppState,
    email: &str,
    reason: &str,
    source: &str,
    detail: &str,
) -> Result<(), ApiError> {
    let email = policy::normalize(email);
    if email.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO suppressed_addresses (email, reason, source, detail)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (email) DO UPDATE
         SET reason = EXCLUDED.reason, source = EXCLUDED.source, detail = EXCLUDED.detail",
    )
    .bind(&email)
    .bind(reason)
    .bind(source)
    .bind(detail)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

pub async fn unsuppress(state: &AppState, email: &str) -> Result<bool, ApiError> {
    let removed = sqlx::query("DELETE FROM suppressed_addresses WHERE email = $1")
        .bind(policy::normalize(email))
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();
    Ok(removed > 0)
}

pub async fn list(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<(String, String, String, String, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT email::text, reason, source, detail, created_at
         FROM suppressed_addresses
         ORDER BY created_at DESC
         LIMIT 1000",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(email, reason, source, detail, created_at)| {
            json!({
                "email": email,
                "reason": reason,
                "source": source,
                "detail": detail,
                "created_at": created_at,
            })
        })
        .collect())
}

/// The subset of `emails` currently on the suppression list (normalised keys).
pub async fn suppressed(
    state: &AppState,
    emails: &[String],
) -> Result<std::collections::HashSet<String>, ApiError> {
    if emails.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let keys: Vec<String> = emails.iter().map(|e| policy::normalize(e)).collect();
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT email::text FROM suppressed_addresses WHERE email::text = ANY($1)")
            .bind(&keys)
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(|(e,)| e).collect())
}

/// Record a delivery-status report. Only hard failures (5.x) persist; soft
/// bounces are ignored so a transient greylisting never blocks an address.
/// Returns true when the address was suppressed.
pub async fn record_bounce(
    state: &AppState,
    address: &str,
    status: &str,
    diagnostic: &str,
) -> Result<bool, ApiError> {
    if !policy::is_hard_bounce(status) {
        return Ok(false);
    }
    let detail = format!("{status} {diagnostic}").trim().to_string();
    suppress(state, address, "hard bounce", "bounce", &detail).await?;
    Ok(true)
}

/// Seed the suppression list from a raw DSN body (RFC 3464), returning the
/// number of newly-suppressed addresses.
pub async fn ingest_dsn(state: &AppState, body: &str) -> Result<usize, ApiError> {
    let mut count = 0;
    for addr in policy::parse_dsn(body) {
        suppress(state, &addr, "hard bounce", "dsn", "").await?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trips_and_rejects_wrong_kind() {
        let secret = "0123456789abcdef0123456789abcdef";
        let token = unsubscribe_token(secret, "Bob@Example.com");
        assert_eq!(
            verify_unsubscribe_token(secret, &token),
            Some("bob@example.com".to_string())
        );
        assert_eq!(
            verify_unsubscribe_token("other-secret-key-000000000000", &token),
            None
        );
        assert_eq!(verify_unsubscribe_token(secret, "not-a-token"), None);
    }
}
