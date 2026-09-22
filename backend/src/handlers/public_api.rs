use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AdminUser;
use crate::services::billing;
use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IncidentRow {
    id: Uuid,
    title: String,
    status: String,
    impact: String,
    message: String,
    started_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

pub async fn plans(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows = billing::active_plans(&state).await?;
    let plans: Vec<Value> = rows
        .into_iter()
        .map(|p| json!({
            "code": p.code,
            "name": p.name,
            "price_cents": p.price_cents,
            "extra_mailbox_price_cents": p.extra_mailbox_price_cents,
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
        }))
        .collect();
    Ok(Json(json!({"plans": plans})))
}

pub async fn status(State(state): State<AppState>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let mail_enabled = state.stalwart.enabled();
    let mail_ok = if mail_enabled {
        matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), state.stalwart.healthcheck()).await,
            Ok(Ok(_))
        )
    } else {
        true
    };
    let incidents: Vec<IncidentRow> = if db_ok {
        sqlx::query_as(
            "SELECT id, title, status, impact, message, started_at, resolved_at, updated_at
             FROM service_incidents
             WHERE started_at > now() - interval '90 days'
             ORDER BY started_at DESC LIMIT 50",
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let active = incidents.iter().filter(|i| i.status != "resolved").count();
    let overall = if !db_ok || !mail_ok { "major_outage" } else if active > 0 { "degraded" } else { "operational" };
    // Status pages must remain readable during an incident. The body carries
    // component health; returning 200 lets clients render the outage instead
    // of replacing it with a generic transport error.
    Ok((StatusCode::OK, Json(json!({
        "product": "CS Mail",
        "status": overall,
        "updated_at": Utc::now(),
        "components": [
            {"key":"mail_delivery","name":"Email delivery","status": if mail_ok {"operational"} else {"outage"}},
            {"key":"api","name":"API & sync","status": if db_ok {"operational"} else {"outage"}},
            {"key":"contacts_calendar","name":"Contacts & calendar","status": if db_ok {"operational"} else {"outage"}},
            {"key":"account","name":"Sign-in & identity","status": if db_ok {"operational"} else {"outage"}}
        ],
        "active_incidents": active,
        "incidents": incidents
    }))))
}

#[derive(Debug, Deserialize)]
pub struct IncidentIn {
    title: String,
    status: String,
    #[serde(default = "default_impact")]
    impact: String,
    #[serde(default)]
    message: String,
}
fn default_impact() -> String { "minor".into() }

fn validate_incident(body: &IncidentIn) -> Result<(String,String,String,String), ApiError> {
    let title: String = body.title.trim().chars().take(200).collect();
    let status = body.status.trim().to_ascii_lowercase();
    let impact = body.impact.trim().to_ascii_lowercase();
    let message: String = body.message.trim().chars().take(5000).collect();
    if title.is_empty() { return Err(ApiError::bad_request("Incident title is required")); }
    if !matches!(status.as_str(), "investigating" | "identified" | "monitoring" | "resolved") {
        return Err(ApiError::bad_request("Invalid incident status"));
    }
    if !matches!(impact.as_str(), "minor" | "major" | "critical") {
        return Err(ApiError::bad_request("Invalid incident impact"));
    }
    Ok((title,status,impact,message))
}

pub async fn admin_incidents(State(state): State<AppState>, _admin: AdminUser) -> Result<Json<Value>, ApiError> {
    let rows: Vec<IncidentRow> = sqlx::query_as(
        "SELECT id, title, status, impact, message, started_at, resolved_at, updated_at
         FROM service_incidents ORDER BY started_at DESC LIMIT 200"
    ).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"incidents":rows})))
}

pub async fn admin_create_incident(
    State(state): State<AppState>, admin: AdminUser, Json(body): Json<IncidentIn>
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (title,status,impact,message)=validate_incident(&body)?;
    let row: IncidentRow=sqlx::query_as(
        "INSERT INTO service_incidents(title,status,impact,message,created_by,resolved_at)
         VALUES ($1,$2,$3,$4,$5,CASE WHEN $2='resolved' THEN now() ELSE NULL END)
         RETURNING id,title,status,impact,message,started_at,resolved_at,updated_at"
    ).bind(&title).bind(&status).bind(&impact).bind(&message).bind(admin.0.user_id)
     .fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(admin.0.user_id),"admin.status.incident_create",json!({"id":row.id,"status":row.status})).await;
    Ok((StatusCode::CREATED,Json(json!({"incident":row}))))
}

pub async fn admin_update_incident(
    State(state): State<AppState>, admin: AdminUser, Path(id): Path<Uuid>, Json(body): Json<IncidentIn>
) -> Result<Json<Value>, ApiError> {
    let (title,status,impact,message)=validate_incident(&body)?;
    let row: IncidentRow=sqlx::query_as(
        "UPDATE service_incidents SET title=$2,status=$3,impact=$4,message=$5,updated_at=now(),
          resolved_at=CASE WHEN $3='resolved' THEN COALESCE(resolved_at,now()) ELSE NULL END
         WHERE id=$1
         RETURNING id,title,status,impact,message,started_at,resolved_at,updated_at"
    ).bind(id).bind(&title).bind(&status).bind(&impact).bind(&message)
     .fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?
     .ok_or_else(||ApiError::not_found("Incident not found"))?;
    audit::record(&state,Some(admin.0.user_id),"admin.status.incident_update",json!({"id":id,"status":row.status})).await;
    Ok(Json(json!({"incident":row})))
}
