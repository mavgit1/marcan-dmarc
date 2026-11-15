//! What sending needs besides an account: on or off, which account, and the
//! addresses besides the account's own (mail always comes from that one).

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The sending group as stored. Off by default, with no account: the server
/// resolves `account_id` to the account's SMTP server and login when it
/// sends. Nothing here is a secret. The server trims and checks it before
/// saving.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendingSettings {
    /// Marcan may send mail (off by default).
    pub enabled: bool,
    /// The account mail goes out through; `None`: none chosen.
    pub account_id: Option<i64>,
    /// Empty: none.
    pub reply_to: String,
    /// Where test emails and previews go. Empty: the account's address.
    pub test_recipient: String,
}

impl SettingsGroup for SendingSettings {
    const KEY: &'static str = "sending";
}

/// How the SMTP submission connection is encrypted: TLS from the start
/// (port 465) or `STARTTLS` on a plain connection (port 587). Never
/// plaintext, and never port 25: Marcan submits mail, it does not relay it.
/// The same two choices as [`ImapSecurity`](super::ImapSecurity), kept apart
/// so neither protocol's ports leak into the other's wording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    #[default]
    Tls,
    StartTls,
}

impl SmtpSecurity {
    /// The stored name (`accounts.smtp_security`).
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
