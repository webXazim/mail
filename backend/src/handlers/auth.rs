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
    client_ip, AUTH_IP_LIMIT, AUTH_IP_WINDOW, CRED_LOCK, CRED_MAX_FAILS, CRED_WINDOW,
    EMAIL_HOURLY_LIMIT, EMAIL_HOURLY_WINDOW,
};
use crate::services::email;
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
pub struct RefreshIn {
    /// Optional legacy body token; new clients rely on the httpOnly cookie.
    #[serde(default)]
    token: String,
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

/// Mint a fresh token pair and persist the refresh half as a session row.
async fn issue_tokens(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    role: &str,
) -> Result<(String, String), ApiError> {
    let (access, refresh) = create_tokens(
        user_id,
        email,
        role,
        &state.jwt_secret,
        state.jwt_access_ttl_secs,
        state.jwt_refresh_ttl_secs,
    );

    let refresh_hash = token_hash(&refresh);

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at)
         VALUES ($1, $2, now() + interval '1 day' * $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(refresh_hash)
    .bind((state.jwt_refresh_ttl_secs / 86400) as i32)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((access, refresh))
}

/// Attach the session cookie (refresh token) to a response.
fn with_session_cookie(state: &AppState, mut res: Response, token: &str) -> Response {
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
type DbLoginUser = (Uuid, String, String, String, String, Option<DateTime<Utc>>);
/// Profile fields for verify/resend lookups.
type DbProfileUser = (Uuid, String, String, String, Option<DateTime<Utc>>);

fn user_json(id: Uuid, email: &str, display_name: &str, role: &str, email_verified: bool) -> Value {
    json!({
        "id": id,
        "email": email,
        "display_name": display_name,
        "role": role,
        "email_verified": email_verified
    })
}

pub async fn register(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<RegisterIn>,
) -> Result<Response, ApiError> {
    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;

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

    let user: (Uuid, String, String) = sqlx::query_as(
        "INSERT INTO users (email, display_name, password_hash) VALUES ($1, $2, $3) RETURNING id, email, display_name",
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
        json!({ "email": user.1 }),
    )
    .await;

    // Provision the mailbox in Stalwart (fail-open: a bridge outage must not
    // block signup; mailbox lands the moment the bridge is healthy again).
    // Future hardening: only provision after the account is email-verified.
    // Persist the account id so the WS2 read path can target this user's mail.
    match state.mail.ensure_mailbox(&user.1, &body.password).await {
        Ok(Some(account_id)) => {
            let _ = sqlx::query("UPDATE users SET mail_account_id = $1 WHERE id = $2")
                .bind(&account_id)
                .bind(user.0)
                .execute(&state.db)
                .await;
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(email = %user.1, "mailbox provisioning failed: {e}"),
    }

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

    // Development path (HARBOR_REQUIRE_VERIFICATION=false): behave like the
    // pre-verification API and issue a session immediately.
    sqlx::query(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
    )
    .bind(user.0)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (access, refresh) = issue_tokens(&state, user.0, &user.1, "member").await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "refresh": refresh,
            "user": user_json(user.0, &user.1, &user.2, "member", true)
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
    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;

    let email = body.email.trim().to_lowercase();
    if email.is_empty() || body.password.is_empty() {
        return Err(ApiError::bad_request("Email and password are required"));
    }

    let cred_key = format!("cred:{email}|{ip}");

    // While a lock is active, refuse before touching the DB or running argon2.
    state.rate.check_lock(&cred_key)?;

    let row: Option<DbLoginUser> = sqlx::query_as(
        "SELECT id, email, display_name, password_hash, role, email_verified_at
             FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (user_id, email, display_name, password_hash, role, email_verified_at) = match row {
        Some(row) => row,
        None => {
            let _ = state
                .rate
                .record_failure(&cred_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK);
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
            .record_failure(&cred_key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK);
        return Err(ApiError::unauthorized("Invalid email or password"));
    }

    state.rate.reset(&cred_key);

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

    audit::record(
        &state,
        Some(user_id),
        "auth.login",
        json!({ "email": &email }),
    )
    .await;

    let (access, refresh) = issue_tokens(&state, user_id, &email, &role).await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "refresh": refresh,
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
    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;

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
        "SELECT id, email, display_name, role, email_verified_at
         FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (user_id, email, display_name, role, already) =
        user.ok_or_else(|| ApiError::internal("Verification target user no longer exists"))?;

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

    let (access, refresh) = issue_tokens(&state, user_id, &email, &role).await?;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "refresh": refresh,
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

    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;
    state.rate.check_burst(
        &format!("email:{email}"),
        EMAIL_HOURLY_LIMIT,
        EMAIL_HOURLY_WINDOW,
    )?;

    let user: Option<(Uuid, String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, email, display_name, email_verified_at FROM users WHERE email = $1",
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

    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;
    state.rate.check_burst(
        &format!("email:{email}"),
        EMAIL_HOURLY_LIMIT,
        EMAIL_HOURLY_WINDOW,
    )?;

    let user: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, email, display_name FROM users
         WHERE email = $1 AND email_verified_at IS NOT NULL",
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
    let ip = client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW)?;

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
        return Err(ApiError::bad_request(
            "This reset link has already been used",
        ));
    }

    let salt = gen_salt();
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .to_string();

    // New password + revoke every session immediately.
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(user_id), "auth.password_reset", json!({})).await;

    Ok(Json(json!({
        "ok": true,
        "message": "Password updated. You can now sign in."
    })))
}

pub async fn refresh(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<RefreshIn>,
) -> Result<Response, ApiError> {
    use jsonwebtoken::{decode, DecodingKey, Validation};

    if !origin_allowed(&state, &headers) {
        return Err(ApiError::forbidden("Cross-site request blocked"));
    }

    // Prefer the cookie; fall back to a body token sent by older frontends.
    let refresh_token = match cookie_token(&headers) {
        Some(t) => t,
        None if !body.token.is_empty() => body.token.clone(),
        None => return Err(ApiError::unauthorized("No refresh token provided")),
    };

    let token_data = decode::<Claims>(
        &refresh_token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::unauthorized("Invalid refresh token"))?;

    if token_data.claims.kind != "refresh" {
        return Err(ApiError::unauthorized("Invalid token type"));
    }

    let hash = token_hash(&refresh_token);
    let user_id = token_data.claims.sub;

    // Presence check *before* consumption: a missing row means the token was
    // already rotated, expired, or never existed — treat as possible replay
    // and revoke the whole session family.
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM sessions
         WHERE user_id = $1 AND token_hash = $2 AND expires_at > now()",
    )
    .bind(user_id)
    .bind(&hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if existing.is_none() {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        audit::record(
            &state,
            Some(user_id),
            "auth.refresh.reuse_revoked",
            json!({}),
        )
        .await;
        return Err(ApiError::unauthorized(
            "Refresh token already used or expired",
        ));
    }

    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash = $2")
        .bind(user_id)
        .bind(&hash)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let (access, refresh) = issue_tokens(
        &state,
        user_id,
        &token_data.claims.email,
        &token_data.claims.role,
    )
    .await?;

    audit::record(&state, Some(user_id), "auth.refresh", json!({})).await;

    let res = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "refresh": refresh
        })),
    )
        .into_response();
    Ok(with_session_cookie(&state, res, &refresh))
}

pub async fn logout(State(state): State<AppState>, auth: AuthUser) -> Result<Response, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "auth.logout", json!({})).await;

    let mut res = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    if let Ok(value) = HeaderValue::from_str(&clear_session_cookie()) {
        res.headers_mut().append(SET_COOKIE, value);
    }
    Ok(res)
}

pub fn routes() -> axum::Router<AppState> {
    use axum::routing::post;

    axum::Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/refresh", post(refresh))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/resend-verification", post(resend_verification))
        .route("/api/auth/forgot", post(forgot_password))
        .route("/api/auth/reset", post(reset_password))
}
