//! The SMTP submission transport: resolve, TCP connect, TLS (from the start
//! or after `STARTTLS`), login and hand-over, each step on its own so a
//! failure names the step. lettre speaks the protocol (its low-level
//! connection); this module decides the order and what counts as which error.
//!
//! Nothing is ever sent in plaintext beyond `EHLO` and `STARTTLS`: a server
//! that does not offer `STARTTLS` is left before the login.

use super::mime::build;
use super::{Message, SendConfig, SendError, SmtpServer, Stage, CONNECT_TIMEOUT, SEND_TIMEOUT};
use dmarc_store::SmtpSecurity;
use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{AsyncSmtpConnection, AsyncTokioStream, TlsParameters};
use lettre::transport::smtp::commands::{Data, Mail, Rcpt};
use lettre::transport::smtp::extension::{ClientId, Extension, MailBodyParameter, MailParameter};
use std::error::Error as _;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{info, warn};

/// How the port-465 connection is secured once TCP is up.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Handshake {
    /// native-tls (the system trust store, like IMAP), TLS 1.2 or newer.
    Native,
    /// No TLS, so tests can script the dialogue after the handshake.
    #[cfg(test)]
    PlainForTests,
}

/// Connect to `server`, log in, send `message` as `config` builds it, and
/// say goodbye.
pub(crate) async fn deliver(
    config: &SendConfig,
    server: &SmtpServer,
    message: &Message,
    handshake: Handshake,
) -> Result<(), SendError> {
    let email = build(config, message)?;
    info!(host = %server.host, port = server.port, to = %message.to, "sending mail");
    let mut conn = connect(server, handshake).await?;
    match timeout(
        SEND_TIMEOUT,
        hand_over(&mut conn, email.envelope(), &email.formatted()),
    )
    .await
    {
        Err(_) => {
            conn.abort().await;
            Err(SendError::Other {
                message: format!(
                    "{}:{} did not confirm the message in time; it may or may not have been sent.",
                    server.host, server.port
                ),
                detail: format!("no answer in {} seconds", SEND_TIMEOUT.as_secs()),
            })
        }
        Ok(Err(Unsendable(why))) => {
            conn.abort().await;
            Err(SendError::Other {
                message: "The mail server cannot take this message.".into(),
                detail: why.into(),
            })
        }
        Ok(Err(Refused(stage, e))) => {
            conn.abort().await;
            Err(match reply(&e) {
                Some(reply) => SendError::refusal(stage, server.redact(&reply)),
                None => SendError::Other {
                    message: "The connection to the mail server broke off while sending.".into(),
                    detail: server.redact(&e.to_string()),
                },
            })
        }
        Ok(Ok(())) => {
            if let Ok(Err(e)) = timeout(SEND_TIMEOUT, conn.quit()).await {
                warn!(error = %e, "SMTP QUIT failed after the message was accepted");
            }
            Ok(())
        }
    }
}

/// Why [`hand_over`] did not hand the message over.
enum HandOverError {
    /// The server answered no at `Stage` (or the connection broke there).
    Refused(Stage, lettre::transport::smtp::Error),
    /// The server lacks an extension the message needs; nothing was sent.
    Unsendable(&'static str),
}
use HandOverError::{Refused, Unsendable};

/// `MAIL FROM`, `RCPT TO` and `DATA`, each step on its own (lettre's
/// `send` with the same options), so a refusal names the step it came at:
/// the sender's, the recipient's, or the message's.
async fn hand_over(
    conn: &mut AsyncSmtpConnection,
    envelope: &Envelope,
    email: &[u8],
) -> Result<(), HandOverError> {
    let mut options = Vec::new();
    let non_ascii = envelope
        .from()
        .into_iter()
        .chain(envelope.to())
        .any(|a| !AsRef::<str>::as_ref(a).is_ascii());
    if non_ascii {
        if !conn.server_info().supports_feature(Extension::SmtpUtfEight) {
            return Err(Unsendable(
                "the addresses need SMTPUTF8, which the server does not offer",
            ));
        }
        options.push(MailParameter::SmtpUtfEight);
    }
    if !email.is_ascii() {
        if !conn.server_info().supports_feature(Extension::EightBitMime) {
            return Err(Unsendable(
                "the message needs 8BITMIME, which the server does not offer",
            ));
        }
        options.push(MailParameter::Body(MailBodyParameter::EightBitMime));
    }
    conn.command(Mail::new(envelope.from().cloned(), options))
        .await
        .map_err(|e| Refused(Stage::Sender, e))?;
    for to in envelope.to() {
        conn.command(Rcpt::new(to.clone(), Vec::new()))
            .await
            .map_err(|e| Refused(Stage::Recipient, e))?;
    }
    conn.command(Data)
        .await
        .map_err(|e| Refused(Stage::Message, e))?;
    conn.message(email)
        .await
        .map_err(|e| Refused(Stage::Message, e))?;
    Ok(())
}

/// Connect and log in, then say goodbye: the connection test of an
/// account's SMTP login (`EHLO` and `AUTH`), no message sent.
pub(crate) async fn login_only(server: &SmtpServer, handshake: Handshake) -> Result<(), SendError> {
    info!(host = %server.host, port = server.port, "testing the SMTP login");
    let mut conn = connect(server, handshake).await?;
    if let Ok(Err(e)) = timeout(SEND_TIMEOUT, conn.quit()).await {
        warn!(error = %e, "SMTP QUIT failed after the login test");
    }
    Ok(())
}

/// The server's negative reply ("550 5.7.1 …"), when the error is one.
fn reply(e: &lettre::transport::smtp::Error) -> Option<String> {
    let code = e.status()?;
    let text = e.source().map(ToString::to_string).unwrap_or_default();
    Some(format!("{code} {text}").trim_end().to_owned())
}

/// Resolve, TCP, TLS, greeting and login: a connection ready for `MAIL FROM`.
async fn connect(
    config: &SmtpServer,
    handshake: Handshake,
) -> Result<AsyncSmtpConnection, SendError> {
    let tcp = tcp_connect(config).await?;
    let hello = ClientId::default();
    let mut conn = match config.security {
        SmtpSecurity::Tls => {
            let stream = secure(config, tcp, handshake).await?;
            greet(config, stream, &hello).await?
        }
        SmtpSecurity::StartTls => {
            let mut conn = greet(config, Box::new(tcp), &hello).await?;
            starttls(config, &mut conn, &hello).await?;
            conn
        }
    };
    login(config, &mut conn).await?;
    Ok(conn)
}

/// Resolve the host and connect to the first address that answers.
async fn tcp_connect(config: &SmtpServer) -> Result<TcpStream, SendError> {
    let resolve_failed = |detail: String| SendError::Resolve {
        host: config.host.clone(),
        detail,
    };
    let addrs: Vec<SocketAddr> = match timeout(
        CONNECT_TIMEOUT,
        tokio::net::lookup_host((config.host.as_str(), config.port)),
    )
    .await
    {
        Ok(Ok(addrs)) => addrs.collect(),
        Ok(Err(e)) => return Err(resolve_failed(e.to_string())),
        Err(_) => {
            return Err(resolve_failed(format!(
                "no answer in {} seconds",
                CONNECT_TIMEOUT.as_secs()
            )))
        }
    };
    if addrs.is_empty() {
        return Err(resolve_failed("no addresses found for host".into()));
    }

    let mut last_err = String::new();
    for addr in addrs {
        match timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last_err = e.to_string(),
            Err(_) => last_err = format!("connect to {addr} timed out"),
        }
    }
    Err(SendError::Connect {
        host: config.host.clone(),
        port: config.port,
        silent: false,
        detail: last_err,
    })
}

/// The TLS handshake of an implicit-TLS (port 465) connection.
async fn secure(
    config: &SmtpServer,
    tcp: TcpStream,
    handshake: Handshake,
) -> Result<Box<dyn AsyncTokioStream>, SendError> {
    let tls_failed = |detail: String| SendError::Tls {
        host: config.host.clone(),
        port: config.port,
        detail,
    };
    match handshake {
        Handshake::Native => {
            let connector = tokio_native_tls::native_tls::TlsConnector::builder()
                .min_protocol_version(Some(tokio_native_tls::native_tls::Protocol::Tlsv12))
                .build()
                .map_err(|e| tls_failed(e.to_string()))?;
            let connector = tokio_native_tls::TlsConnector::from(connector);
            match timeout(CONNECT_TIMEOUT, connector.connect(&config.host, tcp)).await {
                Ok(Ok(stream)) => Ok(Box::new(TlsStream(stream))),
                Ok(Err(e)) => Err(tls_failed(e.to_string())),
                Err(_) => Err(tls_failed(format!(
                    "no handshake in {} seconds",
                    CONNECT_TIMEOUT.as_secs()
                ))),
            }
        }
        #[cfg(test)]
        Handshake::PlainForTests => Ok(Box::new(tcp)),
    }
}

/// Read the greeting and send `EHLO`. A server that stays silent on a
/// `STARTTLS` connection probably waits for a TLS handshake.
async fn greet(
    config: &SmtpServer,
    stream: Box<dyn AsyncTokioStream>,
    hello: &ClientId,
) -> Result<AsyncSmtpConnection, SendError> {
    let starttls = config.security == SmtpSecurity::StartTls;
    match timeout(
        CONNECT_TIMEOUT,
        AsyncSmtpConnection::connect_with_transport(stream, hello),
    )
    .await
    {
        Ok(Ok(conn)) => Ok(conn),
        Err(_) => Err(SendError::Connect {
            host: config.host.clone(),
            port: config.port,
            silent: starttls,
            detail: format!("no greeting in {} seconds", CONNECT_TIMEOUT.as_secs()),
        }),
        Ok(Err(e)) => Err(match reply(&e) {
            Some(reply) => SendError::Other {
                message: format!("{} turned the connection away.", config.host),
                detail: reply,
            },
            None => SendError::Connect {
                host: config.host.clone(),
                port: config.port,
                silent: false,
                detail: e.to_string(),
            },
        }),
    }
}

/// Upgrade a plain connection with `STARTTLS`, or leave before anything else
/// is sent when the server does not offer it.
async fn starttls(
    config: &SmtpServer,
    conn: &mut AsyncSmtpConnection,
    hello: &ClientId,
) -> Result<(), SendError> {
    if !conn.can_starttls() {
        conn.abort().await;
        return Err(SendError::StartTlsNotOffered { port: config.port });
    }
    let tls_failed = |detail: String| SendError::Tls {
        host: config.host.clone(),
        port: config.port,
        detail,
    };
    let params =
        TlsParameters::new_native(config.host.clone()).map_err(|e| tls_failed(e.to_string()))?;
    // On failure the connection is only dropped: lettre already said goodbye
    // after a refusal, and a failed handshake leaves no stream to write to.
    match timeout(CONNECT_TIMEOUT, conn.starttls(params, hello)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            Err(match reply(&e) {
                // It offered STARTTLS and then refused it.
                Some(reply) => SendError::Other {
                    message: format!(
                        "The server refused STARTTLS on port {}, so Marcan did not log in.",
                        config.port
                    ),
                    detail: reply,
                },
                None => tls_failed(
                    e.source()
                        .map_or_else(|| e.to_string(), ToString::to_string),
                ),
            })
        }
        Err(_) => Err(tls_failed(format!(
            "no handshake in {} seconds",
            CONNECT_TIMEOUT.as_secs()
        ))),
    }
}

/// `AUTH` with PLAIN or LOGIN and the password, only ever over TLS.
async fn login(config: &SmtpServer, conn: &mut AsyncSmtpConnection) -> Result<(), SendError> {
    let credentials = Credentials::new(config.username.clone(), config.password.clone());
    let mechanisms = [Mechanism::Plain, Mechanism::Login];
    let result = timeout(CONNECT_TIMEOUT, conn.auth(&mechanisms, &credentials)).await;
    let error = match result {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(e)) => match reply(&e) {
            Some(reply) => SendError::Auth {
                detail: config.redact(&reply),
            },
            None if e.is_client() => SendError::Other {
                message: "The server offers no login method Marcan supports (PLAIN or LOGIN)."
                    .into(),
                detail: config.redact(&e.to_string()),
            },
            None => SendError::Other {
                message: "The mail server returned an error.".into(),
                detail: config.redact(&e.to_string()),
            },
        },
        Err(_) => SendError::Other {
            message: format!("{} did not answer the login in time.", config.host),
            detail: format!("no answer in {} seconds", CONNECT_TIMEOUT.as_secs()),
        },
    };
    conn.abort().await;
    Err(error)
}

/// A TLS stream lettre can carry ([`AsyncTokioStream`] is only implemented
/// for a plain `TcpStream`).
#[derive(Debug)]
struct TlsStream(tokio_native_tls::TlsStream<TcpStream>);

impl AsyncRead for TlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl AsyncTokioStream for TlsStream {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.0.get_ref().get_ref().get_ref().peer_addr()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mail::{parse_address, Transport};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// How the scripted server answers.
    #[derive(Clone, Copy)]
    pub(crate) struct Script {
        /// Offer `STARTTLS` in the `EHLO` answer.
        pub starttls: bool,
        /// The answer to `AUTH`.
        pub auth: &'static str,
        /// The answer to `MAIL FROM`.
        pub mail: &'static str,
        /// The answer to `RCPT TO`.
        pub rcpt: &'static str,
        /// The answer after the message's end.
        pub data: &'static str,
    }

    pub(crate) const ACCEPTING: Script = Script {
        starttls: false,
        auth: "235 2.7.0 Authentication successful",
        mail: "250 2.1.0 OK",
        rcpt: "250 2.1.5 OK",
        data: "250 2.0.0 Queued",
    };

    /// What the scripted server saw: every line, the message, and the first
    /// byte after it agreed to `STARTTLS`.
    #[derive(Debug, Default)]
    pub(crate) struct Seen {
        pub lines: Vec<String>,
        pub data: String,
        after_starttls: Option<u8>,
    }

    impl Seen {
        pub(crate) fn sent(&self, command: &str) -> bool {
            self.lines.iter().any(|l| l.starts_with(command))
        }
    }

    /// A plain SMTP server on a local port that greets, answers `EHLO` (with
    /// `AUTH PLAIN LOGIN`, and `STARTTLS` when the script says so) and plays
    /// the script until `QUIT`, `STARTTLS` or the client hangs up.
    pub(crate) async fn server(script: Script) -> (u16, tokio::task::JoinHandle<Seen>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut seen = Seen::default();
            stream.write_all(b"220 fake ESMTP\r\n").await.unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end().to_owned();
                let verb = line
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                seen.lines.push(line);
                let answer = match verb.as_str() {
                    "EHLO" if script.starttls => {
                        "250-fake\r\n250-STARTTLS\r\n250 AUTH PLAIN LOGIN".to_owned()
                    }
                    "EHLO" => "250-fake\r\n250 AUTH PLAIN LOGIN".to_owned(),
                    "STARTTLS" => {
                        stream.write_all(b"220 2.0.0 Ready\r\n").await.unwrap();
                        let mut next = [0u8; 1];
                        if matches!(stream.read(&mut next).await, Ok(1)) {
                            seen.after_starttls = Some(next[0]);
                        }
                        break;
                    }
                    "AUTH" => script.auth.to_owned(),
                    "MAIL" => script.mail.to_owned(),
                    "RCPT" => script.rcpt.to_owned(),
                    "DATA" => {
                        stream.write_all(b"354 Go ahead\r\n").await.unwrap();
                        loop {
                            let mut data = String::new();
                            if stream.read_line(&mut data).await.unwrap_or(0) == 0
                                || data == ".\r\n"
                            {
                                break;
                            }
                            seen.data.push_str(&data);
                        }
                        script.data.to_owned()
                    }
                    "QUIT" => {
                        let _ = stream.write_all(b"221 2.0.0 Bye\r\n").await;
                        break;
                    }
                    _ => "502 5.5.2 Unknown command".to_owned(),
                };
                let _ = stream.write_all(format!("{answer}\r\n").as_bytes()).await;
            }
            seen
        });
        (port, task)
    }

    pub(crate) fn config(port: u16, security: SmtpSecurity) -> SendConfig {
        SendConfig {
            transport: Transport::Smtp(SmtpServer {
                host: "127.0.0.1".into(),
                port,
                security,
                username: "alerts@example.com".into(),
                password: "hunter2".into(),
            }),
            sender: parse_address("dmarc@example.com").unwrap(),
            sender_name: "Marcan".into(),
            reply_to: parse_address("help@example.com"),
        }
    }

    /// [`deliver`] of `config` through its own SMTP server.
    async fn send(config: &SendConfig, handshake: Handshake) -> Result<(), SendError> {
        deliver(config, config.smtp().unwrap(), &message(), handshake).await
    }

    pub(crate) fn message() -> Message {
        Message {
            to: parse_address("owner@example.org").unwrap(),
            subject: "Marcan test".into(),
            text: "Marcan can send mail from this address.".into(),
            html: None,
            images: Vec::new(),
        }
    }

    #[tokio::test]
    async fn the_login_test_logs_in_and_sends_nothing() {
        let (port, accepted) = server(ACCEPTING).await;
        login_only(
            config(port, SmtpSecurity::Tls).smtp().unwrap(),
            Handshake::PlainForTests,
        )
        .await
        .unwrap();
        let seen = accepted.await.unwrap();
        assert!(seen.sent("EHLO"), "{:?}", seen.lines);
        assert!(seen.sent("AUTH PLAIN "), "{:?}", seen.lines);
        assert!(seen.sent("QUIT"), "{:?}", seen.lines);
        assert!(!seen.sent("MAIL"), "{:?}", seen.lines);

        let (port, refused) = server(Script {
            auth: "535 5.7.8 Authentication credentials invalid",
            ..ACCEPTING
        })
        .await;
        let err = login_only(
            config(port, SmtpSecurity::Tls).smtp().unwrap(),
            Handshake::PlainForTests,
        )
        .await
        .unwrap_err();
        assert_eq!(err.reason(), "auth");
        assert!(!refused.await.unwrap().sent("MAIL"));
    }

    #[tokio::test]
    async fn a_message_goes_out_with_its_headers() {
        let (port, server) = server(ACCEPTING).await;
        let cfg = config(port, SmtpSecurity::Tls);
        send(&cfg, Handshake::PlainForTests).await.unwrap();
        let seen = server.await.unwrap();
        assert!(seen.sent("AUTH PLAIN "), "{:?}", seen.lines);
        assert!(
            seen.sent("MAIL FROM:<dmarc@example.com>"),
            "{:?}",
            seen.lines
        );
        assert!(seen.sent("RCPT TO:<owner@example.org>"), "{:?}", seen.lines);
        assert!(
            seen.data.contains("From: Marcan <dmarc@example.com>\r\n"),
            "{}",
            seen.data
        );
        assert!(
            seen.data.contains("Reply-To: help@example.com\r\n"),
            "{}",
            seen.data
        );
        assert!(
            seen.data.contains("Subject: Marcan test\r\n"),
            "{}",
            seen.data
        );
        assert!(seen.data.contains("@example.com>\r\n"), "{}", seen.data);
        assert!(seen
            .data
            .contains("Marcan can send mail from this address."));
        assert_eq!(seen.lines.last().map(String::as_str), Some("QUIT"));
    }

    #[tokio::test]
    async fn a_server_without_starttls_gets_no_credentials() {
        let (port, server) = server(ACCEPTING).await;
        let err = send(&config(port, SmtpSecurity::StartTls), Handshake::Native)
            .await
            .unwrap_err();
        let seen = server.await.unwrap();
        assert_eq!(err, SendError::StartTlsNotOffered { port });
        assert!(err.message().contains("does not offer STARTTLS"), "{err}");
        // EHLO, then goodbye: no AUTH, no password, no message.
        assert!(!seen.sent("AUTH"), "{:?}", seen.lines);
        assert!(!seen.sent("MAIL"), "{:?}", seen.lines);
        assert!(seen
            .lines
            .iter()
            .all(|l| l.starts_with("EHLO") || l == "QUIT"));
    }

    #[tokio::test]
    async fn starttls_upgrades_before_anything_else_is_sent() {
        let (port, server) = server(Script {
            starttls: true,
            ..ACCEPTING
        })
        .await;
        let err = send(&config(port, SmtpSecurity::StartTls), Handshake::Native)
            .await
            .unwrap_err();
        let seen = server.await.unwrap();
        assert_eq!(seen.lines.last().map(String::as_str), Some("STARTTLS"));
        // 0x16: a TLS handshake record (the ClientHello), not AUTH.
        assert_eq!(seen.after_starttls, Some(0x16));
        assert!(matches!(err, SendError::Tls { .. }), "{err}");
        assert_eq!(err.reason(), "tls");
    }

    #[tokio::test]
    async fn a_rejected_login_is_an_auth_failure() {
        let (port, server) = server(Script {
            auth: "535 5.7.8 Authentication credentials invalid",
            ..ACCEPTING
        })
        .await;
        let err = send(&config(port, SmtpSecurity::Tls), Handshake::PlainForTests)
            .await
            .unwrap_err();
        let seen = server.await.unwrap();
        assert_eq!(
            err,
            SendError::Auth {
                detail: "535 5.7.8 Authentication credentials invalid".into()
            }
        );
        assert!(err.message().starts_with("Login rejected."));
        assert!(!seen.sent("MAIL"), "{:?}", seen.lines);
    }

    #[tokio::test]
    async fn a_refused_recipient_is_a_rejection_with_the_reply() {
        let (port, server) = server(Script {
            rcpt: "550 5.7.1 Relaying denied",
            ..ACCEPTING
        })
        .await;
        let err = send(&config(port, SmtpSecurity::Tls), Handshake::PlainForTests)
            .await
            .unwrap_err();
        server.await.unwrap();
        assert_eq!(
            err,
            SendError::Rejected {
                reply: "550 5.7.1 Relaying denied".into()
            }
        );
    }

    /// A refusal names whose it is by the step it came at: `MAIL FROM` is
    /// the sending account's, `DATA` with Exchange's send-as refusal too,
    /// neither for good; `DATA` with a mailbox code is the recipient's.
    #[tokio::test]
    async fn a_refusal_is_told_apart_by_its_step() {
        let cases = [
            (
                Script {
                    mail: "550 5.7.1 Sender address rejected",
                    ..ACCEPTING
                },
                SendError::SenderRejected {
                    reply: "550 5.7.1 Sender address rejected".into(),
                },
                false,
            ),
            (
                Script {
                    data: "554 5.2.0 STOREDRV.Submission.Exception:SendAsDeniedException",
                    ..ACCEPTING
                },
                SendError::SenderRejected {
                    reply: "554 5.2.0 STOREDRV.Submission.Exception:SendAsDeniedException".into(),
                },
                false,
            ),
            (
                Script {
                    data: "552 5.2.2 Mailbox full",
                    ..ACCEPTING
                },
                SendError::Rejected {
                    reply: "552 5.2.2 Mailbox full".into(),
                },
                true,
            ),
        ];
        for (script, expected, permanent) in cases {
            let (port, server) = server(script).await;
            let err = send(&config(port, SmtpSecurity::Tls), Handshake::PlainForTests)
                .await
                .unwrap_err();
            server.await.unwrap();
            assert_eq!(err, expected);
            assert_eq!(err.is_permanent(), permanent, "{err}");
        }
    }

    #[tokio::test]
    async fn an_unresolvable_host_and_a_closed_port_are_named() {
        let mut cfg = config(465, SmtpSecurity::Tls);
        if let Transport::Smtp(server) = &mut cfg.transport {
            server.host = "smtp.invalid".into();
        }
        let err = send(&cfg, Handshake::Native).await.unwrap_err();
        assert!(matches!(err, SendError::Resolve { .. }), "{err}");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = send(&config(port, SmtpSecurity::Tls), Handshake::Native)
            .await
            .unwrap_err();
        assert!(
            matches!(err, SendError::Connect { silent: false, .. }),
            "{err}"
        );
        assert_eq!(err.message(), format!("Could not reach 127.0.0.1:{port}."));
    }
}
