//! Organization-authoritative entitlements (Upgrade 24).
//!
//! Plans remain global catalog rows, but effective commercial authority belongs
//! to `organization_subscriptions`. Legacy user plan/quota fields are retained
//! only for compatibility with older deployments and admin diagnostics.

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::domain::quota::PlanLimits;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct PlanRow {
    code: String,
    name: String,
    price_cents: i32,
    extra_mailbox_price_cents: i32,
    currency: String,
    interval: String,
    mailbox_bytes: i64,
    storage_pool_bytes: i64,
    mailbox_limit: i32,
    max_mailboxes: i32,
    alias_limit_per_mailbox: Option<i32>,
    domain_limit: i32,
    organization_daily_send_limit: i32,
    max_attachment_bytes: i64,
    max_recipients: i32,
    daily_send_limit: i32,
    seats: i32,
    features: Json<Vec<String>>,
    feature_flags: Json<Value>,
    active: bool,
}

impl PlanRow {
    fn into_limits(self) -> PlanLimits {
        let feature_flags = self
            .feature_flags
            .0
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| value.as_bool().map(|enabled| (key.clone(), enabled)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        PlanLimits {
            code: self.code,
            name: self.name,
            price_cents: self.price_cents as i64,
            extra_mailbox_price_cents: self.extra_mailbox_price_cents as i64,
            currency: self.currency,
            interval: self.interval,
            mailbox_bytes: self.mailbox_bytes.max(0) as u64,
            storage_pool_bytes: self.storage_pool_bytes.max(0) as u64,
            mailbox_limit: self.mailbox_limit,
            max_mailboxes: self.max_mailboxes,
            alias_limit_per_mailbox: self.alias_limit_per_mailbox,
            domain_limit: self.domain_limit,
            organization_daily_send_limit: self.organization_daily_send_limit.max(0) as i64,
            max_attachment_bytes: self.max_attachment_bytes.max(0) as usize,
            max_total_attachment_bytes: self.max_attachment_bytes.max(0) as usize,
            max_recipients: self.max_recipients.max(0) as usize,
            daily_send_limit: self.daily_send_limit.max(0) as i64,
            seats: self.seats,
            features: self.features.0,
            feature_flags,
            active: self.active,
        }
    }
}

pub const PLAN_COLUMNS: &str = "code, name, price_cents, extra_mailbox_price_cents, currency, interval, mailbox_bytes, \
     storage_pool_bytes, mailbox_limit, max_mailboxes, alias_limit_per_mailbox, domain_limit, organization_daily_send_limit, \
     max_attachment_bytes, max_recipients, daily_send_limit, seats, features, feature_flags, active";

#[derive(Clone, Debug)]
pub struct UserEntitlements {
    pub organization_id: Uuid,
    pub plan: PlanLimits,
    /// Effective per-mailbox provider quota.
    pub quota_bytes: u64,
    pub quota_override_bytes: Option<u64>,
    pub storage_pool_bytes: u64,
    pub seat_limit: i32,
    pub mailbox_limit: i32,
    pub domain_limit: i32,
    pub organization_daily_send_limit: i64,
    pub subscription_status: String,
}

impl UserEntitlements {
    pub fn quota_is_overridden(&self) -> bool { self.quota_override_bytes.is_some() }
    pub fn allows(&self, feature: &str) -> bool {
        matches!(self.subscription_status.as_str(), "active" | "trial")
            && self.plan.allows(feature)
    }
}

pub const FEATURE_KEYS: &[&str] = &[
    "mail", "attachments", "scheduled_send", "read_receipts", "contacts", "calendar",
    "mail_rules", "forwarding", "vacation",
];

pub fn default_feature_flags_map() -> BTreeMap<String, bool> {
    FEATURE_KEYS.iter().map(|key| ((*key).to_string(), true)).collect()
}

pub fn validate_feature_flags(flags: &BTreeMap<String, bool>) -> Result<(), ApiError> {
    if let Some(unknown) = flags.keys().find(|key| !FEATURE_KEYS.contains(&key.as_str())) {
        return Err(ApiError::bad_request(format!("Unknown entitlement feature flag '{unknown}'")));
    }
    Ok(())
}

pub async fn all_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    let rows: Vec<PlanRow> = sqlx::query_as(&format!(
        "SELECT {PLAN_COLUMNS} FROM plans ORDER BY sort_order, price_cents, code"
    )).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(PlanRow::into_limits).collect())
}

pub async fn active_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    let rows: Vec<PlanRow> = sqlx::query_as(&format!(
        "SELECT {PLAN_COLUMNS} FROM plans WHERE active ORDER BY sort_order, price_cents, code"
    )).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(PlanRow::into_limits).collect())
}

pub async fn plan(state: &AppState, code: &str) -> Result<PlanLimits, ApiError> {
    let row: Option<PlanRow> = sqlx::query_as(&format!("SELECT {PLAN_COLUMNS} FROM plans WHERE code = $1"))
        .bind(code).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    row.map(PlanRow::into_limits).ok_or_else(|| ApiError::bad_request(format!("Unknown plan '{code}'")))
}

pub async fn active_plan(state: &AppState, code: &str) -> Result<PlanLimits, ApiError> {
    let plan = plan(state, code).await?;
    if !plan.active { return Err(ApiError::bad_request("This plan is not available for new orders")); }
    Ok(plan)
}

pub async fn plan_exists(state: &AppState, code: &str) -> Result<bool, ApiError> {
    let found: Option<bool> = sqlx::query_scalar("SELECT TRUE FROM plans WHERE code = $1")
        .bind(code).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(found.unwrap_or(false))
}

/// Resolve the business that currently owns this request/session's commercial
/// authority. Upgrade 23 persists active_organization_id on switch, so older
/// handler signatures can safely migrate without trusting a client-supplied
/// organization UUID here.
pub async fn active_organization_for_user(state: &AppState, user_id: Uuid) -> Result<Uuid, ApiError> {
    let org: Option<Uuid> = sqlx::query_scalar(
        "SELECT COALESCE(u.active_organization_id, m.organization_id, owner_org.organization_id)
         FROM users u
         LEFT JOIN mailboxes m ON m.id=COALESCE(u.active_mailbox_id,u.primary_mailbox_id) AND m.deleted_at IS NULL
         LEFT JOIN LATERAL (
           SELECT om.organization_id FROM organization_memberships om
           JOIN organizations o ON o.id=om.organization_id AND o.status='active'
           WHERE om.user_id=u.id AND om.status='active'
           ORDER BY CASE om.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'billing' THEN 2 ELSE 3 END, om.joined_at
           LIMIT 1
         ) owner_org ON TRUE
         WHERE u.id=$1"
    ).bind(user_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?.flatten();
    org.ok_or_else(|| ApiError::forbidden("Select an active business before using this feature"))
}

pub async fn for_organization(state: &AppState, organization_id: Uuid) -> Result<UserEntitlements, ApiError> {
    let row: Option<(String,String,Option<i64>,Option<i64>,Option<i32>,Option<i32>,Option<i32>,Option<i32>,i32)> = sqlx::query_as(
        "SELECT s.plan_code,
                CASE
                  WHEN s.status IN ('active','trial') AND s.current_period_end IS NOT NULL AND s.current_period_end <= now() THEN 'past_due'
                  WHEN s.status='past_due' AND s.renewal_grace_end IS NOT NULL AND s.renewal_grace_end <= now() THEN 'suspended'
                  ELSE s.status
                END,
                s.mailbox_quota_override_bytes,s.storage_pool_override_bytes,
                s.seat_limit_override,s.mailbox_limit_override,s.domain_limit_override,
                s.organization_daily_send_override,s.purchased_mailbox_count
         FROM organization_subscriptions s JOIN organizations o ON o.id=s.organization_id
         WHERE s.organization_id=$1 AND o.status='active'"
    ).bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (code,status,mailbox_override,storage_override,seat_override,mailbox_limit_override,domain_override,org_send_override,purchased_mailboxes) =
        row.ok_or_else(|| ApiError::forbidden("Business subscription is not available"))?;
    let plan = plan(state, &code).await?;
    let purchased = purchased_mailboxes.max(plan.mailbox_limit);
    let scaled_storage = scaled_storage_pool(&plan, purchased);
    let scaled_send = plan.daily_send_limit.saturating_mul(purchased as i64);
    Ok(UserEntitlements {
        organization_id,
        quota_bytes: mailbox_override.unwrap_or(plan.mailbox_bytes as i64).max(0) as u64,
        quota_override_bytes: mailbox_override.map(|v| v.max(0) as u64),
        storage_pool_bytes: storage_override.unwrap_or(scaled_storage).max(0) as u64,
        seat_limit: seat_override.unwrap_or(purchased.max(plan.seats)),
        mailbox_limit: mailbox_limit_override.unwrap_or(purchased),
        domain_limit: domain_override.unwrap_or(plan.domain_limit),
        organization_daily_send_limit: org_send_override.map(|v| v as i64).unwrap_or(scaled_send),
        subscription_status: status,
        plan,
    })
}

pub async fn for_user(state: &AppState, user_id: Uuid) -> Result<UserEntitlements, ApiError> {
    let org = active_organization_for_user(state, user_id).await?;
    for_organization(state, org).await
}

pub async fn require_feature(state: &AppState, user_id: Uuid, feature: &str) -> Result<UserEntitlements, ApiError> {
    let entitlements = for_user(state, user_id).await?;
    if !matches!(entitlements.subscription_status.as_str(), "active" | "trial") {
        return Err(ApiError::forbidden("This business subscription is not active"));
    }
    if !entitlements.plan.allows(feature) {
        return Err(ApiError::forbidden(format!("The {feature} feature is not included in this business plan")));
    }
    Ok(entitlements)
}

pub fn effective_quota(plan: &PlanLimits, quota_override: Option<i64>) -> u64 {
    quota_override.filter(|value| *value > 0).map(|value| value as u64).unwrap_or(plan.mailbox_bytes)
}

/// Aggregate storage purchased for a plan quantity. `storage_pool_bytes` is
/// the base bundle pool; each mailbox above the included quantity contributes
/// one additional per-mailbox quota to the organization pool.
pub fn scaled_storage_pool(plan: &PlanLimits, mailbox_count: i32) -> i64 {
    let purchased = mailbox_count.max(plan.mailbox_limit).max(0) as i64;
    let included = plan.mailbox_limit.max(0) as i64;
    let extra = purchased.saturating_sub(included);
    let base = plan.storage_pool_bytes.min(i64::MAX as u64) as i64;
    let per_mailbox = plan.mailbox_bytes.min(i64::MAX as u64) as i64;
    base.saturating_add(per_mailbox.saturating_mul(extra))
}

/// Materialize the effective quota on every live mailbox in an organization.
pub async fn materialize_organization_quota_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, organization_id: Uuid,
) -> Result<i64, ApiError> {
    let effective: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(s.mailbox_quota_override_bytes,p.mailbox_bytes)::bigint
         FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
         WHERE s.organization_id=$1"
    ).bind(organization_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let effective = effective.ok_or_else(|| ApiError::not_found("Business subscription not found"))?;
    sqlx::query("UPDATE mailboxes SET quota_bytes=$1,updated_at=now() WHERE organization_id=$2 AND deleted_at IS NULL AND quota_override_bytes IS NULL")
        .bind(effective).bind(organization_id).execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(effective)
}

/// Reserve the default storage allocation for a newly created mailbox. The
/// subscription row lock serializes this with business-admin quota edits and
/// the database trigger remains the final concurrency-safe pool guard.
pub async fn default_mailbox_allocation_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, organization_id: Uuid,
) -> Result<i64, ApiError> {
    let row: Option<(i64,i64)> = sqlx::query_as(
        "SELECT COALESCE(s.mailbox_quota_override_bytes,p.mailbox_bytes)::bigint,
                COALESCE(s.storage_pool_override_bytes,
                         p.storage_pool_bytes::bigint +
                         p.mailbox_bytes::bigint * GREATEST(s.purchased_mailbox_count-p.mailbox_limit,0)::bigint)::bigint
         FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
         WHERE s.organization_id=$1 FOR UPDATE OF s"
    ).bind(organization_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (default_quota,pool_bytes)=row.ok_or_else(|| ApiError::forbidden("Business subscription is not available"))?;
    let allocated: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quota_bytes),0)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL"
    ).bind(organization_id).fetch_one(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let default_quota=default_quota.max(1_048_576);
    let remaining=pool_bytes.saturating_sub(allocated);
    if remaining < default_quota {
        return Err(ApiError::conflict(format!(
            "A new mailbox requires the default allocation of {default_quota} bytes, but only {remaining} bytes remain in the business storage pool. Rebalance mailbox storage first"
        )));
    }
    Ok(default_quota)
}

/// Legacy helper retained for old admin code. It now materializes the selected
/// user's active organization and never writes user quota as authority.
pub async fn materialize_user_quota_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid,
) -> Result<i64, ApiError> {
    let organization_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT COALESCE(u.active_organization_id,m.organization_id) FROM users u
         LEFT JOIN mailboxes m ON m.id=COALESCE(u.active_mailbox_id,u.primary_mailbox_id)
         WHERE u.id=$1"
    ).bind(user_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?.flatten();
    let org = organization_id.ok_or_else(|| ApiError::not_found("Business not found"))?;
    materialize_organization_quota_tx(tx, org).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_plan() -> PlanLimits {
        PlanLimits {
            code:"test".into(), name:"Test".into(), price_cents:0, extra_mailbox_price_cents:0, currency:"USD".into(), interval:"month".into(),
            mailbox_bytes:5*1024*1024*1024, storage_pool_bytes:20*1024*1024*1024, mailbox_limit:4, max_mailboxes:20, alias_limit_per_mailbox:Some(10),
            domain_limit:2, organization_daily_send_limit:1000, max_attachment_bytes:1024,
            max_total_attachment_bytes:1024, max_recipients:10, daily_send_limit:300, seats:4,
            features:vec![], feature_flags:default_feature_flags_map(), active:true,
        }
    }
    #[test] fn override_wins() { let p=test_plan(); assert_eq!(effective_quota(&p,Some(42)),42); }
    #[test] fn plan_quota_is_default() { let p=test_plan(); assert_eq!(effective_quota(&p,None),p.mailbox_bytes); }
}

#[derive(Clone, Copy, Debug)]
pub enum CapacityKind { Seat, Mailbox, Domain }

/// Reserve/check organization capacity under a subscription row lock. The
/// caller may perform its insert immediately after this returns; invitation
/// handlers use the same transaction pattern directly when atomic reservation
/// is required.
pub async fn require_capacity(
    state: &AppState,
    organization_id: Uuid,
    kind: CapacityKind,
    additional: i64,
) -> Result<(), ApiError> {
    let ent = for_organization(state, organization_id).await?;
    let (used, limit, label): (i64, i64, &str) = match kind {
        CapacityKind::Seat => {
            let n: i64 = sqlx::query_scalar(
                "SELECT (SELECT count(*) FROM organization_memberships WHERE organization_id=$1 AND status IN ('active','invited')) +
                        (SELECT count(*) FROM organization_invitations WHERE organization_id=$1 AND status='pending')"
            ).bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
            (n, ent.seat_limit as i64, "seat")
        }
        CapacityKind::Mailbox => {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status <> 'deleted'")
                .bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
            (n, ent.mailbox_limit as i64, "mailbox")
        }
        CapacityKind::Domain => {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM organization_domains WHERE organization_id=$1 AND status <> 'removing'")
                .bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
            (n, ent.domain_limit as i64, "domain")
        }
    };
    if used.saturating_add(additional) > limit {
        return Err(ApiError::forbidden(format!("This business has reached its {label} limit ({limit})")));
    }
    Ok(())
}

pub async fn validate_plan_change_capacity(
    state: &AppState,
    organization_id: Uuid,
    plan_code: &str,
) -> Result<(), ApiError> {
    let p = plan(state, plan_code).await?;
    validate_plan_change_capacity_with_mailboxes(state, organization_id, plan_code, p.mailbox_limit).await
}

pub async fn validate_plan_change_capacity_with_mailboxes(
    state: &AppState,
    organization_id: Uuid,
    plan_code: &str,
    mailbox_count: i32,
) -> Result<(), ApiError> {
    let p = plan(state, plan_code).await?;
    if mailbox_count < p.mailbox_limit || mailbox_count > p.max_mailboxes {
        return Err(ApiError::bad_request(format!(
            "Mailbox quantity must be between {} and {} for {}", p.mailbox_limit, p.max_mailboxes, p.name
        )));
    }
    let counts: (i64,i64,i64,i64,i64) = sqlx::query_as(
        "SELECT
          ((SELECT count(*) FROM organization_memberships WHERE organization_id=$1 AND status IN ('active','invited')) +
           (SELECT count(*) FROM organization_invitations WHERE organization_id=$1 AND status='pending'))::bigint,
          (SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status <> 'deleted'),
          (SELECT count(*)::bigint FROM organization_domains WHERE organization_id=$1 AND status <> 'removing'),
          COALESCE((SELECT storage_bytes FROM organization_usage WHERE organization_id=$1),0)::bigint,
          COALESCE((SELECT SUM(quota_bytes) FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL),0)::bigint"
    ).bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let effective_storage = scaled_storage_pool(&p, mailbox_count);
    if counts.0 > mailbox_count as i64 { return Err(ApiError::conflict("Choose enough mailboxes for the business members and pending invitations currently in use")); }
    if counts.1 > mailbox_count as i64 { return Err(ApiError::conflict("Choose enough mailboxes for the hosted mailboxes currently in use")); }
    if counts.2 > p.domain_limit as i64 { return Err(ApiError::conflict("The target plan has fewer domains than this business currently uses")); }
    if counts.3 > effective_storage { return Err(ApiError::conflict("The selected mailbox quantity provides less storage than this business currently uses")); }
    if counts.4 > effective_storage { return Err(ApiError::conflict("The selected plan provides less pooled storage than the business currently allocates to its mailboxes. Rebalance mailbox storage before changing plan")); }
    Ok(())
}

