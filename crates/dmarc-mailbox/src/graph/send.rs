//! Sending a message as a Microsoft 365 mailbox over Microsoft Graph:
//! `POST /v1.0/users/{address}/sendMail` with the whole RFC 5322 message
//! in base64 (`Content-Type: text/plain`, Graph's MIME form), through the
//! same [`Session`] as reading: the token from the host, a refused token
//! fetched anew once, throttling waited out within [`SEND_THROTTLE_WAIT`],
//! no redirect followed, no token in any text.
//!
//! Graph takes the recipients from the message's own `To`, `Cc` and `Bcc`
//! headers and answers `202 Accepted` once the message is in the mailbox's
//! Drafts on its way out; whether a recipient's server accepts it shows
//! later, as a bounce in that mailbox. A copy stays in its Sent Items: the
//! MIME form has no way to ask otherwise. What a failure means is the
//! host's to say ([`SendFailure`]): this crate does not word it.

use super::client::{GraphError, Reply, Session, Stop};
use super::{GraphApi, GraphToken};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// The largest request body Graph takes (4 MB): the message in base64, so
/// a message of about 3 MB. A larger one is not sent at all.
pub const MAX_SEND_BODY_BYTES: usize = 4 * 1024 * 1024;

/// How long one `sendMail` request may take, the message's upload included.
pub const SEND_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The most one send waits for throttling in all; a longer wait fails it
/// ([`SendFailure::Throttled`]), and the caller's next run tries again.
pub const SEND_THROTTLE_WAIT: Duration = Duration::from_secs(20);

/// The mailbox a message is sent as, as the host prepares it. The token is
/// never printed by [`Debug`].
#[derive(Clone)]
pub struct GraphSendConfig {
    /// The mailbox's address: the user the request names.
    pub address: String,
    pub api: GraphApi,
    pub token: Arc<dyn GraphToken>,
}

impl fmt::Debug for GraphSendConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphSendConfig")
            .field("address", &self.address)
            .field("api", &self.api)
            .field("token", &"***")
            .finish()
    }
}

/// The same mailbox at the same endpoint; the token source is not compared.
impl PartialEq for GraphSendConfig {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address && self.api == other.api
    }
}

impl Eq for GraphSendConfig {}

/// Why Graph did not take the message. `detail` is Graph's code and
/// message (or the transport's error), never a token; the host says what
/// each means. Only [`SendFailure::Timeout`] leaves open whether the
/// message went out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendFailure {
    /// The message is larger than [`MAX_SEND_BODY_BYTES`] in base64; nothing
    /// was sent.
    TooLarge { bytes: usize },
    /// The host's token source gave no token (its sentence, detail and
    /// docs section).
    Token {
        message: String,
        detail: String,
        docs: Option<&'static str>,
    },
    /// Graph refused the token, a freshly fetched one too (`401`).
    TokenRefused { detail: String },
    /// Graph kept answering `429`/`503` past [`SEND_THROTTLE_WAIT`].
    Throttled { detail: String },
    /// Graph could not be reached at `host`.
    Unreachable { host: String, detail: String },
    /// No answer within [`SEND_REQUEST_TIMEOUT`]: the message may or may
    /// not have been sent.
    Timeout { seconds: u64 },
    /// Graph answered with an error: its HTTP `status` and `code`
    /// (`ErrorAccessDenied`, `ErrorSendAsDenied`, …).
    Refused {
        status: u16,
        code: String,
        detail: String,
    },
    /// Anything else (a redirect, a broken connection), in this crate's
    /// words.
    Other { detail: String },
}

/// Send `mime` (a whole RFC 5322 message, as an SMTP server would get it
/// after `DATA`) as `cfg`'s mailbox.
pub async fn send_mime(cfg: &GraphSendConfig, mime: &[u8]) -> Result<(), SendFailure> {
    const WHAT: &str = "sendMail";
    let body = STANDARD.encode(mime);
    if body.len() > MAX_SEND_BODY_BYTES {
        return Err(SendFailure::TooLarge { bytes: body.len() });
    }
    let url = cfg.api.url(&["users", &cfg.address, "sendMail"], None);
    info!(host = %cfg.api.host(), bytes = mime.len(), "sending mail over Microsoft Graph");
    let mut session = Session::new(&cfg.api, &cfg.token);
    let request = |http: &reqwest::Client| {
        http.post(url.clone())
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body.clone())
    };
    let reply = session
        .call(WHAT, SEND_REQUEST_TIMEOUT, SEND_THROTTLE_WAIT, request)
        .await
        .map_err(|stop| failure(stop, cfg.api.host()))?;
    match reply {
        Reply::Ok(_) => Ok(()),
        Reply::Refused(error) => Err(refused(error)),
    }
}

fn refused(error: GraphError) -> SendFailure {
    SendFailure::Refused {
        status: error.status.as_u16(),
        detail: error.detail(),
        code: error.code,
    }
}

fn failure(stop: Stop, host: &str) -> SendFailure {
    match stop {
        Stop::Token {
            message,
            detail,
            docs,
        } => SendFailure::Token {
            message,
            detail,
            docs,
        },
        Stop::TokenRefused(error) => SendFailure::TokenRefused {
            detail: error.detail(),
        },
        Stop::Throttled(error) => SendFailure::Throttled {
            detail: error.detail(),
        },
        Stop::Timeout => SendFailure::Timeout {
            seconds: SEND_REQUEST_TIMEOUT.as_secs(),
        },
        Stop::Unreachable(detail) => SendFailure::Unreachable {
            host: host.to_owned(),
            detail,
        },
        Stop::Transport(detail) => SendFailure::Other { detail },
        Stop::Redirect { status, location } => SendFailure::Other {
            detail: format!("HTTP {status}: redirected to {location}; Marcan follows no redirect"),
        },
    }
}
