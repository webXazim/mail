//! Deliverability policy (WS7.2). Pure helpers for normalising addresses and
//! reading RFC 3464 delivery status reports: only *hard* failures (5.x) and
//! spam complaints belong on the permanent suppression list, because soft
//! failures (4.x, mailbox full, greylisting) clear on their own.

/// Canonical form used as the suppression-list key.
pub fn normalize(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// A delivery status code is a hard failure when its enhanced class is 5.
pub fn is_hard_bounce(status: &str) -> bool {
    status.trim().starts_with('5')
}

/// Extract the permanently-failed recipients from a DSN body. Best-effort:
/// pairs `Final-Recipient` lines with the following `Status`, and falls back to
/// including recipients when an explicit `Action: failed` is present.
pub fn parse_dsn(text: &str) -> Vec<String> {
    let mut recipients: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    let mut pending_status: Option<String> = None;
    let mut pending_failed = false;

    let flush = |recips: &mut Vec<String>,
                 pending: &mut Option<String>,
                 status: &mut Option<String>,
                 failed: &mut bool| {
        if let Some(addr) = pending.take() {
            let hard = status.take().map(|s| is_hard_bounce(&s)).unwrap_or(*failed);
            if hard {
                let addr = normalize(&addr);
                if !addr.is_empty() && !recips.contains(&addr) {
                    recips.push(addr);
                }
            }
        }
        *status = None;
        *failed = false;
    };

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("Final-Recipient:")
            .or_else(|| line.strip_prefix("final-recipient:"))
        {
            flush(
                &mut recipients,
                &mut pending,
                &mut pending_status,
                &mut pending_failed,
            );
            pending = extract_address(rest);
        } else if let Some(rest) = line
            .strip_prefix("Status:")
            .or_else(|| line.strip_prefix("status:"))
        {
            pending_status = Some(rest.trim().to_string());
        } else if let Some(rest) = line
            .strip_prefix("Action:")
            .or_else(|| line.strip_prefix("action:"))
        {
            if rest.trim().eq_ignore_ascii_case("failed") {
                pending_failed = true;
            }
        }
    }
    flush(
        &mut recipients,
        &mut pending,
        &mut pending_status,
        &mut pending_failed,
    );
    recipients
}

/// `rfc822; user@example.com` (or a bare address) -> `user@example.com`.
fn extract_address(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let after_type = rest.rsplit(';').next().unwrap_or(rest).trim();
    let addr = after_type
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    if addr.contains('@') && !addr.contains(char::is_whitespace) {
        Some(addr.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_vs_soft_classification() {
        assert!(is_hard_bounce("5.1.1"));
        assert!(is_hard_bounce("5.7.1"));
        assert!(is_hard_bounce("5"));
        assert!(!is_hard_bounce("4.2.2"));
        assert!(!is_hard_bounce("2.0.0"));
        assert!(!is_hard_bounce(""));
    }

    #[test]
    fn parses_hard_bounce_from_dsn() {
        let dsn = "Reporting-MTA: dns; mail.example.com\r\n\
                   Final-Recipient: rfc822; Gone@Example.COM\r\n\
                   Action: failed\r\n\
                   Status: 5.1.1\r\n\
                   Diagnostic-Code: smtp; 550 5.1.1 user unknown\r\n";
        assert_eq!(parse_dsn(dsn), vec!["gone@example.com".to_string()]);
    }

    #[test]
    fn ignores_soft_bounces_and_noise() {
        let dsn = "Final-Recipient: rfc822; busy@example.com\r\n\
                   Action: delayed\r\n\
                   Status: 4.2.2\r\n\
                   Diagnostic-Code: smtp; 452 mailbox full\r\n";
        assert!(parse_dsn(dsn).is_empty());
        assert!(parse_dsn("nothing to see").is_empty());
    }

    #[test]
    fn handles_multiple_recipients() {
        let dsn = "Final-Recipient: rfc822; a@example.com\nStatus: 5.1.1\n\
                   Final-Recipient: rfc822; b@example.com\nStatus: 4.4.1\n\
                   Final-Recipient: rfc822; c@example.com\nStatus: 5.2.2\n";
        assert_eq!(
            parse_dsn(dsn),
            vec!["a@example.com".to_string(), "c@example.com".to_string()]
        );
    }

    #[test]
    fn normalizes_case_and_space() {
        assert_eq!(normalize("  Bob@Example.COM "), "bob@example.com");
    }
}
