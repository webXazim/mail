use axum::http::HeaderMap;

use crate::state::AppState;

/// HttpOnly session cookie holding the refresh token. Browser JS can never
/// read it, which removes the localStorage XSS-theft vector. SameSite=Lax
/// keeps it off cross-site POSTs (CSRF), and `Secure` is turned on once the
/// deployment serves TLS.
pub const SESSION_COOKIE: &str = "harbor_session";

pub fn build_session_cookie(state: &AppState, token: &str, max_age_secs: i64) -> String {
    let secure = if state.cookie_secure { " Secure;" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Path=/; SameSite=Lax; Max-Age={max_age_secs};{secure}"
    )
}

pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; Path=/; SameSite=Lax; Max-Age=0")
}

/// Read the refresh token out of the session cookie, if present.
pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|all| {
            all.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix(&format!("{SESSION_COOKIE}="))
                    .map(|value| value.to_string())
            })
        })
}
