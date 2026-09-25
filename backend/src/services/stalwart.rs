//! Production integration boundary for the self-hosted mail server.
//!
//! The rest of the application talks to [`StalwartService`] instead of
//! creating ad-hoc HTTP or SMTP clients. This keeps authentication, timeout,
//! retry and error semantics consistent and gives later reconciliation work a
//! single provider boundary.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use thiserror::Error;

use crate::services::smtp::{self, SmtpConfig};

const CORE_CAPABILITY: &str = "urn:ietf:params:jmap:core";
const MANAGEMENT_CAPABILITY: &str = "urn:stalwart:jmap";
const MAIL_CAPABILITY: &str = "urn:ietf:params:jmap:mail";
const BLOB_CAPABILITY: &str = "urn:ietf:params:jmap:blob";
const SIEVE_CAPABILITY: &str = "urn:ietf:params:jmap:sieve";
pub const MAILER_DOMAIN_MARKER: &str = "CrescentSphere Mailer managed domain";

#[derive(Clone)]
pub struct StalwartConfig {
    pub admin_url: String,
    pub admin_username: String,
    pub admin_secret: String,
    /// Preferred production credential for management automation.
    pub admin_bearer_token: Option<String>,
    /// Basic-auth service credential for cross-account JMAP mail operations.
    pub mail_jmap_username: String,
    pub mail_jmap_secret: String,
    pub default_domain: String,
    /// Stable ownership namespace for provider objects on a shared Stalwart.
    pub ownership_namespace: String,
    pub request_timeout: Duration,
    /// Read-only JMAP calls may be retried after transient transport/5xx/429
    /// failures. Mutating calls are deliberately never retried here because
    /// doing so could duplicate side effects; durable reconciliation handles
    /// those in the next upgrade.
    pub read_retries: usize,
    pub retry_base_delay: Duration,
    pub smtp: SmtpConfig,
}

#[derive(Debug, Error)]
pub enum StalwartError {
    #[error("mail provider configuration error: {0}")]
    Configuration(String),
    #[error("mail provider integration is disabled")]
    Disabled,
    #[error("invalid mail address: {0}")]
    InvalidAddress(String),
    #[error("mail domain is not managed by this service: {0}")]
    UnsupportedDomain(String),
    #[error("mail provider transport failure during {operation}: {source}")]
    Transport {
        operation: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("mail provider HTTP failure during {operation}: {status}: {body}")]
    Http {
        operation: String,
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("mail provider returned invalid JSON during {operation}: {source}")]
    InvalidJson {
        operation: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("mail provider protocol failure during {operation}: {message}")]
    Protocol { operation: String, message: String },
    #[error("mail provider rejected {operation}: {kind}: {description}")]
    Rejected {
        operation: String,
        kind: String,
        description: String,
    },
    #[error("SMTP submission failed: {0}")]
    Submission(smtp::SmtpError),
}

impl StalwartError {
    pub fn delivery_uncertain(&self) -> bool {
        matches!(self, Self::Submission(error) if error.delivery_uncertain())
    }

    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport { .. } | Self::Submission(_) => true,
            Self::Http { status, .. } => {
                *status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            _ => false,
        }
    }

    /// Safe message for API clients. Detailed provider responses stay in logs.
    pub fn public_message(&self) -> &'static str {
        match self {
            Self::Configuration(_) | Self::Disabled => "Mail service is not configured",
            Self::InvalidAddress(_) => "Invalid mail address",
            Self::UnsupportedDomain(_) => "Mail domain is not available",
            Self::Rejected { .. } => "Mail service rejected the operation",
            Self::Submission(error) => error.public_message(),
            _ => "Mail service is temporarily unavailable",
        }
    }
}

impl From<StalwartError> for String {
    fn from(value: StalwartError) -> Self {
        value.to_string()
    }
}

#[derive(Clone)]
pub struct StalwartService {
    client: reqwest::Client,
    config: StalwartConfig,
}

#[derive(Debug, Clone)]
pub struct ProviderDomainSnapshot {
    pub id: String,
    pub name: String,
    pub description: String,
    pub dns_zone_file: String,
    pub enabled: bool,
}

/// One CS Mail-owned receive alias attached to a provider account. `marker`
/// is persisted in the provider description so reconciliation can distinguish
/// our entries from aliases an operator manages directly in the mail server.
#[derive(Debug, Clone)]
pub struct ManagedAccountAlias {
    pub marker: String,
    pub local: String,
    pub domain: String,
    pub enabled: bool,
}

/// A CS Mail-owned external forwarding address represented by a provider
/// MailingList. A one-recipient mailing list is intentional here: Stalwart
/// models external distribution addresses as first-class MailingList objects.
#[derive(Debug, Clone)]
pub struct ManagedMailingList {
    pub marker: String,
    pub local: String,
    pub domain: String,
    pub recipient: String,
    pub enabled: bool,
}

#[derive(Clone, Copy)]
enum RetryMode {
    Never,
    ReadOnly,
}

#[derive(Clone, Copy)]
enum AuthMode {
    Management,
    Mail,
}

impl StalwartService {
    pub fn new(config: StalwartConfig) -> Result<Self, StalwartError> {
        let enabled = !config.admin_url.trim().is_empty();
        let has_bearer = config
            .admin_bearer_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
        let has_basic = !config.admin_username.trim().is_empty()
            && !config.admin_secret.trim().is_empty();
        if enabled && !has_bearer && !has_basic {
            return Err(StalwartError::Configuration(
                "management URL is set but no API token or Basic credential is configured".into(),
            ));
        }
        if enabled
            && (config.mail_jmap_username.trim().is_empty()
                || config.mail_jmap_secret.trim().is_empty())
        {
            return Err(StalwartError::Configuration(
                "cross-account JMAP mail credential is required".into(),
            ));
        }
        if enabled && config.ownership_namespace.trim().is_empty() {
            return Err(StalwartError::Configuration(
                "provider ownership namespace must not be empty".into(),
            ));
        }
        if enabled && config.default_domain.trim().is_empty() {
            return Err(StalwartError::Configuration(
                "default mail domain must not be empty".into(),
            ));
        }
        if config.request_timeout.is_zero() {
            return Err(StalwartError::Configuration(
                "request timeout must be greater than zero".into(),
            ));
        }

        let client = reqwest::Client::builder()
            .connect_timeout(config.request_timeout.min(Duration::from_secs(5)))
            .timeout(config.request_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|source| StalwartError::Transport {
                operation: "client initialization".to_string(),
                source,
            })?;
        Ok(Self { client, config })
    }

    pub fn enabled(&self) -> bool {
        !self.config.admin_url.trim().is_empty()
    }

    pub fn default_domain(&self) -> &str {
        &self.config.default_domain
    }

    /// Stable marker used for every tenant-owned provider object. Keeping the
    /// namespace inside the provider boundary prevents one CrescentSphere
    /// product from adopting or deleting objects created by another product
    /// on the same Stalwart instance.
    pub fn ownership_marker(&self, scope: &str, organization_id: &str, object_id: &str) -> String {
        format!(
            "{}:organization:{}:{}:{}",
            self.config.ownership_namespace, organization_id, scope, object_id
        )
    }

    fn require_owned_marker(&self, marker: &str) -> Result<(), StalwartError> {
        let prefix = format!("{}:organization:", self.config.ownership_namespace);
        if marker.starts_with(&prefix) {
            return Ok(());
        }
        Err(StalwartError::Configuration(format!(
            "provider object marker is outside the configured ownership namespace {}",
            self.config.ownership_namespace
        )))
    }

    /// Read-only management JMAP call. Transient failures may be retried.
    pub(crate) async fn management_read(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, MANAGEMENT_CAPABILITY],
            RetryMode::ReadOnly,
            AuthMode::Management,
        )
        .await
    }

    /// Mutating management JMAP call. It is intentionally single-attempt.
    pub(crate) async fn management_write(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, MANAGEMENT_CAPABILITY],
            RetryMode::Never,
            AuthMode::Management,
        )
        .await
    }

    /// Read-only mail JMAP call. Transient failures may be retried.
    pub(crate) async fn mail_read(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, MAIL_CAPABILITY, BLOB_CAPABILITY],
            RetryMode::ReadOnly,
            AuthMode::Mail,
        )
        .await
    }

    /// Mutating mail JMAP call. It is intentionally single-attempt.
    pub(crate) async fn mail_write(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, MAIL_CAPABILITY, BLOB_CAPABILITY],
            RetryMode::Never,
            AuthMode::Mail,
        )
        .await
    }

    /// Read-only JMAP for Sieve call using the privileged cross-account mail
    /// credential. This is the provider boundary for CS Mail's managed
    /// incoming-mail automation.
    pub(crate) async fn sieve_read(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, BLOB_CAPABILITY, SIEVE_CAPABILITY],
            RetryMode::ReadOnly,
            AuthMode::Mail,
        )
        .await
    }

    /// Mutating JMAP for Sieve call. It is deliberately single-attempt; the
    /// durable automation reconciliation queue owns retries.
    pub(crate) async fn sieve_write(
        &self,
        method: &str,
        args: Value,
    ) -> Result<Value, StalwartError> {
        self.jmap_call(
            method,
            args,
            &[CORE_CAPABILITY, BLOB_CAPABILITY, SIEVE_CAPABILITY],
            RetryMode::Never,
            AuthMode::Mail,
        )
        .await
    }

    /// Upload raw Sieve source to the account-scoped JMAP upload endpoint and
    /// return the resulting blob id (RFC 9661 §2.2).
    pub(crate) async fn upload_sieve(
        &self,
        account: &str,
        script: &str,
    ) -> Result<String, StalwartError> {
        if !self.enabled() {
            return Err(StalwartError::Disabled);
        }
        let url = format!(
            "{}/jmap/upload/{}/",
            self.config.admin_url.trim_end_matches('/'),
            account
        );
        let login = self.impersonation_login(account).await?;
        let response = self
            .client
            .post(url)
            .basic_auth(&login, Some(&self.config.mail_jmap_secret))
            .header(reqwest::header::CONTENT_TYPE, "application/sieve; charset=utf-8")
            .body(script.as_bytes().to_vec())
            .send()
            .await
            .map_err(|source| StalwartError::Transport {
                operation: "Sieve upload".to_string(),
                source,
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|source| StalwartError::Transport {
            operation: "Sieve upload response body".to_string(),
            source,
        })?;
        if !status.is_success() {
            return Err(StalwartError::Http {
                operation: "Sieve upload".to_string(),
                status,
                body: truncate(&text, 2048),
            });
        }
        let payload: Value = serde_json::from_str(&text).map_err(|source| {
            StalwartError::InvalidJson {
                operation: "Sieve upload".to_string(),
                source,
            }
        })?;
        payload
            .get("blobId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| StalwartError::Protocol {
                operation: "Sieve upload".to_string(),
                message: "upload response did not contain blobId".to_string(),
            })
    }

    /// Stalwart's impersonation credential is target%service-user, not the
    /// service user's login on its own. Resolve the target from its provider
    /// account id so mail operations never authenticate into the wrong inbox.
    async fn impersonation_login(&self, account: &str) -> Result<String, StalwartError> {
        // Management reads use jmap_call too. Box this nested future to break
        // the async call-size cycle through jmap_call -> impersonation_login.
        let result = Box::pin(self.management_read("x:Account/get", json!({ "ids": [account] })))
            .await?;
        let item = result
            .get("list")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "mail impersonation".to_string(),
                message: "target account was not returned by x:Account/get".to_string(),
            })?;
        let address = if let Some(address) = item
            .get("emailAddress")
            .and_then(Value::as_str)
            .filter(|value| value.contains('@'))
        {
            address.to_string()
        } else {
            let local = item.get("name").and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StalwartError::Protocol {
                    operation: "mail impersonation".to_string(),
                    message: "target account has no login name".to_string(),
                })?;
            let domain_id = item.get("domainId").and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StalwartError::Protocol {
                    operation: "mail impersonation".to_string(),
                    message: "target account has no domain id".to_string(),
                })?;
            let domain = Box::pin(self.management_read("x:Domain/get", json!({ "ids": [domain_id] }))).await?;
            let name = domain.get("list").and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("name"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StalwartError::Protocol {
                    operation: "mail impersonation".to_string(),
                    message: "target domain has no name".to_string(),
                })?;
            format!("{local}@{name}")
        };
        Ok(format!("{address}%{}", self.config.mail_jmap_username))
    }

    async fn jmap_call(
        &self,
        method: &str,
        args: Value,
        using: &[&str],
        retry: RetryMode,
        auth: AuthMode,
    ) -> Result<Value, StalwartError> {
        if !self.enabled() {
            return Err(StalwartError::Disabled);
        }

        let mail_login = if matches!(auth, AuthMode::Mail) {
            let account = args.get("accountId").and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StalwartError::Protocol {
                    operation: method.to_string(),
                    message: "mail JMAP request is missing accountId".to_string(),
                })?;
            Some(self.impersonation_login(account).await?)
        } else {
            None
        };
        let body = json!({
            "methodCalls": [[method, args, "c1"]],
            "using": using,
        });
        let url = format!("{}/jmap", self.config.admin_url.trim_end_matches('/'));
        let attempts = match retry {
            RetryMode::Never => 1,
            RetryMode::ReadOnly => self.config.read_retries.saturating_add(1),
        };

        for attempt in 0..attempts {
            let request = self.client.post(&url).json(&body);
            let request = match auth {
                AuthMode::Management => {
                    if let Some(token) = self
                        .config
                        .admin_bearer_token
                        .as_deref()
                        .filter(|token| !token.is_empty())
                    {
                        request.bearer_auth(token)
                    } else {
                        request.basic_auth(
                            &self.config.admin_username,
                            Some(&self.config.admin_secret),
                        )
                    }
                }
                AuthMode::Mail => request.basic_auth(
                    mail_login.as_deref().expect("mail login was resolved"),
                    Some(&self.config.mail_jmap_secret),
                ),
            };
            let response = request.send().await;

            let response = match response {
                Ok(response) => response,
                Err(source) => {
                    let error = StalwartError::Transport {
                        operation: method.to_string(),
                        source,
                    };
                    if matches!(retry, RetryMode::ReadOnly)
                        && error.is_transient()
                        && attempt + 1 < attempts
                    {
                        self.retry_sleep(attempt).await;
                        continue;
                    }
                    return Err(error);
                }
            };

            let status = response.status();
            let text = response.text().await.map_err(|source| StalwartError::Transport {
                operation: format!("{method} response body"),
                source,
            })?;

            if !status.is_success() {
                let error = StalwartError::Http {
                    operation: method.to_string(),
                    status,
                    body: truncate(&text, 2048),
                };
                if matches!(retry, RetryMode::ReadOnly)
                    && error.is_transient()
                    && attempt + 1 < attempts
                {
                    self.retry_sleep(attempt).await;
                    continue;
                }
                return Err(error);
            }

            let parsed: Value = serde_json::from_str(&text).map_err(|source| {
                StalwartError::InvalidJson {
                    operation: method.to_string(),
                    source,
                }
            })?;
            let response = parsed
                .get("methodResponses")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_array)
                .ok_or_else(|| StalwartError::Protocol {
                    operation: method.to_string(),
                    message: "missing methodResponses".to_string(),
                })?;

            let response_name = response.first().and_then(Value::as_str).unwrap_or_default();
            let payload = response.get(1).cloned().ok_or_else(|| StalwartError::Protocol {
                operation: method.to_string(),
                message: "missing response payload".to_string(),
            })?;

            if response_name == "error" {
                return Err(StalwartError::Rejected {
                    operation: method.to_string(),
                    kind: payload
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    description: payload
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("request rejected")
                        .to_string(),
                });
            }

            return Ok(payload);
        }

        Err(StalwartError::Protocol {
            operation: method.to_string(),
            message: "retry loop exhausted".to_string(),
        })
    }

    async fn credential_call(
        &self,
        method: &str,
        args: Value,
        username: &str,
        password: &str,
    ) -> Result<Value, StalwartError> {
        if !self.enabled() {
            return Err(StalwartError::Disabled);
        }
        let body = json!({
            "methodCalls": [[method, args, "c1"]],
            "using": [CORE_CAPABILITY, MANAGEMENT_CAPABILITY],
        });
        // Native account-management credential objects are served from the
        // provider management JMAP endpoint (`/api` in current Stalwart), not
        // from the normal mail JMAP endpoint.
        let url = format!("{}/api", self.config.admin_url.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .basic_auth(username, Some(password))
            .json(&body)
            .send()
            .await
            .map_err(|source| StalwartError::Transport {
                operation: method.to_string(),
                source,
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|source| StalwartError::Transport {
            operation: format!("{method} response body"),
            source,
        })?;
        if !status.is_success() {
            return Err(StalwartError::Http {
                operation: method.to_string(),
                status,
                body: truncate(&text, 2048),
            });
        }
        let parsed: Value = serde_json::from_str(&text).map_err(|source| {
            StalwartError::InvalidJson {
                operation: method.to_string(),
                source,
            }
        })?;
        let response = parsed
            .get("methodResponses")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_array)
            .ok_or_else(|| StalwartError::Protocol {
                operation: method.to_string(),
                message: "missing methodResponses".to_string(),
            })?;
        let response_name = response.first().and_then(Value::as_str).unwrap_or_default();
        let payload = response.get(1).cloned().ok_or_else(|| StalwartError::Protocol {
            operation: method.to_string(),
            message: "missing response payload".to_string(),
        })?;
        if response_name == "error" {
            return Err(StalwartError::Rejected {
                operation: method.to_string(),
                kind: payload.get("type").and_then(Value::as_str).unwrap_or("unknown").to_string(),
                description: payload.get("description").and_then(Value::as_str).unwrap_or("request rejected").to_string(),
            });
        }
        Ok(payload)
    }

    /// Create one Stalwart-native application password while authenticated as
    /// the mailbox itself. Stalwart intentionally does not permit an
    /// administrator to create app passwords on a user's behalf, so callers
    /// use a short-lived provider password and rotate it immediately after
    /// this call. The returned secret is shown once and is never persisted by
    /// CS Mail.
    pub async fn create_app_password(
        &self,
        username: &str,
        password: &str,
        description: &str,
        expires_at: Option<&str>,
        allowed_ips: &[String],
    ) -> Result<(String, String), StalwartError> {
        let allowed = allowed_ips
            .iter()
            .map(|ip| (ip.clone(), Value::Bool(true)))
            .collect::<serde_json::Map<String, Value>>();
        let mut credential = serde_json::Map::new();
        credential.insert("description".into(), json!(description));
        credential.insert("permissions".into(), json!({ "@type": "Inherit" }));
        credential.insert("allowedIps".into(), Value::Object(allowed));
        if let Some(expires_at) = expires_at.filter(|value| !value.trim().is_empty()) {
            credential.insert("expiresAt".into(), json!(expires_at));
        }
        let result = self
            .credential_call(
                "x:AppPassword/set",
                json!({ "create": { "new1": Value::Object(credential) } }),
                username,
                password,
            )
            .await?;
        let created = result
            .get("created")
            .and_then(|v| v.get("new1"))
            .ok_or_else(|| {
                let detail = result.get("notCreated").and_then(|v| v.get("new1")).cloned().unwrap_or(result.clone());
                provider_rejection("app password create", &detail)
            })?;
        let id = created.get("id").and_then(Value::as_str).filter(|v| !v.is_empty())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "app password create".into(),
                message: "provider did not return the application-password id".into(),
            })?;
        let secret = created.get("secret").and_then(Value::as_str).filter(|v| !v.is_empty())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "app password create".into(),
                message: "provider did not return the one-time application-password secret".into(),
            })?;
        Ok((id.to_string(), secret.to_string()))
    }

    pub async fn destroy_app_password(
        &self,
        username: &str,
        password: &str,
        credential_id: &str,
    ) -> Result<(), StalwartError> {
        let result = self
            .credential_call(
                "x:AppPassword/set",
                json!({ "destroy": [credential_id] }),
                username,
                password,
            )
            .await?;
        if let Some(reason) = result
            .get("notDestroyed")
            .and_then(|v| v.get(credential_id))
            .filter(|v| !v.is_null())
        {
            // Revoke is intentionally idempotent. If a previous request was
            // committed but its response was lost, retrying must converge.
            if reason.get("type").and_then(Value::as_str) == Some("notFound") {
                return Ok(());
            }
            return Err(provider_rejection("app password revoke", reason));
        }
        Ok(())
    }

    /// Best-effort cleanup for an ambiguous create response. A unique
    /// description marker lets CS Mail find a provider credential that may
    /// have been committed even if the create HTTP response was lost.
    pub async fn destroy_app_passwords_by_description(
        &self,
        username: &str,
        password: &str,
        description: &str,
    ) -> Result<usize, StalwartError> {
        let result = self
            .credential_call("x:AppPassword/get", json!({ "ids": null }), username, password)
            .await?;
        let ids = result
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("description").and_then(Value::as_str) == Some(description))
            .filter_map(|item| item.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(0);
        }
        let destroyed = self
            .credential_call("x:AppPassword/set", json!({ "destroy": ids }), username, password)
            .await?;
        if destroyed
            .get("notDestroyed")
            .and_then(Value::as_object)
            .is_some_and(|items| !items.is_empty())
        {
            return Err(StalwartError::Protocol {
                operation: "app password orphan cleanup".into(),
                message: "provider could not destroy every matching credential".into(),
            });
        }
        Ok(destroyed
            .get("destroyed")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0))
    }

    /// Upload one RFC 5322 message into a target account. The privileged mail
    /// JMAP credential is used only server-to-server; imported messages never
    /// require or expose the mailbox's primary provider password.
    pub async fn upload_email_blob(
        &self,
        account: &str,
        message: &[u8],
    ) -> Result<String, StalwartError> {
        if !self.enabled() {
            return Err(StalwartError::Disabled);
        }
        let url = format!(
            "{}/jmap/upload/{}/",
            self.config.admin_url.trim_end_matches('/'),
            account
        );
        let login = self.impersonation_login(account).await?;
        let response = self
            .client
            .post(url)
            .basic_auth(&login, Some(&self.config.mail_jmap_secret))
            .header(reqwest::header::CONTENT_TYPE, "message/rfc822")
            .body(message.to_vec())
            .send()
            .await
            .map_err(|source| StalwartError::Transport {
                operation: "mail import upload".into(),
                source,
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|source| StalwartError::Transport {
            operation: "mail import upload response body".into(),
            source,
        })?;
        if !status.is_success() {
            return Err(StalwartError::Http {
                operation: "mail import upload".into(),
                status,
                body: truncate(&text, 2048),
            });
        }
        let payload: Value = serde_json::from_str(&text).map_err(|source| StalwartError::InvalidJson {
            operation: "mail import upload".into(),
            source,
        })?;
        payload.get("blobId").and_then(Value::as_str).filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| StalwartError::Protocol {
                operation: "mail import upload".into(),
                message: "provider upload response did not contain blobId".into(),
            })
    }

    pub async fn inbox_id(&self, account: &str) -> Result<String, StalwartError> {
        let result = self
            .mail_read(
                "Mailbox/get",
                json!({ "accountId": account, "properties": ["id", "role", "name"] }),
            )
            .await?;
        result.get("list").and_then(Value::as_array).into_iter().flatten()
            .find(|mailbox| mailbox.get("role").and_then(Value::as_str) == Some("inbox"))
            .and_then(|mailbox| mailbox.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .ok_or_else(|| StalwartError::Protocol {
                operation: "mail import inbox lookup".into(),
                message: "target account has no Inbox mailbox".into(),
            })
    }

    pub async fn import_email_blob(
        &self,
        account: &str,
        blob_id: &str,
        mailbox_id: &str,
        received_at: Option<&str>,
        import_keyword: Option<&str>,
    ) -> Result<String, StalwartError> {
        let mut spec = serde_json::Map::new();
        spec.insert("blobId".into(), json!(blob_id));
        spec.insert("mailboxIds".into(), json!({ mailbox_id: true }));
        let mut keywords = serde_json::Map::new();
        if let Some(keyword) = import_keyword.filter(|value| !value.trim().is_empty()) {
            keywords.insert(keyword.to_string(), Value::Bool(true));
        }
        spec.insert("keywords".into(), Value::Object(keywords));
        if let Some(received_at) = received_at.filter(|value| !value.trim().is_empty()) {
            spec.insert("receivedAt".into(), json!(received_at));
        }
        let result = self
            .mail_write(
                "Email/import",
                json!({ "accountId": account, "emails": { "m1": Value::Object(spec) } }),
            )
            .await?;
        if let Some(created) = result.get("created").and_then(|v| v.get("m1")) {
            return created.get("id").and_then(Value::as_str).filter(|v| !v.is_empty())
                .map(str::to_string)
                .ok_or_else(|| StalwartError::Protocol {
                    operation: "mail import".into(),
                    message: "provider did not return imported email id".into(),
                });
        }
        let detail = result.get("notCreated").and_then(|v| v.get("m1")).cloned().unwrap_or(result);
        Err(provider_rejection("mail import", &detail))
    }

    pub async fn find_email_by_keyword(
        &self,
        account: &str,
        keyword: &str,
    ) -> Result<Option<String>, StalwartError> {
        let result = self.mail_read(
            "Email/query",
            json!({
                "accountId": account,
                "filter": { "hasKeyword": keyword },
                "position": 0,
                "limit": 1
            }),
        ).await?;
        Ok(result.get("ids").and_then(Value::as_array).and_then(|ids| ids.first())
            .and_then(Value::as_str).map(str::to_string))
    }

    pub async fn remove_email_keyword(
        &self,
        account: &str,
        email_id: &str,
        keyword: &str,
    ) -> Result<(), StalwartError> {
        let mut patch = serde_json::Map::new();
        patch.insert(format!("keywords/{keyword}"), Value::Null);
        let mut update = serde_json::Map::new();
        update.insert(email_id.to_string(), Value::Object(patch));
        let result = self.mail_write(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        ).await?;
        if let Some(reason) = result.get("notUpdated").and_then(|v| v.get(email_id)).filter(|v| !v.is_null()) {
            return Err(provider_rejection("mail import marker cleanup", reason));
        }
        Ok(())
    }

    async fn retry_sleep(&self, attempt: usize) {
        let exponent = (attempt as u32).min(6);
        let factor = 1u32 << exponent;
        tokio::time::sleep(self.config.retry_base_delay.saturating_mul(factor)).await;
    }

    pub(crate) async fn domain_id(&self, domain: &str) -> Result<Option<String>, StalwartError> {
        let result = self
            .management_read("x:Domain/query", json!({ "filter": { "name": domain } }))
            .await?;
        Ok(result
            .get("ids")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub async fn customer_domain_snapshot(&self, id: &str) -> Result<ProviderDomainSnapshot, StalwartError> {
        let result = self.management_read("x:Domain/get", json!({ "ids": [id] })).await?;
        let item = result.get("list").and_then(Value::as_array).and_then(|items| items.first())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "customer domain snapshot".to_string(),
                message: "domain was not returned by x:Domain/get".to_string(),
            })?;
        Ok(ProviderDomainSnapshot {
            id: item.get("id").and_then(Value::as_str).unwrap_or(id).to_string(),
            name: item.get("name").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase(),
            description: item.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            dns_zone_file: item.get("dnsZoneFile").and_then(Value::as_str).unwrap_or_default().to_string(),
            enabled: item.get("isEnabled").and_then(Value::as_bool).unwrap_or(true),
        })
    }

    /// Create a customer-owned domain idempotently without ever adopting an
    /// unrelated provider domain that happens to have the same name. This is
    /// critical because CS Mail shares the Stalwart instance with another
    /// CrescentSphere mail application.
    pub async fn ensure_customer_domain(
        &self,
        existing_id: Option<&str>,
        domain: &str,
        marker: &str,
    ) -> Result<ProviderDomainSnapshot, StalwartError> {
        if !self.enabled() { return Err(StalwartError::Disabled); }
        self.require_owned_marker(marker)?;
        if let Some(id) = existing_id.filter(|v| !v.trim().is_empty()) {
            let snapshot = self.customer_domain_snapshot(id).await?;
            if snapshot.name != domain.to_ascii_lowercase() || snapshot.description != marker {
                return Err(StalwartError::Protocol {
                    operation: "customer domain reconciliation".to_string(),
                    message: "stored provider domain id does not belong to this CS Mail domain".to_string(),
                });
            }
            return Ok(snapshot);
        }

        if let Some(id) = self.domain_id(domain).await? {
            let snapshot = self.customer_domain_snapshot(&id).await?;
            if snapshot.description == marker { return Ok(snapshot); }
            return Err(StalwartError::Protocol {
                operation: "customer domain create".to_string(),
                message: format!("provider domain {domain} already exists and is not owned by this CS Mail claim"),
            });
        }

        let result = self.management_write(
            "x:Domain/set",
            json!({
                "create": {
                    "new1": {
                        "name": domain,
                        "description": marker,
                        "aliases": {},
                        "isEnabled": true,
                        "certificateManagement": {"@type": "Manual"},
                        "dkimManagement": {"@type": "Automatic"},
                        "dnsManagement": {"@type": "Manual"},
                        "subAddressing": {"@type": "Enabled"},
                        "allowRelaying": false,
                        "reportAddressUri": null
                    }
                }
            }),
        ).await?;
        if let Some(reason) = result.get("notCreated").and_then(|v| v.get("new1")).filter(|v| !v.is_null()) {
            return Err(provider_rejection("customer domain create", reason));
        }
        let id = result.get("created").and_then(|v| v.get("new1")).and_then(|v| v.get("id")).and_then(Value::as_str)
            .map(str::to_string);
        let id = match id {
            Some(id) => id,
            None => self.domain_id(domain).await?.ok_or_else(|| StalwartError::Protocol {
                operation: "customer domain create".to_string(),
                message: "provider accepted create but did not return or expose the new domain".to_string(),
            })?,
        };
        let snapshot = self.customer_domain_snapshot(&id).await?;
        if snapshot.description != marker {
            return Err(StalwartError::Protocol {
                operation: "customer domain create".to_string(),
                message: "created provider domain ownership marker does not match".to_string(),
            });
        }
        Ok(snapshot)
    }

    /// Bind a DNS-verified CS Mail claim to an existing Mailer domain without
    /// changing that domain's provider ownership or configuration.
    pub async fn ensure_customer_or_mailer_domain(
        &self,
        existing_id: Option<&str>,
        domain: &str,
        marker: &str,
        allow_mailer_binding: bool,
    ) -> Result<(ProviderDomainSnapshot, bool), StalwartError> {
        self.require_owned_marker(marker)?;
        let id = match existing_id.filter(|value| !value.trim().is_empty()) {
            Some(id) => Some(id.to_owned()),
            None => self.domain_id(domain).await?,
        };
        if let Some(id) = id {
            let snapshot = self.customer_domain_snapshot(&id).await?;
            if snapshot.name != domain.to_ascii_lowercase() || !snapshot.enabled {
                return Err(StalwartError::Protocol {
                    operation: "shared domain reconciliation".to_string(),
                    message: "provider domain name or enabled state does not match".to_string(),
                });
            }
            if snapshot.description == marker {
                return Ok((snapshot, false));
            }
            if allow_mailer_binding && snapshot.description == MAILER_DOMAIN_MARKER {
                return Ok((snapshot, true));
            }
            return Err(StalwartError::Protocol {
                operation: "shared domain reconciliation".to_string(),
                message: format!("provider domain {domain} already exists and has an unrelated ownership marker"),
            });
        }
        self.ensure_customer_domain(None, domain, marker).await.map(|snapshot| (snapshot, false))
    }

    pub async fn healthcheck(&self) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Err(StalwartError::Disabled);
        }
        match self.domain_id(&self.config.default_domain).await? {
            Some(_) => Ok(()),
            None => Err(StalwartError::Protocol {
                operation: "healthcheck".to_string(),
                message: format!(
                    "configured domain {} does not exist",
                    self.config.default_domain
                ),
            }),
        }
    }

    /// Replace only the aliases owned by CS Mail while preserving unrelated
    /// provider-side aliases. JMAP state preconditions prevent a concurrent
    /// provider edit from being silently overwritten.
    pub async fn sync_account_aliases(
        &self,
        account: &str,
        desired: &[ManagedAccountAlias],
    ) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }
        let current = self
            .management_read("x:Account/get", json!({ "ids": [account] }))
            .await?;
        let item = current
            .get("list")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "alias reconciliation".to_string(),
                message: "account was not returned by x:Account/get".to_string(),
            })?;

        let mut aliases = Vec::<Value>::new();
        if let Some(existing) = item.get("aliases") {
            if let Some(map) = existing.as_object() {
                let mut entries = map.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(key, _)| key.parse::<usize>().unwrap_or(usize::MAX));
                for (_, value) in entries {
                    if !provider_alias_is_managed(value) {
                        aliases.push(value.clone());
                    }
                }
            } else if let Some(list) = existing.as_array() {
                for value in list {
                    if !provider_alias_is_managed(value) {
                        aliases.push(value.clone());
                    }
                }
            }
        }

        for spec in desired {
            let Some(domain_id) = self.domain_id(&spec.domain).await? else {
                return Err(StalwartError::Protocol {
                    operation: "alias reconciliation".to_string(),
                    message: format!("managed domain {} does not exist", spec.domain),
                });
            };
            aliases.push(json!({
                "name": spec.local,
                "domainId": domain_id,
                "enabled": spec.enabled,
                "description": spec.marker,
            }));
        }

        let alias_map = aliases
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect::<serde_json::Map<String, Value>>();
        let mut update = serde_json::Map::new();
        update.insert(account.to_string(), json!({ "aliases": Value::Object(alias_map) }));
        let mut args = json!({ "update": Value::Object(update) });
        if let Some(state) = current.get("state").and_then(Value::as_str) {
            args["ifInState"] = json!(state);
        }
        let result = self.management_write("x:Account/set", args).await?;
        if let Some(reason) = result
            .get("notUpdated")
            .and_then(|value| value.get(account))
            .filter(|value| !value.is_null())
        {
            return Err(provider_rejection("alias reconciliation", reason));
        }
        Ok(())
    }

    async fn managed_mailing_list_binding(&self, id: &str, marker: &str) -> Result<bool, StalwartError> {
        let result = self.management_read("x:MailingList/get", json!({"ids":[id]})).await?;
        let Some(item) = result.get("list").and_then(Value::as_array).and_then(|items| items.first()) else {
            return Ok(false);
        };
        if item.get("description").and_then(Value::as_str) != Some(marker) {
            return Err(StalwartError::Protocol {
                operation: "managed mailing-list ownership check".to_string(),
                message: "stored provider mailing-list id does not match the expected CS Mail marker".to_string(),
            });
        }
        Ok(true)
    }

    /// Create or update a one-recipient provider MailingList for an alias that
    /// routes to an external address. The description marker makes retries
    /// idempotent even if a prior create succeeded but its HTTP response was
    /// lost.
    pub async fn ensure_managed_mailing_list(
        &self,
        existing_id: Option<&str>,
        spec: &ManagedMailingList,
    ) -> Result<String, StalwartError> {
        if !self.enabled() {
            return Ok(existing_id.unwrap_or_default().to_string());
        }
        let Some(domain_id) = self.domain_id(&spec.domain).await? else {
            return Err(StalwartError::Protocol {
                operation: "external alias reconciliation".to_string(),
                message: format!("managed domain {} does not exist", spec.domain),
            });
        };
        let recipients = if spec.enabled {
            let mut values = serde_json::Map::new();
            values.insert(spec.recipient.clone(), Value::Bool(true));
            Value::Object(values)
        } else {
            json!({})
        };

        let known = match existing_id.filter(|id| !id.trim().is_empty()) {
            Some(id) => {
                if self.managed_mailing_list_binding(id, &spec.marker).await? { Some(id.to_string()) }
                else { self.find_managed_mailing_list(&spec.marker, &spec.local).await? }
            }
            None => self.find_managed_mailing_list(&spec.marker, &spec.local).await?,
        };
        if let Some(id) = known.as_deref() {
            self.managed_mailing_list_binding(id, &spec.marker).await?;
        }
        if let Some(id) = known {
            let mut update = serde_json::Map::new();
            update.insert(
                id.clone(),
                json!({
                    "name": spec.local,
                    "domainId": domain_id,
                    "description": spec.marker,
                    "recipients": recipients,
                }),
            );
            let result = self
                .management_write("x:MailingList/set", json!({ "update": Value::Object(update) }))
                .await?;
            if let Some(reason) = result
                .get("notUpdated")
                .and_then(|value| value.get(&id))
                .filter(|value| !value.is_null())
            {
                return Err(provider_rejection("external alias update", reason));
            }
            return Ok(id);
        }

        let result = self
            .management_write(
                "x:MailingList/set",
                json!({
                    "create": {
                        "new1": {
                            "name": spec.local,
                            "domainId": domain_id,
                            "description": spec.marker,
                            "aliases": {},
                            "recipients": recipients,
                        }
                    }
                }),
            )
            .await?;
        if let Some(id) = result
            .get("created")
            .and_then(|value| value.get("new1"))
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
        {
            return Ok(id.to_string());
        }
        if let Some(id) = self.find_managed_mailing_list(&spec.marker, &spec.local).await? {
            return Ok(id);
        }
        let reason = result
            .get("notCreated")
            .and_then(|value| value.get("new1"))
            .unwrap_or(&result);
        Err(provider_rejection("external alias create", reason))
    }

    pub async fn delete_managed_mailing_list(
        &self,
        existing_id: Option<&str>,
        marker: &str,
        local: &str,
    ) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }
        let id = match existing_id.filter(|id| !id.trim().is_empty()) {
            Some(id) => {
                if self.managed_mailing_list_binding(id, marker).await? { Some(id.to_string()) }
                else { self.find_managed_mailing_list(marker, local).await? }
            }
            None => self.find_managed_mailing_list(marker, local).await?,
        };
        let Some(id) = id else { return Ok(()); };
        self.managed_mailing_list_binding(&id, marker).await?;
        let result = self
            .management_write("x:MailingList/set", json!({ "destroy": [id.clone()] }))
            .await?;
        if let Some(reason) = result
            .get("notDestroyed")
            .and_then(|value| value.get(&id))
            .filter(|value| !value.is_null())
        {
            return Err(provider_rejection("external alias delete", reason));
        }
        Ok(())
    }

    async fn find_managed_mailing_list(
        &self,
        marker: &str,
        local: &str,
    ) -> Result<Option<String>, StalwartError> {
        let query = self
            .management_read("x:MailingList/query", json!({ "filter": { "text": local } }))
            .await?;
        let ids = query
            .get("ids")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(None);
        }
        let result = self
            .management_read("x:MailingList/get", json!({ "ids": ids }))
            .await?;
        Ok(result
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| item.get("description").and_then(Value::as_str) == Some(marker))
            .and_then(|item| item.get("id").and_then(Value::as_str))
            .map(str::to_string))
    }

    async fn find_account(&self, local: &str, domain_id: &str) -> Result<Option<String>, StalwartError> {
        let local = local.trim();
        if local.is_empty() || local.contains('@') || domain_id.is_empty() {
            return Err(StalwartError::InvalidAddress(local.to_string()));
        }
        let result = self
            .management_read("x:Account/query", json!({ "filter": { "name": local, "domainId": domain_id } }))
            .await?;
        let ids = result
            .get("ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(None);
        }
        let accounts = self
            .management_read("x:Account/get", json!({ "ids": ids }))
            .await?;
        let mut matches = accounts
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("@type").and_then(Value::as_str) == Some("User")
                    && item.get("name").and_then(Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(local))
                    && item.get("domainId").and_then(Value::as_str) == Some(domain_id)
            })
            .filter_map(|item| item.get("id").and_then(Value::as_str));
        let first = matches.next().map(str::to_string);
        if matches.next().is_some() {
            return Err(StalwartError::Protocol {
                operation: "account lookup".to_string(),
                message: "multiple exact provider accounts matched one mailbox".to_string(),
            });
        }
        Ok(first)
    }

    pub async fn find_account_by_email(
        &self,
        email: &str,
    ) -> Result<Option<String>, StalwartError> {
        let (local, domain) = split_managed_email(email, &self.config.default_domain)?;
        let Some(domain_id) = self.domain_id(domain).await? else {
            return Ok(None);
        };
        self.find_account(local, &domain_id).await
    }

    pub async fn find_owned_mailbox_account(
        &self,
        email: &str,
        provider_domain_id: Option<&str>,
        marker: &str,
        is_system: bool,
    ) -> Result<Option<String>, StalwartError> {
        if is_system {
            return self.find_account_by_email(email).await;
        }
        let (local, _) = email.split_once('@')
            .ok_or_else(|| StalwartError::InvalidAddress(email.to_string()))?;
        let domain_id = provider_domain_id.filter(|id| !id.is_empty())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "customer mailbox lookup".to_string(),
                message: "customer domain has no provider id".to_string(),
            })?;
        self.find_customer_account(domain_id, local, marker).await
    }

    pub async fn ensure_mailbox_with_quota(
        &self,
        email: &str,
        password: &str,
        quota_bytes: u64,
    ) -> Result<Option<String>, StalwartError> {
        if !self.enabled() {
            return Ok(None);
        }
        let (local, domain) = split_managed_email(email, &self.config.default_domain)?;
        let Some(domain_id) = self.domain_id(domain).await? else {
            return Err(StalwartError::Protocol {
                operation: "mailbox provisioning".to_string(),
                message: format!("configured domain {domain} does not exist"),
            });
        };

        if let Some(id) = self.find_account(local, &domain_id).await? {
            // Reconciliation is idempotent: if creation previously succeeded
            // but the API response was lost, a retry discovers the same
            // account and applies the current authoritative quota.
            self.set_account_quota(&id, quota_bytes).await?;
            return Ok(Some(id));
        }

        let result = self
            .management_write(
                "x:Account/set",
                json!({
                    "create": {
                        "new1": {
                            "@type": "User",
                            "name": local,
                            "domainId": domain_id,
                            "credentials": {
                                "0": { "@type": "Password", "secret": password }
                            },
                            "aliases": {},
                            "memberGroupIds": {},
                            "roles": { "@type": "User" },
                            "permissions": { "@type": "Inherit" },
                            "quotas": { "maxDiskQuota": quota_bytes },
                            "encryptionAtRest": { "@type": "Disabled" }
                        }
                    }
                }),
            )
            .await?;

        let new_id = result
            .get("created")
            .and_then(|created| created.get("new1"))
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str);
        if let Some(id) = new_id {
            tracing::info!(email = %email, account_id = %id, "mailbox created");
            return Ok(Some(id.to_string()));
        }

        // A concurrent worker/manual action may have created the account
        // after our initial read but before x:Account/set. Re-resolve before
        // treating notCreated as terminal so ensure remains idempotent.
        if let Some(id) = self.find_account(local, &domain_id).await? {
            self.set_account_quota(&id, quota_bytes).await?;
            return Ok(Some(id));
        }

        let detail = result
            .get("notCreated")
            .and_then(|not_created| not_created.get("new1"))
            .cloned()
            .unwrap_or(result);
        Err(StalwartError::Rejected {
            operation: "mailbox provisioning".to_string(),
            kind: detail
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("notCreated")
                .to_string(),
            description: detail
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("account was not created")
                .to_string(),
        })
    }

    /// Resolve a CS Mail-owned customer mailbox by its explicit provider
    /// domain id and ownership marker. Account local-parts are not globally
    /// unique on a multi-domain mail host, so name-only lookup is unsafe.
    pub async fn find_customer_account(
        &self,
        provider_domain_id: &str,
        local: &str,
        marker: &str,
    ) -> Result<Option<String>, StalwartError> {
        if !self.enabled() {
            return Ok(None);
        }
        self.require_owned_marker(marker)?;
        if provider_domain_id.trim().is_empty() || marker.trim().is_empty() || local.trim().is_empty() || local.contains('@') {
            return Err(StalwartError::InvalidAddress(local.to_string()));
        }
        let query = self
            .management_read("x:Account/query", json!({ "filter": { "name": local } }))
            .await?;
        let ids = query
            .get("ids")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(None);
        }
        let result = self.management_read("x:Account/get", json!({ "ids": ids })).await?;
        Ok(result
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| {
                item.get("domainId").and_then(Value::as_str) == Some(provider_domain_id)
                    && item.get("description").and_then(Value::as_str) == Some(marker)
            })
            .and_then(|item| item.get("id").and_then(Value::as_str))
            .map(str::to_string))
    }

    async fn customer_account_collision(
        &self,
        provider_domain_id: &str,
        local: &str,
        marker: &str,
    ) -> Result<bool, StalwartError> {
        let query = self
            .management_read("x:Account/query", json!({ "filter": { "name": local } }))
            .await?;
        let ids = query
            .get("ids")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        if ids.is_empty() { return Ok(false); }
        let result = self.management_read("x:Account/get", json!({ "ids": ids })).await?;
        Ok(result.get("list").and_then(Value::as_array).into_iter().flatten().any(|item| {
            item.get("domainId").and_then(Value::as_str) == Some(provider_domain_id)
                && item.get("description").and_then(Value::as_str) != Some(marker)
        }))
    }

    /// Provision an organization mailbox on an already ownership-verified
    /// Stalwart domain. The persisted marker prevents CS Mail from adopting a
    /// provider account created by another application on the shared server.
    pub async fn ensure_customer_mailbox_with_quota(
        &self,
        provider_domain_id: &str,
        local: &str,
        marker: &str,
        password: &str,
        quota_bytes: u64,
    ) -> Result<Option<String>, StalwartError> {
        if !self.enabled() { return Ok(None); }
        self.require_owned_marker(marker)?;
        if let Some(id) = self.find_customer_account(provider_domain_id, local, marker).await? {
            self.set_account_quota(&id, quota_bytes).await?;
            return Ok(Some(id));
        }
        if self.customer_account_collision(provider_domain_id, local, marker).await? {
            return Err(StalwartError::Rejected {
                operation: "customer mailbox provisioning".to_string(),
                kind: "addressExists".to_string(),
                description: "the provider address already exists but is not owned by this CS Mail mailbox".to_string(),
            });
        }
        let result = self.management_write(
            "x:Account/set",
            json!({
                "create": {
                    "new1": {
                        "@type": "User",
                        "name": local,
                        "domainId": provider_domain_id,
                        "description": marker,
                        "credentials": { "0": { "@type": "Password", "secret": password } },
                        "aliases": {},
                        "memberGroupIds": {},
                        "roles": { "@type": "User" },
                        "permissions": { "@type": "Inherit" },
                        "quotas": { "maxDiskQuota": quota_bytes },
                        "encryptionAtRest": { "@type": "Disabled" }
                    }
                }
            })
        ).await?;
        if let Some(id) = result.get("created").and_then(|v| v.get("new1")).and_then(|v| v.get("id")).and_then(Value::as_str) {
            return Ok(Some(id.to_string()));
        }
        if let Some(id) = self.find_customer_account(provider_domain_id, local, marker).await? {
            self.set_account_quota(&id, quota_bytes).await?;
            return Ok(Some(id));
        }
        let detail = result.get("notCreated").and_then(|v| v.get("new1")).unwrap_or(&result);
        Err(provider_rejection("customer mailbox provisioning", detail))
    }

    async fn business_mailing_list_binding(
        &self,
        id: &str,
        provider_domain_id: &str,
        marker: &str,
    ) -> Result<bool, StalwartError> {
        self.require_owned_marker(marker)?;
        let result = self
            .management_read("x:MailingList/get", json!({ "ids": [id] }))
            .await?;
        let Some(item) = result
            .get("list")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            return Ok(false);
        };
        if item.get("description").and_then(Value::as_str) != Some(marker)
            || item.get("domainId").and_then(Value::as_str) != Some(provider_domain_id)
        {
            return Err(StalwartError::Protocol {
                operation: "business address ownership check".to_string(),
                message: "stored provider mailing-list id is outside this CS Mail ownership binding".to_string(),
            });
        }
        Ok(true)
    }

    /// Create/update a tenant-owned alias or group as a provider MailingList.
    /// Explicit domain ids and ownership markers make this safe on the shared
    /// Stalwart instance used by other CrescentSphere applications.
    pub async fn ensure_business_mailing_list(
        &self,
        provider_domain_id: &str,
        existing_id: Option<&str>,
        marker: &str,
        local: &str,
        recipients: &[String],
        enabled: bool,
    ) -> Result<String, StalwartError> {
        if !self.enabled() { return Ok(existing_id.unwrap_or_default().to_string()); }
        self.require_owned_marker(marker)?;
        let recipients = if enabled {
            recipients.iter().map(|value| (value.clone(), Value::Bool(true))).collect::<serde_json::Map<String, Value>>()
        } else { serde_json::Map::new() };
        let known = if let Some(id) = existing_id.filter(|id| !id.trim().is_empty()) {
            if self.business_mailing_list_binding(id, provider_domain_id, marker).await? {
                Some(id.to_string())
            } else {
                self.find_managed_mailing_list(marker, local).await?
            }
        } else {
            self.find_managed_mailing_list(marker, local).await?
        };
        if let Some(id) = known.as_deref() {
            self.business_mailing_list_binding(id, provider_domain_id, marker).await?;
        }
        if let Some(id) = known {
            let mut update = serde_json::Map::new();
            update.insert(id.clone(), json!({
                "name": local, "domainId": provider_domain_id, "description": marker,
                "recipients": Value::Object(recipients)
            }));
            let result = self.management_write("x:MailingList/set", json!({"update": Value::Object(update)})).await?;
            if let Some(reason) = result.get("notUpdated").and_then(|v| v.get(&id)).filter(|v| !v.is_null()) {
                return Err(provider_rejection("business address update", reason));
            }
            return Ok(id);
        }
        let result = self.management_write("x:MailingList/set", json!({
            "create": {"new1": {
                "name": local, "domainId": provider_domain_id, "description": marker,
                "aliases": {}, "recipients": Value::Object(recipients)
            }}
        })).await?;
        if let Some(id) = result.get("created").and_then(|v| v.get("new1")).and_then(|v| v.get("id")).and_then(Value::as_str) {
            return Ok(id.to_string());
        }
        if let Some(id) = self.find_managed_mailing_list(marker, local).await? { return Ok(id); }
        let reason = result.get("notCreated").and_then(|v| v.get("new1")).unwrap_or(&result);
        Err(provider_rejection("business address create", reason))
    }

    pub async fn delete_business_mailing_list(
        &self,
        provider_domain_id: &str,
        existing_id: Option<&str>,
        marker: &str,
        local: &str,
    ) -> Result<(), StalwartError> {
        if !self.enabled() { return Ok(()); }
        self.require_owned_marker(marker)?;
        let id = if let Some(id) = existing_id.filter(|id| !id.trim().is_empty()) {
            if self.business_mailing_list_binding(id, provider_domain_id, marker).await? {
                Some(id.to_string())
            } else {
                self.find_managed_mailing_list(marker, local).await?
            }
        } else {
            self.find_managed_mailing_list(marker, local).await?
        };
        let Some(id) = id else { return Ok(()); };
        self.business_mailing_list_binding(&id, provider_domain_id, marker).await?;
        let result = self.management_write("x:MailingList/set", json!({"destroy":[id.clone()]})).await?;
        if let Some(reason) = result.get("notDestroyed").and_then(|value| value.get(&id)).filter(|value| !value.is_null()) {
            if reason.get("type").and_then(Value::as_str) == Some("notFound") { return Ok(()); }
            return Err(provider_rejection("business address delete", reason));
        }
        Ok(())
    }

    pub async fn account_quota(
        &self,
        account: &str,
    ) -> Result<Option<(u64, u64)>, StalwartError> {
        if !self.enabled() {
            return Ok(None);
        }
        let result = self
            .management_read("x:Account/get", json!({ "ids": [account] }))
            .await?;
        let Some(item) = result
            .get("list")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            return Ok(None);
        };
        Ok(Some(quota_pair(item)))
    }

    pub async fn account_quotas(
        &self,
        accounts: &[String],
    ) -> Result<HashMap<String, (u64, u64)>, StalwartError> {
        let mut out = HashMap::new();
        if !self.enabled() || accounts.is_empty() {
            return Ok(out);
        }
        let result = self
            .management_read("x:Account/get", json!({ "ids": accounts }))
            .await?;
        for item in result
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                out.insert(id.to_string(), quota_pair(item));
            }
        }
        Ok(out)
    }

    /// Replace the account primary password through the management API.
    /// This is used only after the application password change has been
    /// committed to the durable provisioning queue.
    pub async fn set_account_password(
        &self,
        account: &str,
        password: &str,
    ) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }

        // Resolve the actual Password credential rather than assuming it is
        // always key/index 0. Existing or imported accounts may also contain
        // OTP/application credentials. Updating only the Password secret keeps
        // every secondary credential intact.
        let current = self
            .management_read("x:Account/get", json!({ "ids": [account] }))
            .await?;
        let item = current
            .get("list")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| StalwartError::Protocol {
                operation: "credential update".to_string(),
                message: "account was not returned by x:Account/get".to_string(),
            })?;
        let credentials = item.get("credentials").ok_or_else(|| StalwartError::Protocol {
            operation: "credential update".to_string(),
            message: "account has no credential collection".to_string(),
        })?;
        let password_key = if let Some(map) = credentials.as_object() {
            map.iter()
                .find(|(_, credential)| {
                    credential.get("@type").and_then(Value::as_str) == Some("Password")
                })
                .map(|(key, _)| key.clone())
        } else if let Some(list) = credentials.as_array() {
            list.iter()
                .position(|credential| {
                    credential.get("@type").and_then(Value::as_str) == Some("Password")
                })
                .map(|index| index.to_string())
        } else {
            None
        }
        .ok_or_else(|| StalwartError::Protocol {
            operation: "credential update".to_string(),
            message: "account has no primary Password credential".to_string(),
        })?;

        let mut update = serde_json::Map::new();
        let patch_path = format!("credentials/{password_key}/secret");
        let mut patch = serde_json::Map::new();
        patch.insert(patch_path, json!(password));
        update.insert(account.to_string(), Value::Object(patch));
        let result = self
            .management_write("x:Account/set", json!({ "update": update }))
            .await?;
        if let Some(reason) = result
            .get("notUpdated")
            .and_then(|not_updated| not_updated.get(account))
            .filter(|value| !value.is_null())
        {
            return Err(StalwartError::Rejected {
                operation: "credential update".to_string(),
                kind: reason
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("notUpdated")
                    .to_string(),
                description: reason
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("credential was not updated")
                    .to_string(),
            });
        }
        Ok(())
    }

    pub async fn set_account_quota(
        &self,
        account: &str,
        bytes: u64,
    ) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }
        let mut update = serde_json::Map::new();
        update.insert(account.to_string(), json!({ "quotas/maxDiskQuota": bytes }));
        let result = self
            .management_write("x:Account/set", json!({ "update": update }))
            .await?;
        if let Some(reason) = result
            .get("notUpdated")
            .and_then(|not_updated| not_updated.get(account))
            .filter(|value| !value.is_null())
        {
            return Err(StalwartError::Rejected {
                operation: "quota update".to_string(),
                kind: reason
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("notUpdated")
                    .to_string(),
                description: reason
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("quota was not updated")
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Suspend or reactivate a CS Mail mailbox at the provider boundary.
    /// Stalwart Account does not expose an `isEnabled` flag, so suspension is
    /// represented as an explicit empty permission replacement. Reactivation
    /// restores the normal inherited user permission set. Authentication may
    /// still identify the principal, but mail protocols/actions have no
    /// effective permissions while suspended.
    pub async fn set_account_suspended(
        &self,
        account: &str,
        suspended: bool,
    ) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }
        let permissions = if suspended {
            json!({
                "@type": "Replace",
                "enabledPermissions": [],
                "disabledPermissions": []
            })
        } else {
            json!({ "@type": "Inherit" })
        };
        let mut update = serde_json::Map::new();
        update.insert(account.to_string(), json!({ "permissions": permissions }));
        let result = self
            .management_write("x:Account/set", json!({ "update": update }))
            .await?;
        if let Some(reason) = result
            .get("notUpdated")
            .and_then(|not_updated| not_updated.get(account))
            .filter(|value| !value.is_null())
        {
            return Err(StalwartError::Rejected {
                operation: "account access update".to_string(),
                kind: reason
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("notUpdated")
                    .to_string(),
                description: reason
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("account access was not updated")
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Delete a tenant mailbox only after re-reading the provider account and
    /// proving that id, domain, local-part and namespace marker still match.
    pub async fn destroy_customer_account(
        &self,
        account: &str,
        provider_domain_id: &str,
        local: &str,
        marker: &str,
    ) -> Result<(), StalwartError> {
        if !self.enabled() { return Ok(()); }
        self.require_owned_marker(marker)?;
        let current = self.management_read("x:Account/get", json!({"ids":[account]})).await?;
        let Some(item) = current.get("list").and_then(Value::as_array).and_then(|items| items.first()) else {
            return Ok(());
        };
        if item.get("domainId").and_then(Value::as_str) != Some(provider_domain_id)
            || item.get("name").and_then(Value::as_str) != Some(local)
            || item.get("description").and_then(Value::as_str) != Some(marker)
        {
            return Err(StalwartError::Protocol {
                operation: "customer account deletion ownership check".to_string(),
                message: "stored provider account id is outside this CS Mail ownership binding".to_string(),
            });
        }
        self.destroy_account(account).await
    }

    pub async fn destroy_account(&self, account: &str) -> Result<(), StalwartError> {
        if !self.enabled() {
            return Ok(());
        }
        let result = self
            .management_write("x:Account/set", json!({ "destroy": [account] }))
            .await?;
        let destroyed = result
            .get("destroyed")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|value| value.as_str() == Some(account)));
        if destroyed {
            return Ok(());
        }
        if let Some(reason) = result
            .get("notDestroyed")
            .and_then(|not_destroyed| not_destroyed.get(account))
        {
            if reason.get("type").and_then(Value::as_str) == Some("notFound") {
                return Ok(());
            }
            return Err(StalwartError::Rejected {
                operation: "account deletion".to_string(),
                kind: reason
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("notDestroyed")
                    .to_string(),
                description: reason
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("account was not deleted")
                    .to_string(),
            });
        }
        Err(StalwartError::Protocol {
            operation: "account deletion".to_string(),
            message: "unexpected response".to_string(),
        })
    }

    /// SMTP is part of the provider boundary as well. We intentionally do not
    /// auto-retry a DATA submission because an ambiguous network failure after
    /// the server accepted the message could otherwise duplicate delivery.
    pub async fn submit_raw(
        &self,
        from: &str,
        to: &[String],
        message: &[u8],
    ) -> Result<String, StalwartError> {
        smtp::send(&self.config.smtp, from, to, message)
            .await
            .map_err(StalwartError::Submission)
    }
}

const MANAGED_ALIAS_PREFIX: &str = "CS Mail managed alias:";
const LEGACY_MANAGED_ALIAS_PREFIX: &str = "CS Mailer managed alias:";

fn provider_alias_is_managed(value: &Value) -> bool {
    value
        .get("description")
        .and_then(Value::as_str)
        .is_some_and(|description| description.starts_with(MANAGED_ALIAS_PREFIX) || description.starts_with(LEGACY_MANAGED_ALIAS_PREFIX))
}

fn provider_rejection(operation: &str, value: &Value) -> StalwartError {
    StalwartError::Rejected {
        operation: operation.to_string(),
        kind: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("rejected")
            .to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("provider rejected the operation")
            .to_string(),
    }
}

fn split_managed_email<'a>(
    email: &'a str,
    default_domain: &str,
) -> Result<(&'a str, &'a str), StalwartError> {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(StalwartError::InvalidAddress(email.to_string()));
    };
    if local.is_empty() || domain.is_empty() || local.contains('@') || domain.contains('@') {
        return Err(StalwartError::InvalidAddress(email.to_string()));
    }
    if !domain.eq_ignore_ascii_case(default_domain) {
        return Err(StalwartError::UnsupportedDomain(domain.to_string()));
    }
    Ok((local, domain))
}

fn quota_pair(item: &Value) -> (u64, u64) {
    let used = item
        .get("usedDiskQuota")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = item
        .get("quotas")
        .and_then(|quota| quota.get("maxDiskQuota"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    (used, total)
}

fn truncate(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;


    fn base_config() -> StalwartConfig {
        StalwartConfig {
            admin_url: String::new(),
            admin_username: String::new(),
            admin_secret: String::new(),
            admin_bearer_token: None,
            mail_jmap_username: String::new(),
            mail_jmap_secret: String::new(),
            default_domain: "example.com".into(),
            ownership_namespace: "cs-mail".into(),
            request_timeout: Duration::from_secs(1),
            read_retries: 0,
            retry_base_delay: Duration::from_millis(1),
            smtp: SmtpConfig::default(),
        }
    }

    #[test]
    fn disabled_provider_does_not_require_credentials() {
        assert!(StalwartService::new(base_config()).is_ok());
    }

    #[test]
    fn enabled_provider_requires_mail_jmap_credential() {
        let mut config = base_config();
        config.admin_url = "http://mail:8080".into();
        config.admin_bearer_token = Some("management-token".into());
        let err = match StalwartService::new(config) {
            Ok(_) => panic!("missing JMAP credential should be rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, StalwartError::Configuration(_)));
    }

    #[test]
    fn managed_email_rejects_foreign_domain() {
        let err = split_managed_email("alice@example.net", "example.com").unwrap_err();
        assert!(matches!(err, StalwartError::UnsupportedDomain(_)));
    }

    #[test]
    fn managed_email_accepts_domain_case_insensitively() {
        let parsed = split_managed_email("alice@EXAMPLE.COM", "example.com").unwrap();
        assert_eq!(parsed.0, "alice");
    }

    #[test]
    fn public_error_messages_do_not_echo_provider_body() {
        let err = StalwartError::Http {
            operation: "x:Account/get".into(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            body: "sensitive provider response".into(),
        };
        assert_eq!(err.public_message(), "Mail service is temporarily unavailable");
    }
}
