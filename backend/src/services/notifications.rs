use serde_json::json;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;
use crate::ws::emit_event;

pub async fn create(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    title: &str,
    detail: &str,
    action_url: &str,
    dedupe_key: Option<&str>,
) -> Result<Uuid, ApiError> {
    create_scoped(state,user_id,None,kind,title,detail,action_url,dedupe_key).await
}

pub async fn create_for_mailbox(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    kind: &str,
    title: &str,
    detail: &str,
    action_url: &str,
    dedupe_key: Option<&str>,
) -> Result<Uuid, ApiError> {
    create_scoped(state,user_id,Some(mailbox_id),kind,title,detail,action_url,dedupe_key).await
}

async fn create_scoped(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Option<Uuid>,
    kind: &str,
    title: &str,
    detail: &str,
    action_url: &str,
    dedupe_key: Option<&str>,
) -> Result<Uuid, ApiError> {
    let row: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO user_notifications(user_id,mailbox_id,kind,title,detail,action_url,dedupe_key)
         VALUES($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .bind(kind)
    .bind(title)
    .bind(detail)
    .bind(action_url)
    .bind(dedupe_key)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(id)=row {
        let mut payload=json!({"resource":"notifications","id":id});
        if let Some(mailbox_id)=mailbox_id {
            payload["mailbox_id"]=json!(mailbox_id);
        }
        if let Err(error)=emit_event(state,user_id,"resource-changed",payload).await {
            tracing::warn!(%user_id,%error,"failed to publish notification event");
        }
        Ok(id)
    } else {
        Ok(Uuid::nil())
    }
}

/// Convert high-value user-owned audit actions into inbox notifications. The
/// audit row remains the immutable source of activity history; notifications
/// are a dismissible presentation layer.
pub async fn from_audit_action(
    state: &AppState,
    user_id: Uuid,
    action: &str,
    detail: &serde_json::Value,
) {
    let mapped = if action == "auth.login" {
        Some((
            "security",
            "New sign-in detected",
            "A new CS Mail session was created.",
            "/mail/audit-log",
        ))
    } else if matches!(
        action,
        "auth.password_reset" | "auth.password_changed" | "auth.2fa.enabled" | "auth.2fa.disabled"
    ) || action.starts_with("auth.2fa.")
    {
        Some((
            "security",
            "Security settings changed",
            "A security setting on your account changed.",
            "/mail/audit-log",
        ))
    } else if action == "mail.schedule.send" || action == "mail.schedule.reconciled" {
        Some((
            "scheduled",
            "Scheduled message sent",
            "Your scheduled message was delivered to the outgoing mail service.",
            "/mail/sent",
        ))
    } else if action == "mail.schedule.dead_letter" {
        Some((
            "scheduled",
            "Scheduled message needs attention",
            "A scheduled message could not be delivered after retries.",
            "/mail/scheduled",
        ))
    } else if action == "billing.order_paid_submitted" {
        Some((
            "billing",
            "Payment submitted for review",
            "Your payment reference was received and is awaiting review.",
            "/mail/billing",
        ))
    } else {
        None
    };

    if let Some((kind, title, message, url)) = mapped {
        let object_id = detail
            .get("id")
            .or_else(|| detail.get("order_id"))
            .or_else(|| detail.get("request_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let key = (!object_id.is_empty()).then(|| format!("audit:{action}:{object_id}"));
        let mailbox_id = detail
            .get("mailbox_id")
            .and_then(|value| value.as_str())
            .and_then(|value| Uuid::parse_str(value).ok());
        let result = if kind == "scheduled" {
            if let Some(mailbox_id) = mailbox_id {
                create_for_mailbox(state, user_id, mailbox_id, kind, title, message, url, key.as_deref()).await
            } else {
                // Legacy audit rows without a mailbox scope remain account-global.
                create(state, user_id, kind, title, message, url, key.as_deref()).await
            }
        } else {
            create(state, user_id, kind, title, message, url, key.as_deref()).await
        };
        if let Err(error) = result {
            tracing::warn!(%user_id, %action, %error, "notification projection failed");
        }
    }
}
