use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::auth::{
    issue_tokens, session_user_agent, user_json, with_session_cookie,
};
use crate::middleware::auth::AuthUser;
use crate::middleware::rate_limit::{
    AUTH_IP_LIMIT, AUTH_IP_WINDOW, CRED_LOCK, CRED_MAX_FAILS, CRED_WINDOW,
};
use crate::services::two_factor as totp;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginVerifyIn {
    challenge_token: String,
    code: String,
}

#[derive(Deserialize)]
pub struct SetupIn {
    current_password: String,
}

#[derive(Deserialize)]
pub struct ConfirmIn {
    code: String,
}

#[derive(Deserialize)]
pub struct ProtectedFactorIn {
    current_password: String,
    code: String,
}

pub(crate) async fn create_login_challenge(
    state: &AppState,
    user_id: Uuid,
    ip: &str,
    user_agent: &str,
) -> Result<String, ApiError> {
    let token = totp::random_challenge_token();
    let token_hash = totp::token_hash(&token);
    let challenge_id = Uuid::new_v4();

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Only the newest password-authenticated challenge should remain usable.
    sqlx::query(
        "UPDATE two_factor_challenges
         SET consumed_at = COALESCE(consumed_at, now())
         WHERE user_id = $1 AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO two_factor_challenges
           (id, user_id, token_hash, ip, user_agent, max_attempts, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + ($7 * interval '1 minute'))",
    )
    .bind(challenge_id)
    .bind(user_id)
    .bind(token_hash)
    .bind(ip)
    .bind(user_agent)
    .bind(totp::CHALLENGE_MAX_ATTEMPTS)
    .bind(totp::CHALLENGE_TTL_MINUTES)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(token)
}

async fn current_password_ok(
    state: &AppState,
    user_id: Uuid,
    current_password: &str,
) -> Result<bool, ApiError> {
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let hash = hash.ok_or_else(|| ApiError::not_found("Account not found"))?;
    let parsed = PasswordHash::new(&hash).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(current_password.as_bytes(), &parsed)
        .is_ok())
}

async fn active_secret(state: &AppState, user_id: Uuid) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar(
        "SELECT CASE WHEN totp_secret_ciphertext IS NULL THEN NULL
                     ELSE pgp_sym_decrypt(totp_secret_ciphertext, $2) END
         FROM users WHERE id = $1 AND two_factor_enabled_at IS NOT NULL",
    )
    .bind(user_id)
    .bind(&state.two_factor_key)
    .fetch_optional(&state.db)
    .await
    .map(|row: Option<Option<String>>| row.flatten())
    .map_err(|e| ApiError::internal(e.to_string()))
}

/// Verify a TOTP or one-time recovery code. Recovery-code consumption happens
/// transactionally; a successful TOTP leaves recovery codes untouched.
async fn verify_factor_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    secret: &str,
    code: &str,
) -> Result<(bool, bool), ApiError> {
    if let Some(step) = totp::matching_totp_step(secret, code)? {
        // RFC 6238 codes are short-lived bearer secrets. Accept a time-step
        // only once per account so a captured code cannot be replayed during
        // the remainder of the same 30-second window. Lock the user row so
        // concurrent verification attempts cannot both consume the same step.
        let last_used: Option<i64> = sqlx::query_scalar(
            "SELECT totp_last_used_step FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let step = i64::try_from(step)
            .map_err(|_| ApiError::internal("Two-factor time step is invalid"))?;
        if last_used.is_some_and(|last| step <= last) {
            return Ok((false, false));
        }
        sqlx::query("UPDATE users SET totp_last_used_step = $2 WHERE id = $1")
            .bind(user_id)
            .bind(step)
            .execute(&mut **tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok((true, false));
    }

    let Some(prefix) = totp::recovery_prefix(code) else {
        return Ok((false, false));
    };
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, code_hash
         FROM two_factor_recovery_codes
         WHERE user_id = $1 AND lookup_prefix = $2 AND used_at IS NULL
         FOR UPDATE",
    )
    .bind(user_id)
    .bind(prefix)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let Some((id, code_hash)) = row else {
        return Ok((false, false));
    };
    if !totp::verify_recovery_code(&code_hash, code)? {
        return Ok((false, false));
    }
    sqlx::query(
        "UPDATE two_factor_recovery_codes SET used_at = now()
         WHERE id = $1 AND used_at IS NULL",
    )
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((true, true))
}

pub async fn verify_login(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<LoginVerifyIn>,
) -> Result<Response, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    state
        .rate
        .check_burst(&format!("2fa-ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;
    if body.challenge_token.trim().is_empty() || body.code.trim().is_empty() {
        return Err(ApiError::bad_request("Authentication code is required"));
    }

    let token_hash = totp::token_hash(body.challenge_token.trim());
    let lock_key = format!("2fa:{token_hash}|{ip}");
    state.rate.check_lock(&lock_key).await?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    type ChallengeRow = (
        Uuid,
        Uuid,
        i32,
        i32,
        String,
        String,
        String,
        String,
        Option<DateTime<Utc>>,
        Option<String>,
        String,
        String,
    );
    let row: Option<ChallengeRow> = sqlx::query_as(
        "SELECT c.id, c.user_id, c.attempts, c.max_attempts,
                u.email::text, u.display_name, u.platform_role, u.status, u.email_verified_at,
                CASE WHEN u.totp_secret_ciphertext IS NULL THEN NULL
                     ELSE pgp_sym_decrypt(u.totp_secret_ciphertext, $2) END,
                c.ip, c.user_agent
         FROM two_factor_challenges c
         JOIN users u ON u.id = c.user_id
         WHERE c.token_hash = $1 AND c.consumed_at IS NULL AND c.expires_at > now()
         FOR UPDATE OF c",
    )
    .bind(&token_hash)
    .bind(&state.two_factor_key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let Some((
        challenge_id,
        user_id,
        attempts,
        max_attempts,
        email,
        display_name,
        role,
        status,
        verified_at,
        secret,
        challenge_ip,
        challenge_user_agent,
    )) = row else {
        return Err(ApiError::unauthorized(
            "Two-factor challenge is invalid or expired. Sign in again.",
        ));
    };

    let current_user_agent = session_user_agent(&headers);
    if challenge_ip != ip || challenge_user_agent != current_user_agent {
        audit::record(
            &state,
            Some(user_id),
            "auth.two_factor_context_mismatch",
            json!({ "ip": ip }),
        )
        .await;
        return Err(ApiError::unauthorized(
            "Two-factor challenge is invalid or expired. Sign in again.",
        ));
    }

    if status != "active" || secret.as_deref().unwrap_or_default().is_empty() {
        sqlx::query("UPDATE two_factor_challenges SET consumed_at = now() WHERE id = $1")
            .bind(challenge_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(ApiError::forbidden("This account cannot complete two-factor sign-in"));
    }

    let (valid, used_recovery) = verify_factor_tx(
        &mut tx,
        user_id,
        secret.as_deref().unwrap_or_default(),
        body.code.trim(),
    )
    .await?;
    if !valid {
        let next_attempts = attempts + 1;
        sqlx::query(
            "UPDATE two_factor_challenges
             SET attempts = $2,
                 consumed_at = CASE WHEN $2 >= max_attempts THEN now() ELSE consumed_at END
             WHERE id = $1",
        )
        .bind(challenge_id)
        .bind(next_attempts)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let _ = state
            .rate
            .record_failure(&lock_key, max_attempts as u32, CRED_WINDOW, CRED_LOCK).await;
        audit::record(
            &state,
            Some(user_id),
            "auth.two_factor_failed",
            json!({ "ip": ip, "attempt": next_attempts }),
        )
        .await;
        return Err(ApiError::unauthorized("Invalid authentication code"));
    }

    sqlx::query("UPDATE two_factor_challenges SET consumed_at = now() WHERE id = $1")
        .bind(challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    state.rate.reset(&lock_key).await?;

    let (access, refresh) = issue_tokens(
        &state,
        user_id,
        &email,
        &role,
        &current_user_agent,
        &ip,
    )
    .await?;
    audit::record(
        &state,
        Some(user_id),
        "auth.login",
        json!({ "email": email, "two_factor": true, "recovery_code": used_recovery, "ip": ip }),
    )
    .await;

    let response = (
        StatusCode::OK,
        Json(json!({
            "access": access,
            "user": user_json(user_id, &email, &display_name, &role, verified_at.is_some()),
            "recovery_code_used": used_recovery
        })),
    )
        .into_response();
    Ok(with_session_cookie(&state, response, &refresh))
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT two_factor_enabled_at, totp_pending_expires_at FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (enabled_at, pending_expires_at) =
        row.ok_or_else(|| ApiError::not_found("Account not found"))?;
    let recovery_remaining: i64 = if enabled_at.is_some() {
        sqlx::query_scalar(
            "SELECT count(*) FROM two_factor_recovery_codes
             WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(auth.user_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        0
    };
    Ok(Json(json!({
        "enabled": enabled_at.is_some(),
        "enabled_at": enabled_at,
        "recovery_codes_remaining": recovery_remaining,
        "setup_pending_until": pending_expires_at.filter(|v| *v > Utc::now())
    })))
}

pub async fn setup(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetupIn>,
) -> Result<Json<Value>, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    let key = format!("2fa-setup:{}|{ip}", auth.user_id);
    state.rate.check_lock(&key).await?;
    state
        .rate
        .check_burst(&format!("2fa-setup-ip:{ip}"), AUTH_IP_LIMIT, AUTH_IP_WINDOW).await?;
    if !current_password_ok(&state, auth.user_id, &body.current_password).await? {
        let _ = state
            .rate
            .record_failure(&key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Current password is incorrect"));
    }
    state.rate.reset(&key).await?;

    let enabled: Option<(bool,)> = sqlx::query_as(
        "SELECT two_factor_enabled_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if enabled.is_some_and(|row| row.0) {
        return Err(ApiError::conflict("Two-factor authentication is already enabled"));
    }

    let secret = totp::generate_secret();
    let expires_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE users
         SET totp_pending_ciphertext = pgp_sym_encrypt($2, $3, 'cipher-algo=aes256, compress-algo=0'),
             totp_pending_expires_at = now() + ($4 * interval '1 minute'), updated_at = now()
         WHERE id = $1
         RETURNING totp_pending_expires_at",
    )
    .bind(auth.user_id)
    .bind(&secret)
    .bind(&state.two_factor_key)
    .bind(totp::SETUP_TTL_MINUTES)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let otpauth_uri = totp::otpauth_uri(&auth.email, &secret);
    let qr_svg = {
        use qrcode::render::svg;
        let code = qrcode::QrCode::new(otpauth_uri.as_bytes())
            .map_err(|_| ApiError::internal("Could not generate two-factor QR code"))?;
        code.render::<svg::Color>()
            .min_dimensions(220, 220)
            .dark_color(svg::Color("#111111"))
            .light_color(svg::Color("#ffffff"))
            .build()
    };
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.two_factor_setup_started",
        json!({ "ip": ip }),
    )
    .await;
    Ok(Json(json!({
        "secret": secret,
        "otpauth_uri": otpauth_uri,
        "qr_svg": qr_svg,
        "expires_at": expires_at
    })))
}

pub async fn confirm(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ConfirmIn>,
) -> Result<Json<Value>, ApiError> {
    let secret: Option<String> = sqlx::query_scalar(
        "SELECT CASE
             WHEN totp_pending_expires_at > now() AND totp_pending_ciphertext IS NOT NULL
             THEN pgp_sym_decrypt(totp_pending_ciphertext, $2)
             ELSE NULL END
         FROM users WHERE id = $1 AND two_factor_enabled_at IS NULL",
    )
    .bind(auth.user_id)
    .bind(&state.two_factor_key)
    .fetch_optional(&state.db)
    .await
    .map(|row: Option<Option<String>>| row.flatten())
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let secret = secret.ok_or_else(|| ApiError::bad_request("Two-factor setup expired. Start again."))?;
    if !totp::verify_totp(&secret, &body.code)? {
        return Err(ApiError::unauthorized("Invalid authentication code"));
    }

    let codes = totp::generate_recovery_codes();
    let mut hashed = Vec::with_capacity(codes.len());
    for code in &codes {
        hashed.push((
            totp::recovery_prefix(code).expect("generated recovery code has prefix"),
            totp::hash_recovery_code(code)?,
        ));
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let updated = sqlx::query(
        "UPDATE users
         SET totp_secret_ciphertext = totp_pending_ciphertext,
             totp_pending_ciphertext = NULL,
             totp_pending_expires_at = NULL,
             two_factor_enabled_at = now(), updated_at = now()
         WHERE id = $1 AND two_factor_enabled_at IS NULL
           AND totp_pending_ciphertext IS NOT NULL AND totp_pending_expires_at > now()",
    )
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::conflict("Two-factor setup changed. Start again."));
    }

    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    for (prefix, hash) in &hashed {
        sqlx::query(
            "INSERT INTO two_factor_recovery_codes (id, user_id, lookup_prefix, code_hash)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(auth.user_id)
        .bind(prefix)
        .bind(hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(auth.user_id)
    .bind(auth.session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(
        &state,
        Some(auth.user_id),
        "auth.two_factor_enabled",
        json!({ "other_sessions_revoked": true }),
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "recovery_codes": codes,
        "message": "Two-factor authentication is enabled. Store the recovery codes securely."
    })))
}

pub async fn disable(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProtectedFactorIn>,
) -> Result<Json<Value>, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    let key = format!("2fa-disable:{}|{ip}", auth.user_id);
    state.rate.check_lock(&key).await?;
    if !current_password_ok(&state, auth.user_id, &body.current_password).await? {
        let _ = state
            .rate
            .record_failure(&key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Current password is incorrect"));
    }
    let secret = active_secret(&state, auth.user_id)
        .await?
        .ok_or_else(|| ApiError::bad_request("Two-factor authentication is not enabled"))?;

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (valid, used_recovery) = verify_factor_tx(&mut tx, auth.user_id, &secret, &body.code).await?;
    if !valid {
        tx.rollback().await.ok();
        let _ = state.rate.record_failure(&key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Invalid authentication code"));
    }
    state.rate.reset(&key).await?;
    sqlx::query(
        "UPDATE users
         SET two_factor_enabled_at = NULL, totp_secret_ciphertext = NULL,
             totp_pending_ciphertext = NULL, totp_pending_expires_at = NULL,
             totp_last_used_step = NULL, updated_at = now()
         WHERE id = $1",
    )
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("DELETE FROM two_factor_challenges WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(auth.user_id)
    .bind(auth.session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.two_factor_disabled",
        json!({ "recovery_code": used_recovery, "ip": ip }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "message": "Two-factor authentication disabled" })))
}

pub async fn regenerate_recovery_codes(
    headers: HeaderMap,
    connect: ConnectInfo<std::net::SocketAddr>,
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProtectedFactorIn>,
) -> Result<Json<Value>, ApiError> {
    let ip = state.rate.client_ip(&headers, Some(connect.0));
    let key = format!("2fa-recovery:{}|{ip}", auth.user_id);
    state.rate.check_lock(&key).await?;
    if !current_password_ok(&state, auth.user_id, &body.current_password).await? {
        let _ = state.rate.record_failure(&key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Current password is incorrect"));
    }
    let secret = active_secret(&state, auth.user_id)
        .await?
        .ok_or_else(|| ApiError::bad_request("Two-factor authentication is not enabled"))?;

    let codes = totp::generate_recovery_codes();
    let mut hashed = Vec::with_capacity(codes.len());
    for code in &codes {
        hashed.push((
            totp::recovery_prefix(code).expect("generated recovery code has prefix"),
            totp::hash_recovery_code(code)?,
        ));
    }

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (valid, used_recovery) = verify_factor_tx(&mut tx, auth.user_id, &secret, &body.code).await?;
    if !valid {
        tx.rollback().await.ok();
        let _ = state.rate.record_failure(&key, CRED_MAX_FAILS, CRED_WINDOW, CRED_LOCK).await;
        return Err(ApiError::unauthorized("Invalid authentication code"));
    }
    state.rate.reset(&key).await?;
    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    for (prefix, hash) in &hashed {
        sqlx::query(
            "INSERT INTO two_factor_recovery_codes (id, user_id, lookup_prefix, code_hash)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(auth.user_id)
        .bind(prefix)
        .bind(hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(auth.user_id),
        "auth.two_factor_recovery_regenerated",
        json!({ "recovery_code_used_for_authorization": used_recovery, "ip": ip }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "recovery_codes": codes })))
}
