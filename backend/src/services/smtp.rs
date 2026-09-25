//! Minimal async SMTP submission client. Port 465 uses verified implicit TLS;
//! plaintext port 25 remains available only for unauthenticated development
//! relays. Credentials are never sent over a plaintext connection.

use std::collections::HashSet;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls::{pki_types::ServerName, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_PORT: u16 = 25;

/// Immutable SMTP endpoint config. Port 465 is implicit TLS. Empty credentials
/// permit an unauthenticated relay only when the server policy allows it.
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub timeout: Duration,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: "mail".into(),
            port: DEFAULT_PORT,
            username: String::new(),
            password: String::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

fn env_non_empty(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

impl SmtpConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        let host = env_non_empty("CS_MAIL_SMTP_HOST");
        if !host.is_empty() {
            cfg.host = host;
        }
        if let Ok(port) = std::env::var("CS_MAIL_SMTP_PORT") {
            if let Ok(p) = port.trim().parse() {
                cfg.port = p;
            }
        }
        cfg.username = env_non_empty("CS_MAIL_SMTP_USERNAME");
        cfg.password = env_non_empty("CS_MAIL_SMTP_PASSWORD");
        if let Ok(secs) = std::env::var("CS_MAIL_SMTP_TIMEOUT_SECS") {
            if let Ok(t) = secs.trim().parse::<u64>() {
                cfg.timeout = Duration::from_secs(t);
            }
        }
        cfg
    }
}

/// Deliver one already-built MIME message. `from` is the envelope sender,
/// `to` the full recipient list (add the sender yourself for a self-copy).
/// Returns the relay's final reply text on success.
pub async fn send(
    config: &SmtpConfig,
    from: &str,
    to: &[String],
    message: &[u8],
) -> Result<String, String> {
    if to.is_empty() {
        return Err("no recipients".into());
    }
    if from.is_empty() || !from.contains('@') {
        return Err(format!("invalid envelope sender {from:?}"));
    }

    if config.username.is_empty() != config.password.is_empty() {
        return Err("SMTP username and password must be set together".into());
    }
    if !config.username.is_empty() && config.port != 465 {
        return Err("SMTP credentials require verified implicit TLS on port 465".into());
    }

    let tcp = tokio::time::timeout(
        config.timeout,
        TcpStream::connect((config.host.as_str(), config.port)),
    )
    .await
    .map_err(|_| format!("SMTP connect to {}:{} timed out", config.host, config.port))?
    .map_err(|e| {
        format!(
            "SMTP connect to {}:{} failed: {e}",
            config.host, config.port
        )
    })?;
    let stream: Box<dyn SmtpStream> = if config.port == 465 {
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = ServerName::try_from(config.host.clone())
            .map_err(|_| "SMTP TLS hostname is invalid".to_string())?;
        let encrypted = tokio::time::timeout(
            config.timeout,
            TlsConnector::from(std::sync::Arc::new(tls)).connect(server_name, tcp),
        )
        .await
        .map_err(|_| "SMTP TLS handshake timed out".to_string())?
        .map_err(|e| format!("SMTP TLS certificate/handshake failed: {e}"))?;
        Box::new(encrypted)
    } else {
        Box::new(tcp)
    };
    let (read, write) = tokio::io::split(stream);
    let mut conn = Connection {
        reader: BufReader::new(read),
        writer: write,
    };

    let banner = conn.reply(config.timeout).await?;
    require_code(&banner, 2, "greeting")?;

    // RFC 5321: the EHLO argument should be the client's FQDN. Some servers
    // (Stalwart) reject bare single-label names, so qualify hosts like `mail`.
    let ehlo = if config.host.contains('.') {
        config.host.clone()
    } else {
        format!("{}.local", config.host)
    };
    let ehlo = conn.cmd(&format!("EHLO {ehlo}"), config.timeout).await?;
    require_code(&ehlo, 2, "EHLO")?;
    let features = ehlo_features(&ehlo);

    if !config.username.is_empty() {
        if !features.contains("auth") {
            return Err("relay advertises no AUTH but credentials were configured".into());
        }
        let first = conn.cmd("AUTH LOGIN", config.timeout).await?;
        require_code(&first, 3, "AUTH LOGIN")?;
        let user = conn
            .cmd(&B64.encode(config.username.as_bytes()), config.timeout)
            .await?;
        require_code(&user, 3, "AUTH username")?;
        let pass = conn
            .cmd(&B64.encode(config.password.as_bytes()), config.timeout)
            .await?;
        require_code(&pass, 2, "AUTH")?;
    }

    let mail = conn
        .cmd(&format!("MAIL FROM:<{from}>"), config.timeout)
        .await?;
    require_code(&mail, 2, "MAIL FROM")?;

    for recipient in to {
        if recipient.is_empty() {
            continue;
        }
        let rcpt = conn
            .cmd(&format!("RCPT TO:<{recipient}>"), config.timeout)
            .await?;
        require_code(&rcpt, 2, "RCPT TO")?;
    }
    if to.iter().all(|r| r.is_empty()) {
        return Err("no valid recipients".into());
    }

    let data = conn.cmd("DATA", config.timeout).await?;
    require_code(&data, 3, "DATA")?;

    let mut payload = dot_stuff(message);
    if !payload.ends_with(b"\n") {
        payload.extend_from_slice(b"\r\n");
    }
    payload.extend_from_slice(b".\r\n");
    conn.writer
        .write_all(&payload)
        .await
        .map_err(|e| format!("SMTP DATA write failed: {e}"))?;
    conn.writer
        .flush()
        .await
        .map_err(|e| format!("SMTP DATA flush failed: {e}"))?;

    let done = conn.reply(config.timeout).await?;
    require_code(&done, 2, "DATA completion")?;

    let _ = conn.cmd("QUIT", config.timeout).await; // best effort
    Ok(done.lines.join(" ").trim().to_string())
}

trait SmtpStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SmtpStream for T {}

struct Connection {
    reader: BufReader<tokio::io::ReadHalf<Box<dyn SmtpStream>>>,
    writer: tokio::io::WriteHalf<Box<dyn SmtpStream>>,
}

impl Connection {
    /// Read one full reply — the banner line plus any continuation lines —
    /// stopping at the line whose 4th character is a space (RFC 5321 §4.2.1).
    async fn reply(&mut self, timeout: Duration) -> Result<Reply, String> {
        let mut lines = Vec::new();
        let mut buf = Vec::new();
        loop {
            let n = tokio::time::timeout(timeout, self.reader.read_until(b'\n', &mut buf))
                .await
                .map_err(|_| "SMTP read timed out".to_string())?
                .map_err(|e| format!("SMTP read failed: {e}"))?;
            if n == 0 {
                return Err("SMTP connection closed by server".into());
            }
            let raw = String::from_utf8_lossy(&buf)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            buf.clear();
            if raw.len() < 3 || !raw.as_bytes()[..3].iter().all(u8::is_ascii_digit) {
                return Err(format!("SMTP protocol error: {raw:?}"));
            }
            let code: Result<u16, _> = raw[..3].parse();
            let separator = raw.as_bytes().get(3).copied();
            if !matches!(separator, Some(b' ' | b'-')) {
                return Err(format!("SMTP protocol error: {raw:?}"));
            }
            let is_last = separator == Some(b' ');
            // The fourth byte is the SMTP reply separator, not reply text.
            // Keeping the '-' from `250-AUTH` made capability detection see
            // "-auth" and reject valid authenticated Stalwart sessions.
            lines.push(raw.get(4..).unwrap_or_default().trim().to_string());
            if let (Ok(code), true) = (code, is_last) {
                return Ok(Reply { code, lines });
            }
        }
    }

    async fn cmd(&mut self, line: &str, timeout: Duration) -> Result<Reply, String> {
        self.writer
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|e| format!("SMTP write failed: {e}"))?;
        self.writer
            .flush()
            .await
            .map_err(|e| format!("SMTP flush failed: {e}"))?;
        self.reply(timeout).await
    }
}

#[derive(Debug)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

fn require_code(reply: &Reply, class: u16, step: &str) -> Result<(), String> {
    let good = reply.code / 100 == class;
    if good {
        Ok(())
    } else {
        Err(format!(
            "{step}: SMTP {} {:?}",
            reply.code,
            reply.lines.join(" ")
        ))
    }
}

/// RFC 5321 §4.5.2 transparency: a line beginning with `.` gains a leading
/// `.` so the DATA terminator alone on a line is unambiguous.
fn dot_stuff(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 16);
    let mut at_line_start = true;
    for &b in bytes {
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        at_line_start = b == b'\n';
    }
    out
}

/// Fold an EHLO reply (banner + feature lines) into a lowercase token set so
/// capability checks like `features.contains("auth")` are cheap and spelled
/// with the (lower) names RFC 4954/3207/1869 use.
fn ehlo_features(reply: &Reply) -> HashSet<String> {
    reply
        .lines
        .iter()
        .flat_map(|l| l.trim_start_matches(['-', ' ']).split_whitespace())
        .map(|t| t.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_stuffs_leading_dots_only() {
        let input = b"line one\n.dot at start\nkeep .dot\nend.";
        let out = dot_stuff(input);
        assert_eq!(out, b"line one\n..dot at start\nkeep .dot\nend.");
    }

    #[test]
    fn dot_stuff_empty_and_terminator_safe() {
        assert_eq!(dot_stuff(b""), b"");
        let out = dot_stuff(b".\r\n");
        assert_eq!(out, b"..\r\n");
    }

    #[test]
    fn ehlo_features_lowercase_set() {
        let reply = Reply {
            code: 250,
            lines: vec![
                "mail.crescentsphere.com".into(),
                "SIZE 104857600".into(),
                "STARTTLS".into(),
                "8BITMIME".into(),
            ],
        };
        let features = ehlo_features(&reply);
        assert!(features.contains("starttls"));
        assert!(features.contains("size"));
        assert!(features.contains("104857600"));
        assert!(features.contains("8bitmime"));
        assert!(!features.contains("auth"));
    }

    #[tokio::test]
    async fn reply_removes_smtp_continuation_markers() {
        let (client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server
                .write_all(b"250-mail.example\r\n250-AUTH PLAIN LOGIN\r\n250 SIZE 1024\r\n")
                .await
                .unwrap();
        });
        let stream: Box<dyn SmtpStream> = Box::new(client);
        let (read, write) = tokio::io::split(stream);
        let mut connection = Connection {
            reader: BufReader::new(read),
            writer: write,
        };

        let reply = connection.reply(Duration::from_secs(1)).await.unwrap();
        assert_eq!(reply.lines, vec!["mail.example", "AUTH PLAIN LOGIN", "SIZE 1024"]);
        assert!(ehlo_features(&reply).contains("auth"));
    }

    #[test]
    fn require_code_accepts_only_requested_class() {
        let four = Reply {
            code: 354,
            lines: vec!["go ahead".into()],
        };
        let five = Reply {
            code: 503,
            lines: vec!["AUTH not allowed".into()],
        };
        assert!(require_code(&four, 3, "DATA").is_ok());
        assert!(require_code(&five, 2, "MAIL FROM").is_err());
    }

    #[tokio::test]
    async fn refuses_credentials_without_tls_before_connecting() {
        let config = SmtpConfig {
            host: "127.0.0.1".into(),
            port: 25,
            username: "service".into(),
            password: "secret".into(),
            timeout: Duration::from_secs(1),
        };
        let result = send(
            &config,
            "sender@example.com",
            &["to@example.com".into()],
            b"test",
        )
        .await;
        assert!(result
            .unwrap_err()
            .contains("require verified implicit TLS"));
    }
}
