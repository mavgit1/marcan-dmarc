//! The mailbox the IMAP worker polls.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The stored mailbox settings, as saved: nothing is defaulted here, so a
/// caller sees what is set and applies the worker's defaults itself. The
/// server checks and trims them before saving; the password is a secret the
/// API never answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxSettings {
    /// Empty when unset.
    pub host: String,
    pub port: Option<u16>,
    pub security: Option<ImapSecurity>,
    /// Empty when unset.
    pub username: String,
    /// As typed (spaces can be part of a password); empty when unset.
    pub password: String,
    pub folder: Option<String>,
    pub poll_interval_secs: Option<u64>,
}

impl SettingsGroup for MailboxSettings {
    const KEY: &'static str = "mailbox";
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
