//! Upgrade 07: staged attachment/blob lifecycle.
//!
//! Raw bytes are uploaded separately from compose JSON and written to a
//! persistent API volume. PostgreSQL stores ownership, integrity and lifetime
//! metadata plus explicit references from drafts / scheduled sends. The mail
//! send path resolves opaque attachment ids back to bytes only when building
//! the final MIME message.

use std::collections::{HashMap, HashSet};
use std::path::{Component, PathBuf};

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::domain::quota::PlanLimits;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{entitlements, mime, tenancy};
use crate::state::AppState;

/// Absolute single-file ceiling independent of plan configuration. Plans may
/// be lower, never higher, until the SMTP/MIME pipeline is redesigned to fully
/// stream final message construction.
pub const MAX_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_DOWNLOAD_BYTES: usize = MAX_ATTACHMENT_BYTES;
const IO_CHUNK_BYTES: usize = 64 * 1024;
const CLEANUP_BATCH: i64 = 200;

const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "msi", "msp", "cpl", "hta", "vbs", "vbe", "js",
    "jse", "wsf", "wsh", "ps1", "psm1", "jar", "app", "dmg", "pkg", "deb", "rpm", "sh", "bash",
    "zsh", "py", "rb", "pl", "php", "lnk", "reg", "scf", "inf", "apk",
];

const ALLOWED_EXACT: &[&str] = &[
    "application/octet-stream",
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.presentation",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/json",
    "application/rtf",
    "message/rfc822",
    "audio/mpeg",
    "audio/mp4",
    "video/mp4",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentRefIn {
    pub id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AttachmentMeta {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    #[serde(rename = "size")]
    pub byte_size: i64,
    pub sha256_hex: String,
    pub status: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct StoredAttachment {
    id: Uuid,
    filename: String,
    content_type: String,
    byte_size: i64,
    sha256_hex: String,
    storage_key: String,
    #[allow(dead_code)]
    status: String,
    #[allow(dead_code)]
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct UploadQuery {
    filename: String,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct LegacyAttachment {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    data_base64: String,
}

pub fn normalise_type(content_type: &str) -> String {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if base.is_empty() {
        "application/octet-stream".to_string()
    } else {
        base
    }
}

fn extension_of(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn safe_filename(filename: &str) -> String {
    let name = filename
        .replace(['\r', '\n'], " ")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(255)
        .collect::<String>();
    if name.is_empty() {
        "attachment".to_string()
    } else {
        name
    }
}

pub fn is_allowed_type(content_type: &str) -> bool {
    let base = normalise_type(content_type);
    if base == "image/svg+xml" {
        return false;
    }
    if let Some(rest) = base.strip_prefix("text/") {
        return !rest.is_empty();
    }
    if let Some(rest) = base.strip_prefix("image/") {
        return !rest.is_empty();
    }
    ALLOWED_EXACT.contains(&base.as_str())
}

fn looks_executable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ") || bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!")
}

fn validate_metadata(filename: &str, content_type: &str) -> Result<(String, String), ApiError> {
    let filename = safe_filename(filename);
    let ext = extension_of(&filename);
    if !ext.is_empty() && BLOCKED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_blocked",
            format!("Files ending in .{ext} are not allowed as attachments"),
        ));
    }

    let content_type = normalise_type(content_type);
    if !is_allowed_type(&content_type) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_not_allowed",
            format!("Attachments of type {content_type} are not allowed"),
        ));
    }
    Ok((filename, content_type))
}

pub fn validate_upload(
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(String, String), ApiError> {
    let hard_max = max_bytes.min(MAX_ATTACHMENT_BYTES);
    if bytes.len() > hard_max {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment_too_large",
            format!("Attachments may be at most {} MiB on your plan", hard_max / (1024 * 1024)),
        ));
    }
    let (filename, content_type) = validate_metadata(filename, content_type)?;
    if looks_executable(bytes) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_blocked",
            "This file appears to be an executable and cannot be attached",
        ));
    }
    Ok((filename, content_type))
}

fn storage_key(mailbox_id: Uuid, id: Uuid) -> String {
    format!("{mailbox_id}/{id}.blob")
}

fn checked_path(state: &AppState, key: &str) -> Result<PathBuf, ApiError> {
    let rel = PathBuf::from(key);
    if rel.is_absolute()
        || rel
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(ApiError::internal("Invalid attachment storage key"));
    }
    Ok(state.attachment_store_dir.join(rel))
}

fn partial_path(final_path: &PathBuf) -> PathBuf {
    let mut partial = final_path.clone();
    partial.set_extension("part");
    partial
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
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before using attachments"))
}

async fn attachment_entitlements(
    state: &AppState,
    mailbox_id: Uuid,
) -> Result<entitlements::UserEntitlements, ApiError> {
    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT organization_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL AND status='active'",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::forbidden("The selected business mailbox is not active"))?;
    let ent = entitlements::for_organization(state, organization_id).await?;
    if !ent.allows("attachments") {
        return Err(ApiError::forbidden("Attachments are not enabled for this business subscription"));
    }
    Ok(ent)
}

async fn reserve_upload(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    organization_id: Uuid,
    pool_limit: i64,
    mailbox_limit: i64,
    provider_used: i64,
    id: Uuid,
    filename: &str,
    content_type: &str,
    reserve_bytes: i64,
    key: &str,
) -> Result<(), ApiError> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Serialize reservations across every mailbox in the organization.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(organization_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let (staged_mailbox, staged_org, mailbox_used, org_used): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
           COALESCE((SELECT SUM(reserved_bytes) FROM staged_attachments WHERE mailbox_id=$1),0)::bigint,
           COALESCE((SELECT SUM(sa.reserved_bytes) FROM staged_attachments sa JOIN mailboxes m ON m.id=sa.mailbox_id WHERE m.organization_id=$2),0)::bigint,
           COALESCE((SELECT quota_used FROM realtime_mailbox_state WHERE mailbox_id=$1),0)::bigint,
           COALESCE((SELECT storage_bytes FROM organization_usage WHERE organization_id=$2),0)::bigint",
    )
    .bind(mailbox_id)
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if org_used.saturating_add(staged_org).saturating_add(reserve_bytes) > pool_limit {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "organization_storage_quota",
            "This business does not have enough remaining storage for this attachment"));
    }
    if mailbox_used.max(provider_used).saturating_add(staged_mailbox).saturating_add(reserve_bytes) > mailbox_limit {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "mailbox_storage_quota",
            "This mailbox does not have enough remaining storage for this attachment"));
    }

    let limit = state.attachment_staging_quota_bytes.min(i64::MAX as u64) as i64;
    if staged_mailbox.saturating_add(reserve_bytes) > limit {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment_staging_quota",
            format!(
                "Attachment staging is full for this mailbox ({} MiB limit)",
                limit / (1024 * 1024)
            ),
        ));
    }

    sqlx::query(
        "INSERT INTO staged_attachments
           (id, user_id, mailbox_id, filename, content_type, byte_size, reserved_bytes,
            storage_key, status, expires_at)
         VALUES ($1, $2, $3, $4, $5, 0, $6, $7, 'uploading',
                 now() + ($8 * interval '1 second'))",
    )
    .bind(id)
    .bind(user_id)
    .bind(mailbox_id)
    .bind(filename)
    .bind(content_type)
    .bind(reserve_bytes)
    .bind(key)
    .bind(state.attachment_upload_ttl_secs as i64)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn remove_upload_row(state: &AppState, id: Uuid, mailbox_id: Uuid) {
    let _ = sqlx::query("DELETE FROM staged_attachments WHERE id = $1 AND mailbox_id = $2")
        .bind(id)
        .bind(mailbox_id)
        .execute(&state.db)
        .await;
}

/// `POST /api/attachments?filename=...` — raw request body, streamed to disk.
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let ent = attachment_entitlements(&state, mailbox_id).await?;
    let plan_max = ent.plan.max_attachment_bytes.min(MAX_ATTACHMENT_BYTES);
    if plan_max == 0 {
        return Err(ApiError::forbidden("Attachments are disabled for this plan"));
    }
    let header_size = headers.get(header::CONTENT_LENGTH).and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if header_size.is_some() && query.size.is_some() && header_size != query.size {
        return Err(ApiError::bad_request("Attachment size does not match Content-Length"));
    }
    let declared_size = query.size.or(header_size);
    let reserve_bytes = declared_size.unwrap_or(plan_max as u64);
    if reserve_bytes > plan_max as u64 {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "attachment_too_large",
            format!("Attachments may be at most {} MiB on your plan", plan_max / (1024 * 1024))));
    }
    let mailbox_limit: i64 = sqlx::query_scalar("SELECT COALESCE(quota_bytes,$2) FROM mailboxes WHERE id=$1")
        .bind(mailbox_id).bind(ent.quota_bytes.min(i64::MAX as u64) as i64)
        .fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let provider_account: Option<String> = sqlx::query_scalar("SELECT provider_account_id FROM mailboxes WHERE id=$1")
        .bind(mailbox_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let provider_used = if let Some(account) = provider_account.as_deref().filter(|value| !value.is_empty()) {
        state.stalwart.account_quota(account).await.ok().flatten()
            .map(|(used, _)| used.min(i64::MAX as u64) as i64).unwrap_or(0)
    } else { 0 };

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let (filename, content_type) = validate_metadata(&query.filename, content_type)?;

    let id = Uuid::new_v4();
    let key = storage_key(mailbox_id, id);
    // Unknown-length clients reserve the full file allowance; declared-length
    // clients reserve exactly their bytes. The stream is bounded below.
    reserve_upload(
        &state,
        auth.user_id,
        mailbox_id,
        ent.organization_id,
        ent.storage_pool_bytes.min(i64::MAX as u64) as i64,
        mailbox_limit.max(0),
        provider_used,
        id,
        &filename,
        &content_type,
        reserve_bytes as i64,
        &key,
    )
    .await?;

    let final_path = checked_path(&state, &key)?;
    let part_path = partial_path(&final_path);
    if let Some(parent) = final_path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            remove_upload_row(&state, id, mailbox_id).await;
            return Err(ApiError::internal(format!("Unable to prepare attachment storage: {e}")));
        }
    }

    let mut file = match tokio::fs::File::create(&part_path).await {
        Ok(file) => file,
        Err(e) => {
            remove_upload_row(&state, id, mailbox_id).await;
            return Err(ApiError::internal(format!("Unable to open attachment storage: {e}")));
        }
    };

    let mut stream = body.into_data_stream();
    let mut size = 0usize;
    let mut digest = Sha256::new();
    let mut probe = Vec::with_capacity(16);

    let result: Result<(), ApiError> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::bad_request(format!("Upload stream failed: {e}")))?;
            size = size.saturating_add(chunk.len());
            if size > reserve_bytes as usize {
                return Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "attachment_too_large",
                    format!("Attachments may be at most {} MiB on your plan", plan_max / (1024 * 1024)),
                ));
            }
            if probe.len() < 16 {
                let take = (16 - probe.len()).min(chunk.len());
                probe.extend_from_slice(&chunk[..take]);
            }
            digest.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::internal(format!("Attachment write failed: {e}")))?;
        }
        file.flush()
            .await
            .map_err(|e| ApiError::internal(format!("Attachment flush failed: {e}")))?;
        if declared_size.is_some_and(|declared| declared != size as u64) {
            return Err(ApiError::bad_request("Attachment size differs from the declared size"));
        }
        if looks_executable(&probe) {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "attachment_type_blocked",
                "This file appears to be an executable and cannot be attached",
            ));
        }
        Ok(())
    }
    .await;

    if let Err(error) = result {
        drop(file);
        let _ = tokio::fs::remove_file(&part_path).await;
        remove_upload_row(&state, id, mailbox_id).await;
        return Err(error);
    }
    drop(file);

    if let Err(e) = tokio::fs::rename(&part_path, &final_path).await {
        let _ = tokio::fs::remove_file(&part_path).await;
        remove_upload_row(&state, id, mailbox_id).await;
        return Err(ApiError::internal(format!("Attachment finalize failed: {e}")));
    }

    let sha256_hex = format!("{:x}", digest.finalize());
    let meta: AttachmentMeta = sqlx::query_as(
        "UPDATE staged_attachments
            SET byte_size = $3, reserved_bytes = $3, sha256_hex = $4,
                status = 'ready', updated_at = now(),
                expires_at = now() + ($5 * interval '1 second')
          WHERE id = $1 AND mailbox_id = $2
          RETURNING id, filename, content_type, byte_size, sha256_hex, status, expires_at",
    )
    .bind(id)
    .bind(mailbox_id)
    .bind(size as i64)
    .bind(sha256_hex)
    .bind(state.attachment_upload_ttl_secs as i64)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "attachment": meta,
            "download_url": format!("/api/attachments/{}", meta.id),
        })),
    ))
}

async fn own_attachment(state: &AppState, mailbox_id: Uuid, id: Uuid) -> Result<StoredAttachment, ApiError> {
    sqlx::query_as::<_, StoredAttachment>(
        "SELECT id, filename, content_type, byte_size, sha256_hex, storage_key, status, expires_at
         FROM staged_attachments WHERE id = $1 AND mailbox_id = $2 AND status IN ('ready','consumed')",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Attachment not found"))
}

/// Authenticated download/preview of a staged attachment owned by the user.
pub async fn download(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    attachment_entitlements(&state, mailbox_id).await?;
    let item = own_attachment(&state, mailbox_id, id).await?;
    let path = checked_path(&state, &item.storage_key)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::not_found("Attachment bytes are unavailable"))?;

    let stream = stream::unfold(file, |mut file| async move {
        let mut buffer = vec![0u8; IO_CHUNK_BYTES];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(n) => {
                buffer.truncate(n);
                Some((Ok::<Bytes, std::io::Error>(Bytes::from(buffer)), file))
            }
            Err(error) => Some((Err(error), file)),
        }
    });

    let ascii_name: String = item
        .filename
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' })
        .collect();
    let disposition = format!("attachment; filename=\"{}\"", ascii_name);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, item.content_type)
        .header(header::CONTENT_LENGTH, item.byte_size.to_string())
        .header(header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).unwrap_or_else(|_| HeaderValue::from_static("attachment")))
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(stream))
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "DELETE FROM staged_attachments a
         WHERE a.id = $1 AND a.mailbox_id = $2
           AND NOT EXISTS (SELECT 1 FROM attachment_refs r WHERE r.attachment_id = a.id)
         RETURNING a.storage_key",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let Some((key,)) = row else {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM staged_attachments WHERE id = $1 AND mailbox_id = $2)",
        )
        .bind(id)
        .bind(mailbox_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
        if exists {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "attachment_in_use",
                "This attachment is still referenced by a draft or scheduled message",
            ));
        }
        return Err(ApiError::not_found("Attachment not found"));
    };

    if let Ok(path) = checked_path(&state, &key) {
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(partial_path(&path)).await;
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn resolve_refs(
    state: &AppState,
    mailbox_id: Uuid,
    refs: &[AttachmentRefIn],
    plan: &PlanLimits,
) -> Result<Vec<AttachmentMeta>, ApiError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    if !plan.allows("attachments") {
        return Err(ApiError::forbidden("Attachments are not included in your current plan"));
    }

    let mut ids = Vec::with_capacity(refs.len());
    let mut seen = HashSet::new();
    for item in refs {
        if !seen.insert(item.id) {
            return Err(ApiError::bad_request("The same attachment cannot be added twice"));
        }
        ids.push(item.id);
    }

    let rows: Vec<AttachmentMeta> = sqlx::query_as(
        "SELECT id, filename, content_type, byte_size, sha256_hex, status, expires_at
         FROM staged_attachments
         WHERE mailbox_id = $1 AND id = ANY($2) AND status IN ('ready','consumed')",
    )
    .bind(mailbox_id)
    .bind(&ids)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if rows.len() != ids.len() {
        return Err(ApiError::bad_request("One or more attachments is missing or no longer available"));
    }
    let by_id = rows.into_iter().map(|row| (row.id, row)).collect::<HashMap<_, _>>();
    let mut ordered = Vec::with_capacity(ids.len());
    let mut total = 0usize;
    let hard_single = plan.max_attachment_bytes.min(MAX_ATTACHMENT_BYTES);
    for id in ids {
        let item = by_id.get(&id).cloned().ok_or_else(|| ApiError::bad_request("Attachment not found"))?;
        let size = item.byte_size.max(0) as usize;
        if size > hard_single {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "attachment_too_large",
                format!("Attachments may be at most {} MiB on your plan", hard_single / (1024 * 1024)),
            ));
        }
        total = total.saturating_add(size);
        if total > plan.max_total_attachment_bytes.min(MAX_ATTACHMENT_BYTES) {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "attachments_too_large",
                format!(
                    "Attachments for one message may total at most {} MiB on your plan",
                    plan.max_total_attachment_bytes.min(MAX_ATTACHMENT_BYTES) / (1024 * 1024)
                ),
            ));
        }
        ordered.push(item);
    }
    Ok(ordered)
}

pub fn refs_json(items: &[AttachmentMeta]) -> Value {
    json!(items)
}

pub fn ids_from_meta_value(value: &Value) -> Vec<Uuid> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter_map(|id| Uuid::parse_str(id).ok())
        .collect()
}

pub async fn load_for_message(
    state: &AppState,
    mailbox_id: Uuid,
    refs: &[AttachmentRefIn],
    plan: &PlanLimits,
) -> Result<(Vec<mime::Attachment>, Vec<Uuid>), ApiError> {
    let meta = resolve_refs(state, mailbox_id, refs, plan).await?;
    if meta.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let ids = meta.iter().map(|item| item.id).collect::<Vec<_>>();
    let rows: Vec<StoredAttachment> = sqlx::query_as(
        "SELECT id, filename, content_type, byte_size, sha256_hex, storage_key, status, expires_at
         FROM staged_attachments WHERE mailbox_id = $1 AND id = ANY($2)",
    )
    .bind(mailbox_id)
    .bind(&ids)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let by_id = rows.into_iter().map(|row| (row.id, row)).collect::<HashMap<_, _>>();

    let mut out = Vec::with_capacity(ids.len());
    for id in &ids {
        let item = by_id.get(id).ok_or_else(|| ApiError::bad_request("Attachment is unavailable"))?;
        let path = checked_path(state, &item.storage_key)?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| ApiError::bad_request("Attachment bytes are unavailable; re-add the file"))?;
        if bytes.len() as i64 != item.byte_size {
            return Err(ApiError::internal("Attachment size no longer matches stored metadata"));
        }
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != item.sha256_hex {
            return Err(ApiError::internal("Attachment integrity verification failed"));
        }
        out.push(mime::Attachment {
            filename: item.filename.clone(),
            content_type: item.content_type.clone(),
            bytes,
        });
    }
    Ok((out, ids))
}

pub async fn replace_owner_refs_tx(
    tx: &mut Transaction<'_, Postgres>,
    mailbox_id: Uuid,
    owner_type: &str,
    owner_id: Uuid,
    attachment_ids: &[Uuid],
    protect_until: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM attachment_refs WHERE mailbox_id = $1 AND owner_type = $2 AND owner_id = $3")
        .bind(mailbox_id)
        .bind(owner_type)
        .bind(owner_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    for id in attachment_ids {
        let inserted = sqlx::query(
            "INSERT INTO attachment_refs (attachment_id, user_id, mailbox_id, owner_type, owner_id)
             SELECT id, user_id, mailbox_id, $3, $4 FROM staged_attachments
             WHERE id = $1 AND mailbox_id = $2 AND status IN ('ready','consumed')",
        )
        .bind(id)
        .bind(mailbox_id)
        .bind(owner_type)
        .bind(owner_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();
        if inserted != 1 {
            return Err(ApiError::bad_request(
                "An attachment expired while the message was being saved; re-add it",
            ));
        }
    }

    if !attachment_ids.is_empty() {
        sqlx::query(
            "UPDATE staged_attachments SET expires_at = GREATEST(expires_at, $3), updated_at = now()
             WHERE mailbox_id = $1 AND id = ANY($2)",
        )
        .bind(mailbox_id)
        .bind(attachment_ids)
        .bind(protect_until)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(())
}

pub async fn owner_attachment_ids(
    state: &AppState,
    mailbox_id: Uuid,
    owner_type: &str,
    owner_id: Uuid,
) -> Result<Vec<Uuid>, ApiError> {
    sqlx::query_scalar(
        "SELECT attachment_id FROM attachment_refs
         WHERE mailbox_id = $1 AND owner_type = $2 AND owner_id = $3",
    )
    .bind(mailbox_id)
    .bind(owner_type)
    .bind(owner_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn mark_consumed(state: &AppState, mailbox_id: Uuid, ids: &[Uuid]) {
    if ids.is_empty() {
        return;
    }
    let _ = sqlx::query(
        "UPDATE staged_attachments
            SET status = 'consumed',
                expires_at = LEAST(expires_at, now() + ($3 * interval '1 second')),
                updated_at = now()
          WHERE mailbox_id = $1 AND id = ANY($2)",
    )
    .bind(mailbox_id)
    .bind(ids)
    .bind(state.attachment_consumed_grace_secs as i64)
    .execute(&state.db)
    .await;
}

async fn cleanup_once(state: &AppState) -> Result<usize, ApiError> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "WITH doomed AS (
           SELECT a.id
           FROM staged_attachments a
           WHERE a.expires_at <= now()
             AND NOT EXISTS (SELECT 1 FROM attachment_refs r WHERE r.attachment_id = a.id)
           ORDER BY a.expires_at
           FOR UPDATE SKIP LOCKED
           LIMIT $1
         )
         DELETE FROM staged_attachments a
         USING doomed d
         WHERE a.id = d.id
         RETURNING a.id, a.storage_key",
    )
    .bind(CLEANUP_BATCH)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    for (_, key) in &rows {
        if let Ok(path) = checked_path(state, key) {
            let _ = tokio::fs::remove_file(&path).await;
            let _ = tokio::fs::remove_file(partial_path(&path)).await;
        }
    }
    Ok(rows.len())
}

pub fn spawn_cleanup_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            state.attachment_cleanup_secs.max(60),
        ));
        loop {
            tick.tick().await;
            match cleanup_once(&state).await {
                Ok(count) if count > 0 => tracing::info!(count, "expired staged attachments cleaned"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "attachment cleanup failed"),
            }
        }
    });
}

async fn stage_legacy_bytes(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    legacy: LegacyAttachment,
) -> Result<AttachmentMeta, ApiError> {
    let ent = attachment_entitlements(state, mailbox_id).await?;
    let bytes = B64
        .decode(legacy.data_base64.as_bytes())
        .map_err(|_| ApiError::bad_request("Legacy attachment data is not valid base64"))?;
    let (filename, content_type) = validate_upload(
        &legacy.filename,
        &legacy.content_type,
        &bytes,
        ent.plan.max_attachment_bytes.min(MAX_ATTACHMENT_BYTES),
    )?;
    let id = Uuid::new_v4();
    let key = storage_key(mailbox_id, id);
    let path = checked_path(state, &key)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let sha256_hex = format!("{:x}", Sha256::digest(&bytes));
    let inserted = sqlx::query_as(
        "INSERT INTO staged_attachments
           (id, user_id, mailbox_id, filename, content_type, byte_size, reserved_bytes,
            sha256_hex, storage_key, status, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$6,$7,$8,'ready',now() + ($9 * interval '1 second'))
         RETURNING id, filename, content_type, byte_size, sha256_hex, status, expires_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(mailbox_id)
    .bind(filename)
    .bind(content_type)
    .bind(bytes.len() as i64)
    .bind(sha256_hex)
    .bind(key)
    .bind(state.attachment_upload_ttl_secs as i64)
    .fetch_one(&state.db)
    .await;

    match inserted {
        Ok(meta) => Ok(meta),
        Err(error) => {
            let _ = tokio::fs::remove_file(&path).await;
            Err(ApiError::internal(error.to_string()))
        }
    }
}

async fn migrate_legacy_drafts(state: &AppState) -> Result<usize, ApiError> {
    let rows: Vec<(Uuid, Uuid, Uuid, Value)> = sqlx::query_as(
        "SELECT id, user_id, mailbox_id, attachments FROM mail_drafts
         WHERE mailbox_id IS NOT NULL AND attachments::text LIKE '%data_base64%'",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut migrated = 0;
    for (id, user_id, mailbox_id, value) in rows {
        let Some(items) = value.as_array() else { continue };
        let mut metas = Vec::new();
        for item in items {
            if item.get("id").is_some() {
                if let Ok(meta) = serde_json::from_value::<AttachmentMeta>(item.clone()) {
                    metas.push(meta);
                }
                continue;
            }
            let legacy: LegacyAttachment = match serde_json::from_value(item.clone()) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if legacy.data_base64.is_empty() {
                continue;
            }
            metas.push(stage_legacy_bytes(state, user_id, mailbox_id, legacy).await?);
        }
        let ids = metas.iter().map(|meta| meta.id).collect::<Vec<_>>();
        let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mail_drafts SET attachments = $1, updated_at = now() WHERE id = $2 AND user_id = $3 AND mailbox_id = $4")
            .bind(refs_json(&metas))
            .bind(id)
            .bind(user_id)
            .bind(mailbox_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        replace_owner_refs_tx(
            &mut tx,
            mailbox_id,
            "draft",
            id,
            &ids,
            Utc::now() + ChronoDuration::seconds(state.attachment_draft_ttl_secs as i64),
        )
        .await?;
        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
        migrated += 1;
    }
    Ok(migrated)
}

async fn migrate_legacy_scheduled(state: &AppState) -> Result<usize, ApiError> {
    let rows: Vec<(Uuid, Uuid, Uuid, DateTime<Utc>, Value)> = sqlx::query_as(
        "SELECT id, user_id, mailbox_id, send_at, compose FROM scheduled_sends
         WHERE mailbox_id IS NOT NULL AND compose::text LIKE '%data_base64%'",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut migrated = 0;
    for (id, user_id, mailbox_id, send_at, mut compose) in rows {
        let items = compose
            .get("attachments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut metas = Vec::new();
        for item in items {
            if item.get("id").is_some() {
                if let Ok(meta) = serde_json::from_value::<AttachmentMeta>(item) {
                    metas.push(meta);
                }
                continue;
            }
            let legacy: LegacyAttachment = match serde_json::from_value(item) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if legacy.data_base64.is_empty() {
                continue;
            }
            metas.push(stage_legacy_bytes(state, user_id, mailbox_id, legacy).await?);
        }
        if let Some(object) = compose.as_object_mut() {
            object.insert("attachments".to_string(), refs_json(&metas));
        }
        let ids = metas.iter().map(|meta| meta.id).collect::<Vec<_>>();
        let protect_until = std::cmp::max(
            send_at + ChronoDuration::days(7),
            Utc::now() + ChronoDuration::seconds(state.attachment_upload_ttl_secs as i64),
        );
        let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE scheduled_sends SET compose = $1, updated_at = now() WHERE id = $2 AND user_id = $3 AND mailbox_id = $4")
            .bind(compose)
            .bind(id)
            .bind(user_id)
            .bind(mailbox_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        replace_owner_refs_tx(&mut tx, mailbox_id, "scheduled", id, &ids, protect_until).await?;
        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
        migrated += 1;
    }
    Ok(migrated)
}

/// Runtime file migration is required because SQL migrations cannot move the
/// old base64 bytes from JSON into the persistent attachment volume. It is
/// idempotent: rows stop matching once their arrays contain only opaque ids.
pub async fn migrate_legacy_inline_attachments(state: &AppState) {
    match migrate_legacy_drafts(state).await {
        Ok(count) if count > 0 => tracing::info!(count, "legacy draft attachments migrated"),
        Ok(_) => {}
        Err(error) => tracing::error!(%error, "legacy draft attachment migration failed; will retry on next boot"),
    }
    match migrate_legacy_scheduled(state).await {
        Ok(count) if count > 0 => tracing::info!(count, "legacy scheduled attachments migrated"),
        Ok(_) => {}
        Err(error) => tracing::error!(%error, "legacy scheduled attachment migration failed; will retry on next boot"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_executable_extensions_and_magic() {
        assert!(validate_upload("invoice.exe", "application/pdf", b"hello", 1024).is_err());
        assert!(validate_upload("invoice.pdf", "application/pdf", b"MZ\x90\x00", 1024).is_err());
    }

    #[test]
    fn allows_common_and_unknown_binary_payloads() {
        assert!(validate_upload("report.pdf", "application/pdf", b"hello", 1024).is_ok());
        assert!(validate_upload("archive.bin", "application/octet-stream", b"hello", 1024).is_ok());
    }

    #[test]
    fn sanitizes_paths() {
        let (name, _) = validate_upload("../../etc/passwd.txt", "text/plain", b"x", 1024).unwrap();
        assert_eq!(name, "passwd.txt");
    }

    #[test]
    fn plan_cap_is_enforced() {
        let err = validate_upload("big.pdf", "application/pdf", b"12345", 4).unwrap_err();
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
