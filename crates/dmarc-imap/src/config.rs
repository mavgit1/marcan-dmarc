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

/// Settings for connecting to an IMAP mailbox that receives DMARC reports.
/// The password is never printed by [`Debug`].
#[derive(Clone, PartialEq, Eq)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: ImapSecurity,
    pub username: String,
    pub password: String,
    /// Mailbox folder to poll (default `INBOX`).
    pub folder: String,
    /// Seconds between polling passes.
    pub poll_interval_secs: u64,
}

impl fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &"***")
            .field("folder", &self.folder)
            .field("poll_interval_secs", &self.poll_interval_secs)
            .finish()
    }
}

impl ImapConfig {
    /// Identifies the mailbox (host, port, username) its UID cursor belongs to.
    pub fn mailbox_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.host.to_ascii_lowercase(),
            self.port,
            self.username
        )
    }

    /// Whether host, username and password are all set.
    pub fn is_complete(&self) -> bool {
        !self.host.is_empty() && !self.username.is_empty() && !self.password.is_empty()
    }

    /// `text` with any occurrence of the password masked, for errors that echo
    /// server responses back to the UI.
    pub fn redact(&self, text: &str) -> String {
        if self.password.is_empty() {
            text.to_string()
        } else {
            text.replace(&self.password, "***")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ImapConfig {
        ImapConfig {
            host: "h".into(),
            port: DEFAULT_PORT,
            security: ImapSecurity::Tls,
            username: "u".into(),
            password: "super-secret".into(),
            folder: DEFAULT_FOLDER.into(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
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
