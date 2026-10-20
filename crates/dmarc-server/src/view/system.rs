//! The running mailbox pass as `GET /api/system/health` carries it.

use dmarc_assess::is_bulk_import;
use serde::Serialize;
use ts_rs::TS;

/// What a running pass is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ImapPhase {
    /// Resolving, connecting, logging in.
    Connecting,
    /// Selecting the folder, searching for new UIDs and reading their sizes.
    Scanning,
    /// Downloading and ingesting message bodies batch by batch.
    Importing,
}

impl From<dmarc_imap::ImapPhase> for ImapPhase {
    fn from(p: dmarc_imap::ImapPhase) -> Self {
        match p {
            dmarc_imap::ImapPhase::Connecting => Self::Connecting,
            dmarc_imap::ImapPhase::Scanning => Self::Scanning,
            dmarc_imap::ImapPhase::Importing => Self::Importing,
        }
    }
}

/// The background DNS checks as `GET /api/system/health` carries them: how
/// many monitored domains have a stored result, and whether a warm pass runs
/// (or was just asked for, by a new domain).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DnsCheckProgress {
    /// Monitored (not ignored) domains.
    pub domains: i64,
    /// Of those, with a stored DNS result.
    pub checked: i64,
    /// A warm pass is running or about to start.
    pub running: bool,
    /// Domains without a result are being checked now: the pass runs and
    /// some are left. The UI shows "checked of domains" while this holds.
    pub checking: bool,
}

impl DnsCheckProgress {
    pub fn new(coverage: dmarc_store::DnsCoverage, running: bool) -> Self {
        Self {
            domains: coverage.domains,
            checked: coverage.checked,
            running,
            checking: running && coverage.checked < coverage.domains,
        }
    }
}

/// Live progress of a running IMAP pass.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ImapProgressResponse {
    /// Always `true`; present so clients can test one field.
    pub running: bool,
    pub phase: ImapPhase,
    /// New messages found above the cursor (0 until scanning finishes).
    pub total_messages: u32,
    pub processed_messages: u32,
    /// Announced size of the messages being downloaded.
    pub total_bytes: u64,
    pub processed_bytes: u64,
    /// Reports newly stored by this pass so far.
    pub reports_ingested: u32,
    /// A bulk import (a first sync of a full mailbox, say), not routine
    /// polling: worth keeping its progress in view.
    pub bulk: bool,
    /// Unix seconds.
    pub started_at: i64,
}

impl From<dmarc_imap::ImapProgress> for ImapProgressResponse {
    fn from(p: dmarc_imap::ImapProgress) -> Self {
        let phase = ImapPhase::from(p.phase);
        Self {
            running: true,
            phase,
            total_messages: p.total_messages,
            processed_messages: p.processed_messages,
            total_bytes: p.total_bytes,
            processed_bytes: p.processed_bytes,
            reports_ingested: p.reports_ingested,
            bulk: is_bulk_import(phase == ImapPhase::Importing, p.total_messages),
            started_at: p.started_at,
        }
    }
}
