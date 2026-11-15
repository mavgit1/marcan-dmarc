//! The IMAP [`Source`]: one logged-in session, one folder.
//!
//! 1. `EXAMINE` opens the folder read-only; its UIDVALIDITY is the folder
//!    generation.
//! 2. `UID SEARCH` for UIDs above the cursor, then `UID FETCH (UID
//!    RFC822.SIZE)` in chunks of [`Limits::size_chunk`] to learn each
//!    message's size without downloading it.
//! 3. A batch's bodies are streamed with `UID FETCH (UID BODY.PEEK[])` and
//!    handed to the pass one message at a time; the UIDs the server
//!    returned nothing for are searched again (`UID SEARCH UID <set>`) to
//!    tell a message deleted meanwhile from one the server did not answer.

use crate::backend::Cursor;
use crate::batch::Limits;
use crate::imap::config::ImapConfig;
use crate::pass::{timed, Failure, FailureKind, PassResult};
use crate::source::{MessageId, Resume, Sink, Source};
use futures::TryStreamExt;
use std::collections::{HashMap, HashSet};
use std::fmt;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info, warn};

/// A message's UID in one UIDVALIDITY generation of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Uid(pub u32);

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl MessageId for Uid {
    fn key(&self) -> String {
        self.0.to_string()
    }

    fn label(&self) -> String {
        format!("UID {}", self.0)
    }

    fn span(first: &Self, last: &Self) -> String {
        format!("UIDs {}–{}", first.0, last.0)
    }

    fn list(ids: &[Self]) -> String {
        format!("UIDs {}", compact_uid_set(&numbers(ids)))
    }

    fn mark(&self) -> Option<u32> {
        Some(self.0)
    }
}

fn numbers(ids: &[Uid]) -> Vec<u32> {
    ids.iter().map(|uid| uid.0).collect()
}

/// IMAP sequence-set for `uids` (ascending): contiguous runs become `a:b`.
///
/// `[1, 2, 3, 5, 7, 8]` → `"1:3,5,7:8"`. Keeps commands short for large
/// mailboxes whose UIDs are mostly contiguous.
pub(crate) fn compact_uid_set(uids: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < uids.len() {
        let start = uids[i];
        let mut end = start;
        while i + 1 < uids.len() && end.checked_add(1) == Some(uids[i + 1]) {
            i += 1;
            end = uids[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
        i += 1;
    }
    parts.join(",")
}

/// One pass's logged-in session on one folder.
pub(crate) struct ImapSource<T>
where
    T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
{
    session: async_imap::Session<T>,
    folder: String,
    limits: Limits,
    /// The folder's UIDVALIDITY once EXAMINE answered.
    uid_validity: u32,
    /// Every UID up to this one was handled before the pass.
    last_uid: u32,
    /// The highest UID the listing returned (`last_uid` before it).
    listed: u32,
}

impl<T> ImapSource<T>
where
    T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
{
    pub(crate) fn new(session: async_imap::Session<T>, cfg: &ImapConfig, limits: Limits) -> Self {
        Self {
            session,
            folder: cfg.folder.clone(),
            limits,
            uid_validity: 0,
            last_uid: 0,
            listed: 0,
        }
    }

    fn cursor_at(&self, last_uid: u32) -> Cursor {
        Cursor {
            generation: self.uid_validity.to_string(),
            position: last_uid.to_string(),
        }
    }
}

impl<T> Source for ImapSource<T>
where
    T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
{
    type Id = Uid;

    const NAME: &'static str = "IMAP";

    const MISSING_BODY: &'static str = "UID FETCH response missing body";

    async fn open(&mut self) -> PassResult<String> {
        // EXAMINE opens the folder read-only: Marcan never changes the mailbox.
        let examine = format!("EXAMINE {}", self.folder);
        let mailbox = timed(
            self.limits.command_timeout,
            &examine,
            self.session.examine(&self.folder),
        )
        .await?;
        let uid_validity = mailbox.uid_validity.ok_or_else(|| {
            Failure::message(
                FailureKind::Server,
                format!(
                    "EXAMINE {} did not return UIDVALIDITY; cannot track UID cursor",
                    self.folder
                ),
            )
        })?;
        self.uid_validity = uid_validity;
        Ok(uid_validity.to_string())
    }

    fn resume(&mut self, stored: Option<&Cursor>) -> Resume {
        let folder = &self.folder;
        let uid_validity = self.uid_validity;
        let last_uid = match stored {
            Some(c) if c.generation == uid_validity.to_string() => {
                let last_uid = c.position.parse().unwrap_or_else(|_| {
                    warn!(folder = %folder, position = %c.position, "stored IMAP cursor is no UID; starting from UID 0");
                    0
                });
                info!(folder = %folder, uid_validity, last_uid, "loaded IMAP cursor");
                last_uid
            }
            Some(c) => {
                warn!(
                    folder = %folder,
                    stored_validity = %c.generation,
                    uid_validity,
                    "UIDVALIDITY changed; resetting cursor to 0"
                );
                0
            }
            None => {
                info!(folder = %folder, uid_validity, "no stored IMAP cursor; starting from UID 0");
                0
            }
        };
        self.last_uid = last_uid;
        self.listed = last_uid;
        Resume {
            start: (last_uid > 0).then_some(last_uid),
        }
    }

    async fn list(&mut self) -> PassResult<Vec<Uid>> {
        let last_uid = self.last_uid;
        // UID SEARCH UID <from>:* — note IMAP's `n:*` quirk can return the highest
        // existing UID even when it is < n, so we always filter to uid > last_uid.
        let search_from = last_uid.saturating_add(1).max(1);
        let search_query = format!("UID {search_from}:*");
        let found = timed(
            self.limits.command_timeout,
            &format!("IMAP UID SEARCH {search_query}"),
            self.session.uid_search(&search_query),
        )
        .await?;
        let mut uids: Vec<u32> = found.into_iter().filter(|uid| *uid > last_uid).collect();
        uids.sort_unstable();
        if uids.is_empty() {
            info!(last_uid, "no new messages above cursor");
        }
        self.listed = uids.last().copied().unwrap_or(last_uid).max(last_uid);
        Ok(uids.into_iter().map(Uid).collect())
    }

    /// `RFC822.SIZE` of every UID in `new`, in chunks of
    /// [`Limits::size_chunk`]. UIDs the server does not answer for (deleted
    /// meanwhile) are simply absent.
    async fn sizes(&mut self, new: &[Uid]) -> PassResult<HashMap<Uid, u64>> {
        info!(
            count = new.len(),
            last_uid = self.last_uid,
            "reading sizes of new messages"
        );
        let mut sizes = HashMap::with_capacity(new.len());
        for chunk in numbers(new).chunks(self.limits.size_chunk.max(1)) {
            let set = compact_uid_set(chunk);
            let session = &mut self.session;
            let fetches: Vec<_> = timed(
                self.limits.command_timeout,
                "IMAP UID FETCH (RFC822.SIZE)",
                async {
                    session
                        .uid_fetch(&set, "(UID RFC822.SIZE)")
                        .await?
                        .try_collect::<Vec<_>>()
                        .await
                },
            )
            .await?;
            for fetch in &fetches {
                if let (Some(uid), Some(size)) = (fetch.uid, fetch.size) {
                    sizes.insert(Uid(uid), u64::from(size));
                }
            }
        }
        Ok(sizes)
    }

    async fn fetch<K: Sink<Uid>>(&mut self, batch: &[Uid], sink: &mut K) -> PassResult<()> {
        let set = compact_uid_set(&numbers(batch));
        let what = format!("IMAP UID FETCH {set}");
        // UID requested explicitly so Fetch.uid is populated; BODY.PEEK[] avoids \Seen.
        let mut stream = self
            .session
            .uid_fetch(&set, "(UID BODY.PEEK[])")
            .await
            .map_err(|e| Failure::server(&what, e))?;
        while let Some(fetch) = stream
            .try_next()
            .await
            .map_err(|e| Failure::server("reading UID FETCH response", e))?
        {
            // Unsolicited FETCH responses (flag changes by other clients) carry no UID
            // or no body; they are not failures.
            let Some(uid) = fetch.uid else {
                debug!(seq = fetch.message, "ignoring FETCH response without UID");
                continue;
            };
            sink.answer(Uid(uid), fetch.body()).await?;
        }
        Ok(())
    }

    /// The FETCH stream ends at the tagged response whatever its status, so
    /// a missing message is either deleted meanwhile or a server-side
    /// failure: asked again, only UIDs that no longer exist may be skipped.
    async fn still_there(&mut self, missing: &[Uid]) -> PassResult<HashSet<Uid>> {
        let found = timed(
            self.limits.command_timeout,
            "IMAP UID SEARCH (missing messages)",
            self.session
                .uid_search(format!("UID {}", compact_uid_set(&numbers(missing)))),
        )
        .await?;
        Ok(found.into_iter().map(Uid).collect())
    }

    fn cursor_after(&self, last: &Uid) -> Cursor {
        self.cursor_at(last.0)
    }

    fn cursor_listed(&self) -> Cursor {
        self.cursor_at(self.listed)
    }

    async fn close(mut self) {
        match tokio::time::timeout(self.limits.command_timeout, self.session.logout()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "IMAP LOGOUT failed"),
            Err(_) => warn!("IMAP LOGOUT timed out"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_ranges() {
        assert_eq!(compact_uid_set(&[]), "");
        assert_eq!(compact_uid_set(&[7]), "7");
        assert_eq!(compact_uid_set(&[1, 2, 3, 5, 7, 8]), "1:3,5,7:8");
        assert_eq!(compact_uid_set(&[4, 6, 8]), "4,6,8");
        assert_eq!(compact_uid_set(&[1, 2]), "1:2");
        assert_eq!(
            compact_uid_set(&[u32::MAX - 1, u32::MAX]),
            format!("{}:{}", u32::MAX - 1, u32::MAX)
        );
        let many: Vec<u32> = (100..=3_049).collect();
        assert_eq!(compact_uid_set(&many), "100:3049");
    }

    #[test]
    fn uids_are_named_as_before() {
        assert_eq!(Uid(42).key(), "42");
        assert_eq!(Uid(42).label(), "UID 42");
        assert_eq!(Uid::span(&Uid(3), &Uid(4)), "UIDs 3–4");
        assert_eq!(Uid::list(&[Uid(3), Uid(4), Uid(6)]), "UIDs 3:4,6");
        assert_eq!(Uid(42).mark(), Some(42));
    }
}
