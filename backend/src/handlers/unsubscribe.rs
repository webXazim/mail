//! Public one-click unsubscribe endpoint (WS7.2). Mail clients POST here via
//! RFC 8058; humans clicking the link GET it. No session is required — the
//! signed token is the authority, so it must not be guessable.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use serde::Deserialize;

use crate::error::ApiError;
use crate::services::suppression;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UnsubQuery {
    token: String,
}

pub async fn unsubscribe(
    State(state): State<AppState>,
    Query(q): Query<UnsubQuery>,
) -> Result<(StatusCode, Html<String>), ApiError> {
    let claims = suppression::verify_unsubscribe_token(&state.jwt_secret, &q.token)
        .ok_or_else(|| ApiError::bad_request("This unsubscribe link is invalid or has expired"))?;

    // The token carries the business/mailbox that sent the original message,
    // so an unsubscribe never suppresses the recipient for unrelated tenants.
    let valid_scope: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM mailboxes
           WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL
         )",
    )
    .bind(claims.mailbox_id)
    .bind(claims.organization_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !valid_scope {
        return Err(ApiError::bad_request("This unsubscribe link is no longer valid"));
    }

    suppression::suppress_for_scope(
        &state,
        claims.organization_id,
        Some(claims.mailbox_id),
        &claims.email,
        "unsubscribed",
        "unsubscribe",
        "",
    )
    .await?;

    Ok((StatusCode::OK, Html(page(&claims.email))))
}

fn page(email: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <title>Unsubscribed</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:40rem;margin:4rem auto;padding:0 1rem\">\
         <h1>Unsubscribed</h1>\
         <p><strong>{}</strong> will not receive further messages from this sender.</p>\
         </body></html>",
        escape(email)
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralizes_html() {
        assert_eq!(escape("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
        assert_eq!(escape("a&b"), "a&amp;b");
    }
}
