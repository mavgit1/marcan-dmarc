//! The SMTP submission account mail is sent through, and the one place
//! "same login as the mailbox" is resolved.

use super::{AppSettings, SettingsGroup};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The SMTP account mail is sent through, as the API carries it (`PUT
/// /api/settings/sending` with the password beside it, the settings'
/// `sending` with what follows from it). Off by default, and nothing is
/// guessed: host and port stay unset until saved (the UI starts the port
/// field at the usual port). While sending is off it is saved as typed;
/// turned on, it needs everything sending needs. The server trims and
/// checks it before saving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendingGroup {
    /// Marcan may send mail (off by default).
    pub enabled: bool,
    /// Empty: not set.
    pub host: String,
    /// 1–65535, never 25. `null`: not set.
    pub port: Option<u16>,
    /// TLS from the start or `STARTTLS`; never plaintext.
    pub security: SmtpSecurity,
    /// Log in with the mailbox's username and password.
    pub same_login_as_mailbox: bool,
    /// The SMTP account's own username, used without the same login (kept,
    /// unused, with it). Empty: not set.
    pub username: String,
    /// The sender address. Empty: not set (with the same login, the mailbox
    /// username is used when it is an address).
    pub sender: String,
    /// Empty: none.
    pub reply_to: String,
    /// Where test emails and previews go. Empty: the sender address.
    pub test_recipient: String,
}

impl Default for SendingGroup {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: None,
            security: SmtpSecurity::Tls,
            same_login_as_mailbox: true,
            username: String::new(),
            sender: String::new(),
            reply_to: String::new(),
            test_recipient: String::new(),
        }
    }
}

/// The sending group as stored: the group and its password, the one value
/// the API never answers. [`AppSettings::sending_login`] applies "same login
/// as the mailbox".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendingSettings {
    #[serde(flatten)]
    pub group: SendingGroup,
    /// The SMTP account's own password, as typed (like the mailbox's); empty
    /// when unset.
    pub password: String,
}

impl SettingsGroup for SendingSettings {
    const KEY: &'static str = "sending";
}

/// How the SMTP submission connection is encrypted: TLS from the start
/// (port 465) or `STARTTLS` on a plain connection (port 587). Never
/// plaintext, and never port 25: Marcan submits mail, it does not relay it.
/// The same two choices as [`ImapSecurity`], kept apart so neither protocol's
/// ports leak into the other's wording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    #[default]
    Tls,
    StartTls,
}

/// The login and sender address sending uses, once "same login as the
/// mailbox" is applied ([`AppSettings::sending_login`]). The password is
/// never printed by [`Debug`](std::fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct SendingLogin {
    /// Empty when unset.
    pub username: String,
    /// Empty when unset.
    pub password: String,
    /// The stored sender, else (with the same login) the mailbox username when
    /// it looks like an address; empty when neither. Not validated here.
    pub sender: String,
}

impl std::fmt::Debug for SendingLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendingLogin")
            .field("username", &self.username)
            .field("password", &"***")
            .field("sender", &self.sender)
            .finish()
    }
}

impl AppSettings {
    /// The one place "same login as the mailbox" is resolved: with it, the
    /// username and password are the mailbox's, and without a stored sender
    /// mail goes out from the mailbox username when that looks like an
    /// address (an `@` and a dot in the domain; the server validates it).
    pub fn sending_login(&self) -> SendingLogin {
        let sending = &self.sending.group;
        let (username, password) = if sending.same_login_as_mailbox {
            (&self.mailbox.username, &self.mailbox.password)
        } else {
            (&sending.username, &self.sending.password)
        };
        let login_is_address = username
            .rsplit_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
        let sender = if !sending.sender.is_empty() {
            sending.sender.clone()
        } else if sending.same_login_as_mailbox && login_is_address {
            username.clone()
        } else {
            String::new()
        };
        SendingLogin {
            username: username.clone(),
            password: password.clone(),
            sender,
        }
    }

    /// Where test emails and previews go: the stored test recipient, else
    /// the sender address ([`Self::sending_login`]); empty when neither. Not
    /// validated here.
    pub fn test_recipient(&self) -> String {
        match self.sending.group.test_recipient.as_str() {
            "" => self.sending_login().sender,
            to => to.to_owned(),
        }
    }
}
