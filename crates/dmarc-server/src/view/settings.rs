//! Every setting as `GET /api/settings` (and each `PUT /api/settings/{group}`)
//! answers: the stored groups with the worker's defaults applied, what follows
//! from them (`configured`, `report_org_domain`, `reporting_mode_saved`,
//! `effective_sender`, the license `state` and `licensed_to`), and each
//! group's defaults next to it. Secrets never leave: a password is
//! `has_password`, the operator key `has_key` and its `key_hint`.
//!
//! A group's `PUT` takes the group's values in the shape it answers them, the
//! whole group each time: the store's own group (`SendingGroup`,
//! `AlertSettings`, `ClientReportSettings`, `BrandingSettings`,
//! `DnsSettings`, `MonitoringSettings`, `RetentionSettings`,
//! `OnboardingSettings`, `UpdateSettings`), or `MailboxGroup` (the mailbox with the worker's
//! defaults applied). A field means the same in and out, and an empty string
//! means "not set" where the field says so. Only a secret is absent to keep
//! the stored one (a password, the operator key), and the reporting group,
//! which the setup saves one field at a time.

use crate::client_reports::job::ClientReportSchedule;
use crate::imap_backend::stored_imap_config;
use crate::license::{self, Standing};
use crate::mail;
use crate::snapshots::dns::DEFAULT_NAMESERVERS;
use crate::wording::client_reports::undelivered_line;
use dmarc_assess::{ClientReportHold, Status};
use dmarc_store::{
    AlertSettings, AppSettings, BrandingSettings, ClientReportSettings, DnsSettings, ImapSecurity,
    LicenseSettings, LogoInfo, LogoType, MonitoringSettings, OnboardingSettings, PixelSize,
    ReportingMode, ReportingSettings, RetentionSettings, SendingGroup, UpdateSettings,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SettingsResponse {
    pub mailbox: MailboxSettingsResponse,
    pub reporting: ReportingSettingsResponse,
    /// Saved clamped to one hour – one week.
    pub monitoring: MonitoringSettings,
    /// Saved clamped to one month – ten years.
    pub retention: RetentionSettings,
    pub dns: DnsSettingsResponse,
    pub sending: SendingSettingsResponse,
    pub alerts: AlertSettings,
    pub client_reports: ClientReportSettingsResponse,
    pub license: LicenseSettingsResponse,
    pub branding: BrandingSettingsResponse,
    pub onboarding: OnboardingSettings,
    pub updates: UpdateSettings,
}

/// The operator license: whether a key is stored, whether it holds and whom
/// it names, never the key itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct LicenseSettingsResponse {
    /// An operator key is stored.
    pub has_key: bool,
    /// What the stored key means now.
    pub state: LicenseState,
    /// The stored key's email, when the key is valid; `null` otherwise.
    pub licensed_to: Option<String>,
    /// The stored key named without showing it ([`license::key_hint`]):
    /// "you@acme.example · …GPL7CQ"; `null` without a key.
    pub key_hint: Option<String>,
}

/// What the stored operator key means: the free-software notice is off
/// exactly while it is `active`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LicenseState {
    /// No key is stored: the notice is shown.
    NoKey,
    /// A valid key is stored: the notice is off.
    Active,
    /// A key is stored but does not verify (the signing key changed): the
    /// notice is shown.
    Invalid,
}

impl LicenseState {
    pub fn new(license: &LicenseSettings, standing: &Standing) -> Self {
        match (&license.operator_key, &standing.licensed_to) {
            (None, _) => Self::NoKey,
            (Some(_), Some(_)) => Self::Active,
            (Some(_), None) => Self::Invalid,
        }
    }
}

impl LicenseSettingsResponse {
    fn new(license: &LicenseSettings) -> Self {
        let standing = Standing::new(license);
        let key = license.operator_key.as_deref();
        Self {
            has_key: key.is_some(),
            state: LicenseState::new(license, &standing),
            licensed_to: standing.licensed_to,
            key_hint: key.map(license::key_hint),
        }
    }
}

/// The top of every email ([`mail::email_top`]): decided once from the
/// operator's name and logo and the license's [`Standing`], and shown the
/// same way by the HTML, the plain text and the Branding page's preview.
/// The email's own title follows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct EmailTop {
    /// Without a valid operator key, the first line names Marcan; `null` with one.
    pub software: Option<SoftwareLine>,
    /// The operator's letterhead; `null` when there is nothing to show.
    pub operator: Option<OperatorLetterhead>,
}

/// The first line of an email without an operator license: Marcan's bar
/// mark at `mark`'s size, then `wordmark` and `rest`, all muted, on one line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SoftwareLine {
    /// The size the mark is shown at, in pixels.
    pub mark: PixelSize,
    /// "Marcan", the wordmark (semibold).
    pub wordmark: String,
    /// What follows it: ", free software".
    pub rest: String,
}

/// The operator's letterhead in an email.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct OperatorLetterhead {
    /// With a valid operator key: the logo at full height and `name` in bold
    /// beside it. Without one, secondary to Marcan: the logo smaller and
    /// `name` in regular weight beside it. Either way one row, the logo left
    /// of the name, both vertically centred.
    pub prominent: bool,
    /// The size the logo is shown at, in pixels: its own size fitted into
    /// the letterhead's box, keeping its shape; `null` without a logo.
    pub logo: Option<PixelSize>,
    /// "Acme IT", or "Managed by Acme IT" when not prominent; `null` with a
    /// logo and no name.
    pub name: Option<String>,
}

/// `POST /api/settings/branding/preview` (the Branding page's "How your
/// emails look"): the emails for a name and logo that may not be saved yet,
/// rendered by the code the emails use, as they go out now and in the other
/// license state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct EmailPreview {
    /// The operator name as it would be saved (trimmed); `null` when blank.
    pub operator_name: Option<String>,
    /// The notice ends the emails: no valid operator key is stored.
    pub notice_shown: bool,
    /// The emails as they go out now, with the stored license.
    pub current: EmailLook,
    /// The emails in the other state: with an operator license while none
    /// is stored, without one while one is.
    pub other: EmailLook,
}

/// How an email looks in one license state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct EmailLook {
    /// This look carries the free-software notice (no operator license).
    pub notice_shown: bool,
    /// The `From` display name ([`mail::from_name`]).
    pub from_name: String,
    /// The top ([`mail::email_top`]).
    pub top: EmailTop,
    /// A sample title under the top: a client report's.
    pub title: String,
    /// The last line ([`mail::sign_off`]): the free-software notice signed
    /// with the name, or "Sent by <name>." with an operator license.
    pub sign_off: String,
}

/// The branding group with the stored logo. Free for everyone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct BrandingSettingsResponse {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: BrandingSettings,
    /// A logo was uploaded (the image itself is `GET /api/meta`'s `branding.logo`).
    pub has_logo: bool,
    /// The uploaded logo's format; `null` without one.
    pub logo_type: Option<LogoType>,
    /// The uploaded logo's size in bytes; `null` without one.
    pub logo_bytes: Option<u32>,
}

impl BrandingSettingsResponse {
    fn new(branding: &BrandingSettings, logo: Option<LogoInfo>) -> Self {
        Self {
            group: branding.clone(),
            has_logo: logo.is_some(),
            logo_type: logo.map(|l| l.logo_type),
            logo_bytes: logo.map(|l| l.bytes),
        }
    }
}

/// The client reports group and when they go out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ClientReportSettingsResponse {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: ClientReportSettings,
    /// When the reports go out next (unix seconds): the report time that is
    /// due while some recipient does not have it yet, else the next one;
    /// `null` while they are off.
    pub next_send_at: Option<i64>,
    /// The organizations that get their report then: those with a recipient
    /// still waiting for the due one, else every one in the recipient list.
    pub organizations_due: i64,
    /// When a report last went out; `null` before the first.
    pub last_sent_at: Option<i64>,
    /// Why the due reports wait (the mailbox has not been read lately);
    /// `null` unless they are due, not sent, and held.
    pub held_reason: Option<ClientReportHold>,
    /// The latest report time's reports that did not reach a recipient;
    /// `null` when every one tried went out.
    pub undelivered: Option<UndeliveredReports>,
}

/// The reports of the latest report time that did not reach a listed
/// recipient: one line naming each with its cause and what becomes of it
/// ("2 of 4 reports not delivered: x@y (mailbox does not exist), a@b
/// (server unreachable, retrying until 4 November)."), coloured by the
/// worst of them: `warn` while all are still tried, `fail` once one was
/// refused or given up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct UndeliveredReports {
    pub line: String,
    pub status: Status,
}

impl ClientReportSettingsResponse {
    fn new(reports: &ClientReportSettings, schedule: &ClientReportSchedule) -> Self {
        Self {
            group: reports.clone(),
            next_send_at: schedule.next_send_at,
            organizations_due: schedule.organizations_due,
            last_sent_at: schedule.last_sent_at,
            held_reason: schedule.held,
            undelivered: undelivered_line(schedule.attempted, &schedule.undelivered).map(|line| {
                UndeliveredReports {
                    line,
                    status: Status::worst_of(schedule.undelivered.iter().map(|u| u.retry.status())),
                }
            }),
        }
    }
}

/// The sending group, its stored password named, and what follows from it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SendingSettingsResponse {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: SendingGroup,
    /// The SMTP account's own password is stored.
    pub has_password: bool,
    /// The address mail goes out from: `sender`, else (with the same login)
    /// the mailbox username when it is an address; empty when neither.
    pub effective_sender: String,
    /// Where test emails and previews go: `test_recipient`, else
    /// `effective_sender`; empty when neither.
    pub effective_test_recipient: String,
    pub defaults: SendingDefaults,
}

/// The usual submission ports, for the port field's starting value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SendingDefaults {
    /// The usual port with TLS.
    pub port: u16,
    /// The usual port with `STARTTLS`.
    pub starttls_port: u16,
}

impl SendingSettingsResponse {
    fn new(settings: &AppSettings) -> Self {
        let login = settings.sending_login();
        Self {
            group: settings.sending.group.clone(),
            has_password: !settings.sending.password.is_empty(),
            effective_sender: mail::parse_address(&login.sender)
                .map(|a| a.to_string())
                .unwrap_or_default(),
            effective_test_recipient: mail::parse_address(&settings.test_recipient())
                .map(|a| a.to_string())
                .unwrap_or_default(),
            defaults: SendingDefaults {
                port: mail::TLS_PORT,
                starttls_port: mail::STARTTLS_PORT,
            },
        }
    }
}

/// The mailbox the IMAP worker polls, as stored with its defaults applied,
/// and as `PUT /api/settings/mailbox` takes it (with the password beside it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct MailboxGroup {
    /// Never empty once saved.
    pub host: String,
    /// 1–65535; the usual one for `security` until one is saved.
    pub port: u16,
    /// TLS from the start or `STARTTLS`; never plaintext.
    pub security: ImapSecurity,
    /// Never empty once saved.
    pub username: String,
    /// The folder reports are read from. Empty: `INBOX`.
    pub folder: String,
    /// At least 10 seconds.
    pub poll_interval_secs: u64,
}

/// The mailbox group, its stored password named, and whether it polls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct MailboxSettingsResponse {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: MailboxGroup,
    pub has_password: bool,
    /// Host, username and password are all set, so the loop polls.
    pub configured: bool,
    /// What an unset field means, for placeholders and resets.
    pub defaults: MailboxDefaults,
}

/// Where DMARC reports are sent, and how they reach the mailbox. `PUT
/// /api/settings/reporting` takes the same two values, each optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ReportingSettingsResponse {
    /// Mailbox address DMARC `rua=` tags point at. Empty: not set.
    pub report_address: String,
    /// Organizational domain (PSL) of `report_address`'s domain; empty when no
    /// address is set. Domains under the same organizational domain need no
    /// RFC 7489 §7.1 authorization record.
    pub report_org_domain: String,
    pub reporting_mode: ReportingMode,
    /// A reporting mode was saved explicitly (the setup's "Report delivery"
    /// step); `false` while `reporting_mode` is only the default.
    pub reporting_mode_saved: bool,
}

/// The DNS group and the built-in resolvers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct DnsSettingsResponse {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: DnsSettings,
    /// The built-in resolvers used while none are configured.
    pub defaults: Vec<String>,
}

/// The mailbox settings' defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct MailboxDefaults {
    /// The usual port with TLS.
    pub port: u16,
    /// The usual port with `STARTTLS`.
    pub starttls_port: u16,
    pub folder: String,
    pub poll_interval_secs: u64,
}

impl SettingsResponse {
    /// Every group of `settings`, the client reports with their `schedule`,
    /// the branding with the stored `logo`'s type and size.
    pub fn new(
        settings: &AppSettings,
        schedule: &ClientReportSchedule,
        logo: Option<LogoInfo>,
    ) -> Self {
        let cfg = stored_imap_config(&settings.mailbox);
        Self {
            mailbox: MailboxSettingsResponse {
                group: MailboxGroup {
                    host: cfg.host.clone(),
                    port: cfg.port,
                    security: settings.mailbox.security.unwrap_or_default(),
                    username: cfg.username.clone(),
                    folder: cfg.folder.clone(),
                    poll_interval_secs: cfg.poll_interval_secs,
                },
                has_password: !cfg.password.is_empty(),
                configured: cfg.is_complete(),
                defaults: MailboxDefaults::default(),
            },
            reporting: ReportingSettingsResponse::new(&settings.reporting),
            monitoring: settings.monitoring,
            retention: settings.retention,
            dns: DnsSettingsResponse {
                group: settings.dns.clone(),
                defaults: DEFAULT_NAMESERVERS
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            },
            sending: SendingSettingsResponse::new(settings),
            alerts: settings.alerts.clone(),
            client_reports: ClientReportSettingsResponse::new(&settings.client_reports, schedule),
            license: LicenseSettingsResponse::new(&settings.license),
            branding: BrandingSettingsResponse::new(&settings.branding, logo),
            onboarding: settings.onboarding.clone(),
            updates: settings.updates,
        }
    }
}

impl Default for MailboxDefaults {
    fn default() -> Self {
        Self {
            port: dmarc_imap::DEFAULT_PORT,
            starttls_port: dmarc_imap::STARTTLS_PORT,
            folder: dmarc_imap::DEFAULT_FOLDER.to_owned(),
            poll_interval_secs: dmarc_imap::DEFAULT_POLL_INTERVAL_SECS,
        }
    }
}

impl ReportingSettingsResponse {
    fn new(reporting: &ReportingSettings) -> Self {
        let report_address = reporting.report_address.clone().unwrap_or_default();
        Self {
            report_org_domain: report_address
                .rsplit_once('@')
                .map(|(_, domain)| {
                    dmarc_domain::organizational_domain(domain)
                        .unwrap_or_else(|| dmarc_domain::normalize_name(domain))
                })
                .unwrap_or_default(),
            report_address,
            reporting_mode: reporting.reporting_mode.unwrap_or_default(),
            reporting_mode_saved: reporting.reporting_mode.is_some(),
        }
    }
}
