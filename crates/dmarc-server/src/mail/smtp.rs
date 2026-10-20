//! The SMTP submission transport: resolve, TCP connect, TLS (from the start
//! or after `STARTTLS`), login and hand-over, each step on its own so a
//! failure names the step. lettre speaks the protocol (its low-level
//! connection); this module decides the order and what counts as which error.
//!
//! Nothing is ever sent in plaintext beyond `EHLO` and `STARTTLS`: a server
//! that does not offer `STARTTLS` is left before the login.

use super::{Message, SendError, SmtpConfig, CONNECT_TIMEOUT, SEND_TIMEOUT};
use dmarc_store::SmtpSecurity;
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{AsyncSmtpConnection, AsyncTokioStream, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use std::error::Error as _;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{info, warn};

/// How the port-465 connection is secured once TCP is up.
#[derive(Debug, Clone, Copy)]
pub(super) enum Handshake {
    /// native-tls (the system trust store, like IMAP), TLS 1.2 or newer.
    Native,
    /// No TLS, so tests can script the dialogue after the handshake.
    #[cfg(test)]
    PlainForTests,
}

/// Connect, log in, send `message`, and say goodbye.
pub(super) async fn deliver(
    config: &SmtpConfig,
    message: &Message,
    handshake: Handshake,
) -> Result<(), SendError> {
    let email = build(config, message)?;
    info!(host = %config.host, port = config.port, to = %message.to, "sending mail");
    let mut conn = connect(config, handshake).await?;
    match timeout(
        SEND_TIMEOUT,
        conn.send(email.envelope(), &email.formatted()),
    )
    .await
    {
        Err(_) => {
            conn.abort().await;
            Err(SendError::Other {
                message: format!(
                    "{}:{} did not confirm the message in time; it may or may not have been sent.",
                    config.host, config.port
                ),
                detail: format!("no answer in {} seconds", SEND_TIMEOUT.as_secs()),
            })
        }
        Ok(Err(e)) => Err(match reply(&e) {
            Some(reply) => SendError::Rejected {
                reply: config.redact(&reply),
            },
            None => SendError::Other {
                message: "The connection to the mail server broke off while sending.".into(),
                detail: config.redact(&e.to_string()),
            },
        }),
        Ok(Ok(_)) => {
            if let Ok(Err(e)) = timeout(SEND_TIMEOUT, conn.quit()).await {
                warn!(error = %e, "SMTP QUIT failed after the message was accepted");
            }
            Ok(())
        }
    }
}

/// The message as lettre sends it: `From` the sender, `Reply-To` from the
/// settings, plain text with an optional HTML alternative, and its images
/// beside the HTML as an inline attachment (`multipart/related`).
fn build(config: &SmtpConfig, message: &Message) -> Result<lettre::Message, SendError> {
    let mut builder = lettre::Message::builder()
        .from(Mailbox::new(
            Some(config.sender_name.clone()),
            config.sender.clone(),
        ))
        .to(Mailbox::new(None, message.to.clone()))
        .subject(message.subject.as_str())
        .message_id(Some(message_id(config)));
    if let Some(reply_to) = &config.reply_to {
        builder = builder.reply_to(Mailbox::new(None, reply_to.clone()));
    }
    let built = match &message.html {
        Some(html) if !message.images.is_empty() => {
            let mut related = MultiPart::related().singlepart(SinglePart::html(html.clone()));
            for image in &message.images {
                let content_type =
                    ContentType::parse(image.media_type).map_err(|e| SendError::Other {
                        message: "Marcan could not put the message together.".into(),
                        detail: e.to_string(),
                    })?;
                related = related.singlepart(
                    Attachment::new_inline(image.cid.to_owned())
                        .body(image.bytes.clone(), content_type),
                );
            }
            builder.multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(message.text.clone()))
                    .multipart(related),
            )
        }
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(
            message.text.clone(),
            html.clone(),
        )),
        None => builder
            .header(ContentType::TEXT_PLAIN)
            .body(message.text.clone()),
    };
    built.map_err(|e| SendError::Other {
        message: "Marcan could not put the message together.".into(),
        detail: e.to_string(),
    })
}

/// A unique `Message-ID` in the sender's domain (not this host's name, which
/// inside a container means nothing to anyone).
fn message_id(config: &SmtpConfig) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!(
        "<{nanos:x}.{}.{:x}@{}>",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        config.sender.domain()
    )
}

/// The server's negative reply ("550 5.7.1 …"), when the error is one.
fn reply(e: &lettre::transport::smtp::Error) -> Option<String> {
    let code = e.status()?;
    let text = e.source().map(ToString::to_string).unwrap_or_default();
    Some(format!("{code} {text}").trim_end().to_owned())
}

/// Resolve, TCP, TLS, greeting and login: a connection ready for `MAIL FROM`.
async fn connect(
    config: &SmtpConfig,
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
async fn tcp_connect(config: &SmtpConfig) -> Result<TcpStream, SendError> {
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
    config: &SmtpConfig,
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
    config: &SmtpConfig,
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
    config: &SmtpConfig,
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

/// `AUTH` with PLAIN or LOGIN, only ever over TLS.
async fn login(config: &SmtpConfig, conn: &mut AsyncSmtpConnection) -> Result<(), SendError> {
    let credentials = Credentials::new(config.username.clone(), config.password.clone());
    let result = timeout(
        CONNECT_TIMEOUT,
        conn.auth(&[Mechanism::Plain, Mechanism::Login], &credentials),
    )
    .await;
    let error = match result {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(e)) => match reply(&e) {
            Some(reply) => SendError::Auth {
                detail: config.redact(&reply),
            },
            None if e.is_client() => SendError::Other {
                message: "The server offers no login method Marcan supports (PLAIN or LOGIN)."
                    .into(),
                detail: e.to_string(),
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
mod tests {
    use super::*;
    use crate::mail::parse_address;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// How the scripted server answers.
    #[derive(Clone, Copy)]
    struct Script {
        /// Offer `STARTTLS` in the `EHLO` answer.
        starttls: bool,
        /// The answer to `AUTH`.
        auth: &'static str,
        /// The answer to `RCPT TO`.
        rcpt: &'static str,
    }

    const ACCEPTING: Script = Script {
        starttls: false,
        auth: "235 2.7.0 Authentication successful",
        rcpt: "250 2.1.5 OK",
    };

    /// What the scripted server saw: every line, the message, and the first
    /// byte after it agreed to `STARTTLS`.
    #[derive(Debug, Default)]
    struct Seen {
        lines: Vec<String>,
        data: String,
        after_starttls: Option<u8>,
    }

    impl Seen {
        fn sent(&self, command: &str) -> bool {
            self.lines.iter().any(|l| l.starts_with(command))
        }
    }

    /// A plain SMTP server on a local port that greets, answers `EHLO` (with
    /// `AUTH PLAIN LOGIN`, and `STARTTLS` when the script says so) and plays
    /// the script until `QUIT`, `STARTTLS` or the client hangs up.
    async fn server(script: Script) -> (u16, tokio::task::JoinHandle<Seen>) {
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
                    "MAIL" => "250 2.1.0 OK".to_owned(),
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
                        "250 2.0.0 Queued".to_owned()
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

    fn config(port: u16, security: SmtpSecurity) -> SmtpConfig {
        SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            security,
            username: "alerts@example.com".into(),
            password: "hunter2".into(),
            sender: parse_address("dmarc@example.com").unwrap(),
            sender_name: "Marcan".into(),
            reply_to: parse_address("help@example.com"),
        }
    }

    fn message() -> Message {
        Message {
            to: parse_address("owner@example.org").unwrap(),
            subject: "Marcan test".into(),
            text: "Marcan can send mail from this address.".into(),
            html: None,
            images: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_message_goes_out_with_its_headers() {
        let (port, server) = server(ACCEPTING).await;
        let cfg = config(port, SmtpSecurity::Tls);
        deliver(&cfg, &message(), Handshake::PlainForTests)
            .await
            .unwrap();
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
        let err = deliver(
            &config(port, SmtpSecurity::StartTls),
            &message(),
            Handshake::Native,
        )
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
        let err = deliver(
            &config(port, SmtpSecurity::StartTls),
            &message(),
            Handshake::Native,
        )
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
        let err = deliver(
            &config(port, SmtpSecurity::Tls),
            &message(),
            Handshake::PlainForTests,
        )
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
        let err = deliver(
            &config(port, SmtpSecurity::Tls),
            &message(),
            Handshake::PlainForTests,
        )
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

    #[tokio::test]
    async fn an_unresolvable_host_and_a_closed_port_are_named() {
        let mut cfg = config(465, SmtpSecurity::Tls);
        cfg.host = "smtp.invalid".into();
        let err = deliver(&cfg, &message(), Handshake::Native)
            .await
            .unwrap_err();
        assert!(matches!(err, SendError::Resolve { .. }), "{err}");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = deliver(
            &config(port, SmtpSecurity::Tls),
            &message(),
            Handshake::Native,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, SendError::Connect { silent: false, .. }),
            "{err}"
        );
        assert_eq!(err.message(), format!("Could not reach 127.0.0.1:{port}."));
    }

    #[test]
    fn html_goes_out_as_an_alternative() {
        let cfg = config(465, SmtpSecurity::Tls);
        let email = build(
            &cfg,
            &Message {
                html: Some("<p>Hello</p>".into()),
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        assert!(text.contains("multipart/alternative"), "{text}");
        assert!(text.contains("text/html"), "{text}");
        assert!(!text.contains("multipart/related"), "{text}");
    }

    #[test]
    fn the_logo_is_attached_inline_beside_the_html() {
        let cfg = config(465, SmtpSecurity::Tls);
        let email = build(
            &cfg,
            &Message {
                html: Some("<p><img src=\"cid:logo\"><img src=\"cid:marcan-mark\"></p>".into()),
                images: vec![
                    super::super::InlineImage {
                        cid: "logo",
                        media_type: "image/png",
                        bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
                        size: dmarc_store::PixelSize {
                            width: 1,
                            height: 1,
                        },
                    },
                    super::super::InlineImage::marcan_mark(),
                ],
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        // Plain text, then the HTML and its image together.
        assert!(text.contains("multipart/alternative"), "{text}");
        assert!(text.contains("multipart/related"), "{text}");
        assert!(text.contains("Content-ID: <logo>\r\n"), "{text}");
        assert!(text.contains("Content-ID: <marcan-mark>\r\n"), "{text}");
        assert!(text.contains("Content-Disposition: inline"), "{text}");
        assert!(text.contains("Content-Type: image/png\r\n"), "{text}");
        assert!(text.contains("iVBORw0KGgo="), "{text}");
        assert!(
            text.find("text/plain").unwrap() < text.find("multipart/related").unwrap(),
            "{text}"
        );

        // A plain-text message has no attachment, logo or not.
        let plain = build(
            &cfg,
            &Message {
                images: vec![super::super::InlineImage::marcan_mark()],
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(plain.formatted()).unwrap();
        assert!(!text.contains("multipart"), "{text}");
    }

    #[test]
    fn the_sender_name_is_the_from_display_name() {
        let cfg = SmtpConfig {
            sender_name: "Acme IT via Marcan".into(),
            ..config(465, SmtpSecurity::Tls)
        };
        let email = build(&cfg, &message()).unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        assert!(
            text.contains("From: \"Acme IT via Marcan\" <dmarc@example.com>\r\n"),
            "{text}"
        );
        // The envelope sender stays the address.
        assert_eq!(
            email.envelope().from().map(ToString::to_string).as_deref(),
            Some("dmarc@example.com")
        );
    }
}
