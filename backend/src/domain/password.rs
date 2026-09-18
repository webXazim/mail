use crate::error::ApiError;

/// Compact list of the most-attacked passwords. This catches the default
/// credential-stuffing primaries locally; a full breached-password check
/// (k-anonymity API) can be layered on in front of this without changes.
const COMMON_PASSWORDS: &[&str] = &[
    "000000",
    "111111",
    "123123",
    "123321",
    "123456",
    "12345678",
    "123456789",
    "1234567890",
    "123456789a",
    "123456789b",
    "1234abcd",
    "654321",
    "666666",
    "88888888",
    "aaa111",
    "abc123",
    "abcd1234",
    "admin",
    "admin123",
    "asdfghjkl",
    "azerty",
    "baseball",
    "changeme",
    "dragon",
    "football",
    "hello123",
    "iloveyou",
    "letmein",
    "login",
    "login123",
    "master",
    "monkey",
    "passw0rd",
    "password",
    "password1",
    "password123",
    "princess",
    "qwerty",
    "qwerty123",
    "qwertyuiop",
    "superman",
    "sunshine",
    "trustno1",
    "welcome",
    "welcome123",
    "whatever",
    "winter",
    "summer",
];

/// Pure policy: length, character mix, and commonness.
/// `context` is the normalized lower-case email so the password can't echo it.
pub fn validate_password(password: &str, context: &str) -> Result<(), ApiError> {
    let trimmed = password.trim();
    if trimmed.len() < 12 {
        return Err(ApiError::bad_request(
            "Password must be at least 12 characters",
        ));
    }

    let (upper, lower, digit, symbol) = char_classes(trimmed);
    let classes = [upper, lower, digit, symbol]
        .into_iter()
        .filter(|v| *v)
        .count();
    if classes < 3 {
        return Err(ApiError::bad_request(
            "Password must use at least three of: uppercase, lowercase, numbers, symbols",
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    if COMMON_PASSWORDS.contains(&lower.as_str()) {
        return Err(ApiError::bad_request(
            "That password is too common. Choose a unique one.",
        ));
    }

    let context = context.trim().to_ascii_lowercase();
    if !context.is_empty() && lower.contains(&context) {
        return Err(ApiError::bad_request(
            "Password must not contain your email address",
        ));
    }
    if let Some((local, _)) = context.split_once('@') {
        if local.len() >= 3 && lower.contains(local) {
            return Err(ApiError::bad_request(
                "Password must not contain your email address",
            ));
        }
    }

    Ok(())
}

fn char_classes(password: &str) -> (bool, bool, bool, bool) {
    let mut upper = false;
    let mut lower = false;
    let mut digit = false;
    let mut symbol = false;
    for ch in password.chars() {
        if ch.is_ascii_uppercase() {
            upper = true;
        } else if ch.is_ascii_lowercase() {
            lower = true;
        } else if ch.is_ascii_digit() {
            digit = true;
        } else {
            symbol = true;
        }
    }
    (upper, lower, digit, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pw: &str) -> Result<(), ApiError> {
        validate_password(pw, "user@example.com")
    }

    #[test]
    fn rejects_short_and_strength() {
        assert!(p("short").is_err());
        assert!(p("aaaaaaaaaaaa").is_err()); // one class only
        assert!(p("aaaaaaaaaaaa1").is_err()); // two classes
    }

    #[test]
    fn accepts_strong() {
        assert!(p("CorrectHorse-Battery9!").is_ok());
        assert!(p("n0t-common-passphrase").is_ok());
    }

    #[test]
    fn rejects_common() {
        assert!(p("password123").is_err());
        assert!(p("Qwerty123!").is_err()); // case-folded to qwerty123
    }

    #[test]
    fn rejects_embedding_email() {
        assert!(validate_password("example.com UserPass1", "user@example.com").is_err());
        assert!(validate_password("UserPass1", "user@example.com").is_err()); // contains local part
    }

    #[test]
    fn trims() {
        assert!(p("  CorrectHorse-Battery9!  ").is_ok());
    }
}
