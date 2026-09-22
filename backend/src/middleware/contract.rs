use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

pub static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
pub static CONTRACT_VERSION_HEADER: HeaderName = HeaderName::from_static("x-cs-contract-version");

/// Attach a stable request id and the public API contract version to every API
/// response. If Nginx supplies a valid request id we keep it so logs can be
/// correlated across the proxy and application layers.
pub async fn headers(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_request_id(value))
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER.clone(), value);
    }
    response.headers_mut().insert(
        CONTRACT_VERSION_HEADER.clone(),
        HeaderValue::from_static("32"),
    );
    response
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validation_rejects_header_injection() {
        assert!(valid_request_id("proxy-1234.abcd"));
        assert!(!valid_request_id("bad\r\nheader"));
        assert!(!valid_request_id(""));
    }
}
