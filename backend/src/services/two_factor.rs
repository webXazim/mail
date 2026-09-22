//! TOTP and recovery-code primitives for account two-factor authentication.
//!
//! Secrets are generated with the operating-system CSPRNG and stored only
//! encrypted at rest (PostgreSQL pgcrypto in the handlers). The implementation
//! follows RFC 4226 / RFC 6238 using HMAC-SHA1, 6 digits, 30-second steps and a
//! +/- one-step verification window for modest clock drift.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ApiError;

type HmacSha1 = Hmac<Sha1>;

pub const TOTP_PERIOD_SECS: u64 = 30;
pub const TOTP_DIGITS: u32 = 6;
pub const SETUP_TTL_MINUTES: i64 = 10;
pub const CHALLENGE_TTL_MINUTES: i64 = 5;
pub const CHALLENGE_MAX_ATTEMPTS: i32 = 8;
pub const RECOVERY_CODE_COUNT: usize = 10;

pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn random_challenge_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn generate_secret() -> String {
    let mut bytes = [0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    BASE32_NOPAD.encode(&bytes)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn otpauth_uri(email: &str, secret: &str) -> String {
    let issuer = "CS Mail";
    let label = format!("{issuer}:{}", email.trim().to_lowercase());
    format!(
        "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        percent_encode(&label),
        secret,
        percent_encode(issuer),
        TOTP_DIGITS,
        TOTP_PERIOD_SECS
    )
}

fn hotp(secret: &[u8], counter: u64) -> Result<u32, ApiError> {
    let mut mac = HmacSha1::new_from_slice(secret)
        .map_err(|_| ApiError::internal("Could not initialize two-factor verification"))?;
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();
    let offset = (result[19] & 0x0f) as usize;
    let binary = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | result[offset + 3] as u32;
    Ok(binary % 10u32.pow(TOTP_DIGITS))
}

pub fn code_at(secret_b32: &str, unix_secs: u64) -> Result<String, ApiError> {
    let secret = BASE32_NOPAD
        .decode(secret_b32.trim().as_bytes())
        .map_err(|_| ApiError::internal("Stored two-factor secret is invalid"))?;
    let code = hotp(&secret, unix_secs / TOTP_PERIOD_SECS)?;
    Ok(format!("{code:06}"))
}

pub fn matching_totp_step_at(
    secret_b32: &str,
    code: &str,
    unix_secs: u64,
) -> Result<Option<u64>, ApiError> {
    let normalized = code.trim();
    if normalized.len() != TOTP_DIGITS as usize || !normalized.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let expected: u32 = normalized.parse().unwrap_or(u32::MAX);
    let secret = BASE32_NOPAD
        .decode(secret_b32.trim().as_bytes())
        .map_err(|_| ApiError::internal("Stored two-factor secret is invalid"))?;
    let counter = unix_secs / TOTP_PERIOD_SECS;
    for drift in [-1i64, 0, 1] {
        let candidate = counter as i64 + drift;
        if candidate >= 0 && hotp(&secret, candidate as u64)? == expected {
            return Ok(Some(candidate as u64));
        }
    }
    Ok(None)
}

pub fn verify_totp_at(secret_b32: &str, code: &str, unix_secs: u64) -> Result<bool, ApiError> {
    Ok(matching_totp_step_at(secret_b32, code, unix_secs)?.is_some())
}

pub fn matching_totp_step(secret_b32: &str, code: &str) -> Result<Option<u64>, ApiError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::internal("System clock is invalid"))?
        .as_secs();
    matching_totp_step_at(secret_b32, code, now)
}

pub fn verify_totp(secret_b32: &str, code: &str) -> Result<bool, ApiError> {
    Ok(matching_totp_step(secret_b32, code)?.is_some())
}

pub fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_uppercase())
        .collect()
}

pub fn recovery_prefix(code: &str) -> Option<String> {
    let normalized = normalize_recovery_code(code);
    (normalized.len() >= 20).then(|| normalized[..4].to_string())
}

fn format_recovery_code(raw: &str) -> String {
    raw.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn generate_recovery_codes() -> Vec<String> {
    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    while codes.len() < RECOVERY_CODE_COUNT {
        let mut bytes = [0u8; 13]; // 104 bits; first 100 bits become 20 base32 chars.
        OsRng.fill_bytes(&mut bytes);
        let raw = BASE32_NOPAD.encode(&bytes);
        let raw = &raw[..20];
        let formatted = format_recovery_code(raw);
        let prefix = &raw[..4];
        if !codes.iter().any(|existing: &String| normalize_recovery_code(existing).starts_with(prefix)) {
            codes.push(formatted);
        }
    }
    codes
}

pub fn hash_recovery_code(code: &str) -> Result<String, ApiError> {
    let normalized = normalize_recovery_code(code);
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Argon2::default()
        .hash_password(normalized.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub fn verify_recovery_code(hash: &str, code: &str) -> Result<bool, ApiError> {
    let parsed = PasswordHash::new(hash).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(normalize_recovery_code(code).as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_6238_sha1_vector_matches_truncated_six_digits() {
        // RFC 6238 vector uses ASCII "12345678901234567890" at T=59 and gives
        // 94287082 for 8 digits, hence 287082 for the six-digit variant.
        let secret = BASE32_NOPAD.encode(b"12345678901234567890");
        assert!(verify_totp_at(&secret, "287082", 59).unwrap());
        assert!(!verify_totp_at(&secret, "287083", 59).unwrap());
    }

    #[test]
    fn recovery_codes_are_unique_and_normalized() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let mut normalized: Vec<_> = codes.iter().map(|c| normalize_recovery_code(c)).collect();
        assert!(normalized.iter().all(|c| c.len() == 20));
        normalized.sort();
        normalized.dedup();
        assert_eq!(normalized.len(), RECOVERY_CODE_COUNT);
    }
}
