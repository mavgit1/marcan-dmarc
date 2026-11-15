//! What is said about a Microsoft login: why the token endpoint refused
//! (its AADSTS codes in plain words), why Microsoft Graph did not let a
//! pass or a test read the mailbox ([`graph_sentence`]) or did not take a
//! message ([`graph_send_sentence`]), and what the account form is
//! missing. The raw answer always goes beside these as the `detail`, and
//! the guide's section on the fix as the `docs` when one shows it (the
//! `*_docs` functions): a setup step in Entra or Exchange Online.

use crate::accounts::oauth::TokenErrorKind;
use crate::mail::GraphRefusal;
use dmarc_mailbox::GraphProblem;

/// Why Microsoft did not issue a token, and what to change.
pub fn token_error_sentence(kind: TokenErrorKind) -> &'static str {
    match kind {
        TokenErrorKind::WrongTenant => {
            "Microsoft does not know this tenant ID. Copy the Directory (tenant) ID from the app registration's overview in Microsoft Entra."
        }
        TokenErrorKind::UnknownClient => {
            "The tenant has no app registration with this client ID. Copy the Application (client) ID from the app registration's overview."
        }
        TokenErrorKind::AssertionRejected => {
            "Microsoft refused the certificate. Upload the certificate Marcan generated to the app registration (Certificates & secrets, Certificates), then test again."
        }
        TokenErrorKind::Refused => "Microsoft refused the token request.",
        TokenErrorKind::Unreachable => {
            "Could not reach the Microsoft login service (login.microsoftonline.com)."
        }
        TokenErrorKind::BadAnswer => {
            "The Microsoft login service gave an answer Marcan cannot read."
        }
        TokenErrorKind::BadCredential => {
            "The stored certificate key cannot be used. Generate a new certificate and upload it to the app registration."
        }
    }
}

/// The guide's section on registering the app: where the tenant and client
/// ID are copied from.
pub const REGISTER_DOCS: &str = "accounts#register-the-app";

/// The guide's section on the certificate and its upload to the app
/// registration.
pub const CERTIFICATE_DOCS: &str = "accounts#certificate";

/// The guide's section on the Exchange Online roles, with the PowerShell
/// commands.
pub const EXCHANGE_DOCS: &str = "accounts#exchange-online";

/// The guide's section on what [`token_error_sentence`] asks to change.
pub fn token_error_docs(kind: TokenErrorKind) -> Option<&'static str> {
    match kind {
        TokenErrorKind::WrongTenant | TokenErrorKind::UnknownClient => Some(REGISTER_DOCS),
        TokenErrorKind::AssertionRejected | TokenErrorKind::BadCredential => Some(CERTIFICATE_DOCS),
        TokenErrorKind::Refused | TokenErrorKind::Unreachable | TokenErrorKind::BadAnswer => None,
    }
}

/// Why Microsoft Graph did not let a pass or the connection test read the
/// mailbox, and what to change.
pub fn graph_sentence(problem: &GraphProblem<'_>) -> String {
    match *problem {
        GraphProblem::Unreachable { host } => {
            format!("Could not reach Microsoft Graph ({host}).")
        }
        GraphProblem::TokenRefused { mailbox } => format!(
            "Microsoft Graph refused the app registration's token for {mailbox}, a new one too. Check that the tenant ID is the tenant of the mailbox, then test again."
        ),
        GraphProblem::NoAccess { mailbox } => format!(
            "The app registration may not read {mailbox}. In Exchange Online, give the app's service principal the role Application Mail.Read scoped to this mailbox. A new assignment can take 30 minutes to 2 hours to apply: test again later."
        ),
        GraphProblem::NoMailbox { mailbox } => format!(
            "{mailbox} is not a mailbox in this Microsoft 365 tenant. Enter the address of a mailbox in the tenant the app registration belongs to."
        ),
        GraphProblem::NoFolder { mailbox, folder } => format!(
            "{mailbox} has no folder {folder}. Choose one of its folders (the test lists them), or INBOX."
        ),
        GraphProblem::Throttled { mailbox } => format!(
            "Microsoft Graph is limiting the requests for {mailbox} just now. Marcan tries again later."
        ),
    }
}

/// The guide's section on what [`graph_sentence`] asks to change in Entra
/// or Exchange Online.
pub fn graph_docs(problem: &GraphProblem<'_>) -> Option<&'static str> {
    match problem {
        GraphProblem::NoAccess { .. } => Some(EXCHANGE_DOCS),
        GraphProblem::Unreachable { .. }
        | GraphProblem::TokenRefused { .. }
        | GraphProblem::NoMailbox { .. }
        | GraphProblem::NoFolder { .. }
        | GraphProblem::Throttled { .. } => None,
    }
}

/// Why Microsoft Graph did not take a message of `mailbox` (`sendMail`),
/// and what to change.
pub fn graph_send_sentence(mailbox: &str, refusal: &GraphRefusal) -> String {
    match refusal {
        GraphRefusal::NoSendRole => format!(
            "The app registration may not send as {mailbox}. In Exchange Online, give the app's service principal the role Application Mail.Send scoped to this mailbox. A new assignment can take 30 minutes to 2 hours to apply: try again later."
        ),
        GraphRefusal::NoMailbox => format!(
            "{mailbox} is not a mailbox in this Microsoft 365 tenant, so Marcan cannot send as it. Enter the address of a mailbox in the tenant the app registration belongs to."
        ),
        GraphRefusal::TokenRefused => format!(
            "Microsoft Graph refused the app registration's token for {mailbox}, a new one too. Check that the tenant ID is the tenant of the mailbox, then try again."
        ),
        GraphRefusal::Throttled => format!(
            "Microsoft Graph is limiting the requests for {mailbox} just now, so the message was not sent. Try again later."
        ),
        GraphRefusal::TooLarge => {
            "The message is larger than Microsoft Graph takes (about 3 MB of mail), so it was not sent.".to_owned()
        }
        GraphRefusal::Unreachable { host } => {
            format!("Could not reach Microsoft Graph ({host}); the message was not sent.")
        }
        GraphRefusal::Timeout { seconds } => format!(
            "Microsoft Graph did not answer within {seconds} seconds; the message may or may not have been sent."
        ),
        GraphRefusal::Refused => "Microsoft Graph refused the message.".to_owned(),
    }
}

/// The guide's section on what [`graph_send_sentence`] asks to change in
/// Exchange Online.
pub fn graph_send_docs(refusal: &GraphRefusal) -> Option<&'static str> {
    match refusal {
        GraphRefusal::NoSendRole => Some(EXCHANGE_DOCS),
        _ => None,
    }
}

/// The token step of a connection test went through.
pub const TOKEN_ISSUED: &str = "Microsoft issued a token for the app registration.";

/// Said when the tenant ID is empty.
pub const ENTER_TENANT: &str = "Enter the tenant ID.";

/// Said when the client ID is empty.
pub const ENTER_CLIENT: &str = "Enter the client ID.";

/// Said when the tenant ID is not a GUID.
pub const TENANT_NOT_GUID: &str =
    "The tenant ID is a GUID: 32 hex digits in five groups. Copy the Directory (tenant) ID from the app registration's overview.";

/// Said when the client ID is not a GUID.
pub const CLIENT_NOT_GUID: &str =
    "The client ID is a GUID: 32 hex digits in five groups. Copy the Application (client) ID from the app registration's overview.";

/// Said when the certificate login has no certificate.
pub const GENERATE_CERTIFICATE: &str =
    "Generate a certificate, save, then upload it to the app registration.";

/// Said when the draft certificate of an unsaved form is gone.
pub const DRAFT_GONE: &str =
    "The generated certificate is no longer available: it was not saved in time. Generate a new one and save.";

/// Said when a Microsoft login has no mailbox address to read and send as.
pub const ENTER_MAILBOX: &str = "Enter the mailbox's email address: Marcan reads and sends as it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_refusal_says_what_to_change() {
        for kind in TokenErrorKind::ALL {
            let text = token_error_sentence(kind);
            assert!(text.ends_with('.'), "{text}");
            assert!(!text.contains("AADSTS"), "{text}");
        }
        let mailbox = "dmarc@example.com";
        for problem in [
            GraphProblem::Unreachable {
                host: "graph.microsoft.com",
            },
            GraphProblem::TokenRefused { mailbox },
            GraphProblem::NoAccess { mailbox },
            GraphProblem::NoMailbox { mailbox },
            GraphProblem::NoFolder {
                mailbox,
                folder: "DMARC",
            },
            GraphProblem::Throttled { mailbox },
        ] {
            let text = graph_sentence(&problem);
            assert!(text.ends_with('.'), "{text}");
        }
        let text = graph_sentence(&GraphProblem::NoAccess { mailbox });
        for needed in ["Application Mail.Read", "30 minutes to 2 hours", mailbox] {
            assert!(text.contains(needed), "{text}");
        }
        for refusal in [
            GraphRefusal::NoSendRole,
            GraphRefusal::NoMailbox,
            GraphRefusal::TokenRefused,
            GraphRefusal::Throttled,
            GraphRefusal::TooLarge,
            GraphRefusal::Unreachable {
                host: "graph.microsoft.com".into(),
            },
            GraphRefusal::Timeout { seconds: 60 },
            GraphRefusal::Refused,
        ] {
            let text = graph_send_sentence(mailbox, &refusal);
            assert!(text.ends_with('.'), "{text}");
            assert!(!text.contains("SMTP"), "{text}");
        }
        let text = graph_send_sentence(mailbox, &GraphRefusal::NoSendRole);
        for needed in ["Application Mail.Send", "30 minutes to 2 hours", mailbox] {
            assert!(text.contains(needed), "{text}");
        }
    }

    /// A sentence that asks for a step in Entra or Exchange Online leads to
    /// the guide's section on it; one about the form, the network or
    /// Microsoft's side leads nowhere.
    #[test]
    fn setup_steps_lead_to_the_guide() {
        for (kind, docs) in [
            (TokenErrorKind::WrongTenant, Some(REGISTER_DOCS)),
            (TokenErrorKind::UnknownClient, Some(REGISTER_DOCS)),
            (TokenErrorKind::AssertionRejected, Some(CERTIFICATE_DOCS)),
            (TokenErrorKind::BadCredential, Some(CERTIFICATE_DOCS)),
            (TokenErrorKind::Refused, None),
            (TokenErrorKind::Unreachable, None),
        ] {
            assert_eq!(token_error_docs(kind), docs, "{kind:?}");
        }
        let mailbox = "dmarc@example.com";
        assert_eq!(
            graph_docs(&GraphProblem::NoAccess { mailbox }),
            Some(EXCHANGE_DOCS)
        );
        assert_eq!(graph_docs(&GraphProblem::NoMailbox { mailbox }), None);
        assert_eq!(graph_docs(&GraphProblem::TokenRefused { mailbox }), None);
        assert_eq!(
            graph_send_docs(&GraphRefusal::NoSendRole),
            Some(EXCHANGE_DOCS)
        );
        assert_eq!(graph_send_docs(&GraphRefusal::NoMailbox), None);
        assert_eq!(graph_send_docs(&GraphRefusal::TokenRefused), None);
    }
}
