//! The seam between a pass and the protocol it reads a folder with.
//!
//! The pass ([`crate::pass`]) decides everything a pass does: which
//! messages are new, in which batches they are read, what is skipped,
//! retried or quarantined, and when the cursor moves. A [`Source`] answers
//! two things only, in its protocol: what the folder holds since a stored
//! cursor (with each message's size, when it knows it), and the raw mail of
//! a batch of those messages. It also says where the folder stands once a
//! batch was read ([`Source::cursor_after`]) and once everything listed so
//! far was ([`Source::cursor_listed`]), since only it can spell its
//! position.
//!
//! A source may list a large folder in rounds ([`Source::listed_all`]):
//! the pass imports each round and stores the cursor before it lists the
//! next, so neither memory nor a failure late in the listing grows with
//! the folder.
//!
//! The sources are IMAP ([`crate::imap`]) and Microsoft Graph
//! ([`crate::graph`]).

use crate::backend::Cursor;
use crate::pass::PassResult;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::hash::Hash;

/// A message's id within one folder generation. Its order is the order the
/// folder lists the messages in, which is the order they are imported in.
pub(crate) trait MessageId:
    Clone + Ord + Hash + fmt::Debug + fmt::Display + Send + Sync + 'static
{
    /// How the backend stores it ([`MessageRef::message_id`](crate::MessageRef::message_id)).
    fn key(&self) -> String;

    /// How an error names it: `UID 42`.
    fn label(&self) -> String;

    /// How an error names the messages from `first` to `last`: `UIDs 3–5`.
    fn span(first: &Self, last: &Self) -> String;

    /// How an error names several: `UIDs 3:5,7`.
    fn list(ids: &[Self]) -> String;

    /// Its number in a [`FetchSummary`](crate::FetchSummary)'s
    /// `start_position` and `end_position`; `None`: it has none.
    fn mark(&self) -> Option<u32>;
}

/// Where a pass starts, as [`Source::resume`] decides it from the stored
/// cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resume {
    /// The [`FetchSummary::start_position`](crate::FetchSummary::start_position)
    /// of the pass.
    pub start: Option<u32>,
}

/// One way of reading a folder: one pass's session with its server. Every
/// I/O method fails with what the pass records (a timeout, a server error);
/// the pass decides what becomes of it.
pub(crate) trait Source: Send {
    type Id: MessageId;

    /// Its name in errors and the log: `IMAP`.
    const NAME: &'static str;

    /// The quarantine reason of a message the server answered for without
    /// its mail.
    const MISSING_BODY: &'static str;

    /// Open the folder, read-only, and return its generation
    /// ([`Cursor::generation`]).
    fn open(&mut self) -> impl Future<Output = PassResult<String>> + Send;

    /// Where the pass starts, given the folder's stored cursor (of any
    /// generation; `None`: never read). A stored cursor of another
    /// generation is void: the pass starts from the folder's first message.
    fn resume(&mut self, stored: Option<&Cursor>) -> Resume;

    /// The messages new since [`Self::resume`]'s position, or since the
    /// previous round, in import order: all of them, or one round's worth
    /// ([`Self::listed_all`]).
    fn list(&mut self) -> impl Future<Output = PassResult<Vec<Self::Id>>> + Send;

    /// Whether the last [`Self::list`] reached the end of the folder; when
    /// not, the pass imports what it listed and lists again.
    fn listed_all(&self) -> bool {
        true
    }

    /// The announced sizes of `new` in bytes; a message the server gives no
    /// size for is absent (the batches then bound only the count, and the
    /// download bounds each body: [`Sink::too_large`]).
    fn sizes(
        &mut self,
        new: &[Self::Id],
    ) -> impl Future<Output = PassResult<HashMap<Self::Id, u64>>> + Send;

    /// Download `batch`, handing every answer to `sink` as it arrives, at
    /// most one batch in flight. A message the server does not answer for
    /// is simply not handed over; the pass asks [`Self::still_there`] about
    /// those. An error of `sink` ends the download with that error.
    fn fetch<K: Sink<Self::Id>>(
        &mut self,
        batch: &[Self::Id],
        sink: &mut K,
    ) -> impl Future<Output = PassResult<()>> + Send;

    /// Which of `missing` (asked for, not handed over) still exist in the
    /// folder: the others were deleted meanwhile and may be skipped.
    fn still_there(
        &mut self,
        missing: &[Self::Id],
    ) -> impl Future<Output = PassResult<HashSet<Self::Id>>> + Send;

    /// The cursor once every message up to and including `last` (a batch's
    /// last message) was handled.
    fn cursor_after(&self, last: &Self::Id) -> Cursor;

    /// The cursor once every message listed so far was handled; before any
    /// is, where [`Self::resume`] placed the pass. Stored when it differs
    /// from the stored one: a first pass, a changed generation, or a
    /// listing that moved on without new mail (Graph's new delta link).
    fn cursor_listed(&self) -> Cursor;

    /// End the session after a pass that went well (after a failure the
    /// session is dropped: it may be mid-answer).
    fn close(self) -> impl Future<Output = ()> + Send;
}

/// Where [`Source::fetch`] hands the answers: the pass.
pub(crate) trait Sink<I>: Send {
    /// The server's answer for message `id`: its raw RFC 822 mail, or
    /// `None` when it answered for the message without one.
    fn answer(&mut self, id: I, raw: Option<&[u8]>) -> impl Future<Output = PassResult<()>> + Send;

    /// Message `id` turned out larger than the size cap while it was
    /// downloaded (at least `size` bytes; its size was not announced): it
    /// is quarantined like one announced too large.
    fn too_large(&mut self, id: I, size: u64) -> impl Future<Output = PassResult<()>> + Send;
}
