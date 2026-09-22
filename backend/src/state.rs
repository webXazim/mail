use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::FromRef;
use sqlx::PgPool;

use crate::metrics::Metrics;
use crate::middleware::rate_limit::RateLimiter;
use crate::services::provisioning::ProvisioningService;
use crate::services::stalwart::StalwartService;
use crate::ws::EventHub;

/// Application state shared via axum State extracts.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub jwt_refresh_ttl_secs: u64,
    pub delivery_event_secret: Option<String>,
    pub cors_origins: Vec<String>,
    pub hub: EventHub,
    pub realtime_instance_id: uuid::Uuid,
    pub realtime_poll_secs: u64,
    pub realtime_lease_secs: u64,
    pub realtime_batch_size: i64,
    pub realtime_event_retention_secs: u64,
    pub public_origin: String,
    pub require_verification: bool,
    pub return_token_links: bool,
    pub cookie_secure: bool,
    pub stalwart: StalwartService,
    pub provisioning: ProvisioningService,
    pub two_factor_key: String,
    pub mail_client_host: String,
    pub mail_client_imap_port: u16,
    pub mail_client_smtp_port: u16,
    pub mail_client_max_app_passwords: i64,
    pub mail_import_max_bytes: u64,
    pub mail_import_message_max_bytes: u64,
    pub mail_import_poll_secs: u64,
    pub mail_import_lease_secs: u64,
    pub schedule_poll_secs: u64,
    pub schedule_lease_secs: u64,
    pub schedule_retry_base_secs: u64,
    pub schedule_max_attempts: i32,
    pub schedule_batch_size: i64,
    pub attachment_store_dir: PathBuf,
    pub attachment_staging_quota_bytes: u64,
    pub attachment_upload_ttl_secs: u64,
    pub attachment_draft_ttl_secs: u64,
    pub attachment_consumed_grace_secs: u64,
    pub attachment_cleanup_secs: u64,
    pub billing_instant_activation: bool,
    pub rate: RateLimiter,
    pub metrics: Arc<Metrics>,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(s: &AppState) -> Self {
        s.db.clone()
    }
}
