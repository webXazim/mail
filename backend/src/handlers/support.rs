use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::auth::valid_email;
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::services::{email, notifications};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct TicketIn {
    name: String,
    email: String,
    topic: String,
    #[serde(default)]
    subject: String,
    message: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TicketRow {
    id: Uuid,
    reference: String,
    user_id: Option<Uuid>,
    requester_name: String,
    requester_email: String,
    topic: String,
    subject: String,
    status: String,
    priority: String,
    assigned_to: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_reply_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MessageRow {
    id: Uuid,
    author_kind: String,
    body: String,
    created_at: DateTime<Utc>,
}

fn normalize_topic(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "billing" => Some("billing"),
        "security" => Some("security"),
        "technical issue" | "technical" => Some("technical"),
        "product feedback" | "feedback" => Some("feedback"),
        "press inquiry" | "press" => Some("press"),
        "other" => Some("other"),
        _ => None,
    }
}

fn clean(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

fn validate(body: &TicketIn) -> Result<(String, String, &'static str, String, String), ApiError> {
    let name = clean(&body.name, 120);
    let address = clean(&body.email, 320).to_ascii_lowercase();
    let topic = normalize_topic(&body.topic).ok_or_else(|| ApiError::bad_request("Choose a valid support topic"))?;
    let subject = clean(&body.subject, 200);
    let message = clean(&body.message, 10_000);
    if name.len() < 2 { return Err(ApiError::bad_request("Enter your name")); }
    if !valid_email(&address) { return Err(ApiError::bad_request("Enter a valid email address")); }
    if message.len() < 10 { return Err(ApiError::bad_request("Describe how we can help")); }
    let subject = if subject.is_empty() { format!("{} support request", topic) } else { subject };
    Ok((name, address, topic, subject, message))
}

async fn create_ticket(
    state: &AppState,
    user_id: Option<Uuid>,
    body: TicketIn,
) -> Result<TicketRow, ApiError> {
    let (name, address, topic, subject, message) = validate(&body)?;
    let priority = if topic == "security" { "high" } else { "normal" };
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: TicketRow = sqlx::query_as(
        "INSERT INTO support_tickets(user_id, requester_name, requester_email, topic, subject, priority)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING id, reference, user_id, requester_name, requester_email::text AS requester_email,
                   topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at",
    )
    .bind(user_id)
    .bind(&name)
    .bind(&address)
    .bind(topic)
    .bind(&subject)
    .bind(priority)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO support_messages(ticket_id, author_user_id, author_kind, body)
         VALUES ($1,$2,'customer',$3)",
    )
    .bind(row.id)
    .bind(user_id)
    .bind(&message)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(uid) = user_id {
        audit::record(state, Some(uid), "support.ticket.create", json!({"id": row.id, "reference": row.reference})).await;
        let _ = notifications::create(
            state, uid, "support", "Support request received",
            &format!("Ticket {} is open and our team can now review it.", row.reference),
            "/contact", Some(&format!("support:create:{}", row.id)),
        ).await;
    }
    Ok(row)
}

pub async fn create_public(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<TicketIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let ip = state.rate.client_ip(&headers, Some(remote));
    state.rate.check_burst(&format!("support-ip:{ip}"), 8, 3600).await?;
    let email_key: String = body.email.trim().to_ascii_lowercase().chars().take(320).collect();
    state.rate.check_burst(&format!("support-email:{email_key}"), 5, 3600).await?;
    let row = create_ticket(&state, None, body).await?;
    Ok((StatusCode::CREATED, Json(json!({"reference": row.reference, "status": row.status}))))
}

pub async fn create_authenticated(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(mut body): Json<TicketIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // The authenticated identity is the support ownership boundary. Do not let
    // the browser attach a ticket to another email address.
    body.email = auth.email.clone();
    state.rate.check_burst(&format!("support-user:{}", auth.user_id), 10, 3600).await?;
    let row = create_ticket(&state, Some(auth.user_id), body).await?;
    Ok((StatusCode::CREATED, Json(json!({"reference": row.reference, "status": row.status}))))
}

pub async fn my_tickets(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TicketRow> = sqlx::query_as(
        "SELECT id, reference, user_id, requester_name, requester_email::text AS requester_email,
                topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at
         FROM support_tickets WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"tickets": rows})))
}

#[derive(Debug, Deserialize)]
pub struct AdminListQuery { status: Option<String>, q: Option<String> }

pub async fn admin_list(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<Value>, ApiError> {
    let status = query.status.as_deref().filter(|v| matches!(*v, "open" | "pending" | "resolved" | "closed"));
    let q = query.q.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(|v| format!("%{}%", v.chars().take(200).collect::<String>()));
    let rows: Vec<TicketRow> = sqlx::query_as(
        "SELECT id, reference, user_id, requester_name, requester_email::text AS requester_email,
                topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at
         FROM support_tickets
         WHERE ($1::text IS NULL OR status = $1)
           AND ($2::text IS NULL OR reference ILIKE $2 OR requester_email::text ILIKE $2 OR requester_name ILIKE $2 OR subject ILIKE $2)
         ORDER BY CASE status WHEN 'open' THEN 0 WHEN 'pending' THEN 1 WHEN 'resolved' THEN 2 ELSE 3 END,
                  updated_at DESC LIMIT 500",
    )
    .bind(status)
    .bind(q)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"tickets": rows})))
}

pub async fn admin_get(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let ticket: TicketRow = sqlx::query_as(
        "SELECT id, reference, user_id, requester_name, requester_email::text AS requester_email,
                topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at
         FROM support_tickets WHERE id = $1",
    ).bind(id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
      .ok_or_else(|| ApiError::not_found("Support ticket not found"))?;
    let messages: Vec<MessageRow> = sqlx::query_as(
        "SELECT id, author_kind, body, created_at FROM support_messages
         WHERE ticket_id = $1 ORDER BY created_at ASC",
    ).bind(id).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ticket": ticket, "messages": messages})))
}

#[derive(Debug, Deserialize)]
pub struct TicketPatch { status: Option<String>, priority: Option<String> }

pub async fn admin_update(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TicketPatch>,
) -> Result<Json<Value>, ApiError> {
    let status = body.status.as_deref().filter(|v| matches!(*v, "open" | "pending" | "resolved" | "closed"));
    if body.status.is_some() && status.is_none() { return Err(ApiError::bad_request("Invalid support status")); }
    let priority = body.priority.as_deref().filter(|v| matches!(*v, "normal" | "high" | "urgent"));
    if body.priority.is_some() && priority.is_none() { return Err(ApiError::bad_request("Invalid support priority")); }
    let row: TicketRow = sqlx::query_as(
        "UPDATE support_tickets SET
           status = COALESCE($2, status), priority = COALESCE($3, priority),
           assigned_to = COALESCE(assigned_to, $4),
           resolved_at = CASE WHEN COALESCE($2,status) IN ('resolved','closed') THEN COALESCE(resolved_at, now()) ELSE NULL END,
           updated_at = now()
         WHERE id = $1
         RETURNING id, reference, user_id, requester_name, requester_email::text AS requester_email,
                   topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at",
    )
    .bind(id).bind(status).bind(priority).bind(admin.0.user_id)
    .fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Support ticket not found"))?;
    audit::record(&state, Some(admin.0.user_id), "admin.support.update", json!({"id":id,"status":row.status,"priority":row.priority})).await;
    if let Some(uid) = row.user_id {
        let _ = notifications::create(&state, uid, "support", "Support ticket updated", &format!("{} is now {}.", row.reference, row.status), "/contact", Some(&format!("support:update:{}:{}", id, row.updated_at))).await;
    }
    Ok(Json(json!({"ticket": row})))
}

#[derive(Debug, Deserialize)]
pub struct ReplyIn { message: String, #[serde(default)] status: Option<String> }

pub async fn admin_reply(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplyIn>,
) -> Result<Json<Value>, ApiError> {
    let message = clean(&body.message, 10_000);
    if message.len() < 2 { return Err(ApiError::bad_request("Reply cannot be empty")); }
    let next_status = body.status.as_deref().unwrap_or("pending");
    if !matches!(next_status, "open" | "pending" | "resolved" | "closed") { return Err(ApiError::bad_request("Invalid support status")); }
    let ticket: TicketRow = sqlx::query_as(
        "SELECT id, reference, user_id, requester_name, requester_email::text AS requester_email,
                topic, subject, status, priority, assigned_to, created_at, updated_at, last_reply_at
         FROM support_tickets WHERE id = $1",
    ).bind(id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
      .ok_or_else(|| ApiError::not_found("Support ticket not found"))?;

    // Submit the email before recording the reply. This prevents the support UI
    // from claiming a customer-visible response was sent when SMTP rejected it.
    email::send_support_reply(&state, &ticket.requester_email, &ticket.requester_name, &ticket.reference, &ticket.subject, &message).await?;

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("INSERT INTO support_messages(ticket_id, author_user_id, author_kind, body) VALUES ($1,$2,'agent',$3)")
        .bind(id).bind(admin.0.user_id).bind(&message).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE support_tickets SET status=$2, assigned_to=$3, last_reply_at=now(), updated_at=now(),
           resolved_at=CASE WHEN $2 IN ('resolved','closed') THEN COALESCE(resolved_at,now()) ELSE NULL END
         WHERE id=$1",
    ).bind(id).bind(next_status).bind(admin.0.user_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(admin.0.user_id), "admin.support.reply", json!({"id":id,"reference":ticket.reference,"status":next_status})).await;
    if let Some(uid) = ticket.user_id {
        let _ = notifications::create(&state, uid, "support", "Support replied", &format!("Our team replied to {}.", ticket.reference), "/contact", Some(&format!("support:reply:{}:{}", id, Utc::now().timestamp_millis()))).await;
    }
    Ok(Json(json!({"ok":true,"status":next_status})))
}
