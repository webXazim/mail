//! Compose / send / drafts API (WS2.3). Sending builds an RFC 5322 message
//! (see `services::mime`), hands it to the SMTP relay on the internal network
//! (`services::smtp`), then files the sender's self-copy into Sent via the
//! impersonated JMAP session (`services::imap`). Drafts are our Postgres rows
//! — the Stalwart store has object creation disabled in this deployment.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::domain::quota;
use crate::error::ApiError;
use crate::handlers::attachments;
use crate::middleware::auth::AuthUser;
use crate::services::{imap, mime, send_limit, smtp, suppression};
use crate::state::AppState;

fn bridge_err(e: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "mail_store", e)
}

/// Resolve the user's Stalwart account id (same lazy rule as `handlers::mailbox`).
async fn account_for(state: &AppState, user_id: Uuid, email: &str) -> Result<String, ApiError> {
    let cached: (Option<String>,) =
        sqlx::query_as("SELECT mail_account_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(id) = cached.0 {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    if !state.mail.enabled() {
        return Err(ApiError::forbidden(
            "No mailbox provisioned for this account",
        ));
    }
    let local = email.split('@').next().unwrap_or("");
    match state.mail.find_account(local).await {
        Ok(Some(id)) => {
            let _ = sqlx::query("UPDATE users SET mail_account_id = $1 WHERE id = $2")
                .bind(&id)
                .bind(user_id)
                .execute(&state.db)
                .await;
            Ok(id)
        }
        Ok(None) => Err(ApiError::forbidden(
            "No mailbox provisioned for this account",
        )),
        Err(e) => Err(bridge_err(e)),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipientIn {
    #[serde(default)]
    name: String,
    email: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentIn {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    data_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ComposeIn {
    #[serde(default)]
    to: Vec<RecipientIn>,
    #[serde(default)]
    cc: Vec<RecipientIn>,
    #[serde(default)]
    bcc: Vec<RecipientIn>,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body_text: String,
    #[serde(default)]
    body_html: Option<String>,
    #[serde(default)]
    attachments: Vec<AttachmentIn>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Vec<String>,
}

#[derive(Deserialize)]
pub struct SendIn {
    #[serde(flatten)]
    compose: ComposeIn,
    /// Destroy this draft on successful delivery.
    #[serde(default)]
    draft_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DraftPatch {
    #[serde(default)]
    to: Option<Vec<RecipientIn>>,
    #[serde(default)]
    cc: Option<Vec<RecipientIn>>,
    #[serde(default)]
    bcc: Option<Vec<RecipientIn>>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    body_text: Option<String>,
    #[serde(default)]
    body_html: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<AttachmentIn>>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Option<Vec<String>>,
}

/// Validate a recipient list into MIME addresses, rejecting anything that
/// could inject SMTP/header syntax.
fn to_addresses(list: Vec<RecipientIn>, label: &str) -> Result<Vec<mime::Address>, ApiError> {
    let mut out = Vec::with_capacity(list.len());
    for r in list {
        let email = r.email.trim().to_string();
        let valid = !email.is_empty()
            && email.contains('@')
            && email.split('@').all(|p| !p.is_empty())
            && email
                .chars()
                .all(|c| !c.is_control() && c != ' ' && c != '\r' && c != '\n');
        if !valid {
            return Err(ApiError::bad_request(format!(
                "Invalid {label} recipient {email:?}"
            )));
        }
        let name = r.name.trim().to_string();
        out.push(mime::Address {
            name: if name.is_empty() { None } else { Some(name) },
            email,
        });
    }
    Ok(out)
}

fn to_attachments(
    list: Vec<AttachmentIn>,
    plan: &quota::PlanLimits,
) -> Result<Vec<mime::Attachment>, ApiError> {
    let mut out = Vec::with_capacity(list.len());
    let mut total = 0usize;
    for a in list {
        if a.data_base64.is_empty() {
            continue;
        }
        let bytes = B64
            .decode(a.data_base64.as_bytes())
            .map_err(|_| ApiError::bad_request("Attachment data is not valid base64"))?;
        let valid = attachments::validate_upload(
            &a.filename,
            &a.content_type,
            &bytes,
            plan.max_attachment_bytes,
        )?;
        total += bytes.len();
        if total > plan.max_total_attachment_bytes {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "attachments_too_large",
                format!(
                    "Attachments for one message may total at most {} MiB on your plan",
                    plan.max_total_attachment_bytes / (1024 * 1024)
                ),
            ));
        }
        out.push(mime::Attachment {
            filename: valid.filename,
            content_type: valid.content_type,
            bytes,
        });
    }
    Ok(out)
}

/// The user's billing plan entitlements, defaulting to Solo for unknown/unset
/// rows. Loaded from the admin-editable `plans` table (WS4).
async fn plan_for(state: &AppState, user_id: Uuid) -> Result<quota::PlanLimits, ApiError> {
    crate::services::billing::for_user(state, user_id).await
}

/// Strip anything that could break out of a header value line.
fn header_safe(s: Option<String>) -> Option<String> {
    s.map(|v| v.replace(['\r', '\n'], " ").trim().to_string())
        .filter(|v| !v.is_empty())
}

async fn display_name(state: &AppState, user_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT display_name FROM users WHERE id = $1 AND display_name <> ''",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

/// Outcome of a successful delivery, shared by the API and the scheduler worker.
pub struct SentOutcome {
    pub message_id: String,
    pub sent_id: Option<String>,
    pub recipients: Vec<String>,
}

/// Build the MIME message, submit it over SMTP, then file the sender a copy in
/// Sent. Shared by `POST /api/send` and the scheduled-send worker (`main.rs`).
pub async fn deliver(
    state: &AppState,
    user_id: Uuid,
    from_email: &str,
    compose: ComposeIn,
    draft_id: Option<Uuid>,
) -> Result<SentOutcome, ApiError> {
    let account = account_for(state, user_id, from_email).await?;

    // WS7.2: drop suppressed recipients before anything is built, so a hard
    // bounce or unsubscribe is honoured even mid-thread.
    let mut compose = compose;
    let requested: Vec<String> = compose
        .to
        .iter()
        .chain(&compose.cc)
        .chain(&compose.bcc)
        .map(|r| r.email.clone())
        .collect();
    let blocked = suppression::suppressed(state, &requested).await?;
    let had_to = !compose.to.is_empty();
    if !blocked.is_empty() {
        state
            .metrics
            .record_suppressed_dropped(blocked.len() as u64);
        let keep =
            |r: &RecipientIn| !blocked.contains(&crate::domain::suppression::normalize(&r.email));
        compose.to.retain(keep);
        compose.cc.retain(keep);
        compose.bcc.retain(keep);
    }

    let to = to_addresses(compose.to, "to")?;
    let cc = to_addresses(compose.cc, "cc")?;
    let bcc = to_addresses(compose.bcc, "bcc")?;
    if to.is_empty() {
        if had_to {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "recipients_suppressed",
                "Every intended recipient is on the suppression list",
            ));
        }
        return Err(ApiError::bad_request(
            "At least one To recipient is required",
        ));
    }
    let plan = plan_for(state, user_id).await?;

    // WS7.3: per-message recipient cap (plan-scaled, hard ceiling).
    let external = to.len() + cc.len() + bcc.len();
    let per_message = crate::domain::send_limit::per_message(plan.max_recipients);
    if external > per_message {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_recipients",
            format!(
                "This message has {external} recipients; your plan allows at most {per_message}"
            ),
        ));
    }

    let attachments = to_attachments(compose.attachments, &plan)?;

    let from = mime::Address {
        name: display_name(state, user_id).await,
        email: from_email.to_string(),
    };
    let domain = from_email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .unwrap_or_default();
    let message_id_local = Uuid::new_v4().as_simple().to_string();

    // One-click unsubscribe is only meaningful with a single external
    // recipient, since one header cannot carry a per-recipient token.
    let list_unsubscribe = {
        let mut external = to.iter().chain(cc.iter()).chain(bcc.iter());
        let first = external.next();
        if external.next().is_none() {
            first.map(|a| suppression::unsubscribe_url(state, &a.email))
        } else {
            None
        }
    };

    let outgoing = mime::Outgoing {
        from,
        to,
        cc,
        subject: compose.subject,
        body_text: compose.body_text,
        body_html: compose.body_html,
        attachments,
        in_reply_to: header_safe(compose.in_reply_to),
        references: compose
            .references
            .into_iter()
            .map(|r| r.replace(['\r', '\n'], " ").trim().to_string())
            .filter(|r| !r.is_empty())
            .collect(),
        list_unsubscribe,
        message_id_local,
        domain: domain.clone(),
    };
    let message_id = outgoing.message_id();
    let bytes = outgoing
        .build()
        .map_err(|e| ApiError::internal(format!("MIME build failed: {e}")))?;

    // Envelope = direct + hidden-copy recipients, plus the sender themself so
    // a self-delivered copy lands for the Sent folder. Bcc is envelope-only
    // (RFC 5322), so it rides here and never in the message headers.
    let mut envelope: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_env = |addrs: &[mime::Address]| {
        for addr in addrs {
            let email = addr.email.to_lowercase();
            if seen.insert(email.clone()) {
                envelope.push(email);
            }
        }
    };
    push_env(&outgoing.to);
    push_env(&outgoing.cc);
    push_env(&bcc);
    let sender_lower = from_email.to_lowercase();
    if seen.insert(sender_lower.clone()) {
        envelope.push(sender_lower);
    }

    // WS5.3: charge the daily budgets before touching SMTP. `envelope` already
    // dedupes recipients and includes the sender's self-copy, so it is the
    // honest cost of this message.
    send_limit::enforce(
        state,
        user_id,
        &domain,
        plan.daily_send_limit,
        envelope.len() as i64,
    )
    .await?;

    // WS6.3: record the delivery outcome for Prometheus.
    if let Err(e) = smtp::send(&state.smtp, from_email, &envelope, &bytes).await {
        state.metrics.record_send_failed();
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, "smtp_submission", e));
    }
    state.metrics.record_send_ok();

    // File the self-copy into Sent (best effort, poll briefly for delivery).
    let mut sent_id: Option<String> = None;
    if let Ok(Some(sent_mailbox)) = imap::sent_mailbox_id(&state.mail, &account).await {
        for _ in 0..8 {
            match imap::find_by_message_id(&state.mail, &account, &message_id, 15).await {
                Ok(Some(id)) => {
                    if let Err(e) =
                        imap::move_to_sent(&state.mail, &account, &id, &sent_mailbox).await
                    {
                        tracing::warn!(%message_id, "sent-copy move failed: {e}");
                    } else {
                        sent_id = Some(id);
                    }
                    break;
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(e) => {
                    tracing::warn!(%message_id, "sent-copy lookup failed: {e}");
                    break;
                }
            }
        }
    }

    if let Some(draft_id) = draft_id {
        delete_draft(state, user_id, draft_id).await?;
    }

    Ok(SentOutcome {
        message_id,
        sent_id,
        recipients: envelope,
    })
}

/// `POST /api/send` — deliver one message, then file the sender a copy in Sent.
pub async fn send(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SendIn>,
) -> Result<Json<Value>, ApiError> {
    let outcome = deliver(
        &state,
        auth.user_id,
        &auth.email,
        body.compose,
        body.draft_id,
    )
    .await?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.send",
        json!({ "message_id": &outcome.message_id, "recipients": &outcome.recipients }),
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "message_id": outcome.message_id,
        "sent_id": outcome.sent_id,
        "stored": outcome.sent_id.is_some(),
    })))
}

#[derive(sqlx::FromRow)]
struct DraftRow {
    id: Uuid,
    to_list: Value,
    cc_list: Value,
    bcc_list: Value,
    subject: String,
    body_text: String,
    body_html: String,
    attachments: Value,
    in_reply_to: String,
    references_list: Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn draft_to_json(r: &DraftRow) -> Value {
    json!({
        "id": r.id,
        "to": r.to_list,
        "cc": r.cc_list,
        "bcc": r.bcc_list,
        "subject": r.subject,
        "body_text": r.body_text,
        "body_html": r.body_html,
        "attachments": r.attachments,
        "in_reply_to": r.in_reply_to,
        "references": r.references_list,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
    })
}

fn snippet(body_text: &str, subject: &str) -> String {
    let text = body_text.trim();
    if !text.is_empty() {
        return text.chars().take(160).collect();
    }
    subject.to_string()
}

const DRAFT_COLUMNS: &str =
    "id, to_list, cc_list, bcc_list, subject, body_text, body_html, attachments, in_reply_to, references_list, created_at, updated_at";

pub async fn drafts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<DraftRow> = sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mail_drafts WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 200"
    ))
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "to": r.to_list,
                "subject": r.subject,
                "snippet": snippet(&r.body_text, &r.subject),
                "has_attachments": !r.attachments.as_array().map(|a| a.is_empty()).unwrap_or(true),
                "updated_at": r.updated_at,
            })
        })
        .collect();
    Ok(Json(json!({ "drafts": items })))
}

pub async fn create_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(compose): Json<ComposeIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    to_addresses(compose.to.clone(), "to")?;
    to_addresses(compose.cc.clone(), "cc")?;
    to_addresses(compose.bcc.clone(), "bcc")?;
    let plan = plan_for(&state, auth.user_id).await?;
    to_attachments(compose.attachments.clone(), &plan)?;

    let row = sqlx::query_as::<_, DraftRow>(&format!(
        "INSERT INTO mail_drafts
           (user_id, to_list, cc_list, bcc_list, subject, body_text, body_html,
            attachments, in_reply_to, references_list)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(auth.user_id)
    .bind(json!(compose.to))
    .bind(json!(compose.cc))
    .bind(json!(compose.bcc))
    .bind(compose.subject)
    .bind(compose.body_text)
    .bind(compose.body_html.unwrap_or_default())
    .bind(json!(&compose.attachments))
    .bind(header_safe(compose.in_reply_to).unwrap_or_default())
    .bind(json!(&compose.references))
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.draft.create",
        json!({ "id": row.id }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(draft_to_json(&row))))
}

pub async fn get_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<DraftRow> = sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mail_drafts WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match row {
        Some(r) => Ok(Json(draft_to_json(&r))),
        None => Err(ApiError::not_found("Draft not found")),
    }
}

pub async fn update_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(patch): Json<DraftPatch>,
) -> Result<Json<Value>, ApiError> {
    let to = match &patch.to {
        Some(l) => {
            to_addresses(l.clone(), "to")?;
            Some(json!(l))
        }
        None => None,
    };
    let cc = match &patch.cc {
        Some(l) => {
            to_addresses(l.clone(), "cc")?;
            Some(json!(l))
        }
        None => None,
    };
    let bcc = match &patch.bcc {
        Some(l) => {
            to_addresses(l.clone(), "bcc")?;
            Some(json!(l))
        }
        None => None,
    };
    let attachments = match &patch.attachments {
        Some(l) => {
            let plan = plan_for(&state, auth.user_id).await?;
            to_attachments(l.clone(), &plan)?;
            Some(json!(l))
        }
        None => None,
    };

    let row: Option<DraftRow> = sqlx::query_as(&format!(
        "UPDATE mail_drafts SET
            to_list     = COALESCE($2, to_list),
            cc_list     = COALESCE($3, cc_list),
            bcc_list    = COALESCE($4, bcc_list),
            subject     = COALESCE($5, subject),
            body_text   = COALESCE($6, body_text),
            body_html   = COALESCE($7, body_html),
            attachments = COALESCE($8, attachments),
            in_reply_to = COALESCE($9, in_reply_to),
            references_list = COALESCE($10, references_list),
            updated_at  = now()
         WHERE id = $1 AND user_id = $11
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(id)
    .bind(to)
    .bind(cc)
    .bind(bcc)
    .bind(patch.subject)
    .bind(patch.body_text)
    .bind(patch.body_html)
    .bind(attachments)
    .bind(
        patch
            .in_reply_to
            .map(|s| s.replace(['\r', '\n'], " ").trim().to_string()),
    )
    .bind(patch.references.map(|v| json!(v)))
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match row {
        Some(r) => {
            audit::record(
                &state,
                Some(auth.user_id),
                "mail.draft.update",
                json!({ "id": id }),
            )
            .await;
            Ok(Json(draft_to_json(&r)))
        }
        None => Err(ApiError::not_found("Draft not found")),
    }
}

pub async fn delete_draft_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    delete_draft(&state, auth.user_id, id).await?;
    audit::record(
        &state,
        Some(auth.user_id),
        "mail.draft.delete",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_draft(state: &AppState, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let deleted = sqlx::query("DELETE FROM mail_drafts WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found("Draft not found"));
    }
    Ok(())
}
