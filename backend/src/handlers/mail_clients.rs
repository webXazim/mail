//! Upgrade 26: external mail-client credentials and durable mailbox imports.
//!
//! Website authentication remains completely separate from provider protocol
//! credentials. External clients use Stalwart-native AppPassword credentials;
//! CS Mail stores only metadata/provider ids and returns each secret once.
//! MBOX imports are staged on the existing persistent blob volume and claimed
//! by a PostgreSQL-leased worker so multiple API replicas remain safe.

use std::net::IpAddr;
use std::path::{Component, PathBuf};
use std::time::Duration;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::middleware::rate_limit::{CRED_LOCK, CRED_MAX_FAILS, CRED_WINDOW};
use crate::services::email;
use crate::services::stalwart::StalwartError;
use crate::services::{entitlements, tenancy};
use crate::state::AppState;

const IMPORT_IO_CHUNK: usize = 64 * 1024;
const IMPORT_HISTORY_LIMIT: i64 = 50;
const APP_PASSWORD_LABEL_MAX: usize = 120;
const APP_PASSWORD_MAX_EXPIRY_DAYS: i64 = 365;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AppPasswordView {
    id: Uuid,
    label: String,
    allowed_ips: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
    status: String,
    last_error: String,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAppPassword {
    label: String,
    current_password: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    allowed_ips: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AutoconfigQuery {
    #[serde(default, alias = "emailaddress")]
    email: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    #[serde(default)]
    filename: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ImportView {
    id: Uuid,
    original_filename: String,
    byte_size: i64,
    status: String,
    total_messages: i64,
    imported_messages: i64,
    failed_messages: i64,
    last_error: String,
    attempts: i32,
    max_attempts: i32,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ImportJob {
    id: Uuid,
    organization_id: Uuid,
    mailbox_id: Uuid,
    user_id: Uuid,
    storage_key: String,
    attempts: i32,
    max_attempts: i32,
}

fn provider_error(error: StalwartError) -> ApiError {
    tracing::warn!(error = %error, "mail provider client/import operation failed");
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "mail_provider_error",
        error.public_message(),
    )
}

fn safe_filename(value: &str) -> String {
    let name = value
        .replace(['\r', '\n'], " ")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(180)
        .collect::<String>();
    if name.is_empty() {
        "mailbox.mbox".to_string()
    } else {
        name
    }
}

fn import_storage_key(mailbox_id: Uuid, import_id: Uuid) -> String {
    format!("imports/{mailbox_id}/{import_id}.mbox")
}

fn checked_import_path(state: &AppState, key: &str) -> Result<PathBuf, ApiError> {
    let rel = PathBuf::from(key);
    if rel.is_absolute()
        || rel.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ApiError::internal("Invalid mailbox import storage key"));
    }
    Ok(state.attachment_store_dir.join(rel))
}

async fn selected_mailbox(
    state: &AppState,
    auth: &AuthUser,
) -> Result<tenancy::ActiveMailbox, ApiError> {
    let mailbox = tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .ok_or_else(|| ApiError::conflict("Select a business mailbox first"))?;
    if mailbox.status != "active" {
        return Err(ApiError::forbidden("The selected mailbox is not active"));
    }
    if mailbox.sync_status != "ready"
        || mailbox
            .provider_account_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(ApiError::conflict(
            "The selected mailbox is not ready on the mail server",
        ));
    }
    Ok(mailbox)
}

async fn current_password_ok(
    state: &AppState,
    user_id: Uuid,
    current_password: &str,
) -> Result<bool, ApiError> {
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let hash = hash.ok_or_else(|| ApiError::not_found("Account not found"))?;
    let parsed = PasswordHash::new(&hash).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(current_password.as_bytes(), &parsed)
        .is_ok())
}

fn validate_label(label: &str) -> Result<String, ApiError> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > APP_PASSWORD_LABEL_MAX {
        return Err(ApiError::bad_request(format!(
            "App-password label must be 1-{APP_PASSWORD_LABEL_MAX} characters"
        )));
    }
    Ok(label.to_string())
}

fn validate_ip_mask(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if let Some((ip, prefix)) = value.split_once('/') {
        let Ok(ip) = ip.parse::<IpAddr>() else {
            return false;
        };
        let Ok(prefix) = prefix.parse::<u8>() else {
            return false;
        };
        return match ip {
            IpAddr::V4(_) => prefix <= 32,
            IpAddr::V6(_) => prefix <= 128,
        };
    }
    value.parse::<IpAddr>().is_ok()
}

fn validate_allowed_ips(values: &[String]) -> Result<Vec<String>, ApiError> {
    if values.len() > 16 {
        return Err(ApiError::bad_request(
            "At most 16 IP addresses or CIDR ranges may be allowed",
        ));
    }
    let mut out = Vec::new();
    for value in values {
        let value = value.trim();
        if !validate_ip_mask(value) {
            return Err(ApiError::bad_request(format!(
                "Invalid allowed IP address or CIDR range: {value}"
            )));
        }
        if !out.iter().any(|existing| existing == value) {
            out.push(value.to_string());
        }
    }
    Ok(out)
}

fn validate_expiry(value: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| ApiError::bad_request("expires_at must be an RFC 3339 timestamp"))?
        .with_timezone(&Utc);
    if parsed <= Utc::now() + ChronoDuration::minutes(5) {
        return Err(ApiError::bad_request(
            "App-password expiry must be at least five minutes in the future",
        ));
    }
    if parsed > Utc::now() + ChronoDuration::days(APP_PASSWORD_MAX_EXPIRY_DAYS) {
        return Err(ApiError::bad_request(format!(
            "App-password expiry may be at most {APP_PASSWORD_MAX_EXPIRY_DAYS} days away"
        )));
    }
    Ok(Some(parsed))
}

async fn mailbox_advisory_lock(
    tx: &mut Transaction<'_, Postgres>,
    mailbox_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 2600))")
        .bind(mailbox_id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn rotate_hidden_provider_password(state: &AppState, account_id: &str) {
    // A failed cleanup is not an exposure of a user-known credential: every
    // temporary password is random and never persisted or returned. Retrying
    // here simply narrows the lifetime of the process-known value.
    for attempt in 0..2 {
        let next = email::random_token();
        match state.stalwart.set_account_password(account_id, &next).await {
            Ok(()) => return,
            Err(error) if attempt == 0 && error.is_transient() => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => {
                tracing::error!(error=%error, account_id=%account_id, "could not rotate hidden provider mailbox password after app-password operation");
                return;
            }
        }
    }
}

fn client_config(state: &AppState, address: &str) -> Value {
    json!({
        "username": address,
        "incoming": {
            "protocol": "IMAP",
            "host": state.mail_client_host,
            "port": state.mail_client_imap_port,
            "security": "TLS",
            "authentication": "app_password"
        },
        "outgoing": {
            "protocol": "SMTP",
            "host": state.mail_client_host,
            "port": state.mail_client_smtp_port,
            "security": "STARTTLS",
            "authentication": "app_password"
        }
    })
}

pub async fn overview(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    let passwords: Vec<AppPasswordView> = sqlx::query_as(
        "SELECT id,label,allowed_ips,expires_at,
                CASE WHEN status='active' AND expires_at IS NOT NULL AND expires_at <= now()
                     THEN 'expired' ELSE status END AS status,
                last_error,created_at,revoked_at
         FROM mailbox_app_passwords
         WHERE mailbox_id=$1 AND user_id=$2
         ORDER BY created_at DESC",
    )
    .bind(mailbox.id)
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "mailbox": { "id": mailbox.id, "address": mailbox.address, "organization_id": mailbox.organization_id },
        "config": client_config(&state, &mailbox.address),
        "max_app_passwords": state.mail_client_max_app_passwords,
        "app_passwords": passwords,
        "autoconfig": {
            "thunderbird_path": "/.well-known/autoconfig/mail/config-v1.1.xml",
            "note": "Use the full mailbox address as the username and an app password as the password."
        }
    })))
}

pub async fn create_app_password(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateAppPassword>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    let failure_key = format!("mail-client-password:{}:{}", auth.user_id, mailbox.id);
    let create_key = format!("mail-client-create:{}:{}", auth.user_id, mailbox.id);
    state.rate.check_lock(&failure_key).await?;
    state.rate.check_burst(&create_key, 10, 3600).await?;
    if !current_password_ok(&state, auth.user_id, &body.current_password).await? {
        state
            .rate
            .record_failure(&failure_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK)
            .await?;
        return Err(ApiError::unauthorized("Current CS Mail password is incorrect"));
    }
    state.rate.reset(&failure_key).await?;
    let label = validate_label(&body.label)?;
    let allowed_ips = validate_allowed_ips(&body.allowed_ips)?;
    let expires_at = validate_expiry(body.expires_at.as_deref())?;
    let provider_account_id = mailbox.provider_account_id.as_deref().unwrap().to_string();

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    mailbox_advisory_lock(&mut tx, mailbox.id).await?;

    let active_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM mailbox_app_passwords
         WHERE mailbox_id=$1 AND status='active' AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(mailbox.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if active_count >= state.mail_client_max_app_passwords {
        return Err(ApiError::conflict(format!(
            "This mailbox already has the maximum of {} active app passwords",
            state.mail_client_max_app_passwords
        )));
    }

    let id = Uuid::new_v4();
    let provider_description = format!("CS Mail: {label} [{id}]");
    let temporary_provider_password = email::random_token();
    state
        .stalwart
        .set_account_password(&provider_account_id, &temporary_provider_password)
        .await
        .map_err(provider_error)?;

    let provider_expires_at = expires_at.as_ref().map(|value| value.to_rfc3339());
    let provider_result = state
        .stalwart
        .create_app_password(
            &mailbox.address,
            &temporary_provider_password,
            &provider_description,
            provider_expires_at.as_deref(),
            &allowed_ips,
        )
        .await;

    let (provider_credential_id, secret) = match provider_result {
        Ok(result) => result,
        Err(error) => {
            if let Err(cleanup_error) = state
                .stalwart
                .destroy_app_passwords_by_description(
                    &mailbox.address,
                    &temporary_provider_password,
                    &provider_description,
                )
                .await
            {
                tracing::error!(error=%cleanup_error, mailbox_id=%mailbox.id, operation_id=%id, "could not verify cleanup after ambiguous app-password create");
            }
            rotate_hidden_provider_password(&state, &provider_account_id).await;
            return Err(provider_error(error));
        }
    };

    let inserted = sqlx::query(
        "INSERT INTO mailbox_app_passwords
           (id,organization_id,mailbox_id,user_id,label,provider_credential_id,allowed_ips,expires_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(mailbox.organization_id)
    .bind(mailbox.id)
    .bind(auth.user_id)
    .bind(&label)
    .bind(&provider_credential_id)
    .bind(&allowed_ips)
    .bind(expires_at)
    .execute(&mut *tx)
    .await;

    if let Err(error) = inserted {
        let _ = state
            .stalwart
            .destroy_app_password(
                &mailbox.address,
                &temporary_provider_password,
                &provider_credential_id,
            )
            .await;
        rotate_hidden_provider_password(&state, &provider_account_id).await;
        return Err(ApiError::internal(error.to_string()));
    }

    // Rotate the short-lived self-service credential after the provider
    // application password and durable metadata both exist. Failure here is
    // logged but does not expose a usable password to the customer because the
    // temporary provider password was never returned or persisted.
    rotate_hidden_provider_password(&state, &provider_account_id).await;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail_client.app_password.created",
        json!({ "mailbox_id": mailbox.id, "organization_id": mailbox.organization_id, "credential_id": id, "label": label }),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "label": label,
            "secret": secret,
            "expires_at": expires_at,
            "config": client_config(&state, &mailbox.address),
            "message": "Copy this app password now. CS Mail cannot show it again."
        })),
    ))
}

pub async fn revoke_app_password(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    let provider_account_id = mailbox.provider_account_id.as_deref().unwrap().to_string();
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    mailbox_advisory_lock(&mut tx, mailbox.id).await?;
    let provider_credential_id: Option<String> = sqlx::query_scalar(
        "SELECT provider_credential_id FROM mailbox_app_passwords
         WHERE id=$1 AND mailbox_id=$2 AND user_id=$3 AND status='active' FOR UPDATE",
    )
    .bind(id)
    .bind(mailbox.id)
    .bind(auth.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let provider_credential_id = provider_credential_id
        .ok_or_else(|| ApiError::not_found("App password not found"))?;

    let temporary_provider_password = email::random_token();
    state
        .stalwart
        .set_account_password(&provider_account_id, &temporary_provider_password)
        .await
        .map_err(provider_error)?;
    let revoked = state
        .stalwart
        .destroy_app_password(
            &mailbox.address,
            &temporary_provider_password,
            &provider_credential_id,
        )
        .await;
    rotate_hidden_provider_password(&state, &provider_account_id).await;
    if let Err(error) = revoked {
        return Err(provider_error(error));
    }

    sqlx::query(
        "UPDATE mailbox_app_passwords
         SET status='revoked',revoked_at=now(),last_error='',updated_at=now()
         WHERE id=$1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail_client.app_password.revoked",
        json!({ "mailbox_id": mailbox.id, "organization_id": mailbox.organization_id, "credential_id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

fn public_domain_from_email(email: &str) -> String {
    let candidate = email
        .split_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !candidate.is_empty()
        && candidate.len() <= 253
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        && !candidate.starts_with('.')
        && !candidate.ends_with('.')
    {
        candidate
    } else {
        "crescentsphere.com".to_string()
    }
}

pub async fn autoconfig(
    State(state): State<AppState>,
    Query(query): Query<AutoconfigQuery>,
) -> Response {
    let domain = public_domain_from_email(&query.email);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="{domain}">
    <domain>{domain}</domain>
    <displayName>CS Mail</displayName>
    <displayShortName>CS Mail</displayShortName>
    <incomingServer type="imap">
      <hostname>{host}</hostname>
      <port>{imap_port}</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>{host}</hostname>
      <port>{smtp_port}</port>
      <socketType>STARTTLS</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#,
        host = state.mail_client_host,
        imap_port = state.mail_client_imap_port,
        smtp_port = state.mail_client_smtp_port,
    );
    let mut response = xml.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

pub async fn list_imports(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    let imports: Vec<ImportView> = sqlx::query_as(
        "SELECT id,original_filename,byte_size,status,total_messages,imported_messages,
                failed_messages,last_error,attempts,max_attempts,started_at,completed_at,created_at,updated_at
         FROM mailbox_imports
         WHERE mailbox_id=$1 AND user_id=$2
         ORDER BY created_at DESC LIMIT $3",
    )
    .bind(mailbox.id)
    .bind(auth.user_id)
    .bind(IMPORT_HISTORY_LIMIT)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "imports": imports })))
}

pub async fn upload_import(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ImportQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    state
        .rate
        .check_burst(&format!("mail-import-upload:{}:{}", auth.user_id, mailbox.id), 10, 3600)
        .await?;
    let ent = entitlements::for_organization(&state, mailbox.organization_id).await?;
    if !matches!(ent.subscription_status.as_str(), "active" | "trial") {
        return Err(ApiError::forbidden("This business subscription is not active"));
    }
    let max_bytes = state.mail_import_max_bytes.min(i64::MAX as u64) as i64;
    if let Some(length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
    {
        if length < 0 || length > max_bytes {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "mail_import_too_large",
                format!("Mailbox imports may be at most {} MiB", max_bytes / (1024 * 1024)),
            ));
        }
    }

    // Conservative pool guard: importing N raw bytes cannot be allowed when
    // the business already has less than N bytes remaining in its pool.
    let cached_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(storage_bytes,0)::bigint FROM organization_usage WHERE organization_id=$1",
    )
    .bind(mailbox.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .unwrap_or(0);
    let remaining = (ent.storage_pool_bytes.min(i64::MAX as u64) as i64).saturating_sub(cached_used);
    if remaining <= 0 {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "organization_storage_quota",
            "This business has no storage remaining for mailbox import",
        ));
    }

    let id = Uuid::new_v4();
    let filename = safe_filename(&query.filename);
    let storage_key = import_storage_key(mailbox.id, id);
    let path = checked_import_path(&state, &storage_key)?;
    let mut part = path.clone();
    part.set_extension("part");
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::internal(format!("Unable to prepare import storage: {e}")))?;
    }
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| ApiError::internal(format!("Unable to create import file: {e}")))?;
    let mut stream = body.into_data_stream();
    let mut size: i64 = 0;
    let mut digest = Sha256::new();
    let allowed = max_bytes.min(remaining.max(0));
    let upload_result: Result<(), ApiError> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::bad_request(format!("Import upload failed: {e}")))?;
            size = size.saturating_add(chunk.len() as i64);
            if size > allowed {
                return Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "mail_import_too_large",
                    "The import exceeds the available mailbox/business storage allowance",
                ));
            }
            digest.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::internal(format!("Import write failed: {e}")))?;
        }
        file.flush()
            .await
            .map_err(|e| ApiError::internal(format!("Import flush failed: {e}")))?;
        if size == 0 {
            return Err(ApiError::bad_request("The mailbox import file is empty"));
        }
        Ok(())
    }
    .await;
    if let Err(error) = upload_result {
        drop(file);
        let _ = tokio::fs::remove_file(&part).await;
        return Err(error);
    }
    drop(file);
    tokio::fs::rename(&part, &path)
        .await
        .map_err(|e| ApiError::internal(format!("Unable to finalize import file: {e}")))?;
    let sha256 = format!("{:x}", digest.finalize());

    let insert_result = sqlx::query(
        "INSERT INTO mailbox_imports
           (id,organization_id,mailbox_id,user_id,source_type,original_filename,storage_key,file_sha256,byte_size)
         VALUES($1,$2,$3,$4,'mbox',$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(mailbox.organization_id)
    .bind(mailbox.id)
    .bind(auth.user_id)
    .bind(&filename)
    .bind(&storage_key)
    .bind(&sha256)
    .bind(size)
    .execute(&state.db)
    .await;
    if let Err(error) = insert_result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(ApiError::internal(error.to_string()));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "mail_import.queued",
        json!({ "import_id": id, "mailbox_id": mailbox.id, "organization_id": mailbox.organization_id, "bytes": size, "sha256": sha256 }),
    )
    .await;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "id": id,
            "status": "queued",
            "filename": filename,
            "byte_size": size,
            "message": "Mailbox import queued. You can leave this page and check progress later."
        })),
    ))
}

pub async fn cancel_import(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox = selected_mailbox(&state, &auth).await?;
    let row: Option<String> = sqlx::query_scalar(
        "UPDATE mailbox_imports
         SET status='cancelled',completed_at=now(),locked_by=NULL,locked_until=NULL,updated_at=now()
         WHERE id=$1 AND mailbox_id=$2 AND user_id=$3 AND status IN ('queued','running')
         RETURNING storage_key",
    )
    .bind(id)
    .bind(mailbox.id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let key = row.ok_or_else(|| ApiError::conflict("Only a queued or running import can be cancelled"))?;
    if let Ok(path) = checked_import_path(&state, &key) {
        let _ = tokio::fs::remove_file(path).await;
    }
    audit::record(
        &state,
        Some(auth.user_id),
        "mail_import.cancelled",
        json!({ "import_id": id, "mailbox_id": mailbox.id, "organization_id": mailbox.organization_id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub fn spawn_import_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(state.mail_import_poll_secs.max(1)));
        let worker_id = Uuid::new_v4();
        loop {
            tick.tick().await;
            match claim_import(&state, worker_id).await {
                Ok(Some(job)) => {
                    if let Err(error) = process_import(&state, worker_id, &job).await {
                        fail_or_retry_import(&state, worker_id, &job, &error).await;
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(error=%error, "mail import worker claim failed"),
            }
        }
    });
}

async fn claim_import(state: &AppState, worker_id: Uuid) -> Result<Option<ImportJob>, ApiError> {
    let lease = state.mail_import_lease_secs.max(60) as i64;
    sqlx::query_as::<_, ImportJob>(
        "WITH next_job AS (
           SELECT id FROM mailbox_imports
           WHERE attempts < max_attempts
             AND ((status='queued' AND next_attempt_at <= now())
               OR (status='running' AND locked_until < now()))
           ORDER BY created_at
           FOR UPDATE SKIP LOCKED
           LIMIT 1
         )
         UPDATE mailbox_imports j
            SET status='running',attempts=j.attempts+1,locked_by=$1,
                locked_until=now()+($2*interval '1 second'),
                started_at=COALESCE(started_at,now()),updated_at=now(),last_error=''
           FROM next_job
          WHERE j.id=next_job.id
         RETURNING j.id,j.organization_id,j.mailbox_id,j.user_id,j.storage_key,j.attempts,j.max_attempts",
    )
    .bind(worker_id)
    .bind(lease)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

async fn import_still_running(
    state: &AppState,
    job: &ImportJob,
    worker_id: Uuid,
) -> Result<bool, ApiError> {
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM mailbox_imports WHERE id=$1 AND locked_by=$2",
    )
    .bind(job.id)
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(status.as_deref() == Some("running"))
}

async fn heartbeat_import(
    state: &AppState,
    job: &ImportJob,
    worker_id: Uuid,
    total: i64,
    imported: i64,
    failed: i64,
) -> Result<(), ApiError> {
    let lease = state.mail_import_lease_secs.max(60) as i64;
    let result = sqlx::query(
        "UPDATE mailbox_imports
         SET total_messages=$3,imported_messages=$4,failed_messages=$5,
             locked_until=now()+($6*interval '1 second'),updated_at=now()
         WHERE id=$1 AND locked_by=$2 AND status='running'",
    )
    .bind(job.id)
    .bind(worker_id)
    .bind(total)
    .bind(imported)
    .bind(failed)
    .bind(lease)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict("Mailbox import lease is no longer owned by this worker"));
    }
    Ok(())
}

fn received_at_from_message(message: &[u8]) -> Option<String> {
    let head_len = message.len().min(128 * 1024);
    let text = String::from_utf8_lossy(&message[..head_len]);
    for line in text.lines() {
        if line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Date:").or_else(|| line.strip_prefix("date:")) {
            let value = value.trim();
            if let Ok(date) = DateTime::parse_from_rfc2822(value) {
                return Some(date.with_timezone(&Utc).to_rfc3339());
            }
            if let Ok(date) = DateTime::parse_from_rfc3339(value) {
                return Some(date.with_timezone(&Utc).to_rfc3339());
            }
        }
    }
    None
}

async fn existing_import_message(
    state: &AppState,
    job: &ImportJob,
    ordinal: i64,
) -> Result<Option<(String, String, String)>, ApiError> {
    sqlx::query_as(
        "SELECT status,message_sha256,provider_email_id FROM mailbox_import_messages
         WHERE import_id=$1 AND message_ordinal=$2",
    )
    .bind(job.id)
    .bind(ordinal)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

fn import_marker(job_id: Uuid, ordinal: i64, message_hash: &str) -> String {
    let marker_seed = format!("{job_id}:{ordinal}:{message_hash}");
    let marker_hash = format!("{:x}", Sha256::digest(marker_seed.as_bytes()));
    format!("cs-mail-import-{marker_hash}")
}

async fn record_failed_import_message(
    state: &AppState,
    job: &ImportJob,
    ordinal: i64,
    hash: &str,
    byte_size: i64,
    error: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO mailbox_import_messages
           (import_id,message_ordinal,message_sha256,provider_email_id,byte_size,status,error)
         VALUES($1,$2,$3,'',$4,'failed',$5)
         ON CONFLICT(import_id,message_ordinal) DO UPDATE
         SET message_sha256=EXCLUDED.message_sha256,provider_email_id='',
             byte_size=EXCLUDED.byte_size,status='failed',error=EXCLUDED.error",
    )
    .bind(job.id)
    .bind(ordinal)
    .bind(hash)
    .bind(byte_size)
    .bind(error.chars().take(1000).collect::<String>())
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn import_one_message(
    state: &AppState,
    job: &ImportJob,
    ordinal: i64,
    provider_account_id: &str,
    inbox_id: &str,
    message: &[u8],
) -> Result<(), StalwartError> {
    let hash = format!("{:x}", Sha256::digest(message));
    let keyword = import_marker(job.id, ordinal, &hash);

    // If the provider import committed but the PostgreSQL ledger commit was
    // lost, the provider marker lets a replacement worker converge on the
    // same source ordinal without creating another copy.
    if let Some(provider_email_id) = state
        .stalwart
        .find_email_by_keyword(provider_account_id, &keyword)
        .await?
    {
        sqlx::query(
            "INSERT INTO mailbox_import_messages
               (import_id,message_ordinal,message_sha256,provider_email_id,byte_size,status)
             VALUES($1,$2,$3,$4,$5,'imported')
             ON CONFLICT(import_id,message_ordinal) DO UPDATE
             SET message_sha256=EXCLUDED.message_sha256,
                 provider_email_id=EXCLUDED.provider_email_id,
                 byte_size=EXCLUDED.byte_size,status='imported',error=''",
        )
        .bind(job.id)
        .bind(ordinal)
        .bind(&hash)
        .bind(&provider_email_id)
        .bind(message.len().min(i64::MAX as usize) as i64)
        .execute(&state.db)
        .await
        .map_err(|e| StalwartError::Protocol {
            operation: "mail import idempotency recovery".into(),
            message: e.to_string(),
        })?;
        let _ = state
            .stalwart
            .remove_email_keyword(provider_account_id, &provider_email_id, &keyword)
            .await;
        return Ok(());
    }

    let blob_id = state
        .stalwart
        .upload_email_blob(provider_account_id, message)
        .await?;
    let received_at = received_at_from_message(message);
    let provider_email_id = state
        .stalwart
        .import_email_blob(
            provider_account_id,
            &blob_id,
            inbox_id,
            received_at.as_deref(),
            Some(&keyword),
        )
        .await?;
    sqlx::query(
        "INSERT INTO mailbox_import_messages
           (import_id,message_ordinal,message_sha256,provider_email_id,byte_size,status)
         VALUES($1,$2,$3,$4,$5,'imported')
         ON CONFLICT(import_id,message_ordinal) DO UPDATE
         SET message_sha256=EXCLUDED.message_sha256,
             provider_email_id=EXCLUDED.provider_email_id,
             byte_size=EXCLUDED.byte_size,status='imported',error=''",
    )
    .bind(job.id)
    .bind(ordinal)
    .bind(&hash)
    .bind(&provider_email_id)
    .bind(message.len().min(i64::MAX as usize) as i64)
    .execute(&state.db)
    .await
    .map_err(|e| StalwartError::Protocol {
        operation: "mail import idempotency commit".into(),
        message: e.to_string(),
    })?;
    if let Err(error) = state
        .stalwart
        .remove_email_keyword(provider_account_id, &provider_email_id, &keyword)
        .await
    {
        tracing::warn!(error=%error, import_id=%job.id, ordinal, "could not remove temporary mail-import keyword");
    }
    Ok(())
}

async fn process_buffered_message(
    state: &AppState,
    job: &ImportJob,
    ordinal: i64,
    provider_account_id: &str,
    inbox_id: &str,
    message: &[u8],
    oversized: bool,
) -> Result<(bool, bool), ApiError> {
    if message.is_empty() && !oversized {
        return Ok((false, false));
    }
    if let Some((status, stored_hash, provider_email_id)) = existing_import_message(state, job, ordinal).await? {
        if status == "imported" {
            if !provider_email_id.is_empty() {
                let keyword = import_marker(job.id, ordinal, &stored_hash);
                let _ = state
                    .stalwart
                    .remove_email_keyword(provider_account_id, &provider_email_id, &keyword)
                    .await;
            }
            return Ok((true, false));
        }
        if status == "failed" {
            return Ok((false, true));
        }
        return Ok((false, false));
    }
    if oversized {
        let hash = format!("{:x}", Sha256::digest(format!("oversized:{}:{ordinal}", job.id).as_bytes()));
        record_failed_import_message(
            state,
            job,
            ordinal,
            &hash,
            state.mail_import_message_max_bytes.min(i64::MAX as u64) as i64,
            "Message exceeds the configured per-message import ceiling",
        )
        .await?;
        return Ok((false, true));
    }
    let hash = format!("{:x}", Sha256::digest(message));
    match import_one_message(state, job, ordinal, provider_account_id, inbox_id, message).await {
        Ok(()) => Ok((true, false)),
        Err(error) if error.is_transient() => Err(provider_error(error)),
        Err(error) => {
            let detail = error.public_message();
            record_failed_import_message(
                state,
                job,
                ordinal,
                &hash,
                message.len().min(i64::MAX as usize) as i64,
                &detail,
            )
            .await?;
            tracing::warn!(error=%error, import_id=%job.id, ordinal, "skipping permanently rejected imported message");
            Ok((false, true))
        }
    }
}

async fn process_import(state: &AppState, worker_id: Uuid, job: &ImportJob) -> Result<(), ApiError> {
    let mailbox: Option<(String, String, String)> = sqlx::query_as(
        "SELECT m.address::text,m.status,COALESCE(m.provider_account_id,'')
         FROM mailboxes m JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         WHERE m.id=$1 AND m.organization_id=$2 AND m.user_id=$3 AND m.deleted_at IS NULL",
    )
    .bind(job.mailbox_id)
    .bind(job.organization_id)
    .bind(job.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (_address, status, provider_account_id) = mailbox
        .ok_or_else(|| ApiError::not_found("Import mailbox no longer exists"))?;
    if status != "active" || provider_account_id.is_empty() {
        return Err(ApiError::conflict("Import mailbox is not active and provider-ready"));
    }
    let inbox_id = state
        .stalwart
        .inbox_id(&provider_account_id)
        .await
        .map_err(provider_error)?;
    let path = checked_import_path(state, &job.storage_key)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal(format!("Mailbox import file is unavailable: {e}")))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::with_capacity(IMPORT_IO_CHUNK);
    let mut message = Vec::new();
    let mut oversized = false;
    let mut saw_delimiter = false;
    let max_message = state.mail_import_message_max_bytes.min(usize::MAX as u64) as usize;
    let mut total = 0i64;
    let mut imported = 0i64;
    let mut failed = 0i64;

    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| ApiError::internal(format!("Mailbox import read failed: {e}")))?;
        if read == 0 {
            break;
        }
        if line.starts_with(b"From ") {
            if saw_delimiter || !message.is_empty() || oversized {
                total += 1;
                let (ok, bad) = process_buffered_message(
                    state,
                    job,
                    total,
                    &provider_account_id,
                    &inbox_id,
                    &message,
                    oversized,
                )
                .await?;
                if ok {
                    imported += 1;
                }
                if bad {
                    failed += 1;
                }
                if total % 10 == 0 {
                    if !import_still_running(state, job, worker_id).await? {
                        return Ok(());
                    }
                    heartbeat_import(state, job, worker_id, total, imported, failed).await?;
                }
            }
            message.clear();
            oversized = false;
            saw_delimiter = true;
            continue;
        }
        if line.starts_with(b">From ") {
            line.remove(0);
        }
        if !oversized {
            if message.len().saturating_add(line.len()) > max_message {
                oversized = true;
                message.clear();
            } else {
                message.extend_from_slice(&line);
            }
        }
    }

    if !message.is_empty() || oversized {
        total += 1;
        let (ok, bad) = process_buffered_message(
            state,
            job,
            total,
            &provider_account_id,
            &inbox_id,
            &message,
            oversized,
        )
        .await?;
        if ok {
            imported += 1;
        }
        if bad {
            failed += 1;
        }
    }

    if !import_still_running(state, job, worker_id).await? {
        return Ok(());
    }
    let completed = sqlx::query(
        "UPDATE mailbox_imports
         SET status='completed',total_messages=$3,imported_messages=$4,failed_messages=$5,
             completed_at=now(),locked_by=NULL,locked_until=NULL,last_error='',updated_at=now()
         WHERE id=$1 AND locked_by=$2 AND status='running'",
    )
    .bind(job.id)
    .bind(worker_id)
    .bind(total)
    .bind(imported)
    .bind(failed)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if completed.rows_affected() != 1 {
        // Cancellation or lease takeover won the race. Never delete the source
        // archive on behalf of a worker that no longer owns the job.
        return Ok(());
    }
    let _ = tokio::fs::remove_file(&path).await;
    audit::record(
        state,
        Some(job.user_id),
        "mail_import.completed",
        json!({ "import_id": job.id, "mailbox_id": job.mailbox_id, "organization_id": job.organization_id, "total": total, "imported": imported, "failed": failed }),
    )
    .await;
    Ok(())
}

async fn fail_or_retry_import(
    state: &AppState,
    worker_id: Uuid,
    job: &ImportJob,
    error: &ApiError,
) {
    let message = error.message.chars().take(1000).collect::<String>();
    let final_failure = job.attempts >= job.max_attempts;
    let delay = 30i64.saturating_mul(1i64 << (job.attempts.saturating_sub(1).min(6) as u32));
    let result = if final_failure {
        sqlx::query(
            "UPDATE mailbox_imports
             SET status='failed',last_error=$2,completed_at=now(),locked_by=NULL,locked_until=NULL,updated_at=now()
             WHERE id=$1 AND status='running' AND locked_by=$3",
        )
        .bind(job.id)
        .bind(&message)
        .bind(worker_id)
        .execute(&state.db)
        .await
    } else {
        sqlx::query(
            "UPDATE mailbox_imports
             SET status='queued',last_error=$2,next_attempt_at=now()+($3*interval '1 second'),
                 locked_by=NULL,locked_until=NULL,updated_at=now()
             WHERE id=$1 AND status='running' AND locked_by=$4",
        )
        .bind(job.id)
        .bind(&message)
        .bind(delay)
        .bind(worker_id)
        .execute(&state.db)
        .await
    };
    match result {
        Ok(done) if done.rows_affected() == 0 => {
            // The lease was cancelled or reclaimed by another replica. The
            // stale worker must not alter job state or delete source bytes.
            tracing::info!(import_id=%job.id, worker_id=%worker_id, "stale import worker discarded failure result");
            return;
        }
        Ok(_) => {}
        Err(db_error) => {
            tracing::error!(error=%db_error, import_id=%job.id, "could not update failed mailbox import");
            return;
        }
    }
    if final_failure {
        // There is no manual retry endpoint after max-attempt failure, so do
        // not retain multi-gigabyte source archives indefinitely. Per-message
        // failure metadata remains in PostgreSQL for operator diagnostics.
        if let Ok(path) = checked_import_path(state, &job.storage_key) {
            let _ = tokio::fs::remove_file(path).await;
        }
        audit::record(
            state,
            Some(job.user_id),
            "mail_import.failed",
            json!({ "import_id": job.id, "mailbox_id": job.mailbox_id, "organization_id": job.organization_id, "error": message }),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ip_masks() {
        assert!(validate_ip_mask("192.0.2.1"));
        assert!(validate_ip_mask("192.0.2.0/24"));
        assert!(validate_ip_mask("2001:db8::/32"));
        assert!(!validate_ip_mask("192.0.2.0/99"));
        assert!(!validate_ip_mask("not-an-ip"));
    }

    #[test]
    fn autoconfig_domain_is_conservative() {
        assert_eq!(public_domain_from_email("a@example.com"), "example.com");
        assert_eq!(public_domain_from_email("<bad>@evil<script>"), "crescentsphere.com");
    }
}
