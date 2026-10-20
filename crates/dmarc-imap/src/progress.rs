//! Live state of the running pass, for the UI.

use std::sync::{Arc, Mutex};

/// What a running pass is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapPhase {
    /// Resolving, connecting, logging in.
    Connecting,
    /// Selecting the folder, searching for new UIDs and reading their sizes.
    Scanning,
    /// Downloading and ingesting message bodies batch by batch.
    Importing,
}

/// Live state of the running pass. Totals are 0 until scanning has found the
/// new messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapProgress {
    pub phase: ImapPhase,
    /// New messages found above the cursor.
    pub total_messages: u32,
    pub processed_messages: u32,
    /// Announced size of the messages that will be downloaded.
    pub total_bytes: u64,
    pub processed_bytes: u64,
    /// Reports newly stored so far in this pass.
    pub reports_ingested: u32,
    /// Unix seconds when the pass started.
    pub started_at: i64,
}

impl ImapProgress {
    pub(crate) fn starting(now: i64) -> Self {
        Self {
            phase: ImapPhase::Connecting,
            total_messages: 0,
            processed_messages: 0,
            total_bytes: 0,
            processed_bytes: 0,
            reports_ingested: 0,
            started_at: now,
        }
    }
}

/// The progress slot the worker publishes and a pass writes into.
#[derive(Clone, Default)]
pub(crate) struct SharedProgress(Arc<Mutex<Option<ImapProgress>>>);

impl SharedProgress {
    pub(crate) fn get(&self) -> Option<ImapProgress> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn begin(&self, now: i64) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ImapProgress::starting(now));
    }

    pub(crate) fn end(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// Change the progress; starts one at `now` when none is published yet.
    pub(crate) fn update(&self, now: i64, f: impl FnOnce(&mut ImapProgress)) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(guard.get_or_insert_with(|| ImapProgress::starting(now)));
    }
}
