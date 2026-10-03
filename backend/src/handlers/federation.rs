//! CS Connect is the identity provider for cross-service sign-in. A verified
//! identity creates a Mail login, never a mailbox, business, plan, or role.
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::handlers::auth::{issue_tokens, session_user_agent, user_json, with_session_cookie};
use crate::state::AppState;

const PROVIDER: &str = "connect";
const TOKEN_URL: &str = "https://connect.crescentsphere.com/o/token/";
const USERINFO_URL: &str =
    "https://connect.crescentsphere.com/api/v1/accounts/federation/userinfo/";
const AUTHORIZE_URL: &str = "https://connect.crescentsphere.com/o/authorize/";

#[derive(Deserialize)]
pub struct CompleteIn {
    code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct Identity {
    sub: String,
    email: String,
    email_verified: bool,
    name: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/connect/config", get(config))
        .route("/api/auth/connect/complete", post(complete))
}

fn available(state: &AppState) -> bool {
    state.connect_client_id.is_some()
        && state.connect_client_secret.is_some()
        && state.public_origin.starts_with("https://")
}

async fn config(State(state): State<AppState>) -> Json<Value> {
    if !available(&state) {
        return Json(json!({"enabled": false}));
    }
    Json(json!({
        "enabled": true,
        "client_id": state.connect_client_id,
        "authorize_url": AUTHORIZE_URL,
        "redirect_uri": format!("{}/auth/connect/callback", state.public_origin.trim_end_matches('/')),
    }))
}

async fn complete(
    headers: HeaderMap,
    peer: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(input): Json<CompleteIn>,
) -> Result<Response, ApiError> {
    if !available(&state) {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "federation_unavailable",
            "CS Connect sign-in is unavailable",
        ));
    }
    let ip = state.rate.client_ip(&headers, Some(peer.0));
    state
        .rate
        .check_burst(&format!("connect-signin:{ip}"), 12, 300)
        .await?;
    if input.code.is_empty()
        || input.code.len() > 2048
        || !(43..=128).contains(&input.code_verifier.len())
        || !input
            .code_verifier
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
    {
        return Err(ApiError::bad_request("Invalid sign-in response"));
    }
    let client_id = state.connect_client_id.as_deref().unwrap_or_default();
    let client_secret = state.connect_client_secret.as_deref().unwrap_or_default();
    let redirect_uri = format!(
        "{}/auth/connect/callback",
        state.public_origin.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::internal("Federation HTTP client failed"))?;
    let token = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", input.code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code_verifier", input.code_verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "provider_unavailable",
                "CS Connect could not complete sign-in",
            )
        })?;
    if !token.status().is_success() {
        return Err(ApiError::bad_request(
            "CS Connect authorization expired or was rejected",
        ));
    }
    let token: TokenResponse = token
        .json()
        .await
        .map_err(|_| ApiError::bad_request("Invalid CS Connect authorization response"))?;
    if token.access_token.is_empty() {
        return Err(ApiError::bad_request("Missing CS Connect authorization"));
    }
    let profile = client
        .get(USERINFO_URL)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "provider_unavailable",
                "CS Connect profile is unavailable",
            )
        })?;
    if !profile.status().is_success() {
        return Err(ApiError::bad_request(
            "CS Connect profile could not be verified",
        ));
    }
    let profile: Identity = profile
        .json()
        .await
        .map_err(|_| ApiError::bad_request("Invalid CS Connect profile"))?;
    let email = profile.email.trim().to_lowercase();
    if profile.sub.is_empty()
        || profile.sub.len() > 255
        || !profile.email_verified
        || !crate::handlers::auth::valid_email(&email)
    {
        return Err(ApiError::forbidden(
            "A verified CS Connect email is required",
        ));
    }
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let linked: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM federated_identities WHERE provider=$1 AND subject=$2 FOR UPDATE",
    )
    .bind(PROVIDER)
    .bind(&profile.sub)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let user_id = if let Some((id,)) = linked {
        id
    } else {
        let existing: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email=$1")
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if let Some((existing_id,)) = existing {
            let raw_cookie = crate::middleware::cookie::cookie_token(&headers).unwrap_or_default();
            let cookie_hash = format!("{:x}", Sha256::digest(raw_cookie.as_bytes()));
            let owned: Option<(Uuid,)> = sqlx::query_as(
                "SELECT u.id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND u.id=$2 AND u.email=$3 AND u.status='active' AND u.email_verified_at IS NOT NULL AND u.two_factor_enabled_at IS NULL AND s.revoked_at IS NULL AND s.expires_at>now()"
            ).bind(cookie_hash).bind(existing_id).bind(&email).fetch_optional(&mut *tx).await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            if owned.is_none() {
                return Err(ApiError::conflict("A CS Mail account already uses this email. Sign in to CS Mail first to link it. Accounts with two-factor authentication need a separate step-up flow."));
            }
            sqlx::query("INSERT INTO federated_identities (provider,subject,user_id,email_at_link) VALUES ($1,$2,$3,$4)")
                .bind(PROVIDER).bind(&profile.sub).bind(existing_id).bind(&email).execute(&mut *tx).await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            existing_id
        } else {
            // Federated-only accounts retain an unguessable local password hash.
            let random_password = Uuid::new_v4().to_string() + &Uuid::new_v4().to_string();
            let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let password_hash = Argon2::default()
                .hash_password(random_password.as_bytes(), &salt)
                .map_err(|e| ApiError::internal(e.to_string()))?
                .to_string();
            let display_name = profile.name.as_deref().unwrap_or("").trim();
            let display_name = if display_name.is_empty() || display_name.len() > 120 {
                email.split('@').next().unwrap_or("Member")
            } else {
                display_name
            };
            let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO users (email,display_name,password_hash,plan,quota_bytes,platform_role,mail_sync_status,email_verified_at) SELECT $1,$2,$3,p.code,p.mailbox_bytes,'user','none',now() FROM plans p WHERE p.code='solo' RETURNING id"
        ).bind(&email).bind(display_name).bind(password_hash).fetch_one(&mut *tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query("INSERT INTO federated_identities (provider,subject,user_id,email_at_link) VALUES ($1,$2,$3,$4)")
            .bind(PROVIDER).bind(&profile.sub).bind(id).bind(&email).execute(&mut *tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
            id
        }
    };
    let account: Option<(String, String, String, String, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT email::text,display_name,platform_role,status,email_verified_at,two_factor_enabled_at FROM users WHERE id=$1"
    ).bind(user_id).fetch_optional(&mut *tx).await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let (local_email, name, role, status, verified, two_factor_enabled_at) =
        account.ok_or_else(|| ApiError::internal("Federated account missing"))?;
    if status != "active" || verified.is_none() {
        return Err(ApiError::forbidden("This CS Mail account is unavailable"));
    }
    if two_factor_enabled_at.is_some() {
        return Err(ApiError::forbidden(
            "This CS Mail account requires a local two-factor sign-in",
        ));
    }
    sqlx::query(
        "UPDATE federated_identities SET last_login_at=now() WHERE provider=$1 AND subject=$2",
    )
    .bind(PROVIDER)
    .bind(&profile.sub)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let (access, refresh) = issue_tokens(
        &state,
        user_id,
        &local_email,
        &role,
        &session_user_agent(&headers),
        &ip,
    )
    .await?;
    Ok(with_session_cookie(
        &state,
        Json(json!({
            "access": access,
            "user": user_json(user_id, &local_email, &name, &role, true),
        }))
        .into_response(),
        &refresh,
    ))
}
