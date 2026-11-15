//! Access tokens for a Microsoft login: the OAuth 2.0 client credentials
//! grant of an Entra app registration ([`TokenSource`]), proved with a
//! client assertion signed with the account's certificate key
//! (`certificate`), for Microsoft Graph ([`GRAPH_SCOPE`]), which reads the
//! mailbox and sends as it with the same token.
//!
//! Tokens are kept in memory only, per tenant, client ID and certificate (a
//! new certificate fetches a new token; two accounts of one app
//! registration with different certificates each keep their own), and
//! refreshed [`REFRESH_BEFORE`] they expire, [`PASS_REFRESH_BEFORE`] when a
//! mailbox pass starts. Expiry is checked on the monotonic clock and on the
//! wall clock, since the monotonic one may stop while the host sleeps.
//! A token Graph refused, and the tokens of an account saved or deleted,
//! are forgotten ([`TokenSource::forget`], [`TokenSource::forget_login`]).
//! Requests for the same login while one is on its way wait for it instead
//! of asking again. The token endpoint is
//! the constructor's (`login.microsoftonline.com` in production, a fake one
//! in tests), never a setting: the signed assertion goes nowhere else, and
//! the client follows no redirect.
//!
//! The token goes only to the accounts' Graph endpoint (a constructor
//! argument like the token endpoint).

use super::certificate::client_assertion;
use crate::wording::microsoft::{token_error_docs, token_error_sentence};
use dmarc_mailbox::{GraphToken, TokenFailure};
use futures::future::BoxFuture;
use reqwest::Url;
use ring::digest::{digest, SHA256};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

/// The Microsoft identity platform: where tokens come from in production.
pub const MICROSOFT_LOGIN_URL: &str = "https://login.microsoftonline.com";

/// The `scope` of every grant: Microsoft Graph, with the access the
/// Exchange role assignments scoped to the mailbox give the app (the token
/// names none).
pub const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";

/// A cached token is replaced this long before it expires (a send or a
/// pass's next request, which uses it at once).
pub const REFRESH_BEFORE: Duration = Duration::from_mins(5);

/// A mailbox pass starts with a token that lasts at least this long, so a
/// short pass needs no other (a long one asks again, [`GraphAccountToken`]).
pub const PASS_REFRESH_BEFORE: Duration = Duration::from_mins(20);

/// The longest token answer read.
const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// `text` has the shape of a GUID: 8-4-4-4-12 hex digits (the tenant and
/// client IDs Entra shows).
pub fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, len)| g.len() == len && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// What the app registration proves itself with: the certificate uploaded
/// to it and its private key (PKCS#8 PEM). Never printed by
/// [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct ClientCertificate {
    pub private_key_pem: String,
    pub cert_der: Vec<u8>,
}

impl fmt::Debug for ClientCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientCertificate(***)")
    }
}

impl ClientCertificate {
    /// A fingerprint that changes with the key and certificate (SHA-256,
    /// hex): what the cache tells two certificates of one login apart by.
    fn fingerprint(&self) -> String {
        let input = [self.private_key_pem.as_bytes(), &self.cert_der].concat();
        digest(&SHA256, &input)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// One Microsoft login: whose token to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub tenant_id: String,
    pub client_id: String,
    pub certificate: ClientCertificate,
}

/// Why no token was issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenErrorKind {
    /// No such tenant (AADSTS90002, AADSTS900023).
    WrongTenant,
    /// No app registration with the client ID in the tenant (AADSTS700016).
    UnknownClient,
    /// The signed assertion was refused: the certificate is not uploaded,
    /// or the assertion is invalid or out of time (AADSTS700027,
    /// AADSTS700024, AADSTS700023, AADSTS7000274).
    AssertionRejected,
    /// Another refusal: the detail has Microsoft's code and description.
    Refused,
    /// The token endpoint could not be reached.
    Unreachable,
    /// The token endpoint answered something that is no token answer.
    BadAnswer,
    /// The stored credential cannot be used (a key that does not read).
    BadCredential,
}

impl TokenErrorKind {
    pub const ALL: [Self; 7] = [
        Self::WrongTenant,
        Self::UnknownClient,
        Self::AssertionRejected,
        Self::Refused,
        Self::Unreachable,
        Self::BadAnswer,
        Self::BadCredential,
    ];

    /// The kind of refusal AADSTS `code` is.
    pub fn of_code(code: i64) -> Self {
        match code {
            90002 | 900_023 => Self::WrongTenant,
            700_016 => Self::UnknownClient,
            700_027 | 700_024 | 700_023 | 7_000_274 => Self::AssertionRejected,
            _ => Self::Refused,
        }
    }
}

/// No token: `message` in plain words ([`token_error_sentence`]), `detail`
/// what the endpoint said (the AADSTS code and description), `docs` the
/// guide's section on the fix ([`token_error_docs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError {
    pub kind: TokenErrorKind,
    pub message: String,
    pub detail: String,
    pub docs: Option<&'static str>,
}

impl TokenError {
    pub fn new(kind: TokenErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            message: token_error_sentence(kind).to_owned(),
            detail: detail.into(),
            docs: token_error_docs(kind),
        }
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{} ({})", self.message, self.detail)
        }
    }
}

impl std::error::Error for TokenError {}

/// A token and when it expires, on both clocks.
#[derive(Clone)]
struct Cached {
    token: String,
    /// Monotonic: immune to clock changes, but may stop while the host or
    /// VM sleeps.
    expires: Instant,
    /// Wall clock (unix seconds): keeps running through a sleep.
    expires_unix: i64,
}

impl Cached {
    fn new(token: String, lifetime: Duration) -> Self {
        Self {
            token,
            expires: Instant::now() + lifetime,
            expires_unix: dmarc_store::unix_now()
                .saturating_add(i64::try_from(lifetime.as_secs()).unwrap_or(i64::MAX)),
        }
    }

    /// Whether it still lasts more than `left` at `now` and `unix` (unix
    /// seconds): by both clocks, so either one running out expires it.
    fn lasts(&self, left: Duration, now: Instant, unix: i64) -> bool {
        let left_secs = i64::try_from(left.as_secs()).unwrap_or(i64::MAX);
        now + left < self.expires && unix.saturating_add(left_secs) < self.expires_unix
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    tenant: String,
    client: String,
    credential: String,
}

type Slot = Arc<tokio::sync::Mutex<Option<Cached>>>;

/// Tokens from one token endpoint, cached per login.
pub struct TokenSource {
    http: reqwest::Client,
    base: Url,
    cache: Mutex<HashMap<CacheKey, Slot>>,
}

impl fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSource")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

/// The token endpoint's answer, success or error.
#[derive(Deserialize)]
struct Answer {
    access_token: Option<String>,
    expires_in: Option<serde_json::Value>,
    error: Option<String>,
    error_description: Option<String>,
    #[serde(default)]
    error_codes: Vec<i64>,
}

/// The AADSTS code at the start of `description`, if any.
fn code_in(description: &str) -> Option<i64> {
    let rest = description.trim_start().strip_prefix("AADSTS")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl TokenSource {
    /// Tokens from `{token_base_url}/{tenant}/oauth2/v2.0/token` through
    /// `http` (which should follow no redirect).
    pub fn new(http: reqwest::Client, token_base_url: Url) -> Self {
        Self {
            http,
            base: token_base_url,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Tokens from Microsoft's identity platform, over rustls, following no
    /// redirect, each request bounded in time.
    pub fn microsoft() -> Self {
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Marcan")
            .build()
            .unwrap_or_default();
        let base = Url::parse(MICROSOFT_LOGIN_URL).expect("the Microsoft login URL parses");
        Self::new(http, base)
    }

    /// The token endpoint of `tenant` (a GUID, checked here too: it goes
    /// into the path).
    pub fn token_url(&self, tenant: &str) -> Result<Url, TokenError> {
        if !is_guid(tenant) {
            return Err(TokenError::new(
                TokenErrorKind::WrongTenant,
                format!("tenant ID {tenant:?} is no GUID"),
            ));
        }
        let base = self.base.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}/{tenant}/oauth2/v2.0/token"))
            .map_err(|e| TokenError::new(TokenErrorKind::BadAnswer, format!("token endpoint: {e}")))
    }

    fn key(request: &TokenRequest) -> CacheKey {
        CacheKey {
            tenant: request.tenant_id.to_ascii_lowercase(),
            client: request.client_id.to_ascii_lowercase(),
            credential: request.certificate.fingerprint(),
        }
    }

    /// The cache slot of `request`'s login and certificate.
    /// Slots are not evicted here: two accounts of one app registration
    /// with different certificates would evict each other on every call. An
    /// account's save or deletion forgets its slots instead.
    fn slot(&self, request: &TokenRequest) -> Slot {
        let key = Self::key(request);
        self.cache().entry(key).or_default().clone()
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, Slot>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Forget the token of `request`'s login and certificate: Microsoft
    /// refused it, or its account changed. The next request fetches anew.
    pub fn forget(&self, request: &TokenRequest) {
        let key = Self::key(request);
        self.cache().remove(&key);
    }

    /// Forget every token of the app registration `client_id` in
    /// `tenant_id`, whatever its certificate: for an account whose key
    /// cannot be read to name it.
    pub fn forget_login(&self, tenant_id: &str, client_id: &str) {
        self.cache().retain(|k, _| {
            !(k.tenant.eq_ignore_ascii_case(tenant_id) && k.client.eq_ignore_ascii_case(client_id))
        });
    }

    /// A token for `request` that lasts more than `left`: the cached one,
    /// else a new one. Concurrent requests for one login share one fetch.
    pub async fn token_lasting(
        &self,
        request: &TokenRequest,
        left: Duration,
    ) -> Result<String, TokenError> {
        let slot = self.slot(request);
        let mut cached = slot.lock().await;
        if let Some(c) = cached
            .as_ref()
            .filter(|c| c.lasts(left, Instant::now(), dmarc_store::unix_now()))
        {
            return Ok(c.token.clone());
        }
        let (token, lifetime) = self.fetch(request).await?;
        *cached = Some(Cached::new(token.clone(), lifetime));
        Ok(token)
    }

    /// A new token for `request`, whatever is cached (which it replaces):
    /// the connection test, so a change since the last token shows at once.
    pub async fn fresh_token(&self, request: &TokenRequest) -> Result<String, TokenError> {
        let slot = self.slot(request);
        let mut cached = slot.lock().await;
        let (token, lifetime) = self.fetch(request).await?;
        *cached = Some(Cached::new(token.clone(), lifetime));
        Ok(token)
    }

    /// One client credentials grant for Graph: the token and how long it
    /// lives.
    async fn fetch(&self, request: &TokenRequest) -> Result<(String, Duration), TokenError> {
        let url = self.token_url(&request.tenant_id)?;
        let certificate = &request.certificate;
        let assertion = client_assertion(
            &certificate.private_key_pem,
            &certificate.cert_der,
            &request.client_id,
            url.as_str(),
            dmarc_store::unix_now(),
        )
        .map_err(|e| TokenError::new(TokenErrorKind::BadCredential, e))?;
        let form: [(&str, String); 5] = [
            ("client_id", request.client_id.clone()),
            ("scope", GRAPH_SCOPE.to_owned()),
            ("grant_type", "client_credentials".to_owned()),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_owned(),
            ),
            ("client_assertion", assertion),
        ];
        let unreachable =
            |e: reqwest::Error| TokenError::new(TokenErrorKind::Unreachable, e.to_string());
        let mut response = self
            .http
            .post(url)
            .form(&form)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
            if body.len() + chunk.len() > MAX_ANSWER_BYTES {
                return Err(TokenError::new(
                    TokenErrorKind::BadAnswer,
                    format!("answer larger than {MAX_ANSWER_BYTES} bytes"),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let Ok(answer) = serde_json::from_slice::<Answer>(&body) else {
            return Err(TokenError::new(
                TokenErrorKind::BadAnswer,
                format!("HTTP {status}, no JSON answer"),
            ));
        };
        if let Some(token) = answer.access_token.filter(|_| status.is_success()) {
            let secs = match &answer.expires_in {
                Some(serde_json::Value::Number(n)) => n.as_u64(),
                Some(serde_json::Value::String(s)) => s.parse().ok(),
                _ => None,
            }
            .unwrap_or(3600)
            .max(60);
            return Ok((token, Duration::from_secs(secs)));
        }
        let description = answer.error_description.unwrap_or_default();
        let first_line = description.lines().next().unwrap_or_default().trim();
        let code = answer
            .error_codes
            .first()
            .copied()
            .or_else(|| code_in(first_line));
        let kind = code.map_or(TokenErrorKind::Refused, TokenErrorKind::of_code);
        let error = answer.error.unwrap_or_else(|| format!("HTTP {status}"));
        let detail = match (code, first_line) {
            (_, "") => match code {
                Some(code) => format!("{error}: AADSTS{code}"),
                None => error,
            },
            (Some(code), line) if code_in(line).is_none() => {
                format!("{error}: AADSTS{code}: {line}")
            }
            (_, line) => format!("{error}: {line}"),
        };
        Err(TokenError::new(kind, detail))
    }
}

/// One account's Graph token, for a mailbox pass or a send: the cached
/// one, else a new one; a token Graph refused is forgotten.
pub struct GraphAccountToken {
    source: Arc<TokenSource>,
    request: TokenRequest,
}

impl GraphAccountToken {
    pub fn new(source: Arc<TokenSource>, request: TokenRequest) -> Self {
        Self { source, request }
    }
}

impl GraphToken for GraphAccountToken {
    fn token(&self) -> BoxFuture<'_, Result<String, TokenFailure>> {
        Box::pin(async move {
            self.source
                .token_lasting(&self.request, REFRESH_BEFORE)
                .await
                .map_err(|e| TokenFailure {
                    message: e.message,
                    detail: e.detail,
                    docs: e.docs,
                })
        })
    }

    fn refused(&self) {
        self.source.forget(&self.request);
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
pub(crate) mod tests;
