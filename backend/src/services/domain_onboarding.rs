use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

use crate::audit;
use crate::services::dns;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedRecord {
    pub kind: String,
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DnsReadiness {
    pub mx: bool,
    pub spf: bool,
    pub dkim: bool,
    pub dmarc: bool,
    pub ready: bool,
    pub expected: Vec<ExpectedRecord>,
    pub observed: Value,
}

#[derive(Debug, FromRow)]
struct DueDomain {
    id: Uuid,
    organization_id: Uuid,
    domain: String,
    provider_domain_id: String,
}

fn clean_name(value: &str) -> String {
    value.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn expand_owner(value: &str, apex: &str) -> String {
    let absolute = value.trim().ends_with('.');
    let cleaned = clean_name(value);
    if cleaned == "@" { return apex.to_string(); }
    if absolute || cleaned.ends_with(apex) || cleaned.contains('.') { cleaned } else { format!("{cleaned}.{apex}") }
}

fn clean_txt(value: &str) -> String {
    let value = value.trim();
    if !value.contains('"') {
        return value.split_whitespace().collect::<Vec<_>>().join(" ");
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

fn strip_zone_comment(line: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped { out.push(ch); escaped = false; continue; }
        if ch == '\\' && quoted { out.push(ch); escaped = true; continue; }
        if ch == '"' { quoted = !quoted; out.push(ch); continue; }
        if ch == ';' && !quoted { break; }
        out.push(ch);
    }
    out.trim().to_string()
}

/// Parse Stalwart's server-generated zone text into the four records CS Mail
/// requires before public business mail becomes active. Stalwart's exported
/// zone may use either `owner IN TYPE value` or `TYPE owner value`; both are
/// accepted and quoted TXT semicolons are preserved.
pub fn parse_required_records(zone: &str, domain: &str) -> Vec<ExpectedRecord> {
    let mut records = Vec::new();
    let apex = clean_name(domain);
    let mut logical = String::new();
    let mut parens = 0i32;
    let mut lines = Vec::new();
    for raw in zone.lines() {
        let stripped = strip_zone_comment(raw);
        if stripped.is_empty() || stripped.starts_with('$') { continue; }
        if !logical.is_empty() { logical.push(' '); }
        logical.push_str(&stripped);
        parens += stripped.matches('(').count() as i32;
        parens -= stripped.matches(')').count() as i32;
        if parens <= 0 {
            lines.push(logical.replace('(', " ").replace(')', " "));
            logical.clear();
            parens = 0;
        }
    }
    if !logical.is_empty() { lines.push(logical); }

    for line in lines {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 3 { continue; }
        let type_idx = parts.iter().position(|p| matches!(p.to_ascii_uppercase().as_str(), "MX" | "TXT"));
        let Some(type_idx) = type_idx else { continue; };
        let (name, data_idx) = if type_idx == 0 {
            if parts.len() < 3 { continue; }
            (expand_owner(parts[1], &apex), 2usize)
        } else {
            (expand_owner(parts[0], &apex), type_idx + 1)
        };
        let kind = parts[type_idx].to_ascii_uppercase();
        if kind == "MX" {
            if data_idx + 1 >= parts.len() { continue; }
            let priority = parts[data_idx].parse::<u16>().ok();
            let value = expand_owner(parts[data_idx + 1], &apex);
            if name == apex && !value.is_empty() {
                records.push(ExpectedRecord { kind, name, value, priority });
            }
            continue;
        }
        if data_idx >= parts.len() { continue; }
        let txt = clean_txt(&parts[data_idx..].join(" "));
        let lower = txt.to_ascii_lowercase();
        let wanted = (name == apex && lower.starts_with("v=spf1"))
            || (name.starts_with("_dmarc.") && lower.starts_with("v=dmarc1"))
            || (name.contains("._domainkey.") && lower.starts_with("v=dkim1"));
        if wanted {
            records.push(ExpectedRecord { kind, name, value: txt, priority: None });
        }
    }
    records
}

fn txt_matches(expected: &str, observed: &[String]) -> bool {
    let expected = clean_txt(expected).to_ascii_lowercase();
    observed.iter().any(|v| clean_txt(v).to_ascii_lowercase() == expected)
}

fn spf_matches(expected: &str, observed: &[String]) -> bool {
    let spf = observed.iter().map(|v| clean_txt(v)).filter(|v| v.to_ascii_lowercase().starts_with("v=spf1")).collect::<Vec<_>>();
    // Multiple SPF records are invalid and must not activate the domain.
    if spf.len() != 1 { return false; }
    let current = spf[0].to_ascii_lowercase();
    let expected_tokens = clean_txt(expected).to_ascii_lowercase();
    expected_tokens.split_whitespace().skip(1).filter(|token| {
        !token.ends_with("all") && !token.starts_with("ra=") && !token.starts_with("rp=")
    }).all(|token| current.split_whitespace().any(|have| have == token))
}

fn dmarc_present(observed: &[String]) -> bool {
    observed.iter().filter(|v| clean_txt(v).to_ascii_lowercase().starts_with("v=dmarc1")).count() == 1
}

pub async fn check(zone: &str, domain: &str) -> Result<DnsReadiness, dns::DnsError> {
    let expected = parse_required_records(zone, domain);
    let mut txt_cache: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut mx_cache: BTreeMap<String, Vec<dns::MxRecord>> = BTreeMap::new();
    let mut mx_expected = 0usize;
    let mut mx_matched = 0usize;
    let mut spf_ok = false;
    let mut dkim_expected = 0usize;
    let mut dkim_ok = 0usize;
    let mut dmarc_ok = false;

    for record in &expected {
        if record.kind == "MX" {
            let values = if let Some(v) = mx_cache.get(&record.name) { v.clone() } else {
                let v = dns::mx_records(&record.name).await?;
                mx_cache.insert(record.name.clone(), v.clone()); v
            };
            mx_expected += 1;
            let matched = values.iter().any(|v| clean_name(&v.exchange) == clean_name(&record.value)
                && record.priority.map(|p| p == v.priority).unwrap_or(true));
            if matched { mx_matched += 1; }
        } else if record.kind == "TXT" {
            let values = if let Some(v) = txt_cache.get(&record.name) { v.clone() } else {
                let v = dns::txt_records(&record.name).await?;
                txt_cache.insert(record.name.clone(), v.clone()); v
            };
            let lower_name = record.name.to_ascii_lowercase();
            let lower_value = record.value.to_ascii_lowercase();
            if lower_name.contains("._domainkey.") && lower_value.starts_with("v=dkim1") {
                dkim_expected += 1;
                if txt_matches(&record.value, &values) { dkim_ok += 1; }
            } else if lower_name.starts_with("_dmarc.") && lower_value.starts_with("v=dmarc1") {
                dmarc_ok |= dmarc_present(&values);
            } else if lower_value.starts_with("v=spf1") {
                spf_ok |= spf_matches(&record.value, &values);
            }
        }
    }

    let mx_ok = mx_expected > 0 && mx_matched == mx_expected;
    let dkim_ready = dkim_expected > 0 && dkim_ok == dkim_expected;
    // Missing expectations are never treated as ready. This catches provider
    // configuration errors (for example a broken default hostname/DKIM setup)
    // instead of activating a customer domain with an incomplete zone.
    let has_spf = expected.iter().any(|r| r.kind == "TXT" && r.value.to_ascii_lowercase().starts_with("v=spf1"));
    let has_dmarc = expected.iter().any(|r| r.kind == "TXT" && r.name.to_ascii_lowercase().starts_with("_dmarc."));
    spf_ok &= has_spf;
    dmarc_ok &= has_dmarc;
    let ready = mx_ok && spf_ok && dkim_ready && dmarc_ok;

    Ok(DnsReadiness {
        mx: mx_ok,
        spf: spf_ok,
        dkim: dkim_ready,
        dmarc: dmarc_ok,
        ready,
        expected,
        observed: json!({"txt": txt_cache, "mx": mx_cache}),
    })
}

pub async fn refresh_one(state: &AppState, domain_id: Uuid) -> Result<DnsReadiness, String> {
    let row: Option<(String, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT domain::text, provider_domain_id, provider_marker, dns_zone_file, status FROM organization_domains WHERE id=$1",
    ).bind(domain_id).fetch_optional(&state.db).await.map_err(|e| e.to_string())?;
    let Some((domain, provider_id, marker, _zone, status)) = row else { return Err("domain not found".into()); };
    let provider_id = provider_id.filter(|value| !value.trim().is_empty()).ok_or_else(|| "domain is not provisioned".to_string())?;
    if marker.trim().is_empty() { return Err("provider ownership marker is missing".into()); }

    // Refresh and re-validate provider ownership before using its DNS output.
    // Never trust an id alone on the Stalwart instance shared with other apps.
    let snapshot = state.stalwart.customer_domain_snapshot(&provider_id).await.map_err(|e| e.to_string())?;
    if snapshot.name != domain || snapshot.description != marker || !snapshot.enabled {
        let next_status = if status == "active" { "degraded" } else { "failed" };
        let reason = if !snapshot.enabled {
            "Mail-provider domain is disabled"
        } else {
            "Mail-provider domain ownership check failed"
        };
        let _ = sqlx::query(
            "UPDATE organization_domains SET status=$2, dns_ready=FALSE, last_error=$3, next_dns_check_at=now()+interval '5 minutes', updated_at=now() WHERE id=$1",
        ).bind(domain_id).bind(next_status).bind(reason).execute(&state.db).await;
        return Err(reason.to_string());
    }
    let zone = snapshot.dns_zone_file;
    sqlx::query("UPDATE organization_domains SET dns_zone_file=$2, provider_synced_at=now(), updated_at=now() WHERE id=$1")
        .bind(domain_id).bind(&zone).execute(&state.db).await.map_err(|e| e.to_string())?;
    if zone.trim().is_empty() { return Err("mail provider has not generated DNS records yet".into()); }
    let result = check(&zone, &domain).await.map_err(|e| e.to_string())?;
    let next_status = if result.ready { "active" } else if status == "active" { "degraded" } else { "dns_pending" };
    let next_seconds: i64 = if result.ready { 900 } else { 120 };
    sqlx::query(
        "UPDATE organization_domains SET status=$2, dns_expected=$3, dns_observed=$4,
         dns_mx_ready=$5, dns_spf_ready=$6, dns_dkim_ready=$7, dns_dmarc_ready=$8,
         dns_ready=$9, last_dns_readiness_check=now(), next_dns_check_at=now()+($10 * interval '1 second'),
         dns_check_attempts=dns_check_attempts+1, activated_at=CASE WHEN $9 AND activated_at IS NULL THEN now() ELSE activated_at END,
         last_error=CASE WHEN $9 THEN '' ELSE 'Waiting for required MX/SPF/DKIM/DMARC DNS records' END, updated_at=now()
         WHERE id=$1",
    )
    .bind(domain_id).bind(next_status).bind(json!(&result.expected)).bind(&result.observed)
    .bind(result.mx).bind(result.spf).bind(result.dkim).bind(result.dmarc).bind(result.ready).bind(next_seconds)
    .execute(&state.db).await.map_err(|e| e.to_string())?;
    if next_status != status {
        // Domain suspension/degradation is enforced at the provider account level.
        // When DNS becomes healthy again, enqueue fresh access reconciliation for
        // every mailbox so IMAP/SMTP logins are restored only if the organization,
        // membership, user and mailbox are all currently active.
        if next_status == "active" {
            let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
            let mailbox_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM mailboxes WHERE domain_id=$1 AND deleted_at IS NULL AND status <> 'deleting'"
            )
            .bind(domain_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            for mailbox_id in mailbox_ids {
                state.provisioning.enqueue_mailbox_access_tx(&mut tx, mailbox_id)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
        }
        audit::record(state, None, "business.domain.dns_status", json!({
            "domain_id": domain_id,
            "domain": domain,
            "from": status,
            "to": next_status,
            "mx": result.mx,
            "spf": result.spf,
            "dkim": result.dkim,
            "dmarc": result.dmarc
        })).await;
    }
    Ok(result)
}

async fn claim_due(state: &AppState) -> Result<Vec<DueDomain>, sqlx::Error> {
    let mut tx = state.db.begin().await?;
    let rows = sqlx::query_as::<_, DueDomain>(
        "SELECT id, organization_id, domain::text AS domain, provider_domain_id
         FROM organization_domains
         WHERE is_system=FALSE AND status IN ('dns_pending','active','degraded')
           AND provider_domain_id IS NOT NULL AND provider_domain_id<>''
           AND (next_dns_check_at IS NULL OR next_dns_check_at <= now())
         ORDER BY COALESCE(next_dns_check_at, created_at) ASC
         LIMIT 20 FOR UPDATE SKIP LOCKED",
    ).fetch_all(&mut *tx).await?;
    for row in &rows {
        sqlx::query("UPDATE organization_domains SET next_dns_check_at=now()+interval '2 minutes' WHERE id=$1")
            .bind(row.id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(rows)
}

pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let rows = match claim_due(&state).await {
                Ok(rows) => rows,
                Err(error) => { tracing::warn!(%error, "domain DNS readiness claim failed"); continue; }
            };
            for row in rows {
                if let Err(error) = refresh_one(&state, row.id).await {
                    tracing::warn!(domain_id=%row.id, organization_id=%row.organization_id, domain=%row.domain, provider_domain_id=%row.provider_domain_id, %error, "domain DNS readiness refresh failed");
                    let _ = sqlx::query("UPDATE organization_domains SET last_error=$2, next_dns_check_at=now()+interval '5 minutes', updated_at=now() WHERE id=$1")
                        .bind(row.id).bind(error).execute(&state.db).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_required_records;
    #[test]
    fn parses_required_zone_records() {
        let zone = r#"
MX example.com. 10 smtp.crescentsphere.com.
TXT example.com. "v=spf1 mx -all"
TXT 202609._domainkey.example.com. "v=DKIM1; k=rsa; p=abc"
TXT _dmarc.example.com. "v=DMARC1; p=reject"
"#;
        let records = parse_required_records(zone, "example.com");
        assert_eq!(records.len(), 4);
    }
}
