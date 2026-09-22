//! WS4 manual-payment billing. Customers order a plan and pay out-of-band;
//! admins review orders on this same surface and approve/reject them, which
//! activates the plan. No external payment processor is in the loop.

use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::domain::quota::PlanLimits;
use crate::error::ApiError;
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::services::{billing, entitlements};
use crate::state::AppState;

const PAYMENT_METHODS: [&str; 4] = ["bank", "paypal", "card", "other"];

fn plan_json(p: &PlanLimits) -> Value {
    json!({
        "code": p.code,
        "name": p.name,
        "price_cents": p.price_cents,
        "extra_mailbox_price_cents": p.extra_mailbox_price_cents,
        "price": p.price_display(),
        "currency": p.currency,
        "interval": p.interval,
        "mailbox_bytes": p.mailbox_bytes,
        "storage_pool_bytes": p.storage_pool_bytes,
        "mailbox_limit": p.mailbox_limit,
        "max_mailboxes": p.max_mailboxes,
        "alias_limit_per_mailbox": p.alias_limit_per_mailbox,
        "domain_limit": p.domain_limit,
        "organization_daily_send_limit": p.organization_daily_send_limit,
        "max_attachment_bytes": p.max_attachment_bytes,
        "max_recipients": p.max_recipients,
        "daily_send_limit": p.daily_send_limit,
        "seats": p.seats,
        "features": p.features,
        "feature_flags": p.feature_flags,
        "active": p.active,
    })
}

/// `GET /api/billing` — everything the billing page needs in one round trip.
pub async fn summary(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let user_entitlements = entitlements::for_user(&state, auth.user_id).await?;
    let current = user_entitlements.plan.clone();
    let settings = billing::settings(&state).await?;
    let billing_profile = billing::billing_profile(&state, user_entitlements.organization_id).await?;
    let orders = billing::orders_for_user(&state, auth.user_id).await?;
    let usage: (i64,i64,i64,i64) = sqlx::query_as(
        "SELECT (SELECT count(*)::bigint FROM organization_memberships WHERE organization_id=$1 AND status IN ('active','invited')),
                (SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status <> 'deleted'),
                (SELECT count(*)::bigint FROM organization_domains WHERE organization_id=$1 AND status <> 'removing'),
                COALESCE((SELECT sum(quota_bytes)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status <> 'deleted'),0)::bigint"
    ).bind(user_entitlements.organization_id).fetch_one(&state.db).await
     .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "organization_id": user_entitlements.organization_id,
        "subscription_status": user_entitlements.subscription_status,
        "current_plan": plan_json(&current),
        "quota_bytes": user_entitlements.quota_bytes,
        "mailbox_quota_bytes": user_entitlements.quota_bytes,
        "storage_pool_bytes": user_entitlements.storage_pool_bytes,
        "storage_allocated_bytes": usage.3,
        "seat_limit": user_entitlements.seat_limit,
        "mailbox_limit": user_entitlements.mailbox_limit,
        "domain_limit": user_entitlements.domain_limit,
        "organization_daily_send_limit": user_entitlements.organization_daily_send_limit,
        "usage": {"seats": usage.0, "mailboxes": usage.1, "domains": usage.2},
        "quota_override_bytes": user_entitlements.quota_override_bytes,
        "quota_source": if user_entitlements.quota_is_overridden() { "override" } else { "plan" },
        "settings": settings,
        "billing_profile": billing_profile,
        "instant_activation": state.billing_instant_activation,
        "orders": orders,
    })))
}

/// `GET /api/billing/plans` — active plans for the pricing/upgrade view.
pub async fn plans(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let plans: Vec<Value> = billing::active_plans(&state)
        .await?
        .iter()
        .map(plan_json)
        .collect();
    Ok(Json(json!({ "plans": plans })))
}

#[derive(Deserialize)]
pub struct OrderIn {
    plan_code: String,
    #[serde(default)]
    mailbox_count: Option<i32>,
    /// `bank`, `paypal`, `card` or `other`.
    payment_method: String,
    #[serde(default)]
    customer_note: String,
}

/// `POST /api/billing/orders` — open a pending order for an active plan.
pub async fn create_order(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<OrderIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    crate::services::platform_control::require_plan_ordering(&state.db).await?;
    let method = body.payment_method.trim().to_string();
    if !PAYMENT_METHODS.contains(&method.as_str()) {
        return Err(ApiError::bad_request("Invalid payment method"));
    }
    if body.plan_code.trim().is_empty() {
        return Err(ApiError::bad_request("A plan is required"));
    }
    let note = body
        .customer_note
        .trim()
        .chars()
        .take(500)
        .collect::<String>();
    let catalog_plan = billing::for_code(&state, &body.plan_code).await?;
    let mailbox_count = body.mailbox_count.unwrap_or(catalog_plan.mailbox_limit);
    let order = billing::create_order(
        &state, auth.user_id, &body.plan_code, mailbox_count, &method, &note,
    ).await?;

    audit::record(
        &state,
        Some(auth.user_id),
        "billing.order_create",
        json!({ "order_id": order.id, "plan": order.plan_code, "mailbox_count": order.mailbox_count, "invoice": order.invoice_number, "activation_mode": order.activation_mode }),
    )
    .await;
    let message = if order.activation_mode == "test_instant" {
        format!("{} is active immediately for testing. Invoice {} remains due for manual payment.", order.plan_name, order.invoice_number.clone().unwrap_or_default())
    } else {
        format!("Invoice {} was issued for {}. The plan activates after payment approval.", order.invoice_number.clone().unwrap_or_default(), order.plan_name)
    };
    let _ = crate::services::notifications::create(
        &state, auth.user_id, "billing", "Invoice issued", &message, "/mail/billing",
        Some(&format!("billing:invoice:{}", order.id)),
    ).await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(&order).unwrap_or(Value::Null)),
    ))
}

#[derive(Deserialize)]
pub struct SubmitIn {
    payment_method: String,
    payment_reference: String,
}

/// `POST /api/billing/orders/:id/paid` — customer marks a pending order as paid,
/// supplying the bank/PayPal reference or transaction id they used.
pub async fn submit_paid(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SubmitIn>,
) -> Result<Json<Value>, ApiError> {
    let method = body.payment_method.trim().to_string();
    if !PAYMENT_METHODS.contains(&method.as_str()) {
        return Err(ApiError::bad_request("Invalid payment method"));
    }
    let reference = body.payment_reference.trim();
    if reference.is_empty() {
        return Err(ApiError::bad_request("Payment reference is required"));
    }
    if reference.chars().count() > 200 {
        return Err(ApiError::bad_request("Payment reference is too long"));
    }
    let order = billing::submit_order(&state, auth.user_id, id, &method, reference).await?;
    audit::record(
        &state,
        Some(auth.user_id),
        "billing.order_paid_submitted",
        json!({ "order_id": id }),
    )
    .await;
    Ok(Json(serde_json::to_value(&order).unwrap_or(Value::Null)))
}

/// `POST /api/billing/orders/:id/cancel` — customer withdraws an open order.
pub async fn cancel_order(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    billing::cancel_order(&state, auth.user_id, id).await?;
    audit::record(
        &state,
        Some(auth.user_id),
        "billing.order_cancel",
        json!({ "order_id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/billing/invoices` — paid orders (the customer's invoice list).
pub async fn invoices(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let orders = billing::orders_for_user(&state, auth.user_id).await?;
    let invoices: Vec<Value> = orders
        .iter()
        .filter(|o| o.invoice_number.is_some())
        .map(|o| serde_json::to_value(o).unwrap_or(Value::Null))
        .collect();
    Ok(Json(json!({ "invoices": invoices })))
}

// ---------------------------------------------------------------------------
// Admin surface
// ---------------------------------------------------------------------------

/// `GET /api/admin/plans` — every plan, active or not (admin CRUD view).
pub async fn admin_plans(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let plans: Vec<Value> = billing::all_plans(&state)
        .await?
        .iter()
        .map(plan_json)
        .collect();
    Ok(Json(json!({ "plans": plans })))
}

#[derive(Deserialize)]
pub struct PlanIn {
    code: String,
    name: String,
    price_cents: i64,
    #[serde(default)]
    extra_mailbox_price_cents: i64,
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_interval")]
    interval: String,
    mailbox_bytes: i64,
    #[serde(default)]
    storage_pool_bytes: i64,
    #[serde(default)]
    mailbox_limit: i64,
    #[serde(default)]
    max_mailboxes: i64,
    #[serde(default)]
    alias_limit_per_mailbox: Option<i64>,
    #[serde(default)]
    domain_limit: i64,
    #[serde(default)]
    organization_daily_send_limit: i64,
    max_attachment_bytes: i64,
    max_recipients: i64,
    #[serde(default)]
    daily_send_limit: i64,
    #[serde(default = "one")]
    seats: i64,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    feature_flags: Option<BTreeMap<String, bool>>,
    #[serde(default)]
    sort_order: i64,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_currency() -> String {
    "SAR".into()
}
fn default_interval() -> String {
    "year".into()
}
fn one() -> i64 {
    1
}
fn default_true() -> bool {
    true
}

fn validate_plan_input(body: &PlanIn) -> Result<billing::PlanInput, ApiError> {
    let storage_pool_bytes = if body.storage_pool_bytes > 0 { body.storage_pool_bytes } else { body.mailbox_bytes };
    let mailbox_limit = if body.mailbox_limit > 0 { body.mailbox_limit } else { body.seats };
    let max_mailboxes = if body.max_mailboxes > 0 { body.max_mailboxes } else { mailbox_limit };
    let domain_limit = if body.domain_limit > 0 { body.domain_limit } else { 1 };
    let organization_daily_send_limit = if body.organization_daily_send_limit > 0 { body.organization_daily_send_limit } else { body.daily_send_limit };
    if body.code.trim().is_empty() || body.name.trim().is_empty() {
        return Err(ApiError::bad_request("Plan code and name are required"));
    }
    if !body.interval.is_empty() && !matches!(body.interval.as_str(), "month" | "year") {
        return Err(ApiError::bad_request("Interval must be 'month' or 'year'"));
    }
    let required_base_pool = body.mailbox_bytes.saturating_mul(mailbox_limit);
    if body.price_cents < 0
        || body.price_cents > i32::MAX as i64
        || body.extra_mailbox_price_cents < 0
        || body.extra_mailbox_price_cents > i32::MAX as i64
        || body.mailbox_bytes <= 0
        || storage_pool_bytes <= 0
        || storage_pool_bytes < required_base_pool
        || mailbox_limit <= 0
        || max_mailboxes < mailbox_limit
        || max_mailboxes > 500
        || body.alias_limit_per_mailbox.is_some_and(|value| value <= 0)
        || domain_limit <= 0
        || organization_daily_send_limit < 0
        || body.max_attachment_bytes <= 0
        || body.max_attachment_bytes > crate::handlers::attachments::MAX_ATTACHMENT_BYTES as i64
        || body.max_recipients <= 0
        || body.daily_send_limit < 0
        || body.seats <= 0
    {
        return Err(ApiError::bad_request(
            "Numeric limits are invalid; the base storage pool must cover every included mailbox at its default quota, and attachments may be at most 100 MiB (daily limit may be zero)",
        ));
    }
    if let Some(flags) = body.feature_flags.as_ref() {
        entitlements::validate_feature_flags(flags)?;
    }
    Ok(billing::PlanInput {
        code: body.code.trim().to_lowercase().replace(' ', "-"),
        name: body.name.trim().to_string(),
        price_cents: body.price_cents,
        extra_mailbox_price_cents: body.extra_mailbox_price_cents,
        currency: body.currency.trim().to_uppercase(),
        interval: body.interval.clone(),
        mailbox_bytes: body.mailbox_bytes,
        storage_pool_bytes,
        mailbox_limit,
        max_mailboxes,
        alias_limit_per_mailbox: body.alias_limit_per_mailbox,
        domain_limit,
        organization_daily_send_limit,
        max_attachment_bytes: body.max_attachment_bytes,
        max_recipients: body.max_recipients,
        daily_send_limit: body.daily_send_limit,
        seats: body.seats,
        features: body.features.clone(),
        feature_flags: body.feature_flags.clone(),
        sort_order: body.sort_order,
        active: body.active,
    })
}

/// `POST /api/admin/plans` — create a plan row.
pub async fn admin_create_plan(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<PlanIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let input = validate_plan_input(&body)?;
    billing::create_plan(&state, &input).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.plan_create",
        json!({ "code": input.code }),
    )
    .await;
    let plan = billing::for_code(&state, &input.code).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "plan": plan_json(&plan) })),
    ))
}

/// `PATCH /api/admin/plans/:code` — edit a plan row (code is immutable).
pub async fn admin_update_plan(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(code): Path<String>,
    Json(body): Json<PlanIn>,
) -> Result<Json<Value>, ApiError> {
    let input = validate_plan_input(&body)?;
    billing::update_plan(&state, &code, &input).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.plan_update",
        json!({ "code": code }),
    )
    .await;
    let plan = billing::for_code(&state, &code).await?;
    Ok(Json(json!({ "plan": plan_json(&plan) })))
}

/// `DELETE /api/admin/plans/:code` — hide a plan from new orders.
pub async fn admin_deactivate_plan(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(code): Path<String>,
) -> Result<Json<Value>, ApiError> {
    billing::deactivate_plan(&state, &code).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.plan_deactivate",
        json!({ "code": code }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct OrderQuery {
    #[serde(default)]
    status: Option<String>,
}

/// `GET /api/admin/orders?status=` — the payment queue for manual review.
pub async fn admin_orders(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<OrderQuery>,
) -> Result<Json<Value>, ApiError> {
    if let Some(status) = &query.status {
        if status != "paid" && status != "cancelled" && status != "rejected" {
            return Err(ApiError::bad_request(
                "Status filter must be paid, cancelled or rejected (omit for the open queue)",
            ));
        }
    }
    let orders = billing::all_orders(&state, query.status.as_deref()).await?;
    Ok(Json(json!({ "orders": orders })))
}

#[derive(Deserialize)]
pub struct ReviewIn {
    #[serde(default)]
    admin_note: String,
}

/// `POST /api/admin/orders/:id/approve` — mark paid, invoice it, activate plan.
pub async fn admin_approve_order(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ReviewIn>,
) -> Result<Json<Value>, ApiError> {
    let order = billing::approve_order(&state, admin.0.user_id, id, body.admin_note.trim()).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.order_approve",
        json!({
            "order_id": id,
            "invoice": order.invoice_number,
            "organization_id": order.organization_id,
            "plan_code": order.plan_code,
            "mailbox_count": order.mailbox_count,
            "assignment_source": "payment_approval"
        }),
    )
    .await;
    let _ = crate::services::notifications::create(
        &state,
        order.user_id,
        "billing",
        "Payment confirmed and plan assigned",
        &format!(
            "Payment for invoice {} has been confirmed. {} with {} mailbox{} is now the authoritative business subscription.",
            order.invoice_number.clone().unwrap_or_default(),
            order.plan_name,
            order.mailbox_count,
            if order.mailbox_count == 1 { "" } else { "es" }
        ),
        "/mail/billing",
        Some(&format!("billing:approved:{}", order.id)),
    )
    .await;
    Ok(Json(serde_json::to_value(&order).unwrap_or(Value::Null)))
}

/// `POST /api/admin/orders/:id/reject` — refuse an order with a note.
pub async fn admin_reject_order(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ReviewIn>,
) -> Result<Json<Value>, ApiError> {
    let order = billing::reject_order(&state, admin.0.user_id, id, body.admin_note.trim()).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.order_reject",
        json!({ "order_id": id }),
    )
    .await;
    let _ = crate::services::notifications::create(
        &state,
        order.user_id,
        "billing",
        "Payment review needs attention",
        "Your plan order was not approved. Open Billing to review the order status and support options.",
        "/mail/billing",
        Some(&format!("billing:rejected:{}", order.id)),
    )
    .await;
    Ok(Json(serde_json::to_value(&order).unwrap_or(Value::Null)))
}

/// `GET /api/admin/billing-settings` — the payment instructions shown to users.
pub async fn admin_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<billing::BillingSettings>, ApiError> {
    Ok(Json(billing::settings(&state).await?))
}

#[derive(Deserialize)]
pub struct SettingsIn {
    #[serde(default)] bank_details: String,
    #[serde(default)] paypal_email: String,
    #[serde(default)] instructions: String,
    #[serde(default)] seller_legal_name: String,
    #[serde(default)] seller_email: String,
    #[serde(default)] seller_cr_number: String,
    #[serde(default)] seller_vat_number: String,
    #[serde(default)] seller_address: String,
    #[serde(default = "default_tax_rate_bps")] tax_rate_bps: i32,
    #[serde(default = "default_invoice_due_days")] invoice_due_days: i32,
    #[serde(default = "default_grace_days")] grace_days: i32,
}
fn default_tax_rate_bps() -> i32 { 1500 }
fn default_invoice_due_days() -> i32 { 7 }
fn default_grace_days() -> i32 { 7 }

/// `PUT /api/admin/billing-settings` — update bank/PayPal details.
pub async fn admin_update_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SettingsIn>,
) -> Result<Json<billing::BillingSettings>, ApiError> {
    if !(0..=10000).contains(&body.tax_rate_bps) || !(0..=90).contains(&body.invoice_due_days) || !(0..=90).contains(&body.grace_days) {
        return Err(ApiError::bad_request("Invalid tax, due-day or grace-day setting"));
    }
    let input = billing::BillingSettingsInput {
        bank_details: body.bank_details.trim().to_string(),
        paypal_email: body.paypal_email.trim().to_string(),
        instructions: body.instructions.trim().to_string(),
        seller_legal_name: body.seller_legal_name.trim().to_string(),
        seller_email: body.seller_email.trim().to_string(),
        seller_cr_number: body.seller_cr_number.trim().to_string(),
        seller_vat_number: body.seller_vat_number.trim().to_string(),
        seller_address: body.seller_address.trim().to_string(),
        tax_rate_bps: body.tax_rate_bps,
        invoice_due_days: body.invoice_due_days,
        grace_days: body.grace_days,
    };
    billing::update_settings(&state, &input).await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.settings_update",
        json!({}),
    )
    .await;
    Ok(Json(billing::settings(&state).await?))
}

#[derive(Deserialize)]
pub struct BillingProfileIn {
    #[serde(default)] legal_name: String,
    #[serde(default)] billing_email: String,
    #[serde(default)] vat_number: String,
    #[serde(default)] cr_number: String,
    #[serde(default)] address_line1: String,
    #[serde(default)] address_line2: String,
    #[serde(default)] city: String,
    #[serde(default)] postal_code: String,
    #[serde(default)] country: String,
}

pub async fn billing_profile(
    State(state): State<AppState>, auth: AuthUser,
) -> Result<Json<billing::BillingProfile>, ApiError> {
    let organization_id = entitlements::active_organization_for_user(&state,auth.user_id).await?;
    Ok(Json(billing::billing_profile(&state,organization_id).await?))
}

pub async fn update_billing_profile(
    State(state): State<AppState>, auth: AuthUser, Json(body): Json<BillingProfileIn>,
) -> Result<Json<billing::BillingProfile>, ApiError> {
    let organization_id = entitlements::active_organization_for_user(&state,auth.user_id).await?;
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 AND status='active'")
        .bind(organization_id).bind(auth.user_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if !matches!(role.as_deref(),Some("owner")|Some("billing")|Some("admin")) { return Err(ApiError::forbidden("Billing access required")); }
    let email=body.billing_email.trim();
    if !email.is_empty() && (!email.contains('@') || email.len()>320) { return Err(ApiError::bad_request("Enter a valid billing email")); }
    let profile=billing::BillingProfile {
        organization_id, legal_name:body.legal_name.trim().chars().take(160).collect(), billing_email:email.to_string(),
        vat_number:body.vat_number.trim().chars().take(64).collect(), cr_number:body.cr_number.trim().chars().take(64).collect(),
        address_line1:body.address_line1.trim().chars().take(200).collect(), address_line2:body.address_line2.trim().chars().take(200).collect(),
        city:body.city.trim().chars().take(120).collect(), postal_code:body.postal_code.trim().chars().take(32).collect(),
        country:if body.country.trim().is_empty(){"Saudi Arabia".to_string()}else{body.country.trim().chars().take(120).collect()},
    };
    billing::update_billing_profile(&state,&profile).await?;
    audit::record(&state,Some(auth.user_id),"billing.profile_update",json!({"organization_id":organization_id})).await;
    Ok(Json(billing::billing_profile(&state,organization_id).await?))
}

#[derive(Deserialize)]
pub struct SubscriptionPatchIn {
    plan_code: String,
    #[serde(default)] status: Option<String>,
    #[serde(default)] purchased_mailbox_count: Option<i32>,
    #[serde(default)] storage_pool_override_bytes: Option<i64>,
    #[serde(default)] mailbox_quota_override_bytes: Option<i64>,
    #[serde(default)] seat_limit_override: Option<i32>,
    #[serde(default)] mailbox_limit_override: Option<i32>,
    #[serde(default)] domain_limit_override: Option<i32>,
    #[serde(default)] organization_daily_send_override: Option<i32>,
    #[serde(default)] current_period_end: Option<String>,
    #[serde(default)] reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct AdminSubscriptionsQuery {
    q: Option<String>,
    status: Option<String>,
    plan: Option<String>,
    organization_id: Option<Uuid>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct AdminSubscriptionRow {
    organization_id: Uuid,
    organization_name: String,
    is_system: bool,
    organization_created_at: chrono::DateTime<chrono::Utc>,
    plan_code: String,
    status: String,
    plan_name: String,
    purchased_mailbox_count: i32,
    seat_count: i64,
    mailbox_count: i64,
    domain_count: i64,
    storage_used_bytes: i64,
    storage_allocated_bytes: i64,
    storage_pool_bytes: i64,
    assignment_source: String,
    assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    assignment_invoice_number: Option<String>,
    assignment_order_user_email: Option<String>,
    assigned_by_email: Option<String>,
    last_order_id: Option<Uuid>,
    current_period_start: chrono::DateTime<chrono::Utc>,
    current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    renewal_grace_end: Option<chrono::DateTime<chrono::Utc>>,
    payment_due_at: Option<chrono::DateTime<chrono::Utc>>,
    grace_period_end: Option<chrono::DateTime<chrono::Utc>>,
    cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
    owner_email: Option<String>,
    billing_email: Option<String>,
    last_paid_at: Option<chrono::DateTime<chrono::Utc>>,
    paid_invoice_count: i64,
    total_paid_cents: i64,
}

/// Platform-admin inventory of every business subscription and its complete
/// operational lifecycle. This is the authoritative "who has which plan"
/// view used by the localhost-only control plane.
pub async fn admin_subscriptions(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<AdminSubscriptionsQuery>,
) -> Result<Json<Value>, ApiError> {
    billing::reconcile_subscription_lifecycle(&state).await?;
    let q = query.q.unwrap_or_default().trim().chars().take(200).collect::<String>();
    let status = query.status.filter(|value| !value.trim().is_empty());
    if status.as_deref().is_some_and(|value| !matches!(value, "trial" | "active" | "past_due" | "suspended" | "cancelled")) {
        return Err(ApiError::bad_request("Invalid subscription status filter"));
    }
    let plan = query.plan.filter(|value| !value.trim().is_empty());
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM organizations o JOIN organization_subscriptions s ON s.organization_id=o.id
         JOIN plans p ON p.code=s.plan_code
         LEFT JOIN organization_billing_profiles bp ON bp.organization_id=o.id
         WHERE ($1::text='' OR o.name ILIKE '%'||$1||'%' OR p.name ILIKE '%'||$1||'%' OR p.code ILIKE '%'||$1||'%'
            OR COALESCE(bp.billing_email::text,'') ILIKE '%'||$1||'%' OR COALESCE(s.assignment_invoice_number,'') ILIKE '%'||$1||'%'
            OR EXISTS (SELECT 1 FROM organization_memberships om JOIN users u ON u.id=om.user_id WHERE om.organization_id=o.id AND om.role='owner' AND u.email::text ILIKE '%'||$1||'%'))
           AND ($2::text IS NULL OR s.status=$2)
           AND ($3::text IS NULL OR s.plan_code=$3)
           AND ($4::uuid IS NULL OR o.id=$4)"
    ).bind(&q).bind(status.as_deref()).bind(plan.as_deref()).bind(query.organization_id)
     .fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let rows: Vec<AdminSubscriptionRow> = sqlx::query_as(
        "SELECT o.id AS organization_id,o.name AS organization_name,o.is_system,o.created_at AS organization_created_at,
          s.plan_code,s.status,p.name AS plan_name,s.purchased_mailbox_count,
          (SELECT count(*)::bigint FROM organization_memberships om WHERE om.organization_id=o.id AND om.status IN ('active','invited')) AS seat_count,
          (SELECT count(*)::bigint FROM mailboxes m WHERE m.organization_id=o.id AND m.deleted_at IS NULL AND m.status <> 'deleted') AS mailbox_count,
          (SELECT count(*)::bigint FROM organization_domains d WHERE d.organization_id=o.id AND d.status <> 'removing') AS domain_count,
          COALESCE((SELECT storage_bytes FROM organization_usage ou WHERE ou.organization_id=o.id),0)::bigint AS storage_used_bytes,
          COALESCE((SELECT sum(m.quota_bytes) FROM mailboxes m WHERE m.organization_id=o.id AND m.deleted_at IS NULL),0)::bigint AS storage_allocated_bytes,
          COALESCE(s.storage_pool_override_bytes,p.storage_pool_bytes::bigint + p.mailbox_bytes::bigint*GREATEST(s.purchased_mailbox_count-p.mailbox_limit,0)::bigint)::bigint AS storage_pool_bytes,
          s.assignment_source,s.assigned_at,s.assignment_invoice_number,au.email::text AS assignment_order_user_email,
          ab.email::text AS assigned_by_email,s.last_order_id,s.current_period_start,s.current_period_end,
          s.renewal_grace_end,s.payment_due_at,s.grace_period_end,s.cancelled_at,
          (SELECT u.email::text FROM organization_memberships om JOIN users u ON u.id=om.user_id
             WHERE om.organization_id=o.id AND om.role='owner' AND om.status='active' ORDER BY om.joined_at LIMIT 1) AS owner_email,
          bp.billing_email::text AS billing_email,
          (SELECT max(ord.paid_at) FROM orders ord WHERE ord.organization_id=o.id AND ord.invoice_status='paid') AS last_paid_at,
          (SELECT count(*)::bigint FROM orders ord WHERE ord.organization_id=o.id AND ord.invoice_status='paid') AS paid_invoice_count,
          COALESCE((SELECT sum(ord.total_cents)::bigint FROM orders ord WHERE ord.organization_id=o.id AND ord.invoice_status='paid'),0)::bigint AS total_paid_cents
         FROM organizations o JOIN organization_subscriptions s ON s.organization_id=o.id
         JOIN plans p ON p.code=s.plan_code
         LEFT JOIN users au ON au.id=s.assignment_order_user_id
         LEFT JOIN users ab ON ab.id=s.assigned_by
         LEFT JOIN organization_billing_profiles bp ON bp.organization_id=o.id
         WHERE ($1::text='' OR o.name ILIKE '%'||$1||'%' OR p.name ILIKE '%'||$1||'%' OR p.code ILIKE '%'||$1||'%'
            OR COALESCE(bp.billing_email::text,'') ILIKE '%'||$1||'%' OR COALESCE(s.assignment_invoice_number,'') ILIKE '%'||$1||'%'
            OR EXISTS (SELECT 1 FROM organization_memberships omq JOIN users uq ON uq.id=omq.user_id WHERE omq.organization_id=o.id AND omq.role='owner' AND uq.email::text ILIKE '%'||$1||'%'))
           AND ($2::text IS NULL OR s.status=$2)
           AND ($3::text IS NULL OR s.plan_code=$3)
           AND ($4::uuid IS NULL OR o.id=$4)
         ORDER BY o.is_system DESC,lower(o.name) LIMIT $5 OFFSET $6"
    ).bind(&q).bind(status.as_deref()).bind(plan.as_deref()).bind(query.organization_id).bind(limit).bind(offset)
     .fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"subscriptions": rows.into_iter().map(|r| json!({
        "organization_id":r.organization_id,"organization_name":r.organization_name,"is_system":r.is_system,
        "organization_created_at":r.organization_created_at,"plan_code":r.plan_code,"status":r.status,"plan_name":r.plan_name,
        "purchased_mailbox_count":r.purchased_mailbox_count,
        "usage":{"seats":r.seat_count,"mailboxes":r.mailbox_count,"domains":r.domain_count,"storage_bytes":r.storage_used_bytes},
        "storage_allocated_bytes":r.storage_allocated_bytes,"storage_pool_bytes":r.storage_pool_bytes,
        "assignment_source":r.assignment_source,"assigned_at":r.assigned_at,"assignment_invoice_number":r.assignment_invoice_number,
        "assignment_order_user_email":r.assignment_order_user_email,"assigned_by_email":r.assigned_by_email,"last_order_id":r.last_order_id,
        "current_period_start":r.current_period_start,"current_period_end":r.current_period_end,"renewal_grace_end":r.renewal_grace_end,
        "payment_due_at":r.payment_due_at,"grace_period_end":r.grace_period_end,"cancelled_at":r.cancelled_at,
        "owner_email":r.owner_email,"billing_email":r.billing_email,"last_paid_at":r.last_paid_at,
        "paid_invoice_count":r.paid_invoice_count,"total_paid_cents":r.total_paid_cents
    })).collect::<Vec<_>>(), "total": total, "limit": limit, "offset": offset })))
}

#[derive(sqlx::FromRow)]
struct SubscriptionHistoryRow {
    id: Uuid,
    assigned_at: chrono::DateTime<chrono::Utc>,
    event_type: String,
    assignment_source: String,
    payment_confirmed: bool,
    plan_code: String,
    plan_name: String,
    purchased_mailbox_count: i32,
    invoice_number: Option<String>,
    order_user_email: Option<String>,
    assigned_by_email: Option<String>,
    period_start: Option<chrono::DateTime<chrono::Utc>>,
    period_end: Option<chrono::DateTime<chrono::Utc>>,
    status_after: Option<String>,
    reason: String,
    detail: Value,
}

pub async fn admin_subscription_history(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<SubscriptionHistoryRow> = sqlx::query_as(
        "SELECT h.id,h.assigned_at,h.event_type,h.assignment_source,h.payment_confirmed,h.plan_code,h.plan_name,
                h.purchased_mailbox_count,h.invoice_number,ou.email::text AS order_user_email,
                ab.email::text AS assigned_by_email,h.period_start,h.period_end,h.status_after,h.reason,h.detail
         FROM subscription_assignment_history h
         LEFT JOIN users ou ON ou.id=h.order_user_id
         LEFT JOIN users ab ON ab.id=h.assigned_by
         WHERE h.organization_id=$1 ORDER BY h.assigned_at DESC LIMIT 500"
    ).bind(organization_id).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"history":rows.into_iter().map(|r| json!({
        "id":r.id,"assigned_at":r.assigned_at,"event_type":r.event_type,"assignment_source":r.assignment_source,
        "payment_confirmed":r.payment_confirmed,"plan_code":r.plan_code,"plan_name":r.plan_name,
        "purchased_mailbox_count":r.purchased_mailbox_count,"invoice_number":r.invoice_number,
        "order_user_email":r.order_user_email,"assigned_by_email":r.assigned_by_email,
        "period_start":r.period_start,"period_end":r.period_end,"status_after":r.status_after,"reason":r.reason,"detail":r.detail
    })).collect::<Vec<_>>() })))
}

/// Platform-admin plan/status/period/override control. Unsafe downgrades are
/// rejected before mutating the authoritative organization subscription.
pub async fn admin_update_subscription(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(organization_id): Path<Uuid>,
    Json(body): Json<SubscriptionPatchIn>,
) -> Result<Json<Value>, ApiError> {
    let status=body.status.as_deref().unwrap_or("active");
    if !matches!(status,"trial"|"active"|"past_due"|"suspended"|"cancelled") {
        return Err(ApiError::bad_request("Invalid subscription status"));
    }
    let is_system: bool = sqlx::query_scalar("SELECT is_system FROM organizations WHERE id=$1")
        .bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Business not found"))?;
    if is_system {
        return Err(ApiError::forbidden("The protected system organization subscription cannot be changed"));
    }
    let target_plan = entitlements::plan(&state,&body.plan_code).await?;
    if body.purchased_mailbox_count.is_some_and(|value| value < target_plan.mailbox_limit || value > target_plan.max_mailboxes) {
        return Err(ApiError::bad_request(format!("Purchased mailbox count must be between {} and {}", target_plan.mailbox_limit, target_plan.max_mailboxes)));
    }
    for (label,value) in [("storage pool",body.storage_pool_override_bytes),("mailbox quota",body.mailbox_quota_override_bytes)] {
        if value.is_some_and(|v| v<=0) { return Err(ApiError::bad_request(format!("{label} override must be positive"))); }
    }
    for (label,value) in [("seat",body.seat_limit_override),("mailbox",body.mailbox_limit_override),("domain",body.domain_limit_override)] {
        if value.is_some_and(|v| v<=0) { return Err(ApiError::bad_request(format!("{label} override must be positive"))); }
    }
    if body.organization_daily_send_override.is_some_and(|v| v<0) { return Err(ApiError::bad_request("Daily send override cannot be negative")); }

    let current: Option<(String,i32,String,chrono::DateTime<chrono::Utc>,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT plan_code,purchased_mailbox_count,status,current_period_start,current_period_end
         FROM organization_subscriptions WHERE organization_id=$1"
    ).bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (current_plan,current_count,current_status,current_period_start,current_period_end)=
        current.ok_or_else(|| ApiError::not_found("Business subscription not found"))?;

    let usage: (i64,i64,i64,i64,i64,i64) = sqlx::query_as(
        "SELECT
          ((SELECT count(*) FROM organization_memberships WHERE organization_id=$1 AND status IN ('active','invited')) +
           (SELECT count(*) FROM organization_invitations WHERE organization_id=$1 AND status='pending'))::bigint,
          (SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL),
          (SELECT count(*)::bigint FROM organization_domains WHERE organization_id=$1 AND status <> 'removing'),
          COALESCE((SELECT storage_bytes FROM organization_usage WHERE organization_id=$1),0)::bigint,
          COALESCE((SELECT SUM(quota_bytes) FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NOT NULL),0)::bigint,
          (SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NULL)"
    ).bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let purchased_mailboxes = body.purchased_mailbox_count.unwrap_or(current_count);
    if purchased_mailboxes < target_plan.mailbox_limit || purchased_mailboxes > target_plan.max_mailboxes {
        return Err(ApiError::bad_request(format!("Purchased mailbox count must be between {} and {}", target_plan.mailbox_limit,target_plan.max_mailboxes)));
    }
    let effective_seats = body.seat_limit_override.unwrap_or(purchased_mailboxes.max(target_plan.seats)) as i64;
    let effective_mailboxes = body.mailbox_limit_override.unwrap_or(purchased_mailboxes) as i64;
    let effective_domains = body.domain_limit_override.unwrap_or(target_plan.domain_limit) as i64;
    let scaled_storage = entitlements::scaled_storage_pool(&target_plan,purchased_mailboxes);
    let effective_storage = body.storage_pool_override_bytes.unwrap_or(scaled_storage);
    let effective_default_quota=body.mailbox_quota_override_bytes.unwrap_or(target_plan.mailbox_bytes as i64);
    let projected_allocation=usage.4.saturating_add(usage.5.saturating_mul(effective_default_quota));
    if usage.0>effective_seats || usage.1>effective_mailboxes || usage.2>effective_domains || usage.3>effective_storage {
        return Err(ApiError::conflict("The effective subscription limits cannot be reduced below the business's current usage"));
    }
    if projected_allocation>effective_storage {
        return Err(ApiError::conflict("The target storage pool is smaller than the mailbox allocations that would remain after this subscription change"));
    }

    let requested_period_end = match body.current_period_end.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(chrono::DateTime::parse_from_rfc3339(raw.trim())
            .map_err(|_| ApiError::bad_request("current_period_end must be an RFC 3339 timestamp"))?.with_timezone(&chrono::Utc)),
        _ => None,
    };
    let assignment_changed=current_plan!=body.plan_code || current_count!=purchased_mailboxes;
    let mut effective_period_end=requested_period_end.or(current_period_end);
    if assignment_changed && effective_period_end.map_or(true, |end| end<=chrono::Utc::now()) {
        effective_period_end=Some(sqlx::query_scalar::<_,chrono::DateTime<chrono::Utc>>(
            "SELECT now()+CASE WHEN $1='year' THEN interval '1 year' ELSE interval '1 month' END"
        ).bind(&target_plan.interval).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?);
    }
    if matches!(status,"active"|"trial") && effective_period_end.is_some_and(|end| end<=chrono::Utc::now()) {
        return Err(ApiError::conflict("An active subscription must have a future expiration. Extend the period before reactivating it"));
    }
    let period_changed=effective_period_end!=current_period_end;
    let status_changed=status!=current_status;
    let reason=body.reason.as_deref().map(str::trim).filter(|v|!v.is_empty()).unwrap_or_else(||{
        if assignment_changed {"Platform administrator changed plan or mailbox quantity"}
        else if period_changed {"Platform administrator changed subscription expiration"}
        else if status_changed {"Platform administrator changed subscription status"}
        else {"Platform administrator updated subscription limits"}
    });

    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    if assignment_changed {
        sqlx::query(
            "UPDATE organization_subscriptions SET plan_code=$2,status=$3,purchased_mailbox_count=$4,
              storage_pool_override_bytes=$5,mailbox_quota_override_bytes=$6,seat_limit_override=$7,mailbox_limit_override=$8,
              domain_limit_override=$9,organization_daily_send_override=$10,current_period_end=$11,
              renewal_grace_end=CASE WHEN $11::timestamptz IS NULL THEN NULL ELSE $11::timestamptz+((SELECT grace_days FROM billing_settings WHERE id=TRUE)::text||' days')::interval END,
              assignment_source='admin_manual',assigned_at=now(),assigned_by=$12,assignment_invoice_number=NULL,assignment_order_user_id=NULL,
              cancelled_at=CASE WHEN $3='cancelled' THEN now() ELSE NULL END,updated_at=now() WHERE organization_id=$1"
        ).bind(organization_id).bind(&body.plan_code).bind(status).bind(purchased_mailboxes).bind(body.storage_pool_override_bytes)
         .bind(body.mailbox_quota_override_bytes).bind(body.seat_limit_override).bind(body.mailbox_limit_override)
         .bind(body.domain_limit_override).bind(body.organization_daily_send_override).bind(effective_period_end).bind(admin.0.user_id)
         .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    } else {
        sqlx::query(
            "UPDATE organization_subscriptions SET status=$2,storage_pool_override_bytes=$3,mailbox_quota_override_bytes=$4,
              seat_limit_override=$5,mailbox_limit_override=$6,domain_limit_override=$7,organization_daily_send_override=$8,
              current_period_end=$9,renewal_grace_end=CASE WHEN $9::timestamptz IS NULL THEN NULL ELSE $9::timestamptz+((SELECT grace_days FROM billing_settings WHERE id=TRUE)::text||' days')::interval END,
              cancelled_at=CASE WHEN $2='cancelled' THEN COALESCE(cancelled_at,now()) ELSE NULL END,updated_at=now() WHERE organization_id=$1"
        ).bind(organization_id).bind(status).bind(body.storage_pool_override_bytes).bind(body.mailbox_quota_override_bytes)
         .bind(body.seat_limit_override).bind(body.mailbox_limit_override).bind(body.domain_limit_override)
         .bind(body.organization_daily_send_override).bind(effective_period_end)
         .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    }

    let event_type=if assignment_changed{"assignment"}else if period_changed{"period"}else if status_changed{"status"}else{"limits"};
    sqlx::query(
        "INSERT INTO subscription_assignment_history
          (organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,assigned_by,
           event_type,period_start,period_end,status_after,reason,detail)
         VALUES($1,$2,$3,$4,'admin_manual',FALSE,$5,$6,$7,$8,$9,$10,$11)"
    ).bind(organization_id).bind(&body.plan_code).bind(&target_plan.name).bind(purchased_mailboxes).bind(admin.0.user_id)
     .bind(event_type).bind(current_period_start).bind(effective_period_end).bind(status).bind(reason)
     .bind(sqlx::types::Json(json!({"previous_plan":current_plan,"previous_mailbox_count":current_count,"previous_status":current_status,"previous_period_end":current_period_end})))
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;

    let effective: i64=sqlx::query_scalar(
        "SELECT COALESCE(s.mailbox_quota_override_bytes,p.mailbox_bytes)::bigint FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code WHERE s.organization_id=$1"
    ).bind(organization_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE users SET plan=$2,quota_override_bytes=NULL,quota_bytes=$3,updated_at=now()
         WHERE id IN(SELECT user_id FROM organization_memberships WHERE organization_id=$1)"
    ).bind(organization_id).bind(&body.plan_code).bind(effective).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE mailboxes SET quota_bytes=$2,updated_at=now() WHERE organization_id=$1 AND deleted_at IS NULL AND quota_override_bytes IS NULL")
        .bind(organization_id).bind(effective).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO provisioning_jobs
          (user_id,organization_id,mailbox_id,operation,target_email,account_id,quota_bytes,status,max_attempts,dedupe_key)
         SELECT COALESCE(m.user_id,o.created_by),m.organization_id,m.id,'quota',m.address::text,m.provider_account_id,
                m.quota_bytes,'pending',8,'mailbox.quota:'||m.id::text
         FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
         WHERE m.organization_id=$1 AND m.deleted_at IS NULL AND COALESCE(m.user_id,o.created_by) IS NOT NULL
         ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
         DO UPDATE SET quota_bytes=EXCLUDED.quota_bytes,status='pending',attempts=0,next_attempt_at=now(),last_error='',completed_at=NULL,updated_at=now()"
    ).bind(organization_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(admin.0.user_id),"admin.billing.subscription_update",json!({
        "organization_id":organization_id,"plan":body.plan_code,"status":status,"purchased_mailbox_count":purchased_mailboxes,
        "assignment_changed":assignment_changed,"period_end":effective_period_end,"reason":reason
    })).await;
    Ok(Json(json!({"ok":true,"organization_id":organization_id,"plan_code":body.plan_code,"status":status,"current_period_end":effective_period_end})))
}
