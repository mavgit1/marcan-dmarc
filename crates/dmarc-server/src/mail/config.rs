//! The submission account, resolved from the settings ([`SmtpConfig`]), and
//! the rules for it: the submission ports, valid addresses, and which hosts
//! the mailbox login may be sent to.

use super::{Letterhead, SendError};
use dmarc_store::{AppSettings, SmtpSecurity};
use lettre::Address;
use std::fmt;

/// The submission port for [`SmtpSecurity::Tls`] (implicit TLS, RFC 8314).
pub const TLS_PORT: u16 = 465;
/// The submission port for [`SmtpSecurity::StartTls`] (RFC 6409).
pub const STARTTLS_PORT: u16 = 587;
/// Server-to-server delivery; never used for submission.
const RELAY_PORT: u16 = 25;

/// The usual port for `security`.
pub fn default_port(security: SmtpSecurity) -> u16 {
    match security {
        SmtpSecurity::Tls => TLS_PORT,
        SmtpSecurity::StartTls => STARTTLS_PORT,
    }
}

/// `raw` (trimmed) as an address mail can be sent from or to: valid for SMTP,
/// with a domain that is a real host name (no IP literal, at least two labels).
pub fn parse_address(raw: &str) -> Option<Address> {
    let address: Address = raw.trim().parse().ok()?;
    dmarc_domain::normalize_domain(address.domain()).ok()?;
    Some(address)
}

/// Whether the mailbox login may be used on `smtp_host`: the mailbox's own
/// server, or another host under the same organizational domain
/// (`imap.example.com` and `smtp.example.com`). The stored mailbox password is
/// only ever sent to the mailbox's provider, like the IMAP rule that keeps it
/// for the mailbox it was saved for.
pub fn same_provider(imap_host: &str, smtp_host: &str) -> bool {
    let (imap, smtp) = (
        dmarc_domain::normalize_name(imap_host),
        dmarc_domain::normalize_name(smtp_host),
    );
    if imap.is_empty() || smtp.is_empty() {
        return false;
    }
    let is_ip = |host: &str| host.parse::<std::net::IpAddr>().is_ok();
    imap == smtp
        || !is_ip(&imap)
            && !is_ip(&smtp)
            && matches!(
                (
                    dmarc_domain::organizational_domain(&imap),
                    dmarc_domain::organizational_domain(&smtp),
                ),
                (Some(a), Some(b)) if a == b
            )
}

/// Everything one submission needs, resolved from the settings. The password
/// is never printed by [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: String,
    pub password: String,
    /// The `From` address and the envelope sender.
    pub sender: Address,
    /// The `From` display name, as the letterhead decides it
    /// ([`Letterhead::from_name`]): "Acme IT via Marcan" without an operator
    /// key, "Acme IT" with one, "Marcan" without an operator name.
    pub sender_name: String,
    pub reply_to: Option<Address>,
}

impl fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &"***")
            .field("sender", &self.sender)
            .field("sender_name", &self.sender_name)
            .field("reply_to", &self.reply_to)
            .finish()
    }
}

impl SmtpConfig {
    /// The stored sending settings, ready to send: [`SendError::Disabled`]
    /// while sending is off, [`SendError::Incomplete`] (saying what is
    /// missing) while anything it needs is not set or not valid. The login is
    /// [`AppSettings::sending_login`]'s. Saving an enabled group checks the
    /// same.
    pub fn from_settings(settings: &AppSettings) -> Result<Self, SendError> {
        let sending = &settings.sending.group;
        if !sending.enabled {
            return Err(SendError::Disabled);
        }
        let incomplete = |why: &str| SendError::Incomplete(why.to_owned());
        if sending.host.is_empty() {
            return Err(incomplete("Enter the SMTP server's host name."));
        }
        let Some(port) = sending.port else {
            return Err(incomplete("Enter the SMTP server's port."));
        };
        check_port(port)?;
        let login = settings.sending_login();
        if sending.same_login_as_mailbox {
            if login.username.is_empty() || login.password.is_empty() {
                return Err(incomplete(
                    "The mailbox has no login yet. Set up the mailbox first, or turn off “Same login as the mailbox” and enter the SMTP login.",
                ));
            }
            if !same_provider(&settings.mailbox.host, &sending.host) {
                return Err(SendError::Incomplete(format!(
                    "The mailbox login is only sent to the mailbox's provider, and {} is not under {}. Turn off “Same login as the mailbox” and enter the SMTP login.",
                    sending.host, settings.mailbox.host
                )));
            }
        } else if login.username.is_empty() {
            return Err(incomplete("Enter the SMTP username."));
        } else if login.password.is_empty() {
            return Err(incomplete("Enter the SMTP password."));
        }
        let sender = if login.sender.is_empty() {
            return Err(incomplete("Enter the sender address."));
        } else {
            parse_address(&login.sender).ok_or_else(|| {
                SendError::Incomplete(format!(
                    "The sender address {} is not a valid email address.",
                    login.sender
                ))
            })?
        };
        let reply_to = match Some(&sending.reply_to).filter(|r| !r.is_empty()) {
            None => None,
            Some(raw) => Some(parse_address(raw).ok_or_else(|| {
                SendError::Incomplete(format!(
                    "The Reply-To address {raw} is not a valid email address."
                ))
            })?),
        };
        Ok(Self {
            host: sending.host.clone(),
            port,
            security: sending.security,
            username: login.username,
            password: login.password,
            sender,
            sender_name: Letterhead::new(settings).from_name(),
            reply_to,
        })
    }

    /// `text` with the password masked, for errors that echo server replies.
    pub(super) fn redact(&self, text: &str) -> String {
        if self.password.trim().is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.password, "***")
        }
    }
}

/// A port Marcan submits mail on: 1–65535, and never 25.
pub(crate) fn check_port(port: u16) -> Result<(), SendError> {
    match port {
        0 => Err(SendError::Incomplete(
            "The SMTP port must be between 1 and 65535.".to_owned(),
        )),
        RELAY_PORT => Err(SendError::Incomplete(format!(
            "Port {RELAY_PORT} is for delivery between mail servers. Use your provider's submission port: {TLS_PORT} with TLS or {STARTTLS_PORT} with STARTTLS."
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::license::test_keys::TEST_KEY;
    use dmarc_store::MailboxSettings;

    pub(in crate::mail) fn settings() -> AppSettings {
        let mut s = AppSettings {
            mailbox: MailboxSettings {
                host: "imap.example.com".into(),
                username: "reports@example.com".into(),
                password: "mailbox secret".into(),
                ..MailboxSettings::default()
            },
            ..AppSettings::default()
        };
        s.sending.group.enabled = true;
        s.sending.group.host = "smtp.example.com".into();
        s.sending.group.port = Some(465);
        s
    }

    fn incomplete(s: &AppSettings) -> String {
        match SmtpConfig::from_settings(s) {
            Err(SendError::Incomplete(why)) => why,
            other => panic!("expected Incomplete, got {other:?}"),
        }
    }

    #[test]
    fn the_same_login_is_the_mailbox_login_and_address() {
        let config = SmtpConfig::from_settings(&settings()).unwrap();
        assert_eq!(config.username, "reports@example.com");
        assert_eq!(config.password, "mailbox secret");
        assert_eq!(config.sender.to_string(), "reports@example.com");
        assert_eq!(config.security, SmtpSecurity::Tls);
        assert_eq!(config.reply_to, None);
        assert_eq!(config.sender_name, "Marcan");
        assert!(!format!("{config:?}").contains("secret"));
    }

    /// Without an operator key the From name says Marcan too; with one the
    /// operator alone; the address stays the sender's either way.
    #[test]
    fn the_from_name_follows_the_letterhead() {
        let mut s = settings();
        s.branding.operator_name = "Acme IT".into();
        let config = SmtpConfig::from_settings(&s).unwrap();
        assert_eq!(config.sender_name, "Acme IT via Marcan");
        assert_eq!(config.sender.to_string(), "reports@example.com");
        // A key that does not verify changes nothing.
        s.license.operator_key = Some("test@example.com.AAAA".into());
        let config = SmtpConfig::from_settings(&s).unwrap();
        assert_eq!(config.sender_name, "Acme IT via Marcan");

        s.license.operator_key = Some(TEST_KEY.into());
        let config = SmtpConfig::from_settings(&s).unwrap();
        assert_eq!(config.sender_name, "Acme IT");
        assert_eq!(config.sender.to_string(), "reports@example.com");
        assert_eq!(config.sender_name, Letterhead::new(&s).from_name());

        s.branding.operator_name.clear();
        let config = SmtpConfig::from_settings(&s).unwrap();
        assert_eq!(config.sender_name, "Marcan");
    }

    #[test]
    fn off_is_disabled_and_missing_pieces_are_named() {
        let mut s = settings();
        s.sending.group.enabled = false;
        assert_eq!(SmtpConfig::from_settings(&s), Err(SendError::Disabled));

        let mut s = settings();
        s.sending.group.host.clear();
        assert_eq!(incomplete(&s), "Enter the SMTP server's host name.");

        let mut s = settings();
        s.sending.group.port = None;
        assert_eq!(incomplete(&s), "Enter the SMTP server's port.");

        let mut s = settings();
        s.sending.group.port = Some(25);
        assert!(incomplete(&s).starts_with("Port 25 is for delivery between mail servers."));

        let mut s = settings();
        s.mailbox.password.clear();
        assert!(incomplete(&s).starts_with("The mailbox has no login yet."));

        let mut s = settings();
        s.mailbox.username = "reports".into();
        assert_eq!(incomplete(&s), "Enter the sender address.");

        let mut s = settings();
        s.sending.group.reply_to = "nobody".into();
        assert_eq!(
            incomplete(&s),
            "The Reply-To address nobody is not a valid email address."
        );

        let mut s = settings();
        s.sending.group.same_login_as_mailbox = false;
        assert_eq!(incomplete(&s), "Enter the SMTP username.");
        s.sending.group.username = "alerts".into();
        assert_eq!(incomplete(&s), "Enter the SMTP password.");
        s.sending.password = "smtp secret".into();
        // Its own login names no sender.
        assert_eq!(incomplete(&s), "Enter the sender address.");
        s.sending.group.sender = "dmarc@example.com".into();
        let config = SmtpConfig::from_settings(&s).unwrap();
        assert_eq!(config.username, "alerts");
        assert_eq!(config.password, "smtp secret");
    }

    #[test]
    fn the_mailbox_login_goes_only_to_the_mailbox_provider() {
        let mut s = settings();
        s.sending.group.host = "smtp.elsewhere.net".into();
        assert!(
            incomplete(&s).starts_with("The mailbox login is only sent to the mailbox's provider")
        );
        // Its own login may go anywhere.
        s.sending.group.same_login_as_mailbox = false;
        s.sending.group.username = "alerts@elsewhere.net".into();
        s.sending.password = "smtp secret".into();
        s.sending.group.sender = "dmarc@example.com".into();
        assert!(SmtpConfig::from_settings(&s).is_ok());

        assert!(same_provider("imap.example.com", "SMTP.example.com."));
        assert!(same_provider("outlook.office365.com", "smtp.office365.com"));
        assert!(same_provider("127.0.0.1", "127.0.0.1"));
        assert!(!same_provider("10.0.0.1", "192.168.0.1"));
        assert!(!same_provider("imap.example.com", "smtp.example.net"));
        assert!(!same_provider("", "smtp.example.com"));
        assert!(!same_provider("dovecot", "postfix"));
    }

    #[test]
    fn addresses_need_a_real_domain() {
        assert!(parse_address(" dmarc@example.com ").is_some());
        assert!(parse_address("dmarc@localhost").is_none());
        assert!(parse_address("dmarc@[192.0.2.1]").is_none());
        assert!(parse_address("dmarc").is_none());
        assert!(parse_address("a b@example.com").is_none());
    }
}
