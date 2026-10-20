//! IMAP fetch pass and polling loop for DMARC aggregate report attachments.
//!
//! Polls a mailbox using a UID cursor the host keeps (not `\Seen` / `UNSEEN`),
//! extracts report payloads (`.xml` / `.xml.gz` / `.zip`), parses them with
//! [`dmarc_parser`], and hands them to a [`Backend`] (which deduplicates by
//! `report_id`). Mailbox access is read-only: messages are fetched with
//! `BODY.PEEK[]` and never flagged.
//!
//! This crate talks IMAP and decides what a pass does; everything it needs to
//! remember or store (the configuration, the cursor, skipped messages, retry
//! counts, the reports, the pass outcomes) goes through the [`Backend`] the
//! host provides. It knows nothing about databases.

mod attach;
mod backend;
mod batch;
mod config;
mod connect;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_server;
mod pass;
mod progress;
mod worker;

pub use backend::{AttemptKey, AttemptKind, Backend, Cursor, Ingested, MessageRef, PassRecord};
pub use config::{
    ImapConfig, ImapSecurity, DEFAULT_FOLDER, DEFAULT_POLL_INTERVAL_SECS, DEFAULT_PORT,
    STARTTLS_PORT,
};
pub use connect::{
    connect_mailbox, ConnectError, ConnectStage, FolderStatus, ImapFolder, MailboxProbe,
};
pub use pass::{FailureKind, FetchError, FetchSummary, MAX_ATTEMPTS};
pub use progress::{ImapPhase, ImapProgress};
pub use worker::{ImapWorker, SyncOutcome, SyncStart};
