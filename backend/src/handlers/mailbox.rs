//! Mailbox API (WS2). Thin HTTP layer over `services::imap`, which talks to
//! Stalwart via an impersonating admin JMAP session keyed to the authenticated
//! user's Stalwart account id (resolved from `users.mail_account_id`, lazily
//! backfilled on first mail request if the row predates WS2).
//!
//! Contract (all child routes require the access-token cookie):
//!   GET    /api/mail/mailboxes
//!   GET    /api/mail/threads?mailbox=<id>&limit=<n>&anchor=<emailId>
//!   GET    /api/mail/thread/:thread_id
//!   POST   /api/mail/threads/state   { emailIds, read?, starred? }
//!   POST   /api/mail/threads/move    { emailIds, toMailbox, fromMailbox? }
//!   POST   /api/mail/threads/destroy { emailIds }
//!   GET    /api/mail/search?q=<text>&limit=<n>
//!   GET    /api/mail/attachment/:blob_id

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::handlers::attachments;
use crate::middleware::auth::AuthUser;
use crate::services::imap;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 25;

fn bridge_err(e: String) -> ApiError {
    // The mail store is an upstream dependency: surface as 502, log the real
    // reason server-side (ApiError hides internals on 5xx).
    ApiError::new(StatusCode::BAD_GATEWAY, "mail_store", e)
}

/// Resolve the user's Stalwart account id: the column wins, else a lazy
/// admin-side lookup for pre-WS2 registrations. `None` = no mailbox on the
/// primary domain (foreign address), render as empty rather than error.
async fn account_for(state: &AppState, user: &AuthUser) -> Result<Option<String>, ApiError> {
    let cached: (Option<String>,) =
        sqlx::query_as("SELECT mail_account_id FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(id) = cached.0 {
        if !id.is_empty() {
            return Ok(Some(id));
        }
    }

    if !state.mail.enabled() {
        return Ok(None);
    }
    let local = user.email.split('@').next().unwrap_or("");
    match state.mail.find_account(local).await {
        Ok(Some(id)) => {
            let _ = sqlx::query("UPDATE users SET mail_account_id = $1 WHERE id = $2")
                .bind(&id)
                .bind(user.user_id)
                .execute(&state.db)
                .await;
            tracing::info!(email = %user.email, account = %id, "backfilled mail_account_id");
            Ok(Some(id))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(bridge_err(e)),
    }
}

pub async fn mailboxes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({ "mailboxes": [] })));
    };
    let list = imap::mailboxes(&state.mail, &account)
        .await
        .map_err(bridge_err)?;
    Ok(Json(json!({ "mailboxes": list })))
}

#[derive(Deserialize)]
pub struct ThreadsQuery {
    mailbox: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    anchor: Option<String>,
}

pub async fn threads(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ThreadsQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 100);
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({ "emails": [], "has_more": false })));
    };
    if params.mailbox.is_empty() {
        return Ok(Json(json!({ "emails": [], "has_more": false })));
    }
    let result = imap::thread_previews(
        &state.mail,
        &account,
        &params.mailbox,
        limit,
        params.anchor.as_deref(),
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
    let result = imap::thread(&state.mail, &account, &thread_id)
        .await
        .map_err(bridge_err)?;
    Ok(Json(result))
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
        &state.mail,
        &account,
        &body.email_ids,
        body.read,
        body.starred,
    )
    .await
    .map_err(bridge_err)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct MoveIn {
    #[serde(rename = "emailIds")]
    email_ids: Vec<String>,
    #[serde(rename = "toMailbox")]
    to_mailbox: String,
    #[serde(rename = "fromMailbox", default)]
    from_mailbox: Option<String>,
}

pub async fn move_emails(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MoveIn>,
) -> Result<Json<Value>, ApiError> {
    if body.to_mailbox.is_empty() {
        return Err(ApiError::bad_request("toMailbox is required"));
    }
    let Some(account) = account_for(&state, &auth).await? else {
        return Err(ApiError::not_found("No mailbox for this account"));
    };
    imap::move_emails(
        &state.mail,
        &account,
        &body.email_ids,
        body.from_mailbox.as_deref(),
        &body.to_mailbox,
    )
    .await
    .map_err(bridge_err)?;
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
    imap::destroy(&state.mail, &account, &body.email_ids)
        .await
        .map_err(bridge_err)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 100);
    let Some(account) = account_for(&state, &auth).await? else {
        return Ok(Json(json!({ "emails": [], "has_more": false })));
    };
    if params.q.trim().is_empty() {
        return Ok(Json(json!({ "emails": [], "has_more": false })));
    }
    let result = imap::search(&state.mail, &account, params.q.trim(), limit)
        .await
        .map_err(bridge_err)?;
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
    let Some(item) = imap::attachment_blob(&state.mail, &account, &blob_id)
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
            .map_err(|_| bridge_err("attachment decode failure".into()))?
    } else if let Some(b) = item.get("data:asOctets").and_then(Value::as_str) {
        B64.decode(b)
            .map_err(|_| bridge_err("attachment decode failure".into()))?
    } else {
        return Err(bridge_err("attachment blob missing data".into()));
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
        HeaderValue::from_str(&disposition).map_err(|_| bridge_err("bad disposition".into()))?,
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
