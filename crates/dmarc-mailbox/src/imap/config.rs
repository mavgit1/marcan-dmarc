//! IMAP connection / poll configuration, as the host hands it to a pass.

use std::fmt;

/// The IMAP port for [`ImapSecurity::Tls`] (implicit TLS).
pub const DEFAULT_PORT: u16 = 993;
/// The IMAP port for [`ImapSecurity::StartTls`].
pub const STARTTLS_PORT: u16 = 143;
pub const DEFAULT_FOLDER: &str = "INBOX";
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;

/// How the connection is encrypted. There is no plaintext option: with
/// `StartTls` a server that does not offer the upgrade is an error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImapSecurity {
    /// TLS from the first byte (port 993).
    #[default]
    Tls,
    /// Plain TCP, then `STARTTLS` before anything else is sent (port 143).
    StartTls,
}

impl ImapSecurity {
    /// The usual port for this kind of connection.
    pub fn default_port(self) -> u16 {
        match self {
            Self::Tls => DEFAULT_PORT,
            Self::StartTls => STARTTLS_PORT,
        }
    }
}

/// Settings for connecting to an IMAP mailbox that receives DMARC reports,
/// logging in with `LOGIN` (a Microsoft 365 mailbox is read over Graph
/// instead: [`crate::GraphConfig`]). The password is never printed by
/// [`Debug`].
#[derive(Clone, PartialEq, Eq)]
pub struct ImapConfig {
    /// The mailbox's id ([`MailboxRef::id`](crate::MailboxRef::id)): its
    /// cursor, quarantine, retry counts and passes are stored under it.
    pub id: String,
    /// Which generation of the mailbox's stored state the host prepared
    /// this pass for, opaque to this crate: every write of the pass carries
    /// it back ([`Backend`](crate::Backend)), so the host can drop those of
    /// a pass whose mailbox was replaced while it ran.
    pub epoch: i64,
    pub host: String,
    pub port: u16,
    pub security: ImapSecurity,
    pub username: String,
    /// The password (or app password) `LOGIN` sends with the username.
    pub password: String,
    /// Mailbox folder to poll (default `INBOX`).
    pub folder: String,
}

impl fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapConfig")
            .field("id", &self.id)
            .field("epoch", &self.epoch)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &"***")
            .field("folder", &self.folder)
            .finish()
    }
}

impl ImapConfig {
    /// The mailbox's host (case folded), port and username in one string:
    /// an id for a host whose mailbox has no id of its own.
    pub fn mailbox_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.host.to_ascii_lowercase(),
            self.port,
            self.username
        )
    }

    /// Whether host, username and the password are all set.
    pub fn is_complete(&self) -> bool {
        !self.host.is_empty() && !self.username.is_empty() && !self.password.is_empty()
    }

    /// `text` with any occurrence of the password masked, for errors that
    /// echo server responses back to the UI.
    pub fn redact(&self, text: &str) -> String {
        let secret = self.password.as_str();
        if secret.is_empty() {
            return text.to_string();
        }
        text.replace(secret, "***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ImapConfig {
        ImapConfig {
            id: "m1".into(),
            epoch: 0,
            host: "h".into(),
            port: DEFAULT_PORT,
            security: ImapSecurity::Tls,
            username: "u".into(),
            password: "super-secret".into(),
            folder: DEFAULT_FOLDER.into(),
        }
    }

    #[test]
    fn each_security_has_its_usual_port() {
        assert_eq!(ImapSecurity::default(), ImapSecurity::Tls);
        assert_eq!(ImapSecurity::Tls.default_port(), 993);
        assert_eq!(ImapSecurity::StartTls.default_port(), 143);
    }

    #[test]
    fn debug_redacts_password() {
        let rendered = format!("{:?}", sample());
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("***"));
    }

    #[test]
    fn redact_masks_password_in_text() {
        let cfg = sample();
        assert_eq!(
            cfg.redact("login failed for super-secret"),
            "login failed for ***"
        );
        let empty = ImapConfig {
            password: String::new(),
            ..sample()
        };
        assert_eq!(empty.redact("unchanged"), "unchanged");
    }

    #[test]
    fn completeness_requires_host_username_and_password() {
        assert!(sample().is_complete());
        for cfg in [
            ImapConfig {
                host: String::new(),
                ..sample()
            },
            ImapConfig {
                username: String::new(),
                ..sample()
            },
            ImapConfig {
                password: String::new(),
                ..sample()
            },
        ] {
            assert!(!cfg.is_complete());
        }
    }

    #[test]
    fn mailbox_key_folds_host_case_only() {
        let a = sample();
        let b = ImapConfig {
            host: "H".into(),
            ..sample()
        };
        assert_eq!(a.mailbox_key(), b.mailbox_key());
        let c = ImapConfig {
            username: "U".into(),
            ..sample()
        };
        assert_ne!(a.mailbox_key(), c.mailbox_key());
    }
}
