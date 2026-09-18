use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ContactIn {
    name: String,
    email: String,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    phone: Option<String>,
}

#[derive(Deserialize)]
pub struct ContactPatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    phone: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ContactRow {
    id: Uuid,
    name: String,
    email: String,
    company: String,
    phone: String,
}

fn row_to_json(row: &ContactRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "email": row.email,
        "company": row.company,
        "phone": row.phone,
    })
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let rows: Vec<ContactRow> = sqlx::query_as(
        "SELECT id, name, email, company, phone FROM contacts WHERE user_id = $1 ORDER BY name",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "contacts": rows.iter().map(row_to_json).collect::<Vec<_>>()
    })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContactIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let name = body.name.trim();
    let email = body.email.trim().to_lowercase();
    if name.is_empty() || email.is_empty() || !email.contains('@') {
        return Err(ApiError::bad_request(
            "Contact name and a valid email are required",
        ));
    }

    let dup: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM contacts WHERE user_id = $1 AND email = $2")
            .bind(auth.user_id)
            .bind(&email)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    if dup.is_some() {
        return Err(ApiError::conflict(format!(
            "Contact {email} already exists"
        )));
    }

    let row = sqlx::query_as::<_, ContactRow>(
        "INSERT INTO contacts (user_id, email, name, company, phone) VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, email, company, phone",
    )
    .bind(auth.user_id)
    .bind(&email)
    .bind(name)
    .bind(body.company.as_deref().unwrap_or(""))
    .bind(body.phone.as_deref().unwrap_or(""))
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.create",
        json!({ "email": &email }),
    )
    .await;

    Ok((axum::http::StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ContactPatch>,
) -> Result<Json<Value>, ApiError> {
    let email = body.email.as_ref().map(|e| e.trim().to_lowercase());

    let row = sqlx::query_as::<_, ContactRow>(
        "UPDATE contacts SET
            name    = COALESCE($2, name),
            email   = COALESCE($3, email),
            company = COALESCE($4, company),
            phone   = COALESCE($5, phone),
            updated_at = now()
         WHERE id = $1 AND user_id = $6
         RETURNING id, name, email, company, phone",
    )
    .bind(id)
    .bind(body.name.as_deref())
    .bind(email.as_deref())
    .bind(body.company.as_deref())
    .bind(body.phone.as_deref())
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                return ApiError::conflict("A contact with this email already exists");
            }
        }
        ApiError::internal(e.to_string())
    })?;

    match row {
        Some(row) => {
            audit::record(
                &state,
                Some(auth.user_id),
                "contact.update",
                json!({ "id": id }),
            )
            .await;
            Ok(Json(row_to_json(&row)))
        }
        None => Err(ApiError::not_found("Contact not found")),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let deleted = sqlx::query("DELETE FROM contacts WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();

    if deleted == 0 {
        return Err(ApiError::not_found("Contact not found"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.delete",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}
