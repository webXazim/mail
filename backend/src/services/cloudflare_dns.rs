//! One-time Cloudflare DNS write for a customer's ownership challenge.
//! The customer token is held only for this request and is never persisted.

use std::time::Duration;

use axum::http::StatusCode;
use reqwest::redirect::Policy;
use serde_json::{json, Value};

use crate::error::ApiError;

fn cloudflare_error(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "cloudflare_dns", message)
}

fn zone_candidates(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    (0..labels.len().saturating_sub(1))
        .map(|start| labels[start..].join("."))
        .collect()
}

async fn response_json(response: reqwest::Response) -> Result<Value, ApiError> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(cloudflare_error("Cloudflare rejected the token. Use a token scoped to this zone with Zone Read and DNS Write permissions."));
    }
    if !response.status().is_success() {
        return Err(cloudflare_error("Cloudflare could not complete the DNS request. Check the token permissions and try again."));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| cloudflare_error("Cloudflare returned an unreadable DNS response."))?;
    if payload["success"] != true {
        return Err(cloudflare_error(
            "Cloudflare rejected the DNS request. Check the zone and token permissions.",
        ));
    }
    Ok(payload)
}

/// Publish only the exact TXT challenge for an already claimed business domain.
/// Existing unrelated DNS records are never changed or removed.
pub async fn publish_verification_txt(
    domain: &str,
    name: &str,
    value: &str,
    token: &str,
) -> Result<String, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(Policy::none())
        .build()
        .map_err(|_| cloudflare_error("Cloudflare DNS connection could not be prepared."))?;
    let mut zone: Option<(String, String)> = None;
    for candidate in zone_candidates(domain) {
        let response = client
            .get("https://api.cloudflare.com/client/v4/zones")
            .bearer_auth(token)
            .query(&[("name", candidate.as_str()), ("per_page", "5")])
            .send()
            .await
            .map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
        let payload = response_json(response).await?;
        if let Some(found) = payload["result"].as_array().and_then(|rows| {
            rows.iter()
                .find(|row| row["name"].as_str() == Some(candidate.as_str()))
        }) {
            let id = found["id"].as_str().unwrap_or_default();
            if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(cloudflare_error(
                    "Cloudflare returned an invalid zone identifier.",
                ));
            }
            if found["status"] != "active" {
                return Err(cloudflare_error("The Cloudflare zone is not active. Finish nameserver setup before automatic verification."));
            }
            zone = Some((id.to_string(), candidate));
            break;
        }
    }
    let (zone_id, zone_name) = zone.ok_or_else(|| cloudflare_error("This domain was not found in the token's Cloudflare zones. Use a token scoped to its active DNS zone."))?;
    let records_url = format!("https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records");
    let response = client
        .get(&records_url)
        .bearer_auth(token)
        .query(&[("type", "TXT"), ("name", name), ("per_page", "100")])
        .send()
        .await
        .map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
    let existing = response_json(response).await?;
    if existing["result"].as_array().is_some_and(|records| {
        records.iter().any(|record| {
            record["type"] == "TXT"
                && record["name"].as_str() == Some(name)
                && record["content"].as_str() == Some(value)
        })
    }) {
        return Ok(zone_name);
    }
    let response = client.post(&records_url)
        .bearer_auth(token)
        .json(&json!({"type":"TXT", "name":name, "content":value, "ttl":120, "comment":"CS Mail domain ownership verification"}))
        .send().await.map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
    response_json(response).await?;
    Ok(zone_name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn zone_lookup_uses_only_parent_suffixes() {
        assert_eq!(
            super::zone_candidates("mail.example.com"),
            vec!["mail.example.com", "example.com"]
        );
        assert_eq!(super::zone_candidates("example.com"), vec!["example.com"]);
    }
}
