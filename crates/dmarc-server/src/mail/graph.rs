//! The Microsoft Graph transport: a Microsoft login's message built as
//! for SMTP ([`build`]) and posted as its mailbox
//! ([`dmarc_mailbox::send_mime`]: the token, retries and redaction are the
//! mailbox crate's, shared with reading), and Graph's refusals as
//! [`SendError`]s.
//!
//! Graph takes the recipients from the message's headers: `build` writes
//! the one recipient into `To` and nothing into `Bcc`, so the message goes
//! to exactly the address SMTP's `RCPT TO` would name. Graph answers once
//! the message is on its way, so a recipient's server refusing it shows as
//! a bounce in the account's mailbox, not here.

use super::mime::build;
use super::{GraphRefusal, Message, SendConfig, SendError};
use dmarc_mailbox::{send_mime, GraphSendConfig, SendFailure, MAX_SEND_BODY_BYTES};
use std::time::Duration;
use tracing::info;

/// Send `message` as `mailbox`, as `config` builds it.
pub(crate) async fn deliver(
    config: &SendConfig,
    mailbox: &GraphSendConfig,
    message: &Message,
) -> Result<(), SendError> {
    let email = build(config, message)?;
    info!(mailbox = %mailbox.address, to = %message.to, "sending mail");
    deliver_mime(mailbox, &email.formatted()).await
}

/// Send the built message `mime` as `mailbox`.
pub(crate) async fn deliver_mime(mailbox: &GraphSendConfig, mime: &[u8]) -> Result<(), SendError> {
    send_mime(mailbox, mime)
        .await
        .map_err(|failure| send_error(mailbox, failure))
}

/// The whole send of `mailbox`'s message took longer than `limit`.
pub(crate) fn timed_out(mailbox: &GraphSendConfig, limit: Duration) -> SendError {
    SendError::Graph {
        mailbox: mailbox.address.clone(),
        refusal: GraphRefusal::Timeout {
            seconds: limit.as_secs(),
        },
        detail: format!("gave up after {} seconds", limit.as_secs()),
    }
}

/// Graph's codes for a message larger than it or the mailbox takes.
const TOO_LARGE_CODES: [&str; 3] = [
    "ErrorMessageSizeExceeded",
    "RequestEntityTooLarge",
    "ErrorRequestEntityTooLarge",
];

/// `failure` of sending a message as `mailbox`, by what is to be changed.
fn send_error(mailbox: &GraphSendConfig, failure: SendFailure) -> SendError {
    let graph = |refusal: GraphRefusal, detail: String| SendError::Graph {
        mailbox: mailbox.address.clone(),
        refusal,
        detail,
    };
    let is = |codes: &[&str], code: &str| codes.iter().any(|c| c.eq_ignore_ascii_case(code));
    match failure {
        SendFailure::TooLarge { bytes } => graph(
            GraphRefusal::TooLarge,
            format!("{bytes} bytes in base64; Microsoft Graph takes up to {MAX_SEND_BODY_BYTES}"),
        ),
        SendFailure::Token {
            message,
            detail,
            docs,
        } => SendError::Token {
            message,
            detail,
            docs,
        },
        SendFailure::TokenRefused { detail } => graph(GraphRefusal::TokenRefused, detail),
        SendFailure::Throttled { detail } => graph(GraphRefusal::Throttled, detail),
        SendFailure::Unreachable { host, detail } => graph(GraphRefusal::Unreachable { host }, detail),
        SendFailure::Timeout { seconds } => graph(
            GraphRefusal::Timeout { seconds },
            format!("no answer in {seconds} seconds"),
        ),
        SendFailure::Refused {
            status,
            code,
            detail,
        } => {
            // Marcan sends as the mailbox only: a send-as refusal is the
            // missing (or not yet applied) send role too.
            let refusal = if status == 413 || is(&TOO_LARGE_CODES, &code) {
                GraphRefusal::TooLarge
            } else if status == 403 || code.eq_ignore_ascii_case("ErrorSendAsDenied") {
                GraphRefusal::NoSendRole
            } else if status == 404 || dmarc_mailbox::is_no_mailbox(&code) {
                GraphRefusal::NoMailbox
            } else {
                GraphRefusal::Refused
            };
            graph(refusal, detail)
        }
        SendFailure::Other { detail } => SendError::Other {
            message: "The request to Microsoft Graph broke off; the message may or may not have been sent.".to_owned(),
            detail,
        },
    }
}

#[cfg(test)]
pub(crate) mod tests;
