//! Billing data access (WS4): admin-editable plans, out-of-band payment orders,
//! and the singleton payment-instruction settings. All SQL for the billing
//! surface lives here; handlers stay thin.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::domain::quota::PlanLimits;
use crate::error::ApiError;
use crate::services::entitlements;
use crate::state::AppState;

/// Billing delegates every plan/entitlement read to the authoritative
/// entitlement service. These wrappers preserve the existing handler surface
/// while keeping one source of truth.
pub async fn all_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    entitlements::all_plans(state).await
}

pub async fn active_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    entitlements::active_plans(state).await
}

pub async fn for_code(state: &AppState, code: &str) -> Result<PlanLimits, ApiError> {
    entitlements::plan(state, code).await
}

pub async fn for_user(state: &AppState, user_id: Uuid) -> Result<PlanLimits, ApiError> {
    Ok(entitlements::for_user(state, user_id).await?.plan)
}

pub async fn plan_exists(state: &AppState, code: &str) -> Result<bool, ApiError> {
    entitlements::plan_exists(state, code).await
}


async fn publish_plan_version_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan_code: &str,
) -> Result<Uuid, ApiError> {
    // Serialize publication for one catalog plan so version_no cannot race.
    sqlx::query("SELECT code FROM plans WHERE code=$1 FOR UPDATE")
        .bind(plan_code)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plan_versions(plan_code,version_no,name,price_cents,extra_mailbox_price_cents,currency,interval,
           mailbox_bytes,storage_pool_bytes,mailbox_limit,max_mailboxes,alias_limit_per_mailbox,domain_limit,
           organization_daily_send_limit,max_attachment_bytes,max_recipients,daily_send_limit,seats,features,feature_flags)
         SELECT p.code,COALESCE((SELECT MAX(pv.version_no) FROM plan_versions pv WHERE pv.plan_code=p.code),0)+1,
           p.name,p.price_cents,p.extra_mailbox_price_cents,p.currency,p.interval,
           p.mailbox_bytes,p.storage_pool_bytes,p.mailbox_limit,p.max_mailboxes,p.alias_limit_per_mailbox,p.domain_limit,
           p.organization_daily_send_limit,p.max_attachment_bytes,p.max_recipients,p.daily_send_limit,p.seats,p.features,p.feature_flags
         FROM plans p WHERE p.code=$1
         RETURNING id"
    )
    .bind(plan_code)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query("UPDATE plans SET current_version_id=$2,updated_at=now() WHERE code=$1")
        .bind(plan_code)
        .bind(version_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(version_id)
}

async fn publish_plan_version(state: &AppState, plan_code: &str) -> Result<Uuid, ApiError> {
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let id=publish_plan_version_tx(&mut tx,plan_code).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(id)
}

/// Validated fields for creating or updating a plan.
pub struct PlanInput {
    pub code: String,
    pub name: String,
    pub price_cents: i64,
    pub extra_mailbox_price_cents: i64,
    pub currency: String,
    pub interval: String,
    pub mailbox_bytes: i64,
    pub storage_pool_bytes: i64,
    pub mailbox_limit: i64,
    pub max_mailboxes: i64,
    pub alias_limit_per_mailbox: Option<i64>,
    pub domain_limit: i64,
    pub organization_daily_send_limit: i64,
    pub max_attachment_bytes: i64,
    pub max_recipients: i64,
    pub daily_send_limit: i64,
    pub seats: i64,
    pub features: Vec<String>,
    pub feature_flags: Option<BTreeMap<String, bool>>,
    pub sort_order: i64,
    pub active: bool,
}

pub async fn create_plan(state: &AppState, p: &PlanInput) -> Result<(), ApiError> {
    if let Some(flags) = p.feature_flags.as_ref() {
        entitlements::validate_feature_flags(flags)?;
    }
    if plan_exists(state, &p.code).await? {
        return Err(ApiError::conflict(format!(
            "A plan with code '{}' already exists",
            p.code
        )));
    }
    sqlx::query(
        "INSERT INTO plans
            (code, name, price_cents, extra_mailbox_price_cents, currency, interval, mailbox_bytes, storage_pool_bytes, mailbox_limit, max_mailboxes, alias_limit_per_mailbox, domain_limit, organization_daily_send_limit, max_attachment_bytes,
             max_recipients, daily_send_limit, seats, features, sort_order, active, feature_flags)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
    )
    .bind(&p.code)
    .bind(&p.name)
    .bind(p.price_cents as i32)
    .bind(p.extra_mailbox_price_cents as i32)
    .bind(&p.currency)
    .bind(&p.interval)
    .bind(p.mailbox_bytes)
    .bind(p.storage_pool_bytes)
    .bind(p.mailbox_limit as i32)
    .bind(p.max_mailboxes as i32)
    .bind(p.alias_limit_per_mailbox.map(|v| v as i32))
    .bind(p.domain_limit as i32)
    .bind(p.organization_daily_send_limit as i32)
    .bind(p.max_attachment_bytes)
    .bind(p.max_recipients as i32)
    .bind(p.daily_send_limit as i32)
    .bind(p.seats as i32)
    .bind(Json(&p.features))
    .bind(p.sort_order as i32)
    .bind(p.active)
    .bind(Json(
        p.feature_flags
            .clone()
            .unwrap_or_else(entitlements::default_feature_flags_map),
    ))
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    publish_plan_version(state, &p.code).await?;
    Ok(())
}

pub async fn update_plan(state: &AppState, code: &str, p: &PlanInput) -> Result<(), ApiError> {
    if let Some(flags) = p.feature_flags.as_ref() {
        entitlements::validate_feature_flags(flags)?;
    }
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let previous: Option<Json<BTreeMap<String, bool>>> = sqlx::query_scalar(
        "SELECT feature_flags FROM plans WHERE code = $1 FOR UPDATE",
    )
    .bind(code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let previous_feature_flags = previous.ok_or_else(|| ApiError::not_found("Plan not found"))?;
    let feature_flags = p.feature_flags.clone().unwrap_or(previous_feature_flags.0);

    // Existing subscriptions are version-pinned, so editing the public catalog
    // never mutates or invalidates already-purchased entitlement snapshots.

    sqlx::query(
        "UPDATE plans SET
            name = $2, price_cents = $3, extra_mailbox_price_cents = $4, currency = $5, interval = $6,
            mailbox_bytes = $7, storage_pool_bytes = $8, mailbox_limit = $9,
            max_mailboxes = $10, alias_limit_per_mailbox = $11, domain_limit = $12, organization_daily_send_limit = $13,
            max_attachment_bytes = $14, max_recipients = $15,
            daily_send_limit = $16, seats = $17, features = $18, sort_order = $19,
            active = $20, feature_flags = $21, updated_at = now()
         WHERE code = $1",
    )
    .bind(code)
    .bind(&p.name)
    .bind(p.price_cents as i32)
    .bind(p.extra_mailbox_price_cents as i32)
    .bind(&p.currency)
    .bind(&p.interval)
    .bind(p.mailbox_bytes)
    .bind(p.storage_pool_bytes)
    .bind(p.mailbox_limit as i32)
    .bind(p.max_mailboxes as i32)
    .bind(p.alias_limit_per_mailbox.map(|v| v as i32))
    .bind(p.domain_limit as i32)
    .bind(p.organization_daily_send_limit as i32)
    .bind(p.max_attachment_bytes)
    .bind(p.max_recipients as i32)
    .bind(p.daily_send_limit as i32)
    .bind(p.seats as i32)
    .bind(Json(&p.features))
    .bind(p.sort_order as i32)
    .bind(p.active)
    .bind(Json(feature_flags))
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    // Catalog edits publish a new immutable commercial version. Existing
    // subscriptions remain bound to their purchased version until a reviewed
    // order explicitly assigns the new one.
    publish_plan_version_tx(&mut tx, code).await?;

    // Existing subscriptions intentionally receive no quota or entitlement
    // mutation here; their immutable plan_version_id remains authoritative.

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Plans are never hard-deleted (orders and users reference them); deactivating
/// hides them from new orders while existing assignments keep working.
pub async fn deactivate_plan(state: &AppState, code: &str) -> Result<(), ApiError> {
    let affected =
        sqlx::query("UPDATE plans SET active = FALSE, updated_at = now() WHERE code = $1")
            .bind(code)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found("Plan not found"));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct BillingSettings {
    pub bank_details: String,
    pub paypal_email: String,
    pub instructions: String,
    pub seller_legal_name: String,
    pub seller_email: String,
    pub seller_cr_number: String,
    pub seller_vat_number: String,
    pub seller_address: String,
    pub tax_rate_bps: i32,
    pub invoice_due_days: i32,
    pub grace_days: i32,
    pub retention_days: i32,
    pub renewal_reminder_days: i32,
    pub suspension_warning_days: i32,
}

#[derive(Clone, Debug)]
pub struct BillingSettingsInput {
    pub bank_details: String,
    pub paypal_email: String,
    pub instructions: String,
    pub seller_legal_name: String,
    pub seller_email: String,
    pub seller_cr_number: String,
    pub seller_vat_number: String,
    pub seller_address: String,
    pub tax_rate_bps: i32,
    pub invoice_due_days: i32,
    pub grace_days: i32,
    pub retention_days: i32,
    pub renewal_reminder_days: i32,
    pub suspension_warning_days: i32,
}

pub async fn settings(state: &AppState) -> Result<BillingSettings, ApiError> {
    sqlx::query_as(
        "SELECT bank_details,paypal_email,instructions,seller_legal_name,seller_email::text,
                seller_cr_number,seller_vat_number,seller_address,tax_rate_bps,invoice_due_days,grace_days,
                retention_days,renewal_reminder_days,suspension_warning_days
         FROM billing_settings WHERE id=TRUE",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn update_settings(state: &AppState, input: &BillingSettingsInput) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE billing_settings SET bank_details=$1,paypal_email=$2,instructions=$3,
          seller_legal_name=$4,seller_email=$5,seller_cr_number=$6,seller_vat_number=$7,
          seller_address=$8,tax_rate_bps=$9,invoice_due_days=$10,grace_days=$11,retention_days=$12,
          renewal_reminder_days=$13,suspension_warning_days=$14,updated_at=now()
         WHERE id=TRUE",
    )
    .bind(&input.bank_details)
    .bind(&input.paypal_email)
    .bind(&input.instructions)
    .bind(&input.seller_legal_name)
    .bind(&input.seller_email)
    .bind(&input.seller_cr_number)
    .bind(&input.seller_vat_number)
    .bind(&input.seller_address)
    .bind(input.tax_rate_bps)
    .bind(input.invoice_due_days)
    .bind(input.grace_days)
    .bind(input.retention_days)
    .bind(input.renewal_reminder_days)
    .bind(input.suspension_warning_days)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct BillingProfile {
    pub organization_id: Uuid,
    pub legal_name: String,
    pub billing_email: String,
    pub vat_number: String,
    pub cr_number: String,
    pub address_line1: String,
    pub address_line2: String,
    pub city: String,
    pub postal_code: String,
    pub country: String,
}

pub async fn billing_profile(state: &AppState, organization_id: Uuid) -> Result<BillingProfile, ApiError> {
    sqlx::query_as(
        "SELECT organization_id,legal_name,billing_email::text,vat_number,cr_number,address_line1,
                address_line2,city,postal_code,country
         FROM organization_billing_profiles WHERE organization_id=$1",
    )
    .bind(organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Business billing profile not found"))
}

pub async fn update_billing_profile(state: &AppState, profile: &BillingProfile) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO organization_billing_profiles
          (organization_id,legal_name,billing_email,vat_number,cr_number,address_line1,address_line2,city,postal_code,country,updated_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,now())
         ON CONFLICT(organization_id) DO UPDATE SET legal_name=EXCLUDED.legal_name,billing_email=EXCLUDED.billing_email,
          vat_number=EXCLUDED.vat_number,cr_number=EXCLUDED.cr_number,address_line1=EXCLUDED.address_line1,
          address_line2=EXCLUDED.address_line2,city=EXCLUDED.city,postal_code=EXCLUDED.postal_code,
          country=EXCLUDED.country,updated_at=now()",
    )
    .bind(profile.organization_id)
    .bind(&profile.legal_name)
    .bind(&profile.billing_email)
    .bind(&profile.vat_number)
    .bind(&profile.cr_number)
    .bind(&profile.address_line1)
    .bind(&profile.address_line2)
    .bind(&profile.city)
    .bind(&profile.postal_code)
    .bind(&profile.country)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct ScheduledChangeView {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub change_type: String,
    pub target_plan_code: Option<String>,
    pub target_plan_name: Option<String>,
    pub target_mailbox_count: Option<i32>,
    pub effective_at: chrono::DateTime<chrono::Utc>,
    pub status: String,
    pub requested_at: chrono::DateTime<chrono::Utc>,
    pub blocked_reason: String,
    pub note: String,
}

pub async fn scheduled_change_for_organization(state: &AppState, organization_id: Uuid) -> Result<Option<ScheduledChangeView>, ApiError> {
    sqlx::query_as(
        "SELECT c.id,c.organization_id,c.change_type,c.target_plan_code,p.name AS target_plan_name,c.target_mailbox_count,
                c.effective_at,c.status,c.requested_at,c.blocked_reason,c.note
         FROM subscription_scheduled_changes c LEFT JOIN plans p ON p.code=c.target_plan_code
         WHERE c.organization_id=$1 AND c.status IN ('pending','ready_for_renewal','blocked')
         ORDER BY c.requested_at DESC LIMIT 1"
    ).bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn schedule_subscription_change(
    state: &AppState, user_id: Uuid, target_plan_code: Option<&str>, target_mailbox_count: Option<i32>, cancel: bool, note: &str,
) -> Result<ScheduledChangeView, ApiError> {
    let organization_id=entitlements::active_organization_for_user(state,user_id).await?;
    let role: Option<String>=sqlx::query_scalar("SELECT role FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 AND status='active'")
        .bind(organization_id).bind(user_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if !matches!(role.as_deref(),Some("owner")|Some("billing")) { return Err(ApiError::forbidden("Business owner or billing access required")); }
    let current: (String,i32,String,Option<chrono::DateTime<chrono::Utc>>)=sqlx::query_as(
        "SELECT plan_code,purchased_mailbox_count,status,current_period_end FROM organization_subscriptions WHERE organization_id=$1"
    ).bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if !matches!(current.2.as_str(),"active"|"trial") { return Err(ApiError::conflict("A renewal change can only be scheduled while the current paid term is active")); }
    let effective_at=current.3.ok_or_else(|| ApiError::conflict("The current subscription has no renewal date"))?;
    if effective_at <= chrono::Utc::now() { return Err(ApiError::conflict("The current term has already ended; renew or reactivate the subscription instead")); }

    let (change_type,target_code,target_version,target_count,blocked_reason)=if cancel {
        ("cancel",None,None,None,String::new())
    } else {
        let code=target_plan_code.ok_or_else(|| ApiError::bad_request("Target plan is required"))?;
        let plan=entitlements::active_plan(state,code).await?;
        let count=target_mailbox_count.unwrap_or(plan.mailbox_limit);
        if count < plan.mailbox_limit || count > plan.max_mailboxes { return Err(ApiError::bad_request(format!("Mailbox count must be between {} and {}",plan.mailbox_limit,plan.max_mailboxes))); }
        let current_plan=entitlements::for_organization(state,organization_id).await?.plan;
        if !is_self_service_reduction(&current_plan,current.1,&plan,count) {
            return Err(ApiError::conflict("Upgrades and same-plan renewals can be ordered immediately; only reductions need to be scheduled for renewal"));
        }
        let version: Option<Uuid>=sqlx::query_scalar("SELECT current_version_id FROM plans WHERE code=$1 AND active=TRUE")
            .bind(code).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?.flatten();
        let version=version.ok_or_else(|| ApiError::conflict("The target plan has no published commercial version"))?;
        let usage:(i64,i64,i64,i64)=sqlx::query_as(
            "SELECT (SELECT count(*)::bigint FROM organization_memberships WHERE organization_id=$1 AND status IN ('active','invited')),
                    (SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status<>'deleted'),
                    (SELECT count(*)::bigint FROM organization_domains WHERE organization_id=$1 AND status<>'removing'),
                    COALESCE((SELECT SUM(quota_bytes)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL),0)::bigint"
        ).bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
        let pool=entitlements::scaled_storage_pool(&plan,count);
        let mut blockers=Vec::new();
        if usage.0 > count.max(plan.seats) as i64 { blockers.push(format!("reduce active seats to {} or fewer",count.max(plan.seats))); }
        if usage.1 > count as i64 { blockers.push(format!("reduce hosted mailboxes to {count} or fewer")); }
        if usage.2 > plan.domain_limit as i64 { blockers.push(format!("reduce domains to {} or fewer",plan.domain_limit)); }
        if usage.3 > pool { blockers.push("rebalance mailbox allocations within the target storage pool".to_string()); }
        ("plan_change",Some(code.to_string()),Some(version),Some(count),blockers.join("; "))
    };
    let id:Uuid=sqlx::query_scalar(
        "INSERT INTO subscription_scheduled_changes(organization_id,change_type,target_plan_code,target_plan_version_id,target_mailbox_count,effective_at,status,requested_by,blocked_reason,note,updated_at)
         VALUES($1,$2,$3,$4,$5,$6,'pending',$7,$8,$9,now())
         ON CONFLICT(organization_id) WHERE status IN ('pending','ready_for_renewal','blocked') DO UPDATE SET
           change_type=EXCLUDED.change_type,target_plan_code=EXCLUDED.target_plan_code,target_plan_version_id=EXCLUDED.target_plan_version_id,
           target_mailbox_count=EXCLUDED.target_mailbox_count,effective_at=EXCLUDED.effective_at,status='pending',requested_by=EXCLUDED.requested_by,
           requested_at=now(),cancelled_by=NULL,cancelled_at=NULL,applied_at=NULL,blocked_reason=EXCLUDED.blocked_reason,note=EXCLUDED.note,updated_at=now()
         RETURNING id"
    ).bind(organization_id).bind(change_type).bind(target_code).bind(target_version).bind(target_count).bind(effective_at).bind(user_id)
     .bind(&blocked_reason).bind(note.trim()).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO subscription_assignment_history(organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,assigned_by,event_type,period_end,status_after,reason,detail)
         SELECT s.organization_id,s.plan_code,COALESCE(pv.name,p.name),s.purchased_mailbox_count,'system_lifecycle',FALSE,$2,'scheduled_change',s.current_period_end,s.status,$3,$4
         FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id WHERE s.organization_id=$1"
    ).bind(organization_id).bind(user_id)
     .bind(if cancel{"Cancellation scheduled for renewal"}else{"Plan reduction scheduled for renewal"})
     .bind(Json(serde_json::json!({"scheduled_change_id":id,"change_type":change_type,"target_plan_code":target_code,"target_mailbox_count":target_count,"effective_at":effective_at,"blocked_reason":blocked_reason})))
     .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    scheduled_change_for_organization(state,organization_id).await?.ok_or_else(|| ApiError::internal("Scheduled change could not be loaded"))
}

pub async fn cancel_scheduled_change(state:&AppState,user_id:Uuid)->Result<(),ApiError>{
    let organization_id=entitlements::active_organization_for_user(state,user_id).await?;
    let role:Option<String>=sqlx::query_scalar("SELECT role FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 AND status='active'")
        .bind(organization_id).bind(user_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if !matches!(role.as_deref(),Some("owner")|Some("billing")){return Err(ApiError::forbidden("Business owner or billing access required"));}
    let affected=sqlx::query("UPDATE subscription_scheduled_changes SET status='cancelled',cancelled_by=$2,cancelled_at=now(),updated_at=now() WHERE organization_id=$1 AND status IN ('pending','ready_for_renewal','blocked')")
        .bind(organization_id).bind(user_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected();
    if affected==0{return Err(ApiError::not_found("No scheduled subscription change was found"));}
    Ok(())
}

/// An order plus its immutable invoice snapshot.
#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct OrderView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub email: String,
    pub display_name: String,
    pub plan_code: String,
    pub plan_name: String,
    pub amount_cents: i64,
    pub currency: String,
    pub interval: String,
    pub seats: i32,
    pub mailbox_count: i32,
    pub included_mailbox_count: i32,
    pub extra_mailbox_count: i32,
    pub extra_mailbox_unit_price_cents: i64,
    pub base_price_cents: i64,
    pub status: String,
    pub payment_method: String,
    pub payment_reference: String,
    pub customer_note: String,
    pub admin_note: String,
    pub invoice_number: Option<String>,
    pub invoice_status: String,
    pub subtotal_cents: i64,
    pub tax_rate_bps: i32,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub seller_snapshot: Value,
    pub buyer_snapshot: Value,
    pub activation_mode: String,
    pub subscription_assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub subscription_assigned_by: Option<Uuid>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub period_start: Option<chrono::DateTime<chrono::Utc>>,
    pub period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub paid_at: Option<chrono::DateTime<chrono::Utc>>,
}

const ORDER_COLUMNS: &str = "o.id,o.user_id,o.organization_id,org.name AS organization_name,u.email::text,u.display_name,o.plan_code,o.plan_name,\
 o.amount_cents::int8 AS amount_cents,o.currency,o.interval,o.seats,o.mailbox_count,o.included_mailbox_count,o.extra_mailbox_count,o.extra_mailbox_unit_price_cents::int8 AS extra_mailbox_unit_price_cents,o.base_price_cents::int8 AS base_price_cents,o.status,o.payment_method,o.payment_reference,o.customer_note,o.admin_note,\
 o.invoice_number,o.invoice_status,COALESCE(o.subtotal_cents,o.amount_cents)::int8 AS subtotal_cents,COALESCE(o.tax_rate_bps,0)::int4 AS tax_rate_bps,\
 COALESCE(o.tax_cents,0)::int8 AS tax_cents,COALESCE(o.total_cents,o.amount_cents)::int8 AS total_cents,o.seller_snapshot,o.buyer_snapshot,o.activation_mode,\
 o.subscription_assigned_at,o.subscription_assigned_by,o.issued_at,o.due_at,o.period_start,o.period_end,o.activated_at,o.created_at,o.submitted_at,o.paid_at";

pub async fn orders_for_user(state: &AppState, user_id: Uuid) -> Result<Vec<OrderView>, ApiError> {
    let organization_id = entitlements::active_organization_for_user(state, user_id).await?;
    sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders o JOIN users u ON u.id=o.user_id JOIN organizations org ON org.id=o.organization_id
         WHERE o.organization_id=$1 AND EXISTS(SELECT 1 FROM organization_memberships om
          WHERE om.organization_id=o.organization_id AND om.user_id=$2 AND om.status='active')
         ORDER BY o.created_at DESC",
    ))
    .bind(organization_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn all_orders(state: &AppState, status: Option<&str>) -> Result<Vec<OrderView>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders o JOIN users u ON u.id=o.user_id JOIN organizations org ON org.id=o.organization_id
         WHERE (($1::text IS NULL AND o.status IN ('pending','submitted')) OR o.status=$1) ORDER BY o.created_at DESC LIMIT 500",
    ))
    .bind(status)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn order_by_id(state: &AppState, id: Uuid) -> Result<OrderView, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders o JOIN users u ON u.id=o.user_id JOIN organizations org ON org.id=o.organization_id WHERE o.id=$1",
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Order not found"))
}

fn plan_activation_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23514") {
            let message = database.message();
            if message.contains("organization storage pool allocation exceeded") {
                return ApiError::conflict("Mailbox allocations exceed the target plan's storage pool. Rebalance mailbox storage before changing plan");
            }
            if message.contains("organization mailbox limit reached") {
                return ApiError::conflict("The target plan does not include enough mailboxes for the business's current hosted mailboxes");
            }
            if message.contains("organization seat limit reached") {
                return ApiError::conflict("The target plan does not include enough seats for the business's current members and pending invitations");
            }
            if message.contains("organization domain limit reached") {
                return ApiError::conflict("The target plan does not include enough domains for the business's current domain configuration");
            }
        }
    }
    ApiError::internal(error.to_string())
}

pub(crate) async fn enqueue_organization_access_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO provisioning_jobs
          (user_id,organization_id,mailbox_id,operation,target_email,account_id,status,max_attempts,dedupe_key)
         SELECT m.user_id,m.organization_id,m.id,'set_access',m.address::text,m.provider_account_id,
                'pending',8,'mailbox.access:'||m.id::text
         FROM mailboxes m
         WHERE m.organization_id=$1 AND m.deleted_at IS NULL AND m.status <> 'deleting'
         ON CONFLICT(dedupe_key) WHERE status IN('pending','retry') DO UPDATE SET
           user_id=EXCLUDED.user_id,organization_id=EXCLUDED.organization_id,mailbox_id=EXCLUDED.mailbox_id,
           target_email=EXCLUDED.target_email,account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),
           status='pending',attempts=0,next_attempt_at=now(),last_error='',last_failure_transient=FALSE,
           completed_at=NULL,updated_at=now()",
    )
    .bind(organization_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn enqueue_organization_access(state: &AppState, organization_id: Uuid) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    enqueue_organization_access_tx(&mut tx, organization_id).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

async fn activate_plan_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    plan_code: &str,
    plan_name: &str,
    plan_version_id: Uuid,
    plan_mailbox_bytes: i64,
    mailbox_count: i32,
    period_start: chrono::DateTime<chrono::Utc>,
    period_end: chrono::DateTime<chrono::Utc>,
    order_id: Uuid,
    invoice_number: &str,
    order_user_id: Uuid,
    due_at: chrono::DateTime<chrono::Utc>,
    grace_days: i32,
    assignment_source: &str,
    payment_confirmed: bool,
    assigned_by: Option<Uuid>,
) -> Result<(), ApiError> {
    let purge_in_progress: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM subscription_purge_runs WHERE organization_id=$1 AND status IN ('queued','processing','failed'))"
    )
    .bind(organization_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if purge_in_progress {
        return Err(ApiError::conflict("Retained-data purge is already in progress. Finish or recover the purge operation before activating another paid term."));
    }

    sqlx::query(
        "INSERT INTO organization_subscriptions
          (organization_id,plan_code,plan_version_id,status,purchased_mailbox_count,current_period_start,current_period_end,payment_due_at,grace_period_end,renewal_grace_end,last_order_id,
           assignment_source,assigned_at,assigned_by,assignment_invoice_number,assignment_order_user_id,retention_started_at,data_retention_until,purge_eligible_at,updated_at)
         VALUES($1,$2,$3,'active',$4,$5,$6,$7,$7+($8::text||' days')::interval,$6+($8::text||' days')::interval,$9,$10,now(),$11,$12,$13,NULL,NULL,NULL,now())
         ON CONFLICT(organization_id) DO UPDATE SET plan_code=EXCLUDED.plan_code,plan_version_id=EXCLUDED.plan_version_id,status='active',
          purchased_mailbox_count=EXCLUDED.purchased_mailbox_count,
          storage_pool_override_bytes=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.storage_pool_override_bytes ELSE NULL END,
          mailbox_quota_override_bytes=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.mailbox_quota_override_bytes ELSE NULL END,
          seat_limit_override=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.seat_limit_override ELSE NULL END,
          mailbox_limit_override=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.mailbox_limit_override ELSE NULL END,
          domain_limit_override=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.domain_limit_override ELSE NULL END,
          organization_daily_send_override=CASE WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code THEN organization_subscriptions.organization_daily_send_override ELSE NULL END,
          current_period_start=CASE
            WHEN organization_subscriptions.plan_code=EXCLUDED.plan_code
             AND organization_subscriptions.purchased_mailbox_count=EXCLUDED.purchased_mailbox_count
             AND organization_subscriptions.current_period_end IS NOT NULL
             AND organization_subscriptions.current_period_end > now()
             AND EXCLUDED.current_period_start >= organization_subscriptions.current_period_end
            THEN organization_subscriptions.current_period_start
            ELSE EXCLUDED.current_period_start
          END,current_period_end=EXCLUDED.current_period_end,
          payment_due_at=EXCLUDED.payment_due_at,grace_period_end=EXCLUDED.grace_period_end,renewal_grace_end=EXCLUDED.renewal_grace_end,last_order_id=EXCLUDED.last_order_id,
          assignment_source=EXCLUDED.assignment_source,assigned_at=now(),assigned_by=EXCLUDED.assigned_by,
          assignment_invoice_number=EXCLUDED.assignment_invoice_number,assignment_order_user_id=EXCLUDED.assignment_order_user_id,
          retention_started_at=NULL,data_retention_until=NULL,purge_eligible_at=NULL,retention_expired_at=NULL,purge_started_at=NULL,data_purged_at=NULL,cancelled_at=NULL,updated_at=now()",
    )
    .bind(organization_id)
    .bind(plan_code)
    .bind(plan_version_id)
    .bind(mailbox_count)
    .bind(period_start)
    .bind(period_end)
    .bind(due_at)
    .bind(grace_days)
    .bind(order_id)
    .bind(assignment_source)
    .bind(assigned_by)
    .bind(invoice_number)
    .bind(order_user_id)
    .execute(&mut **tx)
    .await
    .map_err(plan_activation_error)?;

    sqlx::query(
        "INSERT INTO subscription_assignment_history
          (organization_id,order_id,invoice_number,order_user_id,plan_code,plan_name,purchased_mailbox_count,
           assignment_source,payment_confirmed,assigned_by,event_type,period_start,period_end,status_after,reason,detail)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'assignment',$11,$12,'active',$13,$14)
         ON CONFLICT(order_id,assignment_source) WHERE order_id IS NOT NULL DO NOTHING",
    )
    .bind(organization_id)
    .bind(order_id)
    .bind(invoice_number)
    .bind(order_user_id)
    .bind(plan_code)
    .bind(plan_name)
    .bind(mailbox_count)
    .bind(assignment_source)
    .bind(payment_confirmed)
    .bind(assigned_by)
    .bind(period_start)
    .bind(period_end)
    .bind(match assignment_source {
        "payment_approval" => "Payment approved",
        "test_instant" => "Acceptance-test instant activation",
        _ => "Subscription assigned",
    })
    .bind(Json(serde_json::json!({
        "period_start": period_start,
        "period_end": period_end,
        "payment_due_at": due_at,
    })))
    .execute(&mut **tx)
    .await
    .map_err(plan_activation_error)?;

    sqlx::query(
        "UPDATE orders SET subscription_assigned_at=now(),subscription_assigned_by=$2,updated_at=now() WHERE id=$1",
    )
    .bind(order_id)
    .bind(assigned_by)
    .execute(&mut **tx)
    .await
    .map_err(plan_activation_error)?;

    // A reviewed renewal/order resolves the pending renewal intent. A matching
    // scheduled plan change is applied; a cancellation-at-renewal is cancelled
    // because the customer explicitly purchased another paid term.
    sqlx::query(
        "UPDATE subscription_scheduled_changes SET
           status=CASE WHEN change_type='plan_change' AND target_plan_code=$2 AND target_mailbox_count=$3 THEN 'applied' ELSE 'cancelled' END,
           applied_at=CASE WHEN change_type='plan_change' AND target_plan_code=$2 AND target_mailbox_count=$3 THEN now() ELSE applied_at END,
           cancelled_at=CASE WHEN change_type='cancel' OR target_plan_code<>$2 OR target_mailbox_count<>$3 THEN now() ELSE cancelled_at END,
           blocked_reason='',updated_at=now()
         WHERE organization_id=$1 AND status IN('pending','ready_for_renewal','blocked')"
    ).bind(organization_id).bind(plan_code).bind(mailbox_count).execute(&mut **tx).await
     .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE users SET plan=$2,quota_override_bytes=NULL,quota_bytes=$3,updated_at=now()
         WHERE id IN(SELECT user_id FROM organization_memberships WHERE organization_id=$1)",
    )
    .bind(organization_id)
    .bind(plan_code)
    .bind(plan_mailbox_bytes)
    .execute(&mut **tx)
    .await
    .map_err(plan_activation_error)?;

    sqlx::query("UPDATE mailboxes SET quota_bytes=$2,updated_at=now() WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NULL")
        .bind(organization_id)
        .bind(plan_mailbox_bytes)
        .execute(&mut **tx)
        .await
        .map_err(plan_activation_error)?;

    sqlx::query(
        "INSERT INTO provisioning_jobs
          (user_id,organization_id,mailbox_id,operation,target_email,account_id,quota_bytes,status,max_attempts,dedupe_key)
         SELECT COALESCE(m.user_id,o.created_by),m.organization_id,m.id,'set_quota',m.address::text,m.provider_account_id,
                m.quota_bytes,'pending',8,'mailbox.quota:'||m.id::text
         FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
         WHERE m.organization_id=$1 AND m.deleted_at IS NULL AND COALESCE(m.user_id,o.created_by) IS NOT NULL
         ON CONFLICT(dedupe_key) WHERE status IN('pending','retry') DO UPDATE SET quota_bytes=EXCLUDED.quota_bytes,
          status='pending',attempts=0,next_attempt_at=now(),last_error='',completed_at=NULL,updated_at=now()",
    )
    .bind(organization_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    // Subscription activation/reactivation is authoritative for provider
    // access as well as quota. Queue every mailbox so a previously suspended
    // Stalwart account is restored only after the paid subscription is active.
    enqueue_organization_access_tx(tx, organization_id).await?;
    Ok(())
}


/// Keep manual-renewal subscription state aligned with authoritative period
/// dates. Entitlement reads also fail closed on expired periods, so this worker
/// primarily persists the operator-visible lifecycle and immutable history.
pub async fn reconcile_subscription_lifecycle(state: &AppState) -> Result<(), ApiError> {
    let cfg=settings(state).await?;

    // One reminder per paid term.
    sqlx::query(
        "INSERT INTO billing_lifecycle_outbox(organization_id,kind,event_key,recipient)
         SELECT s.organization_id,'renewal_reminder',to_char(s.current_period_end,'YYYYMMDDHH24MISS'),
                COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text)
         FROM organization_subscriptions s
         LEFT JOIN organization_billing_profiles bp ON bp.organization_id=s.organization_id
         LEFT JOIN LATERAL (SELECT u.email FROM organization_memberships om JOIN users u ON u.id=om.user_id
           WHERE om.organization_id=s.organization_id AND om.status='active' AND om.role='owner' ORDER BY om.joined_at LIMIT 1) owner ON TRUE
         WHERE s.status IN('active','trial') AND s.current_period_end IS NOT NULL
           AND s.current_period_end>now() AND s.current_period_end<=now()+($1::text||' days')::interval
           AND COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text) IS NOT NULL
         ON CONFLICT DO NOTHING"
    ).bind(cfg.renewal_reminder_days).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    // Cancellation-at-renewal has priority over the generic past-due transition.
    let cancelled:Vec<Uuid>=sqlx::query_scalar(
        "WITH due AS (
           SELECT c.organization_id,c.id FROM subscription_scheduled_changes c
           WHERE c.change_type='cancel' AND c.status IN('pending','ready_for_renewal','blocked') AND c.effective_at<=now()
         ), changed AS (
           UPDATE organization_subscriptions s SET status='cancelled',cancelled_at=now(),
             retention_started_at=now(),data_retention_until=now()+($1::text||' days')::interval,
             purge_eligible_at=now()+($1::text||' days')::interval,updated_at=now()
           FROM due d WHERE s.organization_id=d.organization_id RETURNING s.organization_id
         )
         UPDATE subscription_scheduled_changes c SET status='applied',applied_at=now(),updated_at=now()
         FROM changed x WHERE c.organization_id=x.organization_id AND c.change_type='cancel' AND c.status IN('pending','ready_for_renewal','blocked')
         RETURNING c.organization_id"
    ).bind(cfg.retention_days).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    for organization_id in cancelled {
        enqueue_organization_access(state,organization_id).await?;
        queue_lifecycle_notice(state,organization_id,"cancelled").await?;
    }

    // A scheduled reduction becomes ready for its renewal invoice at term end;
    // it never mutates paid entitlements without reviewed payment.
    sqlx::query(
        "UPDATE subscription_scheduled_changes SET status='ready_for_renewal',updated_at=now()
         WHERE change_type='plan_change' AND status='pending' AND effective_at<=now()"
    ).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    let expired: Vec<(Uuid, String, String, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "WITH changed AS (
           UPDATE organization_subscriptions s SET status='past_due',
             renewal_grace_end=COALESCE(s.renewal_grace_end,s.current_period_end+($1::text||' days')::interval),updated_at=now()
           WHERE s.status IN('active','trial') AND s.current_period_end IS NOT NULL AND s.current_period_end<=now()
           RETURNING s.organization_id,s.plan_code,s.plan_version_id,s.purchased_mailbox_count,s.current_period_start,s.current_period_end
         )
         SELECT c.organization_id,c.plan_code,COALESCE(pv.name,p.name),c.purchased_mailbox_count,c.current_period_start,c.current_period_end
         FROM changed c JOIN plans p ON p.code=c.plan_code LEFT JOIN plan_versions pv ON pv.id=c.plan_version_id"
    ).bind(cfg.grace_days).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    for (organization_id,plan_code,plan_name,mailbox_count,period_start,period_end) in expired {
        sqlx::query("INSERT INTO subscription_assignment_history(organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,event_type,period_start,period_end,status_after,reason,detail) VALUES($1,$2,$3,$4,'system_lifecycle',FALSE,'status',$5,$6,'past_due','Subscription period expired',$7)")
            .bind(organization_id).bind(plan_code).bind(plan_name).bind(mailbox_count).bind(period_start).bind(period_end)
            .bind(Json(serde_json::json!({"transition":"active_to_past_due"}))).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        queue_lifecycle_notice(state,organization_id,"past_due").await?;
    }

    // Warning shortly before provider access is suspended.
    sqlx::query(
        "INSERT INTO billing_lifecycle_outbox(organization_id,kind,event_key,recipient)
         SELECT s.organization_id,'suspension_warning',to_char(s.renewal_grace_end,'YYYYMMDDHH24MISS'),COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text)
         FROM organization_subscriptions s LEFT JOIN organization_billing_profiles bp ON bp.organization_id=s.organization_id
         LEFT JOIN LATERAL (SELECT u.email FROM organization_memberships om JOIN users u ON u.id=om.user_id WHERE om.organization_id=s.organization_id AND om.status='active' AND om.role='owner' ORDER BY om.joined_at LIMIT 1) owner ON TRUE
         WHERE s.status='past_due' AND s.renewal_grace_end IS NOT NULL AND s.renewal_grace_end>now()
           AND s.renewal_grace_end<=now()+($1::text||' days')::interval AND COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text) IS NOT NULL
         ON CONFLICT DO NOTHING"
    ).bind(cfg.suspension_warning_days).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    let suspended: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE organization_subscriptions SET status='suspended',retention_started_at=COALESCE(retention_started_at,now()),
           data_retention_until=COALESCE(data_retention_until,now()+($1::text||' days')::interval),
           purge_eligible_at=COALESCE(purge_eligible_at,now()+($1::text||' days')::interval),updated_at=now()
         WHERE status='past_due' AND renewal_grace_end IS NOT NULL AND renewal_grace_end<=now() RETURNING organization_id"
    ).bind(cfg.retention_days).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    for organization_id in suspended {
        enqueue_organization_access(state,organization_id).await?;
        queue_lifecycle_notice(state,organization_id,"suspended").await?;
    }

    // Retention warning, no automatic destructive purge. Platform operations
    // may purge only after purge_eligible_at and an explicit operator action.
    sqlx::query(
        "INSERT INTO billing_lifecycle_outbox(organization_id,kind,event_key,recipient)
         SELECT s.organization_id,'retention_warning',to_char(s.data_retention_until,'YYYYMMDDHH24MISS'),COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text)
         FROM organization_subscriptions s LEFT JOIN organization_billing_profiles bp ON bp.organization_id=s.organization_id
         LEFT JOIN LATERAL (SELECT u.email FROM organization_memberships om JOIN users u ON u.id=om.user_id WHERE om.organization_id=s.organization_id AND om.status='active' AND om.role='owner' ORDER BY om.joined_at LIMIT 1) owner ON TRUE
         WHERE s.status IN('suspended','cancelled') AND s.data_retention_until IS NOT NULL AND s.data_retention_until>now()
           AND s.data_retention_until<=now()+interval '7 days' AND COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text) IS NOT NULL
         ON CONFLICT DO NOTHING"
    ).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    // Crossing the retention deadline is intentionally only a marker. Data is
    // never destroyed by the lifecycle clock; an explicit platform-admin
    // purge request is required below this safety boundary.
    sqlx::query(
        "WITH changed AS (
           UPDATE organization_subscriptions SET retention_expired_at=COALESCE(retention_expired_at,now()),updated_at=now()
           WHERE status IN('suspended','cancelled') AND purge_eligible_at IS NOT NULL AND purge_eligible_at<=now()
             AND data_purged_at IS NULL AND retention_expired_at IS NULL
           RETURNING organization_id,plan_code,plan_version_id,purchased_mailbox_count,current_period_start,current_period_end,purge_eligible_at
         )
         INSERT INTO subscription_assignment_history
           (organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,event_type,period_start,period_end,status_after,reason,detail)
         SELECT c.organization_id,c.plan_code,COALESCE(pv.name,p.name),c.purchased_mailbox_count,'system_lifecycle',FALSE,'retention',
                c.current_period_start,c.current_period_end,s.status,'Data retention deadline reached',
                jsonb_build_object('purge_eligible_at',c.purge_eligible_at,'automatic_purge',FALSE)
         FROM changed c JOIN organization_subscriptions s ON s.organization_id=c.organization_id
         JOIN plans p ON p.code=c.plan_code LEFT JOIN plan_versions pv ON pv.id=c.plan_version_id"
    ).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    reconcile_purge_runs(state).await?;
    Ok(())
}

/// Update explicit retained-data purge runs from the durable provider workers.
/// A purge is complete only after every live mailbox has been deleted from the
/// provider and every alias/group marked by the purge reports provider deletion.
pub async fn reconcile_purge_runs(state: &AppState) -> Result<(), ApiError> {
    let runs: Vec<(Uuid,Uuid,chrono::DateTime<chrono::Utc>,i32,i32)> = sqlx::query_as(
        "SELECT id,organization_id,requested_at,mailbox_count_snapshot,address_count_snapshot
         FROM subscription_purge_runs WHERE status IN('queued','processing') ORDER BY requested_at LIMIT 100"
    ).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

    for (run_id,organization_id,requested_at,mailbox_snapshot,address_snapshot) in runs {
        let live_mailboxes: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL"
        ).bind(organization_id).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        let pending_addresses: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM business_addresses
             WHERE organization_id=$1 AND deleted_at IS NOT NULL AND deleted_at >= $2 AND sync_status <> 'deleted'"
        ).bind(organization_id).bind(requested_at).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        let dead_delete: Option<String> = sqlx::query_scalar(
            "SELECT last_error FROM provisioning_jobs WHERE organization_id=$1 AND operation='delete_mailbox'
             AND status='dead' AND created_at >= $2 ORDER BY updated_at DESC LIMIT 1"
        ).bind(organization_id).bind(requested_at).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        let address_failure: Option<String> = sqlx::query_scalar(
            "SELECT NULLIF(sync_error,'') FROM business_addresses
             WHERE organization_id=$1 AND deleted_at IS NOT NULL AND deleted_at >= $2
               AND sync_status='error' AND sync_attempts>=12
             ORDER BY updated_at DESC LIMIT 1"
        ).bind(organization_id).bind(requested_at).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;

        let completed_mailboxes = (mailbox_snapshot as i64 - live_mailboxes).clamp(0, mailbox_snapshot as i64) as i32;
        let completed_addresses = (address_snapshot as i64 - pending_addresses).clamp(0, address_snapshot as i64) as i32;
        if let Some(error)=dead_delete.or(address_failure) {
            sqlx::query(
                "UPDATE subscription_purge_runs SET status='failed',completed_mailbox_count=$2,completed_address_count=$3,
                 last_error=left($4,1000),updated_at=now() WHERE id=$1 AND status IN('queued','processing')"
            ).bind(run_id).bind(completed_mailboxes).bind(completed_addresses).bind(error)
             .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            continue;
        }
        if live_mailboxes == 0 && pending_addresses == 0 {
            let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
            let finalized: Option<Uuid> = sqlx::query_scalar(
                "UPDATE subscription_purge_runs SET status='completed',completed_mailbox_count=mailbox_count_snapshot,
                 completed_address_count=address_count_snapshot,last_error='',completed_at=now(),updated_at=now()
                 WHERE id=$1 AND status IN('queued','processing') RETURNING organization_id"
            ).bind(run_id).fetch_optional(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            if let Some(org_id)=finalized {
                sqlx::query(
                    "UPDATE organization_subscriptions SET data_purged_at=COALESCE(data_purged_at,now()),updated_at=now() WHERE organization_id=$1"
                ).bind(org_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
                // The purge is only finalized after provider deletion has been
                // confirmed. At that point the cached organization usage must
                // also stop reporting storage/mailboxes that no longer exist.
                sqlx::query(
                    "UPDATE organization_usage SET storage_bytes=0,mailbox_count=0,refreshed_at=now() WHERE organization_id=$1"
                ).bind(org_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
                sqlx::query(
                    "INSERT INTO subscription_assignment_history
                      (organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,event_type,period_start,period_end,status_after,reason,detail)
                     SELECT s.organization_id,s.plan_code,COALESCE(pv.name,p.name),s.purchased_mailbox_count,'system_lifecycle',FALSE,'retention',
                            s.current_period_start,s.current_period_end,s.status,'Retained mailbox data purge completed',
                            jsonb_build_object('purge_run_id',$2::text,'data_purged_at',now())
                     FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
                     WHERE s.organization_id=$1"
                ).bind(org_id).bind(run_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            }
            tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
        } else {
            sqlx::query(
                "UPDATE subscription_purge_runs SET status='processing',started_at=COALESCE(started_at,now()),
                 completed_mailbox_count=$2,completed_address_count=$3,last_error='',updated_at=now() WHERE id=$1"
            ).bind(run_id).bind(completed_mailboxes).bind(completed_addresses)
             .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        }
    }
    Ok(())
}

async fn queue_lifecycle_notice(state:&AppState,organization_id:Uuid,kind:&str)->Result<(),ApiError>{
    sqlx::query(
        "INSERT INTO billing_lifecycle_outbox(organization_id,kind,event_key,recipient)
         SELECT s.organization_id,$2,COALESCE(to_char(s.current_period_end,'YYYYMMDDHH24MISS'),to_char(now(),'YYYYMMDDHH24MISS')),
                COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text)
         FROM organization_subscriptions s LEFT JOIN organization_billing_profiles bp ON bp.organization_id=s.organization_id
         LEFT JOIN LATERAL (SELECT u.email FROM organization_memberships om JOIN users u ON u.id=om.user_id WHERE om.organization_id=s.organization_id AND om.status='active' AND om.role='owner' ORDER BY om.joined_at LIMIT 1) owner ON TRUE
         WHERE s.organization_id=$1 AND COALESCE(NULLIF(bp.billing_email::text,''),owner.email::text) IS NOT NULL ON CONFLICT DO NOTHING"
    ).bind(organization_id).bind(kind).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Run subscription expiry/grace enforcement independently from low-frequency
/// housekeeping. Five-minute reconciliation keeps provider access and the UI
/// close to the authoritative billing timestamps without relying on traffic.
pub fn spawn_lifecycle_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tick.tick().await;
            if let Err(error) = reconcile_subscription_lifecycle(&state).await {
                tracing::warn!(%error, "subscription lifecycle reconciliation failed");
            }
        }
    });
}

fn is_self_service_reduction(current: &PlanLimits, current_count: i32, target: &PlanLimits, target_count: i32) -> bool {
    target_count < current_count
        || target_count > target.max_mailboxes
        || target.price_cents < current.price_cents
        || target.mailbox_bytes < current.mailbox_bytes
        || entitlements::scaled_storage_pool(target, target_count) < entitlements::scaled_storage_pool(current, current_count)
        || target.domain_limit < current.domain_limit
        || target.max_attachment_bytes < current.max_attachment_bytes
        || target.max_recipients < current.max_recipients
        || target.organization_daily_send_limit < current.organization_daily_send_limit
        || match (current.alias_limit_per_mailbox, target.alias_limit_per_mailbox) {
            (None, Some(_)) => true,
            (Some(old), Some(new)) => new < old,
            _ => false,
        }
}

/// Create an invoice immediately. In current testing mode the plan is also
/// activated in the same transaction, but the invoice stays due until payment
/// is submitted and approved.
pub async fn create_order(
    state: &AppState,
    user_id: Uuid,
    plan_code: &str,
    mailbox_count: i32,
    payment_method: &str,
    customer_note: &str,
) -> Result<OrderView, ApiError> {
    // Resolve the public catalog entry to its immutable commercial version
    // before pricing or validating the order. This prevents a concurrent
    // platform-admin plan edit from mixing prices/limits from two versions.
    let plan_version_id: Uuid = sqlx::query_scalar(
        "SELECT current_version_id FROM plans WHERE code=$1 AND active=TRUE"
    )
    .bind(plan_code)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .flatten()
    .ok_or_else(|| ApiError::not_found("Active plan not found or no published commercial version exists"))?;
    let plan = entitlements::plan_version(state, plan_version_id).await?;
    let organization_id = entitlements::active_organization_for_user(state, user_id).await?;
    entitlements::validate_plan_change_capacity_with_limits(state, organization_id, &plan, mailbox_count).await?;
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 AND status='active'",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !matches!(membership.as_deref(), Some("owner") | Some("billing")) {
        return Err(ApiError::forbidden("Business owner or billing access required"));
    }

    let cfg = settings(state).await?;
    let profile = billing_profile(state, organization_id).await?;
    let (account_email, display_name): (String, String) = sqlx::query_as(
        "SELECT email::text,display_name FROM users WHERE id=$1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let recipient = if profile.billing_email.trim().is_empty() { account_email.clone() } else { profile.billing_email.clone() };
    let extra_mailbox_count = (mailbox_count - plan.mailbox_limit).max(0);
    let base_price = plan.price_cents.max(0);
    let extra_mailbox_total = plan.extra_mailbox_price_cents.max(0).saturating_mul(extra_mailbox_count as i64);
    let subtotal = base_price.saturating_add(extra_mailbox_total);
    let effective_tax_bps = if cfg.seller_vat_number.trim().is_empty() { 0 } else { cfg.tax_rate_bps.max(0) } as i64;
    let tax = (subtotal.saturating_mul(effective_tax_bps).saturating_add(5000)) / 10000;
    let total = subtotal.saturating_add(tax);
    if subtotal > i32::MAX as i64 || tax > i32::MAX as i64 || total > i32::MAX as i64 {
        return Err(ApiError::bad_request("Order total exceeds the supported invoice amount"));
    }
    let due_at = chrono::Utc::now() + chrono::Duration::days(cfg.invoice_due_days as i64);
    let seller_snapshot = serde_json::json!({
        "legal_name":cfg.seller_legal_name,"email":cfg.seller_email,"cr_number":cfg.seller_cr_number,
        "vat_number":cfg.seller_vat_number,"address":cfg.seller_address
    });
    let buyer_snapshot = serde_json::json!({
        "legal_name":if profile.legal_name.trim().is_empty(){ organization_id.to_string() }else{ profile.legal_name.clone() },
        "billing_email":recipient.clone(),"vat_number":profile.vat_number,"cr_number":profile.cr_number,
        "address_line1":profile.address_line1,"address_line2":profile.address_line2,"city":profile.city,
        "postal_code":profile.postal_code,"country":profile.country,"account_email":account_email,"display_name":display_name
    });

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    // Serialize order creation for this business so the friendly one-open-
    // invoice rule is race-safe, not merely protected by the unique index.
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(organization_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let purge_in_progress: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM subscription_purge_runs WHERE organization_id=$1 AND status IN ('queued','processing','failed'))"
    )
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if purge_in_progress {
        return Err(ApiError::conflict(
            "Retained-data purge is in progress for this business. Contact support before placing another plan order."
        ));
    }
    let open_invoice: Option<String> = sqlx::query_scalar(
        "SELECT invoice_number FROM orders WHERE organization_id=$1 AND status IN ('pending','submitted') AND invoice_status='issued' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(organization_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(invoice) = open_invoice {
        return Err(ApiError::conflict(format!(
            "Invoice {invoice} is still open. Pay or cancel it before ordering another plan."
        )));
    }

    let existing: Option<(String, String, i32, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT plan_code,
                CASE WHEN status IN ('active','trial') AND current_period_end IS NOT NULL AND current_period_end<=now() THEN 'past_due' ELSE status END,
                purchased_mailbox_count,assignment_source,last_order_id
         FROM organization_subscriptions WHERE organization_id=$1 FOR UPDATE"
    ).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let first_test_activation = existing.as_ref().is_some_and(|(_, status, _, source, last_order_id)| {
        status == "suspended" && source == "bootstrap" && last_order_id.is_none()
    });
    if let Some((current_code, _, current_count, _, _)) = existing.as_ref().filter(|(_, status, _, _, _)| matches!(status.as_str(), "active" | "trial")) {
        let current_plan = entitlements::for_organization(state, organization_id).await?.plan;
        if is_self_service_reduction(&current_plan, *current_count, &plan, mailbox_count) {
            return Err(ApiError::conflict(
                "This reduction must be scheduled for renewal from Billing instead of being activated during the current paid term"
            ));
        }
    }

    let invoice_number: String = sqlx::query_scalar(
        "SELECT 'INV-'||to_char(now(),'YYYY')||'-'||lpad(nextval('invoice_number_seq')::text,6,'0')",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    // Test activation is only for the first plan. Never replace a live plan
    // with an unpaid order; upgrades take effect after payment review.
    let activation_mode = if state.billing_instant_activation && first_test_activation { "test_instant" } else { "payment_approval" };
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders
          (user_id,organization_id,plan_code,plan_version_id,plan_name,amount_cents,currency,interval,seats,mailbox_count,included_mailbox_count,
           extra_mailbox_count,extra_mailbox_unit_price_cents,base_price_cents,payment_method,customer_note,
           invoice_number,invoice_status,issued_at,due_at,subtotal_cents,tax_rate_bps,tax_cents,total_cents,
           seller_snapshot,buyer_snapshot,period_start,period_end,activation_mode,activated_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,'issued',now(),$18,$19,$20,$21,$22,$23,$24,
                CASE WHEN $25='test_instant' THEN now() ELSE NULL END,
                CASE WHEN $25='test_instant' THEN now()+CASE WHEN $8='year' THEN interval '1 year' ELSE interval '1 month' END ELSE NULL END,
                $25,CASE WHEN $25='test_instant' THEN now() ELSE NULL END) RETURNING id",
    )
    .bind(user_id).bind(organization_id).bind(&plan.code).bind(plan_version_id).bind(&plan.name).bind(total as i32)
    .bind(&plan.currency).bind(&plan.interval).bind(mailbox_count).bind(mailbox_count).bind(plan.mailbox_limit)
    .bind(extra_mailbox_count).bind(plan.extra_mailbox_price_cents as i32).bind(base_price as i32).bind(payment_method).bind(customer_note)
    .bind(&invoice_number).bind(&due_at).bind(subtotal as i32).bind(effective_tax_bps as i32).bind(tax as i32)
    .bind(total as i32).bind(Json(seller_snapshot)).bind(Json(buyer_snapshot.clone())).bind(activation_mode)
    .fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;

    if activation_mode == "test_instant" {
        let (period_start, period_end): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "SELECT period_start,period_end FROM orders WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        activate_plan_tx(
            &mut tx, organization_id, &plan.code, &plan.name, plan_version_id, plan.mailbox_bytes as i64, mailbox_count,
            period_start, period_end, id, &invoice_number, user_id, due_at, cfg.grace_days,
            "test_instant", false, None,
        )
        .await?;
    }
    sqlx::query("INSERT INTO billing_email_outbox(order_id,recipient,kind) VALUES($1,$2,'invoice_issued') ON CONFLICT DO NOTHING")
        .bind(id).bind(&recipient).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    order_by_id(state, id).await
}

pub async fn submit_order(
    state: &AppState,
    user_id: Uuid,
    id: Uuid,
    payment_method: &str,
    payment_reference: &str,
) -> Result<OrderView, ApiError> {
    let affected = sqlx::query(
        "UPDATE orders SET status='submitted',payment_method=$3,payment_reference=$4,submitted_at=now(),updated_at=now()
         WHERE id=$1 AND user_id=$2 AND organization_id=(SELECT active_organization_id FROM users WHERE id=$2)
           AND status='pending' AND invoice_status='issued'",
    )
    .bind(id).bind(user_id).bind(payment_method).bind(payment_reference)
    .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?.rows_affected();
    if affected == 0 { return Err(ApiError::conflict("Only an unpaid issued invoice can be marked as paid")); }
    order_by_id(state, id).await
}

pub async fn cancel_order(state: &AppState, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(Uuid,String)> = sqlx::query_as(
        "SELECT organization_id,activation_mode FROM orders
         WHERE id=$1 AND user_id=$2 AND organization_id=(SELECT active_organization_id FROM users WHERE id=$2)
           AND status IN('pending','submitted') FOR UPDATE",
    )
    .bind(id).bind(user_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (organization_id, activation_mode) = row.ok_or_else(|| ApiError::not_found("No cancellable order with that id was found"))?;
    sqlx::query("UPDATE orders SET status='cancelled',invoice_status='void',updated_at=now() WHERE id=$1")
        .bind(id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    // Test-instant access must not survive a voided invoice. We cannot safely
    // infer an older commercial subscription here, so suspend only when this
    // exact order is still the authoritative assignment. A new order can
    // immediately activate another test plan.
    if activation_mode == "test_instant" {
        sqlx::query(
            "UPDATE organization_subscriptions SET status='suspended',updated_at=now()
             WHERE organization_id=$1 AND last_order_id=$2 AND assignment_source='test_instant'",
        )
        .bind(organization_id).bind(id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        enqueue_organization_access_tx(&mut tx, organization_id).await?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Verify a submitted manual payment and bind the reviewed invoice to the
/// authoritative business subscription. The exact ordered plan/mailbox count
/// is re-asserted even when acceptance-test instant activation was used.
pub async fn approve_order(
    state: &AppState,
    admin_id: Uuid,
    id: Uuid,
    admin_note: &str,
) -> Result<OrderView, ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(Uuid,Uuid,String,Uuid,String,String,String,i64,i32,chrono::DateTime<chrono::Utc>,i32,String,String,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT user_id,organization_id,plan_code,plan_version_id,plan_name,status,interval,
                COALESCE((SELECT mailbox_bytes FROM plan_versions WHERE id=orders.plan_version_id),(SELECT mailbox_bytes FROM plans WHERE code=orders.plan_code))::bigint,mailbox_count,
                COALESCE(due_at,now()),
                (SELECT grace_days FROM billing_settings WHERE id=TRUE),activation_mode,COALESCE(invoice_number,''),period_start,period_end
         FROM orders WHERE id=$1 FOR UPDATE",
    )
    .bind(id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (order_user_id, organization_id, plan_code, plan_version_id, plan_name, status, interval, mailbox_bytes, mailbox_count, due_at, grace_days, _activation_mode, invoice_number, period_start, period_end) =
        row.ok_or_else(|| ApiError::not_found("Order not found"))?;
    if status != "submitted" {
        return Err(ApiError::conflict("Payment must be submitted with a reference before this invoice can be approved"));
    }
    let current: Option<(String, String, i32, Option<Uuid>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT plan_code,status,purchased_mailbox_count,last_order_id,current_period_start,current_period_end
         FROM organization_subscriptions WHERE organization_id=$1 FOR UPDATE"
    ).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let target_plan = entitlements::plan_version(state, plan_version_id).await?;
    if let Some((_current_code, _, current_count, _, _, _)) = current.as_ref()
        .filter(|(_, state, _, last_order, _, _)| matches!(state.as_str(), "active" | "trial") && *last_order != Some(id)) {
        let current_plan = entitlements::for_organization(state, organization_id).await?.plan;
        if is_self_service_reduction(&current_plan, *current_count, &target_plan, mailbox_count) {
            return Err(ApiError::conflict("The business plan changed after this invoice was issued. Review the invoice before applying a lower plan"));
        }
    }
    entitlements::validate_plan_change_capacity_with_limits(state, organization_id, &target_plan, mailbox_count).await?;

    // Same-plan/same-quantity payment is a renewal. If the customer renews
    // early, start the newly purchased coverage at the existing expiry so no
    // paid time is discarded. Expired/suspended renewals start from now.
    let renewal_anchor = current.as_ref().and_then(|(current_code, current_status, current_count, last_order, _, current_end)| {
        if current_code == &plan_code
            && *current_count == mailbox_count
            && *last_order != Some(id)
            && matches!(current_status.as_str(), "active" | "trial" | "past_due" | "suspended")
        {
            current_end.as_ref().cloned().map(|end| end.max(chrono::Utc::now()))
        } else {
            None
        }
    });

    let (period_start, period_end) = if let Some(anchor) = renewal_anchor {
        sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "UPDATE orders SET activated_at=COALESCE(activated_at,now()),period_start=$2,
               period_end=$2+CASE WHEN $3='year' THEN interval '1 year' ELSE interval '1 month' END,updated_at=now()
             WHERE id=$1 RETURNING period_start,period_end",
        )
        .bind(id)
        .bind(anchor)
        .bind(&interval)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        match (period_start, period_end) {
            (Some(start), Some(end)) => (start, end),
            _ => sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                "UPDATE orders SET
                   activated_at=COALESCE(activated_at,now()),
                   period_start=COALESCE(period_start,now()),
                   period_end=COALESCE(period_end,COALESCE(period_start,now())+CASE WHEN interval='year' THEN interval '1 year' ELSE interval '1 month' END),
                   updated_at=now()
                 WHERE id=$1 RETURNING period_start,period_end",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?,
        }
    };

    // The reviewed invoice is authoritative even in acceptance-test instant
    // mode. Re-assert exactly the ordered plan + mailbox quantity so a paid
    // invoice can never point at a different active subscription.
    activate_plan_tx(
        &mut tx, organization_id, &plan_code, &plan_name, plan_version_id, mailbox_bytes, mailbox_count,
        period_start, period_end, id, &invoice_number, order_user_id, due_at, grace_days,
        "payment_approval", true, Some(admin_id),
    )
    .await?;

    sqlx::query(
        "UPDATE orders SET status='paid',invoice_status='paid',paid_at=now(),reviewed_at=now(),reviewed_by=$2,
          admin_note=$3,activated_at=COALESCE(activated_at,$4),subscription_assigned_at=now(),subscription_assigned_by=$2,updated_at=now() WHERE id=$1",
    )
    .bind(id).bind(admin_id).bind(admin_note).bind(period_start).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let recipient: String = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(buyer_snapshot->>'billing_email',''),u.email::text) FROM orders o JOIN users u ON u.id=o.user_id WHERE o.id=$1",
    )
    .bind(id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("INSERT INTO billing_email_outbox(order_id,recipient,kind) VALUES($1,$2,'payment_received') ON CONFLICT DO NOTHING")
        .bind(id).bind(&recipient).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("INSERT INTO billing_lifecycle_outbox(organization_id,kind,event_key,recipient) VALUES($1,'reactivated',$2,$3) ON CONFLICT DO NOTHING")
        .bind(organization_id).bind(format!("order:{id}")).bind(&recipient).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    order_by_id(state,id).await
}

pub async fn reject_order(state: &AppState, admin_id: Uuid, id: Uuid, admin_note: &str) -> Result<OrderView, ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(Uuid,String)> = sqlx::query_as(
        "SELECT organization_id,activation_mode FROM orders WHERE id=$1 AND status IN('pending','submitted') FOR UPDATE",
    )
    .bind(id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (organization_id, activation_mode) = row.ok_or_else(|| ApiError::not_found("No reviewable order with that id was found"))?;
    sqlx::query(
        "UPDATE orders SET status='rejected',invoice_status='void',reviewed_at=now(),reviewed_by=$2,admin_note=$3,updated_at=now() WHERE id=$1",
    )
    .bind(id).bind(admin_id).bind(admin_note).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if activation_mode == "test_instant" {
        sqlx::query(
            "UPDATE organization_subscriptions SET status='suspended',updated_at=now()
             WHERE organization_id=$1 AND last_order_id=$2 AND assignment_source='test_instant'",
        )
        .bind(organization_id).bind(id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        enqueue_organization_access_tx(&mut tx, organization_id).await?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    order_by_id(state,id).await
}

/// Durable transactional-email worker for issued invoices and paid receipts.
pub fn spawn_email_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            if let Err(error) = process_one_email(&state).await {
                tracing::warn!(%error, "billing email worker iteration failed");
            }
            if let Err(error) = process_one_lifecycle_email(&state).await {
                tracing::warn!(%error, "billing lifecycle email worker iteration failed");
            }
        }
    });
}

async fn process_one_email(state: &AppState) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let claimed: Option<(Uuid,Uuid,String,String)> = sqlx::query_as(
        "WITH candidate AS (
           SELECT id FROM billing_email_outbox
           WHERE (status IN('pending','retry') AND next_attempt_at<=now()) OR (status='sending' AND updated_at<now()-interval '10 minutes')
           ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1
         )
         UPDATE billing_email_outbox b SET status='sending',attempts=attempts+1,updated_at=now()
         FROM candidate c WHERE b.id=c.id RETURNING b.id,b.order_id,b.recipient::text,b.kind",
    )
    .fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((outbox_id,order_id,recipient,kind)) = claimed else { return Ok(()); };
    let order = order_by_id(state,order_id).await?;
    let result = crate::services::email::send_billing_document(state,&recipient,&order,&kind).await;
    match result {
        Ok(()) => {
            sqlx::query("UPDATE billing_email_outbox SET status='sent',sent_at=now(),last_error='',updated_at=now() WHERE id=$1")
                .bind(outbox_id).execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
        }
        Err(error) => {
            let message=error.to_string();
            sqlx::query(
                "UPDATE billing_email_outbox SET status=CASE WHEN attempts>=8 THEN 'failed' ELSE 'retry' END,
                 next_attempt_at=now()+(LEAST(3600,30*GREATEST(1,attempts))::text||' seconds')::interval,
                 last_error=left($2,1000),updated_at=now() WHERE id=$1",
            )
            .bind(outbox_id).bind(message).execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }
    Ok(())
}



async fn process_one_lifecycle_email(state:&AppState)->Result<(),ApiError>{
    let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let claimed:Option<(Uuid,Uuid,String,String)>=sqlx::query_as(
        "WITH candidate AS (SELECT id FROM billing_lifecycle_outbox WHERE (status IN('pending','retry') AND next_attempt_at<=now()) OR (status='sending' AND updated_at<now()-interval '10 minutes') ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1)
         UPDATE billing_lifecycle_outbox b SET status='sending',attempts=attempts+1,updated_at=now() FROM candidate c WHERE b.id=c.id RETURNING b.id,b.organization_id,b.recipient::text,b.kind"
    ).fetch_optional(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let Some((outbox_id,organization_id,recipient,kind))=claimed else{return Ok(());};
    let details:Option<(String,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>)>=sqlx::query_as(
        "SELECT o.name,s.current_period_end,s.renewal_grace_end,s.data_retention_until FROM organizations o JOIN organization_subscriptions s ON s.organization_id=o.id WHERE o.id=$1"
    ).bind(organization_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let Some((organization_name,period_end,grace_end,retention_until))=details else{
        sqlx::query("UPDATE billing_lifecycle_outbox SET status='failed',last_error='organization subscription not found',updated_at=now() WHERE id=$1").bind(outbox_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        return Ok(());
    };
    match crate::services::email::send_subscription_notice(state,&recipient,&organization_name,&kind,period_end,grace_end,retention_until).await{
        Ok(())=>{sqlx::query("UPDATE billing_lifecycle_outbox SET status='sent',sent_at=now(),last_error='',updated_at=now() WHERE id=$1").bind(outbox_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;}
        Err(error)=>{sqlx::query("UPDATE billing_lifecycle_outbox SET status=CASE WHEN attempts>=8 THEN 'failed' ELSE 'retry' END,next_attempt_at=now()+(LEAST(3600,30*GREATEST(1,attempts))::text||' seconds')::interval,last_error=left($2,1000),updated_at=now() WHERE id=$1").bind(outbox_id).bind(error.to_string()).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;}
    }
    Ok(())
}

/// Convenience for tests/handlers that want the raw settings JSON.
pub fn settings_json(s: &BillingSettings) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}
