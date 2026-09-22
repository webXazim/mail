use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{automation, email, entitlements, tenancy};
use crate::state::AppState;

const MAX_IDENTITIES: i64 = 20;
const VERIFY_TTL_MINUTES: i64 = 30;
const RESEND_COOLDOWN_SECONDS: i64 = 60;
const MAX_VERIFY_ATTEMPTS: i32 = 10;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct IdentityRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub reply_to: Option<String>,
    pub is_primary: bool,
    pub status: String,
    pub source: String,
    pub verified_at: Option<chrono::DateTime<chrono::Utc>>,
    pub verification_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn identity_columns() -> &'static str {
    "id,email::text AS email,display_name,reply_to::text AS reply_to,is_primary,status,source,verified_at,verification_expires_at"
}

fn to_json(row: &IdentityRow) -> Value {
    json!({
        "id": row.id,
        "email": row.email,
        "display_name": row.display_name,
        "reply_to": row.reply_to,
        "primary": row.is_primary,
        "status": row.status,
        "source": row.source,
        "verified_at": row.verified_at,
        "verification_expires_at": row.verification_expires_at,
    })
}

fn clean_display_name(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().chars().take(120).collect()
}

fn hash_code(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.trim().to_ascii_uppercase().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn random_code() -> String {
    let mut bytes = [0u8; 4];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// Ensure the mailbox address itself always exists as a verified sender
/// identity. `is_primary` means default sender, not ownership source: if the
/// user selected another verified identity as their default, this function
/// preserves that choice.
pub async fn ensure_primary(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
) -> Result<IdentityRow, ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('identity:' || $1, 0))")
        .bind(mailbox_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mailbox: Option<(String, String)> = sqlx::query_as(
        "SELECT m.address::text,
                CASE WHEN btrim(m.display_name) <> '' THEN m.display_name ELSE u.display_name END
         FROM mailboxes m
         JOIN users u ON u.id = $1 AND u.status='active'
         JOIN organization_memberships om
           ON om.organization_id = m.organization_id
          AND om.user_id = u.id
          AND om.status = 'active'
         JOIN organizations o
           ON o.id = m.organization_id
          AND o.status = 'active'
         WHERE m.id=$2 AND m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'
         FOR UPDATE OF m",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((email_address, display_name)) = mailbox else {
        return Err(ApiError::forbidden("The selected mailbox is not active for this account"));
    };

    sqlx::query(
        "UPDATE sender_identities SET is_primary=FALSE,updated_at=now()
         WHERE mailbox_id=$1 AND is_primary=TRUE AND status<>'verified'",
    )
    .bind(mailbox_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let has_default: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sender_identities
          WHERE mailbox_id=$1 AND is_primary=TRUE AND status='verified'
            AND source IN ('primary','alias'))",
    )
    .bind(mailbox_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE sender_identities SET status='disabled',is_primary=FALSE,updated_at=now()
         WHERE mailbox_id=$1 AND source='primary' AND lower(email::text)<>lower($2)",
    )
    .bind(mailbox_id)
    .bind(&email_address)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let query = format!(
        "INSERT INTO sender_identities
          (user_id,mailbox_id,email,display_name,is_primary,status,source,verified_at)
         VALUES($1,$2,$3,$4,$5,'verified','primary',now())
         ON CONFLICT(mailbox_id,email) DO UPDATE SET
           user_id=EXCLUDED.user_id,status='verified',source='primary',verified_at=COALESCE(sender_identities.verified_at,now()),
           display_name=CASE WHEN sender_identities.display_name='' THEN EXCLUDED.display_name ELSE sender_identities.display_name END,
           is_primary=CASE WHEN $5 THEN TRUE ELSE sender_identities.is_primary END,
           updated_at=now()
         RETURNING {}",
        identity_columns()
    );
    let row = sqlx::query_as::<_, IdentityRow>(&query)
        .bind(user_id)
        .bind(mailbox_id)
        .bind(&email_address)
        .bind(display_name)
        .bind(!has_default)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(row)
}

async fn authorize_from_address(
    state: &AppState,
    mailbox_id: Uuid,
    identity: IdentityRow,
) -> Result<IdentityRow, ApiError> {
    if !matches!(identity.source.as_str(), "primary" | "alias") {
        return Err(ApiError::forbidden(
            "External verified addresses may be used as Reply-To only. Send From an active CS Mail business mailbox or alias.",
        ));
    }
    let Some((_, domain)) = identity.email.rsplit_once('@') else {
        return Err(ApiError::forbidden("Sender address is invalid"));
    };
    let dns_ready: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1
           FROM mailboxes m
           JOIN organization_domains d ON d.organization_id=m.organization_id
           WHERE m.id=$1 AND m.deleted_at IS NULL AND m.status='active'
             AND lower(d.domain::text)=lower($2)
             AND d.status='active' AND d.dns_ready=TRUE
             AND d.provider_domain_id IS NOT NULL
         )",
    )
    .bind(mailbox_id)
    .bind(domain)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !dns_ready {
        return Err(ApiError::forbidden(
            "This sender domain is not DNS-ready for outbound mail. Verify MX, SPF, DKIM and DMARC before sending.",
        ));
    }
    Ok(identity)
}

pub async fn resolve(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    identity_id: Option<Uuid>,
) -> Result<IdentityRow, ApiError> {
    let mailbox_identity = ensure_primary(state, user_id, mailbox_id).await?;
    let identity = if let Some(identity_id) = identity_id {
        if identity_id == mailbox_identity.id {
            mailbox_identity
        } else {
            let query = format!(
                "SELECT {} FROM sender_identities
                 WHERE id=$1 AND mailbox_id=$2 AND status='verified'",
                identity_columns()
            );
            sqlx::query_as::<_, IdentityRow>(&query)
                .bind(identity_id)
                .bind(mailbox_id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?
                .ok_or_else(|| ApiError::forbidden("Sender identity is not verified for this mailbox"))?
        }
    } else {
        let query = format!(
            "SELECT {} FROM sender_identities
             WHERE mailbox_id=$1 AND is_primary=TRUE AND status='verified'
               AND source IN ('primary','alias')
             LIMIT 1",
            identity_columns()
        );
        sqlx::query_as::<_, IdentityRow>(&query)
            .bind(mailbox_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .unwrap_or(mailbox_identity)
    };
    authorize_from_address(state, mailbox_id, identity).await
}

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .map(|mailbox| mailbox.id)
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before managing sender identities"))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    ensure_primary(&state, auth.user_id, mailbox_id).await?;
    let query = format!(
        "SELECT {} FROM sender_identities
         WHERE mailbox_id=$1 AND status<>'disabled'
         ORDER BY is_primary DESC, CASE status WHEN 'verified' THEN 0 ELSE 1 END, created_at, email",
        identity_columns()
    );
    let rows: Vec<IdentityRow> = sqlx::query_as(&query)
        .bind(mailbox_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "identities": rows.iter().map(to_json).collect::<Vec<_>>()
    })))
}

#[derive(Deserialize)]
pub struct CreateIdentityIn {
    email: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    reply_to: Option<String>,
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateIdentityIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    entitlements::require_feature(&state, auth.user_id, "mail").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    ensure_primary(&state, auth.user_id, mailbox_id).await?;
    let target = body.email.trim().to_lowercase();
    if !automation::valid_email(&target) {
        return Err(ApiError::bad_request("Enter a valid sender email address"));
    }
    let display_name = if body.display_name.trim().is_empty() {
        auth.email.split('@').next().unwrap_or("Sender").to_string()
    } else {
        clean_display_name(&body.display_name)
    };
    let reply_to = match body.reply_to {
        Some(value) if !value.trim().is_empty() => {
            let value = value.trim().to_lowercase();
            if !automation::valid_email(&value) {
                return Err(ApiError::bad_request("Reply-To must be a valid email address"));
            }
            Some(value)
        }
        _ => None,
    };

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sender_identities WHERE mailbox_id=$1 AND status<>'disabled'",
    )
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if count >= MAX_IDENTITIES {
        return Err(ApiError::bad_request(format!(
            "A maximum of {MAX_IDENTITIES} sender identities is supported"
        )));
    }

    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id,status FROM sender_identities WHERE mailbox_id=$1 AND lower(email::text)=lower($2)",
    )
    .bind(mailbox_id)
    .bind(&target)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some((_id, status)) = existing.as_ref() {
        if status != "disabled" {
            return Err(ApiError::conflict("That sender identity already exists"));
        }
    }

    let is_managed = target
        .rsplit_once('@')
        .is_some_and(|(_, domain)| domain.eq_ignore_ascii_case(state.stalwart.default_domain()));
    if is_managed {
        let alias: Option<(Uuid,)> = sqlx::query_as(
            "SELECT a.id
               FROM aliases a
              WHERE a.dest_mailbox_id=$1 AND a.deleted_at IS NULL AND a.enabled=TRUE
                AND lower(a.source || '@' || a.domain)=lower($2)
              LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(&target)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let Some((alias_id,)) = alias else {
            return Err(ApiError::forbidden(
                "That CS Mail address must first be assigned to your mailbox as an alias",
            ));
        };
        let query = format!(
            "INSERT INTO sender_identities
              (user_id,mailbox_id,email,display_name,reply_to,is_primary,status,source,alias_id,verified_at)
             VALUES($1,$2,$3,$4,$5,FALSE,'verified','alias',$6,now())
             ON CONFLICT(mailbox_id,email) DO UPDATE SET display_name=EXCLUDED.display_name,
               reply_to=EXCLUDED.reply_to,status='verified',source='alias',alias_id=EXCLUDED.alias_id,
               verified_at=now(),verification_token_hash='',verification_expires_at=NULL,
               verification_sent_at=NULL,verification_attempts=0,updated_at=now()
             RETURNING {}",
            identity_columns()
        );
        let row = sqlx::query_as::<_, IdentityRow>(&query)
            .bind(auth.user_id)
            .bind(mailbox_id)
            .bind(&target)
            .bind(display_name)
            .bind(reply_to)
            .bind(alias_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        audit::record(&state, Some(auth.user_id), "mail.identity.add_alias", json!({"email":target})).await;
        return Ok((StatusCode::CREATED, Json(json!({"identity":to_json(&row),"verificationCode":null}))));
    }

    let query = format!(
        "INSERT INTO sender_identities
          (user_id,mailbox_id,email,display_name,reply_to,is_primary,status,source,verified_at)
         VALUES($1,$2,$3,$4,$5,FALSE,'pending','verified_external',NULL)
         ON CONFLICT(mailbox_id,email) DO UPDATE SET display_name=EXCLUDED.display_name,
           reply_to=EXCLUDED.reply_to,status='pending',source='verified_external',alias_id=NULL,
           verified_at=NULL,verification_token_hash='',verification_expires_at=NULL,
           verification_sent_at=NULL,verification_attempts=0,is_primary=FALSE,updated_at=now()
         RETURNING {}",
        identity_columns()
    );
    let row = sqlx::query_as::<_, IdentityRow>(&query)
        .bind(auth.user_id)
        .bind(mailbox_id)
        .bind(&target)
        .bind(display_name)
        .bind(reply_to)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let dev_code = issue_code(&state, mailbox_id, row.id, &target).await?;
    audit::record(&state, Some(auth.user_id), "mail.identity.create_pending", json!({"email":target})).await;
    let refreshed = get_identity(&state, mailbox_id, row.id).await?;
    Ok((StatusCode::CREATED, Json(json!({"identity":to_json(&refreshed),"verificationCode":dev_code}))))
}

async fn issue_code(
    state: &AppState,
    mailbox_id: Uuid,
    identity_id: Uuid,
    target: &str,
) -> Result<Option<String>, ApiError> {
    let code = random_code();
    let hash = hash_code(&code);
    sqlx::query(
        "UPDATE sender_identities SET verification_token_hash=$3,
          verification_expires_at=now()+($4 * interval '1 minute'),verification_sent_at=now(),
          verification_attempts=0,updated_at=now()
         WHERE id=$1 AND mailbox_id=$2 AND status='pending' AND source='verified_external'",
    )
    .bind(identity_id)
    .bind(mailbox_id)
    .bind(hash)
    .bind(VERIFY_TTL_MINUTES)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    email::send_sender_identity_verification(state, target, &code).await?;
    Ok(state.return_token_links.then_some(code))
}

async fn get_identity(state: &AppState, mailbox_id: Uuid, id: Uuid) -> Result<IdentityRow, ApiError> {
    let query = format!("SELECT {} FROM sender_identities WHERE id=$1 AND mailbox_id=$2", identity_columns());
    sqlx::query_as::<_, IdentityRow>(&query)
        .bind(id)
        .bind(mailbox_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Identity not found"))
}

#[derive(Deserialize)]
pub struct VerifyIdentityIn {
    code: String,
}

pub async fn verify(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<VerifyIdentityIn>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let supplied = hash_code(&body.code);
    let row: Option<IdentityRow> = sqlx::query_as(&format!(
        "UPDATE sender_identities SET status='verified',verified_at=now(),verification_token_hash='',
          verification_expires_at=NULL,verification_sent_at=NULL,verification_attempts=0,updated_at=now()
         WHERE id=$1 AND mailbox_id=$2 AND status='pending' AND source='verified_external'
           AND verification_token_hash=$3 AND verification_expires_at>now()
           AND verification_attempts<$4
         RETURNING {}", identity_columns()
    ))
    .bind(id)
    .bind(mailbox_id)
    .bind(supplied)
    .bind(MAX_VERIFY_ATTEMPTS)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(row) = row else {
        let attempts: Option<i32> = sqlx::query_scalar(
            "UPDATE sender_identities SET verification_attempts=verification_attempts+1,updated_at=now()
             WHERE id=$1 AND mailbox_id=$2 AND status='pending' AND source='verified_external'
               AND verification_expires_at>now() AND verification_attempts<$3
             RETURNING verification_attempts",
        )
        .bind(id)
        .bind(mailbox_id)
        .bind(MAX_VERIFY_ATTEMPTS)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if attempts.is_some_and(|value| value >= MAX_VERIFY_ATTEMPTS) {
            return Err(ApiError::too_many(
                "Too many verification attempts; request a new code",
            ));
        }
        return Err(ApiError::bad_request("Verification code is invalid or expired"));
    };
    audit::record(&state, Some(auth.user_id), "mail.identity.verify", json!({"id":id,"email":row.email})).await;
    Ok(Json(to_json(&row)))
}

pub async fn resend(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT email::text,verification_sent_at FROM sender_identities
         WHERE id=$1 AND mailbox_id=$2 AND status='pending' AND source='verified_external'",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((target, sent_at)) = row else {
        return Err(ApiError::bad_request("No sender verification is pending"));
    };
    if sent_at.is_some_and(|at| chrono::Utc::now() - at < chrono::Duration::seconds(RESEND_COOLDOWN_SECONDS)) {
        return Err(ApiError::too_many("Wait a minute before requesting another code"));
    }
    let dev_code = issue_code(&state, mailbox_id, id, &target).await?;
    Ok(Json(json!({"ok":true,"verificationCode":dev_code})))
}

#[derive(Deserialize)]
pub struct UpdateIdentityIn {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    reply_to: Option<Option<String>>,
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateIdentityIn>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    ensure_primary(&state, auth.user_id, mailbox_id).await?;
    let display_name = body.display_name.map(|value| clean_display_name(&value));
    let reply_to = match body.reply_to {
        None => None,
        Some(None) => Some(None),
        Some(Some(value)) => {
            let value = value.trim().to_lowercase();
            if !automation::valid_email(&value) {
                return Err(ApiError::bad_request("Reply-To must be a valid email address"));
            }
            Some(Some(value))
        }
    };
    let query = format!(
        "UPDATE sender_identities SET
          display_name=COALESCE($3,display_name),
          reply_to=CASE WHEN $4 THEN $5 ELSE reply_to END,
          updated_at=now()
         WHERE id=$1 AND mailbox_id=$2 AND status<>'disabled'
         RETURNING {}",
        identity_columns()
    );
    let row: Option<IdentityRow> = sqlx::query_as(&query)
        .bind(id)
        .bind(mailbox_id)
        .bind(display_name)
        .bind(reply_to.is_some())
        .bind(reply_to.flatten())
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(row) = row else {
        return Err(ApiError::not_found("Identity not found"));
    };
    audit::record(&state, Some(auth.user_id), "mail.identity.update", json!({"id":row.id,"email":row.email})).await;
    Ok(Json(to_json(&row)))
}

pub async fn set_default(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    ensure_primary(&state, auth.user_id, mailbox_id).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('identity:' || $1, 0))")
        .bind(mailbox_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sender_identities
          WHERE id=$1 AND mailbox_id=$2 AND status='verified' AND source IN ('primary','alias'))",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !exists {
        return Err(ApiError::bad_request(
            "Only a verified CS Mail mailbox or alias can be the default From address. External addresses may be Reply-To only.",
        ));
    }
    sqlx::query("UPDATE sender_identities SET is_primary=FALSE,updated_at=now() WHERE mailbox_id=$1")
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let query = format!(
        "UPDATE sender_identities SET is_primary=TRUE,updated_at=now()
         WHERE id=$1 AND mailbox_id=$2 AND status='verified' AND source IN ('primary','alias') RETURNING {}",
        identity_columns()
    );
    let row = sqlx::query_as::<_, IdentityRow>(&query)
        .bind(id)
        .bind(mailbox_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(auth.user_id), "mail.identity.default", json!({"id":id,"email":row.email})).await;
    Ok(Json(to_json(&row)))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    ensure_primary(&state, auth.user_id, mailbox_id).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('identity:' || $1, 0))")
        .bind(mailbox_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let current: Option<(String, String, bool)> = sqlx::query_as(
        "SELECT email::text,source,is_primary FROM sender_identities
         WHERE id=$1 AND mailbox_id=$2 AND status<>'disabled' FOR UPDATE",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((email_address, source, was_default)) = current else {
        return Err(ApiError::not_found("Identity not found"));
    };
    if source == "primary" {
        return Err(ApiError::bad_request("The mailbox identity cannot be removed"));
    }
    sqlx::query(
        "UPDATE sender_identities SET status='disabled',is_primary=FALSE,verification_token_hash='',
          verification_expires_at=NULL,verification_sent_at=NULL,verification_attempts=0,updated_at=now()
         WHERE id=$1 AND mailbox_id=$2",
    )
    .bind(id)
    .bind(mailbox_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if was_default {
        sqlx::query(
            "UPDATE sender_identities SET is_primary=TRUE,updated_at=now()
             WHERE id=(SELECT id FROM sender_identities WHERE mailbox_id=$1 AND source='primary'
                       AND status='verified' ORDER BY created_at LIMIT 1)",
        )
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(auth.user_id), "mail.identity.remove", json!({"id":id,"email":email_address})).await;
    Ok(Json(json!({"ok":true})))
}
