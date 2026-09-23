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

    let previous: Option<(i64, Json<BTreeMap<String, bool>>)> = sqlx::query_as(
        "SELECT mailbox_bytes, feature_flags FROM plans WHERE code = $1 FOR UPDATE",
    )
    .bind(code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (previous_mailbox_bytes, previous_feature_flags) =
        previous.ok_or_else(|| ApiError::not_found("Plan not found"))?;
    let feature_flags = p
        .feature_flags
        .clone()
        .unwrap_or(previous_feature_flags.0);

    // Editing a catalog plan must not silently strand subscribed businesses
    // above the new capacity. Explicit organization overrides are respected.
    let violation: Option<Uuid> = sqlx::query_scalar(
        "SELECT s.organization_id
         FROM organization_subscriptions s
         WHERE s.plan_code=$1 AND (
           s.purchased_mailbox_count > $2
           OR (s.seat_limit_override IS NULL AND (
             (SELECT count(*) FROM organization_memberships om WHERE om.organization_id=s.organization_id AND om.status IN ('active','invited')) +
             (SELECT count(*) FROM organization_invitations oi WHERE oi.organization_id=s.organization_id AND oi.status='pending')
           ) > GREATEST(s.purchased_mailbox_count,$3))
           OR (s.mailbox_limit_override IS NULL AND
             (SELECT count(*) FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL AND m.status <> 'deleted') > s.purchased_mailbox_count)
           OR (s.domain_limit_override IS NULL AND
             (SELECT count(*) FROM organization_domains d WHERE d.organization_id=s.organization_id AND d.status <> 'removing') > $4)
           OR (s.storage_pool_override_bytes IS NULL AND
             COALESCE((SELECT storage_bytes FROM organization_usage ou WHERE ou.organization_id=s.organization_id),0) >
               ($5::bigint + $6::bigint * GREATEST(s.purchased_mailbox_count-$7,0)::bigint))
           OR (s.storage_pool_override_bytes IS NULL AND
             COALESCE((SELECT SUM(m.quota_bytes) FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL),0) >
               ($5::bigint + $6::bigint * GREATEST(s.purchased_mailbox_count-$7,0)::bigint))
         ) LIMIT 1"
    )
    .bind(code)
    .bind(p.max_mailboxes)
    .bind(p.seats)
    .bind(p.domain_limit)
    .bind(p.storage_pool_bytes)
    .bind(p.mailbox_bytes)
    .bind(p.mailbox_limit)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if violation.is_some() {
        return Err(ApiError::conflict(
            "This plan cannot be reduced below current subscriber usage; add an organization override or reduce usage first",
        ));
    }

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

    // A plan quota edit is synchronized to every live mailbox in every
    // organization subscribed to this plan. User rows are compatibility-only.
    if previous_mailbox_bytes != p.mailbox_bytes {
        let organizations: Vec<Uuid> = sqlx::query_scalar(
            "SELECT organization_id FROM organization_subscriptions WHERE plan_code=$1"
        )
        .bind(code)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        for organization_id in organizations {
            let effective: i64 = sqlx::query_scalar(
                "SELECT COALESCE(mailbox_quota_override_bytes,$2)::bigint
                 FROM organization_subscriptions WHERE organization_id=$1"
            )
            .bind(organization_id)
            .bind(p.mailbox_bytes)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
            let allocation: (i64,i64,i64)=sqlx::query_as(
                "SELECT COALESCE(s.storage_pool_override_bytes,p.storage_pool_bytes::bigint + p.mailbox_bytes::bigint*GREATEST(s.purchased_mailbox_count-p.mailbox_limit,0)::bigint)::bigint,
                        COALESCE((SELECT SUM(m.quota_bytes) FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL AND m.quota_override_bytes IS NOT NULL),0)::bigint,
                        (SELECT count(*)::bigint FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL AND m.quota_override_bytes IS NULL)
                 FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code WHERE s.organization_id=$1"
            ).bind(organization_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
            if allocation.1.saturating_add(allocation.2.saturating_mul(effective)) > allocation.0 {
                return Err(ApiError::conflict("This plan storage change would make existing mailbox allocations exceed the business storage pool"));
            }
            sqlx::query(
                "UPDATE mailboxes SET quota_bytes=$2,updated_at=now()
                 WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NULL"
            ).bind(organization_id).bind(effective).execute(&mut *tx).await
             .map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query(
                "INSERT INTO provisioning_jobs
                  (user_id,organization_id,mailbox_id,operation,target_email,account_id,quota_bytes,status,max_attempts,dedupe_key)
                 SELECT COALESCE(m.user_id,o.created_by),m.organization_id,m.id,'quota',m.address::text,
                        m.provider_account_id,m.quota_bytes,'pending',8,'mailbox.quota:'||m.id::text
                 FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
                 WHERE m.organization_id=$1 AND m.deleted_at IS NULL AND m.status <> 'deleted'
                   AND COALESCE(m.user_id,o.created_by) IS NOT NULL
                 ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
                 DO UPDATE SET quota_bytes=EXCLUDED.quota_bytes,status='pending',attempts=0,
                               next_attempt_at=now(),last_error='',completed_at=NULL,updated_at=now()"
            ).bind(organization_id).execute(&mut *tx).await
             .map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }

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
}

pub async fn settings(state: &AppState) -> Result<BillingSettings, ApiError> {
    sqlx::query_as(
        "SELECT bank_details,paypal_email,instructions,seller_legal_name,seller_email::text,
                seller_cr_number,seller_vat_number,seller_address,tax_rate_bps,invoice_due_days,grace_days
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
          seller_address=$8,tax_rate_bps=$9,invoice_due_days=$10,grace_days=$11,updated_at=now()
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

async fn activate_plan_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    plan_code: &str,
    plan_name: &str,
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
    sqlx::query(
        "INSERT INTO organization_subscriptions
          (organization_id,plan_code,status,purchased_mailbox_count,current_period_start,current_period_end,payment_due_at,grace_period_end,renewal_grace_end,last_order_id,
           assignment_source,assigned_at,assigned_by,assignment_invoice_number,assignment_order_user_id,updated_at)
         VALUES($1,$2,'active',$3,$4,$5,$6,$6+($7::text||' days')::interval,$5+($7::text||' days')::interval,$8,$9,now(),$10,$11,$12,now())
         ON CONFLICT(organization_id) DO UPDATE SET plan_code=EXCLUDED.plan_code,status='active',
          purchased_mailbox_count=EXCLUDED.purchased_mailbox_count,
          storage_pool_override_bytes=NULL,mailbox_quota_override_bytes=NULL,seat_limit_override=NULL,
          mailbox_limit_override=NULL,domain_limit_override=NULL,organization_daily_send_override=NULL,
          current_period_start=EXCLUDED.current_period_start,current_period_end=EXCLUDED.current_period_end,
          payment_due_at=EXCLUDED.payment_due_at,grace_period_end=EXCLUDED.grace_period_end,renewal_grace_end=EXCLUDED.renewal_grace_end,last_order_id=EXCLUDED.last_order_id,
          assignment_source=EXCLUDED.assignment_source,assigned_at=now(),assigned_by=EXCLUDED.assigned_by,
          assignment_invoice_number=EXCLUDED.assignment_invoice_number,assignment_order_user_id=EXCLUDED.assignment_order_user_id,
          cancelled_at=NULL,updated_at=now()",
    )
    .bind(organization_id)
    .bind(plan_code)
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
    .map_err(|e| ApiError::internal(e.to_string()))?;

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
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE orders SET subscription_assigned_at=now(),subscription_assigned_by=$2,updated_at=now() WHERE id=$1",
    )
    .bind(order_id)
    .bind(assigned_by)
    .execute(&mut **tx)
    .await
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
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query("UPDATE mailboxes SET quota_bytes=$2,updated_at=now() WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NULL")
        .bind(organization_id)
        .bind(plan_mailbox_bytes)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO provisioning_jobs
          (user_id,organization_id,mailbox_id,operation,target_email,account_id,quota_bytes,status,max_attempts,dedupe_key)
         SELECT COALESCE(m.user_id,o.created_by),m.organization_id,m.id,'quota',m.address::text,m.provider_account_id,
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
    Ok(())
}


/// Keep manual-renewal subscription state aligned with authoritative period
/// dates. Entitlement reads also fail closed on expired periods, so this worker
/// primarily persists the operator-visible lifecycle and immutable history.
pub async fn reconcile_subscription_lifecycle(state: &AppState) -> Result<(), ApiError> {
    let expired: Vec<(Uuid, String, String, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "WITH changed AS (
           UPDATE organization_subscriptions s
           SET status='past_due',
               renewal_grace_end=COALESCE(s.renewal_grace_end,
                 s.current_period_end + ((SELECT grace_days FROM billing_settings WHERE id=TRUE)::text || ' days')::interval),
               updated_at=now()
           WHERE s.status IN ('active','trial')
             AND s.current_period_end IS NOT NULL AND s.current_period_end <= now()
           RETURNING s.organization_id,s.plan_code,s.purchased_mailbox_count,s.current_period_start,s.current_period_end
         )
         SELECT c.organization_id,c.plan_code,p.name,c.purchased_mailbox_count,c.current_period_start,c.current_period_end
         FROM changed c JOIN plans p ON p.code=c.plan_code"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    for (organization_id, plan_code, plan_name, mailbox_count, period_start, period_end) in expired {
        sqlx::query(
            "INSERT INTO subscription_assignment_history
              (organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,
               event_type,period_start,period_end,status_after,reason,detail)
             VALUES($1,$2,$3,$4,'system_lifecycle',FALSE,'status',$5,$6,'past_due','Subscription period expired',$7)"
        )
        .bind(organization_id).bind(plan_code).bind(plan_name).bind(mailbox_count)
        .bind(period_start).bind(period_end)
        .bind(Json(serde_json::json!({"transition":"active_to_past_due"})))
        .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    }

    let suspended: Vec<(Uuid, String, String, i32, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "WITH changed AS (
           UPDATE organization_subscriptions s
           SET status='suspended',updated_at=now()
           WHERE s.status='past_due' AND s.renewal_grace_end IS NOT NULL AND s.renewal_grace_end <= now()
           RETURNING s.organization_id,s.plan_code,s.purchased_mailbox_count,s.current_period_start,s.current_period_end
         )
         SELECT c.organization_id,c.plan_code,p.name,c.purchased_mailbox_count,c.current_period_start,c.current_period_end
         FROM changed c JOIN plans p ON p.code=c.plan_code"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    for (organization_id, plan_code, plan_name, mailbox_count, period_start, period_end) in suspended {
        sqlx::query(
            "INSERT INTO subscription_assignment_history
              (organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,
               event_type,period_start,period_end,status_after,reason,detail)
             VALUES($1,$2,$3,$4,'system_lifecycle',FALSE,'status',$5,$6,'suspended','Renewal grace period expired',$7)"
        )
        .bind(organization_id).bind(plan_code).bind(plan_name).bind(mailbox_count)
        .bind(period_start).bind(period_end)
        .bind(Json(serde_json::json!({"transition":"past_due_to_suspended"})))
        .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(())
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
    let plan = entitlements::active_plan(state, plan_code).await?;
    let organization_id = entitlements::active_organization_for_user(state, user_id).await?;
    entitlements::validate_plan_change_capacity_with_mailboxes(state, organization_id, plan_code, mailbox_count).await?;
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

    let invoice_number: String = sqlx::query_scalar(
        "SELECT 'INV-'||to_char(now(),'YYYY')||'-'||lpad(nextval('invoice_number_seq')::text,6,'0')",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let activation_mode = if state.billing_instant_activation { "test_instant" } else { "payment_approval" };
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders
          (user_id,organization_id,plan_code,plan_name,amount_cents,currency,interval,seats,mailbox_count,included_mailbox_count,
           extra_mailbox_count,extra_mailbox_unit_price_cents,base_price_cents,payment_method,customer_note,
           invoice_number,invoice_status,issued_at,due_at,subtotal_cents,tax_rate_bps,tax_cents,total_cents,
           seller_snapshot,buyer_snapshot,period_start,period_end,activation_mode,activated_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'issued',now(),$17,$18,$19,$20,$21,$22,$23,
                CASE WHEN $24='test_instant' THEN now() ELSE NULL END,
                CASE WHEN $24='test_instant' THEN now()+CASE WHEN $7='year' THEN interval '1 year' ELSE interval '1 month' END ELSE NULL END,
                $24,CASE WHEN $24='test_instant' THEN now() ELSE NULL END) RETURNING id",
    )
    .bind(user_id).bind(organization_id).bind(&plan.code).bind(&plan.name).bind(total as i32)
    .bind(&plan.currency).bind(&plan.interval).bind(mailbox_count).bind(mailbox_count).bind(plan.mailbox_limit)
    .bind(extra_mailbox_count).bind(plan.extra_mailbox_price_cents as i32).bind(base_price as i32).bind(payment_method).bind(customer_note)
    .bind(&invoice_number).bind(&due_at).bind(subtotal as i32).bind(effective_tax_bps as i32).bind(tax as i32)
    .bind(total as i32).bind(Json(seller_snapshot)).bind(Json(buyer_snapshot.clone())).bind(activation_mode)
    .fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;

    if state.billing_instant_activation {
        let (period_start, period_end): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "SELECT period_start,period_end FROM orders WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        activate_plan_tx(
            &mut tx, organization_id, &plan.code, &plan.name, plan.mailbox_bytes as i64, mailbox_count,
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
    let row: Option<(Uuid,Uuid,String,String,String,String,i64,i32,chrono::DateTime<chrono::Utc>,i32,String,String,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT user_id,organization_id,plan_code,plan_name,status,interval,
                (SELECT mailbox_bytes FROM plans WHERE code=orders.plan_code)::bigint,mailbox_count,
                COALESCE(due_at,now()),
                (SELECT grace_days FROM billing_settings WHERE id=TRUE),activation_mode,COALESCE(invoice_number,''),period_start,period_end
         FROM orders WHERE id=$1 FOR UPDATE",
    )
    .bind(id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (order_user_id, organization_id, plan_code, plan_name, status, _interval, mailbox_bytes, mailbox_count, due_at, grace_days, _activation_mode, invoice_number, period_start, period_end) =
        row.ok_or_else(|| ApiError::not_found("Order not found"))?;
    if status != "submitted" {
        return Err(ApiError::conflict("Payment must be submitted with a reference before this invoice can be approved"));
    }
    entitlements::validate_plan_change_capacity_with_mailboxes(state, organization_id, &plan_code, mailbox_count).await?;

    let (period_start, period_end) = match (period_start, period_end) {
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
    };

    // The reviewed invoice is authoritative even in acceptance-test instant
    // mode. Re-assert exactly the ordered plan + mailbox quantity so a paid
    // invoice can never point at a different active subscription.
    activate_plan_tx(
        &mut tx, organization_id, &plan_code, &plan_name, mailbox_bytes, mailbox_count,
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
        .bind(id).bind(recipient).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
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

/// Convenience for tests/handlers that want the raw settings JSON.
pub fn settings_json(s: &BillingSettings) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}
