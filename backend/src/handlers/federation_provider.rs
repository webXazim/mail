//! Narrow OAuth authorization-code provider for CS Connect. Codes and bearer
//! tokens are short lived, hashed at rest, single-purpose and scoped to one
//! confidential client. The browser's Mail session proves source ownership.
use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::middleware::cookie::cookie_token;
use crate::state::AppState;

const CONNECT_CALLBACK: &str =
    "https://connect.crescentsphere.com/api/v1/accounts/federation/callback/";

#[derive(Deserialize)]
struct AuthorizeIn {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: String,
    code_challenge: String,
    code_challenge_method: String,
}

#[derive(Deserialize)]
struct TokenIn {
    grant_type: String,
    code: String,
    redirect_uri: String,
    client_id: String,
    client_secret: String,
    code_verifier: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/federation/authorize", get(authorize))
        .route("/api/auth/federation/token", post(token))
        .route("/api/auth/federation/userinfo", get(userinfo))
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn random_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
fn configured(state: &AppState) -> bool {
    state.federation_client_id.is_some()
        && state.federation_client_secret.is_some()
        && state.public_origin.starts_with("https://")
}
fn equal(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<AuthorizeIn>,
) -> Response {
    if !configured(&state)
        || input.response_type != "code"
        || input.client_id != state.federation_client_id.as_deref().unwrap_or_default()
        || input.redirect_uri != CONNECT_CALLBACK
        || input.code_challenge_method != "S256"
        || input.state.len() < 16
        || input.state.len() > 512
        || input.code_challenge.len() != 43
        || !input
            .code_challenge
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || !["openid", "email", "profile"]
            .iter()
            .all(|scope| input.scope.split_whitespace().any(|part| part == *scope))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_request"})),
        )
            .into_response();
    }
    let raw_cookie = cookie_token(&headers).unwrap_or_default();
    let user: Option<(Uuid,)> = sqlx::query_as(
        "SELECT u.id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.revoked_at IS NULL AND s.expires_at>now() AND u.status='active' AND u.email_verified_at IS NOT NULL"
    ).bind(hash(&raw_cookie)).fetch_optional(&state.db).await.unwrap_or(None);
    let Some((user_id,)) = user else {
        let mut next = reqwest::Url::parse(&format!(
            "{}/login",
            state.public_origin.trim_end_matches('/')
        ))
        .unwrap();
        let mut authorize = reqwest::Url::parse(&format!(
            "{}/api/auth/federation/authorize",
            state.public_origin.trim_end_matches('/')
        ))
        .unwrap();
        authorize
            .query_pairs_mut()
            .append_pair("response_type", &input.response_type)
            .append_pair("client_id", &input.client_id)
            .append_pair("redirect_uri", &input.redirect_uri)
            .append_pair("scope", &input.scope)
            .append_pair("state", &input.state)
            .append_pair("code_challenge", &input.code_challenge)
            .append_pair("code_challenge_method", &input.code_challenge_method);
        next.query_pairs_mut().append_pair(
            "return",
            &format!(
                "{}?{}",
                authorize.path(),
                authorize.query().unwrap_or_default()
            ),
        );
        return Redirect::to(next.as_str()).into_response();
    };
    let code = random_secret();
    let result = sqlx::query("INSERT INTO federation_authorization_codes (code_hash,user_id,challenge,redirect_uri,expires_at) VALUES ($1,$2,$3,$4,now()+interval '3 minutes')")
        .bind(hash(&code)).bind(user_id).bind(&input.code_challenge).bind(CONNECT_CALLBACK).execute(&state.db).await;
    if result.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"temporarily_unavailable"})),
        )
            .into_response();
    }
    let mut redirect = reqwest::Url::parse(CONNECT_CALLBACK).unwrap();
    redirect
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &input.state);
    Redirect::to(redirect.as_str()).into_response()
}

async fn token(State(state): State<AppState>, Form(input): Form<TokenIn>) -> Response {
    if !configured(&state)
        || input.grant_type != "authorization_code"
        || input.redirect_uri != CONNECT_CALLBACK
        || input.client_id != state.federation_client_id.as_deref().unwrap_or_default()
        || !equal(
            &input.client_secret,
            state
                .federation_client_secret
                .as_deref()
                .unwrap_or_default(),
        )
        || input.code.len() > 256
        || !(43..=128).contains(&input.code_verifier.len())
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid_client"})),
        )
            .into_response();
    }
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(input.code_verifier.as_bytes()));
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"temporarily_unavailable"})),
            )
                .into_response()
        }
    };
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "UPDATE federation_authorization_codes SET used_at=now() WHERE code_hash=$1 AND used_at IS NULL AND expires_at>now() AND redirect_uri=$2 RETURNING user_id,challenge"
    ).bind(hash(&input.code)).bind(CONNECT_CALLBACK).fetch_optional(&mut *tx).await.unwrap_or(None);
    let Some((user_id, stored_challenge)) = row else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    };
    if !equal(&challenge, &stored_challenge) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    }
    let access = random_secret();
    if sqlx::query("INSERT INTO federation_access_tokens (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '5 minutes')")
        .bind(hash(&access)).bind(user_id).execute(&mut *tx).await.is_err() || tx.commit().await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"temporarily_unavailable"}))).into_response();
    }
    let mut response =
        Json(json!({"access_token":access,"token_type":"Bearer","expires_in":300})).into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !configured(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if token.len() < 32 || token.len() > 256 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let row: Option<(Uuid,String,String)> = sqlx::query_as(
        "SELECT u.id,u.email::text,u.display_name FROM federation_access_tokens t JOIN users u ON u.id=t.user_id WHERE t.token_hash=$1 AND t.expires_at>now() AND u.status='active' AND u.email_verified_at IS NOT NULL"
    ).bind(hash(token)).fetch_optional(&state.db).await.unwrap_or(None);
    let Some((id, email, name)) = row else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut response =
        Json(json!({"sub":id.to_string(),"email":email,"email_verified":true,"name":name}))
            .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

// Only this read-only route permits credentialed presence checks from sister apps.
pub fn presence_routes() -> Router<AppState> {
    Router::new().route("/api/auth/federation/session-account", get(session_presence))
}

async fn session_presence(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let origin = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()).unwrap_or("");
    if !["https://connect.crescentsphere.com", "https://docs.crescentsphere.com", "https://mail.crescentsphere.com", "https://mailer.crescentsphere.com"].contains(&origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let raw_cookie = cookie_token(&headers).unwrap_or_default();
    let signed_in = if configured(&state) && !raw_cookie.is_empty() {
        match sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.revoked_at IS NULL AND s.expires_at>now() AND u.email_verified_at IS NOT NULL AND u.status='active')")
            .bind(hash(&raw_cookie)).fetch_one(&state.db).await {
            Ok(value) => value,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    } else { false };
    let mut response = Json(json!({"signed_in":signed_in})).into_response();
    response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, headers[header::ORIGIN].clone());
    response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true".parse().unwrap());
    response.headers_mut().insert(header::CACHE_CONTROL, "no-store, private".parse().unwrap());
    response.headers_mut().insert(header::VARY, "Origin, Cookie".parse().unwrap());
    response
}
