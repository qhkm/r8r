//! IMAP mailbox polling trigger (feature `imap`).
//!
//! Polls an IMAP mailbox for matching messages (default: unread / `UNSEEN`)
//! and forwards each message onto the r8r event bus as an `imap.<mailbox>`
//! event. This powers email → AI workflows directly inside r8r: support email
//! classification, invoice extraction from supplier inboxes, CV screening,
//! email routing, and more.
//!
//! Pure-Rust stack: `async-imap` + `tokio-rustls` (no OpenSSL / native-tls),
//! keeping the default binary free of C dependencies.
//!
//! Behaviour notes:
//! - Each poll opens a fresh connection (SELECT → SEARCH → FETCH → STORE →
//!   LOGOUT). If the connection drops, the error is logged and the next poll
//!   retries — the loop never exits on transient failures.
//! - `UNSEEN` search + `mark_seen: true` is the safe default: a message fires
//!   exactly once. Switching `mark_seen` off re-delivers every poll (n8n
//!   behaves the same way), which is useful for idempotent pipelines.

use std::sync::Arc;
use std::time::Duration;

use async_imap::Client;
use base64::Engine as _;
use futures_util::StreamExt;
use mailparse::{DispositionType, MailAddr, ParsedMail, addrparse, dateparse, parse_mail};
use serde_json::{Value, json};
use tokio_rustls::rustls;
use tracing::{debug, warn};

use crate::triggers::event::EventMessage;
use crate::triggers::event_backend::EventBackend;

/// Composite of the stream traits async-imap needs (`async_imap::Client`
/// requires `Read + Write + Unpin + Debug`). Boxed directly as
/// `dyn AsyncRead + AsyncWrite` is E0225 on stable Rust (only one non-auto
/// trait object allowed), so we fan out through a helper trait.
trait AsyncStream:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug
{
}
impl<T> AsyncStream for T where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug
{
}

/// Boxed async read/write stream (TLS or plain TCP).
type BoxStream = Box<dyn AsyncStream>;

/// IMAP trigger configuration.
#[derive(Debug, Clone)]
pub struct ImapTriggerConfig {
    /// IMAP server hostname or IP, e.g. `imap.gmail.com`.
    pub server: String,
    /// Server port — defaults to 993 (TLS) or 143 (plain).
    pub port: u16,
    /// Login username (often the full email address).
    pub username: String,
    /// Login password / app password (never written to disk by r8r).
    pub password: String,
    /// Mailbox to watch, default `INBOX`.
    pub mailbox: String,
    /// Use implicit TLS (port 993). Disable only for LAN servers.
    pub tls: bool,
    /// Seconds between polls, default 60.
    pub poll_interval_secs: u64,
    /// IMAP SEARCH criteria, default `UNSEEN`.
    pub search: String,
    /// Stamp `\Seen` after publishing so messages fire once, default true.
    pub mark_seen: bool,
    /// Delete messages (`\Deleted` + EXPUNGE) after publishing, default false.
    pub delete_messages: bool,
    /// Include base64-encoded attachment bytes in the event payload (default
    /// false — metadata only, keeps payloads small).
    pub include_attachments: bool,
    /// Safety cap on the number of attachments captured per email, default 16.
    pub max_attachments: usize,
}

impl ImapTriggerConfig {
    /// IMAPS configuration (993 / implicit TLS).
    pub fn new(
        server: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            server: server.into(),
            port: 993,
            username: username.into(),
            password: password.into(),
            mailbox: "INBOX".into(),
            tls: true,
            poll_interval_secs: 60,
            search: "UNSEEN".into(),
            mark_seen: true,
            delete_messages: false,
            include_attachments: false,
            max_attachments: 16,
        }
    }

    /// Plain IMAP configuration (143 / no TLS) for local servers.
    pub fn plain(
        server: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let mut cfg = Self::new(server, username, password);
        cfg.tls = false;
        cfg.port = 143;
        cfg
    }
}

/// Run the IMAP polling trigger forever.
///
/// Publishes one `imap.<mailbox>` event per matching message onto the event
/// backend. Never returns on success; poll errors are logged and retried.
pub async fn run_imap_trigger(
    config: ImapTriggerConfig,
    backend: Arc<dyn EventBackend>,
) -> anyhow::Result<()> {
    debug!(
        server = %config.server,
        port = config.port,
        mailbox = %config.mailbox,
        "IMAP trigger started"
    );
    loop {
        match poll_once(&config).await {
            Ok(events) => {
                debug!(count = events.len(), "IMAP poll delivered messages");
                for event in events {
                    if let Err(err) = backend.publish(event).await {
                        warn!(%err, "IMAP event publish failed");
                    }
                }
            }
            Err(err) => warn!(%err, "IMAP poll failed (will retry)"),
        }
        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    }
}

/// One poll cycle: connect, select, search, fetch+parse, apply flags.
async fn poll_once(config: &ImapTriggerConfig) -> anyhow::Result<Vec<EventMessage>> {
    let client = connect(config).await?;
    let mut session = client
        .login(&config.username, &config.password)
        .await
        .map_err(|(err, _)| err)?;

    let mailbox = session.select(&config.mailbox).await?;
    debug!(%mailbox.exists, "IMAP mailbox selected");

    let seqs = session.search(&config.search).await?;
    if seqs.is_empty() {
        session.logout().await?;
        return Ok(Vec::new());
    }

    // Deterministic ascending order.
    let mut seq_list: Vec<u32> = seqs.into_iter().collect();
    seq_list.sort_unstable();
    let seq_set: Vec<String> = seq_list.iter().map(u32::to_string).collect();

    // Fetch full raw messages; scope the stream so `session` is free again for
    // STORE / EXPUNGE afterwards. Raw bytes are owned because `ParsedMail`
    // borrows its input in mailparse 0.15 — parse per-message downstream.
    let mut fetched: Vec<(Option<u32>, Vec<u8>)> = Vec::with_capacity(seq_list.len());
    {
        let mut fetches = session.fetch(seq_set.join(","), "(UID RFC822)").await?;
        while let Some(fetch) = fetches.next().await {
            let fetch = fetch?;
            let uid = fetch.uid;
            let body = match fetch.body() {
                Some(bytes) => bytes.to_vec(),
                None => continue,
            };
            fetched.push((uid, body));
        }
    }

    let topic = imap_topic(&config.mailbox);
    let mut events = Vec::with_capacity(fetched.len());
    for (uid, body) in &fetched {
        let mail = match parse_mail(body) {
            Ok(mail) => mail,
            Err(err) => {
                warn!(%err, "skipping unparseable message");
                continue;
            }
        };
        let payload = mail_to_event(
            &mail,
            *uid,
            &config.mailbox,
            config.include_attachments,
            config.max_attachments,
        );
        events.push(EventMessage::new(topic.clone(), payload).with_source("imap"));

        let uid = uid.map(|u| u.to_string()).unwrap_or_default();
        if uid.is_empty() {
            continue;
        }
        if config.delete_messages {
            drain_responses(session.uid_store(&uid, "+FLAGS (\\Deleted)").await?).await?;
        } else if config.mark_seen {
            drain_responses(session.uid_store(&uid, "+FLAGS (\\Seen)").await?).await?;
        }
    }

    if config.delete_messages {
        drain_responses(session.expunge().await?).await?;
    }

    session.logout().await?;
    Ok(events)
}

/// TlsStream wrapper — connect to the server, optionally wrapping in TLS.
async fn connect(config: &ImapTriggerConfig) -> anyhow::Result<Client<BoxStream>> {
    let tcp = tokio::net::TcpStream::connect((config.server.as_str(), config.port)).await?;
    if !config.tls {
        return Ok(Client::new(Box::new(tcp)));
    }

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls_cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_cfg));
    let server_name = rustls::pki_types::ServerName::try_from(config.server.clone())
        .map_err(|err| anyhow::anyhow!("invalid IMAP server name '{}': {err}", config.server))?;
    let tls = connector.connect(server_name, tcp).await?;
    Ok(Client::new(Box::new(tls)))
}

/// Consume a STORE/EXPUNGE response stream so the connection stays in sync.
/// async-imap yields `impl Stream` that is NOT `Unpin`, so it is pinned here;
/// items are discarded after surfacing any server error.
async fn drain_responses<S, I, E>(stream: S) -> anyhow::Result<()>
where
    S: futures_util::Stream<Item = Result<I, E>>,
    E: std::fmt::Display,
{
    let mut stream = std::pin::pin!(stream);
    while let Some(item) = stream.next().await {
        let _ = item.map_err(|err| anyhow::anyhow!("{err}"))?;
    }
    Ok(())
}

/// Event topic for a mailbox, e.g. `imap.INBOX`.
pub fn imap_topic(mailbox: &str) -> String {
    format!("imap.{mailbox}")
}

/// Convert a parsed email into the JSON event payload.
pub fn mail_to_event(
    mail: &ParsedMail,
    uid: Option<u32>,
    mailbox: &str,
    include_attachments: bool,
    max_attachments: usize,
) -> Value {
    let headers: Vec<(String, String)> = mail
        .headers
        .iter()
        .map(|h| (h.get_key_ref().to_lowercase(), h.get_value()))
        .collect();
    let get = |name: &str| -> String {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };

    let from_raw = get("from");
    let (from_name, from_email) = first_address(&from_raw);
    let date_rfc3339 = parse_date(&get("date"));

    let mut text = String::new();
    let mut html = String::new();
    collect_bodies(mail, &mut text, &mut html);

    json!({
        "uid": uid,
        "mailbox": mailbox,
        "subject": get("subject"),
        "from": from_raw,
        "from_name": from_name,
        "from_email": from_email,
        "to": get("to"),
        "date": date_rfc3339.unwrap_or_else(|| get("date")),
        "message_id": get("message-id"),
        "in_reply_to": get("in-reply-to"),
        "text": text,
        "html": html,
        "attachments": collect_attachments(mail, include_attachments, max_attachments),
    })
}

/// Extract the first text/plain and first text/html bodies (recursively).
/// Attached files — including named inline parts — are never treated as body.
fn collect_bodies(part: &ParsedMail, text: &mut String, html: &mut String) {
    if is_attachment(part) {
        return;
    }
    if part.ctype.mimetype.starts_with("multipart/") {
        for sub in &part.subparts {
            collect_bodies(sub, text, html);
        }
        return;
    }
    match part.ctype.mimetype.as_str() {
        "text/html" if html.is_empty() => {
            if let Ok(body) = part.get_body() {
                *html = body;
            }
        }
        "text/plain" if text.is_empty() => {
            if let Ok(body) = part.get_body() {
                *text = body;
            }
        }
        _ => {}
    }
}

/// Heuristic: is this MIME part a file attachment rather than message content?
fn is_attachment(part: &ParsedMail) -> bool {
    let disp = part.get_content_disposition();

    if matches!(
        disp.disposition,
        DispositionType::Attachment | DispositionType::FormData
    ) {
        return true;
    }
    if disp.params.contains_key("filename")
        || part.ctype.params.contains_key("filename")
        || part.ctype.params.contains_key("name")
    {
        return true;
    }

    let top = part.ctype.mimetype.split('/').next().unwrap_or("text");
    top != "text" && top != "multipart"
}

/// Collect attachment metadata (and optionally base64 data) from the message.
fn collect_attachments(part: &ParsedMail, include_data: bool, max: usize) -> Vec<Value> {
    let mut out = Vec::new();
    walk_attachments(part, include_data, max, &mut out);
    out
}

fn walk_attachments(part: &ParsedMail, include_data: bool, max: usize, out: &mut Vec<Value>) {
    if out.len() >= max {
        return;
    }
    if part.ctype.mimetype.starts_with("multipart/") {
        for sub in &part.subparts {
            walk_attachments(sub, include_data, max, out);
        }
        return;
    }
    if !is_attachment(part) {
        return;
    }

    let disp = part.get_content_disposition();
    let filename = disp
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("filename"))
        .or_else(|| part.ctype.params.get("name"))
        .cloned()
        .unwrap_or_else(|| format!("attachment-{}", out.len() + 1));

    let mut item = json!({
        "filename": filename,
        "content_type": part.ctype.mimetype,
        "size": part.raw_bytes.len(),
    });
    if include_data {
        if let Ok(bytes) = part.get_body_raw() {
            item["data"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    out.push(item);
}

/// Best-effort RFC2822 → RFC3339 conversion. mailparse 0.15's `dateparse`
/// yields a Unix timestamp (i64); render it as UTC RFC3339 via chrono, which
/// is already a hard dependency of r8r.
fn parse_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let secs = dateparse(raw).ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Parse a `From`/`To` header into `(name, address)` — best effort, never
/// fails. Returns the first address that yields anything usable.
fn first_address(raw: &str) -> (String, String) {
    if raw.trim().is_empty() {
        return (String::new(), String::new());
    }
    let Ok(addrs) = addrparse(raw) else {
        return (String::new(), String::new());
    };

    let pick = |name: Option<&String>, addr: &str| -> Option<(String, String)> {
        let name = name.cloned().unwrap_or_default();
        let addr = addr.trim().to_string();
        if !name.is_empty() || !addr.is_empty() {
            Some((name, addr))
        } else {
            None
        }
    };

    for addr in addrs.iter() {
        match addr {
            MailAddr::Single(info) => {
                if let Some(ok) = pick(info.display_name.as_ref(), &info.addr) {
                    return ok;
                }
            }
            MailAddr::Group(group) => {
                for sub in &group.addrs {
                    if let Some(ok) = pick(sub.display_name.as_ref(), &sub.addr) {
                        return ok;
                    }
                }
            }
        }
    }
    (String::new(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN_EMAIL: &str = concat!(
        "From: \"Aisyah Rahman\" <aisyah@example.com>\r\n",
        "To: support@acme.com\r\n",
        "Subject: Invoice INV-2026-001\r\n",
        "Date: Mon, 17 Aug 2026 09:30:00 +0800\r\n",
        "Message-ID: <abc123@example.com>\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "Please find the latest invoice enclosed.\r\n"
    );

    const MULTIPART_EMAIL: &str = concat!(
        "From: vendor@supplier.com\r\n",
        "To: accounts@acme.com\r\n",
        "Subject: Quote Q-2026-077\r\n",
        "Date: Tue, 18 Aug 2026 10:00:02 +0800\r\n",
        "Message-ID: <multi1@supplier.com>\r\n",
        "Content-Type: multipart/mixed; boundary=\"BOUNDARY42\"\r\n",
        "\r\n",
        "--BOUNDARY42\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "Body text here.\r\n",
        "--BOUNDARY42\r\n",
        "Content-Type: application/pdf; name=\"invoice.pdf\"\r\n",
        "Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "JVBERi0xLjQK\r\n",
        "--BOUNDARY42--\r\n"
    );

    // Sample emails are const strings, so the borrow is `'static`.
    fn parse(raw: &'static str) -> ParsedMail<'static> {
        parse_mail(raw.as_bytes()).expect("sample email parses")
    }

    #[test]
    fn parses_plain_text_email() {
        let mail = parse(PLAIN_EMAIL);
        let event = mail_to_event(&mail, Some(42), "INBOX", false, 16);

        assert_eq!(event["uid"], 42);
        assert_eq!(event["mailbox"], "INBOX");
        assert_eq!(event["subject"], "Invoice INV-2026-001");
        assert_eq!(event["from_email"], "aisyah@example.com");
        assert_eq!(event["from_name"], "Aisyah Rahman");
        assert_eq!(event["message_id"], "<abc123@example.com>");
        assert!(event["date"].as_str().unwrap().starts_with("2026-08-17"));
        assert!(event["text"].as_str().unwrap().contains("latest invoice"));
        assert_eq!(event["html"], "");
        assert_eq!(event["attachments"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn extracts_html_alternative() {
        let raw = concat!(
            "From: a@b.com\r\n",
            "Content-Type: multipart/alternative; boundary=\"ALT\"\r\n",
            "\r\n",
            "--ALT\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain body.\r\n",
            "--ALT\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>HTML body.</p>\r\n",
            "--ALT--\r\n"
        );
        let event = mail_to_event(&parse(raw), None, "INBOX", false, 16);

        assert!(event["text"].as_str().unwrap().contains("Plain body"));
        assert!(
            event["html"]
                .as_str()
                .unwrap()
                .contains("<p>HTML body.</p>")
        );
    }

    #[test]
    fn detects_attachment_metadata_only() {
        let mail = parse(MULTIPART_EMAIL);
        let event = mail_to_event(&mail, None, "INBOX", false, 16);

        // Body extracted from the text part, not the PDF part.
        assert!(event["text"].as_str().unwrap().contains("Body text here."));
        assert_eq!(event["html"], "");

        let atts = event["attachments"].as_array().unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["filename"], "invoice.pdf");
        assert_eq!(atts[0]["content_type"], "application/pdf");
        assert!(atts[0]["size"].as_u64().unwrap() > 0);
        assert!(
            atts[0].get("data").is_none(),
            "data must be omitted by default"
        );
    }

    #[test]
    fn includes_attachment_data_when_enabled() {
        let mail = parse(MULTIPART_EMAIL);
        let event = mail_to_event(&mail, None, "INBOX", true, 16);

        let atts = event["attachments"].as_array().unwrap();
        let data = atts[0]["data"].as_str().unwrap();
        // Base64 of "JVBERi0xLjQK" decodes back to the PDF header bytes.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(decoded, b"%PDF-1.4\n");
    }

    #[test]
    fn respects_max_attachments_cap() {
        let raw = concat!(
            "From: a@b.com\r\n",
            "Content-Type: multipart/mixed; boundary=\"MM\"\r\n",
            "\r\n",
            "--MM\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Body.\r\n",
            "--MM\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Disposition: attachment; filename=\"one.bin\"\r\n",
            "\r\n",
            "AAAA\r\n",
            "--MM\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Disposition: attachment; filename=\"two.bin\"\r\n",
            "\r\n",
            "BBBB\r\n",
            "--MM--\r\n"
        );
        let event = mail_to_event(&parse(raw), None, "INBOX", false, 1);
        assert_eq!(event["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(event["attachments"][0]["filename"], "one.bin");
    }

    #[test]
    fn parses_group_and_quoted_addresses() {
        assert_eq!(
            first_address("\"Aisyah Rahman\" <aisyah@example.com>"),
            (
                "Aisyah Rahman".to_string(),
                "aisyah@example.com".to_string()
            )
        );
        assert_eq!(
            first_address("team: a@example.com, b@example.com;"),
            ("".to_string(), "a@example.com".to_string())
        );
        assert_eq!(first_address(""), (String::new(), String::new()));
    }

    #[test]
    fn topic_uses_mailbox() {
        assert_eq!(imap_topic("INBOX"), "imap.INBOX");
        assert_eq!(imap_topic("Work/Invoices"), "imap.Work/Invoices");
    }

    #[test]
    fn config_defaults() {
        let cfg = ImapTriggerConfig::new("imap.gmail.com", "user", "app-pass");
        assert_eq!(cfg.port, 993);
        assert!(cfg.tls);
        assert_eq!(cfg.mailbox, "INBOX");
        assert_eq!(cfg.poll_interval_secs, 60);
        assert_eq!(cfg.search, "UNSEEN");
        assert!(cfg.mark_seen);
        assert!(!cfg.delete_messages);
        assert!(!cfg.include_attachments);
        assert_eq!(cfg.max_attachments, 16);

        let plain = ImapTriggerConfig::plain("localhost", "u", "p");
        assert_eq!(plain.port, 143);
        assert!(!plain.tls);
    }
}
