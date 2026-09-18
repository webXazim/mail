use axum::async_trait;
use axum::extract::FromRef;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ErrResult;
use crate::middleware::cookie::cookie_token;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
    pub kind: String,
    /// Unique per mint. Without it, two tokens minted inside the same second
    /// are byte-identical (same iat/exp), which would mask refresh rotation.
    pub jti: String,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
    pub role: String,
}

#[derive(Debug)]
pub enum AuthError {
    Missing,
    Invalid,
    Expired,
    Forbidden,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            AuthError::Missing => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing authorization header",
            ),
            AuthError::Invalid => (StatusCode::UNAUTHORIZED, "unauthorized", "Invalid token"),
            AuthError::Expired => (StatusCode::UNAUTHORIZED, "unauthorized", "Token expired"),
            AuthError::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "Administrator access required",
            ),
        };
        let body = Json(ErrResult {
            error: error.into(),
            message: message.into(),
        });
        (status, body).into_response()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);

        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::Missing)?;

        let token = header.strip_prefix("Bearer ").ok_or(AuthError::Invalid)?;

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::Expired,
            _ => AuthError::Invalid,
        })?;

        // Refresh tokens are signed with the same key; they must never be
        // accepted as bearer access tokens.
        if token_data.claims.kind != "access" {
            return Err(AuthError::Invalid);
        }

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            email: token_data.claims.email,
            role: token_data.claims.role,
        })
    }
}

/// Authenticated caller whose role is `admin`. Rejects members with 403 so the
/// client-side hide is never the only line of defence.
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

#[async_trait]
impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != "admin" {
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

/// Authenticate a WebSocket / polling client from the HttpOnly session cookie.
/// Browsers cannot set an `Authorization` header on a WS handshake, and putting
/// an access token in the query string would leak it into logs and history, so
/// the same-origin refresh cookie is the transport for realtime (WS2.6).
pub fn user_from_cookie(state: &AppState, headers: &HeaderMap) -> Option<AuthUser> {
    let token = cookie_token(headers)?;
    let data = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    if data.claims.kind != "refresh" {
        return None;
    }
    Some(AuthUser {
        user_id: data.claims.sub,
        email: data.claims.email,
        role: data.claims.role,
    })
}

pub fn create_tokens(
    user_id: Uuid,
    email: &str,
    role: &str,
    secret: &str,
    access_ttl_secs: u64,
    refresh_ttl_secs: u64,
) -> (String, String) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let access_claims = Claims {
        sub: user_id,
        email: email.into(),
        role: role.into(),
        iat: now,
        exp: now + access_ttl_secs as usize,
        kind: "access".into(),
        jti: uuid::Uuid::new_v4().to_string(),
    };

    let refresh_claims = Claims {
        sub: user_id,
        email: email.into(),
        role: role.into(),
        iat: now,
        exp: now + refresh_ttl_secs as usize,
        kind: "refresh".into(),
        jti: uuid::Uuid::new_v4().to_string(),
    };

    let access = encode_jwt(&access_claims, secret);
    let refresh = encode_jwt(&refresh_claims, secret);

    (access, refresh)
}

fn encode_jwt(claims: &Claims, secret: &str) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT encoding should not fail")
}
