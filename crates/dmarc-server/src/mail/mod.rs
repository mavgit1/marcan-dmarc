//! Outbound mail: Marcan as an SMTP submission client, or as a Microsoft
//! 365 mailbox over Microsoft Graph.
//!
//! Mail goes out through the account chosen under Settings → Sending
//! (`accounts::Accounts::send_config`, a [`SendConfig`]): a password login
//! hands it to its SMTP server ([`Transport::Smtp`]: `smtp`), over TLS from
//! the start (port 465) or `STARTTLS` (port 587), always logged in; a
//! Microsoft login posts it to Microsoft Graph's `sendMail` as its mailbox,
//! with the token it reads with ([`Transport::Graph`]: `graph`). Both carry
//! the same MIME, built once ([`mime`]). Marcan never listens on port 25,
//! never delivers to a recipient's MX itself and never signs DKIM: SPF and
//! DKIM are the provider's job.
//!
//! [`send`] is the one way out, reached through the [`MailSender`] on
//! [`AppState`](crate::AppState): the test buttons (`POST
//! /api/settings/sending/test`, `POST /api/settings/alerts/test`, the client
//! report preview), the alert job and the client report job call it, one
//! message per [`SEND_MIN_INTERVAL`] at most ([`PacedSender`]). Each
//! failure is a [`SendError`] that says which step failed and why, in words
//! for the UI.
//!
//! What Marcan sends is rendered here too, from view types to text and HTML
//! with no store access: the alert email ([`alerts`]), the digest
//! ([`digest`]) and the client report ([`client_report`]), each in the one
//! frame ([`frame`]: the operator's [`Letterhead`] on top, the one
//! [`footer`] at the end). Every decision about how an email presents the
//! operator (its top, its `From` name, its sign-off) is made in `text`,
//! from the [`Letterhead`].

pub mod alerts;
pub mod client_report;
mod config;
pub mod digest;
mod error;
pub mod frame;
pub(crate) mod graph;
pub mod image_size;
#[cfg(any(test, feature = "test-support"))]
mod memory_sender;
mod message;
pub(crate) mod mime;
mod sender;
pub(crate) mod smtp;
mod text;

pub(crate) use config::check_port;
pub use config::{
    default_port, parse_address, parse_reply_to, SendConfig, SmtpServer, Transport, STARTTLS_PORT,
    TLS_PORT,
};
pub use error::{GraphRefusal, SendError, Stage};
pub use frame::Content;
#[cfg(any(test, feature = "test-support"))]
pub use memory_sender::MemorySender;
pub use message::{InlineImage, Message, LOGO_CID, MARK_CID};
pub use sender::{MailSender, NetworkSender, PacedSender, SEND_MIN_INTERVAL};
pub use text::{email_top, footer, from_name, possessive, sign_off, Footer, Letterhead};

use std::time::Duration;

pub use lettre::Address;

/// Upper bound for each step of reaching an SMTP server: the TCP connect,
/// the TLS handshake, the greeting, `STARTTLS` and the login.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for handing over one message over SMTP (`MAIL FROM` to the
/// end of `DATA`).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for one whole [`send`], however many addresses, steps or
/// requests it goes through.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Log in to the submission server `server` and say goodbye, sending
/// nothing: the account test's SMTP line of a password login. Bounded like
/// [`send`].
pub async fn login(server: &SmtpServer) -> Result<(), SendError> {
    let login = smtp::login_only(server, smtp::Handshake::Native);
    match tokio::time::timeout(TOTAL_TIMEOUT, login).await {
        Ok(result) => result,
        Err(_) => Err(SendError::Other {
            message: format!(
                "{}:{} did not finish the login in time.",
                server.host, server.port
            ),
            detail: format!("gave up after {} seconds", TOTAL_TIMEOUT.as_secs()),
        }),
    }
}

/// Send `message` the way `config` says: through its SMTP server (connect,
/// secure the connection, log in, hand the message over, say goodbye; each
/// step with its own limit) or as its mailbox over Microsoft Graph; the
/// whole send within [`TOTAL_TIMEOUT`].
pub async fn send(config: &SendConfig, message: &Message) -> Result<(), SendError> {
    match &config.transport {
        Transport::Smtp(server) => {
            let delivery = smtp::deliver(config, server, message, smtp::Handshake::Native);
            match tokio::time::timeout(TOTAL_TIMEOUT, delivery).await {
                Ok(result) => result,
                Err(_) => Err(SendError::Other {
                    message: format!(
                        "{}:{} did not finish in time; the message may or may not have been sent.",
                        server.host, server.port
                    ),
                    detail: format!("gave up after {} seconds", TOTAL_TIMEOUT.as_secs()),
                }),
            }
        }
        Transport::Graph(mailbox) => {
            let delivery = graph::deliver(config, mailbox, message);
            match tokio::time::timeout(TOTAL_TIMEOUT, delivery).await {
                Ok(result) => result,
                Err(_) => Err(graph::timed_out(mailbox, TOTAL_TIMEOUT)),
            }
        }
    }
}
