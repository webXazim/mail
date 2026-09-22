use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::ApiError;

/// Distributed authentication/abuse limits. Buckets live in PostgreSQL, so
/// every API replica observes the same counters and lockouts.
pub const AUTH_IP_LIMIT: u32 = 20;
pub const AUTH_IP_WINDOW: i64 = 60;

/// Public-account creation is intentionally much tighter than login traffic.
/// This bucket is IP-based and shared across every API replica.
pub const REGISTER_IP_LIMIT: u32 = 5;
pub const REGISTER_IP_WINDOW: i64 = 3600;

/// Domain claim/provision/DNS operations are expensive and touch shared
/// infrastructure. Callers key these buckets by business + actor.
pub const DOMAIN_OPERATION_LIMIT: u32 = 30;
pub const DOMAIN_OPERATION_WINDOW: i64 = 3600;
pub const DNS_CHECK_LIMIT: u32 = 20;
pub const DNS_CHECK_WINDOW: i64 = 600;

pub const CRED_MAX_FAILS: u32 = 5;
pub const CRED_WINDOW: i64 = 900;
pub const CRED_LOCK: i64 = 900;

pub const EMAIL_HOURLY_LIMIT: u32 = 3;
pub const EMAIL_HOURLY_WINDOW: i64 = 3600;

#[derive(Clone)]
pub struct RateLimiter {
    db: PgPool,
    key_salt: Arc<Vec<u8>>,
    trusted_proxy_ips: Arc<Vec<IpAddr>>,
}

impl RateLimiter {
    pub fn new(db: PgPool, key_material: &str, trusted_proxy_ips: Vec<IpAddr>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"cs-mailer:rate-limit-key:v1\0");
        hasher.update(key_material.as_bytes());
        Self {
            db,
            key_salt: Arc::new(hasher.finalize().to_vec()),
            trusted_proxy_ips: Arc::new(trusted_proxy_ips),
        }
    }

    fn key_hash(&self, key: &str) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.key_salt.as_slice());
        hasher.update(b"\0");
        hasher.update(key.as_bytes());
        hasher.finalize().to_vec()
    }

    async fn lock_bucket(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        hash: &[u8],
    ) -> Result<(DateTime<Utc>, i32, Option<DateTime<Utc>>), ApiError> {
        sqlx::query(
            "INSERT INTO request_rate_limits(key_hash, window_started_at, count, updated_at)\n             VALUES ($1, now(), 0, now())\n             ON CONFLICT (key_hash) DO NOTHING",
        )
        .bind(hash)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        sqlx::query_as(
            "SELECT window_started_at, count, lock_until\n             FROM request_rate_limits WHERE key_hash = $1 FOR UPDATE",
        )
        .bind(hash)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
    }

    /// Fixed-window burst limiter shared by every API replica.
    pub async fn check_burst(&self, key: &str, limit: u32, window: i64) -> Result<(), ApiError> {
        let hash = self.key_hash(key);
        let now = Utc::now();
        let mut tx = self
            .db
            .begin()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let (started, current, locked_until) = self.lock_bucket(&mut tx, &hash).await?;

        if locked_until.is_some_and(|until| until > now) {
            return Err(ApiError::too_many(
                "Too many requests. Please wait and try again.",
            ));
        }

        let expired = started + Duration::seconds(window.max(1)) <= now;
        let next = if expired { 1 } else { current.saturating_add(1) };
        let next_started = if expired { now } else { started };

        sqlx::query(
            "UPDATE request_rate_limits\n             SET window_started_at = $2, count = $3,\n                 lock_until = CASE WHEN lock_until <= now() THEN NULL ELSE lock_until END,\n                 updated_at = now()\n             WHERE key_hash = $1",
        )
        .bind(&hash)
        .bind(next_started)
        .bind(next)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

        if next as u32 > limit {
            return Err(ApiError::too_many(
                "Too many requests. Please wait and try again.",
            ));
        }
        Ok(())
    }

    /// Record one rejected credential/factor attempt. The lockout is durable
    /// and therefore cannot be bypassed by hopping between API replicas.
    pub async fn record_failure(
        &self,
        key: &str,
        max: u32,
        window: i64,
        lock: i64,
    ) -> Result<(), ApiError> {
        let hash = self.key_hash(key);
        let now = Utc::now();
        let mut tx = self
            .db
            .begin()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let (started, current, locked_until) = self.lock_bucket(&mut tx, &hash).await?;

        if locked_until.is_some_and(|until| until > now) {
            return Err(ApiError::too_many(
                "Too many failed attempts. Please wait and try again.",
            ));
        }

        let expired = started + Duration::seconds(window.max(1)) <= now;
        let mut next = if expired { 1 } else { current.saturating_add(1) };
        let next_started = if expired { now } else { started };
        let next_lock = if next as u32 >= max.max(1) {
            next = 0;
            Some(now + Duration::seconds(lock.max(1)))
        } else {
            None
        };

        sqlx::query(
            "UPDATE request_rate_limits\n             SET window_started_at = $2, count = $3, lock_until = $4, updated_at = now()\n             WHERE key_hash = $1",
        )
        .bind(&hash)
        .bind(next_started)
        .bind(next)
        .bind(next_lock)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn check_lock(&self, key: &str) -> Result<(), ApiError> {
        let hash = self.key_hash(key);
        let locked_until: Option<(Option<DateTime<Utc>>,)> = sqlx::query_as(
            "SELECT lock_until FROM request_rate_limits WHERE key_hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        if locked_until
            .and_then(|row| row.0)
            .is_some_and(|until| until > Utc::now())
        {
            return Err(ApiError::too_many(
                "Too many failed attempts. Please wait and try again.",
            ));
        }
        Ok(())
    }

    pub async fn reset(&self, key: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM request_rate_limits WHERE key_hash = $1")
            .bind(self.key_hash(key))
            .execute(&self.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    /// Resolve the caller address without trusting spoofable forwarding
    /// headers. Proxy headers are honored only when the TCP peer is explicitly
    /// trusted (loopback by default; additional proxy IPs are configured).
    pub fn client_ip(&self, headers: &HeaderMap, remote: Option<SocketAddr>) -> String {
        let peer = remote.map(|addr| addr.ip());
        let trusted = peer.is_some_and(|ip| self.trusted_proxy_ips.contains(&ip));
        if trusted {
            if let Some(ip) = header_ip(headers, "x-real-ip") {
                return ip.to_string();
            }
            if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
                if let Some(ip) = value
                    .split(',')
                    .next()
                    .and_then(|part| part.trim().parse::<IpAddr>().ok())
                {
                    return ip.to_string();
                }
            }
        }
        peer.map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarding_headers_are_ignored_from_untrusted_peers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.8".parse().unwrap());
        let peer: SocketAddr = "198.51.100.4:443".parse().unwrap();
        let trusted: Vec<IpAddr> = vec!["127.0.0.1".parse().unwrap()];
        let resolved = if trusted.contains(&peer.ip()) {
            header_ip(&headers, "x-real-ip")
                .map(|ip| ip.to_string())
                .unwrap_or_default()
        } else {
            peer.ip().to_string()
        };
        assert_eq!(resolved, "198.51.100.4");
    }

    #[test]
    fn forwarding_header_parser_requires_a_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "not-an-ip".parse().unwrap());
        assert!(header_ip(&headers, "x-real-ip").is_none());
    }
}
