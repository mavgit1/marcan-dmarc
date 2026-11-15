//! What is said about an account: what its save or test lacks, that a test
//! step logged in, when its credential expires, whose stored credentials cannot be read
//! (the log line at start and at each failing pass or send, the settings
//! and the health answers, and a send that is refused), why a stored
//! password is not used, that sending has no account chosen or that account
//! no address to send from, and why an account that sending goes through
//! cannot be deleted or lose its SMTP server.

use crate::wording::alerts::long_date;
use dmarc_assess::{CredentialCondition, LockReason};

/// Sending has no account to go through: said by the Sending page's save
/// and its test, by every send of the stored group (alerts, client reports)
/// and, in the same words, beside the UI's disabled Save.
pub const NO_SENDING_ACCOUNT: &str = "Choose an account to send through under Settings → Sending.";

/// A save of an account with neither an IMAP nor an SMTP server: it could
/// neither receive nor send, and a credential would have nowhere to go.
pub const NO_SERVER: &str =
    "Enter an IMAP or SMTP server: without one the account neither receives reports nor sends.";

/// A save or test that would send the stored password to another server or
/// username.
pub const PASSWORD_AGAIN: &str =
    "Enter the password again: the stored one is only used for the servers and username it was saved for.";

/// A save or test of a password login with servers and no password, typed
/// or stored.
pub const ENTER_PASSWORD: &str = "Enter the password.";

/// A save of a Microsoft 365 account with a password login: Marcan reads
/// and sends as a Microsoft 365 mailbox over Microsoft Graph only.
pub const MICROSOFT_LOGIN_ONLY: &str = "Microsoft 365 accounts log in with a Microsoft login.";

/// A save or test of a password login with servers and no username.
pub const ENTER_USERNAME: &str = "Enter the username.";

/// A save of a receiving account without an address.
pub const ENTER_ADDRESS: &str = "Enter the email address reports are sent to.";

/// A save of an account with neither a name nor an address.
pub const ENTER_NAME: &str = "Enter a name for the account.";

/// A save of a password login that receives without an IMAP server.
pub const ENTER_IMAP_HOST: &str = "Enter the IMAP server's host name.";

/// A test of a password login with no server to try.
pub const ENTER_SERVER_TO_TEST: &str = "Enter a server to test.";

/// The SMTP step of a test whose SMTP server has no port.
pub const ENTER_SMTP_PORT: &str = "Enter the SMTP server's port.";

/// What a test's sentences call an account whose form has neither a name
/// nor an address yet.
pub const UNNAMED: &str = "this account";

/// The mailbox step of a Microsoft login's test reached `address`.
pub fn read_over_graph(address: &str) -> String {
    format!("Read {address} over Microsoft Graph.")
}

/// A test step whose login succeeded, as `user`.
pub fn logged_in_as(user: &str) -> String {
    format!("Logged in as {user}.")
}

/// The account sending goes through has no address to send from.
pub fn no_sender_address(name: &str) -> String {
    format!("{name} has no email address to send from. Enter one under Accounts.")
}

/// The address of the account sending goes through is not one.
pub fn invalid_sender_address(name: &str, address: &str) -> String {
    format!("The email address {address} of {name} is not a valid email address.")
}

/// Why the account sending goes through cannot be deleted while sending is on.
pub fn sending_uses(name: &str) -> String {
    format!(
        "Sending goes through {name}. Choose another account under Sending, or turn sending off, first."
    )
}

/// Why the account sending goes through (a password login) keeps its SMTP
/// server while sending is on.
pub fn sending_needs_smtp(name: &str) -> String {
    format!(
        "Sending goes through {name}, so it needs its SMTP server. Choose another account under Sending first."
    )
}

/// Said on an account (its card, its page) when receiving account `other`
/// reads the same mailbox and folder.
pub fn same_mailbox_as(other: &str) -> String {
    format!(
        "{other} reads the same mailbox and folder. Each report is stored once, so one of the two accounts is enough."
    )
}

/// When an expiring or expired credential (a Microsoft login's
/// certificate) runs out: "The certificate expires on 20 November 2026." `None` for any
/// other condition, or without a date.
pub fn expiry_sentence(condition: CredentialCondition, expires_at: Option<i64>) -> Option<String> {
    let verb = match condition {
        CredentialCondition::Expiring => "expires",
        CredentialCondition::Expired => "expired",
        CredentialCondition::Ok | CredentialCondition::Locked => return None,
    };
    expires_at.map(|at| format!("The certificate {verb} on {}.", long_date(at)))
}

/// Why account `name` is locked, and the two ways out.
pub fn locked_sentence(name: &str, reason: LockReason) -> String {
    match reason {
        LockReason::KeyMissing => format!(
            "The credentials of {name} are encrypted and MARCAN_SECRET_KEY is not set. Restore MARCAN_SECRET_KEY, or enter the credentials again under Settings."
        ),
        LockReason::KeyChanged => format!(
            "The credentials of {name} were encrypted with another MARCAN_SECRET_KEY. Restore that key, or enter the credentials again under Settings."
        ),
        LockReason::Damaged => format!(
            "The stored credentials of {name} are damaged: they do not open with MARCAN_SECRET_KEY, which encrypted them. Restore the database from a backup, or enter the credentials again under Settings."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expiry_is_said_with_its_date() {
        let at = Some(1_795_132_800);
        assert_eq!(
            expiry_sentence(CredentialCondition::Expiring, at).as_deref(),
            Some("The certificate expires on 20 November 2026.")
        );
        assert_eq!(
            expiry_sentence(CredentialCondition::Expired, at).as_deref(),
            Some("The certificate expired on 20 November 2026.")
        );
        assert_eq!(expiry_sentence(CredentialCondition::Ok, at), None);
        assert_eq!(expiry_sentence(CredentialCondition::Expiring, None), None);
    }

    #[test]
    fn each_lock_names_the_account_and_both_ways_out() {
        for reason in [
            LockReason::KeyMissing,
            LockReason::KeyChanged,
            LockReason::Damaged,
        ] {
            let text = locked_sentence("Reports", reason);
            assert!(text.contains("Reports"), "{text}");
            assert!(text.contains("MARCAN_SECRET_KEY"), "{text}");
            assert!(text.contains("enter the credentials again"), "{text}");
        }
    }
}
