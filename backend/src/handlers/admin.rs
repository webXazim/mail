use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::domain;
use crate::domain::quota;
use crate::error::ApiError;
use crate::handlers::auth::valid_email;
use crate::middleware::auth::AdminUser;
use crate::services::billing;
use crate::services::imap;
use crate::state::AppState;

const DEFAULT_AUDIT_LIMIT: i64 = 50;
const MAX_AUDIT_LIMIT: i64 = 500;
const AUDIT_EXPORT_CAP: i64 = 10_000;

/// A joined audit row: id, timestamp, actor email, action, JSON detail.
type AuditRow = (
    Uuid,
    chrono::DateTime<chrono::Utc>,
    Option<String>,
    String,
    Value,
);

/// Gated overview that proves the server-side role gate.
pub async fn overview(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let (users, admins): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(*) FILTER (WHERE role = 'admin') FROM users")
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    let (audit_events,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "admin": { "id": admin.0.user_id, "email": admin.0.email },
        "users": users,
        "admins": admins,
        "audit_events": audit_events,
    })))
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    display_name: String,
    role: String,
    plan: String,
    quota_bytes: i64,
    mail_account_id: Option<String>,
    onboarded: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// `GET /api/admin/users` — every business user plus live disk usage pulled
/// from Stalwart in one batched management call.
pub async fn users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<UserRow> = sqlx::query_as(
        "SELECT id, email, display_name, role, plan, quota_bytes,
                mail_account_id, onboarded, created_at
         FROM users
         ORDER BY created_at",
    )
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
    let quotas = imap::account_quotas(&state.mail, &accounts)
        .await
        .unwrap_or_default();

    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            let live = r
                .mail_account_id
                .as_deref()
                .and_then(|a| quotas.get(a))
                .copied();
            let used = live.map_or(0, |(u, _)| u);
            let quota_bytes = live
                .map(|(_, t)| t as i64)
                .filter(|t| *t > 0)
                .unwrap_or(r.quota_bytes);
            json!({
                "id": r.id,
                "email": r.email,
                "display_name": r.display_name,
                "role": r.role,
                "plan": r.plan,
                "status": "active",
                "onboarded": r.onboarded,
                "mail_account_id": r.mail_account_id,
                "quota_bytes": quota_bytes,
                "storage_used_bytes": used,
                "storage_pct": (quota::used_ratio(used, quota_bytes.max(0) as u64) * 100.0).round() as i64,
                "created_at": r.created_at,
            })
        })
        .collect();

    Ok(Json(json!({ "users": list })))
}

#[derive(Deserialize)]
pub struct UserPatch {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    quota_bytes: Option<i64>,
    #[serde(default)]
    password: Option<String>,
}

/// `PATCH /api/admin/users/:id` — role, plan, quota and display name. Quota is
/// mirrored to Stalwart so the change actually constrains the mailbox, not just
/// the row.
pub async fn update_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UserPatch>,
) -> Result<Json<Value>, ApiError> {
    let current: Option<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT role, plan, email, quota_bytes, mail_account_id FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (role, _plan, email, quota_bytes, mail_account_id) =
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
            return Err(ApiError::bad_request(
                "Role must be member, admin or billing",
            ));
        }
        if next_role != role {
            if id == admin.0.user_id {
                return Err(ApiError::bad_request("You cannot change your own role"));
            }
            if role == "admin" {
                let (admins,): (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                        .fetch_one(&state.db)
                        .await
                        .map_err(|e| ApiError::internal(e.to_string()))?;
                if admins <= 1 {
                    return Err(ApiError::bad_request("Cannot remove the last admin"));
                }
            }
            sqlx::query("UPDATE users SET role = $1, updated_at = now() WHERE id = $2")
                .bind(next_role)
                .bind(id)
                .execute(&state.db)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            audit::record(
                &state,
                Some(admin.0.user_id),
                "admin.user.role",
                json!({ "user_id": id, "from": role, "to": next_role }),
            )
            .await;
        }
    }

    // A plan change without an explicit quota adopts that plan's mailbox size.
    // Plans are admin-editable rows (WS4); only existing codes are assignable.
    let mut next_quota = body.quota_bytes.unwrap_or(quota_bytes);
    if let Some(code) = body.plan.as_deref() {
        if !billing::plan_exists(&state, code).await? {
            return Err(ApiError::bad_request(format!("Unknown plan '{code}'")));
        }
        let limits = billing::for_code(&state, code).await;
        sqlx::query("UPDATE users SET plan = $1, updated_at = now() WHERE id = $2")
            .bind(code)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if body.quota_bytes.is_none() {
            next_quota = limits.mailbox_bytes as i64;
        }
    }

    if next_quota != quota_bytes {
        if next_quota < 1024 * 1024 {
            return Err(ApiError::bad_request("Quota must be at least 1 MiB"));
        }
        sqlx::query("UPDATE users SET quota_bytes = $1, updated_at = now() WHERE id = $2")
            .bind(next_quota)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

        if let Some(account) = mail_account_id.filter(|a| !a.is_empty()) {
            if let Err(e) = imap::set_account_quota(&state.mail, &account, next_quota as u64).await
            {
                tracing::warn!(%account, "quota mirror to Stalwart failed: {e}");
            }
        }
        audit::record(
            &state,
            Some(admin.0.user_id),
            "admin.user.quota",
            json!({ "user_id": id, "from": quota_bytes, "to": next_quota }),
        )
        .await;
    }

    if let Some(password) = body.password.as_deref() {
        domain::password::validate_password(password, &email)?;
        let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).expect("base64 salt");
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .to_string();
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(hash)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        audit::record(
            &state,
            Some(admin.0.user_id),
            "admin.user.password_reset",
            json!({ "user_id": id }),
        )
        .await;
    }

    users(State(state), admin).await.map(|Json(v)| {
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
    domain::password::validate_password(&body.password, &email)?;

    let role = body.role.unwrap_or_else(|| "member".into());
    if !matches!(role.as_str(), "member" | "admin" | "billing") {
        return Err(ApiError::bad_request(
            "Role must be member, admin or billing",
        ));
    }

    let plan = body.plan.unwrap_or_else(|| "solo".into());
    if !billing::plan_exists(&state, &plan).await? {
        return Err(ApiError::bad_request(format!("Unknown plan '{plan}'")));
    }
    let limits = billing::for_code(&state, &plan).await;
    let quota_bytes = body
        .quota_bytes
        .unwrap_or(limits.mailbox_bytes as i64)
        .max(1024 * 1024);

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

    let user: (Uuid, String) = sqlx::query_as(
        "INSERT INTO users (email, display_name, password_hash, role, plan, quota_bytes, email_verified_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         RETURNING id, email",
    )
    .bind(&email)
    .bind(display_name)
    .bind(hash)
    .bind(&role)
    .bind(&plan)
    .bind(quota_bytes)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match state.mail.ensure_mailbox(&user.1, &body.password).await {
        Ok(Some(account_id)) => {
            let _ = sqlx::query("UPDATE users SET mail_account_id = $1 WHERE id = $2")
                .bind(&account_id)
                .bind(user.0)
                .execute(&state.db)
                .await;
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(email = %user.1, "mailbox provisioning failed: {e}"),
    }

    audit::record(
        &state,
        Some(admin.0.user_id),
        "admin.user.created",
        json!({ "user_id": user.0, "email": user.1, "role": role, "plan": plan }),
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "id": user.0,
        "email": user.1,
        "role": role,
        "plan": plan,
        "quota_bytes": quota_bytes,
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
        sqlx::query_as("SELECT role, mail_account_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let (role, account_id) = row.ok_or_else(|| ApiError::not_found("User not found"))?;

    if role == "admin" {
        let (admins,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if admins <= 1 {
            return Err(ApiError::bad_request("Cannot delete the last admin"));
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
        "SELECT a.id, a.at, u.email, a.action, a.detail
             FROM audit_log a
             LEFT JOIN users u ON u.id = a.actor_id
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
        .map(|(id, at, actor, action, detail)| {
            json!({
                "id": id,
                "time": at,
                "actor": actor.unwrap_or_else(|| "system".into()),
                "action": action,
                "detail": detail,
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

    let mut csv = String::from("id,time,actor,action,detail\r\n");
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
        csv.push_str("\r\n");
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"harbor-audit.csv\"",
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

/// `POST /api/admin/suppressions/dsn` — ingest a bounce report. Only hard
/// failures (5.x) are recorded by `ingest_dsn`.
pub async fn ingest_dsn(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<DsnIn>,
) -> Result<Json<Value>, ApiError> {
    let count = crate::services::suppression::ingest_dsn(&state, &body.body).await?;
    if count > 0 {
        audit::record(
            &state,
            Some(admin.0.user_id),
            "admin.suppress.dsn",
            json!({ "suppressed": count }),
        )
        .await;
    }
    Ok(Json(json!({ "suppressed": count })))
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
