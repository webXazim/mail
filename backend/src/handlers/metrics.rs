use std::collections::HashMap;

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Prometheus scrape endpoint. Runtime counters come from the in-process
/// registry; durable lifecycle gauges are read from PostgreSQL so worker/admin
/// failures remain visible across API restarts.
pub async fn metrics(State(state): State<AppState>) -> Response {
    let mut body = state
        .metrics
        .render(state.db.size() as u64, state.db.num_idle() as u64);

    if let Ok(row) = sqlx::query_as::<_, (i64,i64,i64,i64,i64,i64,i64)>(
        "SELECT
           (SELECT count(*)::bigint FROM provisioning_jobs WHERE status='dead'),
           (SELECT count(*)::bigint FROM billing_lifecycle_outbox WHERE status='failed'),
           (SELECT count(*)::bigint FROM billing_email_outbox WHERE status='failed'),
           (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status='failed'),
           (SELECT count(*)::bigint FROM subscription_purge_runs WHERE status IN('queued','processing')),
           (SELECT count(*)::bigint FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
             WHERE o.is_system=FALSE AND m.deleted_at IS NULL AND m.status IN('active','provisioning')
               AND (m.provider_reconciled_at IS NULL OR m.provider_reconciled_at<now()-interval '30 minutes')),
           (SELECT count(*)::bigint FROM orders ord JOIN organizations o ON o.id=ord.organization_id
             WHERE o.is_system=FALSE AND ord.status='submitted' AND ord.invoice_status='issued'
               AND ord.updated_at<now()-interval '48 hours')"
    ).fetch_one(&state.db).await {
        body.push_str("# HELP cs_mail_billing_operational_issues Durable billing/provider operational issue counts.\n");
        body.push_str("# TYPE cs_mail_billing_operational_issues gauge\n");
        for (kind, value) in [
            ("provisioning_dead", row.0),
            ("lifecycle_email_failed", row.1),
            ("invoice_email_failed", row.2),
            ("purge_failed", row.3),
            ("purge_running", row.4),
            ("provider_stale_mailboxes", row.5),
            ("payment_review_aging", row.6),
        ] {
            body.push_str(&format!("cs_mail_billing_operational_issues{{kind=\"{kind}\"}} {value}\n"));
        }
    }

    if let Ok(rows) = sqlx::query_as::<_, (String,i64)>(
        "SELECT status,count(*)::bigint FROM organization_subscriptions s
           JOIN organizations o ON o.id=s.organization_id
          WHERE o.is_system=FALSE GROUP BY status ORDER BY status"
    ).fetch_all(&state.db).await {
        body.push_str("# HELP cs_mail_subscriptions Subscriptions by effective stored lifecycle status.\n");
        body.push_str("# TYPE cs_mail_subscriptions gauge\n");
        for (status, count) in rows {
            let safe = status.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            body.push_str(&format!("cs_mail_subscriptions{{status=\"{safe}\"}} {count}\n"));
        }
    }

    // Backup/restore evidence is append-only in PostgreSQL. Missing evidence is
    // exported as -1 so Prometheus can alert on both absence and staleness
    // without relying on host-local files that the metrics container cannot see.
    let mut evidence_age: HashMap<String, f64> = HashMap::new();
    if let Ok(rows) = sqlx::query_as::<_, (String, f64)>(
        "SELECT kind, EXTRACT(EPOCH FROM (now()-max(recorded_at)))::float8
           FROM operational_evidence WHERE status='passed' GROUP BY kind"
    ).fetch_all(&state.db).await {
        for (kind, age) in rows { evidence_age.insert(kind, age.max(0.0)); }
    }
    body.push_str("# HELP cs_mail_operational_evidence_age_seconds Age of the latest successful backup/restore evidence; -1 means missing.\n");
    body.push_str("# TYPE cs_mail_operational_evidence_age_seconds gauge\n");
    for kind in ["local_backup", "restore_drill", "cs_mail_offsite_backup", "stalwart_offsite_backup"] {
        let value = evidence_age.get(kind).copied().unwrap_or(-1.0);
        body.push_str(&format!("cs_mail_operational_evidence_age_seconds{{kind=\"{kind}\"}} {value}\n"));
    }

    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
