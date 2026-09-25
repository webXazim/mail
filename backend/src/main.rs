use std::sync::Arc;
use std::time::Duration;

use cs_mail_api::config::Config;
use cs_mail_api::metrics::Metrics;
use cs_mail_api::middleware::rate_limit::RateLimiter;
use cs_mail_api::router::build_router;
use cs_mail_api::services::provisioning::{self, ProvisioningService};
use cs_mail_api::services::stalwart::{StalwartConfig, StalwartService};
use cs_mail_api::services::mailer::MailerClient;
use cs_mail_api::state::AppState;
use cs_mail_api::ws::{spawn_realtime, EventHub};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cs_mail_api::install_tls_crypto_provider();

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("cs_mail_api=info,tower_http=info,sqlx=warn"));

    let config = Config::from_env()?;

    // WS6.2: JSON logs are one object per line, ready for Loki/Promtail.
    if config.log_format.eq_ignore_ascii_case("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    let stored_provider_namespace: String = sqlx::query_scalar(
        "SELECT provider_namespace FROM platform_runtime_identity WHERE singleton=TRUE",
    )
    .fetch_one(&pool)
    .await?;
    if stored_provider_namespace != config.provider_namespace {
        anyhow::bail!(
            "CS_MAIL_PROVIDER_NAMESPACE={} does not match persisted provider namespace {}; refusing to start",
            config.provider_namespace, stored_provider_namespace
        );
    }

    tokio::fs::create_dir_all(&config.attachment_store_dir).await?;

    let state = AppState {
        db: pool.clone(),
        environment: config.environment.clone(),
        release_sha256: config.release_sha256.clone(),
        jwt_secret: config.jwt_secret.clone(),
        jwt_access_ttl_secs: config.jwt_access_ttl_secs,
        jwt_refresh_ttl_secs: config.jwt_refresh_ttl_secs,
        delivery_event_secret: config.delivery_event_secret.clone(),
        cors_origins: config.cors_origins.clone(),
        hub: EventHub::new(),
        realtime_instance_id: uuid::Uuid::new_v4(),
        realtime_poll_secs: config.realtime_poll_secs,
        realtime_lease_secs: config.realtime_lease_secs,
        realtime_batch_size: config.realtime_batch_size,
        realtime_event_retention_secs: config.realtime_event_retention_secs,
        public_origin: config.public_origin.clone(),
        require_verification: config.require_verification,
        return_token_links: config.return_token_links,
        cookie_secure: config.cookie_secure,
        stalwart: StalwartService::new(StalwartConfig {
            admin_url: config.mail_admin_url.clone(),
            admin_username: config.mail_admin_username.clone(),
            admin_secret: config.mail_admin_secret.clone(),
            admin_bearer_token: config.mail_admin_token.clone(),
            mail_jmap_username: config.mail_jmap_username.clone(),
            mail_jmap_secret: config.mail_jmap_secret.clone(),
            default_domain: config.mail_default_domain.clone(),
            ownership_namespace: config.provider_namespace.clone(),
            request_timeout: Duration::from_secs(config.mail_request_timeout_secs),
            read_retries: config.mail_read_retries,
            retry_base_delay: Duration::from_millis(config.mail_retry_base_ms),
            smtp: config.smtp.clone(),
        })?,
        system_mailer: MailerClient::from_env()?,
        provisioning: ProvisioningService::new(
            config.provisioning_key,
            Duration::from_secs(config.provisioning_poll_secs),
            Duration::from_secs(config.provisioning_lease_secs),
            Duration::from_secs(config.provisioning_retry_base_secs),
            Duration::from_secs(config.provisioning_reconcile_secs),
            config.provisioning_max_attempts,
            config.provisioning_batch_size,
        )
        .map_err(anyhow::Error::msg)?,
        two_factor_key: config.two_factor_key.clone(),
        mail_client_host: config.mail_client_host.clone(),
        mail_client_imap_port: config.mail_client_imap_port,
        mail_client_smtp_port: config.mail_client_smtp_port,
        mail_client_max_app_passwords: config.mail_client_max_app_passwords.max(1),
        mail_import_max_bytes: config.mail_import_max_bytes,
        mail_import_message_max_bytes: config.mail_import_message_max_bytes,
        mail_import_poll_secs: config.mail_import_poll_secs,
        mail_import_lease_secs: config.mail_import_lease_secs,
        schedule_poll_secs: config.schedule_poll_secs,
        schedule_lease_secs: config.schedule_lease_secs,
        schedule_retry_base_secs: config.schedule_retry_base_secs,
        schedule_max_attempts: config.schedule_max_attempts,
        schedule_batch_size: config.schedule_batch_size,
        attachment_store_dir: config.attachment_store_dir.clone(),
        attachment_staging_quota_bytes: config.attachment_staging_quota_bytes,
        attachment_upload_ttl_secs: config.attachment_upload_ttl_secs,
        attachment_draft_ttl_secs: config.attachment_draft_ttl_secs,
        attachment_consumed_grace_secs: config.attachment_consumed_grace_secs,
        attachment_cleanup_secs: config.attachment_cleanup_secs,
        billing_instant_activation: config.billing_instant_activation,
        rate: RateLimiter::new(pool.clone(), &config.jwt_secret, config.trusted_proxy_ips.clone()),
        metrics: Arc::new(Metrics::new()),
    };

    cs_mail_api::handlers::attachments::migrate_legacy_inline_attachments(&state).await;
    cs_mail_api::handlers::attachments::spawn_cleanup_worker(state.clone());
    provisioning::spawn_worker(state.clone());
    spawn_realtime(state.clone());
    spawn_maintenance(state.clone());
    cs_mail_api::handlers::schedule::spawn_worker(state.clone());
    cs_mail_api::handlers::send::spawn_reconciliation_worker(state.clone());
    cs_mail_api::services::automation::spawn_worker(state.clone());
    cs_mail_api::services::addressing::spawn_worker(state.clone());
    cs_mail_api::services::domain_onboarding::spawn_worker(state.clone());
    cs_mail_api::services::business_addressing::spawn_worker(state.clone());
    cs_mail_api::handlers::mail_clients::spawn_import_worker(state.clone());
    cs_mail_api::services::billing::spawn_email_worker(state.clone());
    cs_mail_api::services::billing::spawn_lifecycle_worker(state.clone());

    let app = build_router(state);

    tracing::info!("listening on {}", config.listen_addr);
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

/// Periodic housekeeping: drop expired refresh sessions and single-use email
/// tokens so neither table grows unbounded.
fn spawn_maintenance(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            tick.tick().await;
            if let Err(e) = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
                .execute(&state.db)
                .await
            {
                tracing::warn!("session clean-up failed: {e}");
            }
            // A used refresh token cannot be replayed after its original JWT
            // expiry. Keep a small safety margin beyond the configured refresh
            // lifetime while bounding history for long-lived rotating sessions.
            let history_ttl_secs = state.jwt_refresh_ttl_secs.saturating_add(24 * 3600) as i64;
            if let Err(e) = sqlx::query(
                "DELETE FROM session_refresh_history
                 WHERE used_at <= now() - ($1 * interval '1 second')",
            )
            .bind(history_ttl_secs)
            .execute(&state.db)
            .await
            {
                tracing::warn!("refresh history clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query("DELETE FROM email_tokens WHERE expires_at <= now()")
                .execute(&state.db)
                .await
            {
                tracing::warn!("email token clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "DELETE FROM request_rate_limits WHERE updated_at < now() - interval '48 hours'",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("rate-limit bucket clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "DELETE FROM two_factor_challenges WHERE expires_at <= now() OR consumed_at < now() - interval '1 day'",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("two-factor challenge clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "UPDATE users SET totp_pending_ciphertext = NULL, totp_pending_expires_at = NULL
                 WHERE totp_pending_expires_at IS NOT NULL AND totp_pending_expires_at <= now()",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("two-factor pending setup clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "DELETE FROM mail_send_requests
                 WHERE (status IN ('sent','failed') AND updated_at < now() - interval '90 days')
                    OR (status = 'prepared' AND attempt_count = 0 AND updated_at < now() - interval '7 days')",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("send request ledger clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "DELETE FROM scheduled_sends
                 WHERE status IN ('sent','cancelled')
                   AND completed_at < now() - interval '90 days'",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("scheduled-send history clean-up failed: {e}");
            }
            if let Err(e) = sqlx::query(
                "UPDATE sender_identities SET verification_token_hash='',verification_attempts=0,updated_at=now()
                 WHERE status='pending' AND verification_expires_at IS NOT NULL
                   AND verification_expires_at<=now() AND verification_token_hash<>''",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("sender-identity verification clean-up failed: {e}");
            }
        }
    });
}

/// Blocks until SIGINT (Ctrl+C) or SIGTERM, then lets axum drain in-flight
/// requests instead of tearing the process down immediately.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received, draining in-flight requests");
}
