//! Why a message was not sent ([`SendError`]), by the step that failed, in
//! words for the UI and in a few words for a list of recipients.

use super::{STARTTLS_PORT, TLS_PORT};
use crate::wording::accounts::locked_sentence;
use crate::wording::microsoft::{graph_send_docs, graph_send_sentence};
use dmarc_assess::LockReason;
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
    /// The sending account's credentials cannot be read with the server's
    /// secret key ([`LockReason`]).
    Locked { account: String, reason: LockReason },
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
    /// The server rejected the password login.
    Auth { detail: String },
    /// No token for a Microsoft login: the token endpoint refused, or could
    /// not be reached; `message` says why in plain words, `docs` is the
    /// guide's section on the fix, if one shows it.
    Token {
        message: String,
        detail: String,
        docs: Option<&'static str>,
    },
    /// Microsoft Graph did not take the message of Microsoft login
    /// `mailbox`; `refusal` says why, `detail` is Graph's code and message.
    Graph {
        mailbox: String,
        refusal: GraphRefusal,
        detail: String,
    },
    /// The server refused the recipient (at `RCPT TO`, or at `DATA` with a
    /// code about the recipient's address or mailbox); `reply` is its
    /// answer. A `5xx` is for good ([`SendError::is_permanent`]).
    Rejected { reply: String },
    /// The server refused the sending account, not the recipient: at `MAIL
    /// FROM`, or with a code about the sender (Exchange's `5.7.60` send-as
    /// denied, a `5.7.7xx` tenant block, `SendAsDenied`). Every message
    /// from the account fails the same way until it is fixed.
    SenderRejected { reply: String },
    /// The server refused the message at `DATA` with no sign of whose
    /// fault: its content, size or policy. Not held against the recipient.
    MessageRejected { reply: String },
    /// Anything else: an unexpected reply, a dropped connection, a timeout
    /// while sending.
    Other { message: String, detail: String },
}

/// Why Microsoft Graph did not take a Microsoft login's message
/// (`sendMail`), by what is to be changed. Graph answers before the message
/// reaches anyone, so none of these is about a recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphRefusal {
    /// `403` (`ErrorAccessDenied`, or `ErrorSendAsDenied`): the app may not
    /// send as the mailbox (no `Application Mail.Send` role assignment
    /// scoped to it, or one not applied yet). Marcan sends as the mailbox
    /// only, so a send-as refusal is about the same role.
    NoSendRole,
    /// `404`, or a code for a user without an Exchange Online mailbox: the
    /// address is no mailbox in the tenant.
    NoMailbox,
    /// Graph refused the token, a freshly fetched one too (`401`).
    TokenRefused,
    /// Graph kept throttling (`429`/`503`) past the wait a send allows.
    Throttled,
    /// The message is larger than Graph takes: not sent (over 4 MB in
    /// base64), or refused (`413`).
    TooLarge,
    /// Graph could not be reached at `host`.
    Unreachable { host: String },
    /// No answer within `seconds`: the message may or may not have gone.
    Timeout { seconds: u64 },
    /// Any other refusal (a `400`, a `500`): Graph's code says what.
    Refused,
}

/// The step of handing a message over a refusal came at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `MAIL FROM`: the sender.
    Sender,
    /// `RCPT TO`: the recipient.
    Recipient,
    /// `DATA` and the message's end.
    Message,
}

impl SendError {
    /// A refusal `reply` at `stage`, by whose it is. lettre's error does not
    /// say which command a refusal answered, so the sender hands the
    /// message over one command at a time and passes the stage. A code
    /// about the sender is the sender's at any stage (Exchange refuses
    /// `SendAs` only at the message's end); at `DATA`, only a code about
    /// the recipient's address or mailbox (`5.1.x` but `5.1.7`/`5.1.8`, the
    /// sender's; `5.2.x`) is the recipient's; anything else there is the
    /// message's, and not held against the recipient.
    pub fn refusal(stage: Stage, reply: String) -> Self {
        let code = enhanced_code(&reply);
        let senders = matches!(code, Some((_, "1", "7" | "8") | (_, "7", "57" | "60")))
            || code.is_some_and(|(_, subject, detail)| {
                subject == "7" && detail.parse::<u16>().is_ok_and(|d| (700..800).contains(&d))
            })
            || reply.to_ascii_lowercase().contains("sendasdenied");
        let recipients = matches!(code, Some((_, "1" | "2", _)));
        match stage {
            _ if senders => Self::SenderRejected { reply },
            Stage::Sender => Self::SenderRejected { reply },
            Stage::Recipient => Self::Rejected { reply },
            Stage::Message if recipients => Self::Rejected { reply },
            Stage::Message => Self::MessageRejected { reply },
        }
    }

    /// A short name of the failed step, for the API (`reason`) and logs.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Incomplete(_) => "incomplete",
            Self::Locked { .. } => "locked",
            Self::Resolve { .. } => "resolve",
            Self::Connect { .. } => "connect",
            Self::Tls { .. } => "tls",
            Self::StartTlsNotOffered { .. } => "starttls_not_offered",
            Self::Auth { .. } | Self::Token { .. } => "auth",
            Self::Rejected { .. } | Self::MessageRejected { .. } => "rejected",
            Self::SenderRejected { .. } => "sender_rejected",
            Self::Graph { refusal, .. } => match refusal {
                GraphRefusal::NoSendRole | GraphRefusal::NoMailbox => "sender_rejected",
                GraphRefusal::TokenRefused => "auth",
                GraphRefusal::Unreachable { .. } => "connect",
                GraphRefusal::TooLarge | GraphRefusal::Refused => "rejected",
                GraphRefusal::Throttled | GraphRefusal::Timeout { .. } => "other",
            },
            Self::Other { .. } => "other",
        }
    }

    /// Whether trying the same send again cannot help: the server refused
    /// the recipient for good (a `5xx` answer to its address). Everything
    /// else may pass: a `4xx` answer (try later), a server not reached, a
    /// timeout, a setting, a login or the sending account to fix, a message
    /// refused for no reason of the recipient's.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Rejected { reply } if reply.starts_with('5'))
    }

    /// The failure in a few words, for a list of recipients a report did
    /// not reach: "mailbox does not exist", "server unreachable". A refusal
    /// is read from its enhanced status code (RFC 3463) when it has one.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::Disabled | Self::Incomplete(_) => "sending not set up",
            Self::Locked { .. } => "login locked",
            Self::Resolve { .. } => "mail server not found",
            Self::Connect { .. } => "server unreachable",
            Self::Tls { .. } | Self::StartTlsNotOffered { .. } => "no secure connection",
            Self::Auth { .. } | Self::Token { .. } => "login rejected",
            Self::Rejected { reply } | Self::MessageRejected { reply } => refusal_cause(reply),
            Self::SenderRejected { .. } => "sending account refused",
            Self::Graph { refusal, .. } => match refusal {
                GraphRefusal::NoSendRole | GraphRefusal::NoMailbox => "sending account refused",
                GraphRefusal::TokenRefused => "login rejected",
                GraphRefusal::Throttled => "server busy",
                GraphRefusal::TooLarge => "message too large",
                GraphRefusal::Unreachable { .. } => "server unreachable",
                GraphRefusal::Timeout { .. } => "sending broke off",
                GraphRefusal::Refused => "refused by the server",
            },
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
            Self::Locked { account, reason } => locked_sentence(account, *reason),
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
            Self::Graph {
                mailbox, refusal, ..
            } => graph_send_sentence(mailbox, refusal),
            Self::Rejected { reply } | Self::MessageRejected { reply } => {
                format!("The server refused the message: {reply}")
            }
            Self::SenderRejected { reply } => format!(
                "The server refused to send from this account: {reply}. Check that its login may send as the account's email address."
            ),
            Self::Other { message, .. } | Self::Token { message, .. } => message.clone(),
        }
    }

    /// The section of the docs that shows the fix of a Microsoft login's
    /// setup (relative to their root), when [`SendError::message`] asks for
    /// a step in Entra or Exchange Online.
    pub fn docs(&self) -> Option<&'static str> {
        match self {
            Self::Token { docs, .. } => *docs,
            Self::Graph { refusal, .. } => graph_send_docs(refusal),
            _ => None,
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
            | Self::Graph { detail, .. }
            | Self::Other { detail, .. }
            | Self::Token { detail, .. } => detail,
            Self::Disabled
            | Self::Incomplete(_)
            | Self::Locked { .. }
            | Self::StartTlsNotOffered { .. }
            | Self::Rejected { .. }
            | Self::SenderRejected { .. }
            | Self::MessageRejected { .. } => "",
        }
    }
}

/// The enhanced status code (RFC 3463, `class.subject.detail`) a reply
/// (`550 5.1.1 <x@y>: …`) carries after its basic code, if any.
fn enhanced_code(reply: &str) -> Option<(&str, &str, &str)> {
    let code = reply.split_whitespace().nth(1)?;
    let mut parts = code.splitn(3, '.');
    let (class, subject, detail) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    (matches!(class, "2" | "4" | "5") && digits(subject) && digits(detail))
        .then_some((class, subject, detail))
}

/// A refusal (`550 5.1.1 <x@y>: …`) in a few words: a `4xx` one asks to
/// try later; a `5xx` one is read by its enhanced status code
/// (`class.subject.detail`) when it has one.
fn refusal_cause(reply: &str) -> &'static str {
    if reply.starts_with('4') {
        return "server busy";
    }
    match enhanced_code(reply).filter(|(class, ..)| *class == "5") {
        Some((_, "1", "1" | "10")) => "mailbox does not exist",
        Some((_, "1", _)) => "address refused",
        Some((_, "2", "2")) => "mailbox full",
        Some((_, "2", _)) => "mailbox unavailable",
        Some((_, "7", _)) => "refused by the server's policy",
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

    /// A refusal is the recipient's only at `RCPT TO` or with a code about
    /// the recipient; a sender's code or the `MAIL FROM` stage is the
    /// sending account's, and neither is held against the recipient.
    #[test]
    fn a_refusal_is_the_recipients_only_when_it_is_about_them() {
        use Stage::{Message, Recipient, Sender};
        let cases = [
            (Recipient, "550 5.1.1 <x@y.example>: user unknown", "rejected", true),
            (Recipient, "550 5.7.1 Relaying denied", "rejected", true),
            (Recipient, "450 4.2.1 try later", "rejected", false),
            (Sender, "550 5.7.1 Sender not allowed", "sender_rejected", false),
            (Sender, "553 sender refused", "sender_rejected", false),
            (
                Message,
                "550 5.7.60 SMTP; Client does not have permissions to send as this sender",
                "sender_rejected",
                false,
            ),
            (
                Message,
                "554 5.2.0 STOREDRV.Submission.Exception:SendAsDeniedException.MapiExceptionSendAsDenied",
                "sender_rejected",
                false,
            ),
            (
                Message,
                "550 5.7.708 Service unavailable. Access denied, traffic not accepted from this IP.",
                "sender_rejected",
                false,
            ),
            (Recipient, "550 5.7.60 not allowed to send as", "sender_rejected", false),
            (Recipient, "553 5.1.7 bad sender address", "sender_rejected", false),
            (Message, "552 5.2.2 mailbox full", "rejected", true),
            (Message, "550 5.1.1 user unknown", "rejected", true),
            (Message, "554 5.7.1 message refused as spam", "rejected", false),
            (Message, "554 transaction failed", "rejected", false),
        ];
        for (stage, reply, reason, permanent) in cases {
            let e = SendError::refusal(stage, reply.into());
            assert_eq!(
                (e.reason(), e.is_permanent()),
                (reason, permanent),
                "{stage:?} {reply}"
            );
            if reason == "sender_rejected" {
                assert_eq!(e.cause(), "sending account refused");
                assert!(e.message().contains("send from this account"), "{e}");
            }
        }
    }

    /// Graph's refusals are the sending account's (or the message's),
    /// never the recipient's: none is permanent, and each says why.
    #[test]
    fn a_graph_refusal_is_never_held_against_the_recipient() {
        let cases = [
            (
                GraphRefusal::NoSendRole,
                "sender_rejected",
                "sending account refused",
            ),
            (
                GraphRefusal::NoMailbox,
                "sender_rejected",
                "sending account refused",
            ),
            (GraphRefusal::TokenRefused, "auth", "login rejected"),
            (GraphRefusal::Throttled, "other", "server busy"),
            (GraphRefusal::TooLarge, "rejected", "message too large"),
            (
                GraphRefusal::Unreachable {
                    host: "graph.microsoft.com".into(),
                },
                "connect",
                "server unreachable",
            ),
            (
                GraphRefusal::Timeout { seconds: 60 },
                "other",
                "sending broke off",
            ),
            (GraphRefusal::Refused, "rejected", "refused by the server"),
        ];
        for (refusal, reason, cause) in cases {
            let e = SendError::Graph {
                mailbox: "dmarc@example.com".into(),
                refusal: refusal.clone(),
                detail: "ErrorAccessDenied: Access is denied.".into(),
            };
            assert_eq!((e.reason(), e.cause()), (reason, cause), "{refusal:?}");
            assert!(!e.is_permanent(), "{refusal:?}");
            assert!(e.message().ends_with('.'), "{e}");
            assert!(e
                .to_string()
                .ends_with("(ErrorAccessDenied: Access is denied.)"));
        }
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
