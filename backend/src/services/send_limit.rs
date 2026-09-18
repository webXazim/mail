//! Atomic daily send counters (WS5.3). Increment-then-check so a burst cannot
//! slip through a check-then-record race; a rejected message still consumes its
//! budget, which is the conservative choice for anti-abuse.

use uuid::Uuid;

use crate::domain::send_limit as policy;
use crate::error::ApiError;
use crate::state::AppState;

/// Charge `recipients` against the user's and the domain's daily budgets,
/// returning 429 once either ceiling is crossed. `daily_limit` is the user's
/// plan entitlement; it is capped by the warmup ramp while the account is new.
pub async fn enforce(
    state: &AppState,
    user_id: Uuid,
    domain: &str,
    daily_limit: i64,
    recipients: i64,
) -> Result<(), ApiError> {
    if recipients <= 0 || domain.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();
    let day = policy::day_of(now);
    let grant = recipients as i32;

    // WS7.3: a young mailbox sends on a warmup ramp, so the enforced ceiling is
    // the lower of the plan entitlement and the ramp for the account's age.
    let created: Option<(chrono::DateTime<chrono::Utc>,)> =
        sqlx::query_as("SELECT created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let age_days = created.map_or(0, |(c,)| (now - c).num_days());

    let (user_count,): (i32,) = sqlx::query_as(
        "INSERT INTO send_counters (user_id, day, count)
         VALUES ($1, $2, $3)
         ON CONFLICT (user_id, day)
         DO UPDATE SET count = send_counters.count + EXCLUDED.count
         RETURNING count",
    )
    .bind(user_id)
    .bind(day)
    .bind(grant)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (domain_count,): (i32,) = sqlx::query_as(
        "INSERT INTO send_domain_counters (domain, day, count)
         VALUES ($1, $2, $3)
         ON CONFLICT (domain, day)
         DO UPDATE SET count = send_domain_counters.count + EXCLUDED.count
         RETURNING count",
    )
    .bind(domain)
    .bind(day)
    .bind(grant)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let user_limit = policy::effective_daily(daily_limit, age_days);
    if policy::exceeded(user_count as i64, user_limit) {
        state.metrics.record_rate_limited();
        let minutes = (policy::retry_after_secs(now) + 59) / 60;
        return Err(ApiError::too_many(format!(
            "Daily sending limit reached ({user_limit} recipients). Try again in about {minutes} minutes."
        )));
    }
    if policy::exceeded(domain_count as i64, policy::PER_DOMAIN_DAILY) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(
            "This domain has reached its daily sending ceiling. Try again after 00:00 UTC.",
        ));
    }
    Ok(())
}
