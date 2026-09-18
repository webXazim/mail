//! Billing data access (WS4): admin-editable plans, out-of-band payment orders,
//! and the singleton payment-instruction settings. All SQL for the billing
//! surface lives here; handlers stay thin.

use serde::Serialize;
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::domain::quota::{Plan, PlanLimits};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct PlanRow {
    code: String,
    name: String,
    price_cents: i32,
    currency: String,
    interval: String,
    mailbox_bytes: i64,
    max_attachment_bytes: i64,
    max_recipients: i32,
    daily_send_limit: i32,
    seats: i32,
    features: Json<Vec<String>>,
    active: bool,
}

impl PlanRow {
    fn into_limits(self) -> PlanLimits {
        PlanLimits {
            code: self.code,
            name: self.name,
            price_cents: self.price_cents as i64,
            currency: self.currency,
            interval: self.interval,
            mailbox_bytes: self.mailbox_bytes.max(0) as u64,
            max_attachment_bytes: self.max_attachment_bytes.max(0) as usize,
            max_total_attachment_bytes: self.max_attachment_bytes.max(0) as usize,
            max_recipients: self.max_recipients.max(0) as usize,
            daily_send_limit: self.daily_send_limit as i64,
            seats: self.seats,
            features: self.features.0,
            active: self.active,
        }
    }
}

const PLAN_COLUMNS: &str = "code, name, price_cents, currency, interval, mailbox_bytes, \
     max_attachment_bytes, max_recipients, daily_send_limit, seats, features, active";

/// Every plan, cheapest first (admin view).
pub async fn all_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    let rows: Vec<PlanRow> = sqlx::query_as(&format!(
        "SELECT {PLAN_COLUMNS} FROM plans ORDER BY sort_order, price_cents"
    ))
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(PlanRow::into_limits).collect())
}

/// Active plans for the customer-facing pricing/upgrade view.
pub async fn active_plans(state: &AppState) -> Result<Vec<PlanLimits>, ApiError> {
    let rows: Vec<PlanRow> = sqlx::query_as(&format!(
        "SELECT {PLAN_COLUMNS} FROM plans WHERE active ORDER BY sort_order, price_cents"
    ))
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.into_iter().map(PlanRow::into_limits).collect())
}

/// Entitlements for a plan code, falling back to the built-in defaults if the
/// row is missing or the query fails — enforcement must never hard-fail open.
pub async fn for_code(state: &AppState, code: &str) -> PlanLimits {
    let row: Option<PlanRow> =
        sqlx::query_as(&format!("SELECT {PLAN_COLUMNS} FROM plans WHERE code = $1"))
            .bind(code)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    row.map(PlanRow::into_limits)
        .unwrap_or_else(|| Plan::from_db(code).limits())
}

/// Entitlements for a user's currently assigned plan.
pub async fn for_user(state: &AppState, user_id: Uuid) -> Result<PlanLimits, ApiError> {
    let code: Option<String> = sqlx::query_scalar("SELECT plan FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(for_code(state, code.as_deref().unwrap_or("solo")).await)
}

/// True when the code identifies an existing plan row.
pub async fn plan_exists(state: &AppState, code: &str) -> Result<bool, ApiError> {
    let found: Option<(String,)> = sqlx::query_as("SELECT code FROM plans WHERE code = $1")
        .bind(code)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(found.is_some())
}

/// Validated fields for creating or updating a plan.
pub struct PlanInput {
    pub code: String,
    pub name: String,
    pub price_cents: i64,
    pub currency: String,
    pub interval: String,
    pub mailbox_bytes: i64,
    pub max_attachment_bytes: i64,
    pub max_recipients: i64,
    pub daily_send_limit: i64,
    pub seats: i64,
    pub features: Vec<String>,
    pub sort_order: i64,
    pub active: bool,
}

pub async fn create_plan(state: &AppState, p: &PlanInput) -> Result<(), ApiError> {
    if plan_exists(state, &p.code).await? {
        return Err(ApiError::conflict(format!(
            "A plan with code '{}' already exists",
            p.code
        )));
    }
    sqlx::query(
        "INSERT INTO plans
            (code, name, price_cents, currency, interval, mailbox_bytes, max_attachment_bytes,
             max_recipients, daily_send_limit, seats, features, sort_order, active)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(&p.code)
    .bind(&p.name)
    .bind(p.price_cents as i32)
    .bind(&p.currency)
    .bind(&p.interval)
    .bind(p.mailbox_bytes)
    .bind(p.max_attachment_bytes)
    .bind(p.max_recipients as i32)
    .bind(p.daily_send_limit as i32)
    .bind(p.seats as i32)
    .bind(Json(&p.features))
    .bind(p.sort_order as i32)
    .bind(p.active)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

pub async fn update_plan(state: &AppState, code: &str, p: &PlanInput) -> Result<(), ApiError> {
    let affected = sqlx::query(
        "UPDATE plans SET
            name = $2, price_cents = $3, currency = $4, interval = $5,
            mailbox_bytes = $6, max_attachment_bytes = $7, max_recipients = $8,
            daily_send_limit = $9, seats = $10, features = $11, sort_order = $12,
            active = $13, updated_at = now()
         WHERE code = $1",
    )
    .bind(code)
    .bind(&p.name)
    .bind(p.price_cents as i32)
    .bind(&p.currency)
    .bind(&p.interval)
    .bind(p.mailbox_bytes)
    .bind(p.max_attachment_bytes)
    .bind(p.max_recipients as i32)
    .bind(p.daily_send_limit as i32)
    .bind(p.seats as i32)
    .bind(Json(&p.features))
    .bind(p.sort_order as i32)
    .bind(p.active)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found("Plan not found"));
    }
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

#[derive(Serialize)]
pub struct BillingSettings {
    pub bank_details: String,
    pub paypal_email: String,
    pub instructions: String,
}

pub async fn settings(state: &AppState) -> Result<BillingSettings, ApiError> {
    let row: (String, String, String) = sqlx::query_as(
        "SELECT bank_details, paypal_email, instructions FROM billing_settings WHERE id = TRUE",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(BillingSettings {
        bank_details: row.0,
        paypal_email: row.1,
        instructions: row.2,
    })
}

pub async fn update_settings(
    state: &AppState,
    bank_details: &str,
    paypal_email: &str,
    instructions: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE billing_settings
         SET bank_details = $1, paypal_email = $2, instructions = $3, updated_at = now()
         WHERE id = TRUE",
    )
    .bind(bank_details)
    .bind(paypal_email)
    .bind(instructions)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// An order/invoice row plus the customer it belongs to (admin view).
#[derive(Serialize, sqlx::FromRow)]
pub struct OrderView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub plan_code: String,
    pub plan_name: String,
    pub amount_cents: i64,
    pub currency: String,
    pub interval: String,
    pub seats: i32,
    pub status: String,
    pub payment_method: String,
    pub payment_reference: String,
    pub customer_note: String,
    pub admin_note: String,
    pub invoice_number: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub paid_at: Option<chrono::DateTime<chrono::Utc>>,
}

const ORDER_COLUMNS: &str = "o.id, o.user_id, u.email, u.display_name, o.plan_code, o.plan_name, \
     o.amount_cents::int8 AS amount_cents, o.currency, o.interval, o.seats, o.status, \
     o.payment_method, o.payment_reference, o.customer_note, o.admin_note, o.invoice_number, \
     o.created_at, o.submitted_at, o.paid_at";

pub async fn orders_for_user(state: &AppState, user_id: Uuid) -> Result<Vec<OrderView>, ApiError> {
    let rows: Vec<OrderView> = sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS}
         FROM orders o JOIN users u ON u.id = o.user_id
         WHERE o.user_id = $1 ORDER BY o.created_at DESC"
    ))
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows)
}

pub async fn all_orders(
    state: &AppState,
    status: Option<&str>,
) -> Result<Vec<OrderView>, ApiError> {
    let rows: Vec<OrderView> = sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS}
         FROM orders o JOIN users u ON u.id = o.user_id
         WHERE ($1::text IS NULL OR o.status = $1)
         ORDER BY o.created_at DESC LIMIT 500"
    ))
    .bind(status)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows)
}

pub async fn order_by_id(state: &AppState, id: Uuid) -> Result<OrderView, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS}
         FROM orders o JOIN users u ON u.id = o.user_id WHERE o.id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Order not found"))
}

/// Create a pending order snapshotting the plan's current name and price.
pub async fn create_order(
    state: &AppState,
    user_id: Uuid,
    plan_code: &str,
    payment_method: &str,
    customer_note: &str,
) -> Result<OrderView, ApiError> {
    let plan = for_code(state, plan_code).await;
    if !plan_exists(state, plan_code).await? || !plan.active {
        return Err(ApiError::bad_request("That plan is not available"));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders
            (user_id, plan_code, plan_name, amount_cents, currency, interval, seats,
             payment_method, customer_note)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         RETURNING id",
    )
    .bind(user_id)
    .bind(&plan.code)
    .bind(&plan.name)
    .bind(plan.price_cents as i32)
    .bind(&plan.currency)
    .bind(&plan.interval)
    .bind(plan.seats)
    .bind(payment_method)
    .bind(customer_note)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    order_by_id(state, id).await
}

/// Customer marks a pending order as paid, supplying the reference they used.
pub async fn submit_order(
    state: &AppState,
    user_id: Uuid,
    id: Uuid,
    payment_method: &str,
    payment_reference: &str,
) -> Result<OrderView, ApiError> {
    let affected = sqlx::query(
        "UPDATE orders SET status = 'submitted', payment_method = $3,
                payment_reference = $4, submitted_at = now(), updated_at = now()
         WHERE id = $1 AND user_id = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(user_id)
    .bind(payment_method)
    .bind(payment_reference)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::conflict(
            "Only a pending order can be marked as paid",
        ));
    }
    order_by_id(state, id).await
}

pub async fn cancel_order(state: &AppState, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let affected = sqlx::query(
        "UPDATE orders SET status = 'cancelled', updated_at = now()
         WHERE id = $1 AND user_id = $2 AND status IN ('pending', 'submitted')",
    )
    .bind(id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found(
            "No cancellable order with that id was found",
        ));
    }
    Ok(())
}

/// Admin approves an order: assigns an invoice number, marks it paid, and
/// activates the plan on the customer's account.
pub async fn approve_order(
    state: &AppState,
    admin_id: Uuid,
    id: Uuid,
    admin_note: &str,
) -> Result<OrderView, ApiError> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("SELECT user_id, plan_code, status FROM orders WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let (user_id, plan_code, status) = row.ok_or_else(|| ApiError::not_found("Order not found"))?;
    if !matches!(status.as_str(), "pending" | "submitted") {
        return Err(ApiError::conflict("This order has already been reviewed"));
    }

    let plan = for_code(state, &plan_code).await;

    let invoice_number: String = sqlx::query_scalar(
        "SELECT 'INV-' || to_char(now(), 'YYYY') || '-' ||
                lpad(nextval('invoice_number_seq')::text, 6, '0')",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE orders SET status = 'paid', paid_at = now(), reviewed_at = now(),
                reviewed_by = $2, invoice_number = $3, admin_note = $4,
                activated_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(admin_id)
    .bind(&invoice_number)
    .bind(admin_note)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query("UPDATE users SET plan = $2, quota_bytes = $3, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(&plan.code)
        .bind(plan.mailbox_bytes as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Mirror the new quota into Stalwart (best effort: the DB is the source of
    // truth and the next admin edit retries).
    let account: Option<String> =
        sqlx::query_scalar("SELECT mail_account_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if let Some(account) = account.filter(|a| !a.is_empty()) {
        if let Err(e) =
            crate::services::imap::set_account_quota(&state.mail, &account, plan.mailbox_bytes)
                .await
        {
            tracing::warn!(%account, "quota sync after approval failed: {e}");
        }
    }

    order_by_id(state, id).await
}

pub async fn reject_order(
    state: &AppState,
    admin_id: Uuid,
    id: Uuid,
    admin_note: &str,
) -> Result<OrderView, ApiError> {
    let affected = sqlx::query(
        "UPDATE orders SET status = 'rejected', reviewed_at = now(), reviewed_by = $2,
                admin_note = $3, updated_at = now()
         WHERE id = $1 AND status IN ('pending', 'submitted')",
    )
    .bind(id)
    .bind(admin_id)
    .bind(admin_note)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found(
            "No reviewable order with that id was found",
        ));
    }
    order_by_id(state, id).await
}

/// Convenience for tests/handlers that want the raw settings JSON.
pub fn settings_json(s: &BillingSettings) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}
