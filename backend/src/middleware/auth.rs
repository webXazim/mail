use axum::async_trait;
use axum::extract::FromRef;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::extract::Request;
use axum::middleware::Next;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ErrResult;
use crate::middleware::cookie::cookie_token;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub sid: Uuid,
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
    pub session_id: Uuid,
    pub email: String,
    pub role: String,
    /// Optional request-scoped business context. Values are untrusted hints until
    /// services::tenancy validates membership and mailbox assignment.
    pub organization_id_hint: Option<Uuid>,
    pub mailbox_id_hint: Option<Uuid>,
}

#[derive(Debug)]
pub enum AuthError {
    Missing,
    Invalid,
    Expired,
    Forbidden,
    Suspended,
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
            AuthError::Suspended => (
                StatusCode::FORBIDDEN,
                "account_suspended",
                "This account is suspended",
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

        // Authorization is resolved from current server state on every
        // protected request. A demotion or suspension therefore takes effect
        // immediately instead of waiting for an access token to expire.
        let current: Option<(String, String, String)> = sqlx::query_as(
            "SELECT u.email::text, u.platform_role, u.status
             FROM sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.id = $1 AND s.user_id = $2
               AND s.revoked_at IS NULL AND s.expires_at > now()",
        )
        .bind(token_data.claims.sid)
        .bind(token_data.claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| AuthError::Invalid)?;
        let (email, role, status) = current.ok_or(AuthError::Invalid)?;
        if status != "active" {
            return Err(AuthError::Suspended);
        }

        let organization_id_hint = parts.headers
            .get("x-cs-organization-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok());
        let mailbox_id_hint = parts.headers
            .get("x-cs-mailbox-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok());

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            session_id: token_data.claims.sid,
            email,
            role,
            organization_id_hint,
            mailbox_id_hint,
        })
    }
}

/// Platform-admin requests must arrive through the localhost-only admin reverse
/// proxy. Public Nginx strips this header and blocks `/api/admin/*`; the API
/// itself is loopback-bound in production. Keeping the check here makes an
/// accidental future proxy regression fail closed instead of exposing the
/// platform-control plane to the Internet.
pub const ADMIN_LOCAL_HEADER: &str = "x-cs-admin-local";

pub fn is_local_admin_request(headers: &HeaderMap) -> bool {
    headers
        .get(ADMIN_LOCAL_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "1")
}

/// Route-level defense in depth for every `/api/admin` endpoint, including any
/// future handler that might accidentally omit the `AdminUser` extractor.
pub async fn local_admin_gate(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if (path == "/api/admin" || path.starts_with("/api/admin/"))
        && !is_local_admin_request(request.headers())
    {
        return AuthError::Forbidden.into_response();
    }
    next.run(request).await
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
        if !is_local_admin_request(&parts.headers) {
            return Err(AuthError::Forbidden);
        }
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != "platform_admin" {
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

/// Authenticate a WebSocket / polling client from the HttpOnly refresh cookie.
/// Unlike the old decoder-only path, this verifies that the concrete session
/// has not been revoked and resolves the current role/status from PostgreSQL.
pub async fn user_from_cookie(state: &AppState, headers: &HeaderMap) -> Option<AuthUser> {
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
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let current: Option<(String, String, String)> = sqlx::query_as(
        "SELECT u.email::text, u.platform_role, u.status
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.id = $1 AND s.user_id = $2 AND s.token_hash = $3
           AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(data.claims.sid)
    .bind(data.claims.sub)
    .bind(hash)
    .fetch_optional(&state.db)
    .await
    .ok()?;
    let (email, role, status) = current?;
    if status != "active" {
        return None;
    }
    Some(AuthUser {
        user_id: data.claims.sub,
        session_id: data.claims.sid,
        email,
        role,
        organization_id_hint: headers
            .get("x-cs-organization-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok()),
        mailbox_id_hint: headers
            .get("x-cs-mailbox-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok()),
    })
}

pub fn create_tokens(
    user_id: Uuid,
    session_id: Uuid,
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
        sid: session_id,
        email: email.into(),
        role: role.into(),
        iat: now,
        exp: now + access_ttl_secs as usize,
        kind: "access".into(),
        jti: uuid::Uuid::new_v4().to_string(),
    };

    let refresh_claims = Claims {
        sub: user_id,
        sid: session_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_pair_shares_stable_session_id_and_has_distinct_jti() {
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let secret = "test-secret-long-enough-for-unit-test";
        let (access, refresh) = create_tokens(
            user_id,
            session_id,
            "alice@example.com",
            "member",
            secret,
            300,
            3600,
        );
        let access_claims = decode::<Claims>(
            &access,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .expect("access token decodes")
        .claims;
        let refresh_claims = decode::<Claims>(
            &refresh,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .expect("refresh token decodes")
        .claims;

        assert_eq!(access_claims.sub, user_id);
        assert_eq!(access_claims.sid, session_id);
        assert_eq!(refresh_claims.sid, session_id);
        assert_eq!(access_claims.kind, "access");
        assert_eq!(refresh_claims.kind, "refresh");
        assert_ne!(access_claims.jti, refresh_claims.jti);
    }
}
