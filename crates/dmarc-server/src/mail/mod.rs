//! Outbound mail: Marcan as an SMTP submission client.
//!
//! Mail goes out through the account configured under Settings → Sending
//! ([`SmtpConfig::from_settings`]), over TLS from the start (port 465)
//! or `STARTTLS` (port 587), always logged in. Marcan never listens on port 25,
//! never delivers to a recipient's MX itself and never signs DKIM: SPF and DKIM
//! are the submission provider's job.
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
pub mod image_size;
mod message;
mod sender;
mod smtp;
mod text;

pub(crate) use config::check_port;
pub use config::{default_port, parse_address, same_provider, SmtpConfig, STARTTLS_PORT, TLS_PORT};
pub use error::SendError;
pub use frame::Content;
pub use message::{InlineImage, Message, LOGO_CID, MARK_CID};
pub use sender::{MailSender, MemorySender, PacedSender, SmtpSender, SEND_MIN_INTERVAL};
pub use text::{email_top, footer, from_name, possessive, sign_off, Footer, Letterhead};

use std::time::Duration;

pub use lettre::Address;

/// Upper bound for each step of reaching the server: the TCP connect, the
/// TLS handshake, the greeting, `STARTTLS` and the login.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for handing over one message (`MAIL FROM` to the end of `DATA`).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for one whole [`send`], however many addresses and steps it
/// goes through.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Send `message` through the submission server in `config`: connect, secure
/// the connection, log in, hand the message over and say goodbye. Each step
/// has its own limit, and the whole send [`TOTAL_TIMEOUT`].
pub async fn send(config: &SmtpConfig, message: &Message) -> Result<(), SendError> {
    let delivery = smtp::deliver(config, message, smtp::Handshake::Native);
    match tokio::time::timeout(TOTAL_TIMEOUT, delivery).await {
        Ok(result) => result,
        Err(_) => Err(SendError::Other {
            message: format!(
                "{}:{} did not finish in time; the message may or may not have been sent.",
                config.host, config.port
            ),
            detail: format!("gave up after {} seconds", TOTAL_TIMEOUT.as_secs()),
        }),
    }
}
