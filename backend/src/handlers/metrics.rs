use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Prometheus scrape endpoint (WS6.3). Text exposition so a stock scrape config
/// works without a client library on the API side.
pub async fn metrics(State(state): State<AppState>) -> Response {
    let body = state
        .metrics
        .render(state.db.size() as u64, state.db.num_idle() as u64);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
