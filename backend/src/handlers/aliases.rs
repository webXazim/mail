use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AdminUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AliasIn {
    domain: String,
    source: String,
    #[serde(rename = "forwardTo")]
    forward_to: String,
}

#[derive(sqlx::FromRow)]
struct AliasRow {
    id: Uuid,
    domain: String,
    source: String,
    #[sqlx(rename = "forward_email")]
    forward_email: Option<String>,
    dest_external: String,
}

fn row_to_json(row: &AliasRow) -> Value {
    let address = format!("{}@{}", row.source, row.domain);
    let forward = row
        .forward_email
        .clone()
        .unwrap_or_else(|| row.dest_external.clone());
    json!({
        "id": row.id,
        "address": address,
        "domain": row.domain,
        "source": row.source,
        "forwardTo": forward,
    })
}

/// All aliases with their resolved forward target, ordered by address. Shared
/// by the user-facing list and the admin surface (WS3.6).
pub async fn all(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<AliasRow> = sqlx::query_as(
        "SELECT a.id, a.domain, a.source,
                u.email AS forward_email, a.dest_external
         FROM aliases a
         LEFT JOIN users u ON u.id = a.dest_user
         ORDER BY a.domain, a.source",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.iter().map(row_to_json).collect())
}

pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "aliases": all(&state).await? })))
}

pub async fn create(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<AliasIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let domain = body.domain.trim().to_lowercase();
    let source = body.source.trim().to_lowercase();
    let forward = body.forward_to.trim().to_lowercase();

    if domain.is_empty() || source.is_empty() || forward.is_empty() {
        return Err(ApiError::bad_request(
            "domain, source and forwardTo are required",
        ));
    }
    if !source
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(
            "source may only contain letters, digits, '.', '-' or '_'",
        ));
    }

    let dest_user: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&forward)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if dest_user.is_none()
        && (!forward.contains('@') || forward.split('@').nth(1).unwrap_or("").is_empty())
    {
        return Err(ApiError::bad_request(
            "forwardTo must be a known mailbox email or a valid external address",
        ));
    }

    let row = sqlx::query_as::<_, AliasRow>(
        "INSERT INTO aliases (domain, source, dest_user, dest_external)
         VALUES ($1, $2, $3, $4)
         RETURNING id, domain, source,
                   (SELECT u.email FROM users u WHERE u.id = aliases.dest_user) AS forward_email,
                   dest_external",
    )
    .bind(&domain)
    .bind(&source)
    .bind(dest_user.map(|(id,)| id))
    .bind(if dest_user.is_some() { "" } else { &forward })
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                return ApiError::conflict(format!("{source}@{domain} already exists"));
            }
        }
        ApiError::internal(e.to_string())
    })?;

    audit::record(
        &state,
        Some(admin.0.user_id),
        "alias.create",
        json!({ "address": format!("{source}@{domain}") }),
    )
    .await;

    Ok((axum::http::StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn delete(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let deleted = sqlx::query("DELETE FROM aliases WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();

    if deleted == 0 {
        return Err(ApiError::not_found("Alias not found"));
    }

    audit::record(
        &state,
        Some(admin.0.user_id),
        "alias.delete",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}
