use std::net::SocketAddr;

use dotenvy::dotenv;

use crate::services::smtp::SmtpConfig;

/// Verified load-bearing configuration. Required vars fail-fast at boot;
/// everything else falls back to a safe local-development default.
#[derive(Clone, Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub public_origin: String,
    pub cors_origins: Vec<String>,
    pub database_url: String,
    pub db_max_connections: u32,
    pub jwt_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub jwt_refresh_ttl_secs: u64,
    /// When true (default), new accounts require email verification before
    /// they can sign in.  The verify token is included in the response JSON
    /// when `return_token_links` is also true — for development only.
    pub require_verification: bool,
    /// Echo the raw verification / reset link back in the response JSON.
    /// Intended for local development so flows are testable without SMTP.
    pub return_token_links: bool,
    /// Add `Secure` to the session cookie. Must be true once the API is
    /// served over HTTPS (public launch). False for local http dev.
    pub cookie_secure: bool,
    /// Stalwart admin API base URL (e.g. http://mail:8080/ inside compose).
    /// Empty disables the provisioning bridge (mailbox creation skipped).
    pub mail_admin_url: String,
    pub mail_admin_username: String,
    pub mail_admin_secret: String,
    /// Accounts are only provisioned under this domain (the Stalwart
    /// defaultDomain set during bootstrap).
    pub mail_default_domain: String,
    /// Default mailbox quota in bytes for freshly provisioned accounts.
    pub mail_account_quota_bytes: u64,
    /// SMTP endpoint used for outgoing mail (internal relay `mail:25` in dev).
    pub smtp: SmtpConfig,
    /// Log rendering: `json` emits one object per line for Loki/ELK shipping;
    /// anything else uses the human-readable default.
    pub log_format: String,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenv().ok();

        let public_origin = env_or("HARBOR_PUBLIC_ORIGIN", "http://localhost:5174");
        let cors_origins: Vec<String> = env_or("HARBOR_CORS_ORIGINS", "")
            .split(',')
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect();
        // An explicitly empty CORS list would make the router fall back to
        // "allow any origin" (insecure in production), so default to the
        // public origin instead.
        let cors_origins = if cors_origins.is_empty() {
            vec![public_origin.clone()]
        } else {
            cors_origins
        };

        let database_url = std::env::var("HARBOR_DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("HARBOR_DATABASE_URL is required"))?;
        let jwt_secret = std::env::var("HARBOR_JWT_SECRET")
            .map_err(|_| anyhow::anyhow!("HARBOR_JWT_SECRET is required"))?;
        // A short secret makes the HS256 signature trivially brute-forceable.
        if jwt_secret.len() < 32 {
            return Err(anyhow::anyhow!(
                "HARBOR_JWT_SECRET too short: got {} bytes (need >=32, e.g. openssl rand -base64 48)",
                jwt_secret.len()
            ));
        }

        Ok(Config {
            listen_addr: env_or("HARBOR_LISTEN_ADDR", "0.0.0.0:8080").parse()?,
            public_origin,
            cors_origins,
            database_url,
            db_max_connections: env_or("HARBOR_DB_MAX_CONNECTIONS", "10").parse()?,
            jwt_secret,
            jwt_access_ttl_secs: env_or("HARBOR_JWT_ACCESS_TTL_SECS", "900").parse()?,
            jwt_refresh_ttl_secs: env_or("HARBOR_JWT_REFRESH_TTL_SECS", "2592000").parse()?,
            require_verification: env_or("HARBOR_REQUIRE_VERIFICATION", "true")
                .parse()
                .unwrap_or(true),
            return_token_links: env_or("HARBOR_DEV_RETURN_TOKEN_LINKS", "1")
                .parse()
                .unwrap_or(true),
            cookie_secure: env_or("HARBOR_COOKIE_SECURE", "false")
                .parse()
                .unwrap_or(false),
            mail_admin_url: env_or("HARBOR_MAIL_ADMIN_URL", "").trim().to_string(),
            mail_admin_username: env_or("HARBOR_MAIL_ADMIN_USERNAME", "admin"),
            mail_admin_secret: env_or("HARBOR_MAIL_ADMIN_SECRET", ""),
            mail_default_domain: env_or("HARBOR_MAIL_DEFAULT_DOMAIN", "crescentsphere.com"),
            mail_account_quota_bytes: env_or("HARBOR_MAIL_ACCOUNT_QUOTA_BYTES", "2147483648")
                .parse()
                .unwrap_or(2147483648),
            smtp: SmtpConfig::from_env(),
            log_format: env_or("HARBOR_LOG_FORMAT", "text").trim().to_string(),
        })
    }
}
