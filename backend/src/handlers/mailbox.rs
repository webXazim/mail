//! Mailbox API (WS2 / Upgrade 19 tenancy). Thin HTTP layer over `services::imap`,
//! which talks to Stalwart via an impersonating admin JMAP session keyed to the
//! authenticated user's active hosted `mailboxes` row. A platform login email is
//! never treated as provider mailbox authority.
//!
//! Contract (all child routes require the access-token cookie):
//!   GET    /api/mail/mailboxes
//!   POST   /api/mail/mailboxes
//!   PUT    /api/mail/mailboxes/:id
//!   DELETE /api/mail/mailboxes/:id
//!   POST   /api/mail/mailboxes/:id/empty
//!   GET    /api/mail/threads?scope=<mailbox|all|unread|starred>&mailbox=<id>&limit=<n>&anchor=<emailId>
//!   GET    /api/mail/thread/:thread_id
//!   POST   /api/mail/threads/state   { emailIds, read?, starred? }
//!   POST   /api/mail/threads/move    { emailIds, toMailbox, fromMailbox? }
//!   POST   /api/mail/threads/destroy { emailIds }
//!   GET    /api/mail/search?q=<query>&limit=<n>&anchor=<emailId>&query_state=<state>&sort=<sort>&tz_offset_minutes=<n>
//!   GET    /api/mail/attachment/:blob_id

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::Deserialize;
use std::collections::HashMap;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::handlers::attachments;
use crate::middleware::auth::AuthUser;
use crate::services::{automation, entitlements, imap, tenancy};
use crate::services::stalwart::StalwartError;
use crate::state::AppState;
use crate::ws::emit_event;

const DEFAULT_LIMIT: usize = 25;

fn bridge_err(e: impl std::fmt::Display) -> ApiError {
    // The mail store is an upstream dependency: surface as 502, log the real
    // reason server-side (ApiError hides internals on 5xx).
    ApiError::new(StatusCode::BAD_GATEWAY, "mail_store", e.to_string())
}

async fn announce_mailbox_change(state: &AppState, user: &AuthUser, action: &str, folders_changed: bool) {
    let mailbox_id = match selected_mailbox(state, user).await {
        Ok(Some(mailbox)) => mailbox.id,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(user_id=%user.user_id,%action,"failed to resolve mailbox for realtime event: {err}");
            return;
        }
    };
    if let Err(err) = emit_event(
        state,
        user.user_id,
        "resource-changed",
        json!({
            "resource": "mailbox",
            "action": action,
            "folders_changed": folders_changed,
            "mailbox_id": mailbox_id
        }),
    )
    .await
    {
        tracing::warn!(user_id=%user.user_id,%mailbox_id,%action,"failed to publish mailbox realtime event: {err}");
    }
}

/// Resolve the active hosted mailbox's Stalwart account id. The authoritative
/// provider reference lives on `mailboxes`; a lazy provider lookup is only a
/// reconciliation path for an existing hosted mailbox. `None` means this login
/// does not have a provisioned mailbox yet.
async fn selected_mailbox(state: &AppState, user: &AuthUser) -> Result<Option<tenancy::ActiveMailbox>, ApiError> {
    tenancy::active_mailbox(&state.db, user.user_id, user.organization_id_hint, user.mailbox_id_hint).await
}

async fn account_for(state: &AppState, user: &AuthUser) -> Result<Option<String>, ApiError> {
    entitlements::require_feature(state, user.user_id, "mail").await?;
    let Some(mailbox) = selected_mailbox(state, user).await? else {
        return Ok(None);
    };
    if mailbox.status == "suspended" {
        return Err(ApiError::forbidden("This business mailbox is suspended"));
    }
    if mailbox.status != "active" {
        return Err(ApiError::forbidden("This business mailbox is not active"));
    }
    if let Some(id) = mailbox.provider_account_id.as_deref().filter(|value| !value.is_empty()) {
        return Ok(Some(id.to_string()));
    }
    if !state.stalwart.enabled() {
        return Ok(None);
    }
    match state.stalwart.find_owned_mailbox_account(
        &mailbox.address,
        mailbox.provider_domain_id.as_deref(),
        &mailbox.provider_marker,
        mailbox.domain_is_system,
    ).await {
        Ok(Some(id)) => {
            let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query(
                "UPDATE mailboxes SET provider_account_id=$1, sync_status='ready', sync_error='', updated_at=now() WHERE id=$2",
            )
            .bind(&id)
            .bind(mailbox.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
            tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
            Ok(Some(id))
        }
        Ok(None) => Ok(None),
        Err(StalwartError::UnsupportedDomain(_)) => Ok(None),
        Err(e) => Err(bridge_err(e)),
    }
}

pub async fn mailboxes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({ "mailboxes": [], "state": null })));
    };
    let mut snapshot = imap::mailbox_snapshot(&state.stalwart, &account)
        .await
        .map_err(bridge_err)?;
    let trash_id = snapshot.get("mailboxes")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|mb| mb.get("role").and_then(Value::as_str) == Some("trash")))
        .and_then(|mb| mb.get("id").and_then(Value::as_str))
        .map(str::to_string);
    let virtual_counts = imap::virtual_counts(&state.stalwart, &account, trash_id.as_deref())
        .await
        .map_err(bridge_err)?;
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("virtual_counts".into(), virtual_counts);
    }
    Ok(Json(snapshot))
}

#[derive(Deserialize)]
pub struct ThreadsQuery {
    #[serde(default)]
    mailbox: Option<String>,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    anchor: Option<String>,
    #[serde(default)]
    query_state: Option<String>,
    #[serde(default = "default_sort")]
    sort: String,
    #[serde(default)]
    unread: bool,
    #[serde(default)]
    starred: bool,
    #[serde(default)]
    attachment: bool,
}

pub async fn threads(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ThreadsQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 100);
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({
            "emails": [], "has_more": false, "next_anchor": null,
            "query_state": null, "reset_required": false, "total": 0
        })));
    };
    let scope = params.scope.trim().to_ascii_lowercase();
    if scope == "mailbox" && params.mailbox.as_deref().unwrap_or("").is_empty() {
        return Err(ApiError::bad_request("mailbox is required for mailbox scope"));
    }
    let result = imap::thread_previews(
        &state.stalwart,
        &account,
        &scope,
        params.mailbox.as_deref(),
        limit,
        params.anchor.as_deref(),
        params.query_state.as_deref(),
        &params.sort,
        params.unread,
        params.starred,
        params.attachment,
    )
    .await
    .map_err(bridge_err)?;
    Ok(Json(result))
}

pub async fn thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(thread_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let result = imap::thread(&state.stalwart, &account, &thread_id)
        .await
        .map_err(bridge_err)?;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct MailboxCreateIn {
    name: String,
}

#[derive(Deserialize)]
pub struct MailboxUpdateIn {
    name: String,
}

fn clean_mailbox_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Folder name is required"));
    }
    if name.chars().count() > 255 {
        return Err(ApiError::bad_request("Folder name is too long"));
    }
    if name.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad_request("Folder name contains invalid characters"));
    }
    Ok(name.to_string())
}

pub async fn create_mailbox(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MailboxCreateIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let name = clean_mailbox_name(&body.name)?;
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let id = imap::create_mailbox(&state.stalwart, &account, &name, None)
        .await.map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "folder-created", true).await;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name }))))
}

pub async fn update_mailbox(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(mailbox_id): Path<String>,
    Json(body): Json<MailboxUpdateIn>,
) -> Result<Json<Value>, ApiError> {
    let name = clean_mailbox_name(&body.name)?;
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let mailbox = imap::mailbox_by_id(&state.stalwart, &account, &mailbox_id)
        .await.map_err(bridge_err)?
        .ok_or_else(|| ApiError::not_found("Folder not found"))?;
    if mailbox.get("role").and_then(Value::as_str).is_some() {
        return Err(ApiError::forbidden("System mailboxes cannot be renamed"));
    }
    let old_name = mailbox.get("name").and_then(Value::as_str).unwrap_or("");
    let active = selected_mailbox(&state, &auth).await?.ok_or_else(|| ApiError::not_found("No mailbox for this account"))?;
    let references = automation::folder_reference_count(&state, active.id, old_name)
        .await
        .map_err(ApiError::internal)?;
    if references > 0 {
        return Err(ApiError::conflict(
            "This folder is used by an incoming mail rule. Update that rule before renaming the folder",
        ));
    }
    imap::rename_mailbox(&state.stalwart, &account, &mailbox_id, &name)
        .await.map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "folder-renamed", true).await;
    Ok(Json(json!({ "ok": true, "id": mailbox_id, "name": name })))
}

pub async fn delete_mailbox(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(mailbox_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let mailbox = imap::mailbox_by_id(&state.stalwart, &account, &mailbox_id)
        .await.map_err(bridge_err)?
        .ok_or_else(|| ApiError::not_found("Folder not found"))?;
    if mailbox.get("role").and_then(Value::as_str).is_some() {
        return Err(ApiError::forbidden("System mailboxes cannot be deleted"));
    }
    let folder_name = mailbox.get("name").and_then(Value::as_str).unwrap_or("");
    let active = selected_mailbox(&state, &auth).await?.ok_or_else(|| ApiError::not_found("No mailbox for this account"))?;
    let references = automation::folder_reference_count(&state, active.id, folder_name)
        .await
        .map_err(ApiError::internal)?;
    if references > 0 {
        return Err(ApiError::conflict(
            "This folder is used by an incoming mail rule. Update that rule before deleting the folder",
        ));
    }
    match imap::destroy_mailbox(&state.stalwart, &account, &mailbox_id).await {
        Ok(()) => {
            announce_mailbox_change(&state, &auth, "folder-deleted", true).await;
            Ok(Json(json!({ "ok": true })))
        }
        Err(err) if err.contains("mailboxHasEmail") || err.contains("mailboxHasChild") => {
            Err(ApiError::conflict("Folder must be empty and have no child folders before deletion"))
        }
        Err(err) => Err(bridge_err(err)),
    }
}

pub async fn empty_mailbox(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(mailbox_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let mailbox = imap::mailbox_by_id(&state.stalwart, &account, &mailbox_id)
        .await.map_err(bridge_err)?
        .ok_or_else(|| ApiError::not_found("Folder not found"))?;
    if mailbox.get("role").and_then(Value::as_str) != Some("trash") {
        return Err(ApiError::forbidden("Only Trash can be emptied with this operation"));
    }
    let deleted = imap::empty_mailbox(&state.stalwart, &account, &mailbox_id)
        .await.map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "mailbox-emptied", true).await;
    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

#[derive(Deserialize)]
pub struct StateIn {
    #[serde(rename = "emailIds")]
    email_ids: Vec<String>,
    #[serde(default)]
    read: Option<bool>,
    #[serde(default)]
    starred: Option<bool>,
}

pub async fn set_state(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StateIn>,
) -> Result<Json<Value>, ApiError> {
    if body.read.is_none() && body.starred.is_none() {
        return Err(ApiError::bad_request("Provide read and/or starred"));
    }
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    imap::set_flags(
        &state.stalwart,
        &account,
        &body.email_ids,
        body.read,
        body.starred,
    )
    .await
    .map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "flags-changed", true).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct MoveIn {
    #[serde(rename = "emailIds")]
    email_ids: Vec<String>,
    #[serde(rename = "toMailbox", default)]
    to_mailbox: Option<String>,
    #[serde(rename = "toRole", default)]
    to_role: Option<String>,
    #[serde(rename = "fromMailbox", default)]
    from_mailbox: Option<String>,
    #[serde(rename = "fromMailboxes", default)]
    from_mailboxes: HashMap<String, String>,
}

pub async fn move_emails(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MoveIn>,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let destination = if let Some(id) = body.to_mailbox.as_deref().filter(|id| !id.is_empty()) {
        id.to_string()
    } else if let Some(role) = body.to_role.as_deref().filter(|role| !role.is_empty()) {
        let label = match role {
            "archive" => "Archive",
            "inbox" => "Inbox",
            "sent" => "Sent",
            "drafts" => "Drafts",
            "junk" => "Spam",
            "trash" => "Trash",
            _ => return Err(ApiError::bad_request("Unsupported destination role")),
        };
        if role == "archive" {
            imap::ensure_role_mailbox(&state.stalwart, &account, role, label)
                .await.map_err(bridge_err)?
        } else {
            imap::mailbox_id_for_role(&state.stalwart, &account, role)
                .await.map_err(bridge_err)?
                .ok_or_else(|| ApiError::not_found(format!("{label} mailbox not found")))?
        }
    } else {
        return Err(ApiError::bad_request("toMailbox or toRole is required"));
    };
    imap::move_emails(
        &state.stalwart,
        &account,
        &body.email_ids,
        body.from_mailbox.as_deref(),
        if body.from_mailboxes.is_empty() { None } else { Some(&body.from_mailboxes) },
        &destination,
    )
    .await
    .map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "messages-moved", true).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct DestroyIn {
    #[serde(rename = "emailIds")]
    email_ids: Vec<String>,
}

pub async fn destroy(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DestroyIn>,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    imap::destroy(&state.stalwart, &account, &body.email_ids)
        .await
        .map_err(bridge_err)?;
    announce_mailbox_change(&state, &auth, "messages-destroyed", true).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    anchor: Option<String>,
    #[serde(default)]
    query_state: Option<String>,
    #[serde(default = "default_sort")]
    sort: String,
    #[serde(default)]
    tz_offset_minutes: i32,
}

pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 100);
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({
            "emails": [], "has_more": false, "next_anchor": null,
            "query_state": null, "reset_required": false, "position": 0, "total": 0
        })));
    };
    if params.q.trim().is_empty() {
        return Ok(Json(json!({
            "emails": [], "has_more": false, "next_anchor": null,
            "query_state": null, "reset_required": false, "position": 0, "total": 0
        })));
    }
    let result = imap::search(
        &state.stalwart,
        &account,
        params.q.trim(),
        limit,
        params.anchor.as_deref(),
        params.query_state.as_deref(),
        &params.sort,
        params.tz_offset_minutes,
    )
    .await
    .map_err(|error| match error {
        imap::SearchError::Invalid(message) => ApiError::bad_request(message),
        imap::SearchError::Store(message) => bridge_err(message),
    })?;
    Ok(Json(result))
}

pub async fn attachment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(blob_id): Path<String>,
    Query(params): Query<AttachQuery>,
) -> Result<Response, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    let Some(item) = imap::attachment_blob(&state.stalwart, &account, &blob_id)
        .await
        .map_err(bridge_err)?
    else {
        return Err(ApiError::not_found("Attachment not found"));
    };

    // Stalwart returns the payload under a Name[type] key: text blobs come
    // back as `data:asText`, binary ones as base64 `data:asBase64` /
    // `data:asOctets`.
    let bytes: Vec<u8> = if let Some(t) = item.get("data:asText").and_then(Value::as_str) {
        t.as_bytes().to_vec()
    } else if let Some(b) = item.get("data:asBase64").and_then(Value::as_str) {
        B64.decode(b)
            .map_err(|_| bridge_err("attachment decode failure"))?
    } else if let Some(b) = item.get("data:asOctets").and_then(Value::as_str) {
        B64.decode(b)
            .map_err(|_| bridge_err("attachment decode failure"))?
    } else {
        return Err(bridge_err("attachment blob missing data"));
    };

    if bytes.len() > attachments::MAX_DOWNLOAD_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment_too_large",
            "This attachment is too large to download",
        ));
    }

    // Thread metadata carries name/type; the Blob/get payload usually omits
    // them, so the UI passes them as hints.
    let content_type = item
        .get("type")
        .and_then(Value::as_str)
        .or(params.content_type.as_deref())
        .unwrap_or("application/octet-stream")
        .to_string();
    let filename = item
        .get("name")
        .and_then(Value::as_str)
        .or(params.name.as_deref())
        .unwrap_or("attachment")
        .to_string();
    // RFC 5987 keeps non-ASCII filenames usable without breaking the header.
    let disposition = format!("attachment; filename*=UTF-8''{}", urlenc(&filename));

    let size = bytes.len();
    let mut resp = (StatusCode::OK, bytes).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).map_err(|_| bridge_err("bad disposition"))?,
    );
    resp.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).expect("length is numeric"),
    );
    Ok(resp)
}

#[derive(Deserialize)]
pub struct AttachQuery {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}
fn default_scope() -> String { "mailbox".to_string() }
fn default_sort() -> String { "received_desc".to_string() }
