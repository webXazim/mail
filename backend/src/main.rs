use std::sync::Arc;
use std::time::Duration;

use harbor_api::config::Config;
use harbor_api::metrics::Metrics;
use harbor_api::middleware::rate_limit::RateLimiter;
use harbor_api::router::build_router;
use harbor_api::services::provisioning::MailBridge;
use harbor_api::state::AppState;
use harbor_api::ws::{spawn_realtime, EventHub};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("harbor_api=info,tower_http=info,sqlx=warn"));

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

    let state = AppState {
        db: pool,
        jwt_secret: config.jwt_secret.clone(),
        jwt_access_ttl_secs: config.jwt_access_ttl_secs,
        jwt_refresh_ttl_secs: config.jwt_refresh_ttl_secs,
        cors_origins: config.cors_origins.clone(),
        hub: EventHub::new(),
        public_origin: config.public_origin.clone(),
        require_verification: config.require_verification,
        return_token_links: config.return_token_links,
        cookie_secure: config.cookie_secure,
        mail: MailBridge::new(
            config.mail_admin_url.clone(),
            config.mail_admin_username.clone(),
            config.mail_admin_secret.clone(),
            config.mail_default_domain.clone(),
            config.mail_account_quota_bytes,
        ),
        smtp: config.smtp.clone(),
        rate: RateLimiter::new(),
        metrics: Arc::new(Metrics::new()),
    };

    spawn_realtime(state.clone());
    spawn_maintenance(state.clone());
    harbor_api::handlers::schedule::spawn_worker(state.clone());

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
            if let Err(e) = sqlx::query("DELETE FROM email_tokens WHERE expires_at <= now()")
                .execute(&state.db)
                .await
            {
                tracing::warn!("email token clean-up failed: {e}");
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
