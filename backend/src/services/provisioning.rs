use serde_json::{json, Value};

/// Bridge to the Stalwart management API. Every signup on the primary domain
/// gets a real mailbox here (IMAP/SMTP/JMAP), so the account can log in or
/// receive mail immediately. All calls go over `/jmap` with Basic auth using
/// the `admin` account pinned at bootstrap (`STALWART_RECOVERY_ADMIN`).
#[derive(Clone)]
pub struct MailBridge {
    client: reqwest::Client,
    url: String,
    username: String,
    secret: String,
    pub default_domain: String,
    quota_bytes: u64,
}

impl MailBridge {
    pub fn new(
        url: String,
        username: String,
        secret: String,
        default_domain: String,
        quota_bytes: u64,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client build cannot fail");
        Self {
            client,
            url,
            username,
            secret,
            default_domain,
            quota_bytes,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty()
    }

    /// Management namespace (`urn:stalwart:jmap`) for the service's own admin
    /// API. Public so `services/imap.rs` can reuse it for per-account mailbox
    /// operations via the impersonating admin session.
    pub async fn jmap(&self, method: &str, args: Value) -> Result<Value, String> {
        self.jmap_using(
            method,
            args,
            &["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
        )
        .await
    }

    /// Standard JMAP mail namespace (`urn:ietf:params:jmap:mail`): Mailbox,
    /// Email, Thread, Blob methods. The pinned admin account targets whatever
    /// `accountId` is embedded in `args` (impersonation). The blob capability
    /// rides along because `Blob/get` (attachments) depends on it.
    pub async fn jmap_mail(&self, method: &str, args: Value) -> Result<Value, String> {
        self.jmap_using(
            method,
            args,
            &[
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
                "urn:ietf:params:jmap:blob",
            ],
        )
        .await
    }

    async fn jmap_using(&self, method: &str, args: Value, using: &[&str]) -> Result<Value, String> {
        let body = json!({
            "methodCalls": [[method, args, "c1"]],
            "using": using
        });
        let url = format!("{}/jmap", self.url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.secret))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("mail JMAP {method}: transport error: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("mail JMAP {method}: read error: {e}"))?;
        if !status.is_success() {
            return Err(format!("mail JMAP {method}: HTTP {status}: {text}"));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| format!("mail JMAP {method}: bad JSON: {e}: {text}"))?;
        parsed
            .get("methodResponses")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_array)
            .and_then(|m| m.get(1))
            .cloned()
            .ok_or_else(|| format!("mail JMAP {method}: no methodResponses: {text}"))
    }

    async fn domain_id(&self, domain: &str) -> Result<Option<String>, String> {
        let result = self
            .jmap("x:Domain/query", json!({ "filter": { "name": domain } }))
            .await?;
        let id = result
            .get("ids")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(id)
    }

    pub async fn find_account(&self, local: &str) -> Result<Option<String>, String> {
        let result = self
            .jmap("x:Account/query", json!({ "filter": { "name": local } }))
            .await?;
        let id = result
            .get("ids")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(id)
    }

    /// Create (or keep up to date) the mailbox for `email`, returning the
    /// Stalwart account id (used by the WS2 read path via a single admin
    /// session). Only accounts on the primary domain are provisioned;
    /// foreign-domain signups yield `None` until Harbor supports additional
    /// domains.
    pub async fn ensure_mailbox(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<String>, String> {
        if !self.enabled() {
            return Ok(None);
        }
        let Some((local, domain)) = email.split_once('@') else {
            return Err(format!(
                "cannot provision mailbox for invalid address {email}"
            ));
        };
        if !domain.eq_ignore_ascii_case(&self.default_domain) {
            tracing::debug!("skipping mailbox for foreign domain {domain}");
            return Ok(None);
        }

        let Some(domain_id) = self.domain_id(&self.default_domain).await? else {
            return Err(format!(
                "Stalwart domain {} not found — run deploy/stalwart-bootstrap.ps1 once",
                self.default_domain
            ));
        };

        if let Some(id) = self.find_account(local).await? {
            return Ok(Some(id));
        }

        let args = json!({
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
                    "quotas": { "maxDiskQuota": self.quota_bytes },
                    "encryptionAtRest": { "@type": "Disabled" }
                }
            }
        });
        let result = self.jmap("x:Account/set", args).await?;
        let new_id = result
            .get("created")
            .and_then(|c| c.get("new1"))
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str);
        if let Some(id) = new_id {
            tracing::info!(email = %email, "mailbox created");
            Ok(Some(id.to_string()))
        } else {
            Err(format!(
                "mailbox create rejected: {}",
                result
                    .get("notCreated")
                    .and_then(|n| n.get("new1"))
                    .map_or_else(|| result.to_string(), Value::to_string)
            ))
        }
    }
}
