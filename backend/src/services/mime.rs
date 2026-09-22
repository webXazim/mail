//! RFC 5322 / MIME message construction for outgoing mail (WS2.3). Pure and
//! byte-exact: every line is CRLF-terminated, display names and subjects use
//! RFC 2047 encoded-word encoding only when needed, and attachments become
//! base64 parts. The SMTP layer applies dot-stuffing on top of the bytes this
//! module produces.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chrono::Utc;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            name: None,
            email: email.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Outgoing {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Option<Address>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<Attachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// RFC 8058 one-click unsubscribe URL. Emitted only for single-recipient
    /// mail, since the header cannot carry a token per recipient.
    pub list_unsubscribe: Option<String>,
    /// Left of the angle brackets in `Message-ID`, e.g. the raw uuid.
    pub message_id_local: String,
    /// Domain of the `From` address; forms `<local@domain>`.
    pub domain: String,
}

impl Outgoing {
    pub fn message_id(&self) -> String {
        format!("<{}@{}>", self.message_id_local, self.domain)
    }

    /// Serialize the full message. `Bcc` never appears in the headers (it is
    /// envelope-only per RFC 5322); the sender's own copy rides for free via
    /// the same bytes when the SMTP layer adds the sender to the recipients.
    pub fn build(&self) -> Result<Vec<u8>, String> {
        let mut out = String::new();
        out.push_str(&format!("From: {}\r\n", format_address(&self.from)));
        out.push_str(&format!("To: {}\r\n", format_address_list(&self.to)));

        if !self.cc.is_empty() {
            out.push_str(&format!("Cc: {}\r\n", format_address_list(&self.cc)));
        }
        if let Some(reply_to) = &self.reply_to {
            out.push_str(&format!("Reply-To: {}\r\n", format_address(reply_to)));
        }
        out.push_str(&format!("Subject: {}\r\n", encode_subject(&self.subject)));
        out.push_str(&format!("Date: {}\r\n", Utc::now().to_rfc2822()));
        out.push_str(&format!("Message-ID: {}\r\n", self.message_id()));

        if !self.references.is_empty() {
            out.push_str(&format!("References: {}\r\n", self.references.join(" ")));
        }
        if let Some(irt) = &self.in_reply_to {
            let irt = irt.trim();
            if !irt.is_empty() {
                out.push_str(&format!("In-Reply-To: {irt}\r\n"));
            }
        }
        if let Some(url) = self
            .list_unsubscribe
            .as_deref()
            .map(|u| u.replace(['\r', '\n'], ""))
            .filter(|u| !u.trim().is_empty())
        {
            out.push_str(&format!("List-Unsubscribe: <{}>\r\n", url.trim()));
            out.push_str("List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n");
        }
        out.push_str("MIME-Version: 1.0\r\n");
        out.push_str(&self.body());

        Ok(crlf(&out).into_bytes())
    }

    fn body(&self) -> String {
        let html = self.body_html.as_deref().filter(|h| !h.trim().is_empty());
        let mut s = String::new();
        let txt = text_part(&self.body_text);
        let html_txt = html.map(html_part);

        if self.attachments.is_empty() {
            match html_txt {
                Some(html_txt) => {
                    let boundary = fresh_boundary();
                    s.push_str(&format!(
                        "Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n"
                    ));
                    s.push_str(&format!("--{boundary}\r\n{txt}"));
                    s.push_str(&format!("--{boundary}\r\n{html_txt}"));
                    s.push_str(&format!("--{boundary}--\r\n"));
                }
                None => {
                    s.push_str("Content-Type: text/plain; charset=utf-8\r\n");
                    s.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
                    s.push_str(&format!("{}\r\n", self.body_text));
                }
            }
            return s;
        }

        let mixed = fresh_boundary();
        s.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=\"{mixed}\"\r\n\r\n"
        ));
        s.push_str(&format!("--{mixed}\r\n"));
        match html_txt {
            Some(html_txt) => {
                let alt = fresh_boundary();
                s.push_str(&format!(
                    "Content-Type: multipart/alternative; boundary=\"{alt}\"\r\n\r\n"
                ));
                s.push_str(&format!("--{alt}\r\n{txt}"));
                s.push_str(&format!("--{alt}\r\n{html_txt}"));
                s.push_str(&format!("--{alt}--\r\n"));
            }
            None => s.push_str(&txt),
        }
        s.push_str(&format!(
            "--{mixed}\r\n{}",
            attachment_part(&self.attachments[0])
        ));
        for att in &self.attachments[1..] {
            s.push_str(&format!("--{mixed}\r\n{}", attachment_part(att)));
        }
        s.push_str(&format!("--{mixed}--\r\n"));
        s
    }
}

fn text_part(text: &str) -> String {
    format!(
        "Content-Type: text/plain; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\
         \r\n\
         {text}\r\n"
    )
}

fn html_part(html: &str) -> String {
    format!(
        "Content-Type: text/html; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\
         \r\n\
         {html}\r\n"
    )
}

fn attachment_part(att: &Attachment) -> String {
    let filename = quote_filename(&att.filename);
    let content_type = if att.content_type.trim().is_empty() {
        "application/octet-stream".to_string()
    } else {
        att.content_type.clone()
    };
    let encoded = B64.encode(&att.bytes);
    let mut wrapped = String::new();
    for chunk in encoded.as_bytes().chunks(76) {
        wrapped.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        wrapped.push_str("\r\n");
    }
    format!(
        "Content-Type: {content_type}; name=\"{filename}\"\r\n\
         Content-Disposition: attachment; filename=\"{filename}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {wrapped}"
    )
}

/// Normalize any mix of `\n` / `\r\n` line endings to strict CRLF.
fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn fresh_boundary() -> String {
    format!("csmail{}", Uuid::new_v4().as_simple())
}

fn format_address_list(addrs: &[Address]) -> String {
    addrs
        .iter()
        .map(format_address)
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_address(a: &Address) -> String {
    match &a.name {
        Some(name) if !name.trim().is_empty() => {
            format!("{} <{}>", rfc2047_name(name), a.email)
        }
        _ => a.email.clone(),
    }
}

/// RFC 2047 encode a display name unless it is already plain printable ASCII
/// (which we then quote only when it must be).
fn rfc2047_name(name: &str) -> String {
    let needs_encoding = name
        .bytes()
        .any(|b| !(b' '..=b'~').contains(&b) || b == b'=' || b == b'?');
    if needs_encoding {
        return format!("=?utf-8?B?{}?=", B64.encode(name.as_bytes()));
    }
    if name.is_empty() || name.contains(' ') || name.contains(',') {
        format!("\"{}\"", name.replace('"', "\\\""))
    } else {
        name.to_owned()
    }
}

/// Encode a subject line: RFC 2047 encoded words for any token that isn't
/// printable ASCII, raw otherwise.
fn encode_subject(subject: &str) -> String {
    subject
        .split_whitespace()
        .map(|w| {
            if w.bytes().any(|b| !(b' '..=b'~').contains(&b)) {
                format!("=?utf-8?B?{}?=", B64.encode(w.as_bytes()))
            } else {
                w.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Minimal RFC 5322 quoted-string escaping for filenames.
fn quote_filename(name: &str) -> String {
    name.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Outgoing {
        Outgoing {
            from: Address::new("alice@example.com"),
            to: vec![Address::new("bob@example.com")],
            cc: Vec::new(),
            reply_to: None,
            subject: "hello".into(),
            body_text: "line one\nline two".into(), // bare \n on purpose
            body_html: None,
            attachments: Vec::new(),
            in_reply_to: None,
            references: Vec::new(),
            list_unsubscribe: None,
            message_id_local: "abc123".into(),
            domain: "example.com".into(),
        }
    }

    fn lines_are_crlf(bytes: &[u8]) -> bool {
        bytes.windows(2).all(|w| w[1] != b'\n' || w[0] == b'\r')
    }

    #[test]
    fn plain_message_headers_and_crlf() {
        let bytes = sample().build().unwrap();
        let msg = String::from_utf8(bytes).unwrap();
        assert!(msg.starts_with("From: alice@example.com\r\n"));
        assert!(msg.contains("\r\nTo: bob@example.com\r\n"));
        assert!(msg.contains("\r\nSubject: hello\r\n"));
        assert!(msg.contains("\r\nMessage-ID: <abc123@example.com>\r\n"));
        assert!(msg.contains("MIME-Version: 1.0\r\n"));
        assert!(msg.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(msg.ends_with("line two\r\n"));
        assert!(lines_are_crlf(msg.as_bytes()));
    }

    #[test]
    fn bare_newlines_normalized_to_crlf() {
        let msg = String::from_utf8(sample().build().unwrap()).unwrap();
        assert!(msg.contains("line one\r\nline two\r\n"));
        assert!(!msg.contains("line one\nline two"));
    }

    #[test]
    fn html_body_uses_multipart_alternative_and_is_ascii_safe() {
        let mut m = sample();
        m.body_html = Some("<p>hi</p>".into());
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(msg.contains("multipart/alternative"));
        assert!(msg.contains("Content-Type: text/plain; charset=utf-8"));
        assert!(msg.contains("Content-Type: text/html; charset=utf-8"));
        assert!(msg.contains("<p>hi</p>"));
        assert!(msg.is_ascii());
    }

    #[test]
    fn attachment_part_is_base64_with_disposition() {
        let mut m = sample();
        m.attachments.push(Attachment {
            filename: "report.pdf".into(),
            content_type: "application/pdf".into(),
            bytes: b"\x25\x50\x44\x46-hello".to_vec(),
        });
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(msg.contains("multipart/mixed"));
        assert!(msg.contains("Content-Disposition: attachment; filename=\"report.pdf\""));
        assert!(msg.contains(B64.encode(b"\x25\x50\x44\x46-hello").as_str()));
    }

    #[test]
    fn unicode_display_name_rfc2047_encoded() {
        let mut m = sample();
        m.from = Address {
            name: Some("Rüter".into()),
            email: "alice@example.com".into(),
        };
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(msg.starts_with("From: =?utf-8?B?"));
        assert!(msg.contains(" <alice@example.com>\r\n"));
    }

    #[test]
    fn header_injection_via_display_name_is_neutralized() {
        let mut m = sample();
        m.from = Address {
            name: Some("Sneaky\r\nBcc: evil@example.com".into()),
            email: "alice@example.com".into(),
        };
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        let lower = msg.to_ascii_lowercase();
        assert!(!lower.contains("\r\nbcc:"));
        assert!(!lower.contains("evil@example.com"));
        // The control characters force RFC 2047 encoding, so the name is an
        // encoded-word on one line, not extra headers.
        assert!(msg.contains("From: =?utf-8?B?"));
    }

    #[test]
    fn reply_to_is_emitted_from_authoritative_identity() {
        let mut m = sample();
        m.reply_to = Some(Address::new("replies@example.net"));
        let text = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(text.contains("Reply-To: replies@example.net\r\n"));
    }

    #[test]
    fn references_and_in_reply_to_present_when_set() {
        let mut m = sample();
        m.in_reply_to = Some("<prev@example.com>".into());
        m.references = vec!["<p1@example.com>".into(), "<p2@example.com>".into()];
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(msg.contains("References: <p1@example.com> <p2@example.com>\r\n"));
        assert!(msg.contains("In-Reply-To: <prev@example.com>\r\n"));
    }

    #[test]
    fn list_unsubscribe_headers_emitted_when_set() {
        let mut m = sample();
        m.list_unsubscribe = Some("https://mail.example.com/api/unsubscribe?token=abc".into());
        let msg = String::from_utf8(m.build().unwrap()).unwrap();
        assert!(msg.contains(
            "List-Unsubscribe: <https://mail.example.com/api/unsubscribe?token=abc>\r\n"
        ));
        assert!(msg.contains("List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n"));

        let plain = String::from_utf8(sample().build().unwrap()).unwrap();
        assert!(!plain.contains("List-Unsubscribe"));
    }

    #[test]
    fn empty_attachments_cap_ok() {
        let bytes = sample().build().unwrap();
        assert!(!bytes.is_empty());
        assert!(lines_are_crlf(&bytes));
    }
}
