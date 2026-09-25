use std::collections::HashMap;

use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::domain;
use crate::domain::quota;
use crate::error::ApiError;
use crate::handlers::auth::valid_email;
use crate::middleware::auth::AdminUser;
use crate::services::{automation, billing, email, imap, platform_control, provisioning};
use crate::state::AppState;

const DEFAULT_AUDIT_LIMIT: i64 = 50;
const MAX_AUDIT_LIMIT: i64 = 500;
const AUDIT_EXPORT_CAP: i64 = 10_000;

/// Immutable audit row: id, timestamp, snapshotted actor email, action, detail, seal.
type AuditRow = (
    Uuid,
    chrono::DateTime<chrono::Utc>,
    String,
    String,
    Value,
    String,
);

/// Gated overview that proves the server-side role gate.
pub async fn overview(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    // Keep the operations dashboard lifecycle counts current even before the
    // periodic maintenance worker's next tick.
    billing::reconcile_subscription_lifecycle(&state).await?;
    let (users, admins, suspended_users, unverified_users): (i64, i64, i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(*) FILTER (WHERE platform_role = 'platform_admin' AND status='active'), COUNT(*) FILTER (WHERE status='suspended'), COUNT(*) FILTER (WHERE email_verified_at IS NULL) FROM users")
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    let (audit_events,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let (businesses,active_subscriptions,past_due_subscriptions,suspended_subscriptions,expiring_30_days,payment_reviews): (i64,i64,i64,i64,i64,i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM organizations WHERE is_system=FALSE)::bigint,
           count(*) FILTER (WHERE s.status='active' AND o.is_system=FALSE)::bigint,
           count(*) FILTER (WHERE s.status='past_due' AND o.is_system=FALSE)::bigint,
           count(*) FILTER (WHERE s.status='suspended' AND o.is_system=FALSE)::bigint,
           count(*) FILTER (WHERE s.status IN ('active','trial') AND o.is_system=FALSE AND s.current_period_end>now() AND s.current_period_end<=now()+interval '30 days')::bigint,
           (SELECT count(*) FROM orders ord JOIN organizations oo ON oo.id=ord.organization_id WHERE oo.is_system=FALSE AND ord.status='submitted' AND ord.invoice_status='issued')::bigint
         FROM organization_subscriptions s
         JOIN organizations o ON o.id=s.organization_id"
    ).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;

    let domain = state.stalwart.default_domain().to_string();
    let provider_healthy = if state.stalwart.enabled() {
        state.stalwart.healthcheck().await.is_ok()
    } else {
        false
    };

    Ok(Json(json!({
        "admin": { "id": admin.0.user_id, "email": admin.0.email },
        "domain": domain,
        "user_count": users,
        "admin_count": admins,
        "suspended_user_count": suspended_users,
        "unverified_user_count": unverified_users,
        "audit_events": audit_events,
        "business_count": businesses,
        "active_subscription_count": active_subscriptions,
        "past_due_subscription_count": past_due_subscriptions,
        "suspended_subscription_count": suspended_subscriptions,
        "expiring_30_days_count": expiring_30_days,
        "payment_review_count": payment_reviews,
        "provider_healthy": provider_healthy,
    })))
}

#[derive(Deserialize, Default)]
pub struct UserListQuery {
    q: Option<String>,
    status: Option<String>,
    platform_role: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    display_name: String,
    role: String,
    platform_role: String,
    status: String,
    plan: String,
    quota_bytes: i64,
    quota_override_bytes: Option<i64>,
    plan_mailbox_bytes: i64,
    mail_account_id: Option<String>,
    mail_sync_status: String,
    mail_sync_error: String,
    onboarded: bool,
    email_verified_at: Option<chrono::DateTime<chrono::Utc>>,
    last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    business_count: i64,
    primary_organization_id: Option<Uuid>,
    primary_organization_name: Option<String>,
    primary_organization_role: Option<String>,
    subscription_plan_code: Option<String>,
    subscription_plan_name: Option<String>,
    subscription_status: Option<String>,
    subscription_assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    subscription_period_end: Option<chrono::DateTime<chrono::Utc>>,
    subscription_assignment_source: Option<String>,
    business_memberships: Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// `GET /api/admin/users` — paged global user inventory. Live disk usage is
/// pulled from Stalwart only for accounts on the current page.
pub async fn users(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<UserListQuery>,
) -> Result<Json<Value>, ApiError> {
    let q = query.q.unwrap_or_default().trim().chars().take(200).collect::<String>();
    let status = query.status.filter(|value| !value.trim().is_empty());
    if status.as_deref().is_some_and(|value| !matches!(value, "active" | "suspended")) {
        return Err(ApiError::bad_request("User status filter must be active or suspended"));
    }
    let platform_role = query.platform_role.filter(|value| !value.trim().is_empty());
    if platform_role.as_deref().is_some_and(|value| !matches!(value, "user" | "platform_support" | "platform_admin")) {
        return Err(ApiError::bad_request("Platform role filter is invalid"));
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM users u
         WHERE ($1::text='' OR u.email::text ILIKE '%'||$1||'%' OR u.display_name ILIKE '%'||$1||'%'
            OR EXISTS (
              SELECT 1 FROM organization_memberships omq
              JOIN organizations oq ON oq.id=omq.organization_id
              LEFT JOIN organization_subscriptions sq ON sq.organization_id=oq.id
              LEFT JOIN plans pq ON pq.code=sq.plan_code
              WHERE omq.user_id=u.id AND (
                oq.name ILIKE '%'||$1||'%' OR COALESCE(pq.name,'') ILIKE '%'||$1||'%'
                OR omq.role ILIKE '%'||$1||'%'
              )
            ))
           AND ($2::text IS NULL OR u.status=$2)
           AND ($3::text IS NULL OR u.platform_role=$3)"
    ).bind(&q).bind(status.as_deref()).bind(platform_role.as_deref())
     .fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;

    let rows: Vec<UserRow> = sqlx::query_as(
        "SELECT u.id, u.email::text, u.display_name,
                CASE WHEN u.platform_role='platform_admin' THEN 'admin' WHEN u.platform_role='platform_support' THEN 'billing' ELSE 'member' END AS role,
                u.platform_role, u.status, u.plan, u.quota_bytes,
                u.quota_override_bytes, p.mailbox_bytes AS plan_mailbox_bytes,
                u.mail_account_id, u.mail_sync_status, u.mail_sync_error, u.onboarded,
                u.email_verified_at,
                (SELECT max(s.last_used_at) FROM sessions s WHERE s.user_id=u.id) AS last_active_at,
                (SELECT count(*)::bigint FROM organization_memberships omc WHERE omc.user_id=u.id AND omc.status='active') AS business_count,
                bo.organization_id AS primary_organization_id, bo.organization_name AS primary_organization_name,
                bo.organization_role AS primary_organization_role, bo.plan_code AS subscription_plan_code,
                bo.plan_name AS subscription_plan_name, bo.subscription_status, bo.assigned_at AS subscription_assigned_at,
                bo.current_period_end AS subscription_period_end, bo.assignment_source AS subscription_assignment_source,
                COALESCE((
                  SELECT jsonb_agg(jsonb_build_object(
                    'organization_id',o2.id,'organization_name',o2.name,'role',om2.role,'membership_status',om2.status,
                    'plan_code',s2.plan_code,'plan_name',p3.name,'subscription_status',s2.status,
                    'assigned_at',s2.assigned_at,'current_period_start',s2.current_period_start,
                    'current_period_end',s2.current_period_end,'assignment_source',s2.assignment_source
                  ) ORDER BY lower(o2.name))
                  FROM organization_memberships om2
                  JOIN organizations o2 ON o2.id=om2.organization_id
                  LEFT JOIN organization_subscriptions s2 ON s2.organization_id=o2.id
                  LEFT JOIN plans p3 ON p3.code=s2.plan_code
                  WHERE om2.user_id=u.id
                ),'[]'::jsonb) AS business_memberships,
                u.created_at
         FROM users u JOIN plans p ON p.code = u.plan
         LEFT JOIN LATERAL (
           SELECT o.id AS organization_id,o.name AS organization_name,om.role AS organization_role,
                  s.plan_code,p2.name AS plan_name,s.status AS subscription_status,s.assigned_at,s.current_period_end,s.assignment_source
           FROM organization_memberships om
           JOIN organizations o ON o.id=om.organization_id
           LEFT JOIN organization_subscriptions s ON s.organization_id=o.id
           LEFT JOIN plans p2 ON p2.code=s.plan_code
           WHERE om.user_id=u.id AND om.status='active'
           ORDER BY CASE WHEN o.id=u.active_organization_id THEN 0 ELSE 1 END,
                    CASE om.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'billing' THEN 2 ELSE 3 END,om.joined_at
           LIMIT 1
         ) bo ON TRUE
         WHERE ($1::text='' OR u.email::text ILIKE '%'||$1||'%' OR u.display_name ILIKE '%'||$1||'%'
            OR EXISTS (
              SELECT 1 FROM organization_memberships omq
              JOIN organizations oq ON oq.id=omq.organization_id
              LEFT JOIN organization_subscriptions sq ON sq.organization_id=oq.id
              LEFT JOIN plans pq ON pq.code=sq.plan_code
              WHERE omq.user_id=u.id AND (
                oq.name ILIKE '%'||$1||'%' OR COALESCE(pq.name,'') ILIKE '%'||$1||'%'
                OR omq.role ILIKE '%'||$1||'%'
              )
            ))
           AND ($2::text IS NULL OR u.status=$2)
           AND ($3::text IS NULL OR u.platform_role=$3)
         ORDER BY u.created_at DESC
         LIMIT $4 OFFSET $5"
    )
    .bind(&q).bind(status.as_deref()).bind(platform_role.as_deref()).bind(limit).bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let accounts: Vec<String> = rows
        .iter()
        .filter_map(|r| r.mail_account_id.clone())
        .filter(|a| !a.is_empty())
        .collect();
    // Live usage is best-effort: if Stalwart is unreachable the admin list
    // still renders with zeroed storage rather than failing the whole page.
    let quotas = state.stalwart.account_quotas(&accounts).await.unwrap_or_default();

    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            let live = r
                .mail_account_id
                .as_deref()
                .and_then(|a| quotas.get(a))
                .copied();
            let used = live.map_or(0, |(u, _)| u);
            let provider_quota_bytes = live.map(|(_, t)| t as i64).filter(|t| *t > 0);
            let quota_bytes = r.quota_bytes;
            json!({
                "id": r.id,
                "email": r.email,
                "display_name": r.display_name,
                "role": r.role,
                "platform_role": r.platform_role,
                "plan": r.plan,
                "status": r.status,
                "onboarded": r.onboarded,
                "email_verified": r.email_verified_at.is_some(),
                "email_verified_at": r.email_verified_at,
                "last_active_at": r.last_active_at,
                "business_count": r.business_count,
                "primary_organization_id": r.primary_organization_id,
                "primary_organization_name": r.primary_organization_name,
                "primary_organization_role": r.primary_organization_role,
                "subscription_plan_code": r.subscription_plan_code,
                "subscription_plan_name": r.subscription_plan_name,
                "subscription_status": r.subscription_status,
                "subscription_assigned_at": r.subscription_assigned_at,
                "subscription_period_end": r.subscription_period_end,
                "subscription_assignment_source": r.subscription_assignment_source,
                "business_memberships": r.business_memberships,
                "mail_account_id": r.mail_account_id,
                "mail_sync_status": r.mail_sync_status,
                "mail_sync_error": r.mail_sync_error,
                "quota_bytes": quota_bytes,
                "quota_override_bytes": r.quota_override_bytes,
                "quota_source": if r.quota_override_bytes.is_some() { "override" } else { "plan" },
                "plan_mailbox_bytes": r.plan_mailbox_bytes,
                "provider_quota_bytes": provider_quota_bytes,
                "quota_in_sync": provider_quota_bytes.map(|value| value == quota_bytes).unwrap_or(false),
                "storage_used_bytes": used,
                "storage_pct": (quota::used_ratio(used, quota_bytes.max(0) as u64) * 100.0).round() as i64,
                "created_at": r.created_at,
            })
        })
        .collect();

    Ok(Json(json!({ "users": list, "total": total, "limit": limit, "offset": offset })))
}

#[derive(Deserialize)]
pub struct UserPatch {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    platform_role: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    quota_bytes: Option<i64>,
    /// Clear an existing per-user quota exception and return to the selected
    /// plan's mailbox quota. Kept separate from `quota_bytes` so omitted JSON
    /// cannot accidentally clear an override.
    #[serde(default)]
    reset_quota_override: bool,
    #[serde(default)]
    password: Option<String>,
}

/// `PATCH /api/admin/users/:id` — role, plan, quota and display name. Quota
/// changes are committed with a durable provider job instead of a best-effort
/// synchronous mirror.
pub async fn update_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UserPatch>,
) -> Result<Json<Value>, ApiError> {
    let current: Option<(String, String, String, String, String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT role, platform_role, status, plan, email, quota_bytes, quota_override_bytes, mail_account_id
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (role, current_platform_role, current_status, current_plan, email, _quota_bytes, current_override, mail_account_id) =
        current.ok_or_else(|| ApiError::not_found("User not found"))?;

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
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    if let Some(next_role) = body.role.as_deref() {
        if !matches!(next_role, "member" | "admin" | "billing") {
            return Err(ApiError::bad_request("Legacy role must be member, admin or billing"));
        }
        if next_role != role {
            sqlx::query("UPDATE users SET role=$1,updated_at=now() WHERE id=$2")
                .bind(next_role).bind(id).execute(&state.db).await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            audit::record(&state,Some(admin.0.user_id),"admin.user.legacy_role",json!({
                "user_id":id,"from":role,"to":next_role
            })).await;
        }
    }

    if let Some(next_platform_role) = body.platform_role.as_deref() {
        if !matches!(next_platform_role, "user" | "platform_support" | "platform_admin") {
            return Err(ApiError::bad_request("Platform role must be user, platform_support or platform_admin"));
        }
        if next_platform_role != current_platform_role {
            if id == admin.0.user_id {
                return Err(ApiError::bad_request("You cannot change your own platform role"));
            }
            if current_platform_role == "platform_admin" && next_platform_role != "platform_admin" {
                let (admins,): (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM users WHERE platform_role='platform_admin' AND status='active'",
                ).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
                if admins <= 1 {
                    return Err(ApiError::bad_request("Cannot remove the last platform administrator"));
                }
            }
            sqlx::query("UPDATE users SET platform_role=$1,updated_at=now() WHERE id=$2")
                .bind(next_platform_role).bind(id).execute(&state.db).await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(id).execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
            audit::record(&state,Some(admin.0.user_id),"admin.user.platform_role",json!({
                "user_id":id,"from":current_platform_role,"to":next_platform_role,"sessions_revoked":true
            })).await;
        }
    }

    if let Some(next_status) = body.status.as_deref() {
        if !matches!(next_status, "active" | "suspended") {
            return Err(ApiError::bad_request("Status must be active or suspended"));
        }
        if next_status != current_status {
            if id == admin.0.user_id && next_status != "active" {
                return Err(ApiError::bad_request("You cannot suspend your own account"));
            }
            // Status is a two-system mutation. Commit the desired state and a
            // durable provider access job atomically; the worker applies the
            // current DB status, so rapid status changes cannot replay stale
            // suspension state after an outage.
            let mut status_tx = state
                .db
                .begin()
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query("UPDATE users SET status = $1, updated_at = now() WHERE id = $2")
                .bind(next_status)
                .bind(id)
                .execute(&mut *status_tx)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            state
                .provisioning
                .enqueue_access_tx(
                    &mut status_tx,
                    id,
                    &email,
                    mail_account_id.as_deref(),
                )
                .await?;
            if next_status == "suspended" {
                sqlx::query(
                    "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
                )
                .bind(id)
                .execute(&mut *status_tx)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            }
            status_tx
                .commit()
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            provisioning::process_user_access_now(&state, id).await;
            audit::record(
                &state,
                Some(admin.0.user_id),
                "admin.user.status",
                json!({
                    "user_id": id,
                    "from": current_status,
                    "to": next_status,
                    "provider_access_queued": true,
                    "sessions_revoked": next_status == "suspended"
                }),
            )
            .await;
        }
    }

    // Plan + quota override are one entitlement mutation. The plan owns the
    // default. A custom quota is represented explicitly in
    // `quota_override_bytes`; `quota_bytes` is only the materialized effective
    // value synchronized to Stalwart.
    let mut next_plan = current_plan.clone();
    let mut next_override = current_override;
    if let Some(code) = body.plan.as_deref() {
        if !billing::plan_exists(&state, code).await? {
            return Err(ApiError::bad_request(format!("Unknown plan '{code}'")));
        }
        next_plan = code.to_string();
        // Preserve the old UI semantics: choosing a different plan without an
        // explicit custom quota adopts the new plan quota.
        if body.quota_bytes.is_none() && !body.reset_quota_override {
            next_override = None;
        }
    }
    if body.reset_quota_override {
        next_override = None;
    }
    if let Some(custom_quota) = body.quota_bytes {
        if custom_quota < 1024 * 1024 {
            return Err(ApiError::bad_request("Quota must be at least 1 MiB"));
        }
        next_override = Some(custom_quota);
    }

    let plan_changed = next_plan != current_plan;
    let override_changed = next_override != current_override;
    if plan_changed || override_changed {
        return Err(ApiError::bad_request(
            "User-level plan and quota edits are deprecated. Manage the selected business subscription in Admin Billing.",
        ));
    }

    if let Some(password) = body.password.as_deref() {
        domain::password::validate_password(password, &email)?;
        let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).expect("base64 salt");
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .to_string();
        let mut password_tx = state
            .db
            .begin()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(hash)
            .bind(id)
            .execute(&mut *password_tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let legacy_mailbox: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT m.address::text, m.provider_account_id
             FROM users u
             JOIN mailboxes m ON m.id=u.primary_mailbox_id AND m.deleted_at IS NULL
             JOIN organizations o ON o.id=m.organization_id AND o.is_system=TRUE
             WHERE u.id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *password_tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let sync_legacy_mailbox = legacy_mailbox.is_some();
        if let Some((mailbox_address, provider_account_id)) = legacy_mailbox {
            state
                .provisioning
                .enqueue_credentials_tx(
                    &mut password_tx,
                    id,
                    &mailbox_address,
                    provider_account_id.as_deref(),
                    password,
                )
                .await?;
        }
        sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&mut *password_tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        password_tx
            .commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        audit::record(
            &state,
            Some(admin.0.user_id),
            "admin.user.password_reset",
            json!({ "user_id": id, "sessions_revoked": true }),
        )
        .await;
        if sync_legacy_mailbox {
            provisioning::process_user_credentials_now(&state, id).await;
        }
    }

    users(
        State(state),
        admin,
        Query(UserListQuery {
            q: Some(email.clone()),
            limit: Some(200),
            ..UserListQuery::default()
        }),
    )
    .await
    .map(|Json(v)| {
        let entry = v
            .get("users")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .find(|u| u.get("id").and_then(Value::as_str) == Some(&id.to_string()))
            })
            .cloned()
            .unwrap_or(Value::Null);
        Json(entry)
    })
}

/// Body for `POST /api/admin/users` — admin provisions an account directly,
/// bypassing self-signup verification (an admin vouches for the user).
#[derive(Deserialize)]
pub struct UserCreate {
    email: String,
    display_name: String,
    password: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    platform_role: Option<String>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    quota_bytes: Option<i64>,
}

/// `POST /api/admin/users` — create a verified account + Stalwart mailbox,
/// mirrored from the self-signup path but with admin-set role/plan/quota.
pub async fn create_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<UserCreate>,
) -> Result<Json<Value>, ApiError> {
    let email = body.email.trim().to_lowercase();
    let display_name = body.display_name.trim();

    if display_name.is_empty() {
        return Err(ApiError::bad_request("Display name is required"));
    }
    if display_name.chars().count() > 80 {
        return Err(ApiError::bad_request("Display name is too long"));
    }
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Invalid email address"));
    }
    let email_domain = email.split('@').nth(1).unwrap_or_default();
    if !email_domain.eq_ignore_ascii_case(state.stalwart.default_domain()) {
        return Err(ApiError::bad_request(format!(
            "Platform mailbox creation is limited to @{}. Customer domains must use business onboarding.",
            state.stalwart.default_domain()
        )));
    }
    domain::password::validate_password(&body.password, &email)?;

    let role = body.role.unwrap_or_else(|| "member".into());
    if !matches!(role.as_str(), "member" | "admin" | "billing") {
        return Err(ApiError::bad_request(
            "Role must be member, admin or billing",
        ));
    }
    let platform_role = body.platform_role.unwrap_or_else(|| "user".into());
    if !matches!(platform_role.as_str(), "user" | "platform_support" | "platform_admin") {
        return Err(ApiError::bad_request("Platform role must be user, platform_support or platform_admin"));
    }
    let organization_role = match role.as_str() {
        "admin" => "owner",
        "billing" => "billing",
        _ => "member",
    };

    let plan = body.plan.unwrap_or_else(|| "solo".into());
    if !billing::plan_exists(&state, &plan).await? {
        return Err(ApiError::bad_request(format!("Unknown plan '{plan}'")));
    }
    let quota_override_bytes = body.quota_bytes;
    if quota_override_bytes.is_some_and(|value| value < 1024 * 1024) {
        return Err(ApiError::bad_request("Quota must be at least 1 MiB"));
    }
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if exists.is_some() {
        return Err(ApiError::conflict(
            "An account with this email already exists",
        ));
    }

    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes()).expect("base64 salt");
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .to_string();

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let user: (Uuid, String, i64) = sqlx::query_as(
        "INSERT INTO users
            (email, display_name, password_hash, role, platform_role, plan, quota_bytes,
             quota_override_bytes, email_verified_at, mail_sync_status)
         SELECT $1, $2, $3, $4, $5, p.code, COALESCE($7::bigint, p.mailbox_bytes), $7::bigint, now(), 'pending'
         FROM plans p WHERE p.code = $6
         RETURNING id, email::text, quota_bytes",
    )
    .bind(&email)
    .bind(display_name)
    .bind(hash)
    .bind(&role)
    .bind(&platform_role)
    .bind(&plan)
    .bind(quota_override_bytes)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE is_system=TRUE LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(format!("protected CrescentSphere organization is missing: {e}")))?;

    // The configured default domain is reserved under the protected system
    // organization. This keeps platform-admin mailbox creation separate from
    // public customer domain onboarding.
    sqlx::query(
        "INSERT INTO organization_domains(organization_id,domain,status,is_primary,is_system,activated_at)
         VALUES($1,lower($2)::citext,'active',FALSE,TRUE,now())
         ON CONFLICT DO NOTHING",
    )
    .bind(organization_id)
    .bind(state.stalwart.default_domain())
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let domain_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM organization_domains WHERE lower(domain::text)=lower($1) AND organization_id=$2",
    )
    .bind(state.stalwart.default_domain())
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO organization_memberships(organization_id,user_id,role,status)
         VALUES($1,$2,$3,'active') ON CONFLICT(organization_id,user_id) DO UPDATE
         SET role=EXCLUDED.role,status='active',updated_at=now()",
    )
    .bind(organization_id)
    .bind(user.0)
    .bind(organization_role)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mailbox_id: Uuid = sqlx::query_scalar(
        "INSERT INTO mailboxes(
           organization_id,domain_id,user_id,address,local_part,display_name,status,
           is_primary_for_user,sync_status,quota_bytes
         ) VALUES($1,$2,$3,$4,split_part(lower($4::text),'@',1),$5,'pending',TRUE,'pending',$6)
         RETURNING id",
    )
    .bind(organization_id)
    .bind(domain_id)
    .bind(user.0)
    .bind(&email)
    .bind(display_name)
    .bind(user.2)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE users SET active_organization_id=$1,primary_mailbox_id=$2,updated_at=now() WHERE id=$3",
    )
    .bind(organization_id)
    .bind(mailbox_id)
    .bind(user.0)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .provisioning
        .enqueue_ensure_tx(&mut tx, user.0, &user.1, &body.password, user.2)
        .await?;
    sqlx::query(
        "UPDATE provisioning_jobs SET organization_id=$1,mailbox_id=$2
         WHERE user_id=$3 AND operation='ensure_mailbox' AND status IN ('pending','retry')",
    )
    .bind(organization_id)
    .bind(mailbox_id)
    .bind(user.0)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    provisioning::process_user_ensure_now(&state, user.0).await;

    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.user.created",
        json!({ "user_id": user.0, "email": user.1, "role": role, "platform_role": platform_role, "organization_id": organization_id, "mailbox_id": mailbox_id, "plan": plan }),
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "id": user.0,
        "email": user.1,
        "role": role,
        "platform_role": platform_role,
        "organization_id": organization_id,
        "mailbox_id": mailbox_id,
        "plan": plan,
        "quota_bytes": user.2,
    })))
}

/// `DELETE /api/admin/users/:id` — irreversible erasure of another account
/// (WS5.4). Refuses self-deletion and removing the final admin.
pub async fn delete_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    if id == admin.0.user_id {
        return Err(ApiError::bad_request("You cannot delete your own account"));
    }

    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT platform_role, mail_account_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let (role, account_id) = row.ok_or_else(|| ApiError::not_found("User not found"))?;

    if role == "platform_admin" {
        let (admins,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE platform_role = 'platform_admin'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if admins <= 1 {
            return Err(ApiError::bad_request("Cannot delete the last platform administrator"));
        }
    }

    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.user.erase",
        json!({ "user_id": id }),
    )
    .await;

    crate::handlers::account::erase(&state, id, account_id).await?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/admin/aliases` — admin-wide alias roster (WS3.6).
pub async fn aliases(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({ "aliases": crate::handlers::aliases::all(&state).await? }),
    ))
}

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    action: Option<String>,
}

async fn audit_rows(
    state: &AppState,
    limit: i64,
    offset: i64,
    action: Option<&str>,
) -> Result<(Vec<Value>, i64), ApiError> {
    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
         WHERE ($1::text IS NULL OR action = $1)",
    )
    .bind(action)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT a.id, a.at, a.actor_email::text, a.action, a.detail, a.event_hash
             FROM audit_log a
             WHERE ($1::text IS NULL OR a.action = $1)
             ORDER BY a.at DESC
             LIMIT $2 OFFSET $3",
    )
    .bind(action)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let entries = rows
        .into_iter()
        .map(|(id, at, actor, action, detail, event_hash)| {
            json!({
                "id": id,
                "time": at,
                "actor": actor,
                "action": action,
                "detail": detail,
                "eventHash": event_hash,
            })
        })
        .collect();

    Ok((entries, total))
}

/// `GET /api/admin/audit?limit=&offset=&action=` — paginated audit trail with
/// the actor's email resolved.
pub async fn audit(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = q
        .limit
        .unwrap_or(DEFAULT_AUDIT_LIMIT)
        .clamp(1, MAX_AUDIT_LIMIT);
    let offset = q.offset.unwrap_or(0).max(0);
    let (entries, total) = audit_rows(&state, limit, offset, q.action.as_deref()).await?;
    Ok(Json(json!({ "entries": entries, "total": total })))
}

/// `GET /api/admin/audit/export` — full audit trail as CSV (WS5.5).
pub async fn audit_export(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, ApiError> {
    let (entries, _) = audit_rows(&state, AUDIT_EXPORT_CAP, 0, None).await?;

    let mut csv = String::from("id,time,actor,action,detail,event_hash\r\n");
    for entry in &entries {
        csv.push_str(&csv_field(entry["id"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&csv_field(entry["time"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&csv_field(entry["actor"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&csv_field(entry["action"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&csv_field(&entry["detail"].to_string()));
        csv.push(',');
        csv.push_str(&csv_field(entry["eventHash"].as_str().unwrap_or("")));
        csv.push_str("\r\n");
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"cs-mail-audit.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

/// RFC 4180 quoting: wrap when the value contains a comma, quote or newline.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// `GET /api/admin/suppressions` — permanent do-not-send list (WS7.2).
pub async fn suppressions(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({ "suppressions": crate::services::suppression::list(&state).await? }),
    ))
}

#[derive(Deserialize)]
pub struct SuppressIn {
    email: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `POST /api/admin/suppressions` — manual block (spam complaint, legal hold).
pub async fn add_suppression(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SuppressIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let email = body.email.trim();
    if !email.contains('@') {
        return Err(ApiError::bad_request("A valid email address is required"));
    }
    let reason = body.reason.as_deref().unwrap_or("manual");
    let source = body.source.as_deref().unwrap_or("admin");
    crate::services::suppression::suppress(&state, email, reason, source, "").await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.suppress",
        json!({ "email": email.to_ascii_lowercase(), "reason": reason }),
    )
    .await;
    Ok((axum::http::StatusCode::CREATED, Json(json!({ "ok": true }))))
}

/// `DELETE /api/admin/suppressions/:email`.
pub async fn remove_suppression(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(email): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let removed = crate::services::suppression::unsuppress(&state, &email).await?;
    if !removed {
        return Err(ApiError::not_found("Address is not suppressed"));
    }
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.unsuppress",
        json!({ "email": email.to_ascii_lowercase() }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct DsnIn {
    /// Raw RFC 3464 delivery-status report body.
    body: String,
}

/// Legacy raw-DSN endpoint. Upgrade 25 intentionally refuses to turn an
/// uncorrelated DSN into a platform-global suppression: provider events must
/// identify the business, mailbox, original send request and recipient.
pub async fn ingest_dsn(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<DsnIn>,
) -> Result<Json<Value>, ApiError> {
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.suppress.dsn.rejected",
        json!({ "bytes": body.body.len(), "reason": "uncorrelated_dsn" }),
    )
    .await;
    Err(ApiError::bad_request(
        "Raw DSN suppression is disabled because it has no tenant/send correlation. Submit a normalized event through /api/admin/deliverability/events or the signed provider endpoint.",
    ))
}


fn provider_err(error: crate::services::stalwart::StalwartError) -> ApiError {
    tracing::warn!(%error, "admin mail-provider operation failed");
    ApiError::new(
        axum::http::StatusCode::BAD_GATEWAY,
        "mail_provider",
        error.public_message(),
    )
}

fn hash_admin_token(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

async fn sync_admin_automation(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<Value, ApiError> {
    automation::mark_dirty(state, user_id, mailbox_id)
        .await
        .map_err(ApiError::internal)?;
    if let Err(error) = automation::sync_user(state, user_id, mailbox_id).await {
        tracing::warn!(%user_id, %mailbox_id, %error, "admin forwarding change queued for reconciliation");
    }
    automation::sync_status(state, mailbox_id)
        .await
        .map_err(ApiError::internal)
}

fn provider_singleton(result: &Value) -> Option<&Value> {
    result
        .get("list")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
}

fn zone_flags(zone: &str) -> Value {
    let lower = zone.to_ascii_lowercase();
    json!({
        "mx": lower.contains(" mx "),
        "spf": lower.contains("v=spf1"),
        "dkim": lower.contains("._domainkey") || lower.contains("_domainkey."),
        "dmarc": lower.contains("_dmarc") && lower.contains("v=dmarc1")
    })
}

/// `GET /api/admin/domain` — provider-native domain and expected DNS zone.
pub async fn domain_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let domain = state.stalwart.default_domain().trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err(ApiError::internal("Managed mail domain is not configured"));
    }
    if !state.stalwart.enabled() {
        return Ok(Json(json!({
            "domain": domain,
            "providerAvailable": false,
            "enabled": false,
            "catchAllEnabled": false,
            "catchAll": "",
            "dnsZoneFile": "",
            "dns": {"mx": false, "spf": false, "dkim": false, "dmarc": false},
            "dnsManagement": "Unavailable"
        })));
    }
    let id = state
        .stalwart
        .domain_id(&domain)
        .await
        .map_err(provider_err)?
        .ok_or_else(|| ApiError::not_found("Managed mail domain was not found in the mail server"))?;
    let result = state
        .stalwart
        .management_read("x:Domain/get", json!({"ids": [id]}))
        .await
        .map_err(provider_err)?;
    let item = provider_singleton(&result)
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "mail_provider", "Mail domain was not returned"))?;
    let catch_all = item
        .get("catchAllAddress")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let zone = item
        .get("dnsZoneFile")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let dns_management = item
        .get("dnsManagement")
        .and_then(|value| value.get("@type"))
        .and_then(Value::as_str)
        .unwrap_or("Manual");
    Ok(Json(json!({
        "domain": domain,
        "providerAvailable": true,
        "enabled": item.get("isEnabled").and_then(Value::as_bool).unwrap_or(true),
        "catchAllEnabled": !catch_all.is_empty(),
        "catchAll": catch_all,
        "dnsZoneFile": zone,
        "dns": zone_flags(&zone),
        "dnsManagement": dns_management,
        "dkimManagement": item.get("dkimManagement").cloned().unwrap_or(Value::Null),
    })))
}

#[derive(Deserialize)]
pub struct DomainPatch {
    #[serde(default)]
    catch_all_enabled: Option<bool>,
    #[serde(default)]
    catch_all: Option<String>,
}

async fn valid_catch_all_target(state: &AppState, target: &str) -> Result<bool, ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM users WHERE status='active' AND lower(email::text)=lower($1)
           UNION ALL
           SELECT 1 FROM aliases
            WHERE deleted_at IS NULL AND enabled
              AND lower(source || '@' || domain)=lower($1)
         )",
    )
    .bind(target)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(exists)
}

/// `PATCH /api/admin/domain` — currently the CS Mail admin surface owns the
/// catch-all target. DNS/DKIM continue to be provider-native and are exposed
/// read-only through the zone snapshot above.
pub async fn update_domain_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<DomainPatch>,
) -> Result<Json<Value>, ApiError> {
    if !state.stalwart.enabled() {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "mail_provider",
            "Mail service is not configured",
        ));
    }
    let domain = state.stalwart.default_domain().trim().to_ascii_lowercase();
    let id = state
        .stalwart
        .domain_id(&domain)
        .await
        .map_err(provider_err)?
        .ok_or_else(|| ApiError::not_found("Managed mail domain was not found in the mail server"))?;

    let enabled = body.catch_all_enabled.unwrap_or_else(|| {
        body.catch_all
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    });
    let target = body.catch_all.unwrap_or_default().trim().to_ascii_lowercase();
    if enabled {
        if !valid_email(&target) {
            return Err(ApiError::bad_request("Enter a valid catch-all recipient"));
        }
        if !valid_catch_all_target(&state, &target).await? {
            return Err(ApiError::bad_request(
                "Catch-all must route to an active CS Mail mailbox or alias",
            ));
        }
    }
    let mut update = serde_json::Map::new();
    update.insert(
        id.clone(),
        json!({"catchAllAddress": if enabled { Value::String(target.clone()) } else { Value::Null }}),
    );
    let result = state
        .stalwart
        .management_write("x:Domain/set", json!({"update": update}))
        .await
        .map_err(provider_err)?;
    if let Some(reason) = result
        .get("notUpdated")
        .and_then(|value| value.get(&id))
        .filter(|value| !value.is_null())
    {
        tracing::warn!(domain=%domain, rejection=%reason, "mail provider rejected catch-all update");
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "mail_provider",
            "Mail service rejected the catch-all update",
        ));
    }
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.domain.catch_all",
        json!({"domain":domain,"enabled":enabled,"target":if enabled { target } else { String::new() }}),
    )
    .await;
    domain_settings(State(state), admin).await
}

fn milliseconds_for_days(days: i32) -> Option<i64> {
    if days <= 0 { None } else { Some(i64::from(days) * 86_400_000) }
}

/// `GET /api/admin/security-policy` — authoritative provider-backed controls
/// that map cleanly to Stalwart objects. Controls that require a different
/// deployment topology are explicitly reported as unsupported rather than
/// pretending a browser toggle changed the MTA.
pub async fn security_policy(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let row: (f64, i32, bool) = sqlx::query_as(
        "SELECT spam_threshold, retention_days, trash_auto_purge
           FROM admin_mail_policy WHERE singleton=TRUE",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut spam_threshold = row.0;
    let mut retention_days = row.1;
    let mut trash_auto_purge = row.2;
    let mut provider_available = false;

    if state.stalwart.enabled() {
        let spam = state
            .stalwart
            .management_read("x:SpamSettings/get", json!({"ids":["singleton"]}))
            .await
            .map_err(provider_err)?;
        if let Some(item) = provider_singleton(&spam) {
            spam_threshold = item
                .get("scoreSpam")
                .and_then(Value::as_f64)
                .unwrap_or(spam_threshold);
        }
        let retention = state
            .stalwart
            .management_read("x:DataRetention/get", json!({"ids":["singleton"]}))
            .await
            .map_err(provider_err)?;
        if let Some(item) = provider_singleton(&retention) {
            match item.get("expungeTrashAfter") {
                Some(Value::Number(value)) => {
                    if let Some(ms) = value.as_i64() {
                        retention_days = (ms / 86_400_000).clamp(1, 3650) as i32;
                        trash_auto_purge = true;
                    }
                }
                Some(Value::Null) => {
                    retention_days = 0;
                    trash_auto_purge = false;
                }
                _ => {}
            }
        }
        provider_available = true;
    }

    Ok(Json(json!({
        "spamThreshold": spam_threshold,
        "retentionDays": retention_days,
        "trashAutoPurge": trash_auto_purge,
        "blockedSenders": [],
        "requireTls": false,
        "scanAttachments": false,
        "dmarcPolicy": "none",
        "providerAvailable": provider_available,
        "supported": {
            "spamThreshold": true,
            "retention": true,
            "blockedSenders": true,
            "requireTls": false,
            "scanAttachments": false,
            "dmarcPolicy": false,
            "sso": false
        }
    })))
}

#[derive(Deserialize)]
pub struct SecurityPolicyPatch {
    #[serde(default, rename = "spamThreshold")]
    spam_threshold: Option<f64>,
    #[serde(default, rename = "retentionDays")]
    retention_days: Option<i32>,
    #[serde(default, rename = "trashAutoPurge")]
    trash_auto_purge: Option<bool>,
}

pub async fn update_security_policy(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SecurityPolicyPatch>,
) -> Result<Json<Value>, ApiError> {
    if body.spam_threshold.is_none()
        && body.retention_days.is_none()
        && body.trash_auto_purge.is_none()
    {
        return Err(ApiError::bad_request("No supported security policy field was provided"));
    }
    if let Some(value) = body.spam_threshold {
        if !(-100.0..=100.0).contains(&value) {
            return Err(ApiError::bad_request("Spam threshold must be between -100 and 100"));
        }
    }
    if let Some(value) = body.retention_days {
        if !(0..=3650).contains(&value) {
            return Err(ApiError::bad_request("Retention must be between 0 and 3650 days"));
        }
    }

    let current: (f64, i32, bool) = sqlx::query_as(
        "SELECT spam_threshold, retention_days, trash_auto_purge
           FROM admin_mail_policy WHERE singleton=TRUE",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let next_spam = body.spam_threshold.unwrap_or(current.0);
    let next_retention = body.retention_days.unwrap_or(current.1);
    let next_purge = body.trash_auto_purge.unwrap_or(current.2);

    if next_purge && next_retention <= 0 {
        return Err(ApiError::bad_request(
            "Retention must be at least 1 day when automatic purge is enabled",
        ));
    }
    if !state.stalwart.enabled() {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "mail_provider",
            "Mail service is not configured",
        ));
    }

    if body.spam_threshold.is_some() {
        let result = state
            .stalwart
            .management_write(
                "x:SpamSettings/set",
                json!({"update":{"singleton":{"scoreSpam":next_spam}}}),
            )
            .await
            .map_err(provider_err)?;
        if result
            .get("notUpdated")
            .and_then(|value| value.get("singleton"))
            .is_some_and(|value| !value.is_null())
        {
            tracing::warn!(?result, "mail provider rejected spam-policy update");
            return Err(ApiError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "mail_provider",
                "Mail server rejected the spam-policy update",
            ));
        }
    }
    if body.retention_days.is_some() || body.trash_auto_purge.is_some() {
        let expunge = if next_purge {
            milliseconds_for_days(next_retention)
                .map(Value::from)
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let result = state
            .stalwart
            .management_write(
                "x:DataRetention/set",
                json!({"update":{"singleton":{"expungeTrashAfter":expunge}}}),
            )
            .await
            .map_err(provider_err)?;
        if result
            .get("notUpdated")
            .and_then(|value| value.get("singleton"))
            .is_some_and(|value| !value.is_null())
        {
            tracing::warn!(?result, "mail provider rejected retention-policy update");
            return Err(ApiError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "mail_provider",
                "Mail server rejected the retention-policy update",
            ));
        }
    }

    sqlx::query(
        "UPDATE admin_mail_policy
            SET spam_threshold=$1, retention_days=$2, trash_auto_purge=$3,
                updated_by=$4, updated_at=now()
          WHERE singleton=TRUE",
    )
    .bind(next_spam)
    .bind(next_retention)
    .bind(next_purge)
    .bind(admin.0.user_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.security_policy.update",
        json!({"spamThreshold":next_spam,"retentionDays":next_retention,"trashAutoPurge":next_purge}),
    )
    .await;
    security_policy(State(state), admin).await
}

#[derive(Deserialize)]
pub struct QueueQuery {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Provider-native outbound queue inspection.
pub async fn queue(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<QueueQuery>,
) -> Result<Json<Value>, ApiError> {
    if !state.stalwart.enabled() {
        return Ok(Json(json!({"messages":[],"total":0,"providerAvailable":false})));
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 250);
    let mut args = json!({"position":0,"limit":limit,"calculateTotal":true});
    if let Some(text) = query.text.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        args["filter"] = json!({"text":text});
    }
    let result = state
        .stalwart
        .management_read("x:QueuedMessage/query", args)
        .await
        .map_err(provider_err)?;
    let ids = result
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let list = if ids.is_empty() {
        Vec::new()
    } else {
        state
            .stalwart
            .management_read("x:QueuedMessage/get", json!({"ids":ids}))
            .await
            .map_err(provider_err)?
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    Ok(Json(json!({
        "messages": list,
        "total": result.get("total").and_then(Value::as_u64).unwrap_or(0),
        "providerAvailable": true
    })))
}

pub async fn retry_queued_message(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if id.trim().is_empty() {
        return Err(ApiError::bad_request("Queue message id is required"));
    }
    let mut updates = serde_json::Map::new();
    updates.insert(id.clone(), json!({"nextRetry": chrono::Utc::now()}));
    let result = state
        .stalwart
        .management_write("x:QueuedMessage/set", json!({"update":updates}))
        .await
        .map_err(provider_err)?;
    if result
        .get("notUpdated")
        .and_then(|value| value.get(&id))
        .is_some()
    {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "mail_provider",
            "Queued message could not be retried",
        ));
    }
    audit::record(&state, Some(admin.0.user_id), "admin.queue.retry", json!({"id":id})).await;
    Ok(Json(json!({"ok":true})))
}

pub async fn cancel_queued_message(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .stalwart
        .management_write("x:QueuedMessage/set", json!({"destroy":[id.clone()]}))
        .await
        .map_err(provider_err)?;
    if result
        .get("notDestroyed")
        .and_then(|value| value.get(&id))
        .is_some()
    {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "mail_provider",
            "Queued message could not be cancelled",
        ));
    }
    audit::record(&state, Some(admin.0.user_id), "admin.queue.cancel", json!({"id":id})).await;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub struct ForwarderCreate {
    from: String,
    to: String,
    #[serde(default = "default_keep_copy")]
    keep_copy: bool,
}
fn default_keep_copy() -> bool { true }

fn forwarder_json(
    user_id: Uuid,
    from: String,
    enabled: bool,
    to: String,
    keep_copy: bool,
    verified_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Value {
    json!({
        "id":user_id,
        "from":from,
        "to":to,
        "enabled":enabled,
        "keepCopy":keep_copy,
        "verified":verified_at.is_some(),
        "verifiedAt":verified_at
    })
}

pub async fn forwarders(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(Uuid,String,bool,String,bool,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT f.mailbox_id,m.address::text,f.enabled,f.target_email::text,f.keep_copy,f.verified_at
           FROM mail_forwarding f
           JOIN mailboxes m ON m.id=f.mailbox_id AND m.deleted_at IS NULL
          WHERE f.target_email<>'' ORDER BY lower(m.address::text)",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let values = rows.into_iter().map(|(id,from,enabled,to,keep,verified)| {
        forwarder_json(id,from,enabled,to,keep,verified)
    }).collect::<Vec<_>>();
    Ok(Json(json!({"forwarders":values})))
}

async fn admin_forward_would_loop(
    state: &AppState,
    source: &str,
    target: &str,
) -> Result<bool, ApiError> {
    if source.eq_ignore_ascii_case(target) { return Ok(true); }
    let looped: bool = sqlx::query_scalar(
        "WITH RECURSIVE walk(address, depth) AS (
           SELECT lower($2::text), 0
           UNION ALL
           SELECT lower(f.target_email::text), walk.depth + 1
             FROM walk
             JOIN mailboxes m ON lower(m.address::text)=walk.address AND m.deleted_at IS NULL
             JOIN mail_forwarding f ON f.mailbox_id=m.id
            WHERE f.enabled AND f.verified_at IS NOT NULL AND f.target_email<>'' AND walk.depth < 20
         )
         SELECT EXISTS(SELECT 1 FROM walk WHERE address=lower($1::text) OR depth>=20)",
    )
    .bind(source)
    .bind(target)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(looped)
}

pub async fn create_forwarder(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<ForwarderCreate>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let source = body.from.trim().to_ascii_lowercase();
    let target = body.to.trim().to_ascii_lowercase();
    if !valid_email(&source) || !valid_email(&target) {
        return Err(ApiError::bad_request("Valid source and destination addresses are required"));
    }
    let mailbox: Option<(Uuid,Uuid)> = sqlx::query_as(
        "SELECT m.user_id,m.id
           FROM mailboxes m
           JOIN users u ON u.id=m.user_id AND u.status='active'
           JOIN organizations o ON o.id=m.organization_id AND o.status='active'
          WHERE m.deleted_at IS NULL AND m.status='active' AND lower(m.address::text)=lower($1)",
    )
    .bind(&source)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (user_id, mailbox_id) = mailbox.ok_or_else(|| ApiError::bad_request("Forwarding source must be an active mailbox"))?;
    if admin_forward_would_loop(&state, &source, &target).await? {
        return Err(ApiError::bad_request("This forwarding path would create a loop"));
    }
    let raw = email::random_token();
    let code = raw[..12].to_ascii_uppercase();
    let hash = hash_admin_token(&code);
    sqlx::query(
        "INSERT INTO mail_forwarding
           (user_id,mailbox_id,enabled,target_email,keep_copy,verified_at,verification_token_hash,verification_expires_at,verification_sent_at)
         VALUES($1,$2,false,$3,$4,NULL,$5,now()+interval '30 minutes',now())
         ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET enabled=false,target_email=EXCLUDED.target_email,
           keep_copy=EXCLUDED.keep_copy,verified_at=NULL,verification_token_hash=EXCLUDED.verification_token_hash,
           verification_expires_at=EXCLUDED.verification_expires_at,verification_sent_at=now(),updated_at=now()"
    )
    .bind(user_id).bind(mailbox_id).bind(&target).bind(body.keep_copy).bind(hash)
    .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    email::send_forwarding_verification(&state, &target, &code).await?;
    let sync = sync_admin_automation(&state,user_id,mailbox_id).await?;
    audit::record(&state,Some(admin.0.user_id),"admin.forwarder.create",json!({"from":source,"to":target,"mailbox_id":mailbox_id})).await;
    Ok((axum::http::StatusCode::CREATED, Json(json!({
        "forwarder":forwarder_json(mailbox_id,source,false,target,body.keep_copy,None),
        "verificationPending":true,
        "verificationCode":state.return_token_links.then_some(code),
        "sync":sync
    }))))
}

#[derive(Deserialize)]
pub struct ForwarderVerify { code: String }

pub async fn verify_admin_forwarder(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(mailbox_id): Path<Uuid>,
    Json(body): Json<ForwarderVerify>,
) -> Result<Json<Value>, ApiError> {
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL",
    ).bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
     .ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    let supplied=hash_admin_token(&body.code.trim().to_ascii_uppercase());
    let updated=sqlx::query(
        "UPDATE mail_forwarding SET enabled=true,verified_at=now(),verification_token_hash='',
            verification_expires_at=NULL,verification_sent_at=NULL,updated_at=now()
          WHERE mailbox_id=$1 AND verification_token_hash=$2 AND verification_expires_at>now()"
    ).bind(mailbox_id).bind(&supplied).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if updated.rows_affected()!=1 { return Err(ApiError::bad_request("Verification code is invalid or expired")); }
    let sync=sync_admin_automation(&state,user_id,mailbox_id).await?;
    audit::record(&state,Some(admin.0.user_id),"admin.forwarder.verify",json!({"user_id":user_id,"mailbox_id":mailbox_id})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

#[derive(Deserialize)]
pub struct ForwarderPatch { enabled: bool }

pub async fn update_forwarder(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(mailbox_id): Path<Uuid>,
    Json(body): Json<ForwarderPatch>,
) -> Result<Json<Value>, ApiError> {
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL",
    ).bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
     .ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    let updated=sqlx::query(
        "UPDATE mail_forwarding SET enabled=$2,updated_at=now()
          WHERE mailbox_id=$1 AND ($2=false OR verified_at IS NOT NULL)"
    ).bind(mailbox_id).bind(body.enabled).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if updated.rows_affected()!=1 { return Err(ApiError::bad_request("Forwarder must be verified before it can be enabled")); }
    let sync=sync_admin_automation(&state,user_id,mailbox_id).await?;
    audit::record(&state,Some(admin.0.user_id),"admin.forwarder.status",json!({"user_id":user_id,"mailbox_id":mailbox_id,"enabled":body.enabled})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

pub async fn delete_forwarder(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(mailbox_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL",
    ).bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
     .ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    let deleted=sqlx::query("DELETE FROM mail_forwarding WHERE mailbox_id=$1").bind(mailbox_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if deleted.rows_affected()==0 { return Err(ApiError::not_found("Forwarder not found")); }
    let sync=sync_admin_automation(&state,user_id,mailbox_id).await?;
    audit::record(&state,Some(admin.0.user_id),"admin.forwarder.delete",json!({"user_id":user_id,"mailbox_id":mailbox_id})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

#[derive(Deserialize)]
pub struct QuarantineQuery {
    #[serde(default)]
    limit: Option<usize>,
}

fn first_address(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("email"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Cross-account Junk view. Work is bounded by both mailbox count and message
/// count so an admin request cannot fan out without limit.
pub async fn quarantine(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<QuarantineQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit=query.limit.unwrap_or(100).clamp(1,200);
    let accounts: Vec<(Uuid,String,String)> = sqlx::query_as(
        "SELECT m.id,m.address::text,m.provider_account_id
         FROM mailboxes m
         JOIN users u ON u.id=m.user_id AND u.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=m.user_id AND om.status='active'
         WHERE m.deleted_at IS NULL AND m.status='active'
           AND m.provider_account_id IS NOT NULL AND m.provider_account_id<>''
         ORDER BY m.created_at LIMIT 100"
    ).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let mut messages=Vec::new();
    for (mailbox_id,email,account) in accounts {
        if messages.len()>=limit { break; }
        let Some(junk)=imap::junk_mailbox_id(&state.stalwart,&account).await.map_err(ApiError::internal)? else { continue; };
        let remaining=limit-messages.len();
        let result=imap::thread_previews(&state.stalwart,&account,"mailbox",Some(&junk),remaining.min(20),None,None,"newest",false,false,false)
            .await.map_err(ApiError::internal)?;
        for row in result.get("emails").and_then(Value::as_array).into_iter().flatten() {
            let email_id=row.get("id").and_then(Value::as_str).unwrap_or("");
            if email_id.is_empty() { continue; }
            messages.push(json!({
                "id":format!("{}:{}",mailbox_id,email_id),
                "emailId":email_id,
                "mailboxId":mailbox_id,
                "account":account,
                "from":first_address(row.get("from")),
                "to":email,
                "subject":row.get("subject").and_then(Value::as_str).unwrap_or("(No subject)"),
                "date":row.get("received_at").or_else(||row.get("date")).cloned().unwrap_or(Value::Null),
                "reason":"Junk Mail",
                "sizeKB":((row.get("size").and_then(Value::as_u64).unwrap_or(0)+1023)/1024)
            }));
            if messages.len()>=limit { break; }
        }
    }
    Ok(Json(json!({"messages":messages,"limited":messages.len()>=limit})))
}

fn quarantine_key(value:&str)->Result<(Uuid,String),ApiError>{
    let (mailbox,email)=value.split_once(':').ok_or_else(||ApiError::bad_request("Invalid quarantine id"))?;
    let mailbox_id=Uuid::parse_str(mailbox).map_err(|_|ApiError::bad_request("Invalid quarantine id"))?;
    if email.is_empty(){return Err(ApiError::bad_request("Invalid quarantine id"));}
    Ok((mailbox_id,email.to_string()))
}

async fn quarantine_account(state:&AppState,mailbox_id:Uuid)->Result<String,ApiError>{
    let row:Option<(String,)> = sqlx::query_as(
        "SELECT m.provider_account_id
         FROM mailboxes m
         JOIN users u ON u.id=m.user_id AND u.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=m.user_id AND om.status='active'
         WHERE m.id=$1 AND m.deleted_at IS NULL AND m.status='active'
           AND m.provider_account_id IS NOT NULL AND m.provider_account_id<>''"
    )
        .bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    row.map(|v|v.0).ok_or_else(||ApiError::not_found("Mailbox not found"))
}

pub async fn release_quarantine(
    State(state): State<AppState>, admin:AdminUser, Path(id):Path<String>
)->Result<Json<Value>,ApiError>{
    let (mailbox_id,email_id)=quarantine_key(&id)?;
    let account=quarantine_account(&state,mailbox_id).await?;
    let junk=imap::junk_mailbox_id(&state.stalwart,&account).await.map_err(ApiError::internal)?.ok_or_else(||ApiError::not_found("Junk mailbox not found"))?;
    let inbox=imap::inbox_mailbox_id(&state.stalwart,&account).await.map_err(ApiError::internal)?.ok_or_else(||ApiError::not_found("Inbox not found"))?;
    imap::move_emails(&state.stalwart,&account,&[email_id.clone()],Some(&junk),None,&inbox).await.map_err(ApiError::internal)?;
    audit::record(&state,Some(admin.0.user_id),"admin.quarantine.release",json!({"mailbox_id":mailbox_id,"email_id":email_id})).await;
    Ok(Json(json!({"ok":true})))
}

pub async fn delete_quarantine(
    State(state): State<AppState>, admin:AdminUser, Path(id):Path<String>
)->Result<Json<Value>,ApiError>{
    let (mailbox_id,email_id)=quarantine_key(&id)?;
    let account=quarantine_account(&state,mailbox_id).await?;
    imap::destroy(&state.stalwart,&account,&[email_id.clone()]).await.map_err(ApiError::internal)?;
    audit::record(&state,Some(admin.0.user_id),"admin.quarantine.delete",json!({"mailbox_id":mailbox_id,"email_id":email_id})).await;
    Ok(Json(json!({"ok":true})))
}

pub async fn launch_certifications(
    State(state): State<AppState>, _admin: AdminUser
) -> Result<Json<Value>, ApiError> {
    type CertificationRow = (
        Uuid, String, String, String, String, String, String, i32, i32, i32,
        chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
    );
    let rows: Vec<CertificationRow> = sqlx::query_as(
        "SELECT id,release_label,release_sha256,environment,status,report_sha256,report_path,
                mandatory_passed,mandatory_failed,optional_skipped,started_at,completed_at,created_at
         FROM launch_certification_runs ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "runs": rows.into_iter().map(|r| json!({
            "id": r.0,
            "release_label": r.1,
            "release_sha256": r.2,
            "environment": r.3,
            "status": r.4,
            "report_sha256": r.5,
            "report_path": r.6,
            "mandatory_passed": r.7,
            "mandatory_failed": r.8,
            "optional_skipped": r.9,
            "started_at": r.10,
            "completed_at": r.11,
            "created_at": r.12,
        })).collect::<Vec<_>>()
    })))
}


pub async fn launch_readiness(
    State(state): State<AppState>, _admin: AdminUser
) -> Result<Json<Value>, ApiError> {
    // Reconcile clock-driven billing state before evaluating readiness so an
    // expired subscription cannot hide behind the worker cadence.
    billing::reconcile_subscription_lifecycle(&state).await?;

    let controls = platform_control::load(&state.db).await?;
    let provider_healthy = state.stalwart.enabled() && state.stalwart.healthcheck().await.is_ok();
    let migration_head: Option<i64> = sqlx::query_scalar(
        "SELECT max(version)::bigint FROM _sqlx_migrations WHERE success=TRUE",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(None);

    let operational: (i64,i64,i64,i64,i64,i64,i64,i64,i64) = sqlx::query_as(
        "SELECT
          (SELECT count(*)::bigint FROM provisioning_jobs WHERE status='dead'),
          (SELECT count(*)::bigint FROM billing_lifecycle_outbox WHERE status='failed'),
          (SELECT count(*)::bigint FROM billing_email_outbox WHERE status='failed'),
          (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status='failed'),
          (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status IN('queued','processing')),
          (SELECT count(*)::bigint FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
             WHERE o.is_system=FALSE AND m.deleted_at IS NULL AND m.status IN('active','provisioning')
               AND (m.provider_reconciled_at IS NULL OR m.provider_reconciled_at < now()-interval '30 minutes')),
          (SELECT count(*)::bigint FROM organization_subscriptions s JOIN organizations o ON o.id=s.organization_id
             WHERE o.is_system=FALSE AND s.status IN('active','trial','past_due') AND s.plan_version_id IS NULL),
          (SELECT count(*)::bigint FROM organization_subscriptions s
             JOIN organizations o ON o.id=s.organization_id
             JOIN plans p ON p.code=s.plan_code
             LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
             WHERE o.is_system=FALSE AND s.status IN('active','trial','past_due')
               AND COALESCE((SELECT sum(m.quota_bytes) FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL),0) >
                   COALESCE(s.storage_pool_override_bytes,
                     COALESCE(pv.storage_pool_bytes,p.storage_pool_bytes)::bigint +
                     COALESCE(pv.mailbox_bytes,p.mailbox_bytes)::bigint*GREATEST(s.purchased_mailbox_count-COALESCE(pv.mailbox_limit,p.mailbox_limit),0)::bigint)),
          (SELECT count(*)::bigint FROM orders ord JOIN organizations o ON o.id=ord.organization_id
             WHERE o.is_system=FALSE AND ord.status='submitted' AND ord.invoice_status='issued'
               AND ord.updated_at < now()-interval '48 hours')"
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let release_hash_valid = state.release_sha256.len() == 64
        && state.release_sha256.chars().all(|c| c.is_ascii_hexdigit());
    let latest_cert: Option<(String,chrono::DateTime<chrono::Utc>,i32,i32,String)> = if release_hash_valid {
        sqlx::query_as(
            "SELECT status,COALESCE(completed_at,created_at),mandatory_passed,mandatory_failed,release_sha256
               FROM launch_certification_runs
              WHERE environment='production' AND release_sha256=$1
              ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&state.release_sha256)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    } else { None };

    let evidence_rows: Vec<(String,chrono::DateTime<chrono::Utc>,String)> = sqlx::query_as(
        "SELECT DISTINCT ON (kind) kind,recorded_at,release_sha256
           FROM operational_evidence WHERE status='passed'
          ORDER BY kind,recorded_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let evidence: HashMap<String,(chrono::DateTime<chrono::Utc>,String)> = evidence_rows
        .into_iter().map(|(kind,at,release)| (kind,(at,release))).collect();

    let mut blockers: Vec<Value> = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut push_blocker = |code: &str, message: String| blockers.push(json!({"code":code,"message":message}));
    let mut push_warning = |code: &str, message: String| warnings.push(json!({"code":code,"message":message}));

    if state.billing_instant_activation {
        push_blocker("instant_activation", "Test instant plan activation is enabled. Public billing must require verified payment approval.".into());
    }
    if !provider_healthy {
        push_blocker("mail_provider", "The configured mail provider is unavailable or unhealthy.".into());
    }
    if migration_head.unwrap_or_default() < 48 {
        push_blocker("migration_head", format!("Database migration head is {}, expected at least 48.", migration_head.unwrap_or_default()));
    }
    if state.environment != "production" {
        push_blocker("runtime_profile", format!("Runtime profile is '{}', expected production.", state.environment));
    }
    if !release_hash_valid {
        push_blocker("release_identity", "The running API has no valid immutable release SHA-256. Deploy through the guarded production pipeline before launch.".into());
    }
    for (code,count,label) in [
        ("provisioning_dead",operational.0,"dead provider provisioning jobs"),
        ("lifecycle_email_failed",operational.1,"failed billing lifecycle notifications"),
        ("invoice_email_failed",operational.2,"failed invoice email jobs"),
        ("purge_failed",operational.3,"failed retained-data purge runs"),
        ("plan_version_missing",operational.6,"active subscriptions without immutable plan versions"),
        ("storage_overallocated",operational.7,"subscriptions whose mailbox allocations exceed their storage pool"),
    ] {
        if count > 0 { push_blocker(code, format!("{count} {label} require operator action.")); }
    }
    if operational.4 > 0 { push_warning("purge_running", format!("{} retained-data purge run(s) are still in progress.", operational.4)); }
    if operational.5 > 0 { push_warning("provider_stale", format!("{} mailbox(es) have stale provider reconciliation.", operational.5)); }
    if operational.8 > 0 { push_warning("payment_review_aging", format!("{} submitted manual-payment order(s) have waited more than 48 hours for review.", operational.8)); }

    match &latest_cert {
        Some((status,at,_,failed,_)) if status == "passed" && *failed == 0 => {
            if *at < chrono::Utc::now() - chrono::Duration::hours(24) {
                push_warning("certification_stale", "The passing certification for this exact release is older than 24 hours. Re-run it immediately before launch.".into());
            }
        }
        Some((status,_,_,_,_)) => push_blocker("certification", format!("Certification for the currently deployed release is '{status}', not passed.")),
        None if release_hash_valid => push_blocker("certification_missing", "No production launch certification has been recorded for the currently deployed release SHA-256.".into()),
        None => {}
    }

    let now = chrono::Utc::now();
    for (kind,max_age,label,require_release_match) in [
        ("local_backup",26_i64,"local CS Mail database + attachment backup",true),
        ("restore_drill",168_i64,"isolated CS Mail restore drill",true),
        ("cs_mail_offsite_backup",26_i64,"encrypted offsite CS Mail backup proof",false),
        ("stalwart_offsite_backup",26_i64,"encrypted offsite shared Stalwart backup proof",false),
    ] {
        match evidence.get(kind) {
            Some((at,release)) => {
                let age_hours=(now-*at).num_minutes().max(0) as f64/60.0;
                if age_hours > max_age as f64 {
                    push_blocker(&format!("{kind}_stale"), format!("Latest {label} evidence is {:.1} hours old; maximum is {max_age} hours.", age_hours));
                } else if require_release_match && release_hash_valid && release != &state.release_sha256 {
                    push_blocker(&format!("{kind}_release"), format!("Latest {label} evidence belongs to a different release. Re-run it after deploying the current release."));
                }
            }
            None => push_blocker(&format!("{kind}_missing"), format!("No successful {label} evidence has been recorded.")),
        }
    }

    let controls_json = json!({
        "publicSignup":controls.public_signup_enabled,
        "businessCreation":controls.business_creation_enabled,
        "planOrdering":controls.plan_ordering_enabled,
        "domainOnboarding":controls.domain_onboarding_enabled,
        "mailboxProvisioning":controls.mailbox_provisioning_enabled,
        "outboundSending":controls.outbound_sending_enabled,
    });
    let all_public_controls = controls.public_signup_enabled && controls.business_creation_enabled
        && controls.plan_ordering_enabled && controls.domain_onboarding_enabled
        && controls.mailbox_provisioning_enabled && controls.outbound_sending_enabled;
    if !all_public_controls {
        push_warning("platform_controls", "One or more public platform controls are paused. This is safe before launch, but confirm the intended switches when opening service.".into());
    }

    let status = if !blockers.is_empty() { "blocked" } else if !warnings.is_empty() { "attention" } else { "ready" };
    Ok(Json(json!({
        "status":status,
        "evaluatedAt":chrono::Utc::now(),
        "blockers":blockers,
        "warnings":warnings,
        "checks":{
            "runtimeProfile":state.environment,
            "releaseSha256":state.release_sha256,
            "mailProviderHealthy":provider_healthy,
            "billingInstantActivation":state.billing_instant_activation,
            "migrationHead":migration_head,
            "platformControls":controls_json,
            "operational":{
                "provisioningDead":operational.0,
                "lifecycleEmailFailed":operational.1,
                "invoiceEmailFailed":operational.2,
                "purgeFailed":operational.3,
                "purgeRunning":operational.4,
                "providerStaleMailboxes":operational.5,
                "subscriptionsMissingPlanVersion":operational.6,
                "storageOverallocated":operational.7,
                "agingPaymentReviews":operational.8
            },
            "latestCertification": latest_cert.map(|r| json!({"status":r.0,"at":r.1,"mandatoryPassed":r.2,"mandatoryFailed":r.3,"releaseSha256":r.4})),
            "operationalEvidence": {
                "localBackup": evidence.get("local_backup").map(|r| json!({"at":&r.0,"releaseSha256":r.1.as_str()})),
                "restoreDrill": evidence.get("restore_drill").map(|r| json!({"at":&r.0,"releaseSha256":r.1.as_str()})),
                "csMailOffsiteBackup": evidence.get("cs_mail_offsite_backup").map(|r| json!({"at":&r.0,"releaseSha256":r.1.as_str()})),
                "stalwartOffsiteBackup": evidence.get("stalwart_offsite_backup").map(|r| json!({"at":&r.0,"releaseSha256":r.1.as_str()}))
            }
        }
    })))
}

pub async fn diagnostics(
    State(state):State<AppState>, _admin:AdminUser
)->Result<Json<Value>,ApiError>{
    let database=sqlx::query_scalar::<_,i32>("SELECT 1").fetch_one(&state.db).await.is_ok();
    let provider=if state.stalwart.enabled(){state.stalwart.healthcheck().await.is_ok()}else{false};
    // Diagnostics should describe the state an operator would act on now, not
    // a lifecycle snapshot that may be waiting for the next maintenance tick.
    billing::reconcile_subscription_lifecycle(&state).await?;
    let provisioning:Vec<(String,i64)>=sqlx::query_as("SELECT status,count(*) FROM provisioning_jobs GROUP BY status ORDER BY status")
        .fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let automation_errors:i64=sqlx::query_scalar("SELECT count(*) FROM mail_automation_state WHERE status='error'")
        .fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let address_errors:i64=sqlx::query_scalar("SELECT count(*) FROM address_sync_state WHERE status='error'")
        .fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let billing_operations:(i64,i64,i64,i64,i64,i64)=sqlx::query_as(
        "SELECT
           (SELECT count(*)::bigint FROM organization_subscriptions s JOIN organizations o ON o.id=s.organization_id
             WHERE o.is_system=FALSE AND s.status IN('suspended','cancelled') AND s.purge_eligible_at IS NOT NULL
               AND s.purge_eligible_at<=now() AND s.data_purged_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM subscription_purge_runs pr WHERE pr.organization_id=s.organization_id AND pr.status IN('queued','processing','failed')))::bigint,
           (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status IN('queued','processing'))::bigint,
           (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status='failed')::bigint,
           (SELECT count(*)::bigint FROM provisioning_jobs WHERE status='dead')::bigint,
           (SELECT count(*)::bigint FROM billing_lifecycle_outbox WHERE status='failed')::bigint,
           (SELECT count(*)::bigint FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
             WHERE o.is_system=FALSE AND m.deleted_at IS NULL
               AND (m.provider_reconciled_at IS NULL OR m.provider_reconciled_at<now()-interval '30 minutes'))::bigint"
    ).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let queue_total=if provider {
        state.stalwart.management_read("x:QueuedMessage/query",json!({"position":0,"limit":1,"calculateTotal":true})).await
            .ok().and_then(|v|v.get("total").and_then(Value::as_u64)).unwrap_or(0)
    }else{0};
    Ok(Json(json!({
        "database":database,"mailProvider":provider,"queueTotal":queue_total,
        "automationErrors":automation_errors,"addressSyncErrors":address_errors,
        "billingOperations":{
            "purgeEligible":billing_operations.0,
            "purgeRunning":billing_operations.1,
            "purgeFailed":billing_operations.2,
            "provisioningDead":billing_operations.3,
            "lifecycleEmailFailed":billing_operations.4,
            "providerStaleMailboxes":billing_operations.5
        },
        "provisioning":provisioning.into_iter().map(|(status,count)|json!({"status":status,"count":count})).collect::<Vec<_>>()
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_quotes_only_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }
}
