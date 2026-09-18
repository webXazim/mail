use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const CATEGORIES: [&str; 5] = ["work", "meeting", "personal", "holiday", "reminder"];

#[derive(Deserialize)]
pub struct EventIn {
    title: String,
    /// YYYY-MM-DD
    date: String,
    /// HH:mm (or HH:mm:ss)
    start: String,
    end: String,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: String,
    #[serde(default = "default_category")]
    category: String,
    #[serde(default)]
    invitees: Vec<Value>,
}

fn default_category() -> String {
    "personal".into()
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    title: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    all_day: bool,
    description: String,
    location: String,
    category: String,
    attendees: Value,
}

fn parse_bounds(
    date: &str,
    start: &str,
    end: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>), ApiError> {
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
        ApiError::bad_request(format!("Invalid date '{date}', expected YYYY-MM-DD"))
    })?;
    let start_t = NaiveTime::parse_from_str(
        &(start.to_owned() + if start.len() == 5 { ":00" } else { "" }),
        "%H:%M:%S",
    )
    .map_err(|_| ApiError::bad_request(format!("Invalid start time '{start}'")))?;
    let end_t = NaiveTime::parse_from_str(
        &(end.to_owned() + if end.len() == 5 { ":00" } else { "" }),
        "%H:%M:%S",
    )
    .map_err(|_| ApiError::bad_request(format!("Invalid end time '{end}'")))?;

    let starts = Utc
        .from_local_datetime(&day.and_time(start_t))
        .earliest()
        .unwrap_or_else(|| Utc.from_utc_datetime(&day.and_time(start_t)));
    let ends = Utc
        .from_local_datetime(&day.and_time(end_t))
        .earliest()
        .unwrap_or_else(|| Utc.from_utc_datetime(&day.and_time(end_t)));

    if ends <= starts {
        return Err(ApiError::bad_request("Event end must be after its start"));
    }
    Ok((starts, ends))
}

fn row_to_json(row: &EventRow) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "date": row.starts_at.format("%Y-%m-%d").to_string(),
        "allDay": row.all_day,
        "start": row.starts_at.format("%H:%M").to_string(),
        "end": row.ends_at.format("%H:%M").to_string(),
        "description": row.description,
        "location": row.location,
        "category": row.category,
        "invitees": row.attendees,
    })
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, ApiError> {
    let rows: Vec<EventRow> = sqlx::query_as(
        "SELECT id, title, starts_at, ends_at, all_day, description, location, category, attendees
         FROM calendar_events WHERE user_id = $1 ORDER BY starts_at",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(
        json!({ "events": rows.iter().map(row_to_json).collect::<Vec<_>>() }),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<EventIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("Event title is required"));
    }
    if !CATEGORIES.contains(&body.category.as_str()) {
        return Err(ApiError::bad_request(format!(
            "Invalid category '{}'",
            body.category
        )));
    }
    let (starts_at, ends_at) = parse_bounds(&body.date, &body.start, &body.end)?;

    let row = sqlx::query_as::<_, EventRow>(
        "INSERT INTO calendar_events
            (user_id, title, starts_at, ends_at, all_day, description, location, category, attendees)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id, title, starts_at, ends_at, all_day, description, location, category, attendees",
    )
    .bind(auth.user_id)
    .bind(title)
    .bind(starts_at)
    .bind(ends_at)
    .bind(body.all_day)
    .bind(body.description.trim())
    .bind(body.location.trim())
    .bind(&body.category)
    .bind(serde_json::Value::Array(body.invitees))
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "calendar.create",
        json!({ "title": title }),
    )
    .await;
    Ok((axum::http::StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<EventIn>,
) -> Result<Json<Value>, ApiError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("Event title is required"));
    }
    if !CATEGORIES.contains(&body.category.as_str()) {
        return Err(ApiError::bad_request(format!(
            "Invalid category '{}'",
            body.category
        )));
    }
    let (starts_at, ends_at) = parse_bounds(&body.date, &body.start, &body.end)?;

    let row = sqlx::query_as::<_, EventRow>(
        "UPDATE calendar_events SET
            title = $2, starts_at = $3, ends_at = $4, all_day = $5,
            description = $6, location = $7, category = $8, attendees = $9,
            updated_at = now()
         WHERE id = $1 AND user_id = $10
         RETURNING id, title, starts_at, ends_at, all_day, description, location, category, attendees",
    )
    .bind(id)
    .bind(title)
    .bind(starts_at)
    .bind(ends_at)
    .bind(body.all_day)
    .bind(body.description.trim())
    .bind(body.location.trim())
    .bind(&body.category)
    .bind(serde_json::Value::Array(body.invitees))
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match row {
        Some(row) => {
            audit::record(
                &state,
                Some(auth.user_id),
                "calendar.update",
                json!({ "id": id }),
            )
            .await;
            Ok(Json(row_to_json(&row)))
        }
        None => Err(ApiError::not_found("Event not found")),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let deleted = sqlx::query("DELETE FROM calendar_events WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();

    if deleted == 0 {
        return Err(ApiError::not_found("Event not found"));
    }

    audit::record(
        &state,
        Some(auth.user_id),
        "calendar.delete",
        json!({ "id": id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}
