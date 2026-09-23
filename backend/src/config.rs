use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use dotenvy::dotenv;
use sha2::{Digest, Sha256};

use crate::services::smtp::SmtpConfig;

/// Verified load-bearing configuration. Required vars fail-fast at boot;
/// everything else falls back to a safe local-development default.
#[derive(Clone)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub public_origin: String,
    pub cors_origins: Vec<String>,
    pub database_url: String,
    pub db_max_connections: u32,
    pub jwt_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub jwt_refresh_ttl_secs: u64,
    /// Independent HMAC secret for normalized provider delivery events.
    /// When unset, the machine intake endpoint remains disabled.
    pub delivery_event_secret: Option<String>,
    /// Realtime worker cadence and distributed mailbox lease policy.
    pub realtime_poll_secs: u64,
    pub realtime_lease_secs: u64,
    pub realtime_batch_size: i64,
    /// Durable websocket replay retention. Seven days by default.
    pub realtime_event_retention_secs: u64,
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
    /// TCP peers allowed to supply X-Real-IP/X-Forwarded-For. Direct clients
    /// can never spoof rate-limit identity through these headers.
    pub trusted_proxy_ips: Vec<IpAddr>,
    /// Stalwart management/JMAP base URL (e.g. http://mail:8080 inside compose).
    /// Empty deliberately disables the mail-provider integration.
    pub mail_admin_url: String,
    pub mail_admin_username: String,
    pub mail_admin_secret: String,
    /// Optional bearer/API token used only for management JMAP.
    pub mail_admin_token: Option<String>,
    /// Privileged Basic-auth credential used for cross-account JMAP mail
    /// operations. Stalwart API keys intentionally cannot authenticate mail
    /// protocols, so this is distinct from the management token.
    pub mail_jmap_username: String,
    pub mail_jmap_secret: String,
    /// Protected platform/system mail domain used for CS Mail-generated
    /// transactional messages and provider health checks. Customer domains
    /// are provisioned dynamically after DNS ownership verification.
    pub mail_default_domain: String,
    /// Namespace prefix used in Stalwart provider-object ownership markers.
    /// On a shared provider this must be stable and unique to CS Mail.
    pub provider_namespace: String,
    /// Public mail-client endpoint advertised to IMAP/SMTP applications.
    pub mail_client_host: String,
    pub mail_client_imap_port: u16,
    pub mail_client_smtp_port: u16,
    pub mail_client_max_app_passwords: i64,
    /// Durable MBOX import limits / worker policy.
    pub mail_import_max_bytes: u64,
    pub mail_import_message_max_bytes: u64,
    pub mail_import_poll_secs: u64,
    pub mail_import_lease_secs: u64,
    /// Timeout and retry policy for read-only provider API calls. Mutating
    /// calls are intentionally not retried at this layer.
    pub mail_request_timeout_secs: u64,
    pub mail_read_retries: usize,
    pub mail_retry_base_ms: u64,
    /// Passphrase used only by PostgreSQL pgcrypto to protect short-lived
    /// credentials in durable provisioning jobs. Use a separate 32+ byte
    /// random value in production. If omitted, a domain-separated value is
    /// derived from the already-required JWT secret for backwards rollout.
    pub provisioning_key: String,
    /// Independent passphrase used by PostgreSQL pgcrypto for TOTP secrets.
    /// Production should set CS_MAIL_TOTP_KEY to a separate 32+ character random value.
    pub two_factor_key: String,
    pub provisioning_poll_secs: u64,
    pub provisioning_lease_secs: u64,
    pub provisioning_retry_base_secs: u64,
    pub provisioning_reconcile_secs: u64,
    pub provisioning_max_attempts: i32,
    pub provisioning_batch_size: i64,
    /// Durable scheduled-send worker. Claims are leased in PostgreSQL so any
    /// number of API replicas may safely run the worker.
    pub schedule_poll_secs: u64,
    pub schedule_lease_secs: u64,
    pub schedule_retry_base_secs: u64,
    pub schedule_max_attempts: i32,
    pub schedule_batch_size: i64,
    /// Persistent staged-attachment storage. Bytes live here; PostgreSQL stores
    /// ownership/integrity/reference metadata only.
    pub attachment_store_dir: PathBuf,
    /// Per-user on-disk staging budget across uploads/drafts/scheduled mail.
    pub attachment_staging_quota_bytes: u64,
    /// Lifetime of an unattached upload before cleanup.
    pub attachment_upload_ttl_secs: u64,
    /// Minimum protection window while a draft references an attachment.
    pub attachment_draft_ttl_secs: u64,
    /// Short grace period after successful send/cancel/discard.
    pub attachment_consumed_grace_secs: u64,
    /// Cleanup worker cadence.
    pub attachment_cleanup_secs: u64,
    /// Testing switch: when true, placing an order activates the selected plan immediately
    /// while the invoice remains due for manual payment. Disable before real paid launch.
    pub billing_instant_activation: bool,
    /// SMTP endpoint used for outgoing mail (internal relay `mail:25` in dev).
    pub smtp: SmtpConfig,
    /// Log rendering: `json` emits one object per line for Loki/ELK shipping;
    /// anything else uses the human-readable default.
    pub log_format: String,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_non_empty_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenv().ok();

        let public_origin = env_or("CS_MAIL_PUBLIC_ORIGIN", "http://localhost:5174");
        let cors_origins: Vec<String> = env_or("CS_MAIL_CORS_ORIGINS", "")
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

        let trusted_proxy_ips: Vec<IpAddr> = env_or("CS_MAIL_TRUSTED_PROXY_IPS", "127.0.0.1,::1")
            .split(',')
            .filter_map(|value| value.trim().parse::<IpAddr>().ok())
            .collect();

        let database_url = std::env::var("CS_MAIL_DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("CS_MAIL_DATABASE_URL is required"))?;
        let jwt_secret = std::env::var("CS_MAIL_JWT_SECRET")
            .map_err(|_| anyhow::anyhow!("CS_MAIL_JWT_SECRET is required"))?;
        // A short secret makes the HS256 signature trivially brute-forceable.
        if jwt_secret.len() < 32 {
            return Err(anyhow::anyhow!(
                "CS_MAIL_JWT_SECRET too short: got {} bytes (need >=32, e.g. openssl rand -base64 48)",
                jwt_secret.len()
            ));
        }

        let delivery_event_secret = std::env::var("CS_MAIL_DELIVERY_EVENT_SECRET")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if delivery_event_secret.as_ref().is_some_and(|value| value.len() < 32) {
            return Err(anyhow::anyhow!(
                "CS_MAIL_DELIVERY_EVENT_SECRET must contain at least 32 characters"
            ));
        }

        let provisioning_key = match std::env::var("CS_MAIL_PROVISIONING_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            Some(key) => {
                if key.len() < 32 {
                    return Err(anyhow::anyhow!(
                        "CS_MAIL_PROVISIONING_KEY must contain at least 32 characters"
                    ));
                }
                key
            }
            None => {
                // Safe rollout fallback. Production should use an independent
                // key so JWT rotation and provisioning-secret rotation are
                // separate operational concerns.
                let mut hasher = Sha256::new();
                hasher.update(b"cs-mailer:provisioning-key:v1\0");
                hasher.update(jwt_secret.as_bytes());
                format!("{:x}", hasher.finalize())
            }
        };

        let two_factor_key = match std::env::var("CS_MAIL_TOTP_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            Some(key) => {
                if key.len() < 32 {
                    return Err(anyhow::anyhow!(
                        "CS_MAIL_TOTP_KEY must contain at least 32 characters"
                    ));
                }
                key
            }
            None => {
                let mut hasher = Sha256::new();
                hasher.update(b"cs-mailer:totp-key:v1\0");
                hasher.update(jwt_secret.as_bytes());
                format!("{:x}", hasher.finalize())
            }
        };

        Ok(Config {
            listen_addr: env_or("CS_MAIL_LISTEN_ADDR", "0.0.0.0:8080").parse()?,
            public_origin,
            cors_origins,
            database_url,
            db_max_connections: env_or("CS_MAIL_DB_MAX_CONNECTIONS", "10").parse()?,
            jwt_secret,
            jwt_access_ttl_secs: env_or("CS_MAIL_JWT_ACCESS_TTL_SECS", "900").parse()?,
            jwt_refresh_ttl_secs: env_or("CS_MAIL_JWT_REFRESH_TTL_SECS", "2592000").parse()?,
            delivery_event_secret,
            realtime_poll_secs: env_or("CS_MAIL_REALTIME_POLL_SECS", "2").parse().unwrap_or(2),
            realtime_lease_secs: env_or("CS_MAIL_REALTIME_LEASE_SECS", "60").parse().unwrap_or(60),
            realtime_batch_size: env_or("CS_MAIL_REALTIME_BATCH_SIZE", "20").parse().unwrap_or(20),
            realtime_event_retention_secs: env_or("CS_MAIL_REALTIME_EVENT_RETENTION_SECS", "604800").parse().unwrap_or(604800),
            require_verification: env_or("CS_MAIL_REQUIRE_VERIFICATION", "true")
                .parse()
                .unwrap_or(true),
            return_token_links: env_or("CS_MAIL_DEV_RETURN_TOKEN_LINKS", "false")
                .parse()
                .unwrap_or(false),
            cookie_secure: env_or("CS_MAIL_COOKIE_SECURE", "false")
                .parse()
                .unwrap_or(false),
            trusted_proxy_ips,
            mail_admin_url: env_or("CS_MAIL_MAIL_ADMIN_URL", "").trim().to_string(),
            mail_admin_username: env_or("CS_MAIL_MAIL_ADMIN_USERNAME", "admin"),
            mail_admin_secret: env_or("CS_MAIL_MAIL_ADMIN_SECRET", ""),
            mail_admin_token: {
                let token = env_or("CS_MAIL_MAIL_ADMIN_TOKEN", "").trim().to_string();
                (!token.is_empty()).then_some(token)
            },
            mail_jmap_username: env_non_empty_or(
                "CS_MAIL_MAIL_JMAP_USERNAME",
                &env_non_empty_or("CS_MAIL_MAIL_ADMIN_USERNAME", "admin"),
            ),
            mail_jmap_secret: env_non_empty_or(
                "CS_MAIL_MAIL_JMAP_SECRET",
                &env_or("CS_MAIL_MAIL_ADMIN_SECRET", ""),
            ),
            mail_default_domain: env_or("CS_MAIL_MAIL_DEFAULT_DOMAIN", "crescentsphere.com"),
            provider_namespace: {
                let value = env_non_empty_or("CS_MAIL_PROVIDER_NAMESPACE", "cs-mail").to_ascii_lowercase();
                let valid = value.len() >= 3
                    && value.len() <= 32
                    && value.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                    && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                if !valid {
                    return Err(anyhow::anyhow!(
                        "CS_MAIL_PROVIDER_NAMESPACE must be 3-32 lowercase letters, digits or hyphens"
                    ));
                }
                value
            },
            mail_client_host: env_non_empty_or("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com"),
            mail_client_imap_port: env_or("CS_MAIL_CLIENT_IMAP_PORT", "993").parse().unwrap_or(993),
            mail_client_smtp_port: env_or("CS_MAIL_CLIENT_SMTP_PORT", "465").parse().unwrap_or(465),
            mail_client_max_app_passwords: env_or("CS_MAIL_CLIENT_MAX_APP_PASSWORDS", "5").parse().unwrap_or(5),
            mail_import_max_bytes: env_or("CS_MAIL_IMPORT_MAX_BYTES", "2147483648").parse().unwrap_or(2 * 1024 * 1024 * 1024),
            mail_import_message_max_bytes: env_or("CS_MAIL_IMPORT_MESSAGE_MAX_BYTES", "52428800").parse().unwrap_or(50 * 1024 * 1024),
            mail_import_poll_secs: env_or("CS_MAIL_IMPORT_POLL_SECS", "5").parse().unwrap_or(5),
            mail_import_lease_secs: env_or("CS_MAIL_IMPORT_LEASE_SECS", "900").parse().unwrap_or(900),
            mail_request_timeout_secs: env_or("CS_MAIL_MAIL_REQUEST_TIMEOUT_SECS", "10")
                .parse()
                .unwrap_or(10),
            mail_read_retries: env_or("CS_MAIL_MAIL_READ_RETRIES", "2")
                .parse()
                .unwrap_or(2),
            mail_retry_base_ms: env_or("CS_MAIL_MAIL_RETRY_BASE_MS", "150")
                .parse()
                .unwrap_or(150),
            provisioning_key,
            two_factor_key,
            provisioning_poll_secs: env_or("CS_MAIL_PROVISIONING_POLL_SECS", "2")
                .parse()
                .unwrap_or(2),
            provisioning_lease_secs: env_or("CS_MAIL_PROVISIONING_LEASE_SECS", "120")
                .parse()
                .unwrap_or(120),
            provisioning_retry_base_secs: env_or("CS_MAIL_PROVISIONING_RETRY_BASE_SECS", "5")
                .parse()
                .unwrap_or(5),
            provisioning_reconcile_secs: env_or("CS_MAIL_PROVISIONING_RECONCILE_SECS", "300")
                .parse()
                .unwrap_or(300),
            provisioning_max_attempts: env_or("CS_MAIL_PROVISIONING_MAX_ATTEMPTS", "12")
                .parse()
                .unwrap_or(12),
            provisioning_batch_size: env_or("CS_MAIL_PROVISIONING_BATCH_SIZE", "10")
                .parse()
                .unwrap_or(10),
            schedule_poll_secs: env_or("CS_MAIL_SCHEDULE_POLL_SECS", "5")
                .parse()
                .unwrap_or(5),
            schedule_lease_secs: env_or("CS_MAIL_SCHEDULE_LEASE_SECS", "300")
                .parse()
                .unwrap_or(300),
            schedule_retry_base_secs: env_or("CS_MAIL_SCHEDULE_RETRY_BASE_SECS", "15")
                .parse()
                .unwrap_or(15),
            schedule_max_attempts: env_or("CS_MAIL_SCHEDULE_MAX_ATTEMPTS", "12")
                .parse()
                .unwrap_or(12),
            schedule_batch_size: env_or("CS_MAIL_SCHEDULE_BATCH_SIZE", "20")
                .parse()
                .unwrap_or(20),
            attachment_store_dir: PathBuf::from(env_non_empty_or(
                "CS_MAIL_ATTACHMENT_STORE_DIR",
                "/srv/attachments",
            )),
            attachment_staging_quota_bytes: env_or(
                "CS_MAIL_ATTACHMENT_STAGING_QUOTA_BYTES",
                "1073741824",
            )
            .parse()
            .unwrap_or(1024 * 1024 * 1024),
            attachment_upload_ttl_secs: env_or("CS_MAIL_ATTACHMENT_UPLOAD_TTL_SECS", "86400")
                .parse()
                .unwrap_or(86400),
            attachment_draft_ttl_secs: env_or("CS_MAIL_ATTACHMENT_DRAFT_TTL_SECS", "2592000")
                .parse()
                .unwrap_or(2592000),
            attachment_consumed_grace_secs: env_or(
                "CS_MAIL_ATTACHMENT_CONSUMED_GRACE_SECS",
                "3600",
            )
            .parse()
            .unwrap_or(3600),
            attachment_cleanup_secs: env_or("CS_MAIL_ATTACHMENT_CLEANUP_SECS", "900")
                .parse()
                .unwrap_or(900),
            billing_instant_activation: env_or("CS_MAIL_BILLING_INSTANT_ACTIVATION", "false")
                .parse()
                .unwrap_or(true),
            smtp: SmtpConfig::from_env(),
            log_format: env_or("CS_MAIL_LOG_FORMAT", "text").trim().to_string(),
        })
    }
}
