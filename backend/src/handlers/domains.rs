use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::middleware::rate_limit::{
    DNS_CHECK_LIMIT, DNS_CHECK_WINDOW, DOMAIN_OPERATION_LIMIT, DOMAIN_OPERATION_WINDOW,
};
use crate::services::{cloudflare_dns, dns, domain_onboarding, email, tenancy};
use crate::state::AppState;

const CHALLENGE_TTL_HOURS: i64 = 72;
const VERIFY_COOLDOWN_SECONDS: i64 = 15;
const VERIFY_PREFIX: &str = "_cs-mail-verify";
const VERIFY_VALUE_PREFIX: &str = "cs-mail-verification=";

#[derive(Debug, sqlx::FromRow)]
struct DomainRow {
    id: Uuid,
    organization_id: Uuid,
    domain: String,
    status: String,
    is_primary: bool,
    is_system: bool,
    verified_at: Option<DateTime<Utc>>,
    verification_name: Option<String>,
    verification_token: Option<String>,
    verification_expires_at: Option<DateTime<Utc>>,
    verification_attempts: i32,
    last_checked_at: Option<DateTime<Utc>>,
    last_dns_value: String,
    last_error: String,
    provider_domain_id: Option<String>,
    provider_marker: String,
    shared_mailer_domain: bool,
    provider_synced_at: Option<DateTime<Utc>>,
    dns_zone_file: String,
    dns_expected: Value,
    dns_observed: Value,
    dns_mx_ready: bool,
    dns_spf_ready: bool,
    dns_dkim_ready: bool,
    dns_dmarc_ready: bool,
    dns_ready: bool,
    last_dns_readiness_check: Option<DateTime<Utc>>,
}

fn normalize_domain(raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim().trim_end_matches('.').to_lowercase();
    if raw.is_empty() {
        return Err(ApiError::bad_request("Domain is required"));
    }
    let ascii = idna::domain_to_ascii(&raw)
        .map_err(|_| ApiError::bad_request("Domain name is not valid"))?
        .to_lowercase();
    if ascii.len() > 253 || !ascii.contains('.') {
        return Err(ApiError::bad_request("Enter a registrable business domain"));
    }
    if ascii.parse::<std::net::IpAddr>().is_ok() {
        return Err(ApiError::bad_request("An IP address cannot be claimed as a mail domain"));
    }
    let labels: Vec<&str> = ascii.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        return Err(ApiError::bad_request("Domain name is not valid"));
    }
    // Reserved/non-public namespaces should never enter the customer ownership
    // flow even if a local resolver happens to answer them.
    let tld = labels.last().copied().unwrap_or_default();
    if matches!(tld, "localhost" | "local" | "internal" | "invalid" | "example" | "test") {
        return Err(ApiError::bad_request("This domain cannot be used for public business mail"));
    }
    Ok(ascii)
}

fn reserved_platform_domain(domain: &str) -> bool {
    domain == "crescentsphere.com" || domain.ends_with(".crescentsphere.com")
}

fn verification_name(domain: &str) -> String {
    format!("{VERIFY_PREFIX}.{domain}")
}

fn verification_value(token: &str) -> String {
    format!("{VERIFY_VALUE_PREFIX}{token}")
}

fn serialize_domain(row: &DomainRow, include_challenge: bool) -> Value {
    let value = if include_challenge { row.verification_token.as_deref().map(verification_value) } else { None };
    json!({
        "id": row.id,
        "organization_id": row.organization_id,
        "domain": row.domain,
        "status": row.status,
        "is_primary": row.is_primary,
        "is_system": row.is_system,
        "verified_at": row.verified_at,
        "verification": {
            "type": "TXT",
            "name": if include_challenge { row.verification_name.clone() } else { None },
            "value": value,
            "expires_at": row.verification_expires_at,
            "attempts": row.verification_attempts,
            "last_checked_at": row.last_checked_at,
            "last_observed_value": row.last_dns_value,
        },
        "provider": {
            "provisioned": row.provider_domain_id.as_deref().is_some_and(|v| !v.is_empty()),
            "shared_with_mailer": row.shared_mailer_domain,
            "synced_at": row.provider_synced_at,
        },
        "dns": {
            "zone_file": row.dns_zone_file,
            "expected": row.dns_expected.clone(),
            "observed": row.dns_observed.clone(),
            "mx": row.dns_mx_ready,
            "spf": row.dns_spf_ready,
            "dkim": row.dns_dkim_ready,
            "dmarc": row.dns_dmarc_ready,
            "ready": row.dns_ready,
            "last_checked_at": row.last_dns_readiness_check,
        },
        "last_error": row.last_error,
    })
}

async fn load_domain(state: &AppState, organization_id: Uuid, domain_id: Uuid) -> Result<DomainRow, ApiError> {
    sqlx::query_as::<_, DomainRow>(
        "SELECT id, organization_id, domain::text AS domain, status, is_primary, is_system,
                verified_at, verification_name, verification_token, verification_expires_at,
                verification_attempts, last_checked_at, last_dns_value, last_error,
                provider_domain_id, provider_marker, shared_mailer_domain, provider_synced_at, dns_zone_file, dns_expected, dns_observed,
                dns_mx_ready, dns_spf_ready, dns_dkim_ready, dns_dmarc_ready, dns_ready, last_dns_readiness_check
         FROM organization_domains
         WHERE id=$1 AND organization_id=$2",
    )
    .bind(domain_id)
    .bind(organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Domain claim not found"))
}

async fn record_event(
    state: &AppState,
    domain_id: Uuid,
    organization_id: Uuid,
    user_id: Uuid,
    outcome: &str,
    observed: &[String],
) {
    let _ = sqlx::query(
        "INSERT INTO domain_verification_events(domain_id, organization_id, actor_user_id, outcome, observed_values)
         VALUES($1,$2,$3,$4,$5)",
    )
    .bind(domain_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(outcome)
    .bind(json!(observed))
    .execute(&state.db)
    .await;
}

#[derive(Deserialize)]
pub struct CreateDomainIn {
    domain: String,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let membership = tenancy::require_member(&state.db, auth.user_id, organization_id).await?;
    let include_challenge = matches!(membership.role.as_str(), "owner" | "admin");
    let rows = sqlx::query_as::<_, DomainRow>(
        "SELECT id, organization_id, domain::text AS domain, status, is_primary, is_system,
                verified_at, verification_name, verification_token, verification_expires_at,
                verification_attempts, last_checked_at, last_dns_value, last_error,
                provider_domain_id, provider_marker, shared_mailer_domain, provider_synced_at, dns_zone_file, dns_expected, dns_observed,
                dns_mx_ready, dns_spf_ready, dns_dkim_ready, dns_dmarc_ready, dns_ready, last_dns_readiness_check
         FROM organization_domains WHERE organization_id=$1 ORDER BY is_primary DESC, created_at ASC",
    )
    .bind(organization_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"domains": rows.iter().map(|row| serialize_domain(row, include_challenge)).collect::<Vec<_>>() })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(organization_id): Path<Uuid>,
    Json(body): Json<CreateDomainIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    let subscription = crate::services::entitlements::for_organization(&state, organization_id).await?;
    if !matches!(subscription.subscription_status.as_str(), "active" | "trial") {
        return Err(ApiError::forbidden("Activate a business plan before adding a domain"));
    }
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    crate::services::entitlements::require_capacity(&state, organization_id, crate::services::entitlements::CapacityKind::Domain, 1).await?;
    let domain = normalize_domain(&body.domain)?;
    if reserved_platform_domain(&domain) {
        return Err(ApiError::forbidden("This domain is reserved for CrescentSphere infrastructure"));
    }

    let existing: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT id, organization_id, status FROM organization_domains WHERE lower(domain::text)=lower($1)",
    )
    .bind(&domain)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some((id, owner_org, _)) = existing {
        if owner_org != organization_id {
            return Err(ApiError::conflict("This domain is already claimed by another CS Mail business"));
        }
        let row = load_domain(&state, organization_id, id).await?;
        return Ok(Json(serialize_domain(&row, true)));
    }

    let token = email::random_token();
    let verify_name = verification_name(&domain);
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO organization_domains(
            organization_id, domain, status, verification_name, verification_token,
            verification_expires_at, last_error
         ) VALUES($1,$2,'pending_verification',$3,$4,now()+($5 * interval '1 hour'),'')
         RETURNING id",
    )
    .bind(organization_id)
    .bind(&domain)
    .bind(&verify_name)
    .bind(&token)
    .bind(CHALLENGE_TTL_HOURS)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if matches!(&e, sqlx::Error::Database(db) if db.code().as_deref()==Some("23505")) {
            ApiError::conflict("This domain is already claimed")
        } else {
            ApiError::internal(e.to_string())
        }
    })?;
    sqlx::query(
        "INSERT INTO domain_verification_events(domain_id, organization_id, actor_user_id, outcome)
         VALUES($1,$2,$3,'created')",
    )
    .bind(id).bind(organization_id).bind(auth.user_id)
    .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "business.domain.claim", json!({
        "organization_id": organization_id, "domain_id": id, "domain": domain
    })).await;
    let row = load_domain(&state, organization_id, id).await?;
    Ok(Json(serialize_domain(&row, true)))
}

pub async fn rotate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system || row.verified_at.is_some() || row.status != "pending_verification" {
        return Err(ApiError::bad_request("Only an unverified customer domain can rotate its verification challenge"));
    }
    let token = email::random_token();
    sqlx::query(
        "UPDATE organization_domains SET verification_token=$1,
         verification_expires_at=now()+($2 * interval '1 hour'), verification_attempts=0,
         last_checked_at=NULL, last_dns_value='', last_error='', updated_at=now() WHERE id=$3",
    )
    .bind(&token).bind(CHALLENGE_TTL_HOURS).bind(domain_id)
    .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    record_event(&state, domain_id, organization_id, auth.user_id, "rotated", &[]).await;
    audit::record(&state, Some(auth.user_id), "business.domain.challenge.rotate", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain
    })).await;
    let updated = load_domain(&state, organization_id, domain_id).await?;
    Ok(Json(serialize_domain(&updated, true)))
}

#[derive(Deserialize)]
pub struct CloudflareVerificationIn {
    api_token: String,
}

fn cloudflare_oauth_client_id() -> Option<String> {
    std::env::var("CS_MAIL_CLOUDFLARE_OAUTH_CLIENT_ID").ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 256)
}

pub async fn cloudflare_oauth_config(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let client_id = cloudflare_oauth_client_id();
    Ok(Json(json!({
        "available": client_id.is_some(),
        "client_id": client_id,
        "redirect_uri": format!("{}/mail/business", state.public_origin.trim_end_matches('/')),
        "authorization_url": "https://dash.cloudflare.com/oauth2/auth"
    })))
}

#[derive(Deserialize)]
pub struct CloudflareOAuthExchangeIn {
    code: String,
    code_verifier: String,
}

pub async fn cloudflare_oauth_exchange(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CloudflareOAuthExchangeIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system || matches!(row.status.as_str(), "removing" | "active") {
        return Err(ApiError::conflict("This domain is not awaiting Cloudflare DNS setup"));
    }
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let client_id = cloudflare_oauth_client_id().ok_or_else(|| ApiError::bad_request("Cloudflare connection is not configured"))?;
    if body.code.is_empty() || body.code.len() > 2048 || body.code_verifier.len() < 43 || body.code_verifier.len() > 128
        || !body.code_verifier.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)) {
        return Err(ApiError::bad_request("Cloudflare authorization response is invalid"));
    }
    let redirect_uri = format!("{}/mail/business", state.public_origin.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none()).build()
        .map_err(|_| ApiError::bad_request("Cloudflare authorization could not be prepared"))?;
    let response = client.post("https://dash.cloudflare.com/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id.as_str()),
            ("code", body.code.as_str()),
            ("code_verifier", body.code_verifier.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send().await.map_err(|_| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "cloudflare_oauth", "Cloudflare authorization is temporarily unavailable"))?;
    if !response.status().is_success() {
        return Err(ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, "cloudflare_oauth", "Cloudflare could not complete authorization. Please reconnect and try again."));
    }
    let payload: Value = response.json().await.map_err(|_| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "cloudflare_oauth", "Cloudflare returned an unreadable authorization response"))?;
    let token = payload["access_token"].as_str().unwrap_or_default();
    if token.is_empty() || token.len() > 4096 || payload["token_type"].as_str().is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer")) {
        return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "cloudflare_oauth", "Cloudflare did not return a usable DNS authorization"));
    }
    Ok(Json(json!({"access_token": token})))
}

/// Publish the exact pending ownership challenge with a one-time, zone-scoped
/// Cloudflare token. Public DNS is still checked by the normal verify endpoint.
pub async fn publish_cloudflare_challenge(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CloudflareVerificationIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let token = body.api_token.trim();
    if token.len() < 20 || token.len() > 4096 || token.chars().any(char::is_whitespace) {
        return Err(ApiError::bad_request("Enter a valid Cloudflare API token"));
    }
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system || row.status != "pending_verification" || row.verified_at.is_some() {
        return Err(ApiError::conflict("Only a pending customer domain can use Cloudflare verification"));
    }
    if row.verification_expires_at.is_none_or(|expires| expires <= Utc::now()) {
        return Err(ApiError::conflict("Verification challenge expired. Generate a new challenge first."));
    }
    let name = row.verification_name.as_deref().ok_or_else(|| ApiError::bad_request("Verification challenge is missing"))?;
    let value = verification_value(row.verification_token.as_deref().ok_or_else(|| ApiError::bad_request("Verification challenge is missing"))?);
    let zone = cloudflare_dns::publish_verification_txt(&row.domain, name, &value, token).await?;
    audit::record(&state, Some(auth.user_id), "business.domain.cloudflare_txt", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain, "zone": zone
    })).await;
    Ok(Json(json!({"ok": true, "zone": zone, "message": "Cloudflare TXT record published. Checking public DNS for ownership proof."})))
}

pub async fn verify(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system || row.verified_at.is_some() || row.status == "verified" {
        return Ok(Json(serialize_domain(&row, true)));
    }
    if row.status != "pending_verification" {
        return Err(ApiError::bad_request("This domain is not waiting for ownership verification"));
    }
    let expires = row.verification_expires_at.ok_or_else(|| ApiError::bad_request("Verification challenge is missing"))?;
    if expires <= Utc::now() {
        record_event(&state, domain_id, organization_id, auth.user_id, "expired", &[]).await;
        return Err(ApiError::bad_request("Verification challenge expired. Generate a new challenge."));
    }
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE organization_domains
         SET last_checked_at=now(), verification_attempts=verification_attempts+1, updated_at=now()
         WHERE id=$1 AND organization_id=$2
           AND (last_checked_at IS NULL OR last_checked_at <= now() - ($3 * interval '1 second'))
         RETURNING id",
    )
    .bind(domain_id)
    .bind(organization_id)
    .bind(VERIFY_COOLDOWN_SECONDS)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if claimed.is_none() {
        record_event(&state, domain_id, organization_id, auth.user_id, "rate_limited", &[]).await;
        return Err(ApiError::too_many("DNS verification was checked recently. Wait a few seconds and try again."));
    }
    let name = row.verification_name.as_deref().ok_or_else(|| ApiError::bad_request("Verification challenge is missing"))?;
    let token = row.verification_token.as_deref().ok_or_else(|| ApiError::bad_request("Verification challenge is missing"))?;
    let expected = verification_value(token);

    let observed = match dns::txt_records(name).await {
        Ok(values) => values,
        Err(error) => {
            sqlx::query(
                "UPDATE organization_domains SET last_error=$2, updated_at=now() WHERE id=$1",
            ).bind(domain_id).bind(error.to_string()).execute(&state.db).await
             .map_err(|e| ApiError::internal(e.to_string()))?;
            record_event(&state, domain_id, organization_id, auth.user_id, "error", &[]).await;
            return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "dns_unavailable", "DNS verification service is temporarily unavailable"));
        }
    };
    let matched = observed.iter().any(|value| value.trim() == expected);
    let observed_summary = observed.join(" | ").chars().take(500).collect::<String>();
    if !matched {
        sqlx::query(
            "UPDATE organization_domains SET last_dns_value=$2, last_error='Verification TXT record not found yet', updated_at=now() WHERE id=$1",
        ).bind(domain_id).bind(&observed_summary).execute(&state.db).await
         .map_err(|e| ApiError::internal(e.to_string()))?;
        record_event(&state, domain_id, organization_id, auth.user_id, "not_found", &observed).await;
        let updated = load_domain(&state, organization_id, domain_id).await?;
        return Ok(Json(json!({
            "verified": false,
            "domain": serialize_domain(&updated, true),
            "message": "Verification TXT record not found yet. DNS propagation can take time."
        })));
    }

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE organization_domains SET status='verified', verified_at=now(), verification_token=NULL, verification_expires_at=NULL, last_dns_value=$2, last_error='', updated_at=now()
         WHERE id=$1 AND organization_id=$3 AND status='pending_verification'",
    ).bind(domain_id).bind(&observed_summary).bind(organization_id)
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    // First verified customer domain becomes the business primary domain.
    sqlx::query(
        "UPDATE organization_domains SET is_primary=TRUE, updated_at=now()
         WHERE id=$1 AND NOT EXISTS(
           SELECT 1 FROM organization_domains d WHERE d.organization_id=$2 AND d.is_primary=TRUE AND d.id<>$1
         )",
    ).bind(domain_id).bind(organization_id)
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO domain_verification_events(domain_id, organization_id, actor_user_id, outcome, observed_values)
         VALUES($1,$2,$3,'verified',$4)",
    ).bind(domain_id).bind(organization_id).bind(auth.user_id).bind(json!(observed))
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "business.domain.verified", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain,
        "method": "dns_txt"
    })).await;
    let updated = load_domain(&state, organization_id, domain_id).await?;
    Ok(Json(json!({
        "verified": true,
        "domain": serialize_domain(&updated, true),
        "message": "Domain ownership verified. Mail-provider provisioning is the next setup step."
    })))
}

pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system {
        return Err(ApiError::forbidden("The system domain is managed by the platform operator"));
    }
    if row.verified_at.is_none() {
        return Err(ApiError::bad_request("Verify domain ownership before provisioning mail service"));
    }
    if matches!(row.status.as_str(), "dns_pending" | "active" | "degraded")
        && row.provider_domain_id.as_deref().is_some_and(|v| !v.is_empty())
    {
        return Ok(Json(json!({
            "domain": serialize_domain(&row, true),
            "message": "Mail domain is already provisioned. Complete the DNS readiness checklist."
        })));
    }
    if !matches!(row.status.as_str(), "verified" | "failed" | "provisioning") {
        return Err(ApiError::conflict("This domain is not ready for provider provisioning"));
    }

    let marker = if row.provider_marker.trim().is_empty() {
        state.stalwart.ownership_marker("domain", &organization_id.to_string(), &domain_id.to_string())
    } else {
        row.provider_marker.clone()
    };
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE organization_domains SET status='provisioning', provider_marker=$2, last_error='', updated_at=now()
         WHERE id=$1 AND (status IN ('verified','failed') OR (status='provisioning' AND updated_at < now()-interval '5 minutes'))
         RETURNING id",
    ).bind(domain_id).bind(&marker).fetch_optional(&state.db).await
     .map_err(|e| ApiError::internal(e.to_string()))?;
    if claimed.is_none() {
        return Err(ApiError::conflict("Domain provisioning is already in progress. Wait a few minutes before retrying."));
    }

    let (snapshot, shared_mailer_domain) = match state.stalwart.ensure_customer_or_mailer_domain(
        row.provider_domain_id.as_deref(), &row.domain, &marker,
        row.provider_domain_id.is_none() || row.shared_mailer_domain,
    ).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let detail = error.to_string();
            tracing::error!(organization_id=%organization_id, domain_id=%domain_id, domain=%row.domain, %detail, "customer domain provisioning failed");
            let ownership_conflict = detail.contains("already exists") || detail.contains("does not belong")
                || detail.contains("unrelated ownership marker") || detail.contains("does not match");
            let failure_message = if ownership_conflict {
                "This domain already exists in the shared mail provider under another ownership marker. A platform operator must inspect it before this business can use it."
            } else {
                "Mail-provider provisioning failed. The operation is safe to retry."
            };
            sqlx::query("UPDATE organization_domains SET status='failed', last_error=$2, updated_at=now() WHERE id=$1")
                .bind(domain_id).bind(failure_message)
                .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
            if ownership_conflict {
                return Err(ApiError::conflict(failure_message));
            }
            return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "mail_provider", error.public_message()));
        }
    };
    if !domain_onboarding::provider_binding_matches(&snapshot, &row.domain, &marker, shared_mailer_domain) {
        return Err(ApiError::conflict("Mail-provider domain ownership could not be established safely"));
    }
    let expected = domain_onboarding::parse_required_records(&snapshot.dns_zone_file, &row.domain);
    if expected.is_empty() {
        sqlx::query(
            "UPDATE organization_domains SET provider_domain_id=$2, shared_mailer_domain=$5, provider_synced_at=now(), dns_zone_file=$3,
             dns_expected=$4, status='failed', last_error='Mail provider did not generate the required DNS zone records', updated_at=now() WHERE id=$1",
        ).bind(domain_id).bind(&snapshot.id).bind(&snapshot.dns_zone_file).bind(json!(expected)).bind(shared_mailer_domain)
         .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "mail_provider_dns", "Mail provider did not generate DNS setup records"));
    }

    sqlx::query(
        "UPDATE organization_domains SET provider_domain_id=$2, shared_mailer_domain=$5, provider_synced_at=now(), dns_zone_file=$3,
         dns_expected=$4, status='dns_pending', next_dns_check_at=now(), last_error='', updated_at=now() WHERE id=$1",
    ).bind(domain_id).bind(&snapshot.id).bind(&snapshot.dns_zone_file).bind(json!(expected)).bind(shared_mailer_domain)
     .execute(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "business.domain.provision", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain,
        "provider_domain_id": snapshot.id
    })).await;

    // A first readiness check is best-effort: DNS commonly has not propagated
    // yet and that is not a provisioning failure.
    let _ = domain_onboarding::refresh_one(&state, domain_id).await;
    let updated = load_domain(&state, organization_id, domain_id).await?;
    Ok(Json(json!({
        "domain": serialize_domain(&updated, true),
        "message": if updated.dns_ready { "Domain is active for business mail." } else { "Mail domain provisioned. Publish the required MX/SPF/DKIM/DMARC records, then verify DNS." }
    })))
}

pub async fn check_dns(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("dns-check:{organization_id}:{}", auth.user_id),
        DNS_CHECK_LIMIT,
        DNS_CHECK_WINDOW,
    ).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.provider_domain_id.as_deref().map_or(true, |v| v.trim().is_empty()) {
        return Err(ApiError::bad_request("Provision the verified domain before checking mail DNS"));
    }
    if let Some(last) = row.last_dns_readiness_check {
        if (Utc::now() - last).num_seconds() < VERIFY_COOLDOWN_SECONDS {
            return Err(ApiError::too_many("DNS readiness was checked recently. Wait a few seconds and try again."));
        }
    }
    let readiness = domain_onboarding::refresh_one(&state, domain_id).await.map_err(|error| {
        tracing::warn!(organization_id=%organization_id, domain_id=%domain_id, %error, "manual domain DNS readiness check failed");
        ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "dns_unavailable", "DNS readiness check is temporarily unavailable")
    })?;
    audit::record(&state, Some(auth.user_id), "business.domain.dns_check", json!({
        "organization_id": organization_id, "domain_id": domain_id, "ready": readiness.ready,
        "mx": readiness.mx, "spf": readiness.spf, "dkim": readiness.dkim, "dmarc": readiness.dmarc
    })).await;
    let updated = load_domain(&state, organization_id, domain_id).await?;
    Ok(Json(json!({
        "ready": readiness.ready,
        "domain": serialize_domain(&updated, true),
        "message": if readiness.ready { "MX, SPF, DKIM and DMARC are ready. Business mail is active." } else { "Some required DNS records are still missing or have not propagated." }
    })))
}

/// Publish the provider's mail DNS after public ownership proof and provider
/// provisioning. A zone-scoped Cloudflare token is used for this request only.
pub async fn publish_cloudflare_mail_dns(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CloudflareVerificationIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_domain_onboarding(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let token = body.api_token.trim();
    if token.len() < 20 || token.len() > 4096 || token.chars().any(char::is_whitespace) {
        return Err(ApiError::bad_request("Enter a valid Cloudflare API token"));
    }
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system || row.verified_at.is_none() {
        return Err(ApiError::conflict("Verify ownership of a customer domain first"));
    }
    let provider_id = row.provider_domain_id.as_deref().filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::conflict("Provision this domain on the mail server first"))?;
    if row.provider_marker.trim().is_empty() {
        return Err(ApiError::conflict("Mail-provider ownership metadata is missing"));
    }
    let snapshot = state.stalwart.customer_domain_snapshot(provider_id).await
        .map_err(|error| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "mail_provider", error.public_message()))?;
    if !domain_onboarding::provider_binding_matches(&snapshot, &row.domain, &row.provider_marker, row.shared_mailer_domain) {
        return Err(ApiError::conflict("Mail-provider domain ownership could not be established safely"));
    }
    let records = domain_onboarding::parse_required_records(&snapshot.dns_zone_file, &row.domain);
    let (zone, created) = cloudflare_dns::publish_mail_records(&row.domain, &records, token).await?;
    audit::record(&state, Some(auth.user_id), "business.domain.cloudflare_mail_dns", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain,
        "zone": zone, "created": created
    })).await;
    Ok(Json(json!({
        "ok": true, "zone": zone, "created": created,
        "message": "Mail DNS records are published in Cloudflare. Public DNS checks will activate the domain when MX, SPF, DKIM and DMARC are visible."
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, domain_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(
        &format!("domain-op:{organization_id}:{}", auth.user_id),
        DOMAIN_OPERATION_LIMIT,
        DOMAIN_OPERATION_WINDOW,
    ).await?;
    let row = load_domain(&state, organization_id, domain_id).await?;
    if row.is_system {
        return Err(ApiError::forbidden("The protected CrescentSphere domain cannot be removed"));
    }
    let mailbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mailboxes WHERE domain_id=$1 AND deleted_at IS NULL",
    ).bind(domain_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let address_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM business_addresses WHERE domain_id=$1 AND deleted_at IS NULL",
    ).bind(domain_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if mailbox_count > 0 || address_count > 0 {
        return Err(ApiError::conflict("Remove all mailboxes, aliases and groups on this domain before releasing the claim"));
    }
    if let Some(provider_id) = row.provider_domain_id.as_deref().filter(|value| !value.trim().is_empty()) {
        if row.provider_marker.trim().is_empty() {
            return Err(ApiError::conflict("Provider ownership metadata is missing; a platform administrator must reconcile this domain before removal"));
        }
        let snapshot = state.stalwart.customer_domain_snapshot(provider_id).await
            .map_err(|error| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "mail_provider", error.public_message()))?;
        if !domain_onboarding::provider_binding_matches(&snapshot, &row.domain, &row.provider_marker, row.shared_mailer_domain) {
            return Err(ApiError::conflict("Provider domain binding changed; an operator must reconcile it before release"));
        }
        // A Mailer workspace may also send through this provider domain, even
        // after DKIM rotation. Detach the CS Mail claim; never destroy the
        // shared Stalwart object from a customer-facing operation.
    } else if matches!(row.status.as_str(), "provisioning" | "removing") {
        return Err(ApiError::conflict("Domain provisioning/removal is still in progress"));
    }
    sqlx::query("DELETE FROM organization_domains WHERE id=$1 AND organization_id=$2")
        .bind(domain_id).bind(organization_id).execute(&state.db).await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(auth.user_id), "business.domain.release", json!({
        "organization_id": organization_id, "domain_id": domain_id, "domain": row.domain
    })).await;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::{normalize_domain, reserved_platform_domain};

    #[test]
    fn normalizes_idn_and_trailing_dot() {
        assert_eq!(normalize_domain("BÜCHER.de.").unwrap(), "xn--bcher-kva.de");
    }

    #[test]
    fn rejects_ip_and_single_label() {
        assert!(normalize_domain("127.0.0.1").is_err());
        assert!(normalize_domain("localhost").is_err());
    }

    #[test]
    fn reserves_platform_domain_tree() {
        assert!(reserved_platform_domain("crescentsphere.com"));
        assert!(reserved_platform_domain("mail.crescentsphere.com"));
        assert!(!reserved_platform_domain("example.com"));
    }
}
