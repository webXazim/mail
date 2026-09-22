//! Entitlement primitives shared by billing, sending and provider quota sync.
//! Concrete plan values are loaded only from PostgreSQL by
//! `services::entitlements`; this module intentionally contains no compiled-in
//! plan table so production cannot drift between code and the database.

use std::collections::BTreeMap;

/// Concrete plan entitlements enforced across the API.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanLimits {
    pub code: String,
    pub name: String,
    pub price_cents: i64,
    pub extra_mailbox_price_cents: i64,
    pub currency: String,
    pub interval: String,
    /// Per-mailbox provider quota.
    pub mailbox_bytes: u64,
    /// Aggregate storage available to the whole organization.
    pub storage_pool_bytes: u64,
    /// Mailboxes included in the base plan price.
    pub mailbox_limit: i32,
    /// Highest self-service mailbox quantity for this plan.
    pub max_mailboxes: i32,
    /// Per-mailbox alias cap; None means unlimited.
    pub alias_limit_per_mailbox: Option<i32>,
    pub domain_limit: i32,
    pub organization_daily_send_limit: i64,
    pub max_attachment_bytes: usize,
    /// Until the staged-blob attachment upgrade lands, the total attachment
    /// budget equals the single-attachment cap. Keeping this explicit avoids
    /// hidden constants in the send path and lets a later migration split the
    /// values without changing call sites.
    pub max_total_attachment_bytes: usize,
    pub max_recipients: usize,
    pub daily_send_limit: i64,
    pub seats: i32,
    /// Customer-facing marketing bullets.
    pub features: Vec<String>,
    /// Machine-readable gates. Unknown flags fail closed.
    pub feature_flags: BTreeMap<String, bool>,
    pub active: bool,
}

impl PlanLimits {
    /// Human price, e.g. `SAR 25.00` — no currency conversion, display only.
    pub fn price_display(&self) -> String {
        format!(
            "{}{}.{:02}",
            currency_symbol(&self.currency),
            self.price_cents / 100,
            self.price_cents % 100
        )
    }

    /// Feature gates are explicit: an unknown/missing flag is disabled rather
    /// than accidentally enabled by a typo or incomplete plan migration.
    pub fn allows(&self, feature: &str) -> bool {
        self.feature_flags.get(feature).copied().unwrap_or(false)
    }
}

/// Best-effort symbol for the handful of currencies we expect; falls back to a
/// trailing code so an unknown currency still renders unambiguously.
pub fn currency_symbol(currency: &str) -> String {
    match currency.to_ascii_uppercase().as_str() {
        "USD" => "$".to_string(),
        "EUR" => "€".to_string(),
        "GBP" => "£".to_string(),
        other => format!("{other} "),
    }
}

/// How full a mailbox is, clamped to `[0, 1]`.
pub fn used_ratio(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (used as f64 / total as f64).clamp(0.0, 1.0)
}

/// True when writing `incoming` more bytes would stay within `total`.
pub fn fits(used: u64, total: u64, incoming: u64) -> bool {
    used.saturating_add(incoming) <= total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_accounts_for_usage_and_overflow() {
        assert!(fits(0, 100, 100));
        assert!(!fits(1, 100, 100));
        assert!(!fits(u64::MAX, 100, 1));
    }

    #[test]
    fn used_ratio_is_clamped() {
        assert_eq!(used_ratio(0, 0), 0.0);
        assert_eq!(used_ratio(50, 100), 0.5);
        assert_eq!(used_ratio(200, 100), 1.0);
    }

    #[test]
    fn missing_feature_flag_fails_closed() {
        let mut flags = BTreeMap::new();
        flags.insert("mail".into(), true);
        let plan = PlanLimits {
            code: "test".into(),
            name: "Test".into(),
            price_cents: 0,
            extra_mailbox_price_cents: 0,
            currency: "USD".into(),
            interval: "month".into(),
            mailbox_bytes: 1024,
            storage_pool_bytes: 4096,
            mailbox_limit: 2,
            max_mailboxes: 10,
            alias_limit_per_mailbox: Some(10),
            domain_limit: 1,
            organization_daily_send_limit: 10,
            max_attachment_bytes: 1024,
            max_total_attachment_bytes: 1024,
            max_recipients: 1,
            daily_send_limit: 1,
            seats: 1,
            features: vec![],
            feature_flags: flags,
            active: true,
        };
        assert!(plan.allows("mail"));
        assert!(!plan.allows("unknown"));
    }
}
