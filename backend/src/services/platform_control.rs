use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlatformControls {
    pub public_signup_enabled: bool,
    pub business_creation_enabled: bool,
    pub plan_ordering_enabled: bool,
    pub domain_onboarding_enabled: bool,
    pub mailbox_provisioning_enabled: bool,
    pub outbound_sending_enabled: bool,
    pub maintenance_message: String,
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

pub async fn load(pool: &PgPool) -> Result<PlatformControls, ApiError> {
    sqlx::query_as::<_, PlatformControls>(
        "SELECT public_signup_enabled,business_creation_enabled,plan_ordering_enabled,\
                domain_onboarding_enabled,mailbox_provisioning_enabled,outbound_sending_enabled,maintenance_message,updated_by,updated_at\
         FROM platform_controls WHERE singleton=TRUE",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

fn disabled(default_message: &str, maintenance_message: &str) -> ApiError {
    let message = if maintenance_message.trim().is_empty() { default_message } else { maintenance_message.trim() };
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "platform_paused", message)
}

pub async fn require_signup(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.public_signup_enabled { Ok(()) } else { Err(disabled("New account registration is temporarily paused", &c.maintenance_message)) }
}

pub async fn require_business_creation(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.business_creation_enabled { Ok(()) } else { Err(disabled("New business creation is temporarily paused", &c.maintenance_message)) }
}

pub async fn require_plan_ordering(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.plan_ordering_enabled { Ok(()) } else { Err(disabled("New plan orders are temporarily paused", &c.maintenance_message)) }
}

pub async fn require_domain_onboarding(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.domain_onboarding_enabled { Ok(()) } else { Err(disabled("Domain onboarding is temporarily paused", &c.maintenance_message)) }
}

pub async fn require_mailbox_provisioning(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.mailbox_provisioning_enabled { Ok(()) } else { Err(disabled("New mailbox provisioning is temporarily paused", &c.maintenance_message)) }
}

pub async fn require_outbound_sending(pool: &PgPool) -> Result<(), ApiError> {
    let c=load(pool).await?; if c.outbound_sending_enabled { Ok(()) } else { Err(disabled("Outbound sending is temporarily paused by the platform operator", &c.maintenance_message)) }
}
