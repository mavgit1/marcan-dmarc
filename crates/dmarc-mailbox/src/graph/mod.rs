//! A Microsoft 365 mailbox over Microsoft Graph: reading it, with the
//! settings ([`GraphConfig`], [`GraphApi`]), the requests with their
//! token, retries and error mapping (`client`), the folder lookup and the
//! settings UI's connection test (`folders`, [`probe_graph`]), the
//! [`Source`](crate::source::Source) a pass reads through (`source`);
//! sending a message as it (`send`, [`send_mime`]) through the same
//! requests; and the fake Graph server for tests.
//!
//! The app logs in with the client credentials grant (the host fetches the
//! token, [`GraphToken`]); its access to the mailbox comes from Exchange
//! role assignments scoped to that mailbox (one to read, one to send), so
//! the token names no permission and none is checked here. Reading sends
//! only `GET`s, which neither change the mailbox nor mark mail read; the
//! one other request is a send's `POST` of `sendMail`.
//!
//! The mailbox is `/users/{address}`. The configured folder is a path of
//! display names (`INBOX/DMARC`), `INBOX` being Graph's well-known inbox in
//! any language; it is looked up when the pass opens, and its id is the
//! folder generation, so a folder deleted and created again voids the
//! stored position, quarantine and retry counts as a new UIDVALIDITY does.
//! New mail is listed with the folder's delta query
//! (`messages/delta?changeType=created`); the stored position is the link
//! to ask next (see `source`). Each message is then read raw (`$value`).
//!
//! The token goes only to [`GraphApi::base`]: a link from an answer that
//! points elsewhere is refused, and no redirect is followed.

mod client;
mod folders;
mod send;
mod source;

#[cfg(any(test, feature = "test-support"))]
pub mod fake_server;
#[cfg(test)]
mod tests;

pub use client::is_no_mailbox;
pub use folders::probe_graph;
pub use send::{
    send_mime, GraphSendConfig, SendFailure, MAX_SEND_BODY_BYTES, SEND_REQUEST_TIMEOUT,
    SEND_THROTTLE_WAIT,
};
pub(crate) use source::GraphSource;

use crate::backend::Backend;
use crate::imap::connect::ConnectStage;
use crate::pass::{FailureKind, FetchError, FetchSummary, Pass};
use futures::future::BoxFuture;
use reqwest::Url;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Microsoft Graph in production.
pub const GRAPH_URL: &str = "https://graph.microsoft.com";

/// The Graph endpoint a mailbox is read from and the HTTP client that reads
/// it: [`GRAPH_URL`] in production, a fake one in tests. A constructor
/// argument of the host, never a setting: the token goes nowhere else.
#[derive(Clone)]
pub struct GraphApi {
    http: reqwest::Client,
    base: Url,
}

impl fmt::Debug for GraphApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphApi")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

/// Two endpoints are the same when their base URLs are.
impl PartialEq for GraphApi {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base
    }
}

impl Eq for GraphApi {}

impl GraphApi {
    /// Graph at `base` (scheme, host and port; the API version is added
    /// here), over rustls, following no redirect. Each request sets its
    /// own timeout.
    pub fn new(base: Url) -> Self {
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Marcan")
            .build()
            .unwrap_or_default();
        Self { http, base }
    }

    /// Microsoft Graph ([`GRAPH_URL`]).
    pub fn microsoft() -> Self {
        Self::new(Url::parse(GRAPH_URL).expect("the Graph URL parses"))
    }

    /// Where the requests go.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// The host the requests go to, for the log.
    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }

    /// `url` points at this endpoint (scheme, host and port), under its
    /// API version: the only links the token is sent to.
    pub(crate) fn owns(&self, url: &Url) -> bool {
        url.scheme() == self.base.scheme()
            && url.host_str() == self.base.host_str()
            && url.port_or_known_default() == self.base.port_or_known_default()
            && url.path().starts_with("/v1.0/")
    }

    /// `/v1.0/` and `segments` (each escaped as one path segment) on the
    /// base, with `query` as given.
    pub(crate) fn url(&self, segments: &[&str], query: Option<&str>) -> Url {
        let mut url = self.base.clone();
        url.set_query(None);
        url.set_fragment(None);
        if let Ok(mut path) = url.path_segments_mut() {
            path.clear().push("v1.0").extend(segments);
        }
        url.set_query(query);
        url
    }
}

/// No token could be had: the host's sentence, what its token endpoint
/// said, and the host's docs section on the fix, if one has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenFailure {
    pub message: String,
    pub detail: String,
    pub docs: Option<&'static str>,
}

/// Where a pass gets its access token: the host's token source for the
/// account's login (cached there; a pass may outlive one token).
pub trait GraphToken: Send + Sync {
    /// A token that is valid now.
    fn token(&self) -> BoxFuture<'_, Result<String, TokenFailure>>;

    /// Graph refused the last token (`401`): forget it, so the next
    /// [`Self::token`] fetches a new one.
    fn refused(&self);
}

/// A failure the host words for the person (its sentences say what to
/// change); Graph's own code and message go beside it as the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphProblem<'a> {
    /// Graph could not be reached (`host`).
    Unreachable { host: &'a str },
    /// Graph refused the token, a freshly fetched one too (`401`).
    TokenRefused { mailbox: &'a str },
    /// The app may not read `mailbox` (`403`): the Exchange role does not
    /// cover it, or is not applied yet.
    NoAccess { mailbox: &'a str },
    /// `mailbox` is not a mailbox in the tenant (`404` on the user).
    NoMailbox { mailbox: &'a str },
    /// `mailbox` has no folder `folder`.
    NoFolder { mailbox: &'a str, folder: &'a str },
    /// Graph kept answering `429`/`503`: it throttles the app for now.
    Throttled { mailbox: &'a str },
}

impl GraphProblem<'_> {
    /// What kind of failure of a pass it is.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Unreachable { .. } => FailureKind::Connect(ConnectStage::Tcp),
            Self::TokenRefused { .. } | Self::NoAccess { .. } | Self::NoMailbox { .. } => {
                FailureKind::Connect(ConnectStage::Auth)
            }
            Self::NoFolder { .. } | Self::Throttled { .. } => FailureKind::Server,
        }
    }
}

/// How the host says a [`GraphProblem`].
pub type GraphWording = fn(&GraphProblem<'_>) -> String;

/// The section of the host's docs that shows how to fix a [`GraphProblem`],
/// if one does (relative to the docs' root; this crate only passes it on).
pub type GraphDocs = fn(&GraphProblem<'_>) -> Option<&'static str>;

/// What a pass over Graph reads, as the host prepares it. The token is
/// never printed by [`Debug`].
#[derive(Clone)]
pub struct GraphConfig {
    /// The mailbox's id ([`MailboxRef::id`](crate::MailboxRef::id)).
    pub id: String,
    /// See [`ImapConfig::epoch`](crate::ImapConfig::epoch).
    pub epoch: i64,
    /// The mailbox's address: the user the requests name.
    pub address: String,
    /// The folder's path of display names (default `INBOX`).
    pub folder: String,
    pub api: GraphApi,
    pub token: Arc<dyn GraphToken>,
    pub say: GraphWording,
    pub docs: GraphDocs,
}

impl fmt::Debug for GraphConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphConfig")
            .field("id", &self.id)
            .field("epoch", &self.epoch)
            .field("address", &self.address)
            .field("folder", &self.folder)
            .field("api", &self.api)
            .field("token", &"***")
            .finish_non_exhaustive()
    }
}

/// The same mailbox, folder and endpoint, prepared under the same epoch;
/// the token source and the wording are not compared.
impl PartialEq for GraphConfig {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.epoch == other.epoch
            && self.address == other.address
            && self.folder == other.folder
            && self.api == other.api
    }
}

impl Eq for GraphConfig {}

impl GraphConfig {
    /// `text` with anything shaped like a token masked: a `Bearer`
    /// credential and a JWT. A pass's own texts mask the tokens it sent
    /// already; this is for what reaches the UI from elsewhere.
    pub fn redact(&self, text: &str) -> String {
        redact_tokens(text)
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '=')
}

/// `text` with `Bearer <credential>` and JWTs (`eyJ…` with a dot) masked.
pub(crate) fn redact_tokens(text: &str) -> String {
    let earliest = |s: &str| {
        [s.find("Bearer "), s.find("eyJ")]
            .into_iter()
            .flatten()
            .min()
    };
    let run = |s: &str| s.find(|c: char| !is_token_char(c)).unwrap_or(s.len());
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = earliest(rest) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        if let Some(after) = tail.strip_prefix("Bearer ") {
            out.push_str("Bearer ");
            let len = run(after);
            if len > 0 {
                out.push_str("***");
            }
            rest = &after[len..];
        } else {
            let len = run(tail);
            let word = &tail[..len];
            out.push_str(if word.contains('.') { "***" } else { word });
            rest = &tail[len..];
        }
    }
    out.push_str(rest);
    out
}

/// Run `pass` over `cfg`'s mailbox.
pub(crate) async fn run<B: Backend>(
    pass: &mut Pass<'_, B>,
    cfg: &GraphConfig,
) -> Result<FetchSummary, FetchError> {
    info!(host = %cfg.api.host(), folder = %cfg.folder, "reading the mailbox over Microsoft Graph");
    let limits = *pass.limits;
    pass.run_on(GraphSource::new(cfg, limits)).await
}
