//! Production compose / drafts / send API.
//!
//! Upgrade 09 makes the application authoritative for sender identity and
//! idempotency while Stalwart remains authoritative for delivered mail. Every
//! submission is represented by a durable ledger row with a deterministic
//! Message-ID before SMTP is touched. Retrying the same idempotency key can
//! therefore reconcile the original delivery instead of blindly sending twice.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::{attachments, identities};
use crate::middleware::auth::AuthUser;
use crate::services::stalwart::StalwartError;
use crate::services::{entitlements, imap, mime, send_limit, suppression, tenancy};
use crate::state::AppState;
use crate::ws::emit_event;

fn bridge_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "mail_store", e.to_string())
}

async fn account_for(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<String, ApiError> {
    let mailbox = tenancy::active_mailbox(&state.db, user_id, None, Some(mailbox_id))
        .await?
        .ok_or_else(|| ApiError::forbidden("The selected business mailbox is not assigned to this account"))?;
    if mailbox.status == "suspended" {
        return Err(ApiError::forbidden("This business mailbox is suspended"));
    }
    if mailbox.status != "active" {
        return Err(ApiError::forbidden("This business mailbox is not active"));
    }
    if let Some(id) = mailbox.provider_account_id.as_deref().filter(|value| !value.is_empty()) {
        return Ok(id.to_string());
    }
    if !state.stalwart.enabled() {
        return Err(ApiError::forbidden("No mailbox provisioned for this account"));
    }
    match state.stalwart.find_account_by_email(&mailbox.address).await {
        Ok(Some(id)) => {
            sqlx::query("UPDATE mailboxes SET provider_account_id=$1,sync_status='ready',sync_error='',updated_at=now() WHERE id=$2")
                .bind(&id)
                .bind(mailbox.id)
                .execute(&state.db)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            Ok(id)
        }
        Ok(None) => Err(ApiError::forbidden("No mailbox provisioned for this account")),
        Err(StalwartError::UnsupportedDomain(_)) => Err(ApiError::forbidden("No mailbox provisioned for this account")),
        Err(e) => Err(bridge_err(e)),
    }
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
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before using mail"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipientIn {
    #[serde(default)]
    name: String,
    email: String,
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
    pub attachments: Vec<attachments::AttachmentRefIn>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Vec<String>,
    #[serde(default)]
    pub identity_id: Option<Uuid>,
    /// Stable browser-generated key for idempotent draft creation/autosave.
    #[serde(default)]
    pub client_key: Option<Uuid>,
    /// Stable logical send key persisted with server drafts for response-loss recovery.
    #[serde(default)]
    pub send_key: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct SendIn {
    #[serde(flatten)]
    compose: ComposeIn,
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
    attachments: Option<Vec<attachments::AttachmentRefIn>>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Option<Vec<String>>,
    #[serde(default)]
    identity_id: Option<Uuid>,
    #[serde(default)]
    send_key: Option<Uuid>,
}

fn to_addresses(list: Vec<RecipientIn>, label: &str) -> Result<Vec<mime::Address>, ApiError> {
    let mut out = Vec::with_capacity(list.len());
    for r in list {
        let email = r.email.trim().to_lowercase();
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
        let name = r.name.replace(['\r', '\n'], " ").trim().to_string();
        out.push(mime::Address {
            name: if name.is_empty() { None } else { Some(name) },
            email,
        });
    }
    Ok(out)
}

fn header_safe(s: Option<String>) -> Option<String> {
    s.map(|v| v.replace(['\r', '\n'], " ").trim().to_string())
        .filter(|v| !v.is_empty())
}

fn request_hash(compose: &ComposeIn, draft_id: Option<Uuid>) -> Result<String, ApiError> {
    let mut canonical = compose.clone();
    // Draft/autosave metadata is not message content. Excluding these keys lets
    // another device safely reconcile the same logical submission.
    canonical.client_key = None;
    canonical.send_key = None;
    let payload = serde_json::to_vec(&(canonical, draft_id))
        .map_err(|e| ApiError::internal(format!("Could not fingerprint compose payload: {e}")))?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

fn normalize_idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let value = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim();
    if value.len() < 8 || value.len() > 128 {
        return Err(ApiError::bad_request(
            "Idempotency-Key header is required and must be 8-128 characters",
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(ApiError::bad_request("Idempotency-Key contains invalid characters"));
    }
    Ok(value.to_string())
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SendRequestRow {
    id: Uuid,
    idempotency_key: String,
    request_hash: String,
    status: String,
    message_id: String,
    sent_id: String,
    recipients: Value,
    last_error: String,
}

async fn prepare_send_request(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    key: &str,
    hash: &str,
    identity_id: Uuid,
    draft_id: Option<Uuid>,
    domain: &str,
) -> Result<SendRequestRow, ApiError> {
    let local = Uuid::new_v4().as_simple().to_string();
    let message_id = format!("<{local}@{domain}>");
    sqlx::query(
        "INSERT INTO mail_send_requests
            (user_id, mailbox_id, idempotency_key, request_hash, identity_id, draft_id, message_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (mailbox_id, idempotency_key) DO NOTHING",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .bind(key)
    .bind(hash)
    .bind(identity_id)
    .bind(draft_id)
    .bind(message_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let row: SendRequestRow = sqlx::query_as(
        "SELECT id, idempotency_key, request_hash, status, message_id, sent_id,
                recipients, last_error
         FROM mail_send_requests WHERE mailbox_id = $1 AND idempotency_key = $2",
    )
    .bind(mailbox_id)
    .bind(key)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if row.request_hash != hash {
        return Err(ApiError::conflict(
            "This idempotency key was already used for a different message",
        ));
    }
    Ok(row)
}

fn outcome_from_row(row: &SendRequestRow) -> SentOutcome {
    let recipients = serde_json::from_value::<Vec<String>>(row.recipients.clone()).unwrap_or_default();
    SentOutcome {
        request_id: row.id,
        idempotency_key: row.idempotency_key.clone(),
        message_id: row.message_id.clone(),
        sent_id: (!row.sent_id.is_empty()).then(|| row.sent_id.clone()),
        recipients,
        deduplicated: true,
    }
}

async fn mark_sent(
    state: &AppState,
    request_id: Uuid,
    recipients: &[String],
    sent_id: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE mail_send_requests
         SET status = 'sent', recipients = $2, sent_id = COALESCE($3, sent_id),
             last_error = '', sent_at = COALESCE(sent_at, now()), updated_at = now()
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(json!(recipients))
    .bind(sent_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn reconcile_sent_copy(
    state: &AppState,
    account: &str,
    request_id: Uuid,
    message_id: &str,
) -> Result<Option<String>, ApiError> {
    let Some(id) = imap::find_by_message_id(&state.stalwart, account, message_id, 25)
        .await
        .map_err(bridge_err)?
    else {
        return Ok(None);
    };
    if let Some(sent_mailbox) = imap::sent_mailbox_id(&state.stalwart, account)
        .await
        .map_err(bridge_err)?
    {
        if let Err(error) = imap::move_to_sent(&state.stalwart, account, &id, &sent_mailbox).await {
            tracing::warn!(%request_id, %message_id, "sent-copy move failed during reconciliation: {error}");
            return Ok(None);
        }
    }
    sqlx::query(
        "UPDATE mail_send_requests SET status = 'sent', sent_id = $2,
             last_error = '', sent_at = COALESCE(sent_at, now()), updated_at = now()
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Some(id))
}

pub struct SentOutcome {
    pub request_id: Uuid,
    pub idempotency_key: String,
    pub message_id: String,
    pub sent_id: Option<String>,
    pub recipients: Vec<String>,
    pub deduplicated: bool,
}

/// Build, authorize and submit one message. `idempotency_key` is durable across
/// browser retries. The selected sender identity is always resolved server-side
/// and can never be supplied as an arbitrary From address by the browser.
pub async fn deliver(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    compose: ComposeIn,
    draft_id: Option<Uuid>,
    idempotency_key: &str,
) -> Result<SentOutcome, ApiError> {
    // The mailbox, not the login's mutable active-business preference, owns
    // this send. This keeps queued/retried sends pinned to the original tenant
    // even if the user switches businesses before the job executes.
    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT organization_id FROM mailboxes
         WHERE id=$1 AND deleted_at IS NULL AND status='active'",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::forbidden("The selected business mailbox is not active"))?;
    let business_entitlements = entitlements::for_organization(state, organization_id).await?;
    if !matches!(business_entitlements.subscription_status.as_str(), "active" | "trial")
        || !business_entitlements.plan.allows("mail")
    {
        return Err(ApiError::forbidden("Mail sending is not enabled for this business subscription"));
    }
    let organization_daily_send_limit = business_entitlements.organization_daily_send_limit;
    let plan = business_entitlements.plan.clone();
    let identity = identities::resolve(state, user_id, mailbox_id, compose.identity_id).await?;

    let domain = identity
        .email
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_lowercase())
        .ok_or_else(|| ApiError::internal("Sender identity has an invalid domain"))?;
    let hash = request_hash(&compose, draft_id)?;
    let request = prepare_send_request(
        state,
        user_id,
        mailbox_id,
        idempotency_key,
        &hash,
        identity.id,
        draft_id,
        &domain,
    )
    .await?;

    if request.status == "sent" {
        return Ok(outcome_from_row(&request));
    }
    if request.status == "failed" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "send_failed",
            if request.last_error.is_empty() {
                "This send attempt failed. Edit the message or start a new send attempt.".to_string()
            } else {
                request.last_error.clone()
            },
        ));
    }

    // A saved draft carries the logical send key. This turns multi-device
    // last-write-wins editing into a safe send boundary: a stale device cannot
    // submit an older draft version under a fresh idempotency key. Existing
    // uncertain/submitting requests are still allowed to reconcile below.
    if request.status == "prepared" {
        if let Some(draft_id) = draft_id {
            let draft_key: Option<Uuid> = sqlx::query_scalar(
                "SELECT send_key FROM mail_drafts WHERE id = $1 AND mailbox_id = $2",
            )
            .bind(draft_id)
            .bind(mailbox_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .flatten();
            let Some(draft_key) = draft_key else {
                return Err(ApiError::conflict(
                    "This draft no longer exists. Refresh Drafts before sending again.",
                ));
            };
            if draft_key.to_string() != idempotency_key {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "draft_changed",
                    "This draft changed in another session. Reopen the latest draft before sending.",
                ));
            }
        }
    }

    let account = account_for(state, user_id, mailbox_id).await?;

    if request.status == "submitting" || request.status == "uncertain" {
        for _ in 0..4 {
            if let Some(sent_id) = reconcile_sent_copy(
                state,
                &account,
                request.id,
                &request.message_id,
            )
            .await?
            {
                let mut outcome = outcome_from_row(&request);
                outcome.sent_id = Some(sent_id);
                return Ok(outcome);
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "send_uncertain",
            "The original send is still being reconciled. Do not resend yet; CS Mail will update the Sent copy automatically.",
        ));
    }
    let mut compose = compose;
    let requested: Vec<String> = compose
        .to
        .iter()
        .chain(&compose.cc)
        .chain(&compose.bcc)
        .map(|recipient| recipient.email.clone())
        .collect();
    let blocked =
        suppression::suppressed_for_mailbox(state, organization_id, mailbox_id, &requested).await?;
    let had_to = !compose.to.is_empty();
    if !blocked.is_empty() {
        state.metrics.record_suppressed_dropped(blocked.len() as u64);
        let keep = |recipient: &RecipientIn| {
            !blocked.contains(&crate::domain::suppression::normalize(&recipient.email))
        };
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
        return Err(ApiError::bad_request("At least one To recipient is required"));
    }

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

    let (attachments, attachment_ids) =
        attachments::load_for_message(state, mailbox_id, &compose.attachments, &plan).await?;

    let message_id_local = request
        .message_id
        .trim_start_matches('<')
        .split('@')
        .next()
        .unwrap_or_default()
        .to_string();
    let from = mime::Address {
        name: (!identity.display_name.trim().is_empty()).then(|| identity.display_name.clone()),
        email: identity.email.clone(),
    };
    let reply_to = identity.reply_to.as_ref().map(|email| mime::Address::new(email.clone()));

    let list_unsubscribe = {
        let mut recipients = to.iter().chain(cc.iter()).chain(bcc.iter());
        let first = recipients.next();
        if recipients.next().is_none() {
            first.map(|address| {
                suppression::unsubscribe_url(state, organization_id, mailbox_id, &address.email)
            })
        } else {
            None
        }
    };

    let outgoing = mime::Outgoing {
        from,
        to,
        cc,
        reply_to,
        subject: compose.subject,
        body_text: compose.body_text,
        body_html: compose.body_html,
        attachments,
        in_reply_to: header_safe(compose.in_reply_to),
        references: compose
            .references
            .into_iter()
            .map(|reference| reference.replace(['\r', '\n'], " ").trim().to_string())
            .filter(|reference| !reference.is_empty())
            .collect(),
        list_unsubscribe,
        message_id_local,
        domain: domain.clone(),
    };
    let bytes = outgoing
        .build()
        .map_err(|e| ApiError::internal(format!("MIME build failed: {e}")))?;

    let mut envelope = Vec::<String>::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_env = |addresses: &[mime::Address]| {
        for address in addresses {
            let email = address.email.to_lowercase();
            if seen.insert(email.clone()) {
                envelope.push(email);
            }
        }
    };
    push_env(&outgoing.to);
    push_env(&outgoing.cc);
    push_env(&bcc);
    let self_copy = account_email.to_lowercase();
    if seen.insert(self_copy.clone()) {
        envelope.push(self_copy);
    }

    send_limit::enforce_once(
        state,
        request.id,
        user_id,
        &domain,
        plan.daily_send_limit,
        organization_daily_send_limit,
        envelope.len() as i64,
    )
    .await?;

    let claimed = sqlx::query(
        "UPDATE mail_send_requests SET status = 'submitting', recipients = $2,
                attempt_count = attempt_count + 1, submitted_at = COALESCE(submitted_at, now()),
                last_error = '', updated_at = now()
         WHERE id = $1 AND status = 'prepared'",
    )
    .bind(request.id)
    .bind(json!(&envelope))
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();

    // Only one concurrent request may cross the SMTP boundary for a logical
    // idempotency key. A racing request observes/reconciles the winner instead.
    if claimed == 0 {
        let current: SendRequestRow = sqlx::query_as(
            "SELECT id, idempotency_key, request_hash, status, message_id, sent_id,
                    recipients, last_error
             FROM mail_send_requests WHERE id = $1",
        )
        .bind(request.id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if current.status == "sent" {
            return Ok(outcome_from_row(&current));
        }
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "send_in_progress",
            "This message is already being submitted. Keep the composer open; retrying with the same send key is safe.",
        ));
    }

    if let Err(error) = state
        .stalwart
        .submit_raw(&identity.email, &envelope, &bytes)
        .await
    {
        state.metrics.record_send_failed();
        let public = error.public_message();
        let _ = sqlx::query(
            "UPDATE mail_send_requests SET status = 'uncertain', last_error = $2, updated_at = now()
             WHERE id = $1",
        )
        .bind(request.id)
        .bind(&public)
        .execute(&state.db)
        .await;
        tracing::warn!(request_id = %request.id, error = %error, "mail submission outcome uncertain");
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "send_uncertain",
            "The mail server did not return a conclusive result. CS Mail is reconciling this send; do not submit a duplicate.",
        ));
    }
    state.metrics.record_send_ok();

    // Record SMTP acceptance before any best-effort Sent-folder bookkeeping.
    mark_sent(state, request.id, &envelope, None).await?;

    let mut sent_id = None;
    for _ in 0..8 {
        match reconcile_sent_copy(state, &account, request.id, &request.message_id).await {
            Ok(Some(id)) => {
                sent_id = Some(id);
                break;
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(error) => {
                tracing::warn!(request_id = %request.id, "sent-copy lookup failed: {error}");
                break;
            }
        }
    }

    if let Some(draft_id) = draft_id {
        if let Err(error) = delete_draft(state, mailbox_id, draft_id).await {
            tracing::warn!(%draft_id, %error, "sent message but draft cleanup failed");
        }
    }
    attachments::mark_consumed(state, mailbox_id, &attachment_ids).await;

    Ok(SentOutcome {
        request_id: request.id,
        idempotency_key: request.idempotency_key,
        message_id: request.message_id,
        sent_id,
        recipients: envelope,
        deduplicated: false,
    })
}

pub async fn send(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<SendIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_outbound_sending(&state.db).await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let key = normalize_idempotency_key(&headers)?;
    let outcome = deliver(&state, auth.user_id, mailbox_id, body.compose, body.draft_id, &key).await?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.send",
        json!({
            "request_id": outcome.request_id,
            "message_id": outcome.message_id,
            "recipients": outcome.recipients,
            "deduplicated": outcome.deduplicated,
        }),
    )
    .await;

    if !outcome.deduplicated {
        if let Err(err) = emit_event(
            &state,
            auth.user_id,
            "resource-changed",
            json!({ "resource": "mailbox", "action": "sent", "mailbox_id": mailbox_id }),
        )
        .await
        {
            tracing::warn!(user_id = %auth.user_id, "failed to publish sent-mail realtime event: {err}");
        }
    }

    Ok(Json(json!({
        "ok": true,
        "request_id": outcome.request_id,
        "idempotency_key": outcome.idempotency_key,
        "message_id": outcome.message_id,
        "sent_id": outcome.sent_id,
        "stored": outcome.sent_id.is_some(),
        "deduplicated": outcome.deduplicated,
    })))
}

pub async fn send_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<SendRequestRow> = sqlx::query_as(
        "SELECT id, idempotency_key, request_hash, status, message_id, sent_id,
                recipients, last_error
         FROM mail_send_requests WHERE mailbox_id = $1 AND idempotency_key = $2",
    )
    .bind(mailbox_id)
    .bind(key)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(row) = row else {
        return Err(ApiError::not_found("Send request not found"));
    };
    Ok(Json(json!({
        "request_id": row.id,
        "idempotency_key": row.idempotency_key,
        "status": row.status,
        "message_id": row.message_id,
        "sent_id": if row.sent_id.is_empty() { Value::Null } else { json!(row.sent_id) },
        "last_error": row.last_error,
    })))
}

/// Reconcile accepted/ambiguous send ledger rows with the sender's Stalwart
/// mailbox. This repairs the Sent-folder copy after API restarts and resolves a
/// response-loss scenario without resubmitting SMTP DATA.
pub fn spawn_reconciliation_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await;
        loop {
            tick.tick().await;
            let rows: Vec<(Uuid, Uuid, Uuid, String, String)> = match sqlx::query_as(
                "SELECT id, user_id, mailbox_id, message_id, status
                 FROM mail_send_requests
                 WHERE mailbox_id IS NOT NULL
                   AND status IN ('submitting','uncertain','sent')
                   AND (sent_id = '' OR status <> 'sent')
                   AND updated_at > now() - interval '7 days'
                 ORDER BY updated_at ASC LIMIT 50",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!("send reconciliation scan failed: {error}");
                    continue;
                }
            };

            for (request_id, user_id, mailbox_id, message_id, status) in rows {
                let account_id = match account_for(&state, user_id, mailbox_id).await {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                match reconcile_sent_copy(&state, &account_id, request_id, &message_id).await {
                    Ok(Some(_)) => {}
                    Ok(None) if status == "submitting" => {
                        let _ = sqlx::query(
                            "UPDATE mail_send_requests SET status = 'uncertain', updated_at = now()
                             WHERE id = $1 AND status = 'submitting'",
                        )
                        .bind(request_id)
                        .execute(&state.db)
                        .await;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%request_id, "send reconciliation failed: {error}"),
                }
            }
        }
    });
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
    identity_id: Option<Uuid>,
    client_key: Option<Uuid>,
    send_key: Option<Uuid>,
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
        "identity_id": r.identity_id,
        "client_key": r.client_key,
        "send_key": r.send_key,
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
    "id, to_list, cc_list, bcc_list, subject, body_text, body_html, attachments, in_reply_to, references_list, identity_id, client_key, send_key, created_at, updated_at";

pub async fn drafts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    plan_for(&state, auth.user_id).await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let rows: Vec<DraftRow> = sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mail_drafts WHERE mailbox_id = $1 ORDER BY updated_at DESC LIMIT 200"
    ))
    .bind(mailbox_id)
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
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let identity = identities::resolve(&state, auth.user_id, mailbox_id, compose.identity_id).await?;
    let client_key = compose.client_key.unwrap_or_else(Uuid::new_v4);
    let send_key = compose.send_key;
    let attachment_meta =
        attachments::resolve_refs(&state, mailbox_id, &compose.attachments, &plan).await?;
    let attachment_ids = attachment_meta.iter().map(|item| item.id).collect::<Vec<_>>();
    let protect_until = chrono::Utc::now()
        + chrono::Duration::seconds(state.attachment_draft_ttl_secs as i64);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let row = sqlx::query_as::<_, DraftRow>(&format!(
        "INSERT INTO mail_drafts
           (user_id, mailbox_id, to_list, cc_list, bcc_list, subject, body_text, body_html,
            attachments, in_reply_to, references_list, identity_id, client_key, send_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, COALESCE($14, gen_random_uuid()))
         ON CONFLICT (mailbox_id, client_key) WHERE client_key IS NOT NULL
         DO UPDATE SET
            to_list = EXCLUDED.to_list,
            cc_list = EXCLUDED.cc_list,
            bcc_list = EXCLUDED.bcc_list,
            subject = EXCLUDED.subject,
            body_text = EXCLUDED.body_text,
            body_html = EXCLUDED.body_html,
            attachments = EXCLUDED.attachments,
            in_reply_to = EXCLUDED.in_reply_to,
            references_list = EXCLUDED.references_list,
            identity_id = EXCLUDED.identity_id,
            send_key = COALESCE($14, mail_drafts.send_key),
            updated_at = now()
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(json!(compose.to))
    .bind(json!(compose.cc))
    .bind(json!(compose.bcc))
    .bind(compose.subject)
    .bind(compose.body_text)
    .bind(compose.body_html.unwrap_or_default())
    .bind(attachments::refs_json(&attachment_meta))
    .bind(header_safe(compose.in_reply_to).unwrap_or_default())
    .bind(json!(&compose.references))
    .bind(identity.id)
    .bind(client_key)
    .bind(send_key)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    attachments::replace_owner_refs_tx(
        &mut tx,
        mailbox_id,
        "draft",
        row.id,
        &attachment_ids,
        protect_until,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.draft.save",
        json!({ "id": row.id, "client_key": client_key }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(draft_to_json(&row))))
}

pub async fn get_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    plan_for(&state, auth.user_id).await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<DraftRow> = sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mail_drafts WHERE id = $1 AND mailbox_id = $2"
    ))
    .bind(id)
    .bind(mailbox_id)
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
    let plan = plan_for(&state, auth.user_id).await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let to = match &patch.to {
        Some(list) => {
            to_addresses(list.clone(), "to")?;
            Some(json!(list))
        }
        None => None,
    };
    let cc = match &patch.cc {
        Some(list) => {
            to_addresses(list.clone(), "cc")?;
            Some(json!(list))
        }
        None => None,
    };
    let bcc = match &patch.bcc {
        Some(list) => {
            to_addresses(list.clone(), "bcc")?;
            Some(json!(list))
        }
        None => None,
    };
    let identity_id = match patch.identity_id {
        Some(id) => Some(identities::resolve(&state, auth.user_id, mailbox_id, Some(id)).await?.id),
        None => None,
    };
    let previous_attachment_ids = if patch.attachments.is_some() {
        attachments::owner_attachment_ids(&state, mailbox_id, "draft", id).await?
    } else {
        Vec::new()
    };
    let canonical_attachments = match &patch.attachments {
        Some(list) => Some(
            attachments::resolve_refs(&state, mailbox_id, list, &plan).await?,
        ),
        None => None,
    };
    let attachment_value = canonical_attachments
        .as_ref()
        .map(|items| attachments::refs_json(items));
    let attachment_ids = canonical_attachments.as_ref().map(|items| {
        items.iter().map(|item| item.id).collect::<Vec<_>>()
    });

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
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
            identity_id = COALESCE($11, identity_id),
            send_key    = COALESCE($12, send_key),
            updated_at  = now()
         WHERE id = $1 AND mailbox_id = $13
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(id)
    .bind(to)
    .bind(cc)
    .bind(bcc)
    .bind(patch.subject)
    .bind(patch.body_text)
    .bind(patch.body_html)
    .bind(attachment_value)
    .bind(
        patch
            .in_reply_to
            .map(|value| value.replace(['\r', '\n'], " ").trim().to_string()),
    )
    .bind(patch.references.map(|value| json!(value)))
    .bind(identity_id)
    .bind(patch.send_key)
    .bind(mailbox_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let Some(row) = row else {
        return Err(ApiError::not_found("Draft not found"));
    };

    if let Some(ids) = attachment_ids {
        attachments::replace_owner_refs_tx(
            &mut tx,
            mailbox_id,
            "draft",
            id,
            &ids,
            chrono::Utc::now()
                + chrono::Duration::seconds(state.attachment_draft_ttl_secs as i64),
        )
        .await?;
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(current) = canonical_attachments.as_ref() {
        let current_ids = current.iter().map(|item| item.id).collect::<std::collections::HashSet<_>>();
        let removed = previous_attachment_ids
            .into_iter()
            .filter(|id| !current_ids.contains(id))
            .collect::<Vec<_>>();
        attachments::mark_consumed(&state, mailbox_id, &removed).await;
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "mail.draft.update",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(draft_to_json(&row)))
}

pub async fn delete_draft_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    plan_for(&state, auth.user_id).await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    delete_draft(&state, mailbox_id, id).await?;
    audit::record(
        &state,
        Some(auth.user_id),
        "mail.draft.delete",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_draft(state: &AppState, mailbox_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let attachment_ids = attachments::owner_attachment_ids(state, mailbox_id, "draft", id).await?;
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("DELETE FROM attachment_refs WHERE mailbox_id = $1 AND owner_type = 'draft' AND owner_id = $2")
        .bind(mailbox_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let deleted = sqlx::query("DELETE FROM mail_drafts WHERE id = $1 AND mailbox_id = $2")
        .bind(id)
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found("Draft not found"));
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    attachments::mark_consumed(state, mailbox_id, &attachment_ids).await;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn idempotency_key_is_required_and_sanitized() {
        let empty = HeaderMap::new();
        assert!(normalize_idempotency_key(&empty).is_err());

        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("send_2026-09-21.abc123"));
        assert_eq!(
            normalize_idempotency_key(&headers).unwrap(),
            "send_2026-09-21.abc123"
        );

        headers.insert("idempotency-key", HeaderValue::from_static("bad key with spaces"));
        assert!(normalize_idempotency_key(&headers).is_err());
    }

    #[test]
    fn compose_fingerprint_changes_when_authoritative_sender_changes() {
        let mut compose = ComposeIn::default();
        compose.subject = "Quarterly update".into();
        let first = request_hash(&compose, None).unwrap();
        compose.identity_id = Some(Uuid::new_v4());
        let second = request_hash(&compose, None).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn compose_fingerprint_ignores_draft_transport_keys() {
        let mut compose = ComposeIn::default();
        compose.subject = "Same logical message".into();
        let first = request_hash(&compose, None).unwrap();
        compose.client_key = Some(Uuid::new_v4());
        compose.send_key = Some(Uuid::new_v4());
        let second = request_hash(&compose, None).unwrap();
        assert_eq!(first, second);
    }
}
