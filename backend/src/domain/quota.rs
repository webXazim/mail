//! Plan-based quota policy (WS2.7). The billing plan decides how much mailbox
//! disk a user gets and how large a single outgoing message may be. The API
//! enforces the message caps on send; the mailbox quota is handed to Stalwart
//! when the account is provisioned.

/// Tiers shown to customers, mirroring `frontend/src/lib/plans.ts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Solo,
    Team,
    Business,
}

impl Plan {
    pub fn from_db(value: &str) -> Self {
        match value {
            "business" => Plan::Business,
            "team" => Plan::Team,
            _ => Plan::Solo,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Plan::Solo => "solo",
            Plan::Team => "team",
            Plan::Business => "business",
        }
    }

    /// Mailbox disk quota in bytes.
    pub fn mailbox_bytes(self) -> u64 {
        match self {
            Plan::Solo => 5 * 1024 * 1024 * 1024,
            Plan::Team => 50 * 1024 * 1024 * 1024,
            Plan::Business => 200 * 1024 * 1024 * 1024,
        }
    }

    /// Largest single attachment a plan may send.
    pub fn max_attachment_bytes(self) -> usize {
        match self {
            Plan::Solo => 25 * 1024 * 1024,
            Plan::Team => 50 * 1024 * 1024,
            Plan::Business => 100 * 1024 * 1024,
        }
    }

    /// Combined attachment budget for one outgoing message.
    pub fn max_total_attachment_bytes(self) -> usize {
        self.max_attachment_bytes()
    }

    /// Recipients a single message may address on this tier.
    pub fn max_recipients(self) -> usize {
        match self {
            Plan::Solo => 25,
            Plan::Team => 50,
            Plan::Business => 100,
        }
    }

    /// Recipients per UTC day on this tier.
    pub fn daily_send_limit(self) -> i64 {
        match self {
            Plan::Solo => 300,
            Plan::Team => 2_000,
            Plan::Business => 10_000,
        }
    }

    /// Built-in defaults, used as the fallback when the admin-editable `plans`
    /// row is missing or the DB is unreachable.
    pub fn limits(self) -> PlanLimits {
        PlanLimits {
            code: self.as_str().to_string(),
            name: match self {
                Plan::Solo => "Harbor Solo".to_string(),
                Plan::Team => "Harbor Team".to_string(),
                Plan::Business => "Harbor Business".to_string(),
            },
            price_cents: match self {
                Plan::Solo => 0,
                Plan::Team => 800,
                Plan::Business => 1600,
            },
            currency: "USD".to_string(),
            interval: "month".to_string(),
            mailbox_bytes: self.mailbox_bytes(),
            max_attachment_bytes: self.max_attachment_bytes(),
            max_total_attachment_bytes: self.max_total_attachment_bytes(),
            max_recipients: self.max_recipients(),
            daily_send_limit: self.daily_send_limit(),
            seats: match self {
                Plan::Solo => 1,
                Plan::Team => 5,
                Plan::Business => 25,
            },
            features: Vec::new(),
            active: true,
        }
    }
}

/// The concrete entitlements enforced across the API (WS4). Loaded from the
/// admin-editable `plans` table, falling back to the built-in `Plan` defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanLimits {
    pub code: String,
    pub name: String,
    pub price_cents: i64,
    pub currency: String,
    pub interval: String,
    pub mailbox_bytes: u64,
    pub max_attachment_bytes: usize,
    pub max_total_attachment_bytes: usize,
    pub max_recipients: usize,
    pub daily_send_limit: i64,
    pub seats: i32,
    pub features: Vec<String>,
    pub active: bool,
}

impl PlanLimits {
    /// Human price, e.g. `$8.00` — no currency conversion, display only.
    pub fn price_display(&self) -> String {
        format!(
            "{}{}.{:02}",
            currency_symbol(&self.currency),
            self.price_cents / 100,
            self.price_cents % 100
        )
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
    fn plans_map_from_db_with_solo_fallback() {
        assert_eq!(Plan::from_db("team"), Plan::Team);
        assert_eq!(Plan::from_db("business"), Plan::Business);
        assert_eq!(Plan::from_db("enterprise"), Plan::Solo);
        assert_eq!(Plan::from_db(""), Plan::Solo);
    }

    #[test]
    fn higher_tiers_allow_larger_attachments() {
        assert!(Plan::Business.max_attachment_bytes() > Plan::Team.max_attachment_bytes());
        assert!(Plan::Team.max_attachment_bytes() > Plan::Solo.max_attachment_bytes());
    }

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
}
