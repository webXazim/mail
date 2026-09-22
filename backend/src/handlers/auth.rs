use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::domain;
use crate::error::ApiError;
use crate::middleware::auth::{create_tokens, AuthUser, Claims};
use crate::middleware::cookie::{build_session_cookie, clear_session_cookie, cookie_token};
use crate::middleware::rate_limit::{
    AUTH_IP_LIMIT, AUTH_IP_WINDOW, CRED_LOCK, CRED_MAX_FAILS, CRED_WINDOW,
    EMAIL_HOURLY_LIMIT, EMAIL_HOURLY_WINDOW, REGISTER_IP_LIMIT, REGISTER_IP_WINDOW,
};
use crate::services::{email, provisioning};
use crate::state::AppState;

fn gen_salt() -> SaltString {
    SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).expect("base64 salt")
}

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Lightweight structural email check: non-empty local part and a domain with
/// at least a dot in it. Good enough to reject typos at the door.
pub(crate) fn valid_email(email: &str) -> bool {
    let mut parts = email.trim().splitn(2, '@');
    let Some(local) = parts.next() else {
        return false;
    };
    let Some(domain) = parts.next() else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && domain.len() >= 4
}

#[derive(Deserialize)]
pub struct RegisterIn {
    name: String,
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub struct LoginIn {
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub struct VerifyIn {
    token: String,
}

#[derive(Deserialize)]
pub struct ResendIn {
    email: String,
}

#[derive(Deserialize)]
pub struct ForgotIn {
    email: String,
}

#[derive(Deserialize)]
pub struct ResetIn {
    token: String,
    password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordIn {
    current_password: String,
    new_password: String,
}

/// Metadata persisted with a browser/device session. The raw refresh token is
/// never stored; only its SHA-256 hash is persisted.
pub(crate) fn session_user_agent(headers: &HeaderMap) -> String {
    headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("Unknown client")
        .chars()
        .take(512)
        .collect()
}

fn device_label(user_agent: &str) -> String {
    let browser = if user_agent.contains("Edg/") {
        "Edge"
    } else if user_agent.contains("Firefox/") {
        "Firefox"
    } else if user_agent.contains("Chrome/") {
        "Chrome"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if user_agent.contains("Windows") {
        "Windows"
    } else if user_agent.contains("Mac OS X") || user_agent.contains("Macintosh") {
        "macOS"
    } else if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        "iOS"
    } else if user_agent.contains("Linux") {
        "Linux"
    } else {
        "Unknown OS"
    };
    format!("{browser} on {os}")
}

/// Mint a fresh token pair and persist the refresh half as a stable device
/// session. Refresh rotation updates this row instead of creating a new
/// logical session on every rotation.
pub(crate) async fn issue_tokens(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    role: &str,
    user_agent: &str,
    ip: &str,
) -> Result<(String, String), ApiError> {
    let session_id = Uuid::new_v4();
    let (access, refresh) = create_tokens(
        user_id,
        session_id,
        email,
        role,
        &state.jwt_secret,
        state.jwt_access_ttl_secs,
        state.jwt_refresh_ttl_secs,
    );

    let refresh_hash = token_hash(&refresh);

    sqlx::query(
        "INSERT INTO sessions
            (id, user_id, token_hash, user_agent, ip, expires_at, last_used_at)
         VALUES ($1, $2, $3, $4, $5, now() + ($6 * interval '1 second'), now())",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(refresh_hash)
    .bind(user_agent)
    .bind(ip)
    .bind(state.jwt_refresh_ttl_secs as i64)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((access, refresh))
}

/// Attach the session cookie (refresh token) to a response.
pub(crate) fn with_session_cookie(state: &AppState, mut res: Response, token: &str) -> Response {
    let cookie = build_session_cookie(state, token, state.jwt_refresh_ttl_secs as i64);
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        res.headers_mut().append(SET_COOKIE, value);
    } else {
        tracing::error!("failed to build session cookie header");
    }
    res
}

fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        // No Origin header: non-browser client (curl, servers). Allowed.
        return true;
    };
    state.cors_origins.iter().any(|o| o == origin.trim())
}

/// Full login row: id, email, display_name, password_hash, role, verified-at.
type DbLoginUser = (
    Uuid,
    String,
    String,
    String,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);
/// Profile fields for verify/resend lookups.
type DbProfileUser = (Uuid, String, String, String, String, Option<DateTime<Utc>>);

pub(crate) fn user_json(id: Uuid, email: &str, display_name: &str, platform_role: &str, email_verified: bool) -> Value {
    let client_role = if platform_role == "platform_admin" { "admin" } else { "member" };
    json!({
        "id": id,
        "email": email,
        "display_name": display_name,
        "role": client_role,
        "platform_role": platform_role,
        "email_verified": email_verified
    })
}

pub async fn register(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<RegisterIn>,
) -> Result<Response, ApiError> {
    crate::services::platform_control::require_signup(&state.db).await?;
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;
    state
        .rate
        .check_burst(
            &format!("register-ip:{ip}"),
            REGISTER_IP_LIMIT,
            REGISTER_IP_WINDOW,
        )
        .await?;

    let name = body.name.trim();
    let email = body.email.trim().to_lowercase();

    if name.is_empty() {
        return Err(ApiError::bad_request("Name is required"));
    }
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Invalid email address"));
    }
    domain::password::validate_password(&body.password, &email)?;

    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if exists.is_some() {
        return Err(ApiError::conflict(
            "An account with this email already exists",
        ));
    }

    let local = email.split('@').next().unwrap_or("");
    let display_name = if name.is_empty() { local } else { name };

    let salt = gen_salt();
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .to_string();

    // Upgrade 19: registration creates a platform identity only. A public
    // customer may sign up with Gmail/Outlook and later create a business,
    // verify its domain, and provision mailboxes. Login email is not a mailbox.
    let user: (Uuid, String, String, i64) = sqlx::query_as(
        "INSERT INTO users (email, display_name, password_hash, plan, quota_bytes, platform_role, mail_sync_status)
         SELECT $1, $2, $3, p.code, p.mailbox_bytes, 'user', 'none'
         FROM plans p WHERE p.code = 'solo'
         RETURNING id, email::text, display_name, quota_bytes",
    )
    .bind(&email)
    .bind(display_name)
    .bind(hash)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(user.0),
        "auth.register",
        json!({ "email": user.1, "platform_identity_only": true }),
    )
    .await;

    if state.require_verification {
        let link = email::send_verification(&state, user.0, &user.1, &user.2).await?;
        let mut value = json!({
            "ok": true,
            "message": "Account created. Check your inbox to verify your email."
        });
        if state.return_token_links {
            value["dev"] = json!({ "verify_link": link });
        }
        return Ok(Json(value).into_response());
    }

    // Development path (CS_MAIL_REQUIRE_VERIFICATION=false): behave like the
    // pre-verification API and issue a session immediately.
    sqlx::query(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
    )
    .bind(user.0)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (access, refresh) = issue_tokens(&state, user.0, &user.1, "user", &session_user_agent(&headers), &ip).await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "user": user_json(user.0, &user.1, &user.2, "user", true)
        })),
    )
        .into_response();
    Ok(with_session_cookie(&state, res, &refresh))
}

pub async fn login(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<LoginIn>,
) -> Result<Response, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;

    let email = body.email.trim().to_lowercase();
    if email.is_empty() || body.password.is_empty() {
        return Err(ApiError::bad_request("Email and password are required"));
    }

    let cred_key = format!("cred:{email}|{ip}");

    // While a lock is active, refuse before touching the DB or running argon2.
    state.rate.check_lock(&cred_key).await?;

    let row: Option<DbLoginUser> = sqlx::query_as(
        "SELECT id, email, display_name, password_hash, platform_role, status,
                email_verified_at, two_factor_enabled_at
             FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (
        user_id,
        email,
        display_name,
        password_hash,
        role,
        status,
        email_verified_at,
        two_factor_enabled_at,
    ) = match row {
        Some(row) => row,
        None => {
            let _ = state
                .rate
                .record_failure(&cred_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
            return Err(ApiError::unauthorized("Invalid email or password"));
        }
    };

    let parsed_hash =
        PasswordHash::new(&password_hash).map_err(|e| ApiError::internal(e.to_string()))?;

    if Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .is_err()
    {
        let _ = state
            .rate
            .record_failure(&cred_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Invalid email or password"));
    }

    state.rate.reset(&cred_key).await?;

    if status != "active" {
        audit::record(
            &state,
            Some(user_id),
            "auth.login.blocked_suspended",
            json!({ "email": &email }),
        )
        .await;
        return Err(ApiError::forbidden("This account is suspended"));
    }

    let verified = email_verified_at.is_some();
    if state.require_verification && !verified {
        audit::record(
            &state,
            Some(user_id),
            "auth.login.blocked_unverified",
            json!({ "email": &email }),
        )
        .await;
        return Err(ApiError::forbidden(
            "Please verify your email before signing in",
        ));
    }

    if two_factor_enabled_at.is_some() {
        let user_agent = session_user_agent(&headers);
        let challenge = crate::handlers::two_factor::create_login_challenge(
            &state,
            user_id,
            &ip,
            &user_agent,
        )
        .await?;
        audit::record(
            &state,
            Some(user_id),
            "auth.login.password_verified_2fa_required",
            json!({ "email": &email, "ip": ip }),
        )
        .await;
        return Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "two_factor_required": true,
                "challenge_token": challenge,
                "expires_in": crate::services::two_factor::CHALLENGE_TTL_MINUTES * 60
            })),
        )
            .into_response());
    }

    audit::record(
        &state,
        Some(user_id),
        "auth.login",
        json!({ "email": &email, "two_factor": false }),
    )
    .await;

    let (access, refresh) = issue_tokens(&state, user_id, &email, &role, &session_user_agent(&headers), &ip).await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "user": user_json(user_id, &email, &display_name, &role, verified)
        })),
    )
        .into_response();
    Ok(with_session_cookie(&state, res, &refresh))
}

pub async fn verify(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<VerifyIn>,
) -> Result<Response, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;

    let token_hash = token_hash(&body.token);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let token: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, user_id FROM email_tokens
         WHERE token_hash = $1 AND kind = 'verify'
           AND used_at IS NULL AND expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (token_id, user_id) = token
        .ok_or_else(|| ApiError::bad_request("This verification link is invalid or has expired"))?;

    // Atomic single-use guard: a second request for the same token gets no row.
    let consumed: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now() WHERE id = $1 AND used_at IS NULL RETURNING id",
    )
    .bind(token_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if consumed.is_none() {
        return Err(ApiError::bad_request(
            "This verification link has already been used",
        ));
    }

    let user: Option<DbProfileUser> = sqlx::query_as(
        "SELECT id, email, display_name, platform_role, status, email_verified_at
         FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (user_id, email, display_name, role, status, already) =
        user.ok_or_else(|| ApiError::internal("Verification target user no longer exists"))?;

    if status != "active" {
        return Err(ApiError::forbidden("This account is suspended"));
    }

    if already.is_some() {
        return Err(ApiError::bad_request("This email is already verified"));
    }

    sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(user_id),
        "auth.verify",
        json!({ "email": &email }),
    )
    .await;

    let (access, refresh) = issue_tokens(&state, user_id, &email, &role, &session_user_agent(&headers), &ip).await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "user": user_json(user_id, &email, &display_name, &role, true)
        })),
    )
        .into_response();
    Ok(with_session_cookie(&state, res, &refresh))
}

pub async fn resend_verification(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<ResendIn>,
) -> Result<Json<Value>, ApiError> {
    let email = body.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Invalid email address"));
    }

    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;
    state.rate.check_burst(
        &format!("email:{email}"),
        EMAIL_HOURLY_LIMIT,
        EMAIL_HOURLY_WINDOW,
    ).await?;

    let user: Option<(Uuid, String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, email, display_name, email_verified_at FROM users WHERE email = $1 AND status = 'active'",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    // Never leak whether an account exists: identical success response.
    let mut value = json!({
        "ok": true,
        "message": "If an account exists for that email, a verification link is on its way"
    });
    if let Some((user_id, email, display_name, verified)) = user {
        if verified.is_none() {
            if let Ok(link) = email::send_verification(&state, user_id, &email, &display_name).await
            {
                if state.return_token_links {
                    value["dev"] = json!({ "verify_link": link });
                }
            }
        }
    }
    Ok(Json(value))
}

pub async fn forgot_password(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<ForgotIn>,
) -> Result<Json<Value>, ApiError> {
    let email = body.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Invalid email address"));
    }

    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;
    state.rate.check_burst(
        &format!("email:{email}"),
        EMAIL_HOURLY_LIMIT,
        EMAIL_HOURLY_WINDOW,
    ).await?;

    let user: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, email, display_name FROM users
         WHERE email = $1 AND email_verified_at IS NOT NULL AND status = 'active'",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    // Same anti-enumeration posture as resend.
    let mut value = json!({
        "ok": true,
        "message": "If an account exists for that email, a reset link is on its way"
    });
    if let Some((user_id, email, display_name)) = user {
        if let Ok(link) = email::send_password_reset(&state, user_id, &email, &display_name).await {
            if state.return_token_links {
                value["dev"] = json!({ "reset_link": link });
            }
        }
    }
    Ok(Json(value))
}

pub async fn reset_password(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<ResetIn>,
) -> Result<Json<Value>, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;

    domain::password::validate_password(&body.password, "")?;
    let token_hash = token_hash(&body.token);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let token: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, user_id FROM email_tokens
         WHERE token_hash = $1 AND kind = 'reset'
           AND used_at IS NULL AND expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (token_id, user_id) =
        token.ok_or_else(|| ApiError::bad_request("This reset link is invalid or has expired"))?;

    let consumed: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now() WHERE id = $1 AND used_at IS NULL RETURNING id",
    )
    .bind(token_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if consumed.is_none() {
        return Err(ApiError::bad_request("This reset link has already been used"));
    }

    let user: Option<(String, String)> = sqlx::query_as(
        "SELECT email::text, status FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (email, status) =
        user.ok_or_else(|| ApiError::bad_request("Account no longer exists"))?;
    let legacy_mailbox: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.address::text, m.provider_account_id
         FROM users u
         JOIN mailboxes m ON m.id=u.primary_mailbox_id AND m.deleted_at IS NULL
         JOIN organizations o ON o.id=m.organization_id AND o.is_system=TRUE
         WHERE u.id=$1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if status != "active" {
        return Err(ApiError::forbidden("This account is suspended"));
    }
    domain::password::validate_password(&body.password, &email)?;

    let salt = gen_salt();
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .to_string();

    sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
        .bind(hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let sync_legacy_mailbox = legacy_mailbox.is_some();
    if let Some((mailbox_address, provider_account_id)) = legacy_mailbox {
        state
            .provisioning
            .enqueue_credentials_tx(
                &mut tx,
                user_id,
                &mailbox_address,
                provider_account_id.as_deref(),
                &body.password,
            )
            .await?;
    }

    // A reset is an account-recovery event: revoke every browser/device
    // session and require a clean login with the new password.
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(user_id), "auth.password_reset", json!({ "ip": ip })).await;
    if sync_legacy_mailbox {
        provisioning::process_user_credentials_now(&state, user_id).await;
    }

    Ok(Json(json!({
        "ok": true,
        "message": "Password updated. You can now sign in."
    })))
}

pub async fn refresh(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    use jsonwebtoken::{decode, DecodingKey, Validation};

    if !origin_allowed(&state, &headers) {
        return Err(ApiError::forbidden("Cross-site request blocked"));
    }

    let refresh_token = cookie_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("No refresh token provided"))?;
    let token_data = decode::<Claims>(
        &refresh_token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::unauthorized("Invalid refresh token"))?;
    if token_data.claims.kind != "refresh" {
        return Err(ApiError::unauthorized("Invalid token type"));
    }

    let old_hash = token_hash(&refresh_token);
    let user_id = token_data.claims.sub;
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    let user_agent = session_user_agent(&headers);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let current: Option<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT s.id, u.email::text, u.platform_role, u.status
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.id = $1 AND s.user_id = $2 AND s.token_hash = $3
           AND s.revoked_at IS NULL AND s.expires_at > now()
         FOR UPDATE OF s",
    )
    .bind(token_data.claims.sid)
    .bind(user_id)
    .bind(&old_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let Some((session_id, email, role, status)) = current else {
        // If this exact token was rotated before, revoke only that stable
        // session family. Unknown/expired tokens do not log out unrelated
        // devices.
        let replay: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT session_id, used_at < now() - interval '10 seconds' AS suspicious
             FROM session_refresh_history
             WHERE token_hash = $1 AND user_id = $2",
        )
        .bind(&old_hash)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if let Some((replayed, suspicious)) = replay {
            if suspicious {
                sqlx::query("UPDATE sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1")
                    .bind(replayed)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| ApiError::internal(e.to_string()))?;
            }
            tx.commit()
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            audit::record(
                &state,
                Some(user_id),
                if suspicious { "auth.refresh.reuse_revoked" } else { "auth.refresh.race_ignored" },
                json!({ "session_id": replayed, "ip": ip }),
            )
            .await;
        }
        return Err(ApiError::unauthorized("Refresh token already used or expired"));
    };

    if status != "active" {
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(ApiError::forbidden("This account is suspended"));
    }

    let (access, new_refresh) = create_tokens(
        user_id,
        session_id,
        &email,
        &role,
        &state.jwt_secret,
        state.jwt_access_ttl_secs,
        state.jwt_refresh_ttl_secs,
    );
    let new_hash = token_hash(&new_refresh);

    sqlx::query(
        "INSERT INTO session_refresh_history (token_hash, session_id, user_id)
         VALUES ($1, $2, $3) ON CONFLICT (token_hash) DO NOTHING",
    )
    .bind(&old_hash)
    .bind(session_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE sessions
         SET token_hash = $1, user_agent = $2, ip = $3,
             expires_at = now() + ($4 * interval '1 second'),
             last_used_at = now(), rotated_at = now()
         WHERE id = $5 AND revoked_at IS NULL",
    )
    .bind(new_hash)
    .bind(&user_agent)
    .bind(&ip)
    .bind(state.jwt_refresh_ttl_secs as i64)
    .bind(session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(user_id),
        "auth.refresh",
        json!({ "session_id": session_id, "ip": ip }),
    )
    .await;

    let res = (StatusCode::OK, Json(json!({ "access": access }))).into_response();
    Ok(with_session_cookie(&state, res, &new_refresh))
}

/// Sign out only the browser/device represented by the current refresh cookie.
pub async fn logout(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2")
        .bind(auth.session_id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.logout",
        json!({ "session_id": auth.session_id }),
    )
    .await;

    let mut res = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    if let Ok(value) = HeaderValue::from_str(&clear_session_cookie()) {
        res.headers_mut().append(SET_COOKIE, value);
    }
    Ok(res)
}

/// List active browser/device sessions for the signed-in account.
pub async fn sessions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(Uuid, String, String, DateTime<Utc>, DateTime<Utc>, DateTime<Utc>)> =
        sqlx::query_as(
            "SELECT id, user_agent, ip, created_at, last_used_at, expires_at
             FROM sessions
             WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
             ORDER BY last_used_at DESC, created_at DESC",
        )
        .bind(auth.user_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let sessions: Vec<Value> = rows
        .into_iter()
        .map(|(id, user_agent, ip, created_at, last_used_at, expires_at)| {
            json!({
                "id": id,
                "device": device_label(&user_agent),
                "user_agent": user_agent,
                "ip": ip,
                "created_at": created_at,
                "last_used_at": last_used_at,
                "expires_at": expires_at,
                "current": auth.session_id == id,
            })
        })
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(session_id): axum::extract::Path<Uuid>,
) -> Result<Response, ApiError> {
    let result = sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("Session not found"));
    }
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.session_revoked",
        json!({ "session_id": session_id }),
    )
    .await;

    let mut res = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    if auth.session_id == session_id {
        if let Ok(value) = HeaderValue::from_str(&clear_session_cookie()) {
            res.headers_mut().append(SET_COOKIE, value);
        }
    }
    Ok(res)
}

pub async fn revoke_other_sessions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let affected = sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(auth.user_id)
    .bind(auth.session_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.sessions_revoke_others",
        json!({ "count": affected }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "revoked": affected })))
}

pub async fn logout_all(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let affected = sqlx::query(
        "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(auth.user_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.logout_all",
        json!({ "count": affected }),
    )
    .await;
    let mut res = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    if let Ok(value) = HeaderValue::from_str(&clear_session_cookie()) {
        res.headers_mut().append(SET_COOKIE, value);
    }
    Ok(res)
}

pub async fn change_password(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ChangePasswordIn>,
) -> Result<Json<Value>, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    let cred_key = format!("password-change:{}|{ip}", auth.user_id);
    state.rate.check_lock(&cred_key).await?;
    state
        .rate
        .check_burst(&format!("password-change-ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;

    if body.current_password.is_empty() || body.new_password.is_empty() {
        return Err(ApiError::bad_request("Current and new password are required"));
    }
    domain::password::validate_password(&body.new_password, &auth.email)?;

    let password_hash: Option<String> = sqlx::query_scalar(
        "SELECT password_hash FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let password_hash = password_hash.ok_or_else(|| ApiError::not_found("Account not found"))?;
    let legacy_mailbox: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.address::text, m.provider_account_id
         FROM users u
         JOIN mailboxes m ON m.id=u.primary_mailbox_id AND m.deleted_at IS NULL
         JOIN organizations o ON o.id=m.organization_id AND o.is_system=TRUE
         WHERE u.id=$1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let parsed = PasswordHash::new(&password_hash).map_err(|e| ApiError::internal(e.to_string()))?;
    if Argon2::default()
        .verify_password(body.current_password.as_bytes(), &parsed)
        .is_err()
    {
        let _ = state
            .rate
            .record_failure(&cred_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::bad_request("Current password is incorrect"));
    }
    state.rate.reset(&cred_key).await?;
    if Argon2::default()
        .verify_password(body.new_password.as_bytes(), &parsed)
        .is_ok()
    {
        return Err(ApiError::bad_request("New password must be different"));
    }

    let salt = gen_salt();
    let hash = Argon2::default()
        .hash_password(body.new_password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .to_string();
    let current_session_id = auth.session_id;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
        .bind(hash)
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let sync_legacy_mailbox = legacy_mailbox.is_some();
    if let Some((mailbox_address, provider_account_id)) = legacy_mailbox {
        state
            .provisioning
            .enqueue_credentials_tx(
                &mut tx,
                auth.user_id,
                &mailbox_address,
                provider_account_id.as_deref(),
                &body.new_password,
            )
            .await?;
    }
    sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(auth.user_id)
    .bind(current_session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "auth.password_changed",
        json!({ "other_sessions_revoked": true, "ip": ip }),
    )
    .await;
    if sync_legacy_mailbox {
        provisioning::process_user_credentials_now(&state, auth.user_id).await;
    }
    Ok(Json(json!({
        "ok": true,
        "message": "Password updated. Other sessions were signed out."
    })))
}

pub fn routes() -> axum::Router<AppState> {
    use axum::routing::post;

    axum::Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/2fa/verify", post(crate::handlers::two_factor::verify_login))
        .route("/api/auth/refresh", post(refresh))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/resend-verification", post(resend_verification))
        .route("/api/auth/forgot", post(forgot_password))
        .route("/api/auth/reset", post(reset_password))
}
