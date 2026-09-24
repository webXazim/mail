use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::domain::quota;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::entitlements;
use crate::state::AppState;

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    type ProfileRow = (
        String,
        String,
        String,
        bool,
        i64,
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let row: Option<ProfileRow> = sqlx::query_as(
        "SELECT u.email::text, u.display_name, u.platform_role, u.onboarded, u.quota_bytes, u.plan,
                u.active_organization_id, m.id, u.primary_mailbox_id,
                m.address::text, m.provider_account_id, m.sync_status, m.quota_bytes, m.quota_override_bytes,
                om.role, o.name
         FROM users u
         LEFT JOIN LATERAL (
           SELECT mb.id,mb.address,mb.provider_account_id,mb.sync_status,mb.quota_bytes,mb.quota_override_bytes
           FROM mailboxes mb
           JOIN organizations mbo ON mbo.id=mb.organization_id AND mbo.status='active'
           JOIN organization_memberships mbm ON mbm.organization_id=mb.organization_id AND mbm.user_id=u.id AND mbm.status='active'
           WHERE mb.user_id=u.id AND mb.deleted_at IS NULL AND mb.status='active'
             AND (u.active_organization_id IS NULL OR mb.organization_id=u.active_organization_id)
           ORDER BY CASE WHEN mb.id=u.active_mailbox_id THEN 0 WHEN mb.id=u.primary_mailbox_id THEN 1 ELSE 2 END,
                    mb.created_at ASC
           LIMIT 1
         ) m ON TRUE
         LEFT JOIN organizations o ON o.id = u.active_organization_id AND o.status='active' 
         LEFT JOIN organization_memberships om
           ON om.organization_id = u.active_organization_id
          AND om.user_id = u.id
          AND om.status = 'active'
         WHERE u.id = $1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (
        login_email,
        display_name,
        platform_role,
        onboarded,
        _materialized_quota,
        plan_code,
        active_organization_id,
        active_mailbox_id,
        primary_mailbox_id,
        mailbox_email,
        mailbox_provider_account_id,
        mailbox_sync_status,
        mailbox_quota_bytes,
        mailbox_quota_override_bytes,
        organization_role,
        organization_name,
    ) = row.ok_or_else(|| ApiError::not_found("User not found"))?;

    // A verified login can exist before it owns or joins a business. Its
    // profile must remain readable so the customer can complete onboarding.
    let organization_id = match entitlements::active_organization_for_user(&state, auth.user_id).await {
        Ok(id) => Some(id),
        Err(error) if error.status == StatusCode::FORBIDDEN
            && error.message == "Select an active business before using this feature" => None,
        Err(error) => return Err(error),
    };
    let entitlements = match organization_id {
        Some(id) => Some(entitlements::for_organization(&state, id).await?),
        None => None,
    };
    let plan_active = entitlements.as_ref().is_some_and(|value| matches!(value.subscription_status.as_str(), "active" | "trial"));
    let plan = match &entitlements {
        Some(value) => value.plan.clone(),
        None => entitlements::plan(&state, &plan_code).await?,
    };
    // Storage is mailbox-specific in Upgrade 32. The organization entitlement
    // exposes the default allocation, while the active mailbox row is the
    // authoritative quota shown to the signed-in mailbox user.
    let total = entitlements.as_ref().map(|value| {
        mailbox_quota_bytes.map(|bytes| bytes.max(0) as u64).unwrap_or(value.quota_bytes)
    }).unwrap_or(0);
    let mut used = 0u64;
    let mut provider_total: Option<u64> = None;
    let provider_account = mailbox_provider_account_id
        .as_deref()
        .filter(|value| !value.is_empty());
    if let Some(account) = provider_account {
        if let Ok(Some((provider_used, live_total))) = state.stalwart.account_quota(account).await {
            used = provider_used;
            provider_total = Some(live_total);
        }
    }
    if used == 0 {
        if let Some(mailbox_id) = active_mailbox_id {
            if let Ok(Some(cached_used)) = sqlx::query_scalar::<_, Option<i64>>(
                "SELECT quota_used FROM realtime_mailbox_state WHERE mailbox_id=$1"
            ).bind(mailbox_id).fetch_optional(&state.db).await {
                used = cached_used.unwrap_or(0).max(0) as u64;
            }
        }
    }

    let client_role = if platform_role == "platform_admin" { "admin" } else { "member" };
    let effective_sync_status = mailbox_sync_status.unwrap_or_else(|| "none".to_string());

    Ok(Json(json!({
        "id": auth.user_id,
        "email": login_email,
        "login_email": login_email,
        "display_name": display_name,
        "role": client_role,
        "platform_role": platform_role,
        "onboarded": onboarded,
        "plan": if plan_active { entitlements.as_ref().map(|value| value.plan.code.as_str()) } else { None },
        "plan_name": if plan_active { entitlements.as_ref().map(|value| value.plan.name.as_str()) } else { None },
        "subscription_status": entitlements.as_ref().map(|value| value.subscription_status.as_str()),
        "has_mailbox": active_mailbox_id.is_some() && provider_account.is_some() && effective_sync_status == "ready",
        "mailbox_email": mailbox_email,
        "mail_sync_status": effective_sync_status,
        "active_organization": active_organization_id.map(|id| json!({
            "id": id,
            "name": organization_name,
            "role": organization_role,
        })),
        "active_mailbox_id": active_mailbox_id,
        "primary_mailbox_id": primary_mailbox_id,
        "storage": {
            "used_bytes": used,
            "total_bytes": total,
            "pct": (quota::used_ratio(used, total) * 100.0).round() as i64,
            "provider_total_bytes": provider_total,
            "quota_in_sync": provider_total.map(|value| value == total).unwrap_or(false)
        },
        "entitlements": {
            "quota_bytes": total,
            "quota_override_bytes": mailbox_quota_override_bytes.map(|value| value.max(0) as u64).or(entitlements.as_ref().and_then(|value| value.quota_override_bytes)),
            "quota_source": if mailbox_quota_override_bytes.is_some() || entitlements.as_ref().is_some_and(|value| value.quota_is_overridden()) { "override" } else { "plan" },
            "feature_flags": if plan_active { plan.feature_flags.clone() } else { std::collections::BTreeMap::<String, bool>::new() }
        },
        "limits": {
            "max_attachment_bytes": if plan_active { plan.max_attachment_bytes } else { 0 },
            "max_total_attachment_bytes": if plan_active { plan.max_total_attachment_bytes } else { 0 },
            "mailbox_bytes": if plan_active { plan.mailbox_bytes } else { 0 },
            "max_recipients": if plan_active { plan.max_recipients } else { 0 },
            "daily_send_limit": if plan_active { plan.daily_send_limit } else { 0 },
            "seats": if plan_active { plan.seats } else { 0 }
        }
    })))
}

#[derive(Deserialize)]
pub struct UpdateIn {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    onboarded: Option<bool>,
}

/// Bootstrap fields the client may set on first run: the display name shown as
/// the From identity and the `onboarded` flag that dismisses first-run setup.
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateIn>,
) -> Result<Json<Value>, ApiError> {
    if let Some(raw) = body.display_name {
        let name = raw.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request("Display name is required"));
        }
        if name.chars().count() > 80 {
            return Err(ApiError::bad_request("Display name is too long"));
        }
        sqlx::query("UPDATE users SET display_name = $1, updated_at = now() WHERE id = $2")
            .bind(name)
            .bind(auth.user_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    if body.onboarded == Some(true) {
        sqlx::query("UPDATE users SET onboarded = TRUE, updated_at = now() WHERE id = $1")
            .bind(auth.user_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    get(State(state), auth).await
}
