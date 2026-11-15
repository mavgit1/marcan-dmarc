//! How often the receiving accounts are polled, and how an IMAP connection
//! is secured.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Seconds between two passes of one account unless saved otherwise (the
/// mailbox crate's `DEFAULT_POLL_INTERVAL_SECS`).
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;

/// The receiving group: one poll interval for every account that receives
/// reports. The server checks it before saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReceivingSettings {
    /// From the end of an account's pass to the start of its next.
    pub poll_interval_secs: u64,
}

impl Default for ReceivingSettings {
    fn default() -> Self {
        Self {
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
        }
    }
}

/// The shortest interval, so a typo cannot hammer the mail server.
pub const MIN_POLL_INTERVAL_SECS: u64 = 10;
/// The longest interval: one day.
pub const MAX_POLL_INTERVAL_SECS: u64 = 86_400;

impl ReceivingSettings {
    /// The interval within ten seconds – one day.
    pub fn clamped(self) -> Self {
        Self {
            poll_interval_secs: self
                .poll_interval_secs
                .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS),
        }
    }
}

impl SettingsGroup for ReceivingSettings {
    const KEY: &'static str = "receiving";
}

/// How the IMAP connection is encrypted: TLS from the start (port 993) or
/// `STARTTLS` on a plain connection (port 143). Never plaintext.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ImapSecurity {
    #[default]
    Tls,
    StartTls,
}

impl ImapSecurity {
    /// The stored name (`accounts.imap_security`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
        }
    }

    pub(crate) fn from_stored(raw: &str) -> Self {
        match raw {
            "starttls" => Self::StartTls,
            _ => Self::Tls,
        }
    }
}
