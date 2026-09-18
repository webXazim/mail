//! Request instrumentation for Prometheus (WS6.3). Records latency and status
//! per matched route so metric cardinality stays bounded (no raw paths/ids).

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::metrics::Metrics;

pub async fn track(State(metrics): State<Arc<Metrics>>, req: Request, next: Next) -> Response {
    // Captured before `next` consumes the request; unmatched paths fall back to
    // a constant label rather than the raw URL.
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());

    let started = Instant::now();
    let response = next.run(req).await;
    let elapsed = started.elapsed().as_secs_f64();
    metrics.record_request(&route, response.status().as_u16(), elapsed);
    response
}
