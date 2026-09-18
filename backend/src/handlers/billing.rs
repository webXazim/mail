//! WS4 manual-payment billing. Customers order a plan and pay out-of-band;
//! admins review orders on this same surface and approve/reject them, which
//! activates the plan. No external payment processor is in the loop.

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
use crate::services::billing;
use crate::state::AppState;

const PAYMENT_METHODS: [&str; 4] = ["bank", "paypal", "card", "other"];

fn plan_json(p: &PlanLimits) -> Value {
    json!({
        "code": p.code,
        "name": p.name,
        "price_cents": p.price_cents,
        "price": p.price_display(),
        "currency": p.currency,
        "interval": p.interval,
        "mailbox_bytes": p.mailbox_bytes,
        "max_attachment_bytes": p.max_attachment_bytes,
        "max_recipients": p.max_recipients,
        "daily_send_limit": p.daily_send_limit,
        "seats": p.seats,
        "features": p.features,
        "active": p.active,
    })
}

/// `GET /api/billing` — everything the billing page needs in one round trip.
pub async fn summary(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT plan, quota_bytes, mail_account_id FROM users WHERE id = $1")
            .bind(auth.user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let (_plan_code, quota_bytes, _mail_account_id) =
        row.ok_or_else(|| ApiError::not_found("User not found"))?;

    let current = billing::for_user(&state, auth.user_id).await?;
    let settings = billing::settings(&state).await?;
    let orders = billing::orders_for_user(&state, auth.user_id).await?;

    Ok(Json(json!({
        "current_plan": plan_json(&current),
        "quota_bytes": quota_bytes,
        "settings": settings,
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
    let order =
        billing::create_order(&state, auth.user_id, &body.plan_code, &method, &note).await?;

    audit::record(
        &state,
        Some(auth.user_id),
        "billing.order_create",
        json!({ "order_id": order.id, "plan": order.plan_code }),
    )
    .await;

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
    let paid: Vec<Value> = orders
        .iter()
        .filter(|o| o.status == "paid")
        .map(|o| serde_json::to_value(o).unwrap_or(Value::Null))
        .collect();
    Ok(Json(json!({ "invoices": paid })))
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
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_interval")]
    interval: String,
    mailbox_bytes: i64,
    max_attachment_bytes: i64,
    max_recipients: i64,
    #[serde(default)]
    daily_send_limit: i64,
    #[serde(default = "one")]
    seats: i64,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    sort_order: i64,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_currency() -> String {
    "USD".into()
}
fn default_interval() -> String {
    "month".into()
}
fn one() -> i64 {
    1
}
fn default_true() -> bool {
    true
}

fn validate_plan_input(body: &PlanIn) -> Result<billing::PlanInput, ApiError> {
    if body.code.trim().is_empty() || body.name.trim().is_empty() {
        return Err(ApiError::bad_request("Plan code and name are required"));
    }
    if !body.interval.is_empty() && !matches!(body.interval.as_str(), "month" | "year") {
        return Err(ApiError::bad_request("Interval must be 'month' or 'year'"));
    }
    if body.price_cents < 0
        || body.mailbox_bytes <= 0
        || body.max_attachment_bytes <= 0
        || body.max_recipients <= 0
        || body.daily_send_limit < 0
        || body.seats <= 0
    {
        return Err(ApiError::bad_request(
            "All numeric limits must be positive (daily limit may be zero)",
        ));
    }
    Ok(billing::PlanInput {
        code: body.code.trim().to_lowercase().replace(' ', "-"),
        name: body.name.trim().to_string(),
        price_cents: body.price_cents,
        currency: body.currency.trim().to_uppercase(),
        interval: body.interval.clone(),
        mailbox_bytes: body.mailbox_bytes,
        max_attachment_bytes: body.max_attachment_bytes,
        max_recipients: body.max_recipients,
        daily_send_limit: body.daily_send_limit,
        seats: body.seats,
        features: body.features.clone(),
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
    let plan = billing::for_code(&state, &input.code).await;
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
    let plan = billing::for_code(&state, &code).await;
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
        json!({ "order_id": id, "invoice": order.invoice_number }),
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
    #[serde(default)]
    bank_details: String,
    #[serde(default)]
    paypal_email: String,
    #[serde(default)]
    instructions: String,
}

/// `PUT /api/admin/billing-settings` — update bank/PayPal details.
pub async fn admin_update_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<SettingsIn>,
) -> Result<Json<billing::BillingSettings>, ApiError> {
    billing::update_settings(
        &state,
        body.bank_details.trim(),
        body.paypal_email.trim(),
        body.instructions.trim(),
    )
    .await?;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.billing.settings_update",
        json!({}),
    )
    .await;
    Ok(Json(billing::settings(&state).await?))
}
