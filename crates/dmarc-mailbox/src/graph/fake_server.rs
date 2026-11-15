//! A fake Microsoft Graph on 127.0.0.1 for tests: one mailbox with folders
//! (nested, the inbox well-known), each folder's messages with the delta
//! query's paging and links, the raw messages (`$value`), a bearer check,
//! Graph's error shape, and switches for the failures a pass must survive:
//! a refused token (`401`), another mailbox (`403`), an unknown user
//! (`404`), expired links (`410` or a lost sync state), throttling (`429`
//! with `Retry-After`), a message deleted or failing while it is read,
//! links to another host. It takes mail too (`sendMail` in MIME form:
//! base64 checked and decoded, kept with the mailbox it went out as),
//! refusing a `From` other than the mailbox's own (`403`
//! `ErrorSendAsDenied`) or with any refusal it is told. Every request is
//! logged.
//!
//! Other crates reach it with the `test-support` feature, as
//! `dmarc_mailbox::graph_fake`.

use crate::imap::fake_server::FakeMessage;
use axum::body::{Body, Bytes};
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use reqwest::Url;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// The address [`FakeGraph`] answers `404` for: no such user.
pub const UNKNOWN_USER: &str = "nobody@example.com";

/// How expired links are answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// `410 Gone`.
    Gone,
    /// `400` with the code `SyncStateNotFound`.
    LostState,
}

/// One logged request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `GET`, `POST`.
    pub method: String,
    /// Path and query, as received.
    pub target: String,
    /// The `Authorization` header.
    pub auth: String,
    /// The `Prefer` header.
    pub prefer: String,
    /// The `Content-Type` header.
    pub content_type: String,
}

/// One message `sendMail` took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMail {
    /// The user the request named.
    pub user: String,
    /// The message, decoded from the request's base64.
    pub mime: Vec<u8>,
}

struct Mail {
    id: String,
    raw: Vec<u8>,
    deleted: bool,
}

struct Folder {
    id: String,
    name: String,
    parent: Option<String>,
    inbox: bool,
    mails: Vec<Mail>,
}

#[derive(Default)]
struct Inner {
    /// The fake's own base URL, which its links name.
    base: String,
    address: String,
    folders: Vec<Folder>,
    /// Bearer tokens accepted; empty: any.
    tokens: Vec<String>,
    /// Links carry it; links of an older one are expired.
    link_epoch: u32,
    expiry: Option<Expiry>,
    throttle: u32,
    retry_after: u32,
    forbid: bool,
    foreign_links: bool,
    /// Messages that answer `404` once they are read.
    vanish: HashSet<String>,
    /// Messages that answer `500`.
    failing: HashSet<String>,
    /// A `403` or `500` echoes the `Authorization` header in its message.
    echo_auth: bool,
    /// Delta pages of this size, whatever `Prefer` asks.
    page_size: Option<usize>,
    folder_page: usize,
    /// What `sendMail` took.
    sent: Vec<SentMail>,
    /// Every `sendMail` answers this (status, code, message) instead.
    send_refusal: Option<(StatusCode, String, String)>,
    serial: u64,
    next_mail: u64,
    next_folder: u64,
    log: Vec<Request>,
}

/// The running fake.
#[derive(Clone)]
pub struct FakeGraph {
    /// Its base URL (`http://127.0.0.1:port`), the `GraphApi`'s base.
    pub base: Url,
    inner: Arc<Mutex<Inner>>,
}

fn graph_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({"error": {"code": code, "message": message}});
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn json(value: &serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

impl FakeGraph {
    /// A fake with the mailbox `address`, whose only folder is its inbox
    /// (display name `Inbox`), on a free local port.
    pub async fn start(address: &str) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            address: address.to_owned(),
            retry_after: 1,
            folder_page: 100,
            ..Inner::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the fake Graph");
        let port = listener.local_addr().expect("local address").port();
        let base = format!("http://127.0.0.1:{port}");
        inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .base
            .clone_from(&base);
        let state = Arc::clone(&inner);
        let app = Router::new()
            .fallback(
                move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
                    let state = Arc::clone(&state);
                    async move { handle(&state, &method, &uri, &headers, &body) }
                },
            )
            .layer(DefaultBodyLimit::max(16 * 1024 * 1024));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let fake = Self {
            base: Url::parse(&base).expect("the fake's URL"),
            inner,
        };
        fake.add_folder(None, "Inbox");
        fake.state().folders[0].inbox = true;
        fake
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The inbox's id.
    pub fn inbox(&self) -> String {
        let s = self.state();
        s.folders
            .iter()
            .find(|f| f.inbox)
            .map(|f| f.id.clone())
            .unwrap_or_default()
    }

    /// A new folder `name` under `parent` (`None`: at the top); its id.
    pub fn add_folder(&self, parent: Option<&str>, name: &str) -> String {
        let mut s = self.state();
        s.next_folder += 1;
        let id = format!("AAMkFolder{}=", s.next_folder);
        s.folders.push(Folder {
            id: id.clone(),
            name: name.to_owned(),
            parent: parent.map(str::to_owned),
            inbox: false,
            mails: Vec::new(),
        });
        id
    }

    /// Folder `id` deleted and created again under the same name: a new
    /// id, no messages. Returns the new id.
    pub fn recreate(&self, id: &str) -> String {
        let mut s = self.state();
        s.next_folder += 1;
        let new = format!("AAMkFolder{}=", s.next_folder);
        if let Some(folder) = s.folders.iter_mut().find(|f| f.id == id) {
            folder.id.clone_from(&new);
            folder.mails.clear();
        }
        for folder in &mut s.folders {
            if folder.parent.as_deref() == Some(id) {
                folder.parent = Some(new.clone());
            }
        }
        new
    }

    /// Deliver raw mail `raw` to folder `folder`; the message's id.
    pub fn deliver(&self, folder: &str, raw: &[u8]) -> String {
        let mut s = self.state();
        s.next_mail += 1;
        let id = format!("AAMkMessage{}-x_y=", s.next_mail);
        if let Some(f) = s.folders.iter_mut().find(|f| f.id == folder) {
            f.mails.push(Mail {
                id: id.clone(),
                raw: raw.to_vec(),
                deleted: false,
            });
        }
        id
    }

    /// Deliver [`FakeMessage::report`] `n` for each of `numbers` to
    /// `folder`; their ids.
    pub fn deliver_reports(&self, folder: &str, numbers: &[u32]) -> Vec<String> {
        numbers
            .iter()
            .map(|&n| self.deliver(folder, FakeMessage::report(n).raw()))
            .collect()
    }

    /// Message `id` is deleted (the next delta pages report it removed).
    pub fn delete(&self, id: &str) {
        let mut s = self.state();
        for f in &mut s.folders {
            for m in f.mails.iter_mut().filter(|m| m.id == id) {
                m.deleted = true;
            }
        }
    }

    /// Only these bearer tokens are accepted (`401` for others).
    pub fn accept_tokens(&self, tokens: &[&str]) {
        self.state().tokens = tokens.iter().map(|t| (*t).to_owned()).collect();
    }

    /// Every link handed out so far expires, answered as `how`.
    pub fn expire_links(&self, how: Expiry) {
        let mut s = self.state();
        s.link_epoch += 1;
        s.expiry = Some(how);
    }

    /// The next `n` requests answer `429` with `Retry-After: secs`.
    pub fn throttle(&self, n: u32, secs: u32) {
        let mut s = self.state();
        s.throttle = n;
        s.retry_after = secs;
    }

    /// Every request for the mailbox answers `403`.
    pub fn forbid(&self, forbid: bool) {
        self.state().forbid = forbid;
    }

    /// Next and delta links point at another host.
    pub fn foreign_links(&self, foreign: bool) {
        self.state().foreign_links = foreign;
    }

    /// Message `id` is deleted the moment it is read: `404`.
    pub fn vanish_on_read(&self, id: &str) {
        self.state().vanish.insert(id.to_owned());
    }

    /// Message `id` answers `500` while `failing`.
    pub fn fail_message(&self, id: &str, failing: bool) {
        let mut s = self.state();
        if failing {
            s.failing.insert(id.to_owned());
        } else {
            s.failing.remove(id);
        }
    }

    /// A `403` or `500` echoes the request's `Authorization` header.
    pub fn echo_auth(&self, echo: bool) {
        self.state().echo_auth = echo;
    }

    /// Delta pages of `n` messages, whatever `Prefer` asks (Graph may
    /// answer with other sizes than asked).
    pub fn page_size(&self, n: usize) {
        self.state().page_size = Some(n.max(1));
    }

    /// Folder lists come in pages of `n`.
    pub fn folder_page(&self, n: usize) {
        self.state().folder_page = n.max(1);
    }

    /// Every `sendMail` answers `status` with Graph's error `code` and
    /// `message`; `None`: it takes the mail again.
    pub fn refuse_send(&self, refusal: Option<(StatusCode, &str, &str)>) {
        self.state().send_refusal =
            refusal.map(|(status, code, message)| (status, code.to_owned(), message.to_owned()));
    }

    /// The messages `sendMail` took, oldest first.
    pub fn sent(&self) -> Vec<SentMail> {
        self.state().sent.clone()
    }

    /// Every request so far.
    pub fn requests(&self) -> Vec<Request> {
        self.state().log.clone()
    }

    /// The ids whose `$value` was asked for, in order.
    pub fn downloads(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter_map(|r| {
                let path = r.target.split('?').next()?;
                let rest = path
                    .strip_suffix("/$value")
                    .or_else(|| path.strip_suffix("/%24value"))?;
                rest.rsplit('/').next().map(str::to_owned)
            })
            .collect()
    }

    /// The delta requests (path and query), in order.
    pub fn delta_requests(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|r| r.target)
            .filter(|t| t.contains("/messages/delta"))
            .collect()
    }

    /// Forget the logged requests.
    pub fn clear_log(&self) {
        self.state().log.clear();
    }
}

fn query_of(uri: &Uri) -> HashMap<String, String> {
    uri.query()
        .map(|q| {
            url_pairs(q)
                .into_iter()
                .collect::<HashMap<String, String>>()
        })
        .unwrap_or_default()
}

fn url_pairs(query: &str) -> Vec<(String, String)> {
    let parsed = Url::parse(&format!("http://x/?{query}")).expect("a query");
    parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn folder_json(s: &Inner, f: &Folder) -> serde_json::Value {
    let children = s
        .folders
        .iter()
        .filter(|c| c.parent.as_deref() == Some(f.id.as_str()))
        .count();
    serde_json::json!({
        "id": f.id,
        "displayName": f.name,
        "childFolderCount": children,
        "totalItemCount": f.mails.iter().filter(|m| !m.deleted).count(),
    })
}

fn handle(
    state: &Arc<Mutex<Inner>>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let auth = header("authorization");
    s.log.push(Request {
        method: method.to_string(),
        target: uri
            .path_and_query()
            .map(ToString::to_string)
            .unwrap_or_default(),
        auth: auth.clone(),
        prefer: header("prefer"),
        content_type: header("content-type"),
    });
    if s.throttle > 0 {
        s.throttle -= 1;
        let mut response = graph_error(
            StatusCode::TOO_MANY_REQUESTS,
            "ApplicationThrottled",
            "Application is over its MailboxConcurrency limit.",
        );
        response.headers_mut().insert(
            header::RETRY_AFTER,
            s.retry_after.to_string().parse().expect("a header value"),
        );
        return response;
    }
    let token = auth.strip_prefix("Bearer ").unwrap_or_default();
    if token.is_empty() || (!s.tokens.is_empty() && !s.tokens.iter().any(|t| t == token)) {
        return graph_error(
            StatusCode::UNAUTHORIZED,
            "InvalidAuthenticationToken",
            "Access token has expired or is not yet valid.",
        );
    }
    let path = percent_decode(uri.path());
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let ["v1.0", "users", user, rest @ ..] = segments.as_slice() else {
        return graph_error(
            StatusCode::BAD_REQUEST,
            "BadRequest",
            "Unsupported request.",
        );
    };
    if user.eq_ignore_ascii_case(UNKNOWN_USER) {
        return graph_error(
            StatusCode::NOT_FOUND,
            "ErrorInvalidUser",
            &format!("The requested user '{user}' is invalid."),
        );
    }
    if s.forbid || !user.eq_ignore_ascii_case(&s.address) {
        let message = if s.echo_auth {
            format!("Access is denied for {auth}.")
        } else {
            "Access is denied. Check credentials and try again.".to_owned()
        };
        return graph_error(StatusCode::FORBIDDEN, "ErrorAccessDenied", &message);
    }
    if let ["sendMail"] = rest {
        return if *method == Method::POST {
            send_mail(&mut s, user, &header("content-type"), body)
        } else {
            graph_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "ErrorInvalidRequest",
                "The OData request is not supported.",
            )
        };
    }
    if *method != Method::GET {
        return graph_error(
            StatusCode::BAD_REQUEST,
            "BadRequest",
            "Unsupported request.",
        );
    }
    let query = query_of(uri);
    let prefer = header("prefer");
    match rest {
        ["mailFolders"] => list_folders(&s, None, &query, uri),
        ["mailFolders", id] => {
            let id = resolve_id(&s, id);
            match s.folders.iter().find(|f| Some(&f.id) == id.as_ref()) {
                Some(f) => json(&folder_json(&s, f)),
                None => not_found(),
            }
        }
        ["mailFolders", id, "childFolders"] => match resolve_id(&s, id) {
            Some(id) => list_folders(&s, Some(&id), &query, uri),
            None => not_found(),
        },
        ["mailFolders", id, "messages", "delta"] => delta(&mut s, id, &query, &prefer),
        ["messages", id, "$value"] => {
            let id = (*id).to_owned();
            if s.failing.contains(&id) {
                let message = if s.echo_auth {
                    format!("Internal error for {auth}")
                } else {
                    "Internal error.".to_owned()
                };
                return graph_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ErrorInternalServerError",
                    &message,
                );
            }
            let vanish = s.vanish.remove(&id);
            let found = s
                .folders
                .iter_mut()
                .flat_map(|f| f.mails.iter_mut())
                .find(|m| m.id == id && !m.deleted);
            match found {
                Some(m) if vanish => {
                    m.deleted = true;
                    not_found()
                }
                Some(m) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "message/rfc822")],
                    Body::from(m.raw.clone()),
                )
                    .into_response(),
                None => not_found(),
            }
        }
        _ => graph_error(
            StatusCode::BAD_REQUEST,
            "BadRequest",
            "Unsupported request.",
        ),
    }
}

/// `sendMail` in MIME form as `user`: the body base64 (`text/plain`), the
/// `From` the mailbox's own; `202` with no body.
fn send_mail(s: &mut Inner, user: &str, content_type: &str, body: &[u8]) -> Response {
    use base64::Engine;
    if let Some((status, code, message)) = &s.send_refusal {
        return graph_error(*status, code, message);
    }
    if !content_type.starts_with("text/plain") {
        return graph_error(
            StatusCode::BAD_REQUEST,
            "ErrorInvalidRequest",
            "Only JSON and MIME are taken; this request is neither.",
        );
    }
    let text: Vec<u8> = body
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let Ok(mime) = base64::engine::general_purpose::STANDARD.decode(&text) else {
        return graph_error(
            StatusCode::BAD_REQUEST,
            "ErrorMimeContentInvalidBase64String",
            "Invalid base64 string for MIME content.",
        );
    };
    let from = mail_parser::MessageParser::default()
        .parse(&mime)
        .and_then(|m| {
            m.from()
                .and_then(|f| f.first())
                .and_then(|a| a.address())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if !from.eq_ignore_ascii_case(user) {
        return graph_error(
            StatusCode::FORBIDDEN,
            "ErrorSendAsDenied",
            "The user account which was used to submit this request does not have the right to send mail on behalf of the specified sending account. Cannot submit message.",
        );
    }
    s.sent.push(SentMail {
        user: user.to_owned(),
        mime,
    });
    StatusCode::ACCEPTED.into_response()
}

fn not_found() -> Response {
    graph_error(
        StatusCode::NOT_FOUND,
        "ErrorItemNotFound",
        "The specified object was not found in the store.",
    )
}

/// `path` with its `%XX` escapes decoded (`%24value`, `%40`).
fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `id`, or the inbox's id for the well-known name.
fn resolve_id(s: &Inner, id: &str) -> Option<String> {
    if id.eq_ignore_ascii_case("inbox") {
        return s.folders.iter().find(|f| f.inbox).map(|f| f.id.clone());
    }
    s.folders.iter().find(|f| f.id == id).map(|f| f.id.clone())
}

fn list_folders(
    s: &Inner,
    parent: Option<&str>,
    query: &HashMap<String, String>,
    uri: &Uri,
) -> Response {
    let all: Vec<&Folder> = s
        .folders
        .iter()
        .filter(|f| f.parent.as_deref() == parent)
        .collect();
    let skip: usize = query.get("$skip").and_then(|v| v.parse().ok()).unwrap_or(0);
    let page: Vec<serde_json::Value> = all
        .iter()
        .skip(skip)
        .take(s.folder_page)
        .map(|f| folder_json(s, f))
        .collect();
    let mut body = serde_json::json!({ "value": page });
    if skip + s.folder_page < all.len() {
        body["@odata.nextLink"] = serde_json::Value::String(format!(
            "{}{}?$skip={}",
            link_base(s),
            uri.path(),
            skip + s.folder_page
        ));
    }
    json(&body)
}

fn link_base(s: &Inner) -> String {
    if s.foreign_links {
        "http://graph.evil.example".to_owned()
    } else {
        s.base.clone()
    }
}

/// One delta page of folder `id`: tokens are `epoch.index.serial`.
fn delta(s: &mut Inner, id: &str, query: &HashMap<String, String>, prefer: &str) -> Response {
    let token = query
        .get("$skiptoken")
        .or_else(|| query.get("$deltatoken"))
        .cloned();
    let start = match token {
        None => 0,
        Some(token) => {
            let mut parts = token.split('.');
            let epoch: Option<u32> = parts.next().and_then(|p| p.parse().ok());
            let index: Option<usize> = parts.next().and_then(|p| p.parse().ok());
            match (epoch, index) {
                (Some(e), Some(i)) if e == s.link_epoch => i,
                _ => {
                    return match s.expiry.unwrap_or(Expiry::Gone) {
                        Expiry::Gone => graph_error(
                            StatusCode::GONE,
                            "syncStateNotFound",
                            "The sync state is not found.",
                        ),
                        Expiry::LostState => graph_error(
                            StatusCode::BAD_REQUEST,
                            "SyncStateNotFound",
                            "The sync state generation is not found.",
                        ),
                    }
                }
            }
        }
    };
    let asked: usize = prefer
        .strip_prefix("odata.maxpagesize=")
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(10);
    let size = s.page_size.unwrap_or(asked).max(1);
    let Some(folder) = s.folders.iter().find(|f| f.id == id) else {
        return not_found();
    };
    let mut value = Vec::new();
    let mut taken = 0;
    let mut end = start;
    for (index, mail) in folder.mails.iter().enumerate().skip(start) {
        if taken == size {
            break;
        }
        end = index + 1;
        if mail.deleted {
            value.push(serde_json::json!({"id": mail.id, "@removed": {"reason": "deleted"}}));
        } else {
            value.push(serde_json::json!({"id": mail.id}));
            taken += 1;
        }
    }
    let more = folder.mails.iter().skip(end).any(|m| !m.deleted);
    let len = folder.mails.len();
    s.serial += 1;
    let base = link_base(s);
    let path = format!("/v1.0/users/{}/mailFolders/{id}/messages/delta", s.address);
    let mut body = serde_json::json!({ "value": value });
    if more {
        body["@odata.nextLink"] = serde_json::Value::String(format!(
            "{base}{path}?$skiptoken={}.{end}.{}",
            s.link_epoch, s.serial
        ));
    } else {
        body["@odata.deltaLink"] = serde_json::Value::String(format!(
            "{base}{path}?$deltatoken={}.{len}.{}",
            s.link_epoch, s.serial
        ));
    }
    json(&body)
}
