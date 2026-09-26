use std::sync::Arc;
use std::time::Duration;

use cs_mail_api::config::Config;
use cs_mail_api::metrics::Metrics;
use cs_mail_api::middleware::rate_limit::RateLimiter;
use cs_mail_api::router::build_router;
use cs_mail_api::services::provisioning::{self, ProvisioningService};
use cs_mail_api::services::stalwart::{StalwartConfig, StalwartService};
use cs_mail_api::services::mailer::MailerClient;
use cs_mail_api::services::object_storage::{ObjectStore, R2Config};
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

    if config.environment == "production" && config.billing_instant_activation {
        tracing::warn!(
            "CS_MAIL_BILLING_INSTANT_ACTIVATION is enabled for production acceptance testing; public launch certification remains blocked until it is disabled"
        );
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

    let object_store = ObjectStore::new(
        config.attachment_store_dir.clone(),
        &config.object_storage_backend,
        (config.object_storage_backend == "r2").then(|| R2Config {
            account_id: config.r2_account_id.clone(), bucket: config.r2_bucket.clone(),
            access_key_id: config.r2_access_key_id.clone(), secret_access_key: config.r2_secret_access_key.clone(),
            endpoint: config.r2_endpoint.clone(), region: config.r2_region.clone(), prefix: config.r2_prefix.clone(),
            request_timeout_secs: config.r2_request_timeout_secs,
            max_concurrent_transfers: config.r2_max_concurrent_transfers,
        }),
    ).map_err(anyhow::Error::msg)?;
    object_store.healthcheck().await.map_err(anyhow::Error::msg)?;
    tracing::info!(backend = object_store.active_backend(), "attachment object storage ready");

    let state = AppState {
        db: pool.clone(),
        db_capacity_bytes: config.db_capacity_bytes,
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
        object_store,
        attachment_staging_quota_bytes: config.attachment_staging_quota_bytes,
        attachment_upload_ttl_secs: config.attachment_upload_ttl_secs,
        attachment_draft_ttl_secs: config.attachment_draft_ttl_secs,
        attachment_consumed_grace_secs: config.attachment_consumed_grace_secs,
        attachment_cleanup_secs: config.attachment_cleanup_secs,
        billing_instant_activation: config.billing_instant_activation,
        rate: RateLimiter::new(pool.clone(), &config.jwt_secret, config.trusted_proxy_ips.clone()),
        metrics: Arc::new(Metrics::new()),
    };

    if state.billing_instant_activation {
        let repaired = cs_mail_api::services::billing::reconcile_test_instant_orders(&state).await?;
        if repaired > 0 {
            tracing::warn!(repaired, "reactivated legacy open billing invoice(s) for acceptance testing");
        }
    }

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

            // Operational rate-limit rows are useful for short-term diagnostics,
            // not permanent history. Bound them so a busy SaaS cannot grow one
            // database row per mailbox/domain/hour forever.
            for (label, query) in [
                ("legacy daily send counters", "DELETE FROM send_counters WHERE day < current_date - 90"),
                ("domain daily send counters", "DELETE FROM send_domain_counters WHERE day < current_date - 90"),
                ("mailbox daily send counters", "DELETE FROM mailbox_send_counters WHERE day < current_date - 90"),
                ("organization daily send counters", "DELETE FROM organization_send_counters WHERE day < current_date - 90"),
                ("mailbox hourly send counters", "DELETE FROM mailbox_send_hourly_counters WHERE hour_start < now() - interval '30 days'"),
                ("organization hourly send counters", "DELETE FROM organization_send_hourly_counters WHERE hour_start < now() - interval '30 days'"),
                ("domain hourly send counters", "DELETE FROM domain_send_hourly_counters WHERE hour_start < now() - interval '30 days'"),
            ] {
                if let Err(e) = sqlx::query(query).execute(&state.db).await {
                    tracing::warn!(cleanup = label, "operational counter clean-up failed: {e}");
                }
            }

            // Dismissed notifications are UI history, while read notifications
            // older than one year no longer need to consume the primary DB.
            if let Err(e) = sqlx::query(
                "DELETE FROM user_notifications
                 WHERE (dismissed_at IS NOT NULL AND dismissed_at < now() - interval '90 days')
                    OR (dismissed_at IS NULL AND read_at IS NOT NULL AND read_at < now() - interval '365 days')",
            )
            .execute(&state.db)
            .await
            {
                tracing::warn!("notification history clean-up failed: {e}");
            }

            // MBOX import ledgers can contain one row per imported message. Once
            // an import is long finished, the provider mailbox is authoritative;
            // keeping the crash-recovery ledger forever would dominate Postgres.
            let old_imports: Vec<(uuid::Uuid, String)> = sqlx::query_as(
                "SELECT id,storage_key FROM mailbox_imports
                 WHERE (status='completed' AND completed_at < now() - interval '30 days')
                    OR (status IN ('failed','cancelled') AND completed_at < now() - interval '90 days')
                 ORDER BY completed_at NULLS LAST LIMIT 500",
            )
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
            for (import_id, storage_key) in old_imports {
                if let Ok(path) = state.object_store.local_path(&storage_key) {
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            tracing::warn!(%import_id, %error, "mail import archive clean-up deferred");
                            continue;
                        }
                    }
                }
                if let Err(e) = sqlx::query("DELETE FROM mailbox_imports WHERE id=$1")
                    .bind(import_id)
                    .execute(&state.db)
                    .await
                {
                    tracing::warn!(%import_id, "mail import ledger clean-up failed: {e}");
                }
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
