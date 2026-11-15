//! Which secret a login uses: the one typed, else the stored one, and the
//! stored one only for the server and username it was saved for. Reusing it
//! for any other target would let whoever reaches the UI send the stored
//! password to a server of their choosing.

use super::presets::preset_of;
use dmarc_store::AccountFields;

/// A typed secret exactly as typed (spaces can be part of a password); only
/// an all-blank value counts as not typed.
pub fn typed(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// One of an account's two servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Server {
    Imap,
    Smtp,
}

/// Whether the secret stored with `saved` (the stored account, if any) may
/// go to `fields`' `server`: the one rule for a save and for a test. It may
/// while that server is unset (it sends nothing: a server removed is no
/// leak) or is the one it was saved for, with the same username; never
/// without a stored account.
pub fn reusable(saved: Option<&AccountFields>, fields: &AccountFields, server: Server) -> bool {
    saved.is_some_and(|saved| match server {
        Server::Imap => {
            fields.imap_host.is_empty()
                || same_imap_target(saved, &fields.imap_host, fields.imap_port, &fields.username)
        }
        Server::Smtp => {
            fields.smtp_host.is_empty()
                || same_smtp_target(saved, &fields.smtp_host, fields.smtp_port, &fields.username)
        }
    })
}

/// Whether the stored secret may go to every server of `fields`: what a
/// save, which stores it for all of them at once, asks.
pub fn reusable_for_all(saved: Option<&AccountFields>, fields: &AccountFields) -> bool {
    [Server::Imap, Server::Smtp]
        .into_iter()
        .all(|server| reusable(saved, fields, server))
}

/// Whether `host`, `port` and `username` are the IMAP login `saved` stores
/// its secret for (the host's case aside).
fn same_imap_target(saved: &AccountFields, host: &str, port: u16, username: &str) -> bool {
    saved.imap_host.eq_ignore_ascii_case(host)
        && saved.imap_port == port
        && saved.username == username
}

/// Whether `host`, `port` and `username` are the SMTP login `saved` stores
/// its secret for (the host's case aside). Ports compare as the provider
/// resolves them ([`Preset::smtp_port`](super::presets::Preset::smtp_port)):
/// a stored unset port (an account converted from 0.3 without one) is the
/// port it sends through, and `port` is resolved the same way when saving
/// and testing.
fn same_smtp_target(saved: &AccountFields, host: &str, port: Option<u16>, username: &str) -> bool {
    let saved_port = preset_of(saved).smtp_port(saved.smtp_port, saved.smtp_security);
    saved.smtp_host.eq_ignore_ascii_case(host)
        && port == Some(saved_port)
        && saved.username == username
}

/// The secret a login to a target uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The one typed.
    Typed(String),
    /// The stored one: none was typed and the target is the one it was
    /// saved for (or none is stored, so there is nothing to keep or leak).
    Stored,
    /// None was typed, one is stored, and it belongs to another target: it
    /// must be typed again.
    Required,
}

/// What a login to a target uses: `typed`, else the stored secret when
/// `same_target` or when none is stored (`has_stored`).
pub fn resolve(typed_value: Option<&str>, has_stored: bool, same_target: bool) -> Resolved {
    match typed(typed_value) {
        Some(value) => Resolved::Typed(value.to_owned()),
        None if same_target || !has_stored => Resolved::Stored,
        None => Resolved::Required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> AccountFields {
        AccountFields {
            username: "u@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: Some(587),
            ..AccountFields::default()
        }
    }

    #[test]
    fn the_stored_secret_goes_only_to_its_saved_target() {
        let s = saved();
        assert!(same_imap_target(
            &s,
            "IMAP.example.com",
            993,
            "u@example.com"
        ));
        assert!(!same_imap_target(
            &s,
            "imap.example.net",
            993,
            "u@example.com"
        ));
        assert!(!same_imap_target(
            &s,
            "imap.example.com",
            143,
            "u@example.com"
        ));
        assert!(!same_imap_target(
            &s,
            "imap.example.com",
            993,
            "v@example.com"
        ));
        assert!(same_smtp_target(
            &s,
            "smtp.EXAMPLE.com",
            Some(587),
            "u@example.com"
        ));
        assert!(!same_smtp_target(
            &s,
            "smtp.example.com",
            None,
            "u@example.com"
        ));
        // A stored unset port is the one it sends through.
        let unset = AccountFields {
            smtp_port: None,
            smtp_security: dmarc_store::SmtpSecurity::StartTls,
            ..saved()
        };
        assert!(same_smtp_target(
            &unset,
            "smtp.example.com",
            Some(587),
            "u@example.com"
        ));
        assert!(!same_smtp_target(
            &unset,
            "smtp.example.com",
            Some(465),
            "u@example.com"
        ));

        // Per server for a test, every server for a save: an unset server
        // takes nothing, a changed one or another username is no reuse.
        let moved = AccountFields {
            smtp_host: "smtp.example.net".into(),
            ..saved()
        };
        assert!(reusable(Some(&s), &moved, Server::Imap));
        assert!(!reusable(Some(&s), &moved, Server::Smtp));
        assert!(!reusable_for_all(Some(&s), &moved));
        let receiving_only = AccountFields {
            smtp_host: String::new(),
            ..saved()
        };
        assert!(reusable_for_all(Some(&s), &receiving_only));
        let other_user = AccountFields {
            username: "v@example.com".into(),
            ..saved()
        };
        assert!(!reusable(Some(&s), &other_user, Server::Imap));
        assert!(!reusable_for_all(None, &saved()), "no stored account");

        assert_eq!(
            resolve(Some(" p "), true, false),
            Resolved::Typed(" p ".into())
        );
        assert_eq!(resolve(Some("  "), true, true), Resolved::Stored);
        assert_eq!(resolve(None, true, false), Resolved::Required);
        assert_eq!(resolve(None, false, false), Resolved::Stored);
    }
}
