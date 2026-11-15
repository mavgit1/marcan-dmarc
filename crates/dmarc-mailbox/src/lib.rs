//! Mailbox fetch pass and polling loop for DMARC aggregate report attachments.
//!
//! Polls mailboxes using a cursor the host keeps (not a "seen" flag),
//! extracts report payloads (`.xml` / `.xml.gz` / `.zip`), parses them with
//! [`dmarc_parser`], and hands them to a [`Backend`] (which deduplicates by
//! `report_id`). Mailbox access is read-only. A mailbox is read over IMAP
//! (the folder opened with `EXAMINE`, messages fetched with `BODY.PEEK[]`
//! and never flagged, the login a password: [`ImapConfig`]) or, a Microsoft
//! 365 one, over Microsoft Graph (`GET`s only, with the app's access token:
//! [`GraphConfig`]). Apart from reading, it sends mail as a Microsoft 365
//! mailbox when the host asks, over the same Graph requests
//! ([`send_mime`]); Graph keeps a copy in that mailbox's Sent Items.
//!
//! The host lists its mailboxes ([`Backend::mailboxes`], each a
//! [`MailboxRef`]: an id and a poll interval) and prepares one's
//! configuration right before its pass ([`Backend::prepare`], a
//! [`MailboxConfig`]); every stored state (cursor, quarantine, retry counts,
//! passes) is keyed by that id, and every write of a pass carries the
//! [`MailboxConfig::epoch`] it was prepared with, so the host can drop those
//! of a pass whose mailbox it replaced meanwhile (this crate never forgets a
//! mailbox's state itself). The [`MailboxWorker`]'s polling loop is the one
//! driver of the passes: it runs one at a time over all of them,
//! round-robin, an import giving way after [`YIELD_AFTER`] when another
//! mailbox is due. A manual sync, a re-import or a settings change only
//! marks mailboxes and wakes the loop, so the host must keep
//! [`MailboxWorker::run`] running for anything to be read.
//!
//! This crate reads mailboxes and decides what a pass does; everything it
//! needs to remember or store (the configuration, the cursor, skipped
//! messages, retry counts, the reports, the pass outcomes) goes through the
//! [`Backend`] the host provides. It knows nothing about databases.
//!
//! # Modules
//!
//! - `worker`: the polling loop, its schedule and back-off.
//! - `pass`: one pass, the same whatever reads the folder: what is new,
//!   batches, parsing, storing, quarantine, retry counts, the cursor after
//!   each batch.
//! - `source`: the seam a pass reads a folder through: what is new since
//!   the cursor with sizes, and a batch's raw mail.
//! - `batch`: the pure batch planning and its limits.
//! - `attach`: finding report attachments in a raw message.
//! - `backend`: what the host provides ([`Backend`]) and the keys of the
//!   stored state ([`Cursor`], [`MessageRef`], [`AttemptKey`]).
//! - `progress`: the running pass's progress for the UI.
//! - `imap`: reading over IMAP: its settings, connecting and the connection
//!   test, its source, and the scripted test server.
//! - `graph`: a Microsoft 365 mailbox over Microsoft Graph: reading it
//!   (its settings, the requests, the folder lookup and the connection
//!   test, its source), sending a message as it, and the fake Graph server.

mod attach;
mod backend;
mod batch;
mod graph;
mod imap;
mod pass;
mod progress;
mod source;
mod worker;

pub use backend::{
    AttemptKey, AttemptKind, Backend, Cursor, Ingested, MailboxConfig, MailboxRef, MessageRef,
    PassRecord,
};
#[cfg(any(test, feature = "test-support"))]
pub use graph::fake_server as graph_fake;
pub use graph::{
    is_no_mailbox, probe_graph, send_mime, GraphApi, GraphConfig, GraphDocs, GraphProblem,
    GraphSendConfig, GraphToken, GraphWording, SendFailure, TokenFailure, GRAPH_URL,
    MAX_SEND_BODY_BYTES, SEND_REQUEST_TIMEOUT, SEND_THROTTLE_WAIT,
};
pub use imap::config::{
    ImapConfig, ImapSecurity, DEFAULT_FOLDER, DEFAULT_POLL_INTERVAL_SECS, DEFAULT_PORT,
    STARTTLS_PORT,
};
pub use imap::connect::{
    connect_mailbox, ConnectError, ConnectStage, FolderStatus, ImapFolder, MailboxProbe,
};
#[cfg(any(test, feature = "test-support"))]
pub use imap::fake_server;
pub use pass::{FailureKind, FetchError, FetchSummary, MAX_ATTEMPTS};
pub use progress::{PassPhase, PassProgress};
pub use worker::{MailboxWorker, SyncOutcome, SyncStart, YIELD_AFTER};
