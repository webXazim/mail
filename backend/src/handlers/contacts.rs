use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{automation, entitlements, tenancy};
use crate::state::AppState;

const MAX_CONTACT_PAGE: usize = 100;
const MAX_CONTACT_IMPORT: usize = 2_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactIn {
    name: String,
    email: String,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    phone: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactPatch {
    version: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    phone: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactListQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    cursor: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactImportIn {
    content: String,
    #[serde(default)]
    replace_existing: bool,
}

fn default_limit() -> usize {
    50
}


#[derive(Deserialize)]
pub struct VersionQuery {
    version: i64,
}

#[derive(sqlx::FromRow)]
struct ContactRow {
    id: Uuid,
    name: String,
    email: String,
    company: String,
    phone: String,
    version: i64,
    updated_at: DateTime<Utc>,
}

fn row_to_json(row: &ContactRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "email": row.email,
        "company": row.company,
        "phone": row.phone,
        "version": row.version,
        "updatedAt": row.updated_at,
    })
}

fn normalize_contact(
    name: &str,
    email: &str,
    company: Option<&str>,
    phone: Option<&str>,
) -> Result<(String, String, String, String), ApiError> {
    let email = email.trim().to_lowercase();
    if !automation::valid_email(&email) || email.len() > 320 {
        return Err(ApiError::bad_request("Enter a valid contact email address"));
    }
    let mut name = name.trim().to_string();
    if name.is_empty() {
        name = email.split('@').next().unwrap_or("Contact").to_string();
    }
    if name.len() > 200 {
        return Err(ApiError::bad_request("Contact name is too long"));
    }
    let company = company.unwrap_or("").trim().to_string();
    if company.len() > 200 {
        return Err(ApiError::bad_request("Company name is too long"));
    }
    let phone = phone.unwrap_or("").trim().to_string();
    if phone.len() > 80 {
        return Err(ApiError::bad_request("Phone number is too long"));
    }
    Ok((name, email, company, phone))
}

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(&state.db, auth.user_id, auth.organization_id_hint, auth.mailbox_id_hint)
        .await?
        .map(|mailbox| mailbox.id)
        .ok_or_else(|| ApiError::conflict("Select a business mailbox before using contacts"))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ContactListQuery>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let limit = params.limit.clamp(1, MAX_CONTACT_PAGE);
    let needle = params.q.trim().to_lowercase();

    let mut rows: Vec<ContactRow> = sqlx::query_as(
        "SELECT c.id, c.name, c.email, c.company, c.phone, c.version, c.updated_at
         FROM contacts c
         WHERE c.mailbox_id = $1
           AND ($2 = '' OR lower(c.name) LIKE '%' || $2 || '%'
                         OR lower(c.email) LIKE '%' || $2 || '%'
                         OR lower(c.company) LIKE '%' || $2 || '%'
                         OR lower(c.phone) LIKE '%' || $2 || '%')
           AND ($3::uuid IS NULL OR (lower(c.name), c.id) > (
                 SELECT lower(anchor.name), anchor.id
                 FROM contacts anchor
                 WHERE anchor.mailbox_id = $1 AND anchor.id = $3
               ))
         ORDER BY lower(c.name), c.id
         LIMIT $4",
    )
    .bind(mailbox_id)
    .bind(&needle)
    .bind(params.cursor)
    .bind((limit + 1) as i64)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let next_cursor = if has_more { rows.last().map(|row| row.id) } else { None };

    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM contacts c
         WHERE c.mailbox_id = $1
           AND ($2 = '' OR lower(c.name) LIKE '%' || $2 || '%'
                         OR lower(c.email) LIKE '%' || $2 || '%'
                         OR lower(c.company) LIKE '%' || $2 || '%'
                         OR lower(c.phone) LIKE '%' || $2 || '%')",
    )
    .bind(mailbox_id)
    .bind(&needle)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "contacts": rows.iter().map(row_to_json).collect::<Vec<_>>(),
        "total": total,
        "hasMore": has_more,
        "nextCursor": next_cursor,
    })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContactIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let (name, email, company, phone) = normalize_contact(
        &body.name,
        &body.email,
        body.company.as_deref(),
        body.phone.as_deref(),
    )?;

    let row = sqlx::query_as::<_, ContactRow>(
        "INSERT INTO contacts (user_id, mailbox_id, email, name, company, phone)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, name, email, company, phone, version, updated_at",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(&email)
    .bind(&name)
    .bind(&company)
    .bind(&phone)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                return ApiError::conflict(format!("Contact {email} already exists"));
            }
        }
        ApiError::internal(e.to_string())
    })?;

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.create",
        json!({ "id": row.id, "email": &email }),
    )
    .await;
    queue_contacts_refresh(&state, auth.user_id, mailbox_id, "created").await;

    Ok((axum::http::StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ContactPatch>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    if body.version < 1 {
        return Err(ApiError::bad_request("A valid contact version is required"));
    }

    let current: Option<ContactRow> = sqlx::query_as(
        "SELECT id, name, email, company, phone, version, updated_at
         FROM contacts WHERE id = $1 AND mailbox_id = $2",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(current) = current else {
        return Err(ApiError::not_found("Contact not found"));
    };
    if current.version != body.version {
        return Err(ApiError::conflict(
            "This contact changed elsewhere. Refresh it before saving again.",
        ));
    }

    let (name, email, company, phone) = normalize_contact(
        body.name.as_deref().unwrap_or(&current.name),
        body.email.as_deref().unwrap_or(&current.email),
        body.company.as_deref().or(Some(current.company.as_str())),
        body.phone.as_deref().or(Some(current.phone.as_str())),
    )?;

    let row = sqlx::query_as::<_, ContactRow>(
        "UPDATE contacts SET
            name = $3, email = $4, company = $5, phone = $6,
            version = version + 1, updated_at = now()
         WHERE id = $1 AND mailbox_id = $2 AND version = $7
         RETURNING id, name, email, company, phone, version, updated_at",
    )
    .bind(id)
    .bind(mailbox_id)
    .bind(&name)
    .bind(&email)
    .bind(&company)
    .bind(&phone)
    .bind(body.version)
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

    let Some(row) = row else {
        return Err(ApiError::conflict(
            "This contact changed elsewhere. Refresh it before saving again.",
        ));
    };

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.update",
        json!({ "id": id, "version": row.version }),
    )
    .await;
    queue_contacts_refresh(&state, auth.user_id, mailbox_id, "updated").await;
    Ok(Json(row_to_json(&row)))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(query): Query<VersionQuery>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    if query.version < 1 {
        return Err(ApiError::bad_request("A valid contact version is required"));
    }
    let deleted = sqlx::query("DELETE FROM contacts WHERE id = $1 AND mailbox_id = $2 AND version = $3")
        .bind(id)
        .bind(mailbox_id)
        .bind(query.version)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();

    if deleted == 0 {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM contacts WHERE id = $1 AND mailbox_id = $2)")
            .bind(id)
            .bind(mailbox_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if exists {
            return Err(ApiError::conflict("This contact changed elsewhere. Refresh it before deleting."));
        }
        return Err(ApiError::not_found("Contact not found"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.delete",
        json!({ "id": id }),
    )
    .await;
    queue_contacts_refresh(&state, auth.user_id, mailbox_id, "deleted").await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn export_csv(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let rows: Vec<ContactRow> = sqlx::query_as(
        "SELECT id, name, email, company, phone, version, updated_at
         FROM contacts WHERE mailbox_id = $1 ORDER BY lower(name), id",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut csv = String::from("name,email,company,phone\r\n");
    for row in rows {
        csv.push_str(&format!(
            "{},{},{},{}\r\n",
            csv_cell(&row.name),
            csv_cell(&row.email),
            csv_cell(&row.company),
            csv_cell(&row.phone),
        ));
    }
    Ok(Json(json!({
        "filename": "cs-mail-contacts.csv",
        "content": csv,
    })))
}

pub async fn import_csv(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContactImportIn>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "contacts").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    if body.content.len() > 2 * 1024 * 1024 {
        return Err(ApiError::bad_request("Contact import is too large"));
    }
    let parsed = parse_csv(&body.content)?;
    if parsed.len() > MAX_CONTACT_IMPORT {
        return Err(ApiError::bad_request(format!(
            "Contact import is limited to {MAX_CONTACT_IMPORT} rows"
        )));
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut imported = 0usize;
    let mut updated = 0usize;

    for item in parsed {
        let (name, email, company, phone) = normalize_contact(
            &item.name,
            &item.email,
            item.company.as_deref(),
            item.phone.as_deref(),
        )?;
        let affected = if body.replace_existing {
            sqlx::query(
                "INSERT INTO contacts (user_id, mailbox_id, email, name, company, phone)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (mailbox_id, lower(email)) DO UPDATE SET
                   name = EXCLUDED.name,
                   company = EXCLUDED.company,
                   phone = EXCLUDED.phone,
                   version = contacts.version + 1,
                   updated_at = now()",
            )
            .bind(auth.user_id)
            .bind(mailbox_id)
            .bind(&email)
            .bind(&name)
            .bind(&company)
            .bind(&phone)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .rows_affected()
        } else {
            sqlx::query(
                "INSERT INTO contacts (user_id, mailbox_id, email, name, company, phone)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (mailbox_id, lower(email)) DO NOTHING",
            )
            .bind(auth.user_id)
            .bind(mailbox_id)
            .bind(&email)
            .bind(&name)
            .bind(&company)
            .bind(&phone)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .rows_affected()
        };
        if affected > 0 {
            imported += 1;
            if body.replace_existing {
                updated += 1;
            }
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "contact.import",
        json!({ "rows": imported, "replaceExisting": body.replace_existing }),
    )
    .await;
    queue_contacts_refresh(&state, auth.user_id, mailbox_id, "imported").await;

    Ok(Json(json!({
        "ok": true,
        "processed": imported,
        "updatedOrInserted": updated,
    })))
}

async fn queue_contacts_refresh(state: &AppState, user_id: Uuid, mailbox_id: Uuid, action: &str) {
    if let Err(error) = automation::contacts_changed(state, user_id, mailbox_id).await {
        tracing::warn!(user_id=%user_id, mailbox_id=%mailbox_id, %error, %action, "contact change persisted; vacation automation refresh could not be queued");
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains(',') || value.contains('\"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn parse_csv(content: &str) -> Result<Vec<ContactIn>, ApiError> {
    let rows = csv_rows(content)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let headers = rows[0]
        .iter()
        .map(|item| item.trim().trim_start_matches('\u{feff}').to_lowercase())
        .collect::<Vec<_>>();
    let index = |name: &str| headers.iter().position(|item| item == name);
    let name_i = index("name");
    let email_i = index("email").ok_or_else(|| ApiError::bad_request("CSV must include an email column"))?;
    let company_i = index("company");
    let phone_i = index("phone");

    let mut result = Vec::new();
    for (line, row) in rows.into_iter().skip(1).enumerate() {
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let email = row.get(email_i).cloned().unwrap_or_default();
        let name = name_i.and_then(|i| row.get(i).cloned()).unwrap_or_default();
        if email.trim().is_empty() {
            return Err(ApiError::bad_request(format!(
                "CSV row {} is missing an email address",
                line + 2
            )));
        }
        result.push(ContactIn {
            name,
            email,
            company: company_i.and_then(|i| row.get(i).cloned()),
            phone: phone_i.and_then(|i| row.get(i).cloned()),
        });
    }
    Ok(result)
}

fn csv_rows(content: &str) -> Result<Vec<Vec<String>>, ApiError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut chars = content.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                row.push(std::mem::take(&mut cell));
            }
            '\n' if !quoted => {
                if cell.ends_with('\r') {
                    cell.pop();
                }
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(ch),
        }
    }
    if quoted {
        return Err(ApiError::bad_request("CSV contains an unterminated quoted value"));
    }
    if !cell.is_empty() || !row.is_empty() {
        if cell.ends_with('\r') {
            cell.pop();
        }
        row.push(cell);
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parser_handles_quotes_and_commas() {
        let rows = csv_rows("name,email,company,phone\r\n\"A, B\",a@example.com,\"Acme \"\"EU\"\"\",123\r\n").unwrap();
        assert_eq!(rows[1][0], "A, B");
        assert_eq!(rows[1][2], "Acme \"EU\"");
    }
}
