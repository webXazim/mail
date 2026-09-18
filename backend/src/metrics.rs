//! Minimal Prometheus instrumentation (WS6.3). A hand-rolled registry keeps the
//! dependency tree small: two atomics for delivery events and a mutex-guarded
//! map for the per-route request/histogram series. Scraped by `/api/metrics`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Latency histogram bounds in seconds (Prometheus `le` labels).
const BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
struct Registry {
    /// (route, status) -> count
    requests: HashMap<(String, u16), u64>,
    /// route -> cumulative histogram bucket counts, aligned with BUCKETS
    buckets: HashMap<String, [u64; BUCKETS.len()]>,
    duration_sum: HashMap<String, f64>,
    duration_count: HashMap<String, u64>,
    /// route -> count of responses with status >= 500
    errors: HashMap<String, u64>,
}

pub struct Metrics {
    registry: Mutex<Registry>,
    sends_ok: AtomicU64,
    sends_failed: AtomicU64,
    suppressed_dropped: AtomicU64,
    rate_limited: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(Registry::default()),
            sends_ok: AtomicU64::new(0),
            sends_failed: AtomicU64::new(0),
            suppressed_dropped: AtomicU64::new(0),
            rate_limited: AtomicU64::new(0),
        }
    }

    pub fn record_request(&self, route: &str, status: u16, seconds: f64) {
        let mut reg = self.registry.lock().expect("metrics poisoned");
        *reg.requests.entry((route.to_string(), status)).or_insert(0) += 1;
        let buckets = reg
            .buckets
            .entry(route.to_string())
            .or_insert([0; BUCKETS.len()]);
        for (i, le) in BUCKETS.iter().enumerate() {
            if seconds <= *le {
                buckets[i] += 1;
            }
        }
        *reg.duration_sum.entry(route.to_string()).or_insert(0.0) += seconds;
        *reg.duration_count.entry(route.to_string()).or_insert(0) += 1;
        if status >= 500 {
            *reg.errors.entry(route.to_string()).or_insert(0) += 1;
        }
    }

    pub fn record_send_ok(&self) {
        self.sends_ok.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_send_failed(&self) {
        self.sends_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_suppressed_dropped(&self, n: u64) {
        self.suppressed_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub fn record_rate_limited(&self) {
        self.rate_limited.fetch_add(1, Ordering::Relaxed);
    }

    /// Prometheus text exposition (version 0.0.4). `pool_size` / `pool_idle`
    /// come from the SQLx pool gauge.
    pub fn render(&self, pool_size: u64, pool_idle: u64) -> String {
        let reg = self.registry.lock().expect("metrics poisoned");
        let mut out = String::with_capacity(2048);

        out.push_str("# HELP harbor_http_requests_total Requests by route and status.\n");
        out.push_str("# TYPE harbor_http_requests_total counter\n");
        for ((route, status), count) in &reg.requests {
            out.push_str(&format!(
                "harbor_http_requests_total{{path=\"{}\",status=\"{}\"}} {}\n",
                escape(route),
                status,
                count
            ));
        }

        out.push_str("# HELP harbor_http_request_duration_seconds Request latency.\n");
        out.push_str("# TYPE harbor_http_request_duration_seconds histogram\n");
        for (route, buckets) in &reg.buckets {
            for (i, le) in BUCKETS.iter().enumerate() {
                out.push_str(&format!(
                    "harbor_http_request_duration_seconds_bucket{{path=\"{}\",le=\"{}\"}} {}\n",
                    escape(route),
                    le,
                    buckets[i]
                ));
            }
            out.push_str(&format!(
                "harbor_http_request_duration_seconds_bucket{{path=\"{}\",le=\"+Inf\"}} {}\n",
                escape(route),
                buckets[BUCKETS.len() - 1]
            ));
            let sum = reg.duration_sum.get(route).copied().unwrap_or(0.0);
            let count = reg.duration_count.get(route).copied().unwrap_or(0);
            out.push_str(&format!(
                "harbor_http_request_duration_seconds_sum{{path=\"{}\"}} {}\n",
                escape(route),
                sum
            ));
            out.push_str(&format!(
                "harbor_http_request_duration_seconds_count{{path=\"{}\"}} {}\n",
                escape(route),
                count
            ));
        }

        out.push_str("# HELP harbor_http_responses_5xx_total Server errors by route.\n");
        out.push_str("# TYPE harbor_http_responses_5xx_total counter\n");
        for (route, count) in &reg.errors {
            out.push_str(&format!(
                "harbor_http_responses_5xx_total{{path=\"{}\"}} {}\n",
                escape(route),
                count
            ));
        }

        out.push_str("# HELP harbor_smtp_sends_total Outgoing delivery outcomes.\n");
        out.push_str("# TYPE harbor_smtp_sends_total counter\n");
        out.push_str(&format!(
            "harbor_smtp_sends_total{{result=\"ok\"}} {}\n",
            self.sends_ok.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "harbor_smtp_sends_total{{result=\"failed\"}} {}\n",
            self.sends_failed.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "harbor_smtp_sends_total{{result=\"suppressed\"}} {}\n",
            self.suppressed_dropped.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "harbor_smtp_sends_total{{result=\"rate_limited\"}} {}\n",
            self.rate_limited.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP harbor_db_pool_connections SQLx pool gauge.\n");
        out.push_str("# TYPE harbor_db_pool_connections gauge\n");
        out.push_str(&format!(
            "harbor_db_pool_connections{{state=\"open\"}} {pool_size}\n"
        ));
        out.push_str(&format!(
            "harbor_db_pool_connections{{state=\"idle\"}} {pool_idle}\n"
        ));

        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Escape label values per the Prometheus text format.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_counters_and_histogram() {
        let m = Metrics::new();
        m.record_request("/api/send", 200, 0.03);
        m.record_request("/api/send", 500, 1.2);
        m.record_send_ok();
        m.record_send_failed();
        let text = m.render(10, 7);

        assert!(text.contains("harbor_http_requests_total{path=\"/api/send\",status=\"200\"} 1"));
        assert!(text.contains("harbor_http_requests_total{path=\"/api/send\",status=\"500\"} 1"));
        assert!(text.contains("harbor_http_responses_5xx_total{path=\"/api/send\"} 1"));
        // 0.03 falls in the le="0.05" bucket; both requests are <= le="10".
        assert!(text.contains("le=\"0.05\"} 1"));
        assert!(text.contains("le=\"10\"} 2"));
        assert!(text.contains("harbor_http_request_duration_seconds_count{path=\"/api/send\"} 2"));
        assert!(text.contains("harbor_smtp_sends_total{result=\"ok\"} 1"));
        assert!(text.contains("harbor_db_pool_connections{state=\"idle\"} 7"));
    }

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }
}
