use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;

use crate::error::ApiError;

/// In-memory, single-instance rate limiting for the auth surface: fixed-window
/// burst caps per key (`check_burst`) plus consecutive-failure lockout per
/// credential+IP (`record_failure`). Entries are pruned once the map grows
/// past a bound, so memory never leaks on long-running instances.
pub const AUTH_IP_LIMIT: u32 = 20; // requests per window across /api/auth/*
pub const AUTH_IP_WINDOW: i64 = 60;

pub const CRED_MAX_FAILS: u32 = 5; // failures within the window before lockout
pub const CRED_WINDOW: i64 = 900; // 15 min sliding fix since first failure
pub const CRED_LOCK: i64 = 900; // 15 min hard lock

pub const EMAIL_HOURLY_LIMIT: u32 = 3; // verification/reset emails per address per hour
pub const EMAIL_HOURLY_WINDOW: i64 = 3600;

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, Bucket>>>,
}

#[derive(Clone, Debug)]
struct Bucket {
    window_start: i64,
    count: u32,
    lock_until: Option<i64>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Fixed-window burst check. Consumes one slot per call; returns 429 once
    /// the window's budget is spent or a lock is active.
    pub fn check_burst(&self, key: &str, limit: u32, window: i64) -> Result<(), ApiError> {
        let now = Self::now();
        let mut map = self.inner.lock().expect("rate limiter poisoned");
        let entry = map.entry(key.to_string()).or_insert(Bucket {
            window_start: now,
            count: 0,
            lock_until: None,
        });

        if let Some(until) = entry.lock_until {
            if until > now {
                return Err(ApiError::too_many(
                    "Too many requests. Please wait and try again.",
                ));
            }
        }

        if now - entry.window_start >= window {
            *entry = Bucket {
                window_start: now,
                count: 0,
                lock_until: None,
            };
        }
        entry.count += 1;
        if entry.count > limit {
            return Err(ApiError::too_many(
                "Too many requests. Please wait and try again.",
            ));
        }

        Self::prune(&mut map, now, window);
        Ok(())
    }

    /// Consecutive-failure lockout. Call once per rejected credential attempt
    /// (bad password or unknown account). After `max` failures within `window`
    /// the key is locked for `lock` seconds; further attempts return 429.
    pub fn record_failure(
        &self,
        key: &str,
        max: u32,
        window: i64,
        lock: i64,
    ) -> Result<(), ApiError> {
        let now = Self::now();
        let mut map = self.inner.lock().expect("rate limiter poisoned");
        let entry = map.entry(key.to_string()).or_insert(Bucket {
            window_start: now,
            count: 0,
            lock_until: None,
        });

        if let Some(until) = entry.lock_until {
            if until > now {
                return Err(ApiError::too_many(
                    "Too many failed attempts. Please wait and try again.",
                ));
            }
        }

        if now - entry.window_start >= window && entry.lock_until.is_none() {
            *entry = Bucket {
                window_start: now,
                count: 0,
                lock_until: None,
            };
        }
        entry.count += 1;
        if entry.count >= max {
            entry.count = 0;
            entry.lock_until = Some(now + lock);
        }

        Self::prune(&mut map, now, window);
        Ok(())
    }

    /// Cheap pre-check: returns an early 429 while a lock is active, before
    /// any DB lookup or password hashing. Call at the top of a guarded flow.
    pub fn check_lock(&self, key: &str) -> Result<(), ApiError> {
        let now = Self::now();
        let map = self.inner.lock().expect("rate limiter poisoned");
        if let Some(b) = map.get(key) {
            if b.lock_until.is_some_and(|u| u > now) {
                return Err(ApiError::too_many(
                    "Too many failed attempts. Please wait and try again.",
                ));
            }
        }
        Ok(())
    }

    /// Clears every trace of a key, typically after a successful login.
    pub fn reset(&self, key: &str) {
        self.inner
            .lock()
            .expect("rate limiter poisoned")
            .remove(key);
    }

    fn prune(map: &mut HashMap<String, Bucket>, now: i64, window: i64) {
        if map.len() > 2048 {
            let cutoff = now - (2 * window);
            map.retain(|_, b| b.lock_until.is_some_and(|u| u > now) || b.window_start > cutoff);
        }
    }
}

/// Client IP for rate-limit keys. Trusts the reverse proxy headers when
/// present (Caddy sets x-real-ip); otherwise falls back to the actual socket
/// peer, so direct (non-proxied) deployments still get a per-client bucket.
pub fn client_ip(headers: &HeaderMap, remote: Option<std::net::SocketAddr>) -> String {
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        return v.trim().to_string();
    }
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = v.split(',').next() {
            return first.trim().to_string();
        }
    }
    remote
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
