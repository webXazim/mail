use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::state::AppState;


/// Process liveness only. This endpoint intentionally does not touch external
/// dependencies, so an orchestrator can distinguish a dead process from a
/// temporarily unavailable dependency.
pub async fn live() -> Response {
    (StatusCode::OK, Json(json!({
        "service": "cs-mail-api",
        "status": "alive"
    }))).into_response()
}

/// Explicit readiness alias used by deployment gates.
pub async fn ready(State(state): State<AppState>) -> Response {
    health(State(state)).await
}

/// Liveness/readiness for dependencies required to serve the product.
/// Database failure is always fatal. When the mail provider is configured it
/// must also answer its management probe; a deliberately disabled provider is
/// reported but does not make hermetic development/test instances unhealthy.
pub async fn health(State(state): State<AppState>) -> Response {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    let mail_enabled = state.stalwart.enabled();
    let mail_probe = if mail_enabled {
        Some(
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                state.stalwart.healthcheck(),
            )
            .await
            {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err("mail provider readiness probe timed out".to_string()),
            },
        )
    } else {
        None
    };
    let mail_ok = mail_probe.as_ref().map(|result| result.is_ok()).unwrap_or(true);

    if let Some(Err(error)) = &mail_probe {
        tracing::warn!(error = %error, "mail provider readiness probe failed");
    }

    let (pending_jobs, processing_jobs, dead_jobs): (i64, i64, i64) = if db_ok {
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE status IN ('pending','retry')),
                    COUNT(*) FILTER (WHERE status = 'processing'),
                    COUNT(*) FILTER (WHERE status = 'dead')
             FROM provisioning_jobs",
        )
        .fetch_one(&state.db)
        .await
        .unwrap_or((0, 0, 0))
    } else {
        (0, 0, 0)
    };

    let (uncertain_sends, stale_submitting): (i64, i64) = if db_ok {
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE status = 'uncertain'),
                    COUNT(*) FILTER (WHERE status = 'submitting' AND updated_at < now() - interval '5 minutes')
             FROM mail_send_requests",
        )
        .fetch_one(&state.db)
        .await
        .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    let (schedule_queued, schedule_processing, schedule_dead, schedule_stale): (i64, i64, i64, i64) = if db_ok {
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE status IN ('pending','retry')),
                    COUNT(*) FILTER (WHERE status = 'processing'),
                    COUNT(*) FILTER (WHERE status = 'dead'),
                    COUNT(*) FILTER (WHERE status = 'processing' AND lease_until <= now())
             FROM scheduled_sends",
        )
        .fetch_one(&state.db)
        .await
        .unwrap_or((0, 0, 0, 0))
    } else {
        (0, 0, 0, 0)
    };

    let ready = db_ok && mail_ok;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = json!({
        "service": "cs-mail-api",
        "status": if ready && dead_jobs == 0 && stale_submitting == 0 && schedule_dead == 0 && schedule_stale == 0 { "ok" } else { "degraded" },
        "database": if db_ok { "ok" } else { "unreachable" },
        "mail_provider": if !mail_enabled {
            "disabled"
        } else if mail_ok {
            "ok"
        } else {
            "unreachable"
        },
        "provisioning": {
            "queued": pending_jobs,
            "processing": processing_jobs,
            "dead": dead_jobs
        },
        "sending": {
            "uncertain": uncertain_sends,
            "stale_submitting": stale_submitting
        },
        "scheduled_delivery": {
            "queued": schedule_queued,
            "processing": schedule_processing,
            "dead": schedule_dead,
            "expired_leases": schedule_stale
        }
    });

    (status, Json(body)).into_response()
}
