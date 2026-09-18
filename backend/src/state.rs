use std::sync::Arc;

use axum::extract::FromRef;
use sqlx::PgPool;

use crate::metrics::Metrics;
use crate::middleware::rate_limit::RateLimiter;
use crate::services::provisioning::MailBridge;
use crate::services::smtp::SmtpConfig;
use crate::ws::EventHub;

/// Application state shared via axum State extracts.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub jwt_refresh_ttl_secs: u64,
    pub cors_origins: Vec<String>,
    pub hub: EventHub,
    pub public_origin: String,
    pub require_verification: bool,
    pub return_token_links: bool,
    pub cookie_secure: bool,
    pub mail: MailBridge,
    pub smtp: SmtpConfig,
    pub rate: RateLimiter,
    pub metrics: Arc<Metrics>,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(s: &AppState) -> Self {
        s.db.clone()
    }
}
