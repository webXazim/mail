use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AdminUser;
use crate::state::AppState;

const MAX_EVENT_BODY_BYTES: usize = 64 * 1024;
const COMPLAINT_RESTRICT_THRESHOLD_24H: i64 = 3;
const HARD_BOUNCE_RESTRICT_THRESHOLD_24H: i64 = 25;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Deserialize)]
pub struct DeliveryEventIn {
    pub provider_event_id: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub event_type: String,
    pub organization_id: Uuid,
    pub mailbox_id: Uuid,
    pub send_request_id: Uuid,
    pub recipient: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub diagnostic: String,
}

fn default_provider() -> String {
    "provider".to_string()
}

fn normalized_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "delivered" | "delivery" => Some("delivered"),
        "soft_bounce" | "deferred" | "temporary_failure" => Some("soft_bounce"),
        "hard_bounce" | "bounce" | "failed" => Some("hard_bounce"),
        "complaint" | "spam_complaint" | "feedback_loop" => Some("complaint"),
        _ => None,
    }
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for index in (0..bytes.len()).step_by(2) {
        let high = (bytes[index] as char).to_digit(16)?;
        let low = (bytes[index + 1] as char).to_digit(16)?;
        out.push(((high << 4) | low) as u8);
    }
    Some(out)
}

fn verify_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> Result<(), ApiError> {
    let provided = headers
        .get("x-cs-delivery-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("Delivery event signature is required"))?;
    let provided = provided.strip_prefix("sha256=").unwrap_or(provided);
    let signature = decode_hex(provided)
        .ok_or_else(|| ApiError::unauthorized("Delivery event signature is invalid"))?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| ApiError::internal("Delivery event HMAC key is invalid"))?;
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| ApiError::unauthorized("Delivery event signature is invalid"))
}

async fn suppress_recipient_tx(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    mailbox_id: Uuid,
    recipient: &str,
    reason: &str,
    detail: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO recipient_suppressions(organization_id,mailbox_id,email,reason,source,detail)
         VALUES($1,$2,$3,$4,'provider_event',$5)
         ON CONFLICT (mailbox_id,lower(email::text)) WHERE mailbox_id IS NOT NULL DO UPDATE SET
           organization_id=EXCLUDED.organization_id,reason=EXCLUDED.reason,source=EXCLUDED.source,
           detail=EXCLUDED.detail,updated_at=now()",
    )
    .bind(organization_id)
    .bind(mailbox_id)
    .bind(recipient)
    .bind(reason)
    .bind(detail)
    .execute(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn maybe_restrict_business_tx(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
) -> Result<(), ApiError> {
    let (complaints, hard_bounces): (i64, i64) = sqlx::query_as(
        "SELECT
           count(*) FILTER (WHERE event_type='complaint'),
           count(*) FILTER (WHERE event_type='hard_bounce')
         FROM delivery_events
         WHERE organization_id=$1 AND received_at >= now()-interval '24 hours'",
    )
    .bind(organization_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if complaints >= COMPLAINT_RESTRICT_THRESHOLD_24H || hard_bounces >= HARD_BOUNCE_RESTRICT_THRESHOLD_24H {
        sqlx::query(
            "INSERT INTO organization_sending_controls(organization_id,state,reason)
             VALUES($1,'restricted',$2)
             ON CONFLICT(organization_id) DO UPDATE SET
               state=CASE WHEN organization_sending_controls.state='suspended' THEN 'suspended' ELSE 'restricted' END,
               reason=CASE WHEN organization_sending_controls.state='suspended' THEN organization_sending_controls.reason ELSE EXCLUDED.reason END,
               updated_at=now()",
        )
        .bind(organization_id)
        .bind(format!(
            "Automatic deliverability protection: {complaints} complaint(s), {hard_bounces} hard bounce(s) in 24 hours"
        ))
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(())
}

pub async fn process_event(state: &AppState, event: DeliveryEventIn) -> Result<Value, ApiError> {
    let event_type = normalized_type(&event.event_type)
        .ok_or_else(|| ApiError::bad_request("Unsupported delivery event type"))?;
    let event_id = event.provider_event_id.trim();
    if event_id.is_empty() || event_id.len() > 255 {
        return Err(ApiError::bad_request("provider_event_id is required and must be at most 255 characters"));
    }
    if event.provider.trim().is_empty() || event.provider.trim().len() > 128 {
        return Err(ApiError::bad_request("provider must be 1-128 characters"));
    }
    let recipient = crate::domain::suppression::normalize(&event.recipient);
    if !crate::services::automation::valid_email(&recipient) {
        return Err(ApiError::bad_request("Delivery event recipient is invalid"));
    }
    if event.status.len() > 255 || event.diagnostic.len() > 4096 {
        return Err(ApiError::bad_request("Delivery event status or diagnostic is too long"));
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let send: Option<(Uuid, Uuid, Value)> = sqlx::query_as(
        "SELECT r.mailbox_id,m.organization_id,r.recipients
         FROM mail_send_requests r
         JOIN mailboxes m ON m.id=r.mailbox_id
         WHERE r.id=$1 AND r.mailbox_id IS NOT NULL
         FOR SHARE",
    )
    .bind(event.send_request_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((send_mailbox, send_org, recipients)) = send else {
        return Err(ApiError::not_found("Send request was not found"));
    };
    if send_mailbox != event.mailbox_id || send_org != event.organization_id {
        return Err(ApiError::forbidden("Delivery event scope does not match the send request"));
    }
    let matched = recipients
        .as_array()
        .is_some_and(|items| items.iter().filter_map(Value::as_str).any(|value| value.eq_ignore_ascii_case(&recipient)));
    if !matched {
        return Err(ApiError::forbidden("Delivery event recipient does not match the send request"));
    }

    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO delivery_events(
           provider_event_id,provider,event_type,organization_id,mailbox_id,send_request_id,recipient,status,diagnostic
         ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT(provider,provider_event_id) DO NOTHING
         RETURNING provider_event_id",
    )
    .bind(event_id)
    .bind(event.provider.trim())
    .bind(event_type)
    .bind(event.organization_id)
    .bind(event.mailbox_id)
    .bind(event.send_request_id)
    .bind(&recipient)
    .bind(event.status.trim())
    .bind(event.diagnostic.trim())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if inserted.is_none() {
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok(json!({"accepted": true, "duplicate": true, "provider_event_id": event_id}));
    }

    match event_type {
        "hard_bounce" => {
            let detail = format!("{} {}", event.status.trim(), event.diagnostic.trim()).trim().to_string();
            suppress_recipient_tx(
                &mut tx,
                event.organization_id,
                event.mailbox_id,
                &recipient,
                "hard bounce",
                &detail,
            )
            .await?;
        }
        "complaint" => {
            suppress_recipient_tx(
                &mut tx,
                event.organization_id,
                event.mailbox_id,
                &recipient,
                "spam complaint",
                event.diagnostic.trim(),
            )
            .await?;
        }
        _ => {}
    }

    if matches!(event_type, "hard_bounce" | "complaint") {
        maybe_restrict_business_tx(&mut tx, event.organization_id).await?;
    }

    sqlx::query("UPDATE delivery_events SET processed_at=now() WHERE provider=$1 AND provider_event_id=$2")
        .bind(event.provider.trim())
        .bind(event_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(json!({
        "accepted": true,
        "duplicate": false,
        "provider_event_id": event_id,
        "event_type": event_type,
    }))
}

/// Machine-to-machine provider intake. Disabled unless
/// CS_MAIL_DELIVERY_EVENT_SECRET is configured.
pub async fn provider_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    if body.len() > MAX_EVENT_BODY_BYTES {
        return Err(ApiError::bad_request("Delivery event payload is too large"));
    }
    let secret = state
        .delivery_event_secret
        .as_deref()
        .ok_or_else(|| ApiError::not_found("Delivery event intake is disabled"))?;
    verify_signature(secret, &headers, &body)?;
    let event: DeliveryEventIn =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("Delivery event JSON is invalid"))?;
    let result = process_event(&state, event).await?;
    Ok(Json(result))
}

/// Platform operator path for replay/testing with the same correlation and
/// suppression rules as the signed provider endpoint.
pub async fn admin_event(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(event): Json<DeliveryEventIn>,
) -> Result<Json<Value>, ApiError> {
    let organization_id = event.organization_id;
    let mailbox_id = event.mailbox_id;
    let event_type = event.event_type.clone();
    let result = process_event(&state, event).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.deliverability.event",
        json!({"organization_id":organization_id,"mailbox_id":mailbox_id,"event_type":event_type}),
    )
    .await;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct DeliverabilityListQuery {
    #[serde(default)]
    pub organization_id: Option<Uuid>,
    #[serde(default)]
    pub mailbox_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn admin_events(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<DeliverabilityListQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows: Vec<(String, String, String, Uuid, Uuid, Uuid, String, String, String, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT provider_event_id,provider,event_type,organization_id,mailbox_id,send_request_id,
                recipient::text,status,diagnostic,received_at,processed_at
         FROM delivery_events
         WHERE ($1::uuid IS NULL OR organization_id=$1)
           AND ($2::uuid IS NULL OR mailbox_id=$2)
         ORDER BY received_at DESC
         LIMIT $3",
    )
    .bind(query.organization_id)
    .bind(query.mailbox_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"events": rows.into_iter().map(|row| json!({
        "provider_event_id": row.0,
        "provider": row.1,
        "event_type": row.2,
        "organization_id": row.3,
        "mailbox_id": row.4,
        "send_request_id": row.5,
        "recipient": row.6,
        "status": row.7,
        "diagnostic": row.8,
        "received_at": row.9,
        "processed_at": row.10,
    })).collect::<Vec<_>>() })))
}

pub async fn admin_tenant_suppressions(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<DeliverabilityListQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows: Vec<(Uuid, Uuid, Option<Uuid>, String, String, String, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id,organization_id,mailbox_id,email::text,reason,source,detail,created_at,updated_at
         FROM recipient_suppressions
         WHERE ($1::uuid IS NULL OR organization_id=$1)
           AND ($2::uuid IS NULL OR mailbox_id=$2)
         ORDER BY updated_at DESC
         LIMIT $3",
    )
    .bind(query.organization_id)
    .bind(query.mailbox_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"suppressions": rows.into_iter().map(|row| json!({
        "id": row.0,
        "organization_id": row.1,
        "mailbox_id": row.2,
        "email": row.3,
        "reason": row.4,
        "source": row.5,
        "detail": row.6,
        "created_at": row.7,
        "updated_at": row.8,
    })).collect::<Vec<_>>() })))
}

pub async fn admin_remove_tenant_suppression(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let removed: Option<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "DELETE FROM recipient_suppressions WHERE id=$1
         RETURNING organization_id,mailbox_id,email::text",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((organization_id, mailbox_id, email)) = removed else {
        return Err(ApiError::not_found("Tenant suppression was not found"));
    };
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.deliverability.unsuppress",
        json!({"id":id,"organization_id":organization_id,"mailbox_id":mailbox_id,"email":email}),
    )
    .await;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub struct SendingControlIn {
    pub organization_id: Uuid,
    pub state: String,
    #[serde(default)]
    pub hourly_limit_override: Option<i32>,
    #[serde(default)]
    pub daily_limit_override: Option<i32>,
    #[serde(default)]
    pub reason: String,
}

pub async fn admin_update_sending_control(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SendingControlIn>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(body.state.as_str(), "active" | "restricted" | "suspended") {
        return Err(ApiError::bad_request("Sending state must be active, restricted or suspended"));
    }
    if body.hourly_limit_override.is_some_and(|value| value < 0)
        || body.daily_limit_override.is_some_and(|value| value < 0)
    {
        return Err(ApiError::bad_request("Sending overrides cannot be negative"));
    }
    sqlx::query(
        "INSERT INTO organization_sending_controls(
           organization_id,state,hourly_limit_override,daily_limit_override,reason,updated_by,updated_at
         ) VALUES($1,$2,$3,$4,$5,$6,now())
         ON CONFLICT(organization_id) DO UPDATE SET
           state=EXCLUDED.state,hourly_limit_override=EXCLUDED.hourly_limit_override,
           daily_limit_override=EXCLUDED.daily_limit_override,reason=EXCLUDED.reason,
           updated_by=EXCLUDED.updated_by,updated_at=now()",
    )
    .bind(body.organization_id)
    .bind(&body.state)
    .bind(body.hourly_limit_override)
    .bind(body.daily_limit_override)
    .bind(body.reason.trim())
    .bind(admin.0.user_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.deliverability.sending_control",
        json!({"organization_id":body.organization_id,"state":body.state}),
    )
    .await;
    Ok(Json(json!({"ok":true})))
}

pub async fn admin_sending_controls(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(Uuid, String, String, Option<i32>, Option<i32>, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT c.organization_id,o.name,c.state,c.hourly_limit_override,c.daily_limit_override,c.reason,c.updated_at
             FROM organization_sending_controls c
             JOIN organizations o ON o.id=c.organization_id
             ORDER BY o.name",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"controls": rows.into_iter().map(|row| json!({
        "organization_id":row.0,"organization_name":row.1,"state":row.2,
        "hourly_limit_override":row.3,"daily_limit_override":row.4,
        "reason":row.5,"updated_at":row.6
    })).collect::<Vec<_>>() })))
}
