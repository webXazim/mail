//! Deliverability suppression authority (Upgrade 25).
//!
//! `suppressed_addresses` is intentionally platform-global and reserved for
//! emergency/legal blocks. Customer unsubscribe, bounce and complaint state is
//! stored in `recipient_suppressions` and scoped to a business/mailbox.

use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::domain::suppression as policy;
use crate::error::ApiError;
use crate::state::AppState;

const UNSUB_TTL_SECS: usize = 10 * 365 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubClaims {
    pub email: String,
    pub organization_id: Uuid,
    pub mailbox_id: Uuid,
    kind: String,
    exp: usize,
}

pub fn unsubscribe_token(
    secret: &str,
    organization_id: Uuid,
    mailbox_id: Uuid,
    email: &str,
) -> String {
    let exp = Utc::now().timestamp() as usize + UNSUB_TTL_SECS;
    let claims = UnsubClaims {
        email: policy::normalize(email),
        organization_id,
        mailbox_id,
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

pub fn verify_unsubscribe_token(secret: &str, token: &str) -> Option<UnsubClaims> {
    let data = decode::<UnsubClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    (data.claims.kind == "unsubscribe").then_some(data.claims)
}

pub fn unsubscribe_url(
    state: &AppState,
    organization_id: Uuid,
    mailbox_id: Uuid,
    email: &str,
) -> String {
    let token = unsubscribe_token(&state.jwt_secret, organization_id, mailbox_id, email);
    format!(
        "{}/api/unsubscribe?token={token}",
        state.public_origin.trim_end_matches('/')
    )
}

/// Platform-global emergency/legal block. Customer deliverability state must
/// use `suppress_for_scope` instead.
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

pub async fn suppress_for_scope(
    state: &AppState,
    organization_id: Uuid,
    mailbox_id: Option<Uuid>,
    email: &str,
    reason: &str,
    source: &str,
    detail: &str,
) -> Result<(), ApiError> {
    let email = policy::normalize(email);
    if email.is_empty() {
        return Ok(());
    }
    if let Some(mailbox_id) = mailbox_id {
        sqlx::query(
            "INSERT INTO recipient_suppressions(organization_id,mailbox_id,email,reason,source,detail)
             VALUES($1,$2,$3,$4,$5,$6)
             ON CONFLICT (mailbox_id,lower(email::text)) WHERE mailbox_id IS NOT NULL DO UPDATE SET
               reason=EXCLUDED.reason,source=EXCLUDED.source,detail=EXCLUDED.detail,updated_at=now()",
        )
        .bind(organization_id)
        .bind(mailbox_id)
        .bind(&email)
        .bind(reason)
        .bind(source)
        .bind(detail)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    } else {
        sqlx::query(
            "INSERT INTO recipient_suppressions(organization_id,mailbox_id,email,reason,source,detail)
             VALUES($1,NULL,$2,$3,$4,$5)
             ON CONFLICT (organization_id,lower(email::text)) WHERE mailbox_id IS NULL DO UPDATE SET
               reason=EXCLUDED.reason,source=EXCLUDED.source,detail=EXCLUDED.detail,updated_at=now()",
        )
        .bind(organization_id)
        .bind(&email)
        .bind(reason)
        .bind(source)
        .bind(detail)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
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
         FROM suppressed_addresses ORDER BY created_at DESC LIMIT 1000",
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
                "scope": "global",
            })
        })
        .collect())
}

/// Return recipients blocked either by the platform emergency list, by the
/// current business, or by the concrete mailbox sending the message.
pub async fn suppressed_for_mailbox(
    state: &AppState,
    organization_id: Uuid,
    mailbox_id: Uuid,
    emails: &[String],
) -> Result<std::collections::HashSet<String>, ApiError> {
    if emails.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let keys: Vec<String> = emails.iter().map(|e| policy::normalize(e)).collect();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT lower(email::text) FROM suppressed_addresses WHERE lower(email::text)=ANY($1)
         UNION
         SELECT lower(email::text) FROM recipient_suppressions
          WHERE organization_id=$2 AND (mailbox_id IS NULL OR mailbox_id=$3)
            AND lower(email::text)=ANY($1)",
    )
    .bind(&keys)
    .bind(organization_id)
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(|(email,)| email).collect())
}

/// Legacy global lookup retained for platform-admin tooling.
pub async fn suppressed(
    state: &AppState,
    emails: &[String],
) -> Result<std::collections::HashSet<String>, ApiError> {
    if emails.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let keys: Vec<String> = emails.iter().map(|e| policy::normalize(e)).collect();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT lower(email::text) FROM suppressed_addresses WHERE lower(email::text)=ANY($1)",
    )
    .bind(&keys)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(|(email,)| email).collect())
}

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
    fn scoped_token_round_trips() {
        let secret = "0123456789abcdef0123456789abcdef";
        let organization_id = Uuid::new_v4();
        let mailbox_id = Uuid::new_v4();
        let token = unsubscribe_token(secret, organization_id, mailbox_id, "Bob@Example.com");
        let claims = verify_unsubscribe_token(secret, &token).expect("valid token");
        assert_eq!(claims.email, "bob@example.com");
        assert_eq!(claims.organization_id, organization_id);
        assert_eq!(claims.mailbox_id, mailbox_id);
        assert!(verify_unsubscribe_token("other-secret-key-000000000000", &token).is_none());
    }
}
