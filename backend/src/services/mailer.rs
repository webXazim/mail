use std::time::Duration;

use anyhow::{bail, Result};
use axum::http::StatusCode;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;

use crate::error::ApiError;
use crate::services::mime::Outgoing;

/// Optional transport for platform-generated mail. Customer mailbox mail still
/// uses Stalwart and keeps its own sender identity and mailbox lifecycle.
#[derive(Clone)]
pub struct MailerClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
}

impl MailerClient {
    pub fn from_env() -> Result<Option<Self>> {
        let endpoint = std::env::var("CS_MAILER_API_URL").unwrap_or_default();
        let api_key = std::env::var("CS_MAILER_API_KEY").unwrap_or_default();
        if endpoint.trim().is_empty() && api_key.trim().is_empty() {
            return Ok(None);
        }
        let url = reqwest::Url::parse(endpoint.trim())?;
        if url.scheme() != "https" || url.path() != "/api/v1/emails" || url.host_str().is_none() {
            bail!("CS_MAILER_API_URL must be an HTTPS /api/v1/emails endpoint");
        }
        if !api_key.trim().starts_with("cs_live_") {
            bail!("CS_MAILER_API_KEY must be a production sending key");
        }
        let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
        Ok(Some(Self { client, endpoint: endpoint.trim().into(), api_key: api_key.trim().into() }))
    }

    pub async fn send(&self, message: &Outgoing) -> Result<(), ApiError> {
        if !message.from.email.to_ascii_lowercase().ends_with("@crescentsphere.com") {
            return Err(ApiError::internal("Application sender must use crescentsphere.com"));
        }
        let attachments = message.attachments.iter().map(|item| json!({
            "filename": item.filename,
            "content": STANDARD.encode(&item.bytes),
            "content_type": item.content_type,
        })).collect::<Vec<_>>();
        let sender = message.from.name.as_deref()
            .map(|name| name.chars().filter(|ch| !matches!(ch, '\r' | '\n' | '<' | '>')).collect::<String>())
            .filter(|name| !name.trim().is_empty())
            .map(|name| format!("{} <{}>", name.trim(), message.from.email))
            .unwrap_or_else(|| message.from.email.clone());
        let payload = json!({
            "from": sender,
            "to": message.to.iter().map(|item| item.email.as_str()).collect::<Vec<_>>(),
            "cc": message.cc.iter().map(|item| item.email.as_str()).collect::<Vec<_>>(),
            "subject": message.subject,
            "text": message.body_text,
            "html": message.body_html,
            "reply_to": message.reply_to.as_ref().map(|item| item.email.as_str()),
            "attachments": attachments,
            "environment": "production",
        });
        let response = self.client.post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Idempotency-Key", format!("cs-mail-{}", message.message_id_local))
            .json(&payload).send().await
            .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "mail_submission", "Mailer API unavailable"))?;
        if !response.status().is_success() {
            tracing::warn!(status=%response.status(), "Mailer API rejected CS Mail application email");
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, "mail_submission", format!("Mailer API returned HTTP {}", response.status())));
        }
        let receipt: serde_json::Value = response.json().await
            .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "mail_submission", "Mailer API returned an invalid receipt"))?;
        if receipt["data"]["status"] != "queued" || receipt["data"]["id"].as_str().is_none() {
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, "mail_submission", "Mailer API did not queue the email"));
        }
        Ok(())
    }
}
