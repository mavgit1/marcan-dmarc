//! The servers and login each option of the account form's Provider starts
//! from: IMAP/SMTP's none (entered by hand), Google's fixed hosts, and
//! Microsoft 365's none at all (it reads and sends over Microsoft Graph).
//! Google is a prefill only: its accounts are stored as
//! `generic` (the `accounts.provider` CHECK knows no other), and
//! [`ProviderOption::of`] is the one place that tells one apart again.

use dmarc_store::{AccountFields, AccountProvider, ImapSecurity, LoginKind, SmtpSecurity};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An option of the account form's Provider: what an account is shown as
/// and saved from. Stored as its preset's [`Preset::provider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOption {
    Generic,
    Google,
    Microsoft365,
}

/// Google's IMAP host: a generic account reading it is shown as Google.
const GOOGLE_IMAP_HOST: &str = "imap.gmail.com";

impl ProviderOption {
    /// The option an account stored with `provider` and `imap_host` is
    /// shown as: Google for a generic account reading `imap.gmail.com`.
    pub fn of(provider: AccountProvider, imap_host: &str) -> Self {
        match provider {
            AccountProvider::Microsoft365 => Self::Microsoft365,
            AccountProvider::Generic if imap_host.trim().eq_ignore_ascii_case(GOOGLE_IMAP_HOST) => {
                Self::Google
            }
            AccountProvider::Generic => Self::Generic,
        }
    }
}

/// What an account of one provider option starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub option: ProviderOption,
    /// How its accounts are stored.
    pub provider: AccountProvider,
    /// How the option is named in the account form.
    pub label: &'static str,
    /// The line under the provider's choice, before its docs link: what it
    /// covers, then how it logs in.
    pub hint: &'static str,
    /// Its section of the docs, relative to their root.
    pub docs: &'static str,
    /// The IMAP and SMTP servers its accounts start from; `None`: they have
    /// none (a Microsoft login).
    pub servers: Option<PresetServers>,
    /// The one login its accounts use.
    pub login: LoginKind,
}

/// The servers a password login's preset starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetServers {
    /// Empty: entered by hand.
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: ImapSecurity,
    /// Empty: entered by hand.
    pub smtp_host: &'static str,
    /// `None`: entered by hand (the form starts at the usual port).
    pub smtp_port: Option<u16>,
    pub smtp_security: SmtpSecurity,
}

/// Every option's preset, in the order the form offers them; a new account
/// starts on the first.
pub const PRESETS: [Preset; 3] = [
    Preset {
        option: ProviderOption::Generic,
        provider: AccountProvider::Generic,
        label: "IMAP/SMTP",
        hint: "Any mail provider: enter its servers under Advanced",
        docs: "accounts#imap-smtp",
        servers: Some(PresetServers {
            imap_host: "",
            imap_port: dmarc_mailbox::DEFAULT_PORT,
            imap_security: ImapSecurity::Tls,
            smtp_host: "",
            smtp_port: None,
            smtp_security: SmtpSecurity::Tls,
        }),
        login: LoginKind::Password,
    },
    Preset {
        option: ProviderOption::Google,
        provider: AccountProvider::Generic,
        label: "Google",
        hint: "Gmail or Google Workspace: log in with an app password",
        docs: "accounts#google",
        servers: Some(PresetServers {
            imap_host: GOOGLE_IMAP_HOST,
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            smtp_host: "smtp.gmail.com",
            smtp_port: Some(465),
            smtp_security: SmtpSecurity::Tls,
        }),
        login: LoginKind::Password,
    },
    Preset {
        option: ProviderOption::Microsoft365,
        provider: AccountProvider::Microsoft365,
        label: "Microsoft 365",
        hint: "Exchange Online: set up the mailbox with the guide first",
        docs: "accounts#microsoft-365",
        servers: None,
        login: LoginKind::Microsoft,
    },
];

impl Preset {
    /// The IMAP port an account of this provider uses for `port`: itself,
    /// or for 0 (not set) the preset's port where `security` is the
    /// preset's, else the usual port for `security`. The one place an unset
    /// port is decided, for saving and for testing alike.
    pub fn imap_port(&self, port: u16, security: ImapSecurity) -> u16 {
        let preset = self.servers.map(|s| (s.imap_port, s.imap_security));
        match port {
            0 if preset.is_some_and(|(_, s)| s == security) => preset.map_or(0, |(p, _)| p),
            0 => match security {
                ImapSecurity::Tls => dmarc_mailbox::DEFAULT_PORT,
                ImapSecurity::StartTls => dmarc_mailbox::STARTTLS_PORT,
            },
            port => port,
        }
    }

    /// [`Preset::imap_port`] for SMTP: `None` and 0 are not set.
    pub fn smtp_port(&self, port: Option<u16>, security: SmtpSecurity) -> u16 {
        let preset = self
            .servers
            .and_then(|s| Some((s.smtp_port?, s.smtp_security)));
        match port.unwrap_or(0) {
            0 => match preset {
                Some((preset, preset_security)) if security == preset_security => preset,
                _ => crate::mail::default_port(security),
            },
            port => port,
        }
    }
}

/// `option`'s preset.
pub fn preset(option: ProviderOption) -> &'static Preset {
    PRESETS
        .iter()
        .find(|p| p.option == option)
        .expect("every option has a preset")
}

/// The preset of a stored (or normalised) account: its option's.
pub fn preset_of(fields: &AccountFields) -> &'static Preset {
    preset(ProviderOption::of(fields.provider, &fields.imap_host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_option_has_its_hosts_and_login_and_imap_smtp_comes_first() {
        assert_eq!(PRESETS[0].option, ProviderOption::Generic);
        let ms = preset(ProviderOption::Microsoft365);
        assert_eq!(ms.servers, None);
        assert_eq!(ms.login, LoginKind::Microsoft);
        let google = preset(ProviderOption::Google);
        assert_eq!(google.provider, AccountProvider::Generic);
        let servers = google.servers.unwrap();
        assert_eq!(
            (servers.imap_host, servers.imap_port),
            ("imap.gmail.com", 993)
        );
        assert_eq!(
            (servers.smtp_host, servers.smtp_port, servers.smtp_security),
            ("smtp.gmail.com", Some(465), SmtpSecurity::Tls)
        );
        assert_eq!(google.login, LoginKind::Password);
        let generic = preset(ProviderOption::Generic);
        let servers = generic.servers.unwrap();
        assert_eq!((servers.imap_host, servers.smtp_host), ("", ""));
        assert_eq!(servers.smtp_port, None);
        assert_eq!(generic.login, LoginKind::Password);
    }

    #[test]
    fn a_generic_account_on_gmails_imap_host_is_shown_as_google() {
        use AccountProvider::*;
        let of = ProviderOption::of;
        assert_eq!(of(Generic, "imap.gmail.com"), ProviderOption::Google);
        assert_eq!(of(Generic, " IMAP.Gmail.com "), ProviderOption::Google);
        assert_eq!(of(Generic, "imap.example.com"), ProviderOption::Generic);
        assert_eq!(of(Generic, ""), ProviderOption::Generic);
        assert_eq!(
            of(Microsoft365, "imap.gmail.com"),
            ProviderOption::Microsoft365
        );
    }

    #[test]
    fn an_unset_port_of_either_server_is_the_presets_or_the_usual_one() {
        let google = preset(ProviderOption::Google);
        let generic = preset(ProviderOption::Generic);
        assert_eq!(google.imap_port(0, ImapSecurity::Tls), 993);
        assert_eq!(generic.imap_port(0, ImapSecurity::StartTls), 143);
        assert_eq!(generic.imap_port(1993, ImapSecurity::Tls), 1993);
        for unset in [None, Some(0)] {
            assert_eq!(google.smtp_port(unset, SmtpSecurity::Tls), 465);
            assert_eq!(google.smtp_port(unset, SmtpSecurity::StartTls), 587);
            assert_eq!(generic.smtp_port(unset, SmtpSecurity::Tls), 465);
            assert_eq!(generic.smtp_port(unset, SmtpSecurity::StartTls), 587);
        }
        assert_eq!(generic.smtp_port(Some(2525), SmtpSecurity::Tls), 2525);
    }
}
