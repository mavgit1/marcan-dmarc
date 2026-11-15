//! Requests to Graph. A [`Session`] sends each with the current token, a
//! refused token fetched anew once (`401`), throttling waited out within a
//! bound (`429`/`503` with `Retry-After`), every request bounded in time,
//! only to the configured endpoint, no redirect followed; a pass's or a
//! test's [`Client`] reads with it (`GET`s) and turns Graph's refusals into
//! [`Fault`]s worded by the host, and a send (`send`) posts the message
//! with it. No text either builds holds a token it sent.

use super::{
    redact_tokens, GraphApi, GraphConfig, GraphDocs, GraphProblem, GraphToken, GraphWording,
};
use crate::imap::connect::{ConnectError, ConnectStage};
use crate::pass::{Failure, FailureKind};
use reqwest::{header, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// The most a JSON answer (a folder list, a delta page) may hold.
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// The most of an error answer that is read.
const MAX_ERROR_BYTES: usize = 64 * 1024;

/// How often one request is retried after `429`/`503`.
const MAX_THROTTLE_RETRIES: u32 = 4;

/// The most one read request waits for throttling in all; a longer wait
/// fails the pass, whose back-off then applies.
pub(crate) const MAX_THROTTLE_WAIT: Duration = Duration::from_secs(120);

/// Tokens shorter than this are not searched for in texts.
const MIN_MASKED: usize = 8;

/// The wait when a `429`/`503` names none.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(5);

/// A failed request: what the pass records or the test shows. `message`
/// is the plain sentence, `detail` Graph's code and message (or the
/// transport's error). Neither holds a token. `docs` is the host's docs
/// section on the fix, passed on to the test (a pass records no link).
/// `worded`: the host worded `message` (a sentence that stands alone, so a
/// pass records `detail` apart); else it is this crate's terse "what
/// failed", which needs `detail` beside it to say anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fault {
    pub kind: FailureKind,
    pub message: String,
    pub detail: String,
    pub docs: Option<&'static str>,
    pub worded: bool,
}

impl Fault {
    fn new(kind: FailureKind, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: detail.into(),
            docs: None,
            worded: false,
        }
    }

    /// The host's sentence `message`, with `detail` and `docs` beside it.
    fn worded(
        kind: FailureKind,
        message: String,
        detail: String,
        docs: Option<&'static str>,
    ) -> Self {
        Self {
            docs,
            worded: true,
            ..Self::new(kind, message, detail)
        }
    }

    /// A failure in this crate's own words (`what failed: detail`), like
    /// IMAP's command errors.
    pub(crate) fn server(what: &str, detail: impl fmt::Display) -> Self {
        Self::new(
            FailureKind::Server,
            format!("{what} failed"),
            detail.to_string(),
        )
    }

    /// As a pass records it: the host's sentence with Graph's answer apart
    /// (the UI shows it behind Details), this crate's own words with the
    /// answer in brackets, as one text.
    pub(crate) fn into_failure(self) -> Failure {
        if self.worded {
            Failure::with_detail(self.kind, self.message, self.detail)
        } else if self.detail.is_empty() {
            Failure::message(self.kind, self.message)
        } else {
            Failure::message(self.kind, format!("{} ({})", self.message, self.detail))
        }
    }

    /// As the connection test shows it.
    pub(crate) fn into_connect(self) -> ConnectError {
        ConnectError {
            stage: match self.kind {
                FailureKind::Connect(stage) => stage,
                _ => ConnectStage::Other,
            },
            message: self.message,
            detail: self.detail,
            docs: self.docs,
        }
    }
}

impl From<Fault> for Failure {
    fn from(fault: Fault) -> Self {
        fault.into_failure()
    }
}

/// Graph's answer to a request it did not carry out:
/// `{"error":{"code":"…","message":"…"}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl GraphError {
    /// Graph's words, for the detail: `ErrorAccessDenied: Access is denied.`
    pub(crate) fn detail(&self) -> String {
        match (self.code.as_str(), self.message.as_str()) {
            ("", "") => format!("HTTP {}", self.status.as_u16()),
            ("", message) => format!("HTTP {}: {message}", self.status.as_u16()),
            (code, "") => format!("HTTP {}: {code}", self.status.as_u16()),
            (code, message) => format!("{code}: {message}"),
        }
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorFields>,
}

#[derive(Deserialize)]
struct ErrorFields {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// What a request came back with.
pub(crate) enum Reply {
    /// A success (2xx): the body is the caller's to read.
    Ok(Response),
    /// Graph refused (any status but 401, 429 and 503, which are handled
    /// here first).
    Refused(GraphError),
}

/// What a refusal of a request is about, so a `404` says the right thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// The mailbox itself (its folders): a `404` is no such mailbox.
    Mailbox,
    /// Something in it (a folder's children, its delta, a message).
    Item,
}

/// Why a request came back with nothing to read, before any wording: the
/// reading [`Client`] says it as a [`Fault`], the sender (`send`) as a
/// [`SendFailure`](super::SendFailure). No text in it holds a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The host's token source gave none (its sentence, detail and docs
    /// section).
    Token {
        message: String,
        detail: String,
        docs: Option<&'static str>,
    },
    /// Graph refused the token, a freshly fetched one too (`401`).
    TokenRefused(GraphError),
    /// Graph kept answering `429`/`503` past the wait allowed.
    Throttled(GraphError),
    /// No answer within the request's limit.
    Timeout,
    /// No connection to Graph.
    Unreachable(String),
    /// Another transport error.
    Transport(String),
    /// A redirect (`status`, its masked `Location`): never followed.
    Redirect { status: u16, location: String },
}

/// The token and HTTP side of one pass, one test or one send: each request
/// with the current token, a refused token fetched anew once (`401`),
/// throttling waited out within a bound (`429`/`503` with `Retry-After`),
/// only to the endpoint, no redirect followed, and every token it sent
/// masked in the texts it builds.
pub(crate) struct Session {
    api: GraphApi,
    provider: Arc<dyn GraphToken>,
    token: Option<String>,
    /// Every token sent, masked in every text built.
    sent: Vec<String>,
}

impl Session {
    pub(crate) fn new(api: &GraphApi, provider: &Arc<dyn GraphToken>) -> Self {
        Self {
            api: api.clone(),
            provider: Arc::clone(provider),
            token: None,
            sent: Vec::new(),
        }
    }

    pub(crate) fn api(&self) -> &GraphApi {
        &self.api
    }

    /// `text` without any token this session sent, nor anything shaped like one.
    pub(crate) fn mask(&self, text: &str) -> String {
        let mut text = text.to_owned();
        // Real tokens are hundreds of characters; a few letters would mask
        // words instead.
        for token in self.sent.iter().filter(|t| t.len() >= MIN_MASKED) {
            text = text.replace(token.as_str(), "***");
        }
        redact_tokens(&text)
    }

    /// The current token: the one in hand, else the provider's.
    async fn current(&mut self) -> Result<String, Stop> {
        if let Some(token) = &self.token {
            return Ok(token.clone());
        }
        let token = self.provider.token().await.map_err(|e| Stop::Token {
            message: e.message,
            detail: self.mask(&e.detail),
            docs: e.docs,
        })?;
        if !self.sent.contains(&token) {
            self.sent.push(token.clone());
        }
        self.token = Some(token.clone());
        Ok(token)
    }

    /// The request `build` makes from the client (with the current token
    /// set on it and `limit` as its timeout), sent until Graph answers
    /// something other than a refused token or throttling: at most one new
    /// token, and throttling waited out up to `max_wait` in all. `what`
    /// names it in the log. The caller has checked that the request goes
    /// to [`Self::api`].
    pub(crate) async fn call(
        &mut self,
        what: &str,
        limit: Duration,
        max_wait: Duration,
        build: impl Fn(&reqwest::Client) -> RequestBuilder,
    ) -> Result<Reply, Stop> {
        let mut refreshed = false;
        let mut retries = 0;
        let mut waited = Duration::ZERO;
        loop {
            let token = self.current().await?;
            let response = build(&self.api.http)
                .bearer_auth(&token)
                .timeout(limit)
                .send()
                .await
                .map_err(|e| self.transport(e))?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED {
                let error = self.error_of(response).await;
                if refreshed {
                    return Err(Stop::TokenRefused(error));
                }
                debug!(detail = %error.detail(), "Graph refused the token; fetching a new one");
                self.provider.refused();
                self.token = None;
                refreshed = true;
                continue;
            }
            if matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
            ) {
                let wait = retry_after(&response).unwrap_or(DEFAULT_RETRY_AFTER);
                let error = self.error_of(response).await;
                if retries < MAX_THROTTLE_RETRIES && waited + wait <= max_wait {
                    warn!(
                        status = status.as_u16(),
                        wait_secs = wait.as_secs(),
                        "{what}: Graph is throttling; waiting before asking again"
                    );
                    tokio::time::sleep(wait).await;
                    retries += 1;
                    waited += wait;
                    continue;
                }
                return Err(Stop::Throttled(error));
            }
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                return Err(Stop::Redirect {
                    status: status.as_u16(),
                    location: self.mask(location),
                });
            }
            if status.is_success() {
                return Ok(Reply::Ok(response));
            }
            return Ok(Reply::Refused(self.error_of(response).await));
        }
    }

    /// A request that went wrong before an answer: a timeout, no
    /// connection, or the transport's error (without the URL).
    pub(crate) fn transport(&self, e: reqwest::Error) -> Stop {
        if e.is_timeout() {
            return Stop::Timeout;
        }
        let connect = e.is_connect();
        let detail = self.mask(&e.without_url().to_string());
        if connect {
            Stop::Unreachable(detail)
        } else {
            Stop::Transport(detail)
        }
    }

    /// The error of a refusal; a body that is no Graph error keeps its
    /// first line as the message.
    async fn error_of(&self, mut response: Response) -> GraphError {
        let status = response.status();
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            let room = MAX_ERROR_BYTES.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if body.len() >= MAX_ERROR_BYTES {
                break;
            }
        }
        let (code, message) = match serde_json::from_slice::<ErrorBody>(&body) {
            Ok(ErrorBody {
                error: Some(fields),
            }) => (fields.code, fields.message),
            _ => {
                let text = String::from_utf8_lossy(&body);
                let line: String = text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect();
                (String::new(), line)
            }
        };
        GraphError {
            status,
            code,
            message: self.mask(&message),
        }
    }
}

/// The requests of one pass or one test: its [`Session`], and the
/// mailbox and wording its faults are said with.
pub(crate) struct Client {
    session: Session,
    say: GraphWording,
    docs: GraphDocs,
    pub(crate) mailbox: String,
}

impl Client {
    pub(crate) fn new(cfg: &GraphConfig) -> Self {
        Self {
            session: Session::new(&cfg.api, &cfg.token),
            say: cfg.say,
            docs: cfg.docs,
            mailbox: cfg.address.clone(),
        }
    }

    pub(crate) fn api(&self) -> &GraphApi {
        self.session.api()
    }

    /// `text` without any token this client sent, nor anything shaped like one.
    pub(crate) fn mask(&self, text: &str) -> String {
        self.session.mask(text)
    }

    /// The sentence for `problem`, with `detail` and its docs section beside it.
    pub(crate) fn fault(&self, problem: &GraphProblem<'_>, detail: &str) -> Fault {
        Fault::worded(
            problem.kind(),
            (self.say)(problem),
            self.mask(detail),
            (self.docs)(problem),
        )
    }

    /// The fault of a refusal at `level` of `what`: no access, no mailbox,
    /// or Graph's error in this crate's words.
    pub(crate) fn refusal(&self, error: &GraphError, what: &str, level: Level) -> Fault {
        let mailbox = self.mailbox.as_str();
        let detail = error.detail();
        match (error.status, level) {
            (StatusCode::FORBIDDEN, _) => self.fault(&GraphProblem::NoAccess { mailbox }, &detail),
            (StatusCode::NOT_FOUND, Level::Mailbox) => {
                self.fault(&GraphProblem::NoMailbox { mailbox }, &detail)
            }
            _ if is_no_mailbox(&error.code) => {
                self.fault(&GraphProblem::NoMailbox { mailbox }, &detail)
            }
            _ => Fault::server(what, self.mask(&detail)),
        }
    }

    /// A link from an answer that leads off the endpoint: refused before
    /// anything is sent to it.
    pub(crate) fn foreign(&self, what: &str, link: &str) -> Fault {
        Fault::server(what, foreign_detail(self.api(), link))
    }

    /// `stop` in this pass's words: `what` names the request, `limit` its
    /// timeout.
    fn stopped(&self, stop: Stop, what: &str, limit: Duration) -> Fault {
        let mailbox = self.mailbox.as_str();
        match stop {
            Stop::Token {
                message,
                detail,
                docs,
            } => Fault::worded(
                FailureKind::Connect(ConnectStage::Auth),
                message,
                detail,
                docs,
            ),
            Stop::TokenRefused(error) => {
                self.fault(&GraphProblem::TokenRefused { mailbox }, &error.detail())
            }
            Stop::Throttled(error) => {
                self.fault(&GraphProblem::Throttled { mailbox }, &error.detail())
            }
            Stop::Timeout => Fault::new(
                FailureKind::Timeout,
                format!("{what} timed out after {} seconds", limit.as_secs()),
                String::new(),
            ),
            Stop::Unreachable(detail) => {
                let host = self.api().host();
                self.fault(&GraphProblem::Unreachable { host }, &detail)
            }
            Stop::Transport(detail) => Fault::server(what, detail),
            Stop::Redirect { status, location } => Fault::server(
                what,
                format!("HTTP {status}: redirected to {location}; Marcan follows no redirect"),
            ),
        }
    }

    /// `GET url` (`what` names it in errors), within `limit` including its
    /// body, with `prefer` as the `Prefer` header.
    pub(crate) async fn get(
        &mut self,
        url: &Url,
        what: &str,
        limit: Duration,
        prefer: Option<&str>,
    ) -> Result<Reply, Fault> {
        if !self.api().owns(url) {
            return Err(self.foreign(what, url.as_str()));
        }
        let request = |http: &reqwest::Client| {
            let request = http.get(url.clone());
            match prefer {
                Some(prefer) => request.header("Prefer", prefer),
                None => request,
            }
        };
        match self
            .session
            .call(what, limit, MAX_THROTTLE_WAIT, request)
            .await
        {
            Ok(reply) => Ok(reply),
            Err(stop) => Err(self.stopped(stop, what, limit)),
        }
    }

    /// The JSON body of a success, at most [`MAX_JSON_BYTES`].
    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        mut response: Response,
        what: &str,
        limit: Duration,
    ) -> Result<T, Fault> {
        let mut body = Vec::new();
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => return Err(self.stopped(self.session.transport(e), what, limit)),
            };
            if body.len() + chunk.len() > MAX_JSON_BYTES {
                return Err(Fault::server(
                    what,
                    format!("the answer is larger than {MAX_JSON_BYTES} bytes"),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|e| Fault::server(what, format!("the answer is no JSON Marcan reads: {e}")))
    }

    /// `GET url` and its JSON at `level`: a refusal is a fault.
    pub(crate) async fn get_json<T: DeserializeOwned>(
        &mut self,
        url: &Url,
        what: &str,
        limit: Duration,
        level: Level,
    ) -> Result<Result<T, GraphError>, Fault> {
        match self.get(url, what, limit, None).await? {
            Reply::Ok(response) => self.json(response, what, limit).await.map(Ok),
            Reply::Refused(error)
                if error.status == StatusCode::NOT_FOUND && level == Level::Item =>
            {
                Ok(Err(error))
            }
            Reply::Refused(error) => Err(self.refusal(&error, what, level)),
        }
    }
}

/// Why a link off the endpoint is not followed.
pub(crate) fn foreign_detail(api: &GraphApi, link: &str) -> String {
    let host = Url::parse(link)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "an address that does not parse".to_owned());
    format!(
        "the answer links to {host}; Marcan sends the token only to {}",
        api.host()
    )
}

/// Whether Graph's error `code` says the user is no mailbox here (unknown,
/// or without an Exchange Online mailbox): reading and sending both ask.
pub fn is_no_mailbox(code: &str) -> bool {
    [
        "ErrorInvalidUser",
        "MailboxNotEnabledForRESTAPI",
        "MailboxNotHostedInExchangeOnline",
    ]
    .iter()
    .any(|c| c.eq_ignore_ascii_case(code))
}

/// The `Retry-After` of `response` in seconds (Graph sends seconds).
fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
        .map(Duration::from_secs)
}
