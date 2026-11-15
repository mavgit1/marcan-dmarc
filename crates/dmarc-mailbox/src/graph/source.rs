//! The Graph [`Source`]: one pass on one folder of one mailbox.
//!
//! 1. The folder path is looked up to its folder; the folder id is the
//!    generation.
//! 2. The folder's delta query lists the messages created since the stored
//!    position, a round at a time ([`ROUND_MESSAGES`] or [`ROUND_PAGES`]),
//!    page by page with `Prefer: odata.maxpagesize` as large as a batch.
//!    Each page ends with the link to ask next: `@odata.nextLink`, or at
//!    the end `@odata.deltaLink`, which the next pass asks for new mail.
//! 3. Each message is read raw (`/messages/{id}/$value`), one request each,
//!    streamed and cut off at the size cap; one that answers `404` was
//!    deleted meanwhile.
//!
//! The stored position is the link to ask next. Once a batch is handled,
//! it is the link after the page its last message is on, or, when that
//! message is not the page's last, the link of that page and the message's
//! id on the next line: the next pass asks for that page again and skips
//! what came before it. A link Graph no longer accepts (`410 Gone`, or a
//! sync state it lost) restarts the listing from the folder's first
//! message, once per pass: reports are deduplicated by report id, so that
//! only costs the downloads.
//!
//! Graph names no sizes in the listing (none documented for the delta
//! query), so the batches bound the count and the download bounds each
//! body.

use super::client::{Client, Fault, GraphError, Level, Reply};
use super::GraphConfig;
use crate::backend::Cursor;
use crate::batch::Limits;
use crate::pass::{Failure, PassResult};
use crate::source::{MessageId, Resume, Sink, Source};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;
use tracing::{debug, info, warn};

/// A listing round ends once it has this many messages...
pub(crate) const ROUND_MESSAGES: usize = 1_000;

/// ...or this many pages (empty ones included).
pub(crate) const ROUND_PAGES: usize = 100;

/// A message's Graph id, with its place in this pass's listing (the order
/// it is imported in).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GraphId {
    seq: u64,
    pub(crate) id: String,
}

#[cfg(test)]
impl GraphId {
    pub(crate) fn test(id: &str, seq: u64) -> Self {
        Self {
            seq,
            id: id.to_owned(),
        }
    }
}

impl PartialOrd for GraphId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GraphId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.seq
            .cmp(&other.seq)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl fmt::Display for GraphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

impl MessageId for GraphId {
    fn key(&self) -> String {
        self.id.clone()
    }

    fn label(&self) -> String {
        format!("message {}", self.id)
    }

    fn span(first: &Self, last: &Self) -> String {
        if first == last {
            first.label()
        } else {
            format!("messages {} to {}", first.id, last.id)
        }
    }

    fn list(ids: &[Self]) -> String {
        const SHOWN: usize = 3;
        let names: Vec<&str> = ids.iter().take(SHOWN).map(|i| i.id.as_str()).collect();
        let more = ids.len().saturating_sub(SHOWN);
        match more {
            0 => format!("messages {}", names.join(", ")),
            n => format!("messages {} and {n} more", names.join(", ")),
        }
    }

    fn mark(&self) -> Option<u32> {
        None
    }
}

/// Where a listing round starts: a link, and the message on its page
/// handled last, if the page was left in its middle.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Position {
    link: String,
    after: Option<String>,
}

impl Position {
    fn at(link: &str) -> Self {
        Self {
            link: link.to_owned(),
            after: None,
        }
    }

    fn text(&self) -> String {
        match &self.after {
            None => self.link.clone(),
            Some(id) => format!("{}\n{id}", self.link),
        }
    }

    fn parse(text: &str) -> Self {
        match text.split_once('\n') {
            Some((link, id)) if !id.is_empty() => Self {
                link: link.to_owned(),
                after: Some(id.to_owned()),
            },
            Some((link, _)) => Self::at(link),
            None => Self::at(text),
        }
    }
}

/// One page of the current round.
struct Page {
    /// The link it was asked with.
    link: String,
    ids: Vec<GraphId>,
    /// Its `@odata.nextLink`, or its `@odata.deltaLink` at the end.
    next: String,
}

#[derive(Deserialize)]
struct DeltaItem {
    id: String,
    #[serde(rename = "@removed")]
    removed: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct DeltaPage {
    #[serde(default)]
    value: Vec<DeltaItem>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

/// A delta request's outcome.
enum Delta {
    Page(DeltaPage),
    /// Graph no longer accepts the link.
    Expired(GraphError),
}

/// Graph's codes for a delta link it lost the state of.
fn is_lost_state(error: &GraphError) -> bool {
    let code = error.code.to_ascii_lowercase();
    error.status == StatusCode::GONE || code.contains("syncstate") || code == "resyncrequired"
}

/// One pass's requests on one folder.
pub(crate) struct GraphSource {
    client: Client,
    folder: String,
    limits: Limits,
    /// The folder's id once it was looked up.
    folder_id: String,
    /// The delta query from the folder's first message.
    fresh: String,
    /// Where the next round starts.
    start: Position,
    /// The current round.
    pages: Vec<Page>,
    listed_all: bool,
    /// The last [`GraphId::seq`] handed out.
    seq: u64,
    /// The listing restarted in this pass already.
    restarted: bool,
    /// Messages of the current batch that answered `404`.
    gone: HashSet<GraphId>,
}

impl GraphSource {
    pub(crate) fn new(cfg: &GraphConfig, limits: Limits) -> Self {
        Self {
            client: Client::new(cfg),
            folder: cfg.folder.clone(),
            limits,
            folder_id: String::new(),
            fresh: String::new(),
            start: Position::at(""),
            pages: Vec::new(),
            listed_all: false,
            seq: 0,
            restarted: false,
            gone: HashSet::new(),
        }
    }

    fn cursor(&self, position: &Position) -> Cursor {
        Cursor {
            generation: self.folder_id.clone(),
            position: position.text(),
        }
    }

    fn prefer(&self) -> String {
        format!(
            "odata.maxpagesize={}",
            self.limits.batch_max_messages.max(1)
        )
    }

    /// One page of the delta query at `link`.
    async fn delta_page(&mut self, link: &str) -> Result<Delta, Fault> {
        const WHAT: &str = "listing new mail (Graph delta query)";
        let url = match Url::parse(link) {
            Ok(url) if self.client.api().owns(&url) => url,
            _ => return Err(self.client.foreign(WHAT, link)),
        };
        let prefer = self.prefer();
        let limit = self.limits.command_timeout;
        match self.client.get(&url, WHAT, limit, Some(&prefer)).await? {
            Reply::Ok(response) => Ok(Delta::Page(self.client.json(response, WHAT, limit).await?)),
            Reply::Refused(error) if is_lost_state(&error) => Ok(Delta::Expired(error)),
            Reply::Refused(error) => Err(self.client.refusal(&error, WHAT, Level::Item)),
        }
    }

    /// The link a page ends with, checked: exactly one of the two, on the
    /// endpoint. `true` with the delta link.
    fn next_of(&self, page: &DeltaPage) -> Result<(String, bool), Fault> {
        const WHAT: &str = "listing new mail (Graph delta query)";
        let (link, last) = match (&page.next_link, &page.delta_link) {
            (Some(next), None) => (next, false),
            (None, Some(delta)) => (delta, true),
            _ => {
                return Err(Fault::server(
                    WHAT,
                    "the page has neither a next link nor a delta link, or both",
                ))
            }
        };
        match Url::parse(link) {
            Ok(url) if self.client.api().owns(&url) => Ok((link.clone(), last)),
            _ => Err(self.client.foreign(WHAT, link)),
        }
    }

    /// Download `id` and hand it to `sink`; `Ok(false)`: Graph failed to
    /// serve it (not handed over).
    async fn download<K: Sink<GraphId>>(&mut self, id: &GraphId, sink: &mut K) -> PassResult<bool> {
        const WHAT: &str = "Graph message download";
        let mailbox = self.client.mailbox.clone();
        let url = self
            .client
            .api()
            .url(&["users", &mailbox, "messages", &id.id, "$value"], None);
        let cap = self.limits.max_message_bytes;
        let limit = self.limits.batch_timeout(cap);
        let mut response = match self.client.get(&url, WHAT, limit, None).await? {
            Reply::Ok(response) => response,
            Reply::Refused(error) if error.status == StatusCode::NOT_FOUND => {
                debug!(uid = %id, "message answers 404: deleted meanwhile");
                self.gone.insert(id.clone());
                return Ok(true);
            }
            Reply::Refused(error) if error.status.is_server_error() => {
                warn!(uid = %id, detail = %error.detail(), "Graph failed to serve the message");
                return Ok(false);
            }
            Reply::Refused(error) => {
                return Err(self.client.refusal(&error, WHAT, Level::Item).into())
            }
        };
        if let Some(length) = response.content_length().filter(|&n| n > cap) {
            sink.too_large(id.clone(), length).await?;
            return Ok(true);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| {
            Failure::from(Fault::server(
                WHAT,
                self.client.mask(&e.without_url().to_string()),
            ))
        })? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > cap {
                drop(response);
                sink.too_large(id.clone(), body.len() as u64).await?;
                return Ok(true);
            }
        }
        sink.answer(id.clone(), Some(&body)).await?;
        Ok(true)
    }
}

impl Source for GraphSource {
    type Id = GraphId;

    const NAME: &'static str = "Graph";

    const MISSING_BODY: &'static str = "Graph answered without the message's content";

    async fn open(&mut self) -> PassResult<String> {
        let folder = self.client.resolve(&self.folder).await?;
        let Some(folder) = folder else {
            return Err(self.client.no_folder(&self.folder).into());
        };
        let mailbox = self.client.mailbox.clone();
        self.fresh = self
            .client
            .api()
            .url(
                &[
                    "users",
                    &mailbox,
                    "mailFolders",
                    &folder.id,
                    "messages",
                    "delta",
                ],
                Some("changeType=created&$select=id"),
            )
            .to_string();
        self.folder_id = folder.id;
        Ok(self.folder_id.clone())
    }

    fn resume(&mut self, stored: Option<&Cursor>) -> Resume {
        let folder = &self.folder;
        self.start = match stored {
            Some(c) if c.generation == self.folder_id => {
                let position = Position::parse(&c.position);
                let ours = Url::parse(&position.link).is_ok_and(|url| self.client.api().owns(&url));
                if ours {
                    info!(folder = %folder, "loaded Graph cursor");
                    position
                } else {
                    warn!(folder = %folder, "stored Graph cursor is no link to this Graph endpoint; reading the folder from its first message");
                    Position::at(&self.fresh)
                }
            }
            Some(_) => {
                warn!(folder = %folder, "the folder's id changed (deleted and created again?); reading it from its first message");
                Position::at(&self.fresh)
            }
            None => {
                info!(folder = %folder, "no stored Graph cursor; reading the folder from its first message");
                Position::at(&self.fresh)
            }
        };
        Resume { start: None }
    }

    async fn list(&mut self) -> PassResult<Vec<GraphId>> {
        self.pages.clear();
        self.listed_all = false;
        let mut link = self.start.link.clone();
        let mut after = self.start.after.clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut count = 0;
        loop {
            let page = match self.delta_page(&link).await? {
                Delta::Page(page) => page,
                Delta::Expired(error) => {
                    if self.restarted || link == self.fresh {
                        return Err(Fault::server(
                            "listing new mail (Graph delta query)",
                            error.detail(),
                        )
                        .into());
                    }
                    warn!(
                        folder = %self.folder,
                        detail = %error.detail(),
                        "Graph no longer accepts the stored link; listing the folder from its first message again"
                    );
                    self.restarted = true;
                    self.pages.clear();
                    seen.clear();
                    count = 0;
                    link.clone_from(&self.fresh);
                    after = None;
                    continue;
                }
            };
            let (next, last) = self.next_of(&page)?;
            let mut items = page.value;
            if let Some(handled) = after.take() {
                // The rest of a page left in its middle; a page that changed
                // meanwhile is read whole.
                if let Some(at) = items.iter().position(|m| m.id == handled) {
                    items.drain(..=at);
                }
            }
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                if item.removed.is_some() || !seen.insert(item.id.clone()) {
                    continue;
                }
                self.seq += 1;
                ids.push(GraphId {
                    seq: self.seq,
                    id: item.id,
                });
            }
            count += ids.len();
            self.pages.push(Page {
                link: link.clone(),
                ids,
                next: next.clone(),
            });
            if last {
                self.listed_all = true;
                break;
            }
            if count >= ROUND_MESSAGES || self.pages.len() >= ROUND_PAGES {
                break;
            }
            link = next;
        }
        if let Some(page) = self.pages.last() {
            self.start = Position::at(&page.next);
        }
        info!(
            folder = %self.folder,
            messages = count,
            pages = self.pages.len(),
            complete = self.listed_all,
            "listed new mail"
        );
        Ok(self
            .pages
            .iter()
            .flat_map(|p| p.ids.iter().cloned())
            .collect())
    }

    fn listed_all(&self) -> bool {
        self.listed_all
    }

    async fn sizes(&mut self, _new: &[GraphId]) -> PassResult<HashMap<GraphId, u64>> {
        Ok(HashMap::new())
    }

    /// One request per message, in order. Messages Graph failed to serve
    /// (a server error) are not handed over, so they count as attempts
    /// like IMAP's unanswered ones; when it failed every one of the batch,
    /// the pass fails instead (Graph is down, not the messages bad).
    async fn fetch<K: Sink<GraphId>>(&mut self, batch: &[GraphId], sink: &mut K) -> PassResult<()> {
        self.gone.clear();
        let mut failed = 0;
        for id in batch {
            if !self.download(id, sink).await? {
                failed += 1;
            }
        }
        if failed > 0 && failed == batch.len() {
            return Err(Fault::server(
                "Graph message download",
                format!("Graph failed to serve all {failed} message(s) of the batch"),
            )
            .into());
        }
        Ok(())
    }

    async fn still_there(&mut self, missing: &[GraphId]) -> PassResult<HashSet<GraphId>> {
        Ok(missing
            .iter()
            .filter(|id| !self.gone.contains(id))
            .cloned()
            .collect())
    }

    fn cursor_after(&self, last: &GraphId) -> Cursor {
        let page = self.pages.iter().find(|p| p.ids.contains(last));
        let position = match page {
            Some(page) if page.ids.last() == Some(last) => Position::at(&page.next),
            Some(page) => Position {
                link: page.link.clone(),
                after: Some(last.id.clone()),
            },
            // Not listed in this round (cannot happen): the round again.
            None => self
                .pages
                .first()
                .map_or_else(|| self.start.clone(), |p| Position::at(&p.link)),
        };
        self.cursor(&position)
    }

    fn cursor_listed(&self) -> Cursor {
        self.cursor(&self.start)
    }

    async fn close(self) {}
}
