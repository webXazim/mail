//! Attachment validation (WS2.4). Compose accepts attachments inline as
//! base64, so the API is the last line of defence before hostile bytes reach
//! the relay: every payload is bounded by size caps and screened against a
//! MIME allow-list plus an extension block-list. The download path reuses the
//! cap so what can be sent is what can be fetched back.

use axum::http::StatusCode;

use crate::error::ApiError;

/// Absolute ceiling for a single decoded attachment (100 MiB, the largest
/// plan cap). Per-plan caps are passed to [`validate_upload`]; this bounds
/// anything the caller forgets to scope.
pub const MAX_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;

/// Largest attachment decoded and streamed back to a client. Matches the
/// absolute upload ceiling so every attachment the API accepts is fetchable.
pub const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

/// Combined cap across every attachment on one message.
pub const MAX_TOTAL_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// Filename extensions that are never allowed, whatever the declared content
/// type. These either auto-execute or are trivial to trick a recipient into
/// running.
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "msi", "msp", "cpl", "hta", "vbs", "vbe", "js",
    "jse", "wsf", "wsh", "ps1", "psm1", "jar", "app", "dmg", "pkg", "deb", "rpm", "sh", "bash",
    "zsh", "py", "rb", "pl", "php", "lnk", "reg", "scf", "inf", "apk",
];

/// Exact MIME types accepted. `text/*` and `image/*` are matched by prefix,
/// except for the script-capable `image/svg+xml`.
const ALLOWED_EXACT: &[&str] = &[
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.presentation",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/json",
    "application/rtf",
    "message/rfc822",
];

/// A normalised, screen-cleared attachment ready to be encoded into MIME.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedAttachment {
    pub filename: String,
    pub content_type: String,
}

fn extension_of(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// `Content-Type` headers may carry parameters (`; name=...`); compare only
/// the type/subtype, lower-cased, and default empty values to octet-stream.
pub fn normalise_type(content_type: &str) -> String {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if base.is_empty() {
        "application/octet-stream".to_string()
    } else {
        base
    }
}

pub fn is_allowed_type(content_type: &str) -> bool {
    let base = normalise_type(content_type);
    if base == "image/svg+xml" {
        return false;
    }
    if let Some(rest) = base.strip_prefix("text/") {
        return !rest.is_empty();
    }
    if let Some(rest) = base.strip_prefix("image/") {
        return !rest.is_empty();
    }
    ALLOWED_EXACT.contains(&base.as_str())
}

/// Reject payloads whose leading bytes reveal an executable or script, even
/// when the declared filename and MIME type claim otherwise.
fn looks_executable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ") || bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!")
}

/// Validate one outbound attachment against `max_bytes` (the caller's plan
/// cap). Returns the safe filename and content type to encode, or a typed 4xx
/// describing the rejection.
pub fn validate_upload(
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<ValidatedAttachment, ApiError> {
    if bytes.len() > max_bytes {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment_too_large",
            format!(
                "Attachments may be at most {} MiB on your plan",
                max_bytes / (1024 * 1024)
            ),
        ));
    }

    let filename = filename
        .replace(['\r', '\n'], " ")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let filename = if filename.is_empty() {
        "attachment".to_string()
    } else {
        filename
    };

    let ext = extension_of(&filename);
    if !ext.is_empty() && BLOCKED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_blocked",
            format!("Files ending in .{ext} are not allowed as attachments"),
        ));
    }
    if looks_executable(bytes) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_blocked",
            "This file appears to be an executable and cannot be attached",
        ));
    }

    let content_type = normalise_type(content_type);
    if !is_allowed_type(&content_type) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "attachment_type_not_allowed",
            format!("Attachments of type {content_type} are not allowed"),
        ));
    }

    Ok(ValidatedAttachment {
        filename,
        content_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience wrapper using the absolute ceiling; plan-specific caps are
    /// covered by `rejects_over_plan_cap`.
    fn check(name: &str, mime: &str, bytes: &[u8]) -> Result<ValidatedAttachment, ApiError> {
        validate_upload(name, mime, bytes, MAX_ATTACHMENT_BYTES)
    }

    #[test]
    fn allows_common_document_and_image_types() {
        for (name, mime) in [
            ("report.pdf", "application/pdf"),
            ("photo.png", "image/png"),
            (
                "sheet.xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            ("notes.txt", "text/plain; charset=utf-8"),
            ("archive.zip", "application/zip"),
        ] {
            assert!(
                check(name, mime, b"hello").is_ok(),
                "{name} / {mime} should be allowed"
            );
        }
    }

    #[test]
    fn rejects_dangerous_extension_even_with_benign_type() {
        let err = check("invoice.pdf.exe", "application/pdf", b"hello").unwrap_err();
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(err.message.contains("not allowed"));
    }

    #[test]
    fn rejects_svg_and_unknown_binary_types() {
        assert!(check("logo.svg", "image/svg+xml", b"<svg/>").is_err());
        assert!(check("blob.bin", "application/x-msdownload", b"\x01\x02").is_err());
    }

    #[test]
    fn rejects_oversized_attachment() {
        let big = vec![0u8; MAX_ATTACHMENT_BYTES + 1];
        let err = check("big.bin", "application/pdf", &big).unwrap_err();
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn rejects_over_plan_cap() {
        let err = validate_upload("big.pdf", "application/pdf", b"12345", 4).unwrap_err();
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(err.message.contains("plan"));
    }

    #[test]
    fn rejects_executable_masquerading_as_pdf() {
        let err = check("readme.pdf", "application/pdf", b"MZ\x90\x00").unwrap_err();
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn strips_path_traversal_from_filename() {
        let v = check("../../etc/passwd.txt", "text/plain", b"x").unwrap();
        assert_eq!(v.filename, "passwd.txt");
    }

    #[test]
    fn normalises_content_type_parameters() {
        let v = check("a.txt", "TEXT/Plain; charset=utf-8", b"x").unwrap();
        assert_eq!(v.content_type, "text/plain");
    }

    #[test]
    fn missing_type_defaults_to_octet_stream_but_is_rejected() {
        let err = check("a.dat", "", b"x").unwrap_err();
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}
