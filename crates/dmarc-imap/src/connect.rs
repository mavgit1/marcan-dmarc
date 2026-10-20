//! Connecting to the mailbox: staged TCP / TLS / LOGIN with plain-language
//! errors, and the interactive "connect" probe used by the settings UI
//! (login, list folders, examine one folder).

use crate::config::{ImapConfig, ImapSecurity};
use async_imap::imap_proto::{Response, Status};
use async_imap::types::NameAttribute;
use async_native_tls::TlsConnector;
use futures::TryStreamExt;
use std::fmt;
use std::io;
use std::time::Duration;
use tokio::net::TcpStream;
use tracing::{info, warn};

/// Upper bound for the TCP connect to one resolved address.
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound for the server's greeting before `STARTTLS`. A server that
/// expects TLS from the first byte sends nothing and would wait forever.
const GREETING_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound for one complete [`connect_mailbox`] probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) type ImapSession = async_imap::Session<async_native_tls::TlsStream<TcpStream>>;

/// Which step of reaching the mailbox failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectStage {
    /// The host name did not resolve.
    Dns,
    /// TCP connect was refused or timed out.
    Tcp,
    /// The server did not accept `STARTTLS` (it does not offer it).
    StartTls,
    /// The TLS handshake failed.
    Tls,
    /// The server rejected LOGIN.
    Auth,
    /// Anything else (listing folders, a dropped connection, an overall timeout).
    Other,
}

/// A failed connection: a plain-language `message` for the UI plus the raw
/// `detail` for troubleshooting. Neither contains the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectError {
    pub stage: ConnectStage,
    pub message: String,
    pub detail: String,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{} ({})", self.message, self.detail)
        }
    }
}

impl std::error::Error for ConnectError {}

/// The plain-language message for a failure at `stage`.
pub fn stage_message(stage: ConnectStage, host: &str, port: u16) -> String {
    match stage {
        ConnectStage::Dns => "Server not found. Check the host name.".to_string(),
        ConnectStage::Tcp => format!("Could not reach {host}:{port}."),
        ConnectStage::StartTls => format!(
            "The server does not offer STARTTLS on port {port}. Choose TLS (usually port 993) instead."
        ),
        ConnectStage::Tls => {
            format!("Could not establish a secure (TLS) connection on port {port}.")
        }
        ConnectStage::Auth => {
            "Login rejected. Check the username and password; many providers require an app password."
                .to_string()
        }
        ConnectStage::Other => "The mail server returned an error.".to_string(),
    }
}

impl ConnectError {
    fn new(cfg: &ImapConfig, stage: ConnectStage, detail: impl fmt::Display) -> Self {
        Self {
            stage,
            message: stage_message(stage, &cfg.host, cfg.port),
            detail: cfg.redact(&detail.to_string()),
        }
    }

    fn other(cfg: &ImapConfig, message: impl Into<String>, detail: impl fmt::Display) -> Self {
        Self {
            stage: ConnectStage::Other,
            message: message.into(),
            detail: cfg.redact(&detail.to_string()),
        }
    }
}

/// Stage of a LOGIN failure: a `NO` / `BAD` answer is a rejection; anything
/// else (dropped connection, parse error) is not the user's credentials.
pub fn login_error_stage(err: &async_imap::error::Error) -> ConnectStage {
    match err {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => ConnectStage::Auth,
        _ => ConnectStage::Other,
    }
}

/// Resolve, TCP connect, TLS handshake, and LOGIN.
pub(crate) async fn connect_and_login(cfg: &ImapConfig) -> Result<ImapSession, ConnectError> {
    let addrs: Vec<_> = match tokio::net::lookup_host((cfg.host.as_str(), cfg.port)).await {
        Ok(addrs) => addrs.collect(),
        Err(e) => return Err(ConnectError::new(cfg, ConnectStage::Dns, e)),
    };
    if addrs.is_empty() {
        return Err(ConnectError::new(
            cfg,
            ConnectStage::Dns,
            "no addresses found for host",
        ));
    }

    let mut last_err: Option<io::Error> = None;
    let mut tcp = None;
    for addr in addrs {
        match tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                tcp = Some(stream);
                break;
            }
            Ok(Err(e)) => last_err = Some(e),
            Err(_) => {
                last_err = Some(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("connect to {addr} timed out"),
                ));
            }
        }
    }
    let Some(tcp) = tcp else {
        let detail = last_err.map(|e| e.to_string()).unwrap_or_default();
        return Err(ConnectError::new(cfg, ConnectStage::Tcp, detail));
    };
    // A command goes out in several small writes (TLS records); with Nagle's
    // algorithm the last one waits for the server's delayed ACK, which cost
    // about a quarter second per command (each batch's FETCH) on a real mailbox.
    if let Err(e) = tcp.set_nodelay(true) {
        warn!(error = %e, "could not disable Nagle's algorithm on the IMAP connection");
    }

    let tcp = match cfg.security {
        ImapSecurity::Tls => tcp,
        ImapSecurity::StartTls => starttls(cfg, tcp).await?,
    };
    let tls = TlsConnector::new()
        .connect(&cfg.host, tcp)
        .await
        .map_err(|e| ConnectError::new(cfg, ConnectStage::Tls, e))?;

    async_imap::Client::new(tls)
        .login(&cfg.username, &cfg.password)
        .await
        .map_err(|(e, _)| ConnectError::new(cfg, login_error_stage(&e), e))
}

/// Read the greeting and ask for `STARTTLS` over plain TCP; the stream is
/// then ready for the TLS handshake. Nothing else is sent in plaintext, and
/// anything the server sent after its `OK` is dropped with the plain
/// connection's buffer (RFC 3207 §6: no response injection before TLS).
async fn starttls(cfg: &ImapConfig, tcp: TcpStream) -> Result<TcpStream, ConnectError> {
    let mut client = async_imap::Client::new(tcp);
    let no_greeting = |detail: String| {
        ConnectError::other(
            cfg,
            format!(
                "No IMAP greeting on port {}. The server may expect TLS from the start: choose TLS (usually port 993).",
                cfg.port
            ),
            detail,
        )
    };
    match tokio::time::timeout(GREETING_TIMEOUT, client.read_response()).await {
        Err(_) => {
            return Err(no_greeting(format!(
                "nothing received in {} seconds",
                GREETING_TIMEOUT.as_secs()
            )))
        }
        Ok(Err(e)) => return Err(no_greeting(e.to_string())),
        Ok(Ok(None)) => return Err(no_greeting("connection closed".into())),
        Ok(Ok(Some(greeting))) => {
            if let Response::Data {
                status: Status::Bye,
                information,
                ..
            } = greeting.parsed()
            {
                return Err(ConnectError::other(
                    cfg,
                    "The mail server closed the connection.",
                    information.as_deref().unwrap_or("BYE"),
                ));
            }
        }
    }
    client
        .run_command_and_check_ok("STARTTLS", None)
        .await
        .map_err(|e| match e {
            async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
                ConnectError::new(cfg, ConnectStage::StartTls, e)
            }
            _ => ConnectError::other(cfg, "The mail server returned an error.", e),
        })?;
    Ok(client.into_inner())
}

/// A selectable folder from `LIST`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapFolder {
    pub name: String,
    pub delimiter: Option<String>,
}

/// The examined folder of a [`connect_mailbox`] probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderStatus {
    pub name: String,
    /// `EXAMINE` succeeded.
    pub exists: bool,
    /// Message count, when the folder exists.
    pub messages: Option<u32>,
}

/// Result of a successful [`connect_mailbox`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxProbe {
    pub folders: Vec<ImapFolder>,
    pub selected: FolderStatus,
}

/// The folder for a `LIST` entry, or `None` if it cannot be selected.
pub fn folder_entry(
    name: &str,
    delimiter: Option<&str>,
    attributes: &[NameAttribute<'_>],
) -> Option<ImapFolder> {
    let unselectable = attributes.iter().any(|a| match a {
        NameAttribute::NoSelect => true,
        // RFC 5258: a name that only exists as a hierarchy placeholder.
        NameAttribute::Extension(ext) => ext.eq_ignore_ascii_case("\\NonExistent"),
        _ => false,
    });
    if unselectable || name.is_empty() {
        return None;
    }
    Some(ImapFolder {
        name: name.to_string(),
        delimiter: delimiter.map(str::to_string),
    })
}

/// Sort folders for display: `INBOX` first, then case-insensitively; drop duplicates.
pub fn sort_folders(mut folders: Vec<ImapFolder>) -> Vec<ImapFolder> {
    folders.sort_by(|a, b| {
        let inbox = |f: &ImapFolder| !f.name.eq_ignore_ascii_case("INBOX");
        inbox(a)
            .cmp(&inbox(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    folders.dedup_by(|a, b| a.name == b.name);
    folders
}

/// Log in, list the selectable folders, and `EXAMINE` (read-only) `cfg.folder`
/// for its message count. A missing folder is reported in
/// [`FolderStatus::exists`], not as an error.
pub async fn connect_mailbox(cfg: &ImapConfig) -> Result<MailboxProbe, ConnectError> {
    info!(host = %cfg.host, port = cfg.port, folder = %cfg.folder, "connecting to IMAP (probe)");
    match tokio::time::timeout(PROBE_TIMEOUT, probe(cfg)).await {
        Ok(res) => res,
        Err(_) => Err(ConnectError::other(
            cfg,
            format!("{}:{} did not respond in time.", cfg.host, cfg.port),
            format!("timed out after {} seconds", PROBE_TIMEOUT.as_secs()),
        )),
    }
}

async fn probe(cfg: &ImapConfig) -> Result<MailboxProbe, ConnectError> {
    let mut session = connect_and_login(cfg).await?;

    let list_failed = |e: async_imap::error::Error| {
        ConnectError::other(cfg, "Could not list the mailbox folders.", e)
    };
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await
        .map_err(list_failed)?
        .try_collect()
        .await
        .map_err(list_failed)?;
    let folders = sort_folders(
        names
            .iter()
            .filter_map(|n| folder_entry(n.name(), n.delimiter(), n.attributes()))
            .collect(),
    );

    let selected = match session.examine(&cfg.folder).await {
        Ok(mailbox) => FolderStatus {
            name: cfg.folder.clone(),
            exists: true,
            messages: Some(mailbox.exists),
        },
        Err(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_)) => FolderStatus {
            name: cfg.folder.clone(),
            exists: false,
            messages: None,
        },
        Err(e) => {
            return Err(ConnectError::other(
                cfg,
                format!("Could not open folder {}.", cfg.folder),
                e,
            ))
        }
    };

    if let Err(e) = session.logout().await {
        warn!(error = %e, "IMAP LOGOUT failed after probe");
    }
    Ok(MailboxProbe { folders, selected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn stage_messages_are_plain_language() {
        assert_eq!(
            stage_message(ConnectStage::Dns, "imap.invalid", 993),
            "Server not found. Check the host name."
        );
        assert_eq!(
            stage_message(ConnectStage::Tcp, "mail.example", 993),
            "Could not reach mail.example:993."
        );
        assert_eq!(
            stage_message(ConnectStage::Tls, "mail.example", 143),
            "Could not establish a secure (TLS) connection on port 143."
        );
        assert!(stage_message(ConnectStage::Auth, "h", 993).starts_with("Login rejected."));
    }

    #[test]
    fn login_no_and_bad_are_auth_failures() {
        use async_imap::error::Error;
        assert_eq!(
            login_error_stage(&Error::No(
                "[AUTHENTICATIONFAILED] Invalid credentials".into()
            )),
            ConnectStage::Auth
        );
        assert_eq!(
            login_error_stage(&Error::Bad("bad".into())),
            ConnectStage::Auth
        );
        assert_eq!(
            login_error_stage(&Error::ConnectionLost),
            ConnectStage::Other
        );
        assert_eq!(
            login_error_stage(&Error::Io(io::Error::other("reset"))),
            ConnectStage::Other
        );
    }

    #[test]
    fn error_detail_is_redacted() {
        let cfg = ImapConfig {
            host: "h".into(),
            port: 993,
            security: ImapSecurity::Tls,
            username: "u".into(),
            password: "hunter2".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        };
        let err = ConnectError::new(&cfg, ConnectStage::Auth, "NO bad password hunter2");
        assert!(!err.detail.contains("hunter2"));
        assert!(!err.to_string().contains("hunter2"));
        assert!(err.to_string().starts_with("Login rejected."));
    }

    #[tokio::test]
    async fn unresolvable_host_is_dns_failure() {
        let cfg = ImapConfig {
            host: "imap.invalid".into(),
            port: 993,
            security: ImapSecurity::Tls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        };
        let Err(err) = connect_and_login(&cfg).await else {
            panic!("connect must fail");
        };
        assert_eq!(err.stage, ConnectStage::Dns, "{err}");
    }

    #[tokio::test]
    async fn closed_port_is_tcp_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let cfg = ImapConfig {
            host: "127.0.0.1".into(),
            port,
            security: ImapSecurity::Tls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        };
        let Err(err) = connect_and_login(&cfg).await else {
            panic!("connect must fail");
        };
        assert_eq!(err.stage, ConnectStage::Tcp, "{err}");
        assert_eq!(err.message, format!("Could not reach 127.0.0.1:{port}."));
    }

    /// A plain IMAP server on a local port that greets, answers the first
    /// command (expected to be `STARTTLS`) with `answer`, and then reports
    /// the next line it got and the first byte after it.
    async fn starttls_server(
        answer: &'static str,
    ) -> (u16, tokio::task::JoinHandle<(String, Option<u8>)>) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            stream.write_all(b"* OK IMAP ready\r\n").await.unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap_or("*").to_owned();
            stream
                .write_all(format!("{tag} {answer}\r\n").as_bytes())
                .await
                .unwrap();
            let mut next = [0u8; 1];
            let first = match stream.read(&mut next).await {
                Ok(1) => Some(next[0]),
                _ => None,
            };
            (line, first)
        });
        (port, server)
    }

    fn starttls_cfg(port: u16) -> ImapConfig {
        ImapConfig {
            host: "127.0.0.1".into(),
            port,
            security: ImapSecurity::StartTls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        }
    }

    #[tokio::test]
    async fn starttls_upgrades_before_anything_else_is_sent() {
        let (port, server) = starttls_server("OK Begin TLS negotiation now").await;
        let Err(err) = connect_and_login(&starttls_cfg(port)).await else {
            panic!("the fake server speaks no TLS");
        };
        let (command, first_byte) = server.await.unwrap();
        assert!(command.trim_end().ends_with(" STARTTLS"), "{command:?}");
        // 0x16: a TLS handshake record (the ClientHello), not a LOGIN.
        assert_eq!(first_byte, Some(0x16));
        assert_eq!(err.stage, ConnectStage::Tls, "{err}");
    }

    #[tokio::test]
    async fn a_server_without_starttls_is_refused_not_used_in_plaintext() {
        let (port, server) = starttls_server("BAD Unknown command").await;
        let Err(err) = connect_and_login(&starttls_cfg(port)).await else {
            panic!("connect must fail");
        };
        let (command, after) = server.await.unwrap();
        assert!(command.trim_end().ends_with(" STARTTLS"), "{command:?}");
        // The client hung up: no LOGIN with the password in the clear.
        assert_eq!(after, None);
        assert_eq!(err.stage, ConnectStage::StartTls, "{err}");
        assert_eq!(
            err.message,
            format!("The server does not offer STARTTLS on port {port}. Choose TLS (usually port 993) instead.")
        );
    }

    #[test]
    fn noselect_and_nonexistent_folders_are_skipped() {
        assert_eq!(
            folder_entry("INBOX", Some("/"), &[]),
            Some(ImapFolder {
                name: "INBOX".into(),
                delimiter: Some("/".into())
            })
        );
        assert_eq!(
            folder_entry("[Gmail]", Some("/"), &[NameAttribute::NoSelect]),
            None
        );
        assert_eq!(
            folder_entry(
                "Gone",
                Some("."),
                &[NameAttribute::Extension(Cow::Borrowed("\\NonExistent"))]
            ),
            None
        );
        assert!(folder_entry("[Gmail]/All Mail", Some("/"), &[NameAttribute::All]).is_some());
        assert!(folder_entry("Archive", None, &[NameAttribute::NoInferiors]).is_some());
        assert_eq!(folder_entry("", None, &[]), None);
    }

    #[test]
    fn folders_sort_inbox_first_then_alphabetically() {
        let f = |name: &str| ImapFolder {
            name: name.into(),
            delimiter: Some("/".into()),
        };
        let sorted = sort_folders(vec![
            f("Sent"),
            f("dmarc"),
            f("INBOX"),
            f("Archive"),
            f("dmarc"),
        ]);
        let names: Vec<_> = sorted.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["INBOX", "Archive", "dmarc", "Sent"]);
    }
}
