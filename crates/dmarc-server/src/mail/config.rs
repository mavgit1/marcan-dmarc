//! The sending account as one send needs it ([`SendConfig`], resolved
//! from the sending account by `accounts::Accounts::send_config`: an SMTP
//! server, or a Microsoft 365 mailbox over Graph), and the rules for it:
//! the submission ports and valid addresses.

use super::SendError;
use dmarc_mailbox::GraphSendConfig;
use dmarc_store::SmtpSecurity;
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

/// A Reply-To as entered or stored: `None` when empty, else `raw` as an
/// address ([`parse_address`]) or the sentence that says it is none.
pub fn parse_reply_to(raw: &str) -> Result<Option<Address>, String> {
    if raw.is_empty() {
        return Ok(None);
    }
    parse_address(raw)
        .map(Some)
        .ok_or_else(|| format!("The Reply-To address {raw} is not a valid email address."))
}

/// The submission server a password login sends through, logged in as
/// `username`. The password is never printed by [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpServer {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: String,
    /// `AUTH PLAIN` or `AUTH LOGIN` with this password (or app password).
    pub password: String,
}

impl fmt::Debug for SmtpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpServer")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

impl SmtpServer {
    /// `text` with the password masked, for errors that echo server replies.
    pub(super) fn redact(&self, text: &str) -> String {
        if self.password.trim().is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.password, "***")
        }
    }
}

/// How a message leaves: handed to a submission server over SMTP (a
/// password login), or posted to Microsoft Graph as the account's mailbox
/// (a Microsoft login, with the same token it reads with).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Smtp(SmtpServer),
    Graph(GraphSendConfig),
}

/// Everything one send needs, resolved from the sending account
/// (`accounts::Accounts::send_config`): how the message leaves, and the
/// addresses it carries. The same [`Message`](super::Message) is built into
/// the same MIME whichever way it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendConfig {
    pub transport: Transport,
    /// The `From` address (and over SMTP the envelope sender).
    pub sender: Address,
    /// The `From` display name, as the letterhead decides it
    /// ([`Letterhead::from_name`](super::Letterhead::from_name)): "Acme IT
    /// via Marcan" without an operator key, "Acme IT" with one, "Marcan"
    /// without an operator name.
    pub sender_name: String,
    pub reply_to: Option<Address>,
}

impl SendConfig {
    /// The SMTP server it sends through, if it sends over SMTP.
    pub fn smtp(&self) -> Option<&SmtpServer> {
        match &self.transport {
            Transport::Smtp(server) => Some(server),
            Transport::Graph(_) => None,
        }
    }

    /// The mailbox it sends as over Graph, if it does.
    pub fn graph(&self) -> Option<&GraphSendConfig> {
        match &self.transport {
            Transport::Graph(graph) => Some(graph),
            Transport::Smtp(_) => None,
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

    /// A complete submission account, as the sending account resolves to.
    pub(in crate::mail) fn config() -> SendConfig {
        SendConfig {
            transport: Transport::Smtp(SmtpServer {
                host: "smtp.example.com".into(),
                port: TLS_PORT,
                security: SmtpSecurity::Tls,
                username: "reports@example.com".into(),
                password: "mailbox secret".into(),
            }),
            sender: "reports@example.com".parse().unwrap(),
            sender_name: "Marcan".into(),
            reply_to: None,
        }
    }

    #[test]
    fn debug_and_redact_never_print_the_password() {
        let config = config();
        assert!(!format!("{config:?}").contains("secret"));
        let server = config.smtp().unwrap();
        assert_eq!(server.redact("535 mailbox secret"), "535 ***");
        assert!(config.graph().is_none());
    }

    #[test]
    fn port_25_and_0_are_refused() {
        assert!(check_port(465).is_ok());
        assert!(check_port(0).is_err());
        match check_port(25) {
            Err(SendError::Incomplete(why)) => {
                assert!(why.starts_with("Port 25 is for delivery between mail servers."));
            }
            other => panic!("{other:?}"),
        }
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
