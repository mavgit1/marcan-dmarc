//! Why a message was not sent ([`SendError`]), by the step that failed, in
//! words for the UI and in a few words for a list of recipients.

use super::{STARTTLS_PORT, TLS_PORT};
use std::fmt;

/// Why a message was not sent, by the step that failed. [`SendError::message`]
/// is the sentence for the UI; [`SendError::detail`] the raw reason, with the
/// password masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// Sending is off in Settings.
    Disabled,
    /// A setting sending needs is missing or not valid; the text says which.
    Incomplete(String),
    /// The host name did not resolve.
    Resolve { host: String, detail: String },
    /// The TCP connection was refused or timed out, or the server closed it
    /// or never greeted. `silent`: connected with `STARTTLS`, but the server
    /// said nothing (it probably expects TLS from the start).
    Connect {
        host: String,
        port: u16,
        silent: bool,
        detail: String,
    },
    /// The TLS handshake failed (from the start, or after `STARTTLS`).
    Tls {
        host: String,
        port: u16,
        detail: String,
    },
    /// `STARTTLS` was chosen and the server does not offer it. Nothing else
    /// was sent: no login in plaintext.
    StartTlsNotOffered { port: u16 },
    /// The server rejected the login.
    Auth { detail: String },
    /// The server refused the message; `reply` is its answer.
    Rejected { reply: String },
    /// Anything else: an unexpected reply, a dropped connection, a timeout
    /// while sending.
    Other { message: String, detail: String },
}

impl SendError {
    /// A short name of the failed step, for the API (`reason`) and logs.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Incomplete(_) => "incomplete",
            Self::Resolve { .. } => "resolve",
            Self::Connect { .. } => "connect",
            Self::Tls { .. } => "tls",
            Self::StartTlsNotOffered { .. } => "starttls_not_offered",
            Self::Auth { .. } => "auth",
            Self::Rejected { .. } => "rejected",
            Self::Other { .. } => "other",
        }
    }

    /// Whether trying the same send again cannot help: the server refused
    /// the message or its address for good (a `5xx` answer). Everything else
    /// may pass: a `4xx` answer (try later), a server not reached, a
    /// timeout, a setting or a login to fix.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Rejected { reply } if reply.starts_with('5'))
    }

    /// The failure in a few words, for a list of recipients a report did
    /// not reach: "mailbox does not exist", "server unreachable". A refusal
    /// is read from its enhanced status code (RFC 3463) when it has one.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::Disabled | Self::Incomplete(_) => "sending not set up",
            Self::Resolve { .. } => "mail server not found",
            Self::Connect { .. } => "server unreachable",
            Self::Tls { .. } | Self::StartTlsNotOffered { .. } => "no secure connection",
            Self::Auth { .. } => "login rejected",
            Self::Rejected { reply } => refusal_cause(reply),
            Self::Other { .. } => "sending broke off",
        }
    }

    /// What went wrong and what to check, in plain words.
    pub fn message(&self) -> String {
        match self {
            Self::Disabled => {
                "Sending is off. Turn it on under Settings → Sending first.".to_owned()
            }
            Self::Incomplete(why) => why.clone(),
            Self::Resolve { host, .. } => {
                format!("Server not found: {host} does not resolve. Check the SMTP host name.")
            }
            Self::Connect {
                host,
                port,
                silent: true,
                ..
            } => format!(
                "{host}:{port} did not greet. The server may expect TLS from the start: choose TLS (usually port {TLS_PORT})."
            ),
            Self::Connect { host, port, .. } => format!("Could not reach {host}:{port}."),
            Self::Tls { host, port, .. } => format!(
                "Could not establish a secure (TLS) connection with {host}:{port}. Port {TLS_PORT} usually takes TLS, port {STARTTLS_PORT} STARTTLS."
            ),
            Self::StartTlsNotOffered { port } => format!(
                "The server does not offer STARTTLS on port {port}, so Marcan did not log in. Choose TLS (usually port {TLS_PORT}) instead."
            ),
            Self::Auth { .. } => {
                "Login rejected. Check the username and password; many providers require an app password."
                    .to_owned()
            }
            Self::Rejected { reply } => format!("The server refused the message: {reply}"),
            Self::Other { message, .. } => message.clone(),
        }
    }

    /// The raw reason (an OS error, the server's reply), empty when the
    /// message says it all.
    pub fn detail(&self) -> &str {
        match self {
            Self::Resolve { detail, .. }
            | Self::Connect { detail, .. }
            | Self::Tls { detail, .. }
            | Self::Auth { detail }
            | Self::Other { detail, .. } => detail,
            Self::Disabled
            | Self::Incomplete(_)
            | Self::StartTlsNotOffered { .. }
            | Self::Rejected { .. } => "",
        }
    }
}

/// A refusal (`550 5.1.1 <x@y>: …`) in a few words: a `4xx` one asks to
/// try later; a `5xx` one is read by its enhanced status code
/// (`class.subject.detail`) when it has one.
fn refusal_cause(reply: &str) -> &'static str {
    if reply.starts_with('4') {
        return "server busy";
    }
    let enhanced = reply.split_whitespace().nth(1).and_then(|code| {
        let mut parts = code.splitn(3, '.');
        let class = parts.next()?;
        let subject = parts.next()?;
        let detail = parts.next()?;
        (class == "5"
            && subject.chars().all(|c| c.is_ascii_digit())
            && detail.chars().all(|c| c.is_ascii_digit()))
        .then(|| (subject.to_owned(), detail.to_owned()))
    });
    match enhanced.as_ref().map(|(s, d)| (s.as_str(), d.as_str())) {
        Some(("1", "1" | "10")) => "mailbox does not exist",
        Some(("1", _)) => "address refused",
        Some(("2", "2")) => "mailbox full",
        Some(("2", _)) => "mailbox unavailable",
        Some(("7", _)) => "refused by the server's policy",
        _ => "refused by the server",
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = self.detail();
        if detail.is_empty() {
            f.write_str(&self.message())
        } else {
            write!(f, "{} ({detail})", self.message())
        }
    }
}

impl std::error::Error for SendError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5xx refusal is permanent and read from its enhanced code; anything
    /// else may pass and is tried again.
    #[test]
    fn a_failure_is_permanent_only_when_refused_for_good() {
        let refused = |reply: &str| SendError::Rejected {
            reply: reply.into(),
        };
        let cases = [
            (
                "550 5.1.1 <x@y.example>: user unknown",
                true,
                "mailbox does not exist",
            ),
            ("553 5.1.3 bad address syntax", true, "address refused"),
            ("552 5.2.2 mailbox full", true, "mailbox full"),
            (
                "554 5.7.1 relay denied",
                true,
                "refused by the server's policy",
            ),
            ("550 no such user", true, "refused by the server"),
            ("451 4.7.1 greylisted, try again", false, "server busy"),
            ("421 service not available", false, "server busy"),
        ];
        for (reply, permanent, cause) in cases {
            let e = refused(reply);
            assert_eq!((e.is_permanent(), e.cause()), (permanent, cause), "{reply}");
        }
        let unreachable = SendError::Connect {
            host: "smtp.example.com".into(),
            port: 465,
            silent: false,
            detail: "connection refused".into(),
        };
        assert!(!unreachable.is_permanent());
        assert_eq!(unreachable.cause(), "server unreachable");
        let login = SendError::Auth {
            detail: "535 5.7.8 bad credentials".into(),
        };
        assert!(!login.is_permanent(), "a login is fixed in Settings");
        let timeout = SendError::Other {
            message: "no answer".into(),
            detail: String::new(),
        };
        assert!(!timeout.is_permanent());
    }

    #[test]
    fn errors_say_why_in_plain_words() {
        let e = SendError::Resolve {
            host: "smtp.invalid".into(),
            detail: "failed to lookup address information".into(),
        };
        assert_eq!(
            e.message(),
            "Server not found: smtp.invalid does not resolve. Check the SMTP host name."
        );
        assert_eq!(e.reason(), "resolve");
        assert!(e
            .to_string()
            .ends_with("(failed to lookup address information)"));
        assert_eq!(
            SendError::StartTlsNotOffered { port: 587 }.message(),
            "The server does not offer STARTTLS on port 587, so Marcan did not log in. Choose TLS (usually port 465) instead."
        );
        assert!(SendError::Auth {
            detail: String::new()
        }
        .to_string()
        .starts_with("Login rejected."));
        assert_eq!(
            SendError::Rejected {
                reply: "550 5.7.1 Sender not allowed".into()
            }
            .message(),
            "The server refused the message: 550 5.7.1 Sender not allowed"
        );
    }
}
