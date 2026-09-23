use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{automation, entitlements, tenancy};
use crate::state::AppState;

const CATEGORIES: [&str; 5] = ["work", "meeting", "personal", "holiday", "reminder"];
const MAX_EVENT_PAGE: usize = 200;
const MAX_ATTENDEES: usize = 200;
const MAX_ICS_IMPORT_EVENTS: usize = 500;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInvitee {
    email: String,
    #[serde(default = "pending_status")]
    status: String,
}

fn pending_status() -> String {
    "pending".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recurrence {
    frequency: String,
    #[serde(default = "one")]
    interval: u32,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventIn {
    title: String,
    /// YYYY-MM-DD in the user's local calendar view.
    date: String,
    /// HH:mm (or HH:mm:ss). Ignored for all-day events.
    #[serde(default)]
    start: String,
    #[serde(default)]
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
    invitees: Vec<EventInvitee>,
    #[serde(default)]
    recurrence: Option<Recurrence>,
    #[serde(default)]
    timezone_offset_minutes: i32,
    /// Required for updates; omitted for creates/imports.
    #[serde(default)]
    version: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListQuery {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    category: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    cursor: Option<Uuid>,
    #[serde(default)]
    timezone_offset_minutes: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportIn {
    content: String,
    #[serde(default)]
    timezone_offset_minutes: i32,
}

fn default_limit() -> usize {
    100
}

fn default_category() -> String {
    "personal".into()
}

#[derive(Deserialize)]
pub struct VersionQuery {
    version: i64,
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
    recurrence: Option<Value>,
    timezone_offset_minutes: i32,
    external_uid: String,
    version: i64,
    updated_at: DateTime<Utc>,
}

fn row_to_json(row: &EventRow) -> Value {
    let offset = Duration::minutes(row.timezone_offset_minutes as i64);
    let local_start = row.starts_at + offset;
    let local_end = row.ends_at + offset;
    json!({
        "id": row.id,
        "seriesId": row.id,
        "seriesStartDate": local_start.format("%Y-%m-%d").to_string(),
        "occurrenceIndex": 0,
        "title": row.title,
        "date": local_start.format("%Y-%m-%d").to_string(),
        "allDay": row.all_day,
        "start": if row.all_day { String::new() } else { local_start.format("%H:%M").to_string() },
        "end": if row.all_day { String::new() } else { local_end.format("%H:%M").to_string() },
        "description": row.description,
        "location": row.location,
        "category": row.category,
        "invitees": row.attendees,
        "recurrence": row.recurrence,
        "timezoneOffsetMinutes": row.timezone_offset_minutes,
        "version": row.version,
        "updatedAt": row.updated_at,
    })
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let next = NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("valid month");
    (next - Duration::days(1)).day()
}

fn add_months(base: NaiveDate, months: u32) -> NaiveDate {
    let absolute = base.year() * 12 + base.month0() as i32 + months as i32;
    let year = absolute.div_euclid(12);
    let month = (absolute.rem_euclid(12) + 1) as u32;
    let day = base.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).expect("valid recurrence date")
}

fn recurrence_date(base: NaiveDate, recurrence: &Recurrence, index: u32) -> NaiveDate {
    let step = recurrence.interval.saturating_mul(index);
    match recurrence.frequency.as_str() {
        "daily" => base + Duration::days(step as i64),
        "weekly" => base + Duration::days(step.saturating_mul(7) as i64),
        "monthly" => add_months(base, step),
        "yearly" => add_months(base, step.saturating_mul(12)),
        _ => base,
    }
}

fn row_occurrences(
    row: &EventRow,
    range_start: Option<&DateTime<Utc>>,
    range_end: Option<&DateTime<Utc>>,
) -> Vec<Value> {
    let Some(recurrence_value) = row.recurrence.as_ref() else {
        return vec![row_to_json(row)];
    };
    // Unbounded calls are used by compatibility tests and admin diagnostics;
    // return the series head instead of materializing an arbitrary future.
    if range_start.is_none() && range_end.is_none() {
        return vec![row_to_json(row)];
    }
    let Ok(recurrence) = serde_json::from_value::<Recurrence>(recurrence_value.clone()) else {
        return vec![row_to_json(row)];
    };

    let offset = Duration::minutes(row.timezone_offset_minutes as i64);
    let base_local_start = row.starts_at + offset;
    let base_date = base_local_start.date_naive();
    let start_time = base_local_start.time();
    let duration = row.ends_at - row.starts_at;
    let until = recurrence
        .until
        .as_deref()
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
    let range_start_local_date = range_start
        .map(|value| (*value + offset).date_naive());
    let mut index = recurrence_start_index(base_date, &recurrence, range_start_local_date);
    let mut iterations = 0u32;
    let mut result = Vec::new();
    while iterations < 1000 {
        if recurrence.count.is_some_and(|count| index >= count) {
            break;
        }
        let date = recurrence_date(base_date, &recurrence, index);
        if until.as_ref().is_some_and(|last| date > *last) {
            break;
        }
        let occurrence_start = local_naive_to_utc(date, start_time, row.timezone_offset_minutes);
        let occurrence_end = occurrence_start + duration;
        if range_end
            .is_some_and(|end| occurrence_start >= *end)
        {
            break;
        }
        if range_start
            .is_some_and(|start| occurrence_end < *start)
        {
            index = index.saturating_add(1);
            iterations = iterations.saturating_add(1);
            continue;
        }

        let local_start = occurrence_start + offset;
        let local_end = occurrence_end + offset;
        let mut value = row_to_json(row);
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "id".into(),
                Value::String(if index == 0 {
                    row.id.to_string()
                } else {
                    format!("{}@{}", row.id, local_start.format("%Y-%m-%d"))
                }),
            );
            object.insert("seriesId".into(), Value::String(row.id.to_string()));
            object.insert("occurrenceIndex".into(), json!(index));
            object.insert("date".into(), Value::String(local_start.format("%Y-%m-%d").to_string()));
            object.insert(
                "start".into(),
                Value::String(if row.all_day { String::new() } else { local_start.format("%H:%M").to_string() }),
            );
            object.insert(
                "end".into(),
                Value::String(if row.all_day { String::new() } else { local_end.format("%H:%M").to_string() }),
            );
        }
        result.push(value);
        index = index.saturating_add(1);
        iterations = iterations.saturating_add(1);
    }
    result
}

fn recurrence_start_index(
    base: NaiveDate,
    recurrence: &Recurrence,
    range_start: Option<NaiveDate>,
) -> u32 {
    let Some(start) = range_start else { return 0; };
    if start <= base { return 0; }
    match recurrence.frequency.as_str() {
        "daily" => ((start - base).num_days().max(0) as u32 / recurrence.interval).saturating_sub(1),
        "weekly" => ((start - base).num_days().max(0) as u32 / recurrence.interval.saturating_mul(7)).saturating_sub(1),
        "monthly" => {
            let months = (start.year() - base.year()) * 12 + start.month0() as i32 - base.month0() as i32;
            ((months.max(0) as u32) / recurrence.interval).saturating_sub(1)
        }
        "yearly" => (((start.year() - base.year()).max(0) as u32) / recurrence.interval).saturating_sub(1),
        _ => 0,
    }
}

fn parse_day(value: &str, field: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request(format!("Invalid {field} date '{value}'")))
}

fn parse_time(value: &str, field: &str) -> Result<NaiveTime, ApiError> {
    let normalized = if value.len() == 5 {
        format!("{value}:00")
    } else {
        value.to_string()
    };
    NaiveTime::parse_from_str(&normalized, "%H:%M:%S")
        .map_err(|_| ApiError::bad_request(format!("Invalid {field} time '{value}'")))
}

fn local_naive_to_utc(day: NaiveDate, time: NaiveTime, offset_minutes: i32) -> DateTime<Utc> {
    let naive_utc = day.and_time(time) - Duration::minutes(offset_minutes as i64);
    Utc.from_utc_datetime(&naive_utc)
}

fn parse_bounds(body: &EventIn) -> Result<(DateTime<Utc>, DateTime<Utc>), ApiError> {
    validate_offset(body.timezone_offset_minutes)?;
    let day = parse_day(&body.date, "event")?;
    let (start_t, end_t) = if body.all_day {
        (
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        )
    } else {
        (parse_time(&body.start, "start")?, parse_time(&body.end, "end")?)
    };
    let starts = local_naive_to_utc(day, start_t, body.timezone_offset_minutes);
    let ends = local_naive_to_utc(day, end_t, body.timezone_offset_minutes);
    if ends <= starts {
        return Err(ApiError::bad_request("Event end must be after its start"));
    }
    Ok((starts, ends))
}

fn validate_offset(offset: i32) -> Result<(), ApiError> {
    if !(-840..=840).contains(&offset) {
        return Err(ApiError::bad_request("Timezone offset is outside the supported range"));
    }
    Ok(())
}

fn normalize_event(body: &EventIn) -> Result<(String, Vec<EventInvitee>, Option<Value>), ApiError> {
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::bad_request("Event title is required"));
    }
    if title.len() > 300 {
        return Err(ApiError::bad_request("Event title is too long"));
    }
    if body.description.len() > 20_000 {
        return Err(ApiError::bad_request("Event description is too long"));
    }
    if body.location.len() > 500 {
        return Err(ApiError::bad_request("Event location is too long"));
    }
    if !CATEGORIES.contains(&body.category.as_str()) {
        return Err(ApiError::bad_request(format!("Invalid category '{}'", body.category)));
    }
    if body.invitees.len() > MAX_ATTENDEES {
        return Err(ApiError::bad_request(format!("Events are limited to {MAX_ATTENDEES} attendees")));
    }

    let mut invitees = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for invitee in &body.invitees {
        let email = invitee.email.trim().to_lowercase();
        if !automation::valid_email(&email) {
            return Err(ApiError::bad_request(format!("Invalid attendee email '{email}'")));
        }
        let status = invitee.status.trim().to_lowercase();
        if !matches!(status.as_str(), "pending" | "accepted" | "declined") {
            return Err(ApiError::bad_request(format!("Invalid attendee status '{status}'")));
        }
        if seen.insert(email.clone()) {
            invitees.push(EventInvitee { email, status });
        }
    }

    let recurrence = match body.recurrence.as_ref() {
        Some(value) => Some(serde_json::to_value(validate_recurrence(value.clone())?)
            .map_err(|e| ApiError::internal(e.to_string()))?),
        None => None,
    };
    Ok((title, invitees, recurrence))
}

fn validate_recurrence(mut recurrence: Recurrence) -> Result<Recurrence, ApiError> {
    recurrence.frequency = recurrence.frequency.trim().to_lowercase();
    if !matches!(recurrence.frequency.as_str(), "daily" | "weekly" | "monthly" | "yearly") {
        return Err(ApiError::bad_request("Recurrence frequency must be daily, weekly, monthly, or yearly"));
    }
    if recurrence.interval == 0 || recurrence.interval > 365 {
        return Err(ApiError::bad_request("Recurrence interval is outside the supported range"));
    }
    if let Some(count) = recurrence.count {
        if count == 0 || count > 1000 {
            return Err(ApiError::bad_request("Recurrence count must be between 1 and 1000"));
        }
    }
    if recurrence.count.is_some() && recurrence.until.is_some() {
        return Err(ApiError::bad_request("Use either recurrence count or until, not both"));
    }
    if let Some(until) = recurrence.until.as_ref() {
        parse_day(until, "recurrence until")?;
    }
    Ok(recurrence)
}

async fn active_mailbox_id(state: &AppState, auth: &AuthUser) -> Result<Uuid, ApiError> {
    tenancy::active_mailbox(
        &state.db,
        auth.user_id,
        auth.organization_id_hint,
        auth.mailbox_id_hint,
    )
    .await?
    .map(|mailbox| mailbox.id)
    .ok_or_else(|| ApiError::conflict("Select a business mailbox before using the calendar"))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<EventListQuery>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let limit = params.limit.clamp(1, MAX_EVENT_PAGE);
    validate_offset(params.timezone_offset_minutes)?;
    let needle = params.q.trim().to_lowercase();
    let category = params.category.trim().to_lowercase();
    if !category.is_empty() && !CATEGORIES.contains(&category.as_str()) {
        return Err(ApiError::bad_request("Invalid calendar category filter"));
    }

    let range_start = match params.start.as_deref() {
        Some(value) => Some(local_naive_to_utc(
            parse_day(value, "range start")?,
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            params.timezone_offset_minutes,
        )),
        None => None,
    };
    let range_end = match params.end.as_deref() {
        Some(value) => Some(local_naive_to_utc(
            parse_day(value, "range end")? + Duration::days(1),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            params.timezone_offset_minutes,
        )),
        None => None,
    };

    let mut rows: Vec<EventRow> = sqlx::query_as(
        "SELECT e.id, e.title, e.starts_at, e.ends_at, e.all_day, e.description, e.location,
                e.category, e.attendees, e.recurrence, e.timezone_offset_minutes,
                e.external_uid, e.version, e.updated_at
         FROM calendar_events e
         WHERE e.mailbox_id = $1
           AND ($2::timestamptz IS NULL OR e.ends_at >= $2 OR e.recurrence IS NOT NULL)
           AND ($3::timestamptz IS NULL OR e.starts_at < $3)
           AND ($4 = '' OR lower(e.title) LIKE '%' || $4 || '%'
                        OR lower(e.description) LIKE '%' || $4 || '%'
                        OR lower(e.location) LIKE '%' || $4 || '%')
           AND ($5 = '' OR e.category = $5)
           AND ($6::uuid IS NULL OR (e.starts_at, e.id) > (
                 SELECT anchor.starts_at, anchor.id
                 FROM calendar_events anchor
                 WHERE anchor.mailbox_id = $1 AND anchor.id = $6
               ))
         ORDER BY e.starts_at, e.id
         LIMIT $7",
    )
    .bind(mailbox_id)
    .bind(range_start)
    .bind(range_end)
    .bind(&needle)
    .bind(&category)
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

    let mut events = rows
        .iter()
        .flat_map(|row| row_occurrences(row, range_start.as_ref(), range_end.as_ref()))
        .collect::<Vec<_>>();
    events.sort_by(|left, right| {
        let left_key = format!("{} {}", left.get("date").and_then(Value::as_str).unwrap_or(""), left.get("start").and_then(Value::as_str).unwrap_or(""));
        let right_key = format!("{} {}", right.get("date").and_then(Value::as_str).unwrap_or(""), right.get("start").and_then(Value::as_str).unwrap_or(""));
        left_key.cmp(&right_key)
    });

    Ok(Json(json!({
        "events": events,
        "hasMore": has_more,
        "nextCursor": next_cursor,
    })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<EventIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let (title, invitees, recurrence) = normalize_event(&body)?;
    let (starts_at, ends_at) = parse_bounds(&body)?;
    let attendees = serde_json::to_value(invitees).map_err(|e| ApiError::internal(e.to_string()))?;

    let row = sqlx::query_as::<_, EventRow>(
        "INSERT INTO calendar_events
            (user_id, mailbox_id, title, starts_at, ends_at, all_day, description, location, category,
             attendees, recurrence, timezone_offset_minutes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         RETURNING id, title, starts_at, ends_at, all_day, description, location, category,
                   attendees, recurrence, timezone_offset_minutes, external_uid, version, updated_at",
    )
    .bind(auth.user_id)
    .bind(mailbox_id)
    .bind(&title)
    .bind(starts_at)
    .bind(ends_at)
    .bind(body.all_day)
    .bind(body.description.trim())
    .bind(body.location.trim())
    .bind(&body.category)
    .bind(attendees)
    .bind(recurrence)
    .bind(body.timezone_offset_minutes)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "calendar.create", json!({ "id": row.id, "title": title })).await;
    Ok((axum::http::StatusCode::CREATED, Json(row_to_json(&row))))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<EventIn>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let version = body.version.ok_or_else(|| ApiError::bad_request("A valid event version is required"))?;
    if version < 1 {
        return Err(ApiError::bad_request("A valid event version is required"));
    }
    let (title, invitees, recurrence) = normalize_event(&body)?;
    let (starts_at, ends_at) = parse_bounds(&body)?;
    let attendees = serde_json::to_value(invitees).map_err(|e| ApiError::internal(e.to_string()))?;

    let row = sqlx::query_as::<_, EventRow>(
        "UPDATE calendar_events SET
            title = $3, starts_at = $4, ends_at = $5, all_day = $6,
            description = $7, location = $8, category = $9, attendees = $10,
            recurrence = $11, timezone_offset_minutes = $12,
            version = version + 1, updated_at = now()
         WHERE id = $1 AND mailbox_id = $2 AND version = $13
         RETURNING id, title, starts_at, ends_at, all_day, description, location, category,
                   attendees, recurrence, timezone_offset_minutes, external_uid, version, updated_at",
    )
    .bind(id)
    .bind(mailbox_id)
    .bind(&title)
    .bind(starts_at)
    .bind(ends_at)
    .bind(body.all_day)
    .bind(body.description.trim())
    .bind(body.location.trim())
    .bind(&body.category)
    .bind(attendees)
    .bind(recurrence)
    .bind(body.timezone_offset_minutes)
    .bind(version)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(row) = row {
        audit::record(&state, Some(auth.user_id), "calendar.update", json!({ "id": id, "version": row.version })).await;
        return Ok(Json(row_to_json(&row)));
    }

    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM calendar_events WHERE id = $1 AND mailbox_id = $2)")
        .bind(id)
        .bind(mailbox_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if exists {
        Err(ApiError::conflict("This event changed elsewhere. Refresh it before saving again."))
    } else {
        Err(ApiError::not_found("Event not found"))
    }
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(query): Query<VersionQuery>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    if query.version < 1 {
        return Err(ApiError::bad_request("A valid event version is required"));
    }
    let deleted = sqlx::query("DELETE FROM calendar_events WHERE id = $1 AND mailbox_id = $2 AND version = $3")
        .bind(id)
        .bind(mailbox_id)
        .bind(query.version)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .rows_affected();
    if deleted == 0 {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM calendar_events WHERE id = $1 AND mailbox_id = $2)")
            .bind(id)
            .bind(mailbox_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if exists {
            return Err(ApiError::conflict("This event changed elsewhere. Refresh it before deleting."));
        }
        return Err(ApiError::not_found("Event not found"));
    }
    audit::record(&state, Some(auth.user_id), "calendar.delete", json!({ "id": id })).await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn export_ics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    let row: Option<EventRow> = sqlx::query_as(
        "SELECT id, title, starts_at, ends_at, all_day, description, location, category,
                attendees, recurrence, timezone_offset_minutes, external_uid, version, updated_at
         FROM calendar_events WHERE id = $1 AND mailbox_id = $2",
    )
    .bind(id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(row) = row else {
        return Err(ApiError::not_found("Event not found"));
    };
    Ok(Json(json!({
        "filename": format!("{}.ics", safe_filename(&row.title)),
        "content": event_to_ics(&row),
    })))
}

pub async fn import_ics(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IcsImportIn>,
) -> Result<Json<Value>, ApiError> {
    entitlements::require_feature(&state, auth.user_id, "calendar").await?;
    let mailbox_id = active_mailbox_id(&state, &auth).await?;
    validate_offset(body.timezone_offset_minutes)?;
    if body.content.len() > 4 * 1024 * 1024 {
        return Err(ApiError::bad_request("Calendar import is too large"));
    }
    let parsed = parse_ics(&body.content, body.timezone_offset_minutes)?;
    if parsed.len() > MAX_ICS_IMPORT_EVENTS {
        return Err(ApiError::bad_request(format!("Calendar import is limited to {MAX_ICS_IMPORT_EVENTS} events")));
    }

    let mut imported = Vec::new();
    for item in parsed {
        let (title, invitees, recurrence) = normalize_event(&item.event)?;
        let (starts_at, ends_at) = parse_bounds(&item.event)?;
        let attendees = serde_json::to_value(invitees).map_err(|e| ApiError::internal(e.to_string()))?;
        let row = sqlx::query_as::<_, EventRow>(
            "INSERT INTO calendar_events
                (user_id, mailbox_id, title, starts_at, ends_at, all_day, description, location, category,
                 attendees, recurrence, timezone_offset_minutes, external_uid)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (mailbox_id, external_uid) WHERE external_uid <> '' DO UPDATE SET
                 title = EXCLUDED.title,
                 starts_at = EXCLUDED.starts_at,
                 ends_at = EXCLUDED.ends_at,
                 all_day = EXCLUDED.all_day,
                 description = EXCLUDED.description,
                 location = EXCLUDED.location,
                 attendees = EXCLUDED.attendees,
                 recurrence = EXCLUDED.recurrence,
                 timezone_offset_minutes = EXCLUDED.timezone_offset_minutes,
                 version = calendar_events.version + 1,
                 updated_at = now()
             RETURNING id, title, starts_at, ends_at, all_day, description, location, category,
                       attendees, recurrence, timezone_offset_minutes, external_uid, version, updated_at",
        )
        .bind(auth.user_id)
        .bind(mailbox_id)
        .bind(&title)
        .bind(starts_at)
        .bind(ends_at)
        .bind(item.event.all_day)
        .bind(item.event.description.trim())
        .bind(item.event.location.trim())
        .bind(&item.event.category)
        .bind(attendees)
        .bind(recurrence)
        .bind(item.event.timezone_offset_minutes)
        .bind(item.uid)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        imported.push(row_to_json(&row));
    }

    let count = imported.len();
    audit::record(&state, Some(auth.user_id), "calendar.import", json!({ "events": count })).await;
    Ok(Json(json!({ "events": imported, "count": count })))
}

fn safe_filename(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() { "event".into() } else { trimmed.to_string() }
}

fn ics_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

fn recurrence_to_rrule(value: &Value) -> Option<String> {
    let recurrence: Recurrence = serde_json::from_value(value.clone()).ok()?;
    let mut parts = vec![format!("FREQ={}", recurrence.frequency.to_uppercase())];
    if recurrence.interval > 1 {
        parts.push(format!("INTERVAL={}", recurrence.interval));
    }
    if let Some(count) = recurrence.count {
        parts.push(format!("COUNT={count}"));
    }
    if let Some(until) = recurrence.until {
        parts.push(format!("UNTIL={}", until.replace('-', "")));
    }
    Some(parts.join(";"))
}

fn event_to_ics(row: &EventRow) -> String {
    let uid = if row.external_uid.is_empty() { format!("{}@cs-mail", row.id) } else { row.external_uid.clone() };
    let offset = Duration::minutes(row.timezone_offset_minutes as i64);
    let local_start = row.starts_at + offset;
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//CS Mail//Calendar 1.0//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", Utc::now().format("%Y%m%dT%H%M%SZ")),
    ];
    if row.all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", local_start.format("%Y%m%d")));
        lines.push(format!("DTEND;VALUE=DATE:{}", (local_start + Duration::days(1)).format("%Y%m%d")));
    } else {
        lines.push(format!("DTSTART:{}", row.starts_at.format("%Y%m%dT%H%M%SZ")));
        lines.push(format!("DTEND:{}", row.ends_at.format("%Y%m%dT%H%M%SZ")));
    }
    lines.push(format!("SUMMARY:{}", ics_escape(&row.title)));
    if !row.location.is_empty() { lines.push(format!("LOCATION:{}", ics_escape(&row.location))); }
    if !row.description.is_empty() { lines.push(format!("DESCRIPTION:{}", ics_escape(&row.description))); }
    if let Some(value) = row.recurrence.as_ref().and_then(recurrence_to_rrule) { lines.push(format!("RRULE:{value}")); }
    if let Some(invitees) = row.attendees.as_array() {
        for invitee in invitees {
            if let Some(email) = invitee.get("email").and_then(Value::as_str) {
                let status = invitee.get("status").and_then(Value::as_str).unwrap_or("pending");
                let partstat = match status { "accepted" => "ACCEPTED", "declined" => "DECLINED", _ => "NEEDS-ACTION" };
                lines.push(format!("ATTENDEE;PARTSTAT={partstat}:mailto:{email}"));
            }
        }
    }
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    lines.push(String::new());
    lines.join("\r\n")
}

struct ImportedEvent {
    uid: String,
    event: EventIn,
}

fn unfold_ics(content: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in content.replace('\r', "").split('\n') {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(raw.trim_start());
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
}

fn ics_unescape(value: &str) -> String {
    value.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

fn parse_ics(content: &str, timezone_offset_minutes: i32) -> Result<Vec<ImportedEvent>, ApiError> {
    let lines = unfold_ics(content);
    let mut result = Vec::new();
    let mut block: Option<Vec<String>> = None;
    for line in lines {
        if line == "BEGIN:VEVENT" {
            block = Some(Vec::new());
            continue;
        }
        if line == "END:VEVENT" {
            if let Some(lines) = block.take() {
                result.push(parse_ics_event(&lines, timezone_offset_minutes)?);
            }
            continue;
        }
        if let Some(lines) = block.as_mut() {
            lines.push(line);
        }
    }
    Ok(result)
}

fn parse_ics_event(lines: &[String], timezone_offset_minutes: i32) -> Result<ImportedEvent, ApiError> {
    let mut uid = String::new();
    let mut title = String::new();
    let mut description = String::new();
    let mut location = String::new();
    let mut date = String::new();
    let mut start = String::new();
    let mut end = String::new();
    let mut all_day = false;
    let mut invitees = Vec::new();
    let mut recurrence = None;

    for line in lines {
        let Some((left, raw_value)) = line.split_once(':') else { continue; };
        let prop = left.split(';').next().unwrap_or("").to_uppercase();
        let value = ics_unescape(raw_value.trim());
        match prop.as_str() {
            "UID" => {
                uid = value.replace('\r', "").replace('\n', "").trim().to_string();
                if uid.len() > 255 {
                    return Err(ApiError::bad_request("Calendar UID is too long"));
                }
            }
            "SUMMARY" => title = value,
            "DESCRIPTION" => description = value,
            "LOCATION" => location = value,
            "DTSTART" => {
                let (d, t, is_all_day) = parse_ics_datetime(left, &value, timezone_offset_minutes)?;
                date = d;
                start = t;
                all_day = is_all_day;
            }
            "DTEND" => {
                let (_d, t, _is_all_day) = parse_ics_datetime(left, &value, timezone_offset_minutes)?;
                end = t;
            }
            "ATTENDEE" => {
                let email = value.trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_lowercase();
                if automation::valid_email(&email) {
                    let upper = left.to_uppercase();
                    let status = if upper.contains("PARTSTAT=ACCEPTED") { "accepted" } else if upper.contains("PARTSTAT=DECLINED") { "declined" } else { "pending" };
                    invitees.push(EventInvitee { email, status: status.into() });
                }
            }
            "RRULE" => recurrence = parse_rrule(&value)?,
            _ => {}
        }
    }
    if title.trim().is_empty() || date.is_empty() {
        return Err(ApiError::bad_request("Calendar file contains an event without a title or start date"));
    }
    if all_day {
        start.clear();
        end.clear();
    } else if end.is_empty() {
        end = start.clone();
    }
    if uid.is_empty() {
        uid = format!("import-{}-{}", date, Uuid::new_v4());
    }
    Ok(ImportedEvent {
        uid,
        event: EventIn {
            title,
            date,
            start,
            end,
            all_day,
            description,
            location,
            category: "work".into(),
            invitees,
            recurrence,
            timezone_offset_minutes,
            version: None,
        },
    })
}

fn parse_ics_datetime(left: &str, value: &str, timezone_offset_minutes: i32) -> Result<(String, String, bool), ApiError> {
    if left.to_uppercase().contains("VALUE=DATE") || !value.contains('T') {
        if value.len() < 8 { return Err(ApiError::bad_request("Invalid all-day calendar date")); }
        return Ok((format!("{}-{}-{}", &value[0..4], &value[4..6], &value[6..8]), String::new(), true));
    }
    if value.len() < 13 { return Err(ApiError::bad_request("Invalid calendar date/time")); }
    let raw = value.trim_end_matches('Z');
    let day = NaiveDate::parse_from_str(&raw[0..8], "%Y%m%d").map_err(|_| ApiError::bad_request("Invalid calendar date/time"))?;
    let time = NaiveTime::parse_from_str(&raw[9..15.min(raw.len())], if raw.len() >= 15 { "%H%M%S" } else { "%H%M" })
        .map_err(|_| ApiError::bad_request("Invalid calendar time"))?;
    let local = if value.ends_with('Z') {
        Utc.from_utc_datetime(&day.and_time(time)) + Duration::minutes(timezone_offset_minutes as i64)
    } else {
        Utc.from_utc_datetime(&day.and_time(time))
    };
    Ok((local.format("%Y-%m-%d").to_string(), local.format("%H:%M").to_string(), false))
}

fn parse_rrule(value: &str) -> Result<Option<Recurrence>, ApiError> {
    let mut frequency = None;
    let mut interval = 1u32;
    let mut count = None;
    let mut until = None;
    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else { continue; };
        match key.to_uppercase().as_str() {
            "FREQ" => frequency = Some(val.to_lowercase()),
            "INTERVAL" => interval = val.parse().map_err(|_| ApiError::bad_request("Invalid RRULE interval"))?,
            "COUNT" => count = Some(val.parse().map_err(|_| ApiError::bad_request("Invalid RRULE count"))?),
            "UNTIL" if val.len() >= 8 => until = Some(format!("{}-{}-{}", &val[0..4], &val[4..6], &val[6..8])),
            _ => {}
        }
    }
    let Some(frequency) = frequency else { return Ok(None); };
    Ok(Some(validate_recurrence(Recurrence { frequency, interval, until, count })?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrule_round_trip_basics() {
        let recurrence = parse_rrule("FREQ=WEEKLY;INTERVAL=2;COUNT=4").unwrap().unwrap();
        assert_eq!(recurrence.frequency, "weekly");
        assert_eq!(recurrence.interval, 2);
        assert_eq!(recurrence.count, Some(4));
    }

    #[test]
    fn unfolds_folded_lines() {
        let lines = unfold_ics("BEGIN:VEVENT\r\nDESCRIPTION:hello\r\n world\r\nEND:VEVENT\r\n");
        assert!(lines.iter().any(|line| line == "DESCRIPTION:helloworld"));
    }
}
