//! WS3.3: read receipts. `read_receipts` logs receipts the user sent for
//! inbound mail; `receipt_requests` logs the inbound mail a receipt was
//! requested for on send. Both are mailbox-scoped and idempotent by `mail_id`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{entitlements, tenancy};
use crate::state::AppState;

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .map(|mailbox| mailbox.id)
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before using read receipts"))
}


#[derive(sqlx::FromRow)]
struct ReceiptRow {
    id: Uuid,
    mail_id: String,
    sender: String,
    email: String,
    subject: String,
    at: DateTime<Utc>,
}

fn receipt_to_json(row: &ReceiptRow) -> Value {
    json!({
        "id": row.id,
        "mailId": row.mail_id,
        "sender": row.sender,
        "email": row.email,
        "subject": row.subject,
        "at": row.at,
    })
}

#[derive(Deserialize)]
pub struct ReceiptIn {
    #[serde(rename = "mailId")]
    mail_id: String,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    subject: String,
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let rows: Vec<ReceiptRow> = sqlx::query_as(
        "SELECT id, mail_id, sender, email, subject, at FROM read_receipts
         WHERE mailbox_id = $1 ORDER BY at DESC LIMIT 500",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "receipts": rows.iter().map(receipt_to_json).collect::<Vec<_>>()
    })))
}

pub async fn record(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReceiptIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    entitlements::require_feature(&state, auth.user_id, "read_receipts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let mail_id = body.mail_id.trim();
    if mail_id.is_empty() {
        return Err(ApiError::bad_request("mailId is required"));
    }

    let row = sqlx::query_as::<_, ReceiptRow>(
        "INSERT INTO read_receipts (user_id, mailbox_id, mail_id, sender, email, subject)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (mailbox_id, mail_id) DO UPDATE SET
            sender = EXCLUDED.sender, email = EXCLUDED.email, subject = EXCLUDED.subject
         RETURNING id, mail_id, sender, email, subject, at",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(mail_id)
    .bind(body.sender.trim())
    .bind(body.email.trim())
    .bind(body.subject.trim())
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "receipt.record",
        json!({ "mail_id": mail_id }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(receipt_to_json(&row))))
}

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    mail_id: String,
    recipient: String,
    at: DateTime<Utc>,
}

fn request_to_json(row: &RequestRow) -> Value {
    json!({
        "id": row.id,
        "mailId": row.mail_id,
        "recipient": row.recipient,
        "at": row.at,
    })
}

#[derive(Deserialize)]
pub struct RequestIn {
    #[serde(rename = "mailId")]
    mail_id: String,
    #[serde(default)]
    recipient: String,
}

pub async fn list_requests(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let rows: Vec<RequestRow> = sqlx::query_as(
        "SELECT id, mail_id, recipient, at FROM receipt_requests
         WHERE mailbox_id = $1 ORDER BY at DESC LIMIT 500",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "requests": rows.iter().map(request_to_json).collect::<Vec<_>>()
    })))
}

pub async fn request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RequestIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    entitlements::require_feature(&state, auth.user_id, "read_receipts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let mail_id = body.mail_id.trim();
    if mail_id.is_empty() {
        return Err(ApiError::bad_request("mailId is required"));
    }

    let row = sqlx::query_as::<_, RequestRow>(
        "INSERT INTO receipt_requests (user_id, mailbox_id, mail_id, recipient)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (mailbox_id, mail_id) DO UPDATE SET recipient = EXCLUDED.recipient
         RETURNING id, mail_id, recipient, at",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(mail_id)
    .bind(body.recipient.trim())
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "receipt.request",
        json!({ "mail_id": mail_id }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(request_to_json(&row))))
}
