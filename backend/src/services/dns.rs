use reqwest::header::ACCEPT;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("DNS resolver request failed: {0}")]
    Transport(String),
    #[error("DNS resolver returned an invalid response: {0}")]
    Response(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MxRecord {
    pub priority: u16,
    pub exchange: String,
}

#[derive(Debug, Deserialize)]
struct DnsJsonAnswer {
    #[serde(default)]
    data: String,
}

#[derive(Debug, Deserialize)]
struct DnsJsonResponse {
    #[serde(rename = "Status", default)]
    status: i32,
    #[serde(rename = "Answer", default)]
    answer: Vec<DnsJsonAnswer>,
}

fn normalize_txt(value: &str) -> String {
    let value = value.trim();
    if !value.contains('"') {
        return value.to_string();
    }
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' { continue; }
        let mut escaped = false;
        while let Some(next) = chars.next() {
            if escaped { out.push(next); escaped = false; }
            else if next == '\\' { escaped = true; }
            else if next == '"' { break; }
            else { out.push(next); }
        }
    }
    if out.is_empty() { value.trim_matches('"').to_string() } else { out }
}

fn parse_mx(value: &str) -> Option<MxRecord> {
    let mut parts = value.split_whitespace();
    let priority = parts.next()?.parse::<u16>().ok()?;
    let exchange = parts.next()?.trim_end_matches('.').to_ascii_lowercase();
    (!exchange.is_empty()).then_some(MxRecord { priority, exchange })
}

async fn query_raw(client: &reqwest::Client, base: &str, name: &str, record_type: &str) -> Result<Vec<String>, DnsError> {
    let response = client
        .get(base)
        .query(&[("name", name), ("type", record_type)])
        .header(ACCEPT, "application/dns-json")
        .send().await.map_err(|e| DnsError::Transport(e.to_string()))?;
    if !response.status().is_success() {
        return Err(DnsError::Response(format!("HTTP {}", response.status())));
    }
    let payload: DnsJsonResponse = response.json().await.map_err(|e| DnsError::Response(e.to_string()))?;
    if payload.status == 3 { return Ok(Vec::new()); }
    if payload.status != 0 { return Err(DnsError::Response(format!("DNS status {}", payload.status))); }
    Ok(payload.answer.into_iter().map(|a| a.data).filter(|v| !v.is_empty()).collect())
}

fn client() -> Result<reqwest::Client, DnsError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("CS-Mail-DNS-Readiness/1.0")
        .build().map_err(|e| DnsError::Transport(e.to_string()))
}

async fn resilient_query(name: &str, record_type: &str) -> Result<Vec<String>, DnsError> {
    let client = client()?;
    let cloudflare = query_raw(&client, "https://cloudflare-dns.com/dns-query", name, record_type).await;
    match cloudflare {
        Ok(values) if !values.is_empty() => Ok(values),
        Ok(_) => query_raw(&client, "https://dns.google/resolve", name, record_type).await,
        Err(first) => match query_raw(&client, "https://dns.google/resolve", name, record_type).await {
            Ok(values) => Ok(values),
            Err(second) => Err(DnsError::Transport(format!("{first}; fallback: {second}"))),
        },
    }
}

pub async fn txt_records(name: &str) -> Result<Vec<String>, DnsError> {
    Ok(resilient_query(name, "TXT").await?.into_iter().map(|v| normalize_txt(&v)).filter(|v| !v.is_empty()).collect())
}

pub async fn mx_records(name: &str) -> Result<Vec<MxRecord>, DnsError> {
    Ok(resilient_query(name, "MX").await?.into_iter().filter_map(|v| parse_mx(&v)).collect())
}

#[cfg(test)]
mod tests {
    use super::{normalize_txt, parse_mx};
    #[test]
    fn joins_split_txt_chunks() {
        assert_eq!(normalize_txt("\"cs-mail-verification=abc\" \"123\""), "cs-mail-verification=abc123");
    }
    #[test]
    fn parses_mx_record() {
        let mx = parse_mx("10 mx.example.com.").unwrap();
        assert_eq!(mx.priority, 10);
        assert_eq!(mx.exchange, "mx.example.com");
    }
}
