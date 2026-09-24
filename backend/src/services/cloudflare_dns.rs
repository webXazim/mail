//! One-time Cloudflare DNS write for a customer's ownership challenge.
//! The customer token is held only for this request and is never persisted.

use std::time::Duration;

use axum::http::StatusCode;
use reqwest::redirect::Policy;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::services::domain_onboarding::{txt_equivalent, ExpectedRecord};

fn cloudflare_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "cloudflare_dns", message)
}

fn zone_candidates(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    (0..labels.len().saturating_sub(1))
        .map(|start| labels[start..].join("."))
        .collect()
}

fn validate_mail_records(domain: &str, records: &[ExpectedRecord]) -> Result<(), ApiError> {
    let mut mx = false;
    let mut spf = false;
    let mut dkim = false;
    let mut dmarc = false;
    for record in records {
        let name = record.name.trim_end_matches('.').to_ascii_lowercase();
        if name != domain && !name.ends_with(&format!(".{domain}")) {
            return Err(cloudflare_error("Mail provider returned a DNS record outside this domain."));
        }
        match record.kind.as_str() {
            "MX" if name == domain && record.priority.is_some() => mx = true,
            "TXT" if name == domain && record.value.to_ascii_lowercase().starts_with("v=spf1 ") => spf = true,
            "TXT" if name.starts_with("_dmarc.") && record.value.to_ascii_lowercase().starts_with("v=dmarc1;") => dmarc = true,
            "TXT" if name.contains("._domainkey.") && record.value.to_ascii_lowercase().starts_with("v=dkim1;") => dkim = true,
            _ => return Err(cloudflare_error("Mail provider returned an unsupported DNS record.")),
        }
    }
    if !(mx && spf && dkim && dmarc) {
        return Err(cloudflare_error("Mail provider has not generated a complete MX, SPF, DKIM and DMARC setup."));
    }
    Ok(())
}

fn exact_record(existing: &Value, expected: &ExpectedRecord) -> bool {
    existing["type"] == expected.kind
        && existing["name"].as_str() == Some(expected.name.as_str())
        && existing["content"].as_str().is_some_and(|value| {
            if expected.kind == "TXT" {
                txt_equivalent(&expected.value, value)
            } else {
                value.trim_end_matches('.').eq_ignore_ascii_case(expected.value.trim_end_matches('.'))
            }
        })
        && (expected.kind != "MX" || existing["priority"].as_u64() == expected.priority.map(u64::from))
}

fn conflicting_record(existing: &Value, expected: &ExpectedRecord) -> bool {
    if existing["name"].as_str() != Some(expected.name.as_str()) { return false; }
    if exact_record(existing, expected) { return false; }
    if expected.kind == "MX" { return existing["type"] == "MX"; }
    let content = existing["content"].as_str().unwrap_or_default().to_ascii_lowercase();
    if expected.name.starts_with("_dmarc.") { return content.starts_with("v=dmarc1"); }
    if expected.name.contains("._domainkey.") { return content.starts_with("v=dkim1"); }
    content.starts_with("v=spf1")
}

async fn zone_for(client: &reqwest::Client, domain: &str, token: &str) -> Result<(String, String), ApiError> {
    for candidate in zone_candidates(domain) {
        let response = client.get("https://api.cloudflare.com/client/v4/zones")
            .bearer_auth(token)
            .query(&[("name", candidate.as_str()), ("per_page", "5")])
            .send().await.map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
        let payload = response_json(response).await?;
        if let Some(found) = payload["result"].as_array().and_then(|rows| rows.iter().find(|row| row["name"].as_str() == Some(candidate.as_str()))) {
            let id = found["id"].as_str().unwrap_or_default();
            if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(cloudflare_error("Cloudflare returned an invalid zone identifier."));
            }
            if found["status"] != "active" {
                return Err(cloudflare_error("The Cloudflare zone is not active. Finish nameserver setup before automatic verification."));
            }
            return Ok((id.to_string(), candidate));
        }
    }
    Err(cloudflare_error("This domain was not found in the token's Cloudflare zones. Use a token scoped to its active DNS zone."))
}

async fn named_records(client: &reqwest::Client, zone_id: &str, kind: &str, name: &str, token: &str) -> Result<Vec<Value>, ApiError> {
    let url = format!("https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records");
    let response = client.get(url).bearer_auth(token)
        .query(&[("type", kind), ("name", name), ("per_page", "100")])
        .send().await.map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
    let payload = response_json(response).await?;
    if payload["result_info"]["total_pages"].as_u64().is_some_and(|pages| pages > 1) {
        return Err(cloudflare_error("Too many DNS records have this name. Review them in Cloudflare before continuing."));
    }
    payload["result"].as_array().cloned().ok_or_else(|| cloudflare_error("Cloudflare returned an unreadable DNS record list."))
}

async fn create_record(client: &reqwest::Client, zone_id: &str, record: &ExpectedRecord, token: &str) -> Result<(), ApiError> {
    let url = format!("https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records");
    let mut payload = json!({"type": record.kind, "name": record.name, "content": record.value, "ttl": 120, "comment": "CS Mail business domain setup"});
    if let Some(priority) = record.priority { payload["priority"] = json!(priority); }
    let response = client.post(url).bearer_auth(token).json(&payload)
        .send().await.map_err(|_| cloudflare_error("Cloudflare DNS is temporarily unreachable."))?;
    response_json(response).await?;
    Ok(())
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
    let (zone_id, zone_name) = zone_for(&client, domain, token).await?;
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

/// Add provider-generated mail records only when they do not conflict with
/// existing mail routing. The preflight runs before any write; retries are safe.
pub async fn publish_mail_records(domain: &str, records: &[ExpectedRecord], token: &str) -> Result<(String, usize), ApiError> {
    validate_mail_records(domain, records)?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(12))
        .redirect(Policy::none()).build()
        .map_err(|_| cloudflare_error("Cloudflare DNS connection could not be prepared."))?;
    let (zone_id, zone_name) = zone_for(&client, domain, token).await?;
    let mut missing = Vec::new();
    for record in records {
        let existing = named_records(&client, &zone_id, &record.kind, &record.name, token).await?;
        if existing.iter().any(|item| conflicting_record(item, record)) {
            return Err(cloudflare_error(format!(
                "Existing {} record at {} differs from the mail provider's required value. Review this record in Cloudflare; no records were replaced.",
                record.kind, record.name
            )));
        }
        if !existing.iter().any(|item| exact_record(item, record)) {
            missing.push(record);
        }
    }
    for record in &missing {
        create_record(&client, &zone_id, record, token).await?;
    }
    Ok((zone_name, missing.len()))
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

    #[test]
    fn mail_records_require_complete_provider_zone() {
        use crate::services::domain_onboarding::ExpectedRecord;
        let records = vec![ExpectedRecord { kind: "MX".into(), name: "example.com".into(), value: "mx.example.net".into(), priority: Some(10) }];
        assert!(super::validate_mail_records("example.com", &records).is_err());
    }

    #[test]
    fn existing_mail_routes_are_not_silently_overwritten() {
        use crate::services::domain_onboarding::ExpectedRecord;
        let mx = ExpectedRecord { kind: "MX".into(), name: "example.com".into(), value: "mx.cs.example".into(), priority: Some(10) };
        assert!(super::conflicting_record(&serde_json::json!({"type":"MX","name":"example.com","content":"mx.other.example","priority":10}), &mx));
        assert!(!super::conflicting_record(&serde_json::json!({"type":"MX","name":"example.com","content":"mx.cs.example","priority":10}), &mx));
        let spf = ExpectedRecord { kind: "TXT".into(), name: "example.com".into(), value: "v=spf1 mx -all".into(), priority: None };
        assert!(super::conflicting_record(&serde_json::json!({"type":"TXT","name":"example.com","content":"v=spf1 include:other.example -all"}), &spf));
        assert!(!super::conflicting_record(&serde_json::json!({"type":"TXT","name":"example.com","content":"google-site-verification=abc"}), &spf));
    }

    #[test]
    fn existing_mailer_dkim_with_equivalent_format_is_reused() {
        use crate::services::domain_onboarding::ExpectedRecord;
        let dkim = ExpectedRecord {
            kind: "TXT".into(), name: "cs1._domainkey.example.com".into(),
            value: "v=DKIM1; k=rsa; h=sha256; p=AbC123".into(), priority: None,
        };
        let equivalent = serde_json::json!({"type":"TXT","name":"cs1._domainkey.example.com","content":"v=DKIM1;p=AbC 123; k=RSA"});
        assert!(super::exact_record(&equivalent, &dkim));
        assert!(!super::conflicting_record(&equivalent, &dkim));
        let different_key = serde_json::json!({"type":"TXT","name":"cs1._domainkey.example.com","content":"v=DKIM1; k=rsa; p=AbC124"});
        assert!(!super::exact_record(&different_key, &dkim));
        assert!(super::conflicting_record(&different_key, &dkim));
        let changed_case = serde_json::json!({"type":"TXT","name":"cs1._domainkey.example.com","content":"v=DKIM1; k=rsa; p=aBc123"});
        assert!(super::conflicting_record(&changed_case, &dkim));
        let incompatible_hash = serde_json::json!({"type":"TXT","name":"cs1._domainkey.example.com","content":"v=DKIM1; k=rsa; h=sha1; p=AbC123"});
        assert!(super::conflicting_record(&incompatible_hash, &dkim));
    }
}
