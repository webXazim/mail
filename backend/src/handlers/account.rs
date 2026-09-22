//! Account erasure (WS5.4). Two entry points — self-service (password
//! re-confirmation) and admin — share one `erase` routine. Application data is
//! deleted in the same transaction that durably queues Stalwart mailbox
//! destruction. The queue row survives via `ON DELETE SET NULL`, so a provider
//! outage or process restart cannot leave an untracked remote mailbox.

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

/// Drop application data immediately and durably queue remote mailbox erasure.
pub async fn erase(
    state: &AppState,
    user_id: Uuid,
    account_id: Option<String>,
) -> Result<(), ApiError> {
    // A platform login is no longer implicitly a mailbox. Refuse to erase the
    // only active owner of any business, otherwise that tenant would become
    // orphaned and impossible to administer.
    let orphaned_business: Option<String> = sqlx::query_scalar(
        "SELECT o.name
         FROM organization_memberships mine
         JOIN organizations o ON o.id = mine.organization_id
         WHERE mine.user_id = $1 AND mine.status = 'active' AND mine.role = 'owner'
           AND NOT EXISTS (
             SELECT 1 FROM organization_memberships other
             WHERE other.organization_id = mine.organization_id
               AND other.user_id <> mine.user_id
               AND other.status = 'active'
               AND other.role = 'owner'
           )
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(name) = orphaned_business {
        return Err(ApiError::conflict(format!(
            "Transfer ownership or close {name} before deleting this account"
        )));
    }

    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !exists {
        return Err(ApiError::not_found("Account not found"));
    }

    // Only the protected legacy/system organization retains the old behavior
    // where deleting the login also destroys its provider mailbox. Customer
    // organizations own mailboxes independently from login identities; later
    // mailbox lifecycle APIs handle reassignment/deletion explicitly.
    let legacy_mailbox: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.address::text, m.provider_account_id
         FROM users u
         JOIN mailboxes m ON m.id = u.primary_mailbox_id AND m.deleted_at IS NULL
         JOIN organizations o ON o.id = m.organization_id AND o.is_system = TRUE
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some((mailbox_address, mailbox_account)) = legacy_mailbox {
        let account = account_id
            .filter(|value| !value.is_empty())
            .or_else(|| mailbox_account.filter(|value| !value.is_empty()));
        state
            .provisioning
            .enqueue_delete_tx(&mut tx, user_id, &mailbox_address, account.as_deref())
            .await?;
        sqlx::query(
            "UPDATE mailboxes SET status='deleting', sync_status='pending', updated_at=now()
             WHERE user_id=$1 AND is_primary_for_user=TRUE AND deleted_at IS NULL",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

#[derive(Deserialize)]
pub struct DeleteIn {
    password: String,
}

/// `POST /api/account/delete` — irreversible self-service erasure. Requires the
/// current password so a stolen access token alone cannot wipe the account.
pub async fn delete_self(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DeleteIn>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT password_hash, mail_account_id FROM users WHERE id = $1")
            .bind(auth.user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    let (password_hash, account_id) =
        row.ok_or_else(|| ApiError::not_found("Account not found"))?;

    let parsed =
        PasswordHash::new(&password_hash).map_err(|e| ApiError::internal(e.to_string()))?;
    if Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(ApiError::unauthorized("Password is incorrect"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "account.erase",
        json!({ "self": true }),
    )
    .await;

    erase(&state, auth.user_id, account_id).await?;
    Ok(Json(json!({ "ok": true })))
}
