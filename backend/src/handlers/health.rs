use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// Liveness + readiness: reports 200 only when the database is reachable.
/// Container healthchecks fail fast when Postgres is down (503), which lets
/// orchestrators restart/route away instead of serving a healthy facade.
pub async fn health(State(state): State<AppState>) -> Response {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = json!({
        "service": "harbor-mail-api",
        "domain": "mail.crescentsphere.com",
        "stack": { "web": "mailer.crescentsphere.com", "imap": "mail.crescentsphere.com", "smtp": "mail.crescentsphere.com" },
        "status": if db_ok { "ok" } else { "degraded" },
        "database": if db_ok { "ok" } else { "unreachable" }
    });

    (status, Json(body)).into_response()
}
