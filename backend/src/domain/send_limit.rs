//! Daily send-rate policy (WS5.3). Counts recipients submitted per UTC day,
//! both per user and across the sending domain, so one compromised credential
//! cannot burn the domain's reputation. Pure policy — the atomic counters live
//! in `services::send_limit`.

use chrono::{DateTime, Duration, NaiveDate, Utc};

/// Recipients every mailbox on one domain may send per UTC day. A safety net
/// for the shared sending reputation, not a per-customer entitlement.
pub const PER_DOMAIN_DAILY: i64 = 50_000;

/// Absolute ceiling on one message's recipient list, regardless of plan.
pub const HARD_MAX_RECIPIENTS: usize = 100;

/// Recipients allowed in a single message: the plan entitlement clamped to the
/// absolute ceiling so a misconfigured plan can never exceed it.
pub fn per_message(plan_max_recipients: usize) -> usize {
    plan_max_recipients.min(HARD_MAX_RECIPIENTS)
}

/// Day-one warmup ramp for a freshly provisioned mailbox, in recipients/day.
/// A new domain that suddenly emits thousands of messages is the classic spam
/// signature, so volume climbs over the first month. Returns 0 once warm.
pub fn warmup_daily(account_age_days: i64) -> i64 {
    match account_age_days {
        d if d < 0 => 50,
        0..=2 => 50,
        3..=6 => 100,
        7..=13 => 250,
        14..=29 => 500,
        _ => 0,
    }
}

/// The daily recipient ceiling actually enforced: the plan entitlement capped
/// by the warmup ramp while the account is young.
pub fn effective_daily(plan_daily_limit: i64, account_age_days: i64) -> i64 {
    match warmup_daily(account_age_days) {
        0 => plan_daily_limit,
        warmup => plan_daily_limit.min(warmup),
    }
}

/// The UTC calendar day a timestamp belongs to.
pub fn day_of(now: DateTime<Utc>) -> NaiveDate {
    now.date_naive()
}

/// Whole seconds until the counter resets (next UTC midnight).
pub fn retry_after_secs(now: DateTime<Utc>) -> i64 {
    let tomorrow = (now.date_naive() + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time");
    (tomorrow - now.naive_utc()).num_seconds().max(0)
}

/// True when `count` has reached or passed `limit`.
pub fn exceeded(count: i64, limit: i64) -> bool {
    limit > 0 && count > limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_ceiling_exceeds_every_realistic_volume() {
        // A domain ceiling far above the warmup peak but finite.
        assert!(PER_DOMAIN_DAILY > warmup_daily(20));
    }

    #[test]
    fn retry_after_counts_to_utc_midnight() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 17)
            .unwrap()
            .and_hms_opt(23, 59, 30)
            .unwrap()
            .and_utc();
        assert_eq!(retry_after_secs(now), 30);

        let midnight = NaiveDate::from_ymd_opt(2026, 9, 17)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        assert_eq!(retry_after_secs(midnight), 86_400);
    }

    #[test]
    fn exceeded_is_strict_and_ignores_zero_limits() {
        assert!(!exceeded(10, 10));
        assert!(exceeded(11, 10));
        assert!(!exceeded(1_000_000, 0));
    }

    #[test]
    fn warmup_ramps_then_disappears() {
        assert_eq!(warmup_daily(0), 50);
        assert_eq!(warmup_daily(2), 50);
        assert_eq!(warmup_daily(3), 100);
        assert_eq!(warmup_daily(10), 250);
        assert_eq!(warmup_daily(20), 500);
        assert_eq!(warmup_daily(45), 0);
        // Monotonic non-decreasing across the warmup window (day 30 is warm,
        // where the cap is lifted entirely).
        let ramp: Vec<i64> = (0..30).map(warmup_daily).collect();
        assert!(ramp.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn effective_daily_is_min_of_plan_and_warmup() {
        // A young account is held to the warmup ramp even on a big plan...
        assert_eq!(effective_daily(10_000, 1), 50);
        // ...while a warm account keeps its plan limit.
        assert_eq!(effective_daily(300, 60), 300);
    }

    #[test]
    fn per_message_never_exceeds_hard_cap() {
        assert_eq!(per_message(25), 25);
        assert_eq!(per_message(100), HARD_MAX_RECIPIENTS);
        // A plan configured above the ceiling is clamped down.
        assert_eq!(per_message(5_000), HARD_MAX_RECIPIENTS);
    }
}
